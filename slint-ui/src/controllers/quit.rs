//! Separate quit and reboot handlers. Quitting after a successful install must
//! leave the live environment running so the user can inspect or copy files.

use slint::ComponentHandle;

use crate::ui::{App, DemoState, InstallState};

pub fn setup(app: &App) {
    let weak = app.as_weak();
    app.window().on_close_requested(move || {
        let Some(app) = weak.upgrade() else {
            return slint::CloseRequestResponse::HideWindow;
        };
        if app.global::<DemoState>().get_busy() {
            app.global::<DemoState>()
                .set_status("Wait for the current ZFS operation before quitting".into());
            return slint::CloseRequestResponse::KeepWindowShown;
        }
        if matches!(app.global::<InstallState>().get_state(), 1 | 4) {
            app.invoke_cancel_requested();
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            slint::CloseRequestResponse::HideWindow
        }
    });

    let weak = app.as_weak();
    app.on_quit_requested(move || {
        let Some(app) = weak.upgrade() else { return };
        if app.global::<DemoState>().get_busy() {
            app.global::<DemoState>()
                .set_status("Wait for the current ZFS operation before quitting".into());
            return;
        }
        let _ = app.window().hide();
    });

    let weak = app.as_weak();
    app.on_reboot_requested(move || {
        let Some(app) = weak.upgrade() else { return };
        let _ = app.window().hide();
        let _ = std::process::Command::new("systemctl")
            .arg("reboot")
            .spawn();
    });
}
