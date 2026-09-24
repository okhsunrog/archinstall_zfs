//! What differs between the distributions this installer can install.
//!
//! Everything here used to be constants scattered through the installer: the
//! repository to add for ZFS, the keys to trust for it, the argument to
//! `pacman-key --populate`, the packages a base system starts from. Each was
//! written for Arch, which was fine while Arch was the only answer.
//!
//! A distribution is data, so a second one is an entry rather than a branch.

use crate::config::types::InitSystem;
use crate::kernel::KernelInfo;
use crate::system::sysinfo::{CpuVendor, IsaLevel};

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
    /// The kernels this distribution offers, and where each one's ZFS module
    /// comes from.
    pub kernels: &'static [KernelInfo],
    /// What the installer asks for by role rather than by name.
    pub packages: &'static SystemPackages,
    /// The packaging the distribution is built on, which decides how
    /// everything above is fetched and installed.
    pub family: Family,
}

/// Packages the installer installs for what they do, under the names a
/// distribution gives them.
#[derive(Debug, Clone, Copy)]
pub struct SystemPackages {
    /// ZFS userland, shared by every kernel's module.
    pub zfs_utils: &'static [&'static str],
    /// The ZFS module built by DKMS against a kernel's headers.
    pub zfs_dkms: &'static str,
    pub network_manager: &'static [&'static str],
    pub iwd: &'static [&'static str],
    /// What reads `/etc/systemd/zram-generator.conf` and creates the device.
    pub zram_generator: &'static [&'static str],
    pub intel_microcode: &'static str,
    pub amd_microcode: &'static str,
    /// Each initramfs generator together with its ZFS support. `None` where
    /// the distribution does not offer that generator.
    pub dracut: Option<&'static [&'static str]>,
    pub mkinitcpio: Option<&'static [&'static str]>,
}

impl SystemPackages {
    pub fn microcode(&self, vendor: CpuVendor) -> Option<&'static str> {
        match vendor {
            CpuVendor::Intel => Some(self.intel_microcode),
            CpuVendor::Amd => Some(self.amd_microcode),
            CpuVendor::Unknown => None,
        }
    }

    /// What builds the initramfs for this choice, when the distribution
    /// offers it.
    pub fn initramfs(&self, init_system: InitSystem) -> Option<&'static [&'static str]> {
        match init_system {
            InitSystem::Dracut => self.dracut,
            InitSystem::Mkinitcpio => self.mkinitcpio,
        }
    }

    /// Every name in the table, for the repository check.
    pub fn all(&self) -> Vec<&'static str> {
        let mut names = vec![self.zfs_dkms, self.intel_microcode, self.amd_microcode];
        for set in [
            self.zfs_utils,
            self.network_manager,
            self.iwd,
            self.zram_generator,
        ] {
            names.extend_from_slice(set);
        }
        for set in [self.dracut, self.mkinitcpio].into_iter().flatten() {
            names.extend_from_slice(set);
        }
        names
    }
}

/// The packaging a distribution is built on.
#[derive(Debug, Clone, Copy)]
pub enum Family {
    /// Arch and the distributions that extend its repositories.
    Arch(Pacman),
    /// Debian, bootstrapped with debootstrap and completed with apt.
    Debian(Apt),
}

/// Where an apt-based distribution is fetched from.
#[derive(Debug, Clone, Copy)]
pub struct Apt {
    /// The archive the release and its companion suites are served from.
    pub mirror: &'static str,
    /// The release debootstrap installs.
    pub suite: &'static str,
    /// Suites served from `mirror` next to the release: updates, backports.
    pub companion_suites: &'static [&'static str],
    /// Security updates live in an archive of their own.
    pub security_mirror: &'static str,
    pub security_suite: &'static str,
    /// Every suite is written with these components. ZFS is in `contrib`,
    /// because its licence keeps it out of `main`.
    pub components: &'static [&'static str],
    /// The keyring on the live medium the release is verified against, and
    /// that the installed system keeps trusting.
    pub keyring: &'static str,
    /// Source packages taken from a suite other than the release.
    pub pins: &'static [Pin],
}

/// Keeps a source package's binaries on one suite.
///
/// Pinning by source package rather than by binary is what keeps them in
/// step: trixie-backports serves two OpenZFS branches at once, and
/// `zfsutils-linux` from one breaks `zfs-dkms` from the other.
#[derive(Debug, Clone, Copy)]
pub struct Pin {
    pub source_package: &'static str,
    pub suite: &'static str,
}

