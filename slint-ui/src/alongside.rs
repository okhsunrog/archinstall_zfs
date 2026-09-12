//! Read-only discovery and editable preview of the shared alongside plan.
use crate::{
    busy::{self, Guarded},
    format::{gib, sectors_gib, sectors_mib},
    ui::{AlongsideState, App, DiskSegment, WizardState},
};
use archinstall_zfs_core::{
    config::{
        choices::Choice,
        types::{GlobalConfig, InstallationMode, SwapMode},
    },
    disk::{alongside::*, device},
    system::cmd::RealRunner,
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

#[derive(Clone)]
struct Source {
    source: SpaceSource,
    label: String,
    /// Hard maximum: the whole extent, or what the resizer allows.
    capacity: u64,
    /// For a shrink source: partition size and the filesystem's resize limits.
    shrink: Option<(u64, ShrinkLimits)>,
    detail: String,
    error: String,
}

impl Source {
    fn is_unallocated(&self) -> bool {
        matches!(self.source, SpaceSource::Unallocated { .. })
    }

    /// The allocation proposed for `needed` bytes, and whether it exceeds the
    /// margin the retained system keeps by default.
    fn proposal(&self, needed: u64) -> Option<(u64, bool)> {
        match self.shrink {
            Some((size, limits)) => {
                let d = ShrinkDefaults::compute(size, limits, needed);
                d.default.map(|bytes| (bytes, d.cramped()))
            }
            None => (self.capacity >= needed).then_some((self.capacity / MIB * MIB, false)),
        }
    }
}

/// What shrinking `node` leaves for its current system, for the source line.
fn shrink_detail(node: &std::path::Path, size: u64, limits: ShrinkLimits) -> String {
    let free = size.saturating_sub(limits.used_bytes);
    format!(
        "{} uses {:.0} GiB and has {:.0} GiB free. Only its end moves; the proposal shares that free space and leaves it a working margin.",
        node.display(),
        gib(limits.used_bytes),
        gib(free)
    )
}
#[derive(Clone)]
struct Efi {
    number: u32,
    label: String,
    space: Result<EfiSpace, String>,
}
#[derive(Clone)]
struct Survey {
    layout: Layout,
    sources: Vec<Source>,
    efis: Vec<Efi>,
}
#[derive(Default)]
struct Session {
    disks: Vec<PathBuf>,
    survey: Option<Survey>,
    /// A plan loaded from a configuration file that matches the surveyed
    /// layout. It is used verbatim until the user changes any control.
    kept: Option<Request>,
    notice: String,
}
thread_local! { static SESSION: RefCell<Session> = RefCell::default(); }
impl Session {
    fn with<R>(f: impl FnOnce(&Session) -> R) -> R {
        SESSION.with_borrow(f)
    }
    fn update<R>(f: impl FnOnce(&mut Session) -> R) -> R {
        SESSION.with_borrow_mut(f)
    }
    /// The user changed a control: the loaded plan no longer applies.
    fn touch() {
        Self::update(|s| {
            s.kept = None;
            s.notice.clear();
        });
    }
    fn disk(index: usize) -> Option<PathBuf> {
        Self::with(|s| s.disks.get(index).cloned())
    }
    fn set_disks(disks: Vec<PathBuf>) {
        Self::update(|s| s.disks = disks);
    }
    fn survey() -> Option<Survey> {
        Self::with(|s| s.survey.clone())
    }
    fn set_survey(survey: Option<Survey>) {
        Self::update(|s| s.survey = survey);
    }
    /// Shows and executes `request` as written for the surveyed layout.
    fn keep(survey: Survey, request: Request) {
        Self::update(|s| {
            s.survey = Some(survey);
            s.kept = Some(request);
        });
    }
    fn notice() -> String {
        Self::with(|s| s.notice.clone())
    }
    fn set_notice(notice: &str) {
        Self::update(|s| s.notice = notice.into());
    }
}
fn strings(values: Vec<String>) -> ModelRc<slint::SharedString> {
    ModelRc::new(VecModel::from(
        values.into_iter().map(Into::into).collect::<Vec<_>>(),
    ))
}

/// Filesystem minimum-size probes (e2fsck, resize2fs -P, ntfsresize) take
/// seconds to minutes per partition. Their results from `previous` are reused
/// while the partition table and filesystem type are unchanged; the ESP
/// capacity check is cheap and always repeated. Execution probes again before
/// touching anything.
fn survey(disk: &std::path::Path, previous: Option<&Survey>) -> Result<Survey, String> {
    check_tools(&[
        ("sfdisk", "util-linux"),
        ("sgdisk", "gptfdisk"),
        ("lsblk", "util-linux"),
    ])
    .map_err(|e| e.to_string())?;
    let (layout, filesystems) = inspect(&RealRunner, disk).map_err(|e| format!("{e:#}"))?;
    let mut sources = Vec::new();
    let mut efis = Vec::new();
    for p in &layout.partitions {
        let number = p.number(&layout.device).map_err(|e| e.to_string())?;
        if p.kind.eq_ignore_ascii_case(EFI_TYPE) {
            efis.push(Efi {
                number,
                label: format!(
                    "{} — {:.0} MiB",
                    p.node.display(),
                    sectors_mib(p.size, layout.sectorsize)
                ),
                space: inspect_efi(&RealRunner, &layout, number, &filesystems)
                    .map_err(|e| format!("{e:#}")),
            });
            continue;
        }
        let fs = filesystems
            .iter()
            .find(|f| f.path == p.node)
            .and_then(|f| f.fstype.as_deref())
            .unwrap_or("unknown filesystem");
        let size = p.size * layout.sectorsize;
        let label = format!("{} — {} — {:.1} GiB", p.node.display(), fs, gib(size));
        if let Some(cached) = previous
            .filter(|prev| prev.layout == layout)
            .and_then(|prev| {
                prev.sources.iter().find(|s| {
                    s.source == SpaceSource::Shrink { partition: number } && s.label == label
                })
            })
        {
            sources.push(cached.clone());
            continue;
        }
        let (capacity, shrink, detail, error) = match shrink_limits(&RealRunner, p, fs) {
            Ok(limits) => (
                ShrinkDefaults::compute(size, limits, 0).hard_max,
                Some((size, limits)),
                shrink_detail(&p.node, size, limits),
                String::new(),
            ),
            Err(e) => (0, None, String::new(), format!("{e:#}")),
        };
        sources.push(Source {
            source: SpaceSource::Shrink { partition: number },
            label,
            capacity,
            shrink,
            detail,
            error,
        });
    }
    for (start, end) in layout.free_extents().map_err(|e| e.to_string())? {
        let alignment = MIB / layout.sectorsize;
        let bytes = end.saturating_sub(start.div_ceil(alignment) * alignment) * layout.sectorsize;
        if bytes >= MIN_LINUX_BYTES {
            sources.push(Source {
                source: SpaceSource::Unallocated { start, end },
                label: format!("Unallocated space — {:.1} GiB", gib(bytes)),
                capacity: bytes,
                shrink: None,
                detail: "Create a ZFS partition here without shrinking an existing filesystem."
                    .into(),
                error: String::new(),
            });
        }
    }
    Ok(Survey {
        layout,
        sources,
        efis,
    })
}

fn fixture() -> Survey {
    let layout = Layout {
        label: "gpt".into(),
        id: "preview-disk".into(),
        device: "/dev/nvme0n1".into(),
        unit: "sectors".into(),
        sectorsize: 512,
        firstlba: 2048,
        lastlba: 512 * GIB / 512 - 34,
        partitions: vec![
            Partition {
                node: "/dev/nvme0n1p1".into(),
                start: 2048,
                size: 500 * MIB / 512,
                kind: EFI_TYPE.into(),
                uuid: "preview-efi".into(),
                name: "EFI".into(),
                attrs: "".into(),
            },
            Partition {
                node: "/dev/nvme0n1p2".into(),
                start: GIB / 512,
                size: 450 * GIB / 512,
                kind: BASIC_TYPE.into(),
                uuid: "preview-data".into(),
                name: "Windows".into(),
                attrs: "".into(),
            },
            Partition {
                node: "/dev/nvme0n1p3".into(),
                start: 451 * GIB / 512,
                size: GIB / 512,
                kind: BASIC_TYPE.into(),
                uuid: "preview-recovery".into(),
                name: "Recovery".into(),
                attrs: "".into(),
            },
        ],
    };
    let low = std::env::var("AZFS_PREVIEW_ESP").as_deref() == Ok("small");
    let mut survey = Survey {
        layout: layout.clone(),
        sources: vec![
            {
                // 170 GiB in use: 280 GiB free, 140 GiB proposed, 263 GiB at most.
                let size = 450 * GIB;
                let limits = ShrinkLimits {
                    used_bytes: 170 * GIB,
                    minimum_bytes: 187 * GIB,
                };
                Source {
                    source: SpaceSource::Shrink { partition: 2 },
                    label: "/dev/nvme0n1p2 — NTFS — 450 GiB".into(),
                    capacity: ShrinkDefaults::compute(size, limits, 0).hard_max,
                    shrink: Some((size, limits)),
                    detail: shrink_detail(std::path::Path::new("/dev/nvme0n1p2"), size, limits),
                    error: String::new(),
                }
            },
            Source {
                source: SpaceSource::Unallocated {
                    start: 452 * GIB / 512,
                    end: layout.lastlba + 1,
                },
                label: "Unallocated space — 60 GiB".into(),
                capacity: ((layout.lastlba + 1) * 512 - 452 * GIB) / MIB * MIB,
                shrink: None,
                detail: "Use free space without resizing Windows.".into(),
                error: String::new(),
            },
        ],
        efis: vec![Efi {
            number: 1,
            label: "/dev/nvme0n1p1 — EFI — 500 MiB".into(),
            space: Ok(EfiSpace {
                partition: 1,
                free_bytes: if low { 40 * MIB } else { 350 * MIB },
            }),
        }],
    };
    match std::env::var("AZFS_PREVIEW_ALONGSIDE").as_deref() {
        Ok("ext4") => {
            survey.layout.partitions[1].kind = LINUX_TYPE.into();
            survey.layout.partitions[1].name = "Linux".into();
            survey.sources[0].label = "/dev/nvme0n1p2 — ext4 — 450 GiB".into();
        }
        Ok("missing-tools") => {
            survey.sources[0].capacity = 0;
            survey.sources[0].shrink = None;
            survey.sources[0].error =
                "Resizing NTFS requires ntfsresize (package ntfs-3g). Install it in the live system and refresh, or use unallocated space."
                    .into();
        }
        Ok("no-efi") => survey.efis.clear(),
        _ => {}
    }
    survey
}

/// Free space that belongs to nobody comes first. Otherwise prefer the
/// partition whose proposal stays within its system's margin, then any
/// partition the resizer can make room on.
fn default_source(sources: &[Source]) -> Option<usize> {
    let usable = |s: &Source| s.error.is_empty();
    sources
        .iter()
        .position(|s| usable(s) && s.is_unallocated() && s.capacity >= MIN_LINUX_BYTES)
        .or_else(|| {
            sources
                .iter()
                .enumerate()
                .filter(|(_, s)| usable(s))
                .filter_map(|(i, s)| {
                    s.proposal(MIN_LINUX_BYTES)
                        .filter(|(_, cramped)| !cramped)
                        .map(|(bytes, _)| (i, bytes))
                })
                .max_by_key(|(_, bytes)| *bytes)
                .map(|(i, _)| i)
        })
        .or_else(|| {
            sources
                .iter()
                .position(|s| usable(s) && s.proposal(MIN_LINUX_BYTES).is_some())
        })
}

impl Guarded for AlongsideState<'_> {
    fn generation(app: &App) -> i32 {
        app.global::<AlongsideState>().get_generation()
    }
    fn set_generation(app: &App, generation: i32) {
        app.global::<AlongsideState>().set_generation(generation);
    }
    fn set_busy(app: &App, busy: bool) {
        app.global::<AlongsideState>().set_busy(busy);
    }
    fn set_error(app: &App, error: slint::SharedString) {
        app.global::<AlongsideState>().set_error(error);
    }
}

