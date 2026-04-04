use archinstall_zfs_core::config::types::GlobalConfig;

use super::{MenuItem, MenuKind, StepId};

/// Build a read-only summary of all steps plus validation errors and action buttons.
pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let mut items = Vec::new();

    // Collect items from each step as read-only summary lines
    for step in &StepId::ALL[..6] {
        // Add section header for each step
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

        for item in step_items {
            // Convert all items to non-interactive summary (Custom kind renders as read-only)
            items.push(MenuItem {
                key: item.key,
                label: item.label,
                value: item.value,
                kind: MenuKind::SectionHeader, // read-only display
            });
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

    // Separator before actions
    items.push(MenuItem {
        key: "sep_actions",
        label: "",
        value: String::new(),
        kind: MenuKind::SectionHeader,
    });

    // Actions
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
