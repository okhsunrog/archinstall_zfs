//! Applying wizard edits to the configuration.
//!
//! Both interfaces present the same settings and used to carry their own copy
//! of what each one does — two `apply_text` functions that had already drifted
//! apart, two `apply_radio`s, and a string key matched in each with a
//! catch-all arm that silently swallowed anything unrecognised. A typo in a
//! key produced a control that did nothing, with no compile error and no
//! runtime complaint.
//!
//! The settings are named by enum here, and the edits are applied here. The
//! key strings still exist because the row identity has to survive a trip
//! through the interface toolkit, but they are parsed back into an enum at the
//! boundary, so every match below is exhaustive and a key that does not parse
//! is reported rather than ignored.
//!
//! Settings are grouped by how they are edited — a value chosen from a list, a
//! block device, or free text — because that is what determines the shape of
//! the edit. Rows that open a dedicated editor, and rows that are actions
//! rather than settings, stay with the interface that renders them.

use std::path::Path;

use super::choices::Choice;
use super::types::{
    AudioServer, CompressionAlgo, GlobalConfig, InitSystem, InstallationMode, ProfileSelection,
    SeatAccess, SwapMode, ZfsEncryptionMode,
};

/// Define a settings enum alongside the wire keys the interfaces use for it.
///
/// One table per enum, so the name and the key cannot drift apart, and
/// `parse` is the inverse of `as_str` by construction.
macro_rules! settings {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $key:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// Every setting in this group, for tests and for interfaces that
            /// want to enumerate them.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The key this setting travels under.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $key),+
                }
            }

            /// Recover the setting from a key, or `None` if it names none.
            pub fn parse(key: &str) -> Option<Self> {
                match key {
                    $($key => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

settings! {
    /// A setting picked from a list of alternatives.
    ChoiceSetting {
        InstallationMode => "installation_mode",
        Compression => "compression",
        Encryption => "encryption",
        SwapMode => "swap_mode",
        InitSystem => "init_system",
        Audio => "audio",
        SeatAccess => "seat_access",
        Profile => "profile",
        NetworkCopyIso => "network",
    }
}

settings! {
    /// A setting naming a block device.
    DeviceSetting {
        Disk => "disk",
        EfiPartition => "efi_partition",
        ZfsPartition => "zfs_partition",
        SwapPartition => "swap_partition",
    }
}

settings! {
    /// A setting edited as free text.
    TextSetting {
        PoolName => "pool_name",
        DatasetPrefix => "dataset_prefix",
        Hostname => "hostname",
        RootPassword => "root_password",
        EncryptionPassword => "encryption_password",
        SwapPartitionSize => "swap_partition_size",
        ParallelDownloads => "parallel_downloads",
        AdditionalPackages => "additional_packages",
        AurPackages => "aur_packages",
        ExtraServices => "extra_services",
    }
}

settings! {
    /// A setting turned on and off in place.
    ToggleSetting {
        Ntp => "ntp",
        Bluetooth => "bluetooth",
        Zrepl => "zrepl",
    }
}

settings! {
    /// A setting edited through a dedicated picker rather than in the row.
    ///
    /// Identity only: what the picker looks like, and whether an interface
    /// offers one at all, is that interface's business. Naming them here is
    /// what lets both dispatch on a value the compiler checks instead of on a
    /// string literal repeated in two crates.
    EditorSetting {
        Kernel => "kernel",
        Profile => "profile",
        OptionalPackages => "optional_packages",
        GpuDriver => "gpu_driver",
        DisplayManager => "display_manager",
        Timezone => "timezone",
        Locale => "locale",
        Keyboard => "keyboard",
        Users => "users",
        Packages => "packages",
        // Also a text row: in the mode that installs into an existing pool the
        // terminal interface offers a picker instead of free text, and the row
        // keeps its key either way.
        PoolName => "pool_name",
    }
}

/// Flip a setting that is on or off.
pub fn apply_toggle(config: &mut GlobalConfig, setting: ToggleSetting) {
    match setting {
        ToggleSetting::Ntp => config.ntp = !config.ntp,
        ToggleSetting::Bluetooth => config.bluetooth = !config.bluetooth,
        ToggleSetting::Zrepl => config.zrepl_enabled = !config.zrepl_enabled,
    }
}

/// Apply a choice made at position `index` of the setting's list.
///
/// An index outside the list leaves the configuration untouched: the lists are
/// built from the same tables this resolves against, so it can only mean the
/// two got out of step, and guessing a variant would be worse than doing
/// nothing.
pub fn apply_choice(config: &mut GlobalConfig, setting: ChoiceSetting, index: usize) {
    match setting {
        ChoiceSetting::InstallationMode => {
            let Some(mode) = InstallationMode::from_index(index) else {
                return;
            };
            // Devices chosen under one mode mean nothing under another.
            if config.installation_mode != Some(mode) {
                config.disk = None;
                config.efi_partition = None;
                config.zfs_partition = None;
                config.swap_partition = None;
            }
            config.installation_mode = Some(mode);
        }
        ChoiceSetting::Compression => {
            if let Some(algo) = CompressionAlgo::from_index(index) {
                config.compression = algo;
            }
        }
        ChoiceSetting::Encryption => {
            let Some(mode) = ZfsEncryptionMode::from_index(index) else {
                return;
            };
            config.zfs_encryption_mode = mode;
            if mode == ZfsEncryptionMode::None {
                config.zfs_encryption_password = None;
            }
        }
        ChoiceSetting::SwapMode => {
            if let Some(mode) = SwapMode::from_index(index) {
                config.swap_mode = mode;
            }
        }
        ChoiceSetting::InitSystem => {
            if let Some(init) = InitSystem::from_index(index) {
                config.init_system = init;
            }
        }
        ChoiceSetting::Audio => {
            if let Some(server) = <Option<AudioServer>>::from_index(index) {
                config.audio = server;
            }
        }
        ChoiceSetting::SeatAccess => {
            if let (Some(selection), Some(access)) = (
                config.profile_selection.as_mut(),
                <Option<SeatAccess>>::from_index(index),
            ) {
                selection.seat_access = access;
            }
        }
        ChoiceSetting::Profile => {
            // Position 0 is "no profile"; the registry follows.
            config.profile_selection = index
                .checked_sub(1)
                .and_then(|i| crate::profile::all_profiles().get(i).map(|p| p.name))
                .and_then(ProfileSelection::new);
        }
        ChoiceSetting::NetworkCopyIso => config.network_copy_iso = index == 0,
    }
}

/// Apply a block device selection.
pub fn apply_device(config: &mut GlobalConfig, setting: DeviceSetting, path: &Path) {
    let path = path.to_path_buf();
    match setting {
        DeviceSetting::Disk => {
            // Choosing a disk is what puts the wizard in full-disk mode.
            config.installation_mode = Some(InstallationMode::FullDisk);
            config.disk = Some(path);
        }
        DeviceSetting::EfiPartition => config.efi_partition = Some(path),
        DeviceSetting::ZfsPartition => config.zfs_partition = Some(path),
        DeviceSetting::SwapPartition => config.swap_partition = Some(path),
    }
}

/// Apply text typed into a setting.
///
/// Empty clears the optional settings, which is how a user removes a value
/// they have already entered. The two that cannot be empty — the dataset
/// prefix and the download count — keep their previous value instead.
pub fn apply_text(config: &mut GlobalConfig, setting: TextSetting, value: &str) {
    let text = (!value.is_empty()).then(|| value.to_string());

    match setting {
        TextSetting::PoolName => config.pool_name = text,
        TextSetting::Hostname => config.hostname = text,
        TextSetting::RootPassword => config.root_password = text,
        TextSetting::EncryptionPassword => config.zfs_encryption_password = text,
        TextSetting::SwapPartitionSize => config.swap_partition_size = text,
        TextSetting::DatasetPrefix => {
            if let Some(prefix) = text {
                config.dataset_prefix = prefix;
            }
        }
        TextSetting::ParallelDownloads => {
            if let Ok(count) = value.parse::<u32>() {
                config.parallel_downloads = count.clamp(1, 20);
            }
        }
        TextSetting::AdditionalPackages => config.additional_packages = package_list(value),
        TextSetting::AurPackages => config.aur_packages = package_list(value),
        TextSetting::ExtraServices => config.extra_services = package_list(value),
    }
}

/// Split a whitespace- or comma-separated list of names.
fn package_list(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(|name| name.trim_matches(',').to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GlobalConfig {
        GlobalConfig::default()
    }

    /// Keys round-trip, and no two settings share one.
    #[test]
    fn keys_are_unique_and_reversible() {
        fn check<T: std::fmt::Debug + Copy + PartialEq>(
            all: &[T],
            as_str: impl Fn(T) -> &'static str,
            parse: impl Fn(&str) -> Option<T>,
        ) {
            let mut seen: Vec<&str> = Vec::new();
            for setting in all {
                let key = as_str(*setting);
                assert_eq!(
                    parse(key),
                    Some(*setting),
                    "{setting:?} does not round-trip"
                );
                assert!(!seen.contains(&key), "{key} is used by two settings");
                seen.push(key);
            }
            assert_eq!(parse("no-such-key"), None);
        }

        check(
            ChoiceSetting::ALL,
            ChoiceSetting::as_str,
            ChoiceSetting::parse,
        );
        check(
            DeviceSetting::ALL,
            DeviceSetting::as_str,
            DeviceSetting::parse,
        );
        check(TextSetting::ALL, TextSetting::as_str, TextSetting::parse);
    }

    #[test]
    fn changing_installation_mode_clears_devices_chosen_for_the_old_one() {
        let mut c = cfg();
        apply_device(&mut c, DeviceSetting::Disk, Path::new("/dev/sda"));
        apply_device(&mut c, DeviceSetting::EfiPartition, Path::new("/dev/sda1"));

        apply_choice(&mut c, ChoiceSetting::InstallationMode, 1);

        assert_eq!(c.installation_mode, Some(InstallationMode::NewPool));
        assert!(c.disk.is_none());
        assert!(c.efi_partition.is_none());
    }

    #[test]
    fn reselecting_the_same_mode_keeps_the_devices() {
        let mut c = cfg();
        apply_choice(&mut c, ChoiceSetting::InstallationMode, 0);
        apply_device(&mut c, DeviceSetting::Disk, Path::new("/dev/sda"));

        apply_choice(&mut c, ChoiceSetting::InstallationMode, 0);

        assert_eq!(c.disk.as_deref(), Some(Path::new("/dev/sda")));
    }

    #[test]
    fn turning_encryption_off_discards_the_passphrase() {
        let mut c = cfg();
        apply_choice(&mut c, ChoiceSetting::Encryption, 1);
        apply_text(&mut c, TextSetting::EncryptionPassword, "hunter2hunter2");
        assert!(c.zfs_encryption_password.is_some());

        apply_choice(&mut c, ChoiceSetting::Encryption, 0);

        assert_eq!(c.zfs_encryption_mode, ZfsEncryptionMode::None);
        assert!(
            c.zfs_encryption_password.is_none(),
            "a passphrase must not outlive the encryption it was for"
        );
    }

    #[test]
    fn an_index_outside_the_list_changes_nothing() {
        let mut c = cfg();
        let before = c.compression;

        apply_choice(&mut c, ChoiceSetting::Compression, 99);

        assert_eq!(c.compression, before);
    }

    #[test]
    fn choosing_a_disk_selects_full_disk_mode() {
        let mut c = cfg();
        apply_device(&mut c, DeviceSetting::Disk, Path::new("/dev/disk/by-id/x"));

        assert_eq!(c.installation_mode, Some(InstallationMode::FullDisk));
        assert_eq!(c.disk.as_deref(), Some(Path::new("/dev/disk/by-id/x")));
    }

    #[test]
    fn empty_text_clears_optional_settings() {
        let mut c = cfg();
        apply_text(&mut c, TextSetting::Hostname, "box");
        assert_eq!(c.hostname.as_deref(), Some("box"));

        apply_text(&mut c, TextSetting::Hostname, "");

        assert!(c.hostname.is_none());
    }

    #[test]
    fn settings_that_cannot_be_empty_keep_their_value() {
        let mut c = cfg();
        apply_text(&mut c, TextSetting::DatasetPrefix, "arch1");

        apply_text(&mut c, TextSetting::DatasetPrefix, "");

        assert_eq!(c.dataset_prefix, "arch1");
    }

    #[test]
    fn the_download_count_is_clamped_and_garbage_is_ignored() {
        let mut c = cfg();

        apply_text(&mut c, TextSetting::ParallelDownloads, "999");
        assert_eq!(c.parallel_downloads, 20);

        apply_text(&mut c, TextSetting::ParallelDownloads, "0");
        assert_eq!(c.parallel_downloads, 1);

        apply_text(&mut c, TextSetting::ParallelDownloads, "eight");
        assert_eq!(c.parallel_downloads, 1, "unparsable input is not a change");
    }

    #[test]
    fn package_lists_accept_spaces_and_commas() {
        let mut c = cfg();

        apply_text(&mut c, TextSetting::AdditionalPackages, "vim, git  htop,");

        assert_eq!(c.additional_packages, vec!["vim", "git", "htop"]);
    }

    #[test]
    fn toggles_flip() {
        let mut c = cfg();
        let was = c.ntp;

        apply_toggle(&mut c, ToggleSetting::Ntp);
        assert_eq!(c.ntp, !was);

        apply_toggle(&mut c, ToggleSetting::Ntp);
        assert_eq!(c.ntp, was);
    }

    #[test]
    fn each_toggle_moves_only_its_own_setting() {
        let mut c = cfg();
        apply_toggle(&mut c, ToggleSetting::Bluetooth);

        assert!(c.bluetooth);
        assert!(!c.zrepl_enabled);
        assert!(c.ntp, "ntp defaults on and was not touched");
    }

    #[test]
    fn profile_position_zero_means_no_profile() {
        let mut c = cfg();
        apply_choice(&mut c, ChoiceSetting::Profile, 1);
        assert!(c.profile_selection.is_some());

        apply_choice(&mut c, ChoiceSetting::Profile, 0);

        assert!(c.profile_selection.is_none());
    }
}
