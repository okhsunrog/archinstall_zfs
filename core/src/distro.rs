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
    /// Whether this distribution serves packages built for instruction sets
    /// newer than the base architecture.
    ///
    /// pacman installs those only when told which architectures to accept.
    /// CachyOS's own tooling writes `Architecture = auto` and relies on a
    /// patched pacman to expand it; stock pacman expands `auto` to the output
    /// of `uname -m` and rejects everything else, so the accepted
    /// architectures are listed out instead.
    pub optimised_builds: bool,
    /// The keyring `pacman-key --populate` is given.
    pub keyring: &'static str,
    /// The kernels this distribution offers, and where each one's ZFS module
    /// comes from.
    pub kernels: &'static [KernelInfo],
    /// The package providing ZFSBootMenu, when the distribution has one.
    /// `None` means building it from the AUR, which is what Arch needs.
    pub zfsbootmenu_package: Option<&'static str>,
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
    optimised_builds: false,
    keyring: "archlinux",
    kernels: ARCH_KERNELS,
    zfsbootmenu_package: None,
};

impl Distribution {
    /// The architectures pacman should accept on this machine, when that
    /// needs saying at all.
    pub fn architectures(&self, isa: IsaLevel) -> Option<&'static str> {
        if !self.optimised_builds {
            return None;
        }
        match isa {
            // Zen 4 packages are built as x86-64-v4, so both baselines answer
            // with the same list.
            IsaLevel::V4 | IsaLevel::Znver4 => Some("x86_64 x86_64_v3 x86_64_v4"),
            IsaLevel::V3 => Some("x86_64 x86_64_v3"),
            IsaLevel::Baseline => None,
        }
    }

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

/// Where CachyOS serves each build of its package set.
///
/// The directory is the instruction set the packages were built for, and it is
/// not the repository's own name: the Zen 4 repositories live under the
/// x86-64-v4 directory, and a path built from the repository name instead
/// answers with the mirror's welcome page rather than a database — which
/// pacman then reports as a corrupt signature.
///
/// Their mirrorlist writes these as `$arch_v3` and `$arch_v4`, variables only
/// their patched pacman understands, so the paths are written out here.
const CACHYOS_BASELINE_SERVERS: &[&str] = &[
    "https://cdn77.cachyos.org/repo/x86_64/$repo",
    "https://mirror.cachyos.org/repo/x86_64/$repo",
];
const CACHYOS_V3_SERVERS: &[&str] = &[
    "https://cdn77.cachyos.org/repo/x86_64_v3/$repo",
    "https://mirror.cachyos.org/repo/x86_64_v3/$repo",
];
const CACHYOS_V4_SERVERS: &[&str] = &[
    "https://cdn77.cachyos.org/repo/x86_64_v4/$repo",
    "https://mirror.cachyos.org/repo/x86_64_v4/$repo",
];

/// Their packages are signed with one key, trusted before the repositories are
/// used — the same order their own installer script follows.
const CACHYOS_KEY: &str = "F3B607488DB35A47";

/// One of CachyOS's repositories. They share a key and differ in where they
/// are served from.
const fn cachyos_repo(name: &'static str, servers: &'static [&'static str]) -> Repository {
    Repository {
        name,
        servers,
        // Their mirrorlist cannot be included while installing: the file
        // arrives with a package, and pacman refuses a configuration that
        // includes a file it cannot read. The installed system gets those
        // packages and can use them afterwards.
        mirrorlist: None,
        key_ids: &[CACHYOS_KEY],
        signatures: Signatures::Required,
    }
}

const CACHYOS_V3: &[Repository] = &[
    cachyos_repo("cachyos-v3", CACHYOS_V3_SERVERS),
    cachyos_repo("cachyos-core-v3", CACHYOS_V3_SERVERS),
    cachyos_repo("cachyos-extra-v3", CACHYOS_V3_SERVERS),
    cachyos_repo("cachyos", CACHYOS_BASELINE_SERVERS),
];

const CACHYOS_V4: &[Repository] = &[
    cachyos_repo("cachyos-v4", CACHYOS_V4_SERVERS),
    cachyos_repo("cachyos-core-v4", CACHYOS_V4_SERVERS),
    cachyos_repo("cachyos-extra-v4", CACHYOS_V4_SERVERS),
    cachyos_repo("cachyos", CACHYOS_BASELINE_SERVERS),
];

/// Zen 4 packages are served from the x86-64-v4 directory, not one named
/// after the repository.
const CACHYOS_ZNVER4: &[Repository] = &[
    cachyos_repo("cachyos-znver4", CACHYOS_V4_SERVERS),
    cachyos_repo("cachyos-core-znver4", CACHYOS_V4_SERVERS),
    cachyos_repo("cachyos-extra-znver4", CACHYOS_V4_SERVERS),
    cachyos_repo("cachyos", CACHYOS_BASELINE_SERVERS),
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
    optimised_builds: true,
    keyring: "archlinux",
    kernels: CACHYOS_KERNELS,
    // Theirs is packaged, so there is nothing to build.
    zfsbootmenu_package: Some("zfsbootmenu"),
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

    /// The directory a repository is served from is the instruction set, not
    /// the repository name — getting this wrong answers with the mirror's
    /// welcome page, which pacman reports as a corrupt database signature.
    #[test]
    fn repositories_are_served_from_the_directory_for_their_build() {
        let dir_of = |repos: &'static [Repository], name: &str| {
            repos
                .iter()
                .find(|r| r.name == name)
                .and_then(|r| r.servers.first().copied())
                .unwrap_or_default()
        };

        assert!(dir_of(CACHYOS_V3, "cachyos-v3").contains("/x86_64_v3/"));
        assert!(dir_of(CACHYOS_V4, "cachyos-v4").contains("/x86_64_v4/"));
        assert!(
            dir_of(CACHYOS_ZNVER4, "cachyos-znver4").contains("/x86_64_v4/"),
            "Zen 4 is served from the v4 directory"
        );
        // The unoptimised repository sits in the plain directory in every set.
        for set in [CACHYOS_V3, CACHYOS_V4, CACHYOS_ZNVER4] {
            assert!(dir_of(set, "cachyos").contains("/x86_64/"));
        }
    }

    #[test]
    fn accepted_architectures_match_what_the_packages_are_built_as() {
        // Their packages carry x86_64_v3 and x86_64_v4; Zen 4 builds are
        // stamped x86_64_v4 like the rest of that baseline.
        assert_eq!(
            CACHYOS.architectures(IsaLevel::V3),
            Some("x86_64 x86_64_v3")
        );
        assert_eq!(
            CACHYOS.architectures(IsaLevel::V4),
            Some("x86_64 x86_64_v3 x86_64_v4")
        );
        assert_eq!(
            CACHYOS.architectures(IsaLevel::Znver4),
            CACHYOS.architectures(IsaLevel::V4)
        );
        assert_eq!(CACHYOS.architectures(IsaLevel::Baseline), None);

        // A distribution without optimised builds says nothing about it.
        for isa in [IsaLevel::V3, IsaLevel::V4, IsaLevel::Znver4] {
            assert_eq!(ARCH.architectures(isa), None);
        }
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
