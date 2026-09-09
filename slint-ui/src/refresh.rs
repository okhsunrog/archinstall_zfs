//! `refresh_items` is the single point that rebuilds the wizard's
//! `config-items` list and resets the keyboard focus index. Controllers call
//! it after every config mutation.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use archinstall_zfs_core::config::types::GlobalConfig;

use crate::config_items::build_step_items;
use crate::ui::{App, WizardState};
use archinstall_zfs_core::config::choices::Choice;

pub fn refresh_items(app: &App, config: &GlobalConfig) {
    let step = app.global::<WizardState>().get_current_step() as usize;
    let items = build_step_items(step, config);
    let first = -1;
    let wizard = app.global::<WizardState>();
    wizard.set_focused_index(first);
    wizard.set_storage_mode(
        config
            .installation_mode
            .map(|m| m.index() as i32)
            .unwrap_or(-1),
    );
    refresh_validation(app, config);
    wizard.set_config_items(ModelRc::new(VecModel::from(items)));
    wizard.set_status_text(SharedString::default());
}

pub fn refresh_validation(app: &App, config: &GlobalConfig) {
    let mut errors: Vec<String> = config
        .validate_for_install()
        .iter()
        .map(ToString::to_string)
        .collect();
    errors.extend(crate::storage::issues(config));
    let wizard = app.global::<WizardState>();
    wizard.set_environment_path(
        archinstall_zfs_core::boot_environment::BootEnvironment::new(
            config.pool_name.as_deref().unwrap_or("?"),
            &config.dataset_prefix,
        )
        .base()
        .into(),
    );
    wizard.set_can_install(errors.is_empty());
    wizard.set_validation_summary(if errors.is_empty() {
        "".into()
    } else {
        format!("{} required setting(s) need attention", errors.len()).into()
    });
}

/// Rebuilding a conditional form must not send keyboard focus back to the window.
pub fn focus_item(app: &App, key: &str) {
    use slint::Model;
    let index = app
        .global::<WizardState>()
        .get_config_items()
        .iter()
        .position(|item| item.key == key);
    if let Some(index) = index {
        let weak = app.as_weak();
        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
            if let Some(app) = weak.upgrade() {
                app.global::<WizardState>().set_focused_index(index as i32);
            }
        });
    }
}
