use super::*;

/// Pass the actual generated image size. Never decide to create a second ESP
/// from a guessed bundle size. Existing files are conservatively not credited
/// as reclaimable; the installer does not own other bootloaders' files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootSpace {
    pub image_bytes: u64,
    pub backup: bool,
    pub fallback: bool,
}

impl BootSpace {
    pub fn required_bytes(self) -> Result<u64> {
        ensure!(
            self.image_bytes > 0,
            "Build or select the ZFSBootMenu image before calculating EFI space"
        );
        // Main image plus one replacement, and only explicitly enabled copies.
        // Reserve a modest allowance for cluster rounding and directory entries.
        self.image_bytes
            .checked_mul(2 + u64::from(self.backup) + u64::from(self.fallback))
            .and_then(|size| size.checked_add(8 * MIB))
            .ok_or_else(|| eyre!("EFI space calculation overflow"))
    }
}

/// A second ESP is never a default or an error fallback. It must refer to the
/// existing ESP whose capacity was checked; execution repeats that check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EfiChoice {
    Reuse { partition: u32 },
    CreateAfterInsufficientSpace { existing_partition: u32 },
}

impl EfiChoice {
    pub fn existing_partition(&self) -> u32 {
        match *self {
            Self::Reuse { partition } => partition,
            Self::CreateAfterInsufficientSpace { existing_partition } => existing_partition,
        }
    }

    pub fn validate_space(&self, space: &EfiSpace, budget: BootSpace) -> Result<()> {
        ensure!(
            self.existing_partition() == space.partition,
            "EFI capacity check belongs to another partition"
        );
        match self {
            Self::Reuse { .. } => ensure!(
                space.sufficient(budget)?,
                "The existing EFI partition has {} MiB free; {} MiB is reserved for boot files and updates. Choose explicitly whether to create an additional EFI partition, or free space and refresh.",
                space.free_bytes / MIB,
                budget.required_bytes()?.div_ceil(MIB)
            ),
            Self::CreateAfterInsufficientSpace { .. } => ensure!(
                !space.sufficient(BootSpace {
                    backup: false,
                    fallback: false,
                    ..budget
                })?,
                "The existing EFI partition fits the main image and an update. Reuse it; disable optional copies if necessary. A second ESP is offered only when the minimum configuration does not fit."
            ),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EfiSpace {
    pub partition: u32,
    pub free_bytes: u64,
}

impl EfiSpace {
    pub fn sufficient(&self, budget: BootSpace) -> Result<bool> {
        Ok(self.free_bytes >= budget.required_bytes()?)
    }
}

/// Inspect FAT without mounting or repairing it. A failed consistency or
/// capacity check is not a low-space result.
pub fn inspect_efi(
    runner: &dyn CommandRunner,
    layout: &Layout,
    number: u32,
    filesystems: &[Filesystem],
) -> Result<EfiSpace> {
    check_tools(&[("fsck.fat", "dosfstools"), ("mdir", "mtools")])?;
    let p = layout
        .partitions
        .iter()
        .find(|p| p.number(&layout.device).ok() == Some(number))
        .ok_or_else(|| eyre!("EFI partition disappeared"))?;
    ensure!(
        p.kind.eq_ignore_ascii_case(EFI_TYPE),
        "Selected partition is not an ESP"
    );
    ensure!(
        filesystems
            .iter()
            .any(|f| f.path == p.node && f.fstype.as_deref() == Some("vfat")),
        "The existing ESP must contain a FAT filesystem"
    );
    probe::run(runner, "fsck.fat", &["-n", &p.node.to_string_lossy()]).wrap_err("The existing EFI filesystem needs maintenance. No additional ESP is offered for a failed filesystem check")?;
    let output = probe::run(runner, "mdir", &["-i", &p.node.to_string_lossy(), "::"])?;
    let free_bytes = parse_free_bytes(&output)?;
    ensure!(
        free_bytes <= p.size * layout.sectorsize,
        "Invalid EFI free-space result"
    );
    Ok(EfiSpace {
        partition: number,
        free_bytes,
    })
}

fn parse_free_bytes(output: &str) -> Result<u64> {
    let lines: Vec<_> = output
        .lines()
        .filter_map(|l| l.trim().strip_suffix("bytes free"))
        .collect();
    ensure!(
        lines.len() == 1,
        "Cannot determine free space on the existing ESP; refresh after checking the filesystem"
    );
    let digits: String = lines[0].chars().filter(|c| !c.is_whitespace()).collect();
    Ok(digits.parse()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measured_image_and_only_enabled_copies_contribute_to_budget() {
        let budget = BootSpace {
            image_bytes: 50_681_856,
            backup: false,
            fallback: false,
        };
        assert_eq!(budget.required_bytes().unwrap(), 2 * 50_681_856 + 8 * MIB);
        assert_eq!(
            BootSpace {
                backup: true,
                ..budget
            }
            .required_bytes()
            .unwrap(),
            3 * 50_681_856 + 8 * MIB
        );
        assert!(
            BootSpace {
                image_bytes: 0,
                ..budget
            }
            .required_bytes()
            .is_err()
        );
        assert!(
            BootSpace {
                image_bytes: u64::MAX,
                ..budget
            }
            .required_bytes()
            .is_err()
        );
    }
    #[test]
    fn second_esp_requires_insufficient_space_on_the_selected_esp() {
        let budget = BootSpace {
            image_bytes: 50 * MIB,
            backup: false,
            fallback: false,
        };
        let required = budget.required_bytes().unwrap();
        let enough = EfiSpace {
            partition: 1,
            free_bytes: required,
        };
        let small = EfiSpace {
            partition: 1,
            free_bytes: required - 1,
        };
        let reuse = EfiChoice::Reuse { partition: 1 };
        let create = EfiChoice::CreateAfterInsufficientSpace {
            existing_partition: 1,
        };
        assert!(reuse.validate_space(&enough, budget).is_ok());
        assert!(reuse.validate_space(&small, budget).is_err());
        assert!(create.validate_space(&small, budget).is_ok());
        assert!(create.validate_space(&enough, budget).is_err());
        let with_backup = BootSpace {
            backup: true,
            ..budget
        };
        assert!(reuse.validate_space(&enough, with_backup).is_err());
        // An optional copy must not justify repartitioning the disk.
        assert!(create.validate_space(&enough, with_backup).is_err());
        assert!(
            create
                .validate_space(
                    &EfiSpace {
                        partition: 2,
                        ..small
                    },
                    budget
                )
                .is_err()
        );
    }
    #[test]
    fn unreadable_capacity_is_not_zero_space() {
        assert_eq!(
            parse_free_bytes("  123 456 789 bytes free\n").unwrap(),
            123456789
        );
        assert!(parse_free_bytes("Device unavailable").is_err());
        assert!(parse_free_bytes("12 bytes free\n34 bytes free").is_err());
    }
}
