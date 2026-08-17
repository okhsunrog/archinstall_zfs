//! What differs between the distributions this installer can install.
//!
//! Everything here used to be constants scattered through the installer: the
//! repository to add for ZFS, the keys to trust for it, the argument to
//! `pacman-key --populate`, the packages a base system starts from. Each was
//! written for Arch, which was fine while Arch was the only answer.
//!
//! A distribution is data, so a second one is an entry rather than a branch.

use crate::kernel::KernelInfo;
use crate::system::sysinfo::IsaLevel;

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

/// Which repositories a distribution wants added.
#[derive(Debug, Clone, Copy)]
pub enum RepositorySelection {
    /// The same everywhere.
    Fixed(&'static [Repository]),
    /// Chosen by what the processor supports, because the distribution serves
    /// a different build of the same packages for each baseline.
    ByIsaLevel {
        v3: &'static [Repository],
        v4: &'static [Repository],
        znver4: &'static [Repository],
    },
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
    pub repositories: RepositorySelection,
    /// Value for pacman's `Architecture`, when the distribution needs one
    /// other than the default. CachyOS serves packages built for a newer
    /// instruction set, which pacman only accepts as `auto`.
    pub architecture: Option<&'static str>,
    /// The keyring `pacman-key --populate` is given.
    pub keyring: &'static str,
    /// The kernels this distribution offers, and where each one's ZFS module
    /// comes from.
    pub kernels: &'static [KernelInfo],
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

/// Arch's own kernels, each with the archzfs module built for it.
const ARCH_KERNELS: &[KernelInfo] = &[
    KernelInfo {
        name: "linux-lts",
        display_name: "Linux LTS",
        precompiled_package: Some("zfs-linux-lts"),
        headers_package: "linux-lts-headers",
    },
    KernelInfo {
        name: "linux",
        display_name: "Linux",
        precompiled_package: Some("zfs-linux"),
        headers_package: "linux-headers",
    },
    KernelInfo {
        name: "linux-zen",
        display_name: "Linux Zen",
        precompiled_package: Some("zfs-linux-zen"),
        headers_package: "linux-zen-headers",
    },
    KernelInfo {
        name: "linux-hardened",
        display_name: "Linux Hardened",
        precompiled_package: Some("zfs-linux-hardened"),
        headers_package: "linux-hardened-headers",
    },
];

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
    repositories: RepositorySelection::Fixed(&[ARCHZFS]),
    architecture: None,
    keyring: "archlinux",
    kernels: ARCH_KERNELS,
};

impl Distribution {
    /// The repositories to add on a machine with this instruction set.
    ///
    /// Empty when the distribution has nothing to offer this processor:
    /// CachyOS's repositories start at x86-64-v3, and on anything older its
    /// own tooling adds none either.
    pub fn repositories(&self, isa: IsaLevel) -> &'static [Repository] {
        match self.repositories {
            RepositorySelection::Fixed(repos) => repos,
            RepositorySelection::ByIsaLevel { v3, v4, znver4 } => match isa {
                IsaLevel::Znver4 => znver4,
                IsaLevel::V4 => v4,
                IsaLevel::V3 => v3,
                IsaLevel::Baseline => &[],
            },
        }
    }
}

/// CachyOS serves its packages from its own mirror, addressed by repository
/// name. Their mirrorlist package names the same mirrors and is installed as
/// part of the system, but a `Include` of it cannot be used while installing:
/// the file does not exist until the package is, and pacman refuses a
/// configuration that includes a file it cannot read.
const CACHYOS_SERVER: &str = "https://mirror.cachyos.org/repo/x86_64/$repo";

/// Their packages are signed with one key, which is trusted before the
/// repositories are used — the same order their own installer script follows.
const CACHYOS_KEY: &str = "F3B607488DB35A47";

/// One of CachyOS's repositories. They all share a server and a key.
const fn cachyos_repo(name: &'static str) -> Repository {
    Repository {
        name,
        servers: &[CACHYOS_SERVER],
        mirrorlist: None,
        key_ids: &[CACHYOS_KEY],
        signatures: Signatures::Required,
    }
}

const CACHYOS_V3: &[Repository] = &[
    cachyos_repo("cachyos-v3"),
    cachyos_repo("cachyos-core-v3"),
    cachyos_repo("cachyos-extra-v3"),
    cachyos_repo("cachyos"),
];

