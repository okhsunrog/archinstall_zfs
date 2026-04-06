//! `refresh_items` is the single point that rebuilds the wizard's
//! `config-items` list and resets the keyboard focus index. Controllers call
//! it after every config mutation.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use archinstall_zfs_core::config::types::GlobalConfig;

use crate::config_items::{build_step_items, next_selectable_index};
use crate::ui::{App, WizardState};

pub fn refresh_items(app: &App, config: &GlobalConfig) {
    let step = app.global::<WizardState>().get_current_step() as usize;
    let items = build_step_items(step, config);
    let first = next_selectable_index(&items, -1, 1);
    let wizard = app.global::<WizardState>();
    wizard.set_focused_index(first);
    wizard.set_config_items(ModelRc::new(VecModel::from(items)));
    wizard.set_status_text(SharedString::default());
}