fn load(app: &App, index: Option<usize>, keep: Option<Request>) {
    let generation = busy::begin::<AlongsideState>(app);
    Session::touch();
    app.global::<AlongsideState>().invoke_rebuild();
    let disk = index
        .and_then(Session::disk)
        .or_else(|| keep.as_ref().map(|r| r.before.device.clone()));
    let previous = Session::survey();
    let work = async move {
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            if crate::preview::enabled() {
                return Ok((
                    vec![PathBuf::from("/dev/nvme0n1")],
                    vec!["Samsung SSD — 512 GiB".into()],
                    0,
                    if std::env::var("AZFS_PREVIEW_ALONGSIDE").as_deref() == Ok("mbr") {
                        Err(
                            "This disk does not use GPT. Automatic MBR conversion is not supported; existing data has not been changed."
                                .into(),
                        )
                    } else {
                        Ok(fixture())
                    },
                ));
            }
            let disks = device::disk_choices().map_err(|e| e.to_string())?;
            let paths: Vec<_> = disks.iter().map(|d| d.path.clone()).collect();
            let names = disks
                .iter()
                .map(|d| format!("{} — {} — {}", d.label, d.model, d.size))
                .collect::<Vec<_>>();
            let same = |a: &PathBuf, b: &PathBuf| {
                a == b || a.canonicalize().ok() == b.canonicalize().ok()
            };
            let selected_index = disk
                .as_ref()
                .and_then(|d| paths.iter().position(|p| same(p, d)))
                .unwrap_or(0);
            let inspected = paths
                .get(selected_index)
                .ok_or_else(|| "No disks found".to_string())
                .and_then(|p| survey(p, previous.as_ref()));
            Ok((paths, names, selected_index, inspected))
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
    };
    busy::spawn::<AlongsideState, _>(app, generation, work, move |app, result| {
        let state = app.global::<AlongsideState>();
        let result = result.and_then(|(paths, names, selected_index, inspected)| {
            state.set_disks(strings(names));
            state.set_disk_index(selected_index as i32);
            Session::set_disks(paths);
            inspected
        });
        match result {
            Ok(survey) => {
                state.set_sources(strings(
                    survey.sources.iter().map(|s| s.label.clone()).collect(),
                ));
                state.set_efi_partitions(strings(
                    survey.efis.iter().map(|s| s.label.clone()).collect(),
                ));
                // A plan from a configuration file, or the current plan on
                // a refresh, is shown and executed as written when it
                // still describes this disk.
                let kept = keep.as_ref().and_then(|r| {
                    let source = survey.sources.iter().position(|s| s.source == r.source)?;
                    let efi = survey
                        .efis
                        .iter()
                        .position(|e| e.number == r.efi.existing_partition())?;
                    (r.before == survey.layout).then_some((source, efi))
                });
                if let Some((source, efi)) = kept {
                    let r = keep.clone().expect("kept implies keep");
                    state.set_source_index(source as i32);
                    state.set_efi_index(efi as i32);
                    state.set_additional_efi(matches!(r.efi, EfiChoice::CreateSeparate { .. }));
                    state.set_use_all(false);
                    state.set_custom_allocation(true);
                    state.set_allocation(gib(r.allocation_bytes) as f32);
                    if r.swap_bytes > 0 {
                        state.set_swap_size((r.swap_bytes / GIB) as f32);
                    }
                    Session::keep(survey, r);
                    state.invoke_rebuild();
                    return;
                }
                if keep.is_some() {
                    Session::set_notice(
                        "The previous plan no longer matches this disk; this is a new plan built from the current layout.",
                    );
                }
                let chosen = default_source(&survey.sources);
                state.set_source_index(chosen.map(|i| i as i32).unwrap_or(0));
                state.set_use_all(chosen.is_some_and(|i| survey.sources[i].is_unallocated()));
                state.set_custom_allocation(false);
                state.set_efi_index(
                    survey
                        .efis
                        .iter()
                        .position(|e| {
                            e.space
                                .as_ref()
                                .is_ok_and(|s| s.sufficient(Default::default()).unwrap_or(false))
                        })
                        .unwrap_or(0) as i32,
                );
                state.set_additional_efi(false);
                Session::set_survey(Some(survey));
                state.invoke_rebuild();
            }
            Err(error) => {
                Session::set_survey(None);
                state.set_before(Default::default());
                state.set_after(Default::default());
                state.set_sources(Default::default());
                state.set_efi_partitions(Default::default());
                state.set_error(error.into());
            }
        }
    });
}

