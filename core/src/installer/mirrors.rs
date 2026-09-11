use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::mirrors::{RankOptions, refresh_blocking};

/// Write a measured mirrorlist for the target, restricted to `countries`
/// (names or codes). Without countries the target keeps the copy of the
/// live medium's list, which was ranked the same way without a filter.
pub fn configure_mirrors(target: &Path, countries: &[String]) -> Result<()> {
    if countries.is_empty() {
        return Ok(());
    }
    tracing::info!(?countries, "ranking mirrors for the target");
    let opts = RankOptions {
        countries: countries.to_vec(),
        ..RankOptions::default()
    };
    refresh_blocking(&target.join("etc/pacman.d/mirrorlist"), opts)
        .wrap_err("mirror ranking for the target failed")?;
    tracing::info!("mirrors configured for target");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_countries_means_the_target_keeps_the_copied_list() {
        let target = tempfile::tempdir().unwrap();
        configure_mirrors(target.path(), &[]).unwrap();
        assert!(!target.path().join("etc/pacman.d/mirrorlist").exists());
    }
}
