pub mod desktop;
pub mod server;

/// A post-installation step to run after packages and services are configured.
#[derive(Debug, Clone)]
pub enum PostInstallStep {
    /// Run a command inside the chroot as a specific system user.
    /// Translates to: `arch-chroot <target> runuser -u <user> -- <cmd> [args…]`
    RunAsUser {
        user: &'static str,
        cmd: &'static str,
        args: &'static [&'static str],
    },
    /// Run a command inside the chroot as root.
    /// Translates to: `arch-chroot <target> <cmd> [args…]`
    RunAsRoot {
        cmd: &'static str,
        args: &'static [&'static str],
    },
    /// Add every user defined in the installer config to a named group.
    AddUsersToGroup { group: &'static str },
}

#[derive(Default)]
pub struct Profile {
    pub name: &'static str,
    pub display_name: &'static str,
    pub packages: Vec<&'static str>,
    /// Optional extras shown in a multi-select checklist after profile selection.
    /// User choices are merged into `GlobalConfig::additional_packages`.
    pub optional_packages: Vec<&'static str>,
    pub services: Vec<&'static str>,
    /// User-level systemd units enabled globally via `systemctl --global enable`.
    /// Primarily used for PipeWire (handled separately by the audio block) and
    /// any compositor-specific user services.
    pub user_services: Vec<&'static str>,
    /// Whether this profile requires seat access configuration (Wayland compositors).
    pub needs_seat_access: bool,
    /// Steps run after packages and services are set up.
    pub post_install_steps: Vec<PostInstallStep>,
}

const DISPLAY_MANAGERS: &[&str] = &["gdm", "sddm", "lightdm", "ly", "cosmic-greeter"];

impl Profile {
    /// Return the display manager service this profile uses, if any.
    pub fn display_manager(&self) -> Option<&str> {
        self.services
            .iter()
            .find(|s| DISPLAY_MANAGERS.contains(s))
            .copied()
    }
}

pub fn get_profile(name: &str) -> Option<Profile> {
    let all = all_profiles();
    all.into_iter().find(|p| p.name == name)
}

pub fn all_profiles() -> Vec<Profile> {
    let mut profiles = Vec::new();
    profiles.extend(desktop::desktop_profiles());
    profiles.extend(server::server_profiles());
    profiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_profile_gnome() {
        let p = get_profile("gnome").unwrap();
        assert_eq!(p.display_name, "GNOME");
        assert!(p.packages.contains(&"gnome"));
    }

    #[test]
    fn test_get_profile_unknown() {
        assert!(get_profile("nonexistent").is_none());
    }

    #[test]
    fn test_all_profiles_nonempty() {
        let profiles = all_profiles();
        assert!(profiles.len() > 5);
    }

    #[test]
    fn test_wayland_profiles_need_seat_access() {
        for name in &["hyprland", "sway", "labwc", "niri", "river"] {
            let p = get_profile(name).unwrap();
            assert!(
                p.needs_seat_access,
                "{name} should have needs_seat_access=true"
            );
        }
    }

    #[test]
    fn test_server_profiles_have_post_install() {
        let pg = get_profile("postgresql").unwrap();
        assert!(
            !pg.post_install_steps.is_empty(),
            "postgresql needs post_install_steps"
        );

        let maria = get_profile("mariadb").unwrap();
        assert!(
            !maria.post_install_steps.is_empty(),
            "mariadb needs post_install_steps"
        );

        let docker = get_profile("docker").unwrap();
        assert!(
            !docker.post_install_steps.is_empty(),
            "docker needs post_install_steps"
        );
    }
}