fn segments(layout: &Layout, plan: Option<&Plan>) -> ModelRc<DiskSegment> {
    let total = layout.lastlba + 1;
    let mut regions = Vec::new();
    for p in &layout.partitions {
        let resized = plan
            .and_then(|p2| p2.shrink.as_ref())
            .filter(|(old, _)| old.node == p.node);
        let size = resized.map(|(_, size)| *size).unwrap_or(p.size);
        let kind = if p.kind.eq_ignore_ascii_case(EFI_TYPE) {
            2
        } else if resized.is_some() {
            3
        } else {
            0
        };
        let name = if p.name.is_empty() {
            p.node.to_string_lossy().into_owned()
        } else {
            p.name.clone()
        };
        regions.push((p.start, p.start + size, kind, name));
    }
    if let Some(p) = plan {
        regions.push((
            p.zfs_start,
            p.swap_start.unwrap_or(p.end),
            1,
            "New ZFS".into(),
        ));
        if let Some(start) = p.swap_start {
            regions.push((start, p.end, 5, "Swap".into()));
        }
        if let Some(start) = p.efi_start {
            regions.push((start, p.zfs_start, 2, "New EFI".into()));
        }
    }
    regions.sort_by_key(|r| r.0);
    let mut cursor = layout.firstlba;
    let mut gaps = Vec::new();
    for &(start, end, _, _) in &regions {
        if start > cursor {
            gaps.push((cursor, start, 4, "Free".into()));
        }
        cursor = end;
    }
    if cursor < total {
        gaps.push((cursor, total, 4, "Free".into()));
    }
    regions.extend(gaps);
    ModelRc::new(VecModel::from(
        regions
            .into_iter()
            .map(|(start, end, kind, name)| DiskSegment {
                offset: start as f32 / total as f32,
                fraction: (end - start) as f32 / total as f32,
                kind,
                short_label: format!("{:.0} GiB", sectors_gib(end - start, layout.sectorsize))
                    .into(),
                label: format!(
                    "{name} · {:.0} GiB",
                    sectors_gib(end - start, layout.sectorsize)
                )
                .into(),
            })
            .collect::<Vec<_>>(),
    ))
}

