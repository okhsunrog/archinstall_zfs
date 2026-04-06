//! Quit / reboot handler. If the install completed successfully (state == 2)
//! and the user clicks Reboot, exec `systemctl reboot` after hiding the window.

use slint::ComponentHandle;

use crate::ui::{App, InstallState};

pub fn setup(app: &App) {
    let weak = app.as_weak();
    app.on_quit_requested(move || {
        let Some(app) = weak.upgrade() else { return };
        let should_reboot = app.global::<InstallState>().get_state() == 2;
        let _ = app.window().hide();
        if should_reboot {
            let _ = std::process::Command::new("systemctl")
                .arg("reboot")
                .spawn();
        }
    });
}