impl Apt {
    /// The installed system's `/etc/apt/sources.list.d/debian.sources`.
    pub fn sources(&self) -> String {
        let components = self.components.join(" ");
        let mut suites = vec![self.suite];
        suites.extend_from_slice(self.companion_suites);
        format!(
            "Types: deb\nURIs: {}\nSuites: {}\nComponents: {components}\nSigned-By: {}\n\n\
             Types: deb\nURIs: {}\nSuites: {}\nComponents: {components}\nSigned-By: {}\n",
            self.mirror,
            suites.join(" "),
            self.keyring,
            self.security_mirror,
            self.security_suite,
            self.keyring,
        )
    }

    /// The installed system's apt preferences for `pins`. Priority 990 is
    /// what `apt-get -t` would give the suite, here limited to the pinned
    /// packages and kept for later upgrades.
    pub fn preferences(&self) -> String {
        self.pins
            .iter()
            .map(|pin| {
                format!(
                    "Package: src:{}\nPin: release n={}\nPin-Priority: 990\n",
                    pin.source_package, pin.suite
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// What a pacman-based distribution adds to Arch.
#[derive(Debug, Clone, Copy)]
pub struct Pacman {
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

/// Arch's names, which CachyOS shares: it extends Arch's repositories rather
/// than replacing them.
const ARCH_PACKAGES: SystemPackages = SystemPackages {
    zfs_utils: &["zfs-utils"],
    zfs_dkms: "zfs-dkms",
    network_manager: &["networkmanager"],
    iwd: &["iwd"],
    zram_generator: &["zram-generator"],
    intel_microcode: "intel-ucode",
    amd_microcode: "amd-ucode",
    dracut: Some(&["dracut"]),
    mkinitcpio: Some(&["mkinitcpio"]),
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
    kernels: ARCH_KERNELS,
    packages: &ARCH_PACKAGES,
    family: Family::Arch(Pacman {
        repositories: RepositorySelection::Fixed(&[ARCHZFS]),
        optimised_builds: false,
        keyring: "archlinux",
        zfsbootmenu_package: None,
    }),
};

impl Distribution {
    /// The pacman side of the distribution, when it has one.
    pub fn pacman(&self) -> Option<&Pacman> {
        match &self.family {
            Family::Arch(pacman) => Some(pacman),
            Family::Debian(_) => None,
        }
    }

    /// The apt side of the distribution, when it has one.
    pub fn apt(&self) -> Option<&Apt> {
        match &self.family {
            Family::Debian(apt) => Some(apt),
            Family::Arch(_) => None,
        }
    }
}

impl Pacman {
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
        // os-release comes from Arch's filesystem package; these hooks rewrite
        // it (and lsb-release, issue) to CachyOS after every filesystem update.
        "cachyos-hooks",
    ],
    kernels: CACHYOS_KERNELS,
    packages: &ARCH_PACKAGES,
    family: Family::Arch(Pacman {
        repositories: RepositorySelection::ByIsaLevel {
            v3: CACHYOS_V3,
            v4: CACHYOS_V4,
            znver4: CACHYOS_ZNVER4,
        },
        optimised_builds: true,
        keyring: "archlinux",
        // Theirs is packaged, so there is nothing to build.
        zfsbootmenu_package: Some("zfsbootmenu"),
    }),
};

/// Debian's names for the role packages. ZFS is DKMS-only: Debian ships no
/// prebuilt module, and the kernels carry no ZFS of their own.
///
/// dracut's ZFS module goes in with the utilities rather than with dracut:
/// `zfs-dracut` depends on `zfs-dkms`, and installed with the base system it
/// would pull the module in before the ZFS phase has set it up.
const DEBIAN_PACKAGES: SystemPackages = SystemPackages {
    zfs_utils: &["zfsutils-linux", "zfs-zed", "zfs-dracut"],
    zfs_dkms: "zfs-dkms",
    network_manager: &["network-manager"],
    iwd: &["iwd"],
    zram_generator: &["systemd-zram-generator"],
    intel_microcode: "intel-microcode",
    amd_microcode: "amd64-microcode",
    dracut: Some(&["dracut"]),
    mkinitcpio: None,
};

const DEBIAN_KERNELS: &[KernelInfo] = &[KernelInfo {
    name: "linux-image-amd64",
    display_name: "Debian stable",
    precompiled_package: None,
    headers_package: "linux-headers-amd64",
}];

/// Debian stable with OpenZFS from backports.
///
/// Not in [`ALL`] until the installer can complete a Debian installation.
pub const DEBIAN: Distribution = Distribution {
    name: "debian",
    display_name: "Debian",
    base_packages: &[
        "locales",
        "console-setup",
        "keyboard-configuration",
        "sudo",
        "systemd-timesyncd",
        "ca-certificates",
        "dosfstools",
        "efibootmgr",
        "firmware-linux",
        "firmware-sof-signed",
    ],
    kernels: DEBIAN_KERNELS,
    packages: &DEBIAN_PACKAGES,
    family: Family::Debian(Apt {
        mirror: "http://deb.debian.org/debian",
        suite: "trixie",
        companion_suites: &["trixie-updates", "trixie-backports"],
        security_mirror: "http://security.debian.org/debian-security",
        security_suite: "trixie-security",
        components: &["main", "contrib", "non-free-firmware"],
        keyring: "/usr/share/keyrings/debian-archive-keyring.gpg",
        pins: &[Pin {
            source_package: "zfs-linux",
            suite: "trixie-backports",
        }],
    }),
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

    fn pacman(distro: &'static Distribution) -> &'static Pacman {
        distro.pacman().expect("a pacman-based distribution")
    }

    #[test]
    fn distributions_are_found_by_name() {
        assert_eq!(get("arch").map(|d| d.display_name), Some("Arch Linux"));
        assert!(get("plan9").is_none());
        assert_eq!(default().name, "arch");
    }

    /// A configuration that does not choose a generator gets the default one,
    /// so every distribution has to be able to install it.
    #[test]
    fn every_distribution_offers_the_default_initramfs() {
        for distro in ALL {
            assert!(
                distro.packages.initramfs(InitSystem::default()).is_some(),
                "{} cannot build the default initramfs",
                distro.name
            );
        }
    }

    #[test]
    fn debian_is_installed_with_apt() {
        assert!(DEBIAN.pacman().is_none());
        assert!(DEBIAN.apt().is_some());
        assert!(ARCH.apt().is_none());
        assert!(DEBIAN.packages.initramfs(InitSystem::default()).is_some());
        assert!(DEBIAN.packages.initramfs(InitSystem::Mkinitcpio).is_none());
    }

    #[test]
    fn debian_sources_list_every_suite_with_zfs_reachable() {
        let apt = DEBIAN.apt().unwrap();
        let sources = apt.sources();

        let stanzas: Vec<&str> = sources.split_inclusive("\n\n").collect();
        assert_eq!(stanzas.len(), 2, "archive and security: {sources}");
        assert!(stanzas[0].contains("Suites: trixie trixie-updates trixie-backports\n"));
        assert!(stanzas[1].contains("URIs: http://security.debian.org/debian-security\n"));
        assert!(stanzas[1].contains("Suites: trixie-security\n"));
        for stanza in stanzas {
            assert!(stanza.contains("Components: main contrib non-free-firmware\n"));
            assert!(stanza.contains("Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg\n"));
        }
    }

    #[test]
    fn zfs_is_pinned_to_backports_by_source_package() {
        assert_eq!(
            DEBIAN.apt().unwrap().preferences(),
            "Package: src:zfs-linux\nPin: release n=trixie-backports\nPin-Priority: 990\n"
        );
    }

    #[test]
    fn debian_zfs_module_is_always_built_with_dkms() {
        use crate::config::types::ZfsModuleMode;
        let kernel = DEBIAN.kernels[0].name;
        for mode in [ZfsModuleMode::Precompiled, ZfsModuleMode::Dkms] {
            assert_eq!(
                crate::kernel::zfs_module_packages(&DEBIAN, kernel, mode),
                ["zfs-dkms", "linux-headers-amd64"]
            );
        }
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
        let v3 = pacman(&CACHYOS).repositories(IsaLevel::V3);
        let v4 = pacman(&CACHYOS).repositories(IsaLevel::V4);
        let zen = pacman(&CACHYOS).repositories(IsaLevel::Znver4);

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
        assert!(pacman(&CACHYOS).repositories(IsaLevel::Baseline).is_empty());
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
            pacman(&CACHYOS).architectures(IsaLevel::V3),
            Some("x86_64 x86_64_v3")
        );
        assert_eq!(
            pacman(&CACHYOS).architectures(IsaLevel::V4),
            Some("x86_64 x86_64_v3 x86_64_v4")
        );
        assert_eq!(
            pacman(&CACHYOS).architectures(IsaLevel::Znver4),
            pacman(&CACHYOS).architectures(IsaLevel::V4)
        );
        assert_eq!(pacman(&CACHYOS).architectures(IsaLevel::Baseline), None);

        // A distribution without optimised builds says nothing about it.
        for isa in [IsaLevel::V3, IsaLevel::V4, IsaLevel::Znver4] {
            assert_eq!(pacman(&ARCH).architectures(isa), None);
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
            assert_eq!(pacman(&ARCH).repositories(isa).len(), 1);
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
