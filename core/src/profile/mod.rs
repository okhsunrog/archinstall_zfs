//! Profile registry.
//!
//! A `Profile` is a named bundle of packages, services, and post-install
//! steps. Behavior that is specific to a *kind* of profile (desktop vs server
//! vs minimal) lives inside `ProfileKind` so that fields are only present
//! where they actually apply.
//!
//! ## Design notes
//!
//! - Common fields (name, display_name, packages, services, …) live on
//!   `Profile`.
//! - Kind-specific fields (display manager, seat access, optional packages,
//!   sddm session) live inside `ProfileKind::Desktop(DesktopProfile)`. Server
//!   and minimal profiles carry no extra data.
//! - Display managers are typed via the `DisplayManager` enum rather than
//!   stringly-typed inside `services`. The DM is *not* duplicated in
//!   `services` — `install_profile` enables it explicitly via
//!   `DisplayManager::service()`.
//! - User-visible selection state lives in
//!   `crate::config::types::ProfileSelection`, not on `Profile`.

pub mod desktop;
pub mod server;

use serde::{Deserialize, Serialize};

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

/// Top-level profile definition. Common fields apply to every kind; the
/// `kind` discriminant carries kind-specific data.
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: &'static str,
    pub display_name: &'static str,
    /// Short user-facing description. May be empty.
    pub description: &'static str,
    pub packages: Vec<&'static str>,
    pub services: Vec<&'static str>,
    /// User-level systemd units enabled globally via `systemctl --global enable`.
    pub user_services: Vec<&'static str>,
    /// Steps run after packages and services are set up.
    pub post_install_steps: Vec<PostInstallStep>,
    pub kind: ProfileKind,
}

#[derive(Debug, Clone)]
pub enum ProfileKind {
    /// Bare minimal install — no extras beyond the base system.
    Minimal,
    /// Headless / server profile.
    Server,
    /// Graphical desktop profile.
    Desktop(DesktopProfile),
}

#[derive(Debug, Clone)]
pub struct DesktopProfile {
    /// Which display server protocol the profile expects to run under.
    pub display_server: DisplayServer,
    /// Display manager shipped with the profile, if any.
    /// Some Wayland compositors are launched directly without a DM.
    pub default_display_manager: Option<DisplayManager>,
    /// True for compositors that need explicit seat access (typically
    /// Wayland compositors that don't go through logind/polkit).
    pub needs_seat_access: bool,
    /// Session name written into SDDM's autologin config (.desktop stem).
    /// Only consulted when SDDM is the active DM and a user has autologin.
    pub sddm_session: Option<&'static str>,
    /// Optional extras shown in a multi-select checklist after the user
    /// picks this profile.
    pub optional_packages: Vec<OptionalPackage>,
}

/// Display server protocol(s) supported by a desktop profile.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayServer {
    Xorg,
    Wayland,
    /// Profile supports both (e.g. KDE Plasma, GNOME).
    Both,
}

/// Display manager enum.
///
/// Replaces the previous string-based representation. The `service()` and
/// `package()` helpers map each variant to its arch package / systemd unit.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayManager {
    Gdm,
    Sddm,
    Lightdm,
    Ly,
    CosmicGreeter,
}

impl DisplayManager {
    /// Systemd unit name (without `.service` suffix).
    pub const fn service(self) -> &'static str {
        match self {
            Self::Gdm => "gdm",
            Self::Sddm => "sddm",
            Self::Lightdm => "lightdm",
            Self::Ly => "ly",
            Self::CosmicGreeter => "cosmic-greeter",
        }
    }

    /// Arch package name. Currently identical to `service()` for every
    /// known DM, but kept distinct so future divergences are localised here.
    pub const fn package(self) -> &'static str {
        self.service()
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Gdm => "GDM",
            Self::Sddm => "SDDM",
            Self::Lightdm => "LightDM",
            Self::Ly => "Ly",
            Self::CosmicGreeter => "COSMIC Greeter",
        }
    }

    pub const ALL: &'static [DisplayManager] = &[
        Self::Gdm,
        Self::Sddm,
        Self::Lightdm,
        Self::Ly,
        Self::CosmicGreeter,
    ];
}