/// Swap reserved inside the allocation: the kept plan's figure, or the
/// configured size when a swap partition is wanted.
fn planned_swap_bytes(
    kept: Option<&Request>,
    config: &GlobalConfig,
    state: &AlongsideState,
) -> u64 {
    if let Some(k) = kept {
        k.swap_bytes
    } else if config.swap_mode.uses_partition() {
        state.get_swap_size().round().clamp(1.0, 1024.0) as u64 * GIB
    } else {
        0
    }
}

/// How much of the existing ESP the boot files need, and what reusing it means.
fn efi_details(space: &EfiSpace, budget: BootSpace, insufficient: bool) -> Result<String, String> {
    let required_bytes = budget.required_bytes().map_err(|e| e.to_string())?;
    let required = required_bytes.div_ceil(MIB);
    let consequence = if insufficient {
        "It cannot be reused; select a separate EFI partition to continue."
    } else if space.free_bytes < required_bytes + budget.image_bytes {
        // Updates stage a second copy only when a whole image fits next to
        // the installed one.
        "Later ZFSBootMenu updates replace the image in place instead of writing a second copy first."
    } else {
        "Reusing it formats nothing and keeps the existing loaders."
    };
    Ok(format!(
        "{} MiB free; {required} MiB is needed to reuse it. {consequence}",
        space.free_bytes / MIB
    ))
}

