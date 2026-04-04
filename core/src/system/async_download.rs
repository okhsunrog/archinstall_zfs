use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result, bail};
use futures::stream::{self, StreamExt};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

/// Metadata needed to download a single package.
pub struct DownloadTask {
    pub filename: String,
    pub servers: Vec<String>,
    pub sha256: Option<String>,
    pub size: i64,
    pub sig_required: bool,
}

/// Download all packages to `cache_dir` in parallel.
///
/// Designed to be called from `spawn_blocking` via `Handle::block_on()`.
/// Downloads are cancellable via the provided `CancellationToken`.
pub async fn download_packages(
    tasks: Vec<DownloadTask>,
    cache_dir: PathBuf,
    concurrency: usize,
    cancel: CancellationToken,
) -> Result<()> {
    if tasks.is_empty() {
        return Ok(());
    }

    let total_size: i64 = tasks.iter().map(|t| t.size).sum();
    let total_count = tasks.len();
    tracing::info!(
        count = total_count,
        total_bytes = total_size,
        "downloading {total_count} packages"
    );

    let client = reqwest::Client::builder()
        .user_agent("archinstall-zfs-rs")
        .build()
        .wrap_err("failed to create HTTP client")?;

    let results: Vec<Result<()>> = stream::iter(tasks)
        .map(|task| {
            let client = client.clone();
            let cache_dir = cache_dir.clone();
            let cancel = cancel.clone();
            async move { download_single(&client, task, &cache_dir, &cancel).await }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    // Check for errors
    let mut errors = Vec::new();
    for result in results {
        if let Err(e) = result {
            errors.push(e);
        }
    }

    if !errors.is_empty() {
        let first = errors.remove(0);
        if first.to_string().contains("cancelled") {
            bail!("download cancelled");
        }
        return Err(first.wrap_err(format!("{} package download(s) failed", errors.len() + 1)));
    }

    tracing::info!("all {total_count} packages downloaded");
    Ok(())
}

async fn download_single(
    client: &reqwest::Client,
    task: DownloadTask,
    cache_dir: &Path,
    cancel: &CancellationToken,
) -> Result<()> {
    let dest = cache_dir.join(&task.filename);
    let dest_sig = cache_dir.join(format!("{}.sig", &task.filename));

    // Skip if already cached and valid
    if dest.exists()
        && verify_sha256_sync(&dest, task.sha256.as_deref())
        && (!task.sig_required || dest_sig.exists())
    {
        tracing::debug!(file = %task.filename, "already cached, skipping");
        return Ok(());
    }

    tracing::info!(
        file = %task.filename,
        size = task.size,
        "downloading"
    );

    // Try each mirror in order
    let mut last_error = None;
    for server in &task.servers {
        let url = format!("{}/{}", server, task.filename);

        match download_file(client, &url, &dest, task.sha256.as_deref(), cancel).await {
            Ok(()) => {
                // Download signature if required
                if task.sig_required {
                    let sig_url = format!("{}.sig", url);
                    if let Err(e) = download_file(client, &sig_url, &dest_sig, None, cancel).await {
                        tracing::warn!(
                            file = %task.filename,
                            server,
                            "failed to download signature: {e}"
                        );
                        // Try next mirror for both pkg + sig
                        last_error = Some(e);
                        continue;
                    }
                }

                tracing::info!(file = %task.filename, "download complete");
                return Ok(());
            }
            Err(e) => {
                if cancel.is_cancelled() {
                    bail!("download cancelled");
                }
                tracing::debug!(
                    file = %task.filename,
                    server,
                    "mirror failed: {e}, trying next"
                );
                last_error = Some(e);
                continue;
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| color_eyre::eyre::eyre!("no mirrors available for {}", task.filename)))
}

async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    cancel: &CancellationToken,
) -> Result<()> {
    let resp = client
        .get(url)
        .send()
        .await
        .wrap_err_with(|| format!("HTTP request failed: {url}"))?;

    if !resp.status().is_success() {
        bail!("HTTP {} for {url}", resp.status());
    }

    // Write to .part file, then rename on success
    let part_path = dest.with_extension(format!(
        "{}.part",
        dest.extension().unwrap_or_default().to_string_lossy()
    ));

    let mut file = tokio::fs::File::create(&part_path)
        .await
        .wrap_err_with(|| format!("failed to create {}", part_path.display()))?;

    let mut hasher = Sha256::new();
    let mut stream = resp.bytes_stream();

    loop {
        tokio::select! {
            biased;

            _ = cancel.cancelled() => {
                drop(file);
                tokio::fs::remove_file(&part_path).await.ok();
                bail!("download cancelled");
            }

            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        hasher.update(&bytes);
                        file.write_all(&bytes).await
                            .wrap_err("failed to write chunk")?;
                    }
                    Some(Err(e)) => {
                        drop(file);
                        tokio::fs::remove_file(&part_path).await.ok();
                        return Err(e).wrap_err("download stream error");
                    }
                    None => break, // stream finished
                }
            }
        }
    }

    file.flush().await?;
    drop(file);

    // Verify SHA256
    if let Some(expected) = expected_sha256 {
        let actual = hex::encode(hasher.finalize());
        if actual != expected {
            tokio::fs::remove_file(&part_path).await.ok();
            bail!(
                "SHA256 mismatch for {}: expected {expected}, got {actual}",
                dest.display()
            );
        }
    }

    // Atomic rename .part → final
    tokio::fs::rename(&part_path, dest)
        .await
        .wrap_err_with(|| format!("failed to rename to {}", dest.display()))?;

    Ok(())
}

/// Synchronous SHA256 verification for checking already-cached files.
fn verify_sha256_sync(path: &Path, expected: Option<&str>) -> bool {
    let expected = match expected {
        Some(e) => e,
        None => return true, // no checksum in DB, assume valid
    };

    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return false,
    };

    let actual = hex::encode(Sha256::digest(&data));
    actual == expected
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_verify_sha256_sync_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pkg");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"hello world").unwrap();
        drop(f);

        let expected = hex::encode(Sha256::digest(b"hello world"));
        assert!(verify_sha256_sync(&path, Some(&expected)));
    }

    #[test]
    fn test_verify_sha256_sync_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pkg");
        std::fs::write(&path, b"hello world").unwrap();

        assert!(!verify_sha256_sync(&path, Some("0000bad")));
    }

    #[test]
    fn test_verify_sha256_sync_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pkg");
        std::fs::write(&path, b"anything").unwrap();

        assert!(verify_sha256_sync(&path, None));
    }

    #[test]
    fn test_verify_sha256_sync_missing_file() {
        assert!(!verify_sha256_sync(Path::new("/nonexistent"), Some("abc")));
    }
}
