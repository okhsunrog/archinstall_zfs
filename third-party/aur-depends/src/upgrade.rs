use crate::Error;
use crate::Flags;
use crate::Resolver;
use srcinfo::Srcinfo;

use alpm::{Alpm, AlpmList, Version};
use alpm_utils::DbListExt;
use raur::ArcPackage;
use raur::Raur;

/// An AUR package that should be updated.
#[derive(Debug)]
pub struct AurUpdate<'a> {
    /// The local package.
    pub local: &'a alpm::Package,
    /// The AUR package.
    pub remote: ArcPackage,
}

/// A pkgbuild should be updated.
#[derive(Debug)]
pub struct PkgbuildUpdate<'a> {
    /// The local package.
    pub local: &'a alpm::Package,
    /// The pkgbuild repo the package belongs to.
    pub repo: String,
    /// The pkgbuild package base srcinfo.
    pub remote_srcinfo: &'a Srcinfo,
    /// The pkgbuild package base package,
    pub remote_pkg: &'a srcinfo::Package,
}

/// Collection of AUR updates and missing packages.
#[derive(Debug, Default)]
pub struct Updates<'a> {
    /// The aur updates.
    pub aur_updates: Vec<AurUpdate<'a>>,
    /// The pkgbuild updates.
    pub pkgbuild_updates: Vec<PkgbuildUpdate<'a>>,
    /// Packages that matched ignore pkg/group.
    pub aur_ignored: Vec<AurUpdate<'a>>,
    /// Packages that matched ignore pkg/group.
    pub pkgbuild_ignored: Vec<PkgbuildUpdate<'a>>,
    /// Packages that were not found in the AUR or elsewhere.
    pub missing: Vec<&'a alpm::Package>,
}

impl<'a, 'b, E: std::error::Error + Sync + Send + 'static, H: Raur<Err = E> + Sync>
    Resolver<'a, 'b, H>
{
    fn update_targs<'c>(
        alpm: &'c Alpm,
        local: Option<AlpmList<&'c alpm::Db>>,
    ) -> Vec<&'c alpm::Package> {
        let dbs = alpm.syncdbs();

        if let Some(local) = local {
            local.iter().flat_map(|db| db.pkgs()).collect()
        } else {
            alpm.localdb()
                .pkgs()
                .into_iter()
                .filter(|p| dbs.pkg(p.name()).is_err())
                .collect()
        }
    }

    /// Get aur packages need to be updated.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use aur_depends::{Error, Updates};
    /// # #[tokio::test]
    /// # async fn run() -> Result<(), Error> {
    /// use std::collections::HashSet;
    /// use alpm::Alpm;
    /// use raur::Handle;
    ///
    /// use aur_depends::{Flags, Resolver};
    ///
    /// let alpm = Alpm::new("/", "/var/lib/pacman")?;
    /// let raur = Handle::default();
    /// let mut cache = HashSet::new();
    /// let mut resolver = Resolver::new(&alpm, Vec::new(), &mut cache, &raur, Flags::aur_only());
    ///
    /// let updates = resolver.updates().await?;
    ///
    /// for update in updates.aur_updates {
    ///     println!("update: {}: {} -> {}", update.local.name(), update.local.version(),
    ///     update.remote.version);
    /// }
    /// # Ok (())
    /// # }
    /// ```
    pub async fn updates(&mut self, local: Option<&[&str]>) -> Result<Updates<'a>, Error> {
        let local = match local {
            Some(local) => {
                let mut local_dbs = self.alpm.syncdbs().to_list_mut();
                local_dbs.retain(|db| local.iter().any(|name| *name == db.name()));
                Some(local_dbs)
            }
            None => None,
        };

        let targets = Self::update_targs(self.alpm, local.as_ref().map(|l| l.list()));
        let (mut pkgbuilds, mut aur) = targets
            .into_iter()
            .partition::<Vec<_>, _>(|p| self.is_pkgbuild(p.name()));

        if !self.flags.contains(Flags::AUR) {
            aur.clear();
        }
        if !self.flags.contains(Flags::PKGBUILDS) {
            pkgbuilds.clear();
        }

        let aur_pkg_names = aur.iter().map(|pkg| pkg.name()).collect::<Vec<_>>();
        self.raur
            .cache_info(self.cache, &aur_pkg_names)
            .await
            .map_err(|e| Error::Raur(Box::new(e)))?;

        let mut updates = Updates::default();

        for local in pkgbuilds {
            let (repo, srcinfo, remote) = self.find_pkgbuild(local.name()).unwrap();

            let should_upgrade = if self.flags.contains(Flags::ENABLE_DOWNGRADE) {
                Version::new(srcinfo.version()) != local.version()
            } else {
                Version::new(srcinfo.version()) > local.version()
            };
            if !should_upgrade {
                continue;
            }

            let update = PkgbuildUpdate {
                local,
                repo: repo.to_string(),
                remote_srcinfo: srcinfo,
                remote_pkg: remote,
            };

            if local.should_ignore() {
                updates.pkgbuild_ignored.push(update);
            } else {
                updates.pkgbuild_updates.push(update);
            }
        }

        for local in aur {
            if let Some(pkg) = self.cache.get(local.name()) {
                let should_upgrade = if self.flags.contains(Flags::ENABLE_DOWNGRADE) {
                    Version::new(&*pkg.version) != local.version()
                } else {
                    Version::new(&*pkg.version) > local.version()
                };
                if !should_upgrade {
                    continue;
                }

                let should_ignore = local.should_ignore();

                let update = AurUpdate {
                    local,
                    remote: pkg.clone(),
                };

                if should_ignore {
                    updates.aur_ignored.push(update);
                } else {
                    updates.aur_updates.push(update);
                }
            } else {
                updates.missing.push(local);
            }
        }

        Ok(updates)
    }
}
