//! Read-only discovery and editable preview of the shared alongside plan.
use crate::{
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
    capacity: u64,
    detail: String,
    error: String,
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
/// The user changed a control: the loaded plan no longer applies.
fn touched() {
    SESSION.with_borrow_mut(|s| {
        s.kept = None;
        s.notice.clear();
    });
}
thread_local! { static SESSION: RefCell<Session> = RefCell::default(); }
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
        let minimum = minimum_size(&RealRunner, p, fs);
        let (capacity, error) = match minimum {
            Ok(min) => (
                size.saturating_sub(min.saturating_add(2 * MIB)),
                String::new(),
            ),
            Err(e) => (0, format!("{e:#}")),
        };
        sources.push(Source {
            source: SpaceSource::Shrink { partition: number },
            label,
            capacity,
            detail: format!(
                "Keep {} at its current start; reduce only its end. Up to {:.0} GiB can be allocated here.",
                p.node.display(),
                gib(capacity)
            ),
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
            Source {
                source: SpaceSource::Shrink { partition: 2 },
                label: "/dev/nvme0n1p2 — NTFS — 450 GiB".into(),
                capacity: 240 * GIB,
                detail: "Windows keeps at least 210 GiB, including working space. Only the end of this partition will move.".into(),
                error: String::new(),
            },
            Source {
                source: SpaceSource::Unallocated {
                    start: 452 * GIB / 512,
                    end: layout.lastlba + 1,
                },
                label: "Unallocated space — 60 GiB".into(),
                capacity: ((layout.lastlba + 1) * 512 - 452 * GIB) / MIB * MIB,
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
            survey.sources[0].error =
                "Resizing NTFS requires ntfsresize (package ntfs-3g). Install it in the live system and refresh, or use unallocated space."
                    .into();
        }
        Ok("no-efi") => survey.efis.clear(),
        _ => {}
    }
    survey
}

fn load(app: &App, index: Option<usize>, keep: Option<Request>) {
    let state = app.global::<AlongsideState>();
    let generation = state.get_generation() + 1;
    state.set_generation(generation);
    state.set_busy(true);
    state.set_error("".into());
    SESSION.with_borrow_mut(|s| {
        s.kept = None;
        s.notice.clear();
    });
    state.invoke_rebuild();
    let disk = index
        .and_then(|i| SESSION.with_borrow(|s| s.disks.get(i).cloned()))
        .or_else(|| keep.as_ref().map(|r| r.before.device.clone()));
    let previous = SESSION.with_borrow(|s| s.survey.clone());
    let weak = app.as_weak();
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || -> Result<_, String> {
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
        .unwrap_or_else(|e| Err(e.to_string()));
        let _ = weak.upgrade_in_event_loop(move |app| {
            let state = app.global::<AlongsideState>();
            if state.get_generation() != generation {
                return;
            }
            state.set_busy(false);
            let result = result.and_then(|(paths, names, selected_index, inspected)| {
                state.set_disks(strings(names));
                state.set_disk_index(selected_index as i32);
                SESSION.with_borrow_mut(|s| s.disks = paths);
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
                        state.set_allocation(gib(r.allocation_bytes) as f32);
                        if r.swap_bytes > 0 {
                            state.set_swap_size((r.swap_bytes / GIB) as f32);
                        }
                        SESSION.with_borrow_mut(|s| {
                            s.survey = Some(survey);
                            s.kept = Some(r);
                        });
                        state.invoke_rebuild();
                        return;
                    }
                    if keep.is_some() {
                        SESSION.with_borrow_mut(|s| {
                            s.notice = "The previous plan no longer matches this disk; this is a new plan built from the current layout.".into()
                        });
                    }
                    state.set_source_index(
                        survey
                            .sources
                            .iter()
                            .position(|s| {
                                matches!(s.source, SpaceSource::Unallocated { .. })
                                    && s.error.is_empty()
                                    && s.capacity >= MIN_LINUX_BYTES
                            })
                            .or_else(|| {
                                survey.sources.iter().position(|s| {
                                    s.error.is_empty() && s.capacity >= MIN_LINUX_BYTES
                                })
                            })
                            .map(|i| i as i32)
                            .unwrap_or(0),
                    );
                    state.set_efi_index(
                        survey
                            .efis
                            .iter()
                            .position(|e| {
                                e.space.as_ref().is_ok_and(|s| {
                                    s.sufficient(Default::default()).unwrap_or(false)
                                })
                            })
                            .unwrap_or(0) as i32,
                    );
                    state.set_additional_efi(false);
                    state.set_use_all(true);
                    SESSION.with_borrow_mut(|s| {
                        s.survey = Some(survey);
                    });
                    state.invoke_rebuild();
                }
                Err(error) => {
                    SESSION.with_borrow_mut(|s| s.survey = None);
                    state.set_before(Default::default());
                    state.set_after(Default::default());
                    state.set_sources(Default::default());
                    state.set_efi_partitions(Default::default());
                    state.set_error(error.into());
                }
            }
        });
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
    let required = budget
        .required_bytes()
        .map_err(|e| e.to_string())?
        .div_ceil(MIB);
    let consequence = if insufficient {
        "It cannot be reused; select a separate EFI partition to continue."
    } else if space.free_bytes < required + budget.image_bytes {
        "Later ZFSBootMenu updates replace the image in place instead of writing a second copy first."
    } else {
        "Reusing it formats nothing and keeps the existing loaders."
    };
    Ok(format!(
        "{} MiB free; {required} MiB is needed to reuse it. {consequence}",
        space.free_bytes / MIB
    ))
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
    SESSION.with_borrow_mut(|s| {
        if s.kept
            .as_ref()
            .is_some_and(|k| (k.swap_bytes > 0) != wants_swap)
        {
            s.kept = None;
            s.notice.clear();
        }
    });
    state.set_allocation_summary("".into());
    state.set_insufficient(false);
    state.set_efi_details("".into());
    let result = SESSION.with_borrow(|session| -> Result<Request, String> {
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
        let min = gib(MIN_LINUX_BYTES + swap_bytes + extra) as f32;
        let max = gib(source.capacity) as f32;
        state.set_minimum(min);
        state.set_maximum(max);
        let all_bytes = source.capacity / MIB * MIB;
        let allocation_bytes = if let Some(k) = kept {
            k.allocation_bytes
        } else if state.get_use_all() {
            all_bytes
        } else {
            (state.get_allocation().round().max(min) as u64)
                .saturating_mul(GIB)
                .min(all_bytes)
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
                format!(
                    "{}: {:.0} → {:.0} GiB. Its start and existing data are preserved.",
                    old.node.display(),
                    sectors_gib(old.size, s.layout.sectorsize),
                    sectors_gib(*size, s.layout.sectorsize)
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
            let notice = SESSION.with_borrow(|s| s.notice.clone());
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
    let weak = app.as_weak();
    app.global::<AlongsideState>()
        .on_select_source(move |index| {
            touched();
            if let Some(app) = weak.upgrade() {
                let s = app.global::<AlongsideState>();
                s.set_source_index(index);
                s.set_use_all(true);
                s.invoke_rebuild();
            }
        });
    let weak = app.as_weak();
    app.global::<AlongsideState>().on_select_efi(move |index| {
        touched();
        if let Some(app) = weak.upgrade() {
            let s = app.global::<AlongsideState>();
            s.set_efi_index(index);
            s.set_additional_efi(false);
            s.invoke_rebuild();
        }
    });
    let weak = app.as_weak();
    app.global::<AlongsideState>().on_allocate(move |value| {
        touched();
        if let Some(app) = weak.upgrade()
            && value.is_finite()
        {
            let s = app.global::<AlongsideState>();
            s.set_use_all(false);
            s.set_allocation(value);
            s.invoke_rebuild();
        }
    });
    let weak = app.as_weak();
    app.global::<AlongsideState>().on_all_space(move |value| {
        touched();
        if let Some(app) = weak.upgrade() {
            let s = app.global::<AlongsideState>();
            s.set_use_all(value);
            s.invoke_rebuild();
        }
    });
    let weak = app.as_weak();
    let cfg = config.clone();
    app.global::<AlongsideState>().on_select_swap(move |index| {
        touched();
        if let Some(app) = weak.upgrade()
            && let Some(mode) = SwapMode::from_index(index as usize)
        {
            cfg.borrow_mut().swap_mode = mode;
            app.global::<AlongsideState>().invoke_rebuild();
        }
    });
    let weak = app.as_weak();
    app.global::<AlongsideState>().on_size_swap(move |value| {
        touched();
        if let Some(app) = weak.upgrade()
            && value.is_finite()
        {
            let s = app.global::<AlongsideState>();
            s.set_swap_size(value.round().clamp(1.0, 1024.0));
            s.invoke_rebuild();
        }
    });
    let weak = app.as_weak();
    app.global::<AlongsideState>().on_additional(move |value| {
        touched();
        if let Some(app) = weak.upgrade() {
            let s = app.global::<AlongsideState>();
            s.set_additional_efi(value);
            s.invoke_rebuild();
        }
    });
}
