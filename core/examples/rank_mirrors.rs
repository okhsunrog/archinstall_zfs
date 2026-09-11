//! Rank Arch mirrors the way the installer does and print the result.
//!
//! Usage: `cargo run -p archinstall-zfs-core --example rank_mirrors [COUNTRY...]`
//! Writes the mirrorlist to a temporary file and prints it; nothing on the
//! host is changed.
use archinstall_zfs_core::system::mirrors::{RankOptions, refresh};
use std::time::Instant;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,archinstall_zfs_core=debug")
        .init();
    let countries: Vec<String> = std::env::args().skip(1).collect();
    let opts = RankOptions {
        countries,
        ..RankOptions::default()
    };
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("mirrorlist");
    let started = Instant::now();
    let ranked = refresh(&path, &opts).await?;
    println!(
        "ranked {} mirrors in {:.1}s",
        ranked.len(),
        started.elapsed().as_secs_f64()
    );
    print!("{}", std::fs::read_to_string(&path)?);
    Ok(())
}
