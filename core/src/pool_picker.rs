//! UI-facing helpers for the pool-selection screen. Each one wraps a
//! palimpsest entry point and trims the result down to the primitive the
//! picker actually needs (a `Vec<String>` of names, a `bool`), keeping
//! palimpsest out of the TUI/Slint crates.

/// Discover importable pools by name. Drops palimpsest's richer
/// `DiscoveredPool` (id, state, status) — the picker only needs the name
/// list. On any error returns an empty `Vec`.
pub async fn discover_importable_pools() -> Vec<String> {
    palimpsest::Zfs::new()
        .discover_importable_pools()
        .await
        .map(|ps| ps.into_iter().map(|p| p.name).collect())
        .unwrap_or_default()
}

/// Detect whether a pool is encrypted via an ephemeral import. Collapses
/// any failure to `false` — the installer flow treats "can't tell" as
/// "not encrypted" (the passphrase prompt simply isn't shown).
pub async fn detect_pool_encryption(pool_name: &str) -> bool {
    palimpsest::Zfs::new()
        .pool(pool_name)
        .is_encrypted()
        .await
        .unwrap_or(false)
}

/// Verify a pool passphrase via an ephemeral import. Same collapse-to-false
/// semantics as [`detect_pool_encryption`].
pub async fn verify_pool_passphrase(pool_name: &str, password: &str) -> bool {
    palimpsest::Zfs::new()
        .pool(pool_name)
        .verify_passphrase(password.as_bytes())
        .await
        .unwrap_or(false)
}
