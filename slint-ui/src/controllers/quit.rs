//! Quit / reboot handlers for the GUI.

use slint::ComponentHandle;

use crate::ui::App;

pub fn setup(app: &App) {
    let weak = app.as_weak();
    app.on_quit_requested(move || {
        let Some(app) = weak.upgrade() else { return };
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
