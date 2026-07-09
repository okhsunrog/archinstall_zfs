//! UI-facing helpers for the pool-selection screen. Each one wraps a
//! zfskit entry point and trims the result down to the primitive the
//! picker actually needs (a `Vec<String>` of names, a `bool`), keeping
//! zfskit out of the TUI/Slint crates.

/// Discover importable pools by name. Drops zfskit's richer
/// `DiscoveredPool` (id, state, status) — the picker only needs the name
/// list. On any error returns an empty `Vec`.
pub async fn discover_importable_pools() -> Vec<String> {
    zfskit::Zfs::new()
        .discover_importable_pools()
        .await
        .map(|ps| ps.into_iter().map(|p| p.name).collect())
        .unwrap_or_default()
}

/// Detect whether a pool is encrypted through an explicit no-mount import.
/// Import, property, and cleanup failures remain visible to the UI.
pub async fn detect_pool_encryption(pool_name: &str) -> color_eyre::Result<bool> {
    let zfs = zfskit::Zfs::new();
    let pool = zfs.pool(pool_name)?;
    pool.import(&zfskit::pool::ImportOptions {
        force: true,
        no_mount: true,
        ..Default::default()
    })
    .await?;
    let result = pool.root_dataset().get_property("encryption").await;
    let cleanup = pool.export(&zfskit::pool::ExportOptions::default()).await;
    let encrypted = result.map(|p| p.value != "off" && !p.value.is_empty())?;
    cleanup?;
    Ok(encrypted)
}

/// Verify a pool passphrase via an explicit no-mount import. Import,
/// verification, and cleanup failures remain visible to the UI.
pub async fn verify_pool_passphrase(pool_name: &str, password: &str) -> color_eyre::Result<bool> {
    let zfs = zfskit::Zfs::new();
    let pool = zfs.pool(pool_name)?;
    pool.import(&zfskit::pool::ImportOptions {
        force: true,
        no_mount: true,
        ..Default::default()
    })
    .await?;
    let result = pool
        .root_dataset()
        .verify_passphrase(password.as_bytes())
        .await;
    let cleanup = pool.export(&zfskit::pool::ExportOptions::default()).await;
    let verified = result?;
    cleanup?;
    Ok(verified)
}