const CACHYOS_V4: &[Repository] = &[
    cachyos_repo("cachyos-v4"),
    cachyos_repo("cachyos-core-v4"),
    cachyos_repo("cachyos-extra-v4"),
    cachyos_repo("cachyos"),
];

const CACHYOS_ZNVER4: &[Repository] = &[
    cachyos_repo("cachyos-znver4"),
    cachyos_repo("cachyos-core-znver4"),
    cachyos_repo("cachyos-extra-znver4"),
    cachyos_repo("cachyos"),
];

/// CachyOS builds a ZFS module for each of its kernels, version-locked to it.
/// The kernel itself cannot carry ZFS — the CDDL and the GPL do not permit
/// distributing that — so this is the same shape as archzfs, under their
/// names.
const CACHYOS_KERNELS: &[KernelInfo] = &[
    KernelInfo {
        name: "linux-cachyos",
        display_name: "CachyOS (BORE + sched-ext)",
        precompiled_package: Some("linux-cachyos-zfs"),
        headers_package: "linux-cachyos-headers",
    },
    KernelInfo {
        name: "linux-cachyos-lts",
        display_name: "CachyOS LTS",
        precompiled_package: Some("linux-cachyos-lts-zfs"),
        headers_package: "linux-cachyos-lts-headers",
    },
    KernelInfo {
        name: "linux-cachyos-bore",
        display_name: "CachyOS BORE",
        precompiled_package: Some("linux-cachyos-bore-zfs"),
        headers_package: "linux-cachyos-bore-headers",
    },
    KernelInfo {
        name: "linux-cachyos-deckify",
        display_name: "CachyOS Deckify",
        precompiled_package: Some("linux-cachyos-deckify-zfs"),
        headers_package: "linux-cachyos-deckify-headers",
    },
];

pub const CACHYOS: Distribution = Distribution {
    name: "cachyos",
    display_name: "CachyOS",
    base_packages: &[
        "base",
        "base-devel",
        "linux-firmware",
        "linux-firmware-marvell",
        "sof-firmware",
        // Their keyring and mirrorlists belong on the installed system, which
        // is what lets it reach their repositories on its own afterwards.
        "cachyos-keyring",
        "cachyos-mirrorlist",
        "cachyos-v3-mirrorlist",
        "cachyos-v4-mirrorlist",
        "cachyos-settings",
    ],
    repositories: RepositorySelection::ByIsaLevel {
        v3: CACHYOS_V3,
        v4: CACHYOS_V4,
        znver4: CACHYOS_ZNVER4,
    },
    architecture: Some("auto"),
    keyring: "archlinux",
    kernels: CACHYOS_KERNELS,
};

/// Every distribution the installer knows.
pub const ALL: &[Distribution] = &[ARCH, CACHYOS];

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
    fn cachyos_serves_a_different_build_per_processor() {
        let v3 = CACHYOS.repositories(IsaLevel::V3);
        let v4 = CACHYOS.repositories(IsaLevel::V4);
        let zen = CACHYOS.repositories(IsaLevel::Znver4);

        assert!(v3.iter().any(|r| r.name == "cachyos-v3"));
        assert!(v4.iter().any(|r| r.name == "cachyos-v4"));
        assert!(zen.iter().any(|r| r.name == "cachyos-znver4"));

        // The unoptimised repository is in every set: it carries what is not
        // rebuilt per baseline.
        for set in [v3, v4, zen] {
            assert!(
                set.iter().any(|r| r.name == "cachyos"),
                "plain repo missing"
            );
            assert_eq!(set.len(), 4);
        }

        // A processor below the baseline gets nothing, as with their own
        // tooling; add_repositories turns that into a refusal.
        assert!(CACHYOS.repositories(IsaLevel::Baseline).is_empty());
    }

    #[test]
    fn a_fixed_selection_ignores_the_processor() {
        for isa in [
            IsaLevel::Baseline,
            IsaLevel::V3,
            IsaLevel::V4,
            IsaLevel::Znver4,
        ] {
            assert_eq!(ARCH.repositories(isa).len(), 1);
        }
    }

    #[test]
    fn every_cachyos_kernel_has_its_own_zfs_module() {
        for kernel in CACHYOS.kernels {
            let module = kernel
                .precompiled_package
                .expect("CachyOS builds a module for each of its kernels");
            assert_eq!(
                module,
                format!("{}-zfs", kernel.name),
                "the module is named after the kernel it is built for"
            );
            assert_eq!(kernel.headers_package, format!("{}-headers", kernel.name));
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
