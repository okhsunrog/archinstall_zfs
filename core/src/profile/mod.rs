pub mod desktop;
pub mod server;

pub struct Profile {
    pub name: &'static str,
    pub display_name: &'static str,
    pub packages: Vec<&'static str>,
    pub services: Vec<&'static str>,
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
}