impl std::fmt::Display for DisplayManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.service())
    }
}

/// Optional package with a user-facing description.
///
/// Description may be empty for entries we haven't documented yet.
#[derive(Debug, Clone)]
pub struct OptionalPackage {
    pub package: &'static str,
    pub description: &'static str,
}

impl OptionalPackage {
    pub const fn new(package: &'static str) -> Self {
        Self {
            package,
            description: "",
        }
    }

    pub const fn with_desc(package: &'static str, description: &'static str) -> Self {
        Self {
            package,
            description,
        }
    }
}

// ── Profile accessors ──────────────────────────────────────────────────────

impl Profile {
    /// Returns the desktop-specific data, if this profile is a desktop.
    pub fn desktop(&self) -> Option<&DesktopProfile> {
        match &self.kind {
            ProfileKind::Desktop(d) => Some(d),
            _ => None,
        }
    }

    pub fn is_desktop(&self) -> bool {
        matches!(self.kind, ProfileKind::Desktop(_))
    }

    /// DM shipped with the profile, if any. None for non-desktop profiles or
    /// compositors launched without a DM.
    pub fn default_display_manager(&self) -> Option<DisplayManager> {
        self.desktop().and_then(|d| d.default_display_manager)
    }

    pub fn needs_seat_access(&self) -> bool {
        self.desktop().is_some_and(|d| d.needs_seat_access)
    }

    pub fn optional_packages(&self) -> &[OptionalPackage] {
        self.desktop()
            .map(|d| d.optional_packages.as_slice())
            .unwrap_or(&[])
    }

    pub fn sddm_session(&self) -> Option<&'static str> {
        self.desktop().and_then(|d| d.sddm_session)
    }
}

pub fn get_profile(name: &str) -> Option<Profile> {
    all_profiles().into_iter().find(|p| p.name == name)
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
        assert_eq!(p.default_display_manager(), Some(DisplayManager::Gdm));
        assert!(p.is_desktop());
    }

    #[test]
    fn test_get_profile_unknown() {
        assert!(get_profile("nonexistent").is_none());
    }

    #[test]
    fn test_all_profiles_nonempty() {
        assert!(all_profiles().len() > 5);
    }

    #[test]
    fn test_wayland_profiles_need_seat_access() {
        for name in &["hyprland", "sway", "labwc", "niri", "river"] {
            let p = get_profile(name).unwrap();
            assert!(
                p.needs_seat_access(),
                "{name} should have needs_seat_access=true"
            );
        }
    }

    #[test]
    fn test_minimal_is_not_desktop() {
        let m = get_profile("minimal").unwrap();
        assert!(!m.is_desktop());
        assert!(!m.needs_seat_access());
        assert_eq!(m.default_display_manager(), None);
        assert!(m.optional_packages().is_empty());
    }

    #[test]
    fn test_server_profiles_have_post_install() {
        for name in &["postgresql", "mariadb", "docker"] {
            let p = get_profile(name).unwrap();
            assert!(
                !p.post_install_steps.is_empty(),
                "{name} needs post_install_steps"
            );
            assert!(matches!(p.kind, ProfileKind::Server));
        }
    }

    #[test]
    fn test_display_manager_service_mapping() {
        assert_eq!(DisplayManager::Gdm.service(), "gdm");
        assert_eq!(DisplayManager::CosmicGreeter.service(), "cosmic-greeter");
    }

    #[test]
    fn test_cosmic_uses_cosmic_greeter() {
        let p = get_profile("cosmic").unwrap();
        assert_eq!(
            p.default_display_manager(),
            Some(DisplayManager::CosmicGreeter)
        );
    }

    #[test]
    fn test_kde_sddm_session() {
        let p = get_profile("kde").unwrap();
        assert_eq!(p.default_display_manager(), Some(DisplayManager::Sddm));
        assert_eq!(p.sddm_session(), Some("plasma"));
    }
}
