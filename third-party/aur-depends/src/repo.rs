use std::sync::Arc;

use srcinfo::Srcinfo;

#[derive(Debug, Clone)]
/// A custom repo.
pub struct Repo {
    /// Name of the repo.
    pub name: String,
    /// Packages in the repo.
    pub pkgs: Arc<Vec<Srcinfo>>,
}
