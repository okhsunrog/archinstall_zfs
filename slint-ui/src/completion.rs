//! Non-secret result shared by successive GUI processes and the console supervisor.
use archinstall_zfs_core::installed_system::InstalledSystem;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Completion {
    pub target: Option<InstalledSystem>,
    pub notice: String,
}

pub type State = Arc<Mutex<Completion>>;

pub fn show(app: &crate::ui::App, completion: &Completion) {
    use slint::ComponentHandle;
    let state = app.global::<crate::ui::InstallState>();
    state.set_state(2);
    state.set_phase(14);
    state.set_phase_label("Installation complete".into());
    state.set_shell_available(completion.target.is_some() && available());
    state.set_shell_notice(completion.notice.clone().into());
}

pub fn available() -> bool {
    #[cfg(feature = "linuxkms")]
    {
        crate::console_session::connected()
    }
    #[cfg(not(feature = "linuxkms"))]
    {
        false
    }
}
