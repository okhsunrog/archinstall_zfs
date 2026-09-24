//! Every package name the installer can ask Arch for, held against the
//! repositories.
//!
//! Package names age: `swww` was renamed to `awww`, `nitrogen` was dropped,
//! and an install that asks for either now fails on the user's machine at the
//! phase that installs it. Nothing in a unit test catches that, because the
//! answer lives in the package databases rather than in this repository.
//!
//! The test is ignored by default: it reads the live pacman databases, syncs
//! them when they are stale, and so needs an Arch system with network access.
//! Run it before a release:
//!
//! ```sh
//! just check-packages
//! # or
//! cargo test -p archinstall-zfs-core --test repo_packages -- --ignored --nocapture
//! ```

use archinstall_zfs_core::distro;
use archinstall_zfs_core::installer::fixed_packages;
use archinstall_zfs_core::kernel;
use archinstall_zfs_core::profile;
use archinstall_zfs_core::system::alpm_pacman::resolve_name;
use archinstall_zfs_core::system::gpu::GfxDriver;

/// The repositories an installation can actually install from: what the live
/// medium carries, plus the archzfs repository the installer adds itself.
/// A name that only a user's own third-party repository answers to is as
/// unusable here as one that does not exist.
const INSTALLABLE_REPOS: &[&str] = &["core", "extra", "multilib", "archzfs"];

/// A package name together with where the installer gets it from, so a
/// failure says which table to fix.
struct Wanted {
    name: String,
    source: String,
}

fn wanted(source: &str, names: impl IntoIterator<Item = String>) -> Vec<Wanted> {
    names
        .into_iter()
        .map(|name| Wanted {
            name,
            source: source.to_string(),
        })
        .collect()
}

/// The package lists of rendered live-image profiles, as `just
/// check-packages` renders them: one path per kernel, separated by colons.
///
/// The image's own list is the other half of this check. A name that has
/// left the repositories stops `mkarchiso` rather than an installation —
/// which is how September's monthly build died on `broadcom-wl`, five days
/// before anyone noticed the release had not appeared.
const ISO_PACKAGE_LISTS: &str = "AZFS_ISO_PACKAGE_LISTS";

fn iso_profile_packages() -> Vec<Wanted> {
    let Ok(lists) = std::env::var(ISO_PACKAGE_LISTS) else {
        println!("{ISO_PACKAGE_LISTS} is unset: the live image's own list is not checked");
        return Vec::new();
    };

    let mut names = Vec::new();
    for path in lists.split(':').filter(|p| !p.is_empty()) {
        let listing = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("cannot read the rendered list {path}: {error}"));
        let source = format!("live image {}", std::path::Path::new(path).display());
        names.extend(wanted(
            &source,
            listing
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(str::to_string),
        ));
    }
    names
}

/// Every Arch package name that is written down in this repository rather
/// than typed by the user.
fn every_package_name() -> Vec<Wanted> {
    let arch = &distro::ARCH;
    let mut names = iso_profile_packages();

    names.extend(wanted(
        "distro::ARCH base_packages",
        arch.base_packages.iter().map(|p| p.to_string()),
    ));

    for kernel in arch.kernels {
        names.extend(wanted(
            &format!("distro::ARCH kernel {}", kernel.name),
            [kernel.name.to_string(), kernel.headers_package.to_string()]
                .into_iter()
                .chain(kernel.precompiled_package.map(str::to_string)),
        ));
        // Both module modes, so a missing dkms package is caught as well.
        for mode in [
            archinstall_zfs_core::config::types::ZfsModuleMode::Precompiled,
            archinstall_zfs_core::config::types::ZfsModuleMode::Dkms,
        ] {
            names.extend(wanted(
                &format!("kernel::zfs_module_packages {} {mode:?}", kernel.name),
                kernel::zfs_module_packages(arch, kernel.name, mode),
            ));
        }
    }

    names.extend(wanted(
        "distro::ARCH packages",
        arch.packages.all().into_iter().map(str::to_string),
    ));

    for profile in profile::all_profiles() {
        let source = format!("profile '{}'", profile.name);
        names.extend(wanted(
            &source,
            profile
                .packages
                .iter()
                .chain(profile.excluded_packages.iter())
                .map(|p| p.to_string()),
        ));
        names.extend(wanted(
            &format!("{source} optional"),
            profile
                .optional_packages()
                .iter()
                .map(|o| o.package.to_string()),
        ));
    }

    names.extend(wanted(
        "profile::DisplayManager",
        profile::DisplayManager::ALL
            .iter()
            .map(|dm| dm.package().to_string()),
    ));

    for driver in GfxDriver::ALL {
        names.extend(wanted(
            &format!("gpu::GfxDriver::{driver:?}"),
            driver.packages().iter().map(|p| p.to_string()),
        ));
    }

    names.extend(wanted(
        "installer::fixed_packages",
        fixed_packages::ALL
            .iter()
            .flat_map(|set| set.iter())
            .map(|p| p.to_string()),
    ));

    names
}

#[test]
#[ignore = "reads the live pacman databases; run it with --ignored on Arch"]
fn every_package_we_install_is_in_the_repositories() {
    let handle = kernel::init_alpm().expect("the live pacman databases");
    let registered: Vec<String> = handle
        .syncdbs()
        .iter()
        .map(|db| db.name().to_string())
        .collect();
    for repo in INSTALLABLE_REPOS {
        assert!(
            registered.iter().any(|name| name == repo),
            "this machine has no [{repo}] repository, so the check would pass names it \
             cannot see. Registered: {registered:?}"
        );
    }

    let mut failures = Vec::new();
    let mut checked = 0usize;
    for want in every_package_name() {
        checked += 1;
        match resolve_name(&handle, &want.name) {
            Err(_) => failures.push(format!(
                "{}: '{}' is in no repository",
                want.source, want.name
            )),
            Ok(packages) => {
                // A group takes whichever members each repository gives it, so
                // a development machine's own repositories add members an
                // installation never sees. The group itself is what has to
                // exist in a repository an installation can reach.
                let installable: Vec<_> = packages
                    .iter()
                    .filter(|pkg| {
                        pkg.db()
                            .is_some_and(|db| INSTALLABLE_REPOS.contains(&db.name()))
                    })
                    .collect();
                if installable.is_empty() {
                    let found: Vec<String> = packages
                        .iter()
                        .map(|pkg| {
                            format!(
                                "{} in [{}]",
                                pkg.name(),
                                pkg.db().map(|db| db.name()).unwrap_or("?")
                            )
                        })
                        .collect();
                    failures.push(format!(
                        "{}: '{}' is in no repository an installation can reach, only {}",
                        want.source,
                        want.name,
                        found.join(", "),
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {checked} package names cannot be installed:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
    println!("{checked} package names check out against {registered:?}");
}

/// `pacman -S` installs a package that only *provides* the name it is given,
/// which is how a rename survives: `awww` still provides `swww`. The
/// installer resolves names itself, so it has to do the same.
#[test]
#[ignore = "reads the live pacman databases; run it with --ignored on Arch"]
fn a_name_only_a_provider_answers_to_still_resolves() {
    let handle = kernel::init_alpm().expect("the live pacman databases");

    // ttf-font is virtual: no package carries the name, many provide it.
    let packages = resolve_name(&handle, "ttf-font").expect("a font provides ttf-font");
    assert_eq!(packages.len(), 1);
    assert_ne!(packages[0].name(), "ttf-font");
}