/// Both sides of a shrink: the new size, what the retained system keeps
/// free, and a warning when that is little.
fn shrink_outcome(
    old: &Partition,
    new_sectors: u64,
    sectorsize: u64,
    limits: Option<ShrinkLimits>,
) -> String {
    let before = sectors_gib(old.size, sectorsize);
    let after = sectors_gib(new_sectors, sectorsize);
    let mut text = format!(
        "{}: {before:.0} → {after:.0} GiB. Its start and existing data are preserved.",
        old.node.display()
    );
    if let Some(limits) = limits {
        let size = old.size * sectorsize;
        let allocation = (old.size - new_sectors) * sectorsize;
        let (free_after, low) = retained_free(size, limits.used_bytes, allocation);
        text.push_str(&format!(" It keeps {:.0} GiB free.", gib(free_after)));
        if low {
            text.push_str(&format!(
                " That is under {WARN_FREE_PERCENT}% of the partition; updates and everyday use need room."
            ));
        }
    }
    text
}

fn allocation_summary(plan: &Plan, layout: &Layout, swap_bytes: u64) -> String {
    let zfs_sectors = plan.swap_start.unwrap_or(plan.end) - plan.zfs_start;
    let efi = if plan.efi_start.is_some() {
        " · New EFI: 512 MiB"
    } else {
        " · Existing EFI reused"
    };
    format!(
        "New ZFS pool: {:.1} GiB · Swap: {} GiB{efi}",
        sectors_gib(zfs_sectors, layout.sectorsize),
        swap_bytes / GIB
    )
}

