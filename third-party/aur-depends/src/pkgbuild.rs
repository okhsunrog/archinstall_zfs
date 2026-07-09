use srcinfo::Srcinfo;

#[derive(Debug, Clone)]
/// A pkgbuild repo.
pub struct PkgbuildRepo<'a> {
    /// Name of the repo.
    pub name: &'a str,
    /// Packages in the repo.
    pub pkgs: Vec<&'a Srcinfo>,
}
