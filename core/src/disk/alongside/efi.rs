use super::*;

/// Planning uses a 48 MiB image allowance. The image is built for this machine
/// with `xz -9` (see `bootmenu::ZBM_DRACUT_CONF`); a stock `linux-lts` target
/// measured 33 MiB. With [`ESP_SLACK_BYTES`] added, a 100 MiB Windows ESP holding only
/// Microsoft's loader qualifies for reuse. Actual installation verifies the
/// generated file before replacing anything. Existing files are not credited
/// as reclaimable: they may be other loaders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootSpace {
    pub image_bytes: u64,
    pub backup: bool,
    pub fallback: bool,
}

impl Default for BootSpace {
    fn default() -> Self {
        Self {
            image_bytes: 48 * MIB,
            backup: false,
            fallback: false,
        }
    }
}

impl BootSpace {
    pub fn required_bytes(self) -> Result<u64> {
        ensure!(
            self.image_bytes > 0,
            "ZFSBootMenu image allowance must be nonzero"
        );
        // One image, and only explicitly enabled persistent copies.
        self.image_bytes
            .checked_mul(1 + u64::from(self.backup) + u64::from(self.fallback))
            .and_then(|size| size.checked_add(ESP_SLACK_BYTES))
            .ok_or_else(|| eyre!("EFI space calculation overflow"))
    }
}

/// Reuse is the default and the recommendation. A separate ESP is only ever an
/// explicit selection, never an automatic fallback: it costs `ESP_BYTES` of the
/// allocation and leaves two ESPs for firmware and the other system to choose
/// between. Both variants name the existing ESP that was inspected, so a disk
/// without a readable FAT ESP is rejected either way; execution repeats that
/// inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EfiChoice {
    Reuse {
        partition: u32,
    },
    #[serde(alias = "CreateAfterInsufficientSpace")]
    CreateSeparate {
        existing_partition: u32,
    },
}

impl EfiChoice {
    pub fn existing_partition(&self) -> u32 {
        match *self {
            Self::Reuse { partition } => partition,
            Self::CreateSeparate { existing_partition } => existing_partition,
        }
    }

    pub fn validate_space(&self, space: &EfiSpace, budget: BootSpace) -> Result<()> {
        ensure!(
            self.existing_partition() == space.partition,
            "EFI capacity check belongs to another partition"
        );
        if let Self::Reuse { .. } = self {
            ensure!(
                space.sufficient(budget)?,
                "The existing EFI partition has {} MiB free; {} MiB is needed for boot files and updates. Select a separate EFI partition, or free space and refresh.",
                space.free_bytes / MIB,
                budget.required_bytes()?.div_ceil(MIB)
            );
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
        assert_eq!(budget.required_bytes().unwrap(), 50_681_856 + 8 * MIB);
        assert_eq!(
            BootSpace {
                backup: true,
                ..budget
            }
            .required_bytes()
            .unwrap(),
            2 * 50_681_856 + 8 * MIB
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
    fn reuse_requires_space_and_a_separate_esp_is_an_explicit_choice() {
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
        let create = EfiChoice::CreateSeparate {
            existing_partition: 1,
        };
        assert!(reuse.validate_space(&enough, budget).is_ok());
        assert!(reuse.validate_space(&small, budget).is_err());
        // The separate ESP is allowed whether or not the existing one fits.
        assert!(create.validate_space(&small, budget).is_ok());
        assert!(create.validate_space(&enough, budget).is_ok());
        let with_backup = BootSpace {
            backup: true,
            ..budget
        };
        assert!(reuse.validate_space(&enough, with_backup).is_err());
        // Either choice must refer to the ESP whose capacity was inspected.
        for choice in [&reuse, &create] {
            assert!(
                choice
                    .validate_space(
                        &EfiSpace {
                            partition: 2,
                            ..enough
                        },
                        budget
                    )
                    .is_err()
            );
        }
        let old_name: EfiChoice =
            serde_json::from_str(r#"{"CreateAfterInsufficientSpace":{"existing_partition":1}}"#)
                .unwrap();
        assert_eq!(old_name, create);
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
