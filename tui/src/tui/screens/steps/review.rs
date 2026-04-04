use archinstall_zfs_core::config::types::GlobalConfig;

use super::{MenuItem, MenuKind, StepId};

/// Build a read-only summary of all steps plus validation errors and action buttons.
pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let mut items = Vec::new();

    for step in &StepId::ALL[..6] {
        items.push(MenuItem {
            key: "section",
            label: step.label(),
            value: String::new(),
            kind: MenuKind::SectionHeader,
        });

        let step_items = match step {
            StepId::Welcome => super::welcome::items(config),
            StepId::Disk => super::disk::items(config),
            StepId::Zfs => super::zfs::items(config),
            StepId::System => super::system::items(config),
            StepId::Users => super::users::items(config),
            StepId::Desktop => super::desktop::items(config),
            StepId::Review => unreachable!(),
        };

        // Flatten radio groups: show "Header: Selected option" as one summary line
        let mut i = 0;
        while i < step_items.len() {
            let item = &step_items[i];
            match &item.kind {
                MenuKind::RadioHeader => {
                    let header_label = item.label;
                    let mut selected_label = "Not set";
                    i += 1;
                    while i < step_items.len() {
                        if let MenuKind::RadioOption { selected, .. } = &step_items[i].kind {
                            if *selected {
                                selected_label = step_items[i].label;
                            }
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    items.push(MenuItem {
                        key: "summary",
                        label: header_label,
                        value: selected_label.to_string(),
                        kind: MenuKind::SectionHeader,
                    });
                }
                _ => {
                    items.push(MenuItem {
                        key: item.key,
                        label: item.label,
                        value: item.value.clone(),
                        kind: MenuKind::SectionHeader,
                    });
                    i += 1;
                }
            }
        }
    }

    // Validation errors
    let errors = config.validate_for_install();
    if !errors.is_empty() {
        items.push(MenuItem {
            key: "sep_errors",
            label: "",
            value: String::new(),
            kind: MenuKind::SectionHeader,
        });
        items.push(MenuItem {
            key: "errors_header",
            label: "Validation Errors",
            value: String::new(),
            kind: MenuKind::SectionHeader,
        });
        for error in &errors {
            items.push(MenuItem {
                key: "error",
                label: "",
                value: error.clone(),
                kind: MenuKind::SectionHeader,
            });
        }
    }

    items.push(MenuItem {
        key: "sep_actions",
        label: "",
        value: String::new(),
        kind: MenuKind::SectionHeader,
    });

    items.extend([
        MenuItem {
            key: "save",
            label: "Save configuration",
            value: String::new(),
            kind: MenuKind::Action,
        },
        MenuItem {
            key: "install",
            label: "Install",
            value: String::new(),
            kind: MenuKind::Action,
        },
        MenuItem {
            key: "quit",
            label: "Quit",
            value: String::new(),
            kind: MenuKind::Action,
        },
    ]);

    items
}
