//! What differs between the distributions this installer can install.
//!
//! Everything here used to be constants scattered through the installer: the
//! repository to add for ZFS, the keys to trust for it, the argument to
//! `pacman-key --populate`, the packages a base system starts from. Each was
//! written for Arch, which was fine while Arch was the only answer.
//!
//! A distribution is data, so a second one is an entry rather than a branch.

/// How much pacman verifies of what a repository serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signatures {
    /// Packages and database are taken on trust.
    Never,
    /// Verified when a signature is present, accepted when it is not.
    Optional,
    /// Every package must carry a valid signature.
    Required,
}

/// A repository added to pacman.conf beyond what the medium already carries.
#[derive(Debug, Clone, Copy)]
pub struct Repository {
    pub name: &'static str,
    /// Servers written into the repository's block. Empty when `mirrorlist`
    /// supplies them instead.
    pub servers: &'static [&'static str],
    /// A mirrorlist to `Include`, for distributions that ship one as a
    /// package. Its packages must be installed before the repository is
    /// usable, which is what `bootstrap_packages` is for.
    pub mirrorlist: Option<&'static str>,
    /// Keys received and locally signed before the repository is used.
    pub key_ids: &'static [&'static str],
    pub signatures: Signatures,
}

impl Repository {
    /// The block this repository contributes to pacman.conf.
    pub fn pacman_conf_block(&self) -> String {
        let mut block = format!("\n[{}]\n", self.name);
        match self.signatures {
            Signatures::Never => block.push_str("SigLevel = Never\n"),
            Signatures::Optional => block.push_str("SigLevel = Optional TrustAll\n"),
            Signatures::Required => block.push_str("SigLevel = Required DatabaseOptional\n"),
        }
        if let Some(mirrorlist) = self.mirrorlist {
            block.push_str(&format!("Include = {mirrorlist}\n"));
        }
        for server in self.servers {
            block.push_str(&format!("Server = {server}\n"));
        }
        block
    }
}

/// A distribution the installer can install.
#[derive(Debug, Clone, Copy)]
pub struct Distribution {
    /// Identifier used in configuration files.
    pub name: &'static str,
    pub display_name: &'static str,
    /// What a base installation starts from, before kernels, initramfs and
    /// microcode are added.
    pub base_packages: &'static [&'static str],
    /// Repositories to add, in the order they should appear. Order matters:
    /// pacman prefers the first repository that offers a package.
    pub repositories: &'static [Repository],
    /// The keyring `pacman-key --populate` is given.
    pub keyring: &'static str,
}

/// Where the ZFS module packages for Arch kernels come from.
///
/// KNOWN GAP: `Signatures::Never`. The archzfs experimental channel's signing
/// is not reliable enough to gate installations on, so its packages are
/// installed on the strength of the sync database's checksums alone, and the
/// keys below go unused. Revisit together with the `.sig` fetch in
/// `system::async_download` — the two have to change as one.
const ARCHZFS: Repository = Repository {
    name: "archzfs",
    servers: &["https://github.com/archzfs/archzfs/releases/download/experimental"],
    mirrorlist: None,
    key_ids: &[
        "3A9917BF0DED5C13F69AC68FABEC0A1208037BE9",
        "DDF7DB817396A49B2A2723F7403BD972F75D9D76",
    ],
    signatures: Signatures::Never,
};

pub const ARCH: Distribution = Distribution {
    name: "arch",
    display_name: "Arch Linux",
    base_packages: &[
        "base",
        "base-devel",
        "linux-firmware",
        "linux-firmware-marvell",
        "sof-firmware",
    ],
    repositories: &[ARCHZFS],
    keyring: "archlinux",
};

/// Every distribution the installer knows.
pub const ALL: &[Distribution] = &[ARCH];

/// Look a distribution up by the name a configuration file uses.
pub fn get(name: &str) -> Option<&'static Distribution> {
    ALL.iter().find(|distro| distro.name == name)
}

/// The distribution assumed when a configuration does not name one.
pub fn default() -> &'static Distribution {
    &ALL[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distributions_are_found_by_name() {
        assert_eq!(get("arch").map(|d| d.display_name), Some("Arch Linux"));
        assert!(get("plan9").is_none());
        assert_eq!(default().name, "arch");
    }

    #[test]
    fn names_are_unique() {
        for (index, distro) in ALL.iter().enumerate() {
            assert!(
                !ALL[..index].iter().any(|other| other.name == distro.name),
                "{} appears twice",
                distro.name
            );
        }
    }

    #[test]
    fn a_repository_with_servers_writes_them_out() {
        let block = ARCHZFS.pacman_conf_block();

        assert!(block.starts_with("\n[archzfs]\n"), "got: {block}");
        assert!(block.contains("SigLevel = Never\n"));
        assert!(block.contains("Server = https://github.com/archzfs/"));
        assert!(!block.contains("Include ="), "no mirrorlist for this one");
    }

    #[test]
    fn a_repository_with_a_mirrorlist_includes_it() {
        let repo = Repository {
            name: "cachyos",
            servers: &[],
            mirrorlist: Some("/etc/pacman.d/cachyos-mirrorlist"),
            key_ids: &[],
            signatures: Signatures::Required,
        };

        let block = repo.pacman_conf_block();

        assert!(block.contains("Include = /etc/pacman.d/cachyos-mirrorlist\n"));
        assert!(block.contains("SigLevel = Required DatabaseOptional\n"));
        assert!(!block.contains("Server ="));
    }
}
