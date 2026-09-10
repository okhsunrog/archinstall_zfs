use archinstall_zfs_core::config::types::GlobalConfig;

use super::{MenuItem, MenuKind, StepId, items_for};

/// Build a read-only summary of all steps plus validation errors and action buttons.
pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let mut items = Vec::new();

    for step in &StepId::ALL[..6] {
        items.push(MenuItem::header("section", step.label(), String::new()));

        let step_items = items_for(*step, config);

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
                    items.push(MenuItem::header(
                        "summary",
                        header_label,
                        selected_label.to_string(),
                    ));
                }
                _ => {
                    items.push(MenuItem::header(item.key, item.label, item.value.clone()));
                    i += 1;
                }
            }
        }
    }

    // Validation errors
    let errors = config.validate_for_install();
    if !errors.is_empty() {
        items.push(MenuItem::header("sep_errors", "", String::new()));
        items.push(MenuItem::header(
            "errors_header",
            "Validation Errors",
            String::new(),
        ));
        for error in &errors {
            items.push(MenuItem::header("error", "", error.to_string()));
        }
    }

    items.push(MenuItem::header("sep_actions", "", String::new()));

    items.extend([
        MenuItem::action("save", "Save configuration"),
        MenuItem::action("install", "Install"),
        MenuItem::action("quit", "Quit"),
    ]);

    items
}
