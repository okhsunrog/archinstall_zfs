use crate::{AurPackage, Pkgbuild};

use std::fmt::{Display, Formatter, Write};

enum PkgNames<A, C> {
    Aur(A),
    Pkgbuild(C),
}

impl<'a, A, C> Iterator for PkgNames<A, C>
where
    A: Iterator<Item = &'a str>,
    C: Iterator<Item = &'a str>,
{
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            PkgNames::Aur(i) => i.next(),
            PkgNames::Pkgbuild(i) => i.next(),
        }
    }
}

/// Packages from a pkgbuild repo.
#[derive(Debug, Eq, Clone, PartialEq, Ord, PartialOrd, Hash)]
pub struct PkgbuildPackages {
    /// the repo the package came from.
    pub repo: String,
    /// The srcinfo of the pkgbase.
    pub srcinfo: Box<srcinfo::Srcinfo>,
    /// The pkgs from the srcinfo to install.
    pub pkgs: Vec<Pkgbuild>,
    /// Should the package be built.
    pub build: bool,
}

/// Describes an AUR package base.
#[derive(Debug, Eq, Clone, PartialEq, Ord, PartialOrd, Hash)]
pub struct AurBase {
    /// List of packages belonging to the package base.
    pub pkgs: Vec<AurPackage>,
    /// Should the package be built.
    pub build: bool,
}

/// A package base.
/// This descripes  packages that should be built then installed.
#[derive(Debug, Eq, Clone, PartialEq, Ord, PartialOrd, Hash)]
pub enum Base {
    /// Aur packages.
    Aur(AurBase),
    /// pkgbuild packages.
    Pkgbuild(PkgbuildPackages),
}

impl Display for AurBase {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let pkgs = self.pkgs.iter().map(|p| p.pkg.name.as_str());
        Base::write_base(f, self.package_base(), &self.version(), pkgs)
    }
}

impl Display for PkgbuildPackages {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let pkgs = self.pkgs.iter().map(|p| p.pkg.pkgname.as_str());
        Base::write_base(f, self.package_base(), &self.version(), pkgs)
    }
}

impl Display for Base {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Base::Aur(base) => base.fmt(f),
            Base::Pkgbuild(base) => base.fmt(f),
        }
    }
}

impl AurBase {
    /// Gets the package base of base.
    pub fn package_base(&self) -> &str {
        &self.pkgs[0].pkg.package_base
    }

    /// Gets the version of base.
    pub fn version(&self) -> String {
        self.pkgs[0].pkg.version.clone()
    }
}

impl PkgbuildPackages {
    /// Gets the package base of base.
    pub fn package_base(&self) -> &str {
        &self.srcinfo.base.pkgbase
    }

    /// Gets the version of base.
    pub fn version(&self) -> String {
        self.srcinfo.version()
    }
}

impl Base {
    /// Gets the package base of base.
    pub fn package_base(&self) -> &str {
        match self {
            Base::Aur(base) => base.package_base(),
            Base::Pkgbuild(base) => base.package_base(),
        }
    }

    /// Gets the version of base.
    pub fn version(&self) -> String {
        match self {
            Base::Aur(base) => base.version(),
            Base::Pkgbuild(base) => base.version(),
        }
    }

    /// Ammount of packages in this base.
    pub fn package_count(&self) -> usize {
        match self {
            Base::Aur(base) => base.pkgs.len(),
            Base::Pkgbuild(base) => base.pkgs.len(),
        }
    }

    /// Iterator of package names in this base.
    pub fn packages(&self) -> impl Iterator<Item = &str> {
        match self {
            Base::Aur(base) => PkgNames::Aur(base.pkgs.iter().map(|p| p.pkg.name.as_str())),
            Base::Pkgbuild(base) => {
                PkgNames::Pkgbuild(base.pkgs.iter().map(|p| p.pkg.pkgname.as_str()))
            }
        }
    }

    /// Are any packages in this base make only.
    pub fn make(&self) -> bool {
        match self {
            Base::Aur(a) => a.pkgs.iter().any(|p| p.make),
            Base::Pkgbuild(c) => c.pkgs.iter().any(|p| p.make),
        }
    }

    /// Are any packages in this base targets.
    pub fn target(&self) -> bool {
        match self {
            Base::Aur(a) => a.pkgs.iter().any(|p| p.target),
            Base::Pkgbuild(c) => c.pkgs.iter().any(|p| p.target),
        }
    }

    /// Should the packages be built
    pub fn build(&self) -> bool {
        match self {
            Base::Aur(a) => a.build,
            Base::Pkgbuild(c) => c.build,
        }
    }

    /// Formats a base into the format:
    /// pkgname-ver
    /// or, if there are multiple packages:
    /// pkgbase-ver (pkg1 pkg2 pkg2)
    pub fn write_base<'a, W: Write, I: IntoIterator<Item = &'a str>>(
        mut writer: W,
        pkgbase: &str,
        ver: &str,
        pkgs: I,
    ) -> std::fmt::Result {
        let mut pkgs = pkgs.into_iter().peekable();
        let name = pkgs.next().unwrap_or("");

        if pkgs.peek().is_none() && name == pkgbase {
            write!(writer, "{pkgbase}-{ver}")
        } else {
            write!(writer, "{pkgbase}-{ver} ({name}")?;
            for pkg in pkgs {
                writer.write_str(" ")?;
                writer.write_str(pkg.as_ref())?;
            }
            writer.write_str(")")?;
            Ok(())
        }
    }

    /// True if the package base contains a single package
    /// with the same name as the pkgbase.
    pub fn base_is_pkg<'a, I: IntoIterator<Item = &'a str>>(pkgbase: &str, pkgs: I) -> bool {
        let mut pkgs = pkgs.into_iter();
        let Some(p) = pkgs.next() else { return false };
        p == pkgbase && pkgs.next().is_none()
    }
}