fn rebuild(app: &App, config: &mut GlobalConfig) {
    if config.installation_mode != Some(InstallationMode::Alongside) {
        return;
    }
    config.alongside = None;
    let state = app.global::<AlongsideState>();
    if state.get_busy() {
        crate::refresh::refresh_validation(app, config);
        return;
    }
    state.set_swap_mode(config.swap_mode.index() as i32);
    // A kept plan whose swap no longer matches the configured method (changed
    // while another storage mode was selected) is rebuilt from the controls.
    let wants_swap = config.swap_mode.uses_partition();
    let stale = Session::with(|s| {
        s.kept
            .as_ref()
            .is_some_and(|k| (k.swap_bytes > 0) != wants_swap)
    });
    if stale {
        Session::touch();
    }
    state.set_allocation_summary("".into());
    state.set_insufficient(false);
    state.set_efi_details("".into());
    let result = Session::with(|session| -> Result<Request, String> {
        let kept = session.kept.as_ref();
        let s = session.survey.as_ref().ok_or("Select a disk")?;
        state.set_before(segments(&s.layout, None));
        state.set_after(segments(&s.layout, None));
        let source = s
            .sources
            .get(state.get_source_index() as usize)
            .ok_or("No suitable partitions or unallocated space")?;
        state.set_details(source.detail.clone().into());
        let swap_bytes = planned_swap_bytes(kept, config, &state);
        let extra = if state.get_additional_efi() {
            ESP_BYTES
        } else {
            0
        };
        let needed = MIN_LINUX_BYTES + swap_bytes + extra;
        let min = gib(needed) as f32;
        let max = gib(source.capacity) as f32;
        state.set_minimum(min);
        state.set_maximum(max);
        let all_bytes = source.capacity / MIB * MIB;
        let allocation_bytes = if let Some(k) = kept {
            k.allocation_bytes
        } else if state.get_use_all() {
            all_bytes
        } else if state.get_custom_allocation() {
            (state.get_allocation().round().max(min) as u64)
                .saturating_mul(GIB)
                .min(all_bytes)
        } else {
            source
                .proposal(needed)
                .map(|(bytes, _)| bytes)
                .unwrap_or(all_bytes)
        };
        state.set_allocation((gib(allocation_bytes) * 10.0).round() as f32 / 10.0);
        let efi = s.efis.get(state.get_efi_index() as usize).ok_or(
            "No existing EFI partition on this disk. Prepare an EFI partition before using this mode.",
        )?;
        let space = efi.space.as_ref().map_err(Clone::clone)?;
        let budget = BootSpace::default();
        let insufficient = !space.sufficient(budget).map_err(|e| e.to_string())?;
        state.set_insufficient(insufficient);
        state.set_efi_details(efi_details(space, budget, insufficient)?.into());
        if !source.error.is_empty() {
            return Err(source.error.clone());
        }
        if max < min {
            return Err(format!(
                "At least {min:.0} GiB is needed for the ZFS pool, swap and EFI; only {max:.1} GiB is available. Reduce swap or choose another source."
            ));
        }
        let efi_choice = if state.get_additional_efi() {
            EfiChoice::CreateSeparate {
                existing_partition: efi.number,
            }
        } else {
            EfiChoice::Reuse {
                partition: efi.number,
            }
        };
        efi_choice
            .validate_space(space, budget)
            .map_err(|e| e.to_string())?;
        let request = match kept {
            Some(k) => k.clone(),
            None => Request {
                before: s.layout.clone(),
                source: source.source.clone(),
                efi: efi_choice,
                allocation_bytes,
                swap_bytes,
            },
        };
        let plan = request.plan().map_err(|e| e.to_string())?;
        state.set_after(segments(&s.layout, Some(&plan)));
        state.set_allocation_summary(allocation_summary(&plan, &s.layout, swap_bytes).into());
        if let Some((old, size)) = &plan.shrink {
            state.set_details(
                shrink_outcome(
                    old,
                    *size,
                    s.layout.sectorsize,
                    source.shrink.map(|(_, limits)| limits),
                )
                .into(),
            );
        }
        Ok(request)
    });
    match result {
        Ok(request) => {
            config.alongside = Some(request);
            state.set_error("".into());
            let notice = Session::notice();
            if !notice.is_empty() {
                state.set_details(format!("{notice} {}", state.get_details()).into());
            }
        }
        Err(e) => state.set_error(e.into()),
    }
    // The survey finishes asynchronously. Rebuilding the item model resets
    // keyboard focus, which must not happen under the user's hands on another
    // step; those steps rebuild their items when shown.
    if app.global::<WizardState>().get_current_step() == DISK_STEP {
        crate::refresh::refresh_items(app, config);
    } else {
        crate::refresh::refresh_validation(app, config);
    }
}

/// Index of the Disk step in `WizardState.step-labels`.
const DISK_STEP: i32 = 1;

pub fn setup(app: &App, config: &Rc<RefCell<GlobalConfig>>) {
    let weak = app.as_weak();
    let cfg = config.clone();
    app.global::<AlongsideState>().on_reload(move || {
        if let Some(app) = weak.upgrade() {
            let keep = cfg.borrow().alongside.clone();
            load(&app, None, keep);
        }
    });
    let weak = app.as_weak();
    app.global::<AlongsideState>().on_select_disk(move |index| {
        if let Some(app) = weak.upgrade() {
            load(&app, Some(index as usize), None);
        }
    });
    let weak = app.as_weak();
    let cfg = config.clone();
    app.global::<AlongsideState>().on_rebuild(move || {
        if let Some(app) = weak.upgrade() {
            rebuild(&app, &mut cfg.borrow_mut());
        }
    });
    let state = app.global::<AlongsideState>();
    state.on_select_source(edit(app, |s, index: i32| {
        s.set_source_index(index);
        // Free space is taken whole; a shrink starts from the proposal.
        let unallocated = Session::with(|session| {
            session
                .survey
                .as_ref()
                .and_then(|survey| survey.sources.get(index as usize))
                .is_some_and(Source::is_unallocated)
        });
        s.set_use_all(unallocated);
        s.set_custom_allocation(false);
        true
    }));
    state.on_select_efi(edit(app, |s, index: i32| {
        s.set_efi_index(index);
        s.set_additional_efi(false);
        true
    }));
    state.on_allocate(edit(app, |s, value: f32| {
        if !value.is_finite() {
            return false;
        }
        s.set_use_all(false);
        s.set_custom_allocation(true);
        s.set_allocation(value);
        true
    }));
    state.on_all_space(edit(app, |s, value: bool| {
        s.set_use_all(value);
        // Unticking returns to the proposal rather than a stale number.
        s.set_custom_allocation(false);
        true
    }));
    let cfg = config.clone();
    state.on_select_swap(edit(app, move |_, index: i32| {
        let Some(mode) = SwapMode::from_index(index as usize) else {
            return false;
        };
        cfg.borrow_mut().swap_mode = mode;
        true
    }));
    state.on_size_swap(edit(app, |s, value: f32| {
        if !value.is_finite() {
            return false;
        }
        s.set_swap_size(value.round().clamp(1.0, 1024.0));
        true
    }));
    state.on_additional(edit(app, |s, value: bool| {
        s.set_additional_efi(value);
        true
    }));
}

/// A handler for one edited control. The loaded plan no longer applies;
/// `apply` changes the state and returns whether the plan must be rebuilt.
fn edit<T: 'static>(
    app: &App,
    apply: impl Fn(&AlongsideState, T) -> bool + 'static,
) -> impl Fn(T) + 'static {
    let weak = app.as_weak();
    move |value| {
        Session::touch();
        if let Some(app) = weak.upgrade() {
            let state = app.global::<AlongsideState>();
            if apply(&state, value) {
                state.invoke_rebuild();
            }
        }
    }
}
