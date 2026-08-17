use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use color_eyre::eyre::{Context, Result, bail};
use futures::stream::{self, StreamExt};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

// ── Public types ──────────────────────────────────────

/// Metadata needed to download a single package.
pub struct DownloadTask {
    pub filename: String,
    pub servers: Vec<String>,
    pub sha256: Option<String>,
    pub size: i64,
}

/// Configuration for the download manager.
#[derive(Clone, Debug)]
pub struct DownloadConfig {
    /// Max parallel downloads (default: 5).
    pub concurrency: usize,
    /// Max retries per mirror before moving to the next (default: 3).
    pub retries_per_mirror: u32,
    /// Base delay for exponential backoff (default: 1s).
    pub backoff_base: Duration,
    /// Connection timeout (default: 10s).
    pub connect_timeout: Duration,
    /// Idle timeout — abort if no data received for this long (default: 30s).
    pub idle_timeout: Duration,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            concurrency: 5,
            retries_per_mirror: 3,
            backoff_base: Duration::from_secs(1),
            connect_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(30),
        }
    }
}

/// State of a single package download.
#[derive(Clone, Debug)]
pub enum PackageState {
    Queued,
    Downloading {
        filename: String,
        downloaded: u64,
        total: u64,
        speed_bps: u64,
        mirror: String,
        attempt: u32,
    },
    Verifying {
        filename: String,
        bytes: u64,
    },
    Done {
        filename: String,
        bytes: u64,
    },
    Failed {
        filename: String,
        error: String,
    },
}

impl PackageState {
    /// Bytes of this package that are on disk.
    ///
    /// Taken from the package's own state rather than accumulated as chunks
    /// arrive: a download that fails partway and is retried against another
    /// mirror would otherwise have its bytes counted once per attempt, and the
    /// aggregate would climb past the total.
    fn bytes_on_disk(&self) -> u64 {
        match self {
            Self::Downloading { downloaded, .. } => *downloaded,
            Self::Verifying { bytes, .. } | Self::Done { bytes, .. } => *bytes,
            Self::Queued | Self::Failed { .. } => 0,
        }
    }
}

/// Aggregate package progress, broadcast via `watch` channel.
/// Covers both the download phase and the install phase sequentially.
#[derive(Clone, Debug)]
pub enum PackageProgress {
    /// Downloading packages from mirrors.
    Downloading {
        packages: Vec<PackageState>,
        total_bytes: u64,
        downloaded_bytes: u64,
        active_downloads: usize,
        completed: usize,
        failed: usize,
    },
    /// Installing/upgrading packages locally.
    Installing {
        package: String,
        current: usize,
        total: usize,
        percent: u32,
    },
    /// All done (download + install finished).
    Done,
}

impl Default for PackageProgress {
    fn default() -> Self {
        Self::Downloading {
            packages: Vec::new(),
            total_bytes: 0,
            downloaded_bytes: 0,
            active_downloads: 0,
            completed: 0,
            failed: 0,
        }
    }
}

impl PackageProgress {
    /// Overall download speed in bytes/sec (only meaningful in Downloading phase).
    pub fn total_speed_bps(&self) -> u64 {
        match self {
            Self::Downloading { packages, .. } => packages
                .iter()
                .filter_map(|p| match p {
                    PackageState::Downloading { speed_bps, .. } => Some(*speed_bps),
                    _ => None,
                })
                .sum(),
            _ => 0,
        }
    }

    /// Estimated time remaining for download phase.
    pub fn eta(&self) -> Option<Duration> {
        match self {
            Self::Downloading {
                total_bytes,
                downloaded_bytes,
                ..
            } => {
                let speed = self.total_speed_bps();
                if speed == 0 {
                    return None;
                }
                let remaining = total_bytes.saturating_sub(*downloaded_bytes);
                Some(Duration::from_secs(remaining / speed))
            }
            _ => None,
        }
    }
}

// Keep the old name as an alias during migration
pub type DownloadProgress = PackageProgress;

// ── Download manager ──────────────────────────────────

/// Download all packages to `cache_dir` with full progress tracking.
///
/// If `progress_tx` is provided (from TUI), uses it for progress reporting.
/// Otherwise creates an internal channel (headless mode).
///
/// Returns a `watch::Receiver<DownloadProgress>` for TUI rendering and
/// a `JoinHandle` for the download task. The caller should `.await` the
/// handle to get the result.
pub fn start_downloads(
    tasks: Vec<DownloadTask>,
    cache_dir: PathBuf,
    config: DownloadConfig,
    cancel: CancellationToken,
    progress_tx: Option<Arc<watch::Sender<DownloadProgress>>>,
) -> (
    watch::Receiver<DownloadProgress>,
    tokio::task::JoinHandle<Result<()>>,
) {
    let total_bytes: u64 = tasks.iter().map(|t| t.size.max(0) as u64).sum();
    let pkg_count = tasks.len();

    let initial = PackageProgress::Downloading {
        packages: vec![PackageState::Queued; pkg_count],
        total_bytes,
        downloaded_bytes: 0,
        active_downloads: 0,
        completed: 0,
        failed: 0,
    };

    let (tx, rx) = if let Some(ref tx) = progress_tx {
        tx.send_replace(initial);
        let rx = tx.subscribe();
        (tx.clone(), rx)
    } else {
        let (tx, rx) = watch::channel(initial);
        (Arc::new(tx), rx)
    };

    let handle =
        tokio::spawn(async move { run_downloads(tasks, cache_dir, config, cancel, tx).await });

    (rx, handle)
}

/// Convenience wrapper that starts downloads and awaits completion.
/// Used from `AlpmContext::install_packages` via `block_on`.
pub async fn download_packages(
    tasks: Vec<DownloadTask>,
    cache_dir: PathBuf,
    concurrency: usize,
    cancel: CancellationToken,
    progress_tx: Option<Arc<watch::Sender<DownloadProgress>>>,
) -> Result<()> {
    if tasks.is_empty() {
        return Ok(());
    }

    tracing::debug!(
        task_count = tasks.len(),
        concurrency,
        has_progress_tx = progress_tx.is_some(),
        "download_packages: starting"
    );

    let config = DownloadConfig {
        concurrency,
        ..Default::default()
    };

    let (_rx, handle) = start_downloads(tasks, cache_dir, config, cancel, progress_tx);
    tracing::debug!("download_packages: awaiting handle");
    let result = handle.await?;
    tracing::debug!("download_packages: done");
    result
}

// ── Internal implementation ───────────────────────────

/// Shared state for coordinating progress updates.
struct SharedProgress {
    tx: Arc<watch::Sender<DownloadProgress>>,
}

impl SharedProgress {
    /// Record `state` for package `index` and recompute the aggregates.
    ///
    /// The counters are derived from the package vector instead of being
    /// adjusted per transition. Enumerating transitions has to get every edge
    /// right: a package already counted as cached (queued straight to done) was
    /// missing from `completed`, and a failed SHA256 sends a package from
    /// verifying back to downloading, which the deltas did not expect — the
    /// following completion then subtracted from an `active_downloads` that had
    /// already reached zero, underflowing the count.
    fn update_package(&self, index: usize, state: PackageState) {
        self.tx.send_modify(|progress| {
            let PackageProgress::Downloading {
                packages,
                active_downloads,
                completed,
                failed,
                downloaded_bytes,
                ..
            } = progress
            else {
                return;
            };
            packages[index] = state;

            *active_downloads = packages
                .iter()
                .filter(|p| matches!(p, PackageState::Downloading { .. }))
                .count();
            *completed = packages
                .iter()
                .filter(|p| matches!(p, PackageState::Done { .. }))
                .count();
            *failed = packages
                .iter()
                .filter(|p| matches!(p, PackageState::Failed { .. }))
                .count();
            *downloaded_bytes = packages.iter().map(PackageState::bytes_on_disk).sum();
        });
    }
}

async fn run_downloads(
    tasks: Vec<DownloadTask>,
    cache_dir: PathBuf,
    config: DownloadConfig,
    cancel: CancellationToken,
    tx: Arc<watch::Sender<DownloadProgress>>,
) -> Result<()> {
    let total_bytes: u64 = tasks.iter().map(|t| t.size.max(0) as u64).sum();
    let total_count = tasks.len();

    tracing::info!(
        count = total_count,
        total_bytes,
        "downloading {total_count} packages"
    );

    let shared = Arc::new(SharedProgress { tx });

    // No overall request timeout on purpose: a kernel or firmware package is
    // hundreds of megabytes and may legitimately take many minutes on a slow
    // link. Stalls are caught by the per-chunk idle timeout instead, which
    // distinguishes "slow" from "stopped".
    let client = reqwest::Client::builder()
        .user_agent("archinstall-zfs-rs")
        .connect_timeout(config.connect_timeout)
        .build()
        .wrap_err("failed to create HTTP client")?;

    // Sort by size descending — start large packages first
    let mut indexed_tasks: Vec<(usize, DownloadTask)> = tasks.into_iter().enumerate().collect();
    indexed_tasks.sort_by_key(|t| std::cmp::Reverse(t.1.size));

    let results: Vec<Result<()>> = stream::iter(indexed_tasks)
        .map(|(index, task)| {
            let client = client.clone();
            let cache_dir = cache_dir.clone();
            let cancel = cancel.clone();
            let config = config.clone();
            let shared = shared.clone();
            async move {
                download_single(&client, task, index, &cache_dir, &cancel, &config, &shared).await
            }
        })
        .buffer_unordered(config.concurrency)
        .collect()
        .await;

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
    index: usize,
    cache_dir: &Path,
    cancel: &CancellationToken,
    config: &DownloadConfig,
    shared: &SharedProgress,
) -> Result<()> {
    let dest = cache_dir.join(&task.filename);
    let total_size = task.size.max(0) as u64;

    // Skip if already cached and valid
    if is_cached_and_valid(&dest, task.sha256.as_deref()).await {
        tracing::debug!(file = %task.filename, "already cached, skipping");
        let bytes = tokio::fs::metadata(&dest)
            .await
            .map(|meta| meta.len())
            .unwrap_or(total_size);
        shared.update_package(
            index,
            PackageState::Done {
                filename: task.filename.clone(),
                bytes,
            },
        );
        return Ok(());
    }

    let download_started = std::time::Instant::now();

    // Try each mirror
    let mut last_error = None;
    for server in &task.servers {
        // Retry on same mirror with exponential backoff
        for attempt in 1..=config.retries_per_mirror {
            let url = format!("{}/{}", server, task.filename);

            shared.update_package(
                index,
                PackageState::Downloading {
                    filename: task.filename.clone(),
                    downloaded: 0,
                    total: total_size,
                    speed_bps: 0,
                    mirror: server.clone(),
                    attempt,
                },
            );

            match download_file_with_progress(
                client,
                &url,
                &dest,
                total_size,
                task.sha256.as_deref(),
                cancel,
                config,
                index,
                &task.filename,
                server,
                attempt,
                shared,
            )
            .await
            {
                Ok(bytes) => {
                    shared.update_package(
                        index,
                        PackageState::Done {
                            filename: task.filename.clone(),
                            bytes,
                        },
                    );
                    let duration_ms = download_started.elapsed().as_millis() as u64;
                    let speed_bps = (total_size * 1000).checked_div(duration_ms).unwrap_or(0);
                    tracing::info!(
                        target: "metrics",
                        event = "pkg_download",
                        filename = task.filename.as_str(),
                        bytes = total_size,
                        duration_ms = duration_ms,
                        mirror = server.as_str(),
                        speed_bps = speed_bps,
                    );
                    tracing::info!(file = %task.filename, "download complete");
                    return Ok(());
                }
                Err(e) => {
                    if cancel.is_cancelled() {
                        shared.update_package(
                            index,
                            PackageState::Failed {
                                filename: task.filename.clone(),
                                error: "cancelled".to_string(),
                            },
                        );
                        bail!("download cancelled");
                    }

                    tracing::debug!(
                        file = %task.filename,
                        server,
                        attempt,
                        "attempt failed: {e}"
                    );
                    last_error = Some(e);

                    // Exponential backoff before retry (on same mirror)
                    if attempt < config.retries_per_mirror {
                        let delay = config.backoff_base * 2u32.pow(attempt - 1);
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }
    }

    let error = last_error
        .unwrap_or_else(|| color_eyre::eyre::eyre!("no mirrors available for {}", task.filename));
    shared.update_package(
        index,
        PackageState::Failed {
            filename: task.filename.clone(),
            error: error.to_string(),
        },
    );
    Err(error)
}

/// Download a file with progress reporting, resume support, and SHA256
/// verification. Returns the number of bytes the finished file holds.
#[expect(clippy::too_many_arguments)]
async fn download_file_with_progress(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    total_size: u64,
    expected_sha256: Option<&str>,
    cancel: &CancellationToken,
    config: &DownloadConfig,
    index: usize,
    filename: &str,
    mirror: &str,
    attempt: u32,
    shared: &SharedProgress,
) -> Result<u64> {
    let part_path = part_file_path(dest);

    // Check for existing .part file for resume
    let existing_size = tokio::fs::metadata(&part_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    let mut request = client.get(url);
    let mut resume_offset = 0u64;

    if existing_size > 0 {
        // Try HTTP Range resume
        request = request.header("Range", format!("bytes={existing_size}-"));
        resume_offset = existing_size;
        tracing::debug!(file = filename, existing_size, "attempting resume");
    }

    let resp = request
        .send()
        .await
        .wrap_err_with(|| format!("HTTP request failed: {url}"))?;

    let status = resp.status();
    if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
        // If resume was rejected (416 Range Not Satisfiable), start fresh
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && resume_offset > 0 {
            tokio::fs::remove_file(&part_path).await.ok();
            return Err(color_eyre::eyre::eyre!(
                "resume rejected, will retry from start"
            ));
        }
        bail!("HTTP {} for {url}", status);
    }

    let resuming = status == reqwest::StatusCode::PARTIAL_CONTENT;

    // Open file for writing (append if resuming, create if fresh)
    let mut file = if resuming {
        let mut f = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&part_path)
            .await
            .wrap_err_with(|| format!("failed to open for resume: {}", part_path.display()))?;
        f.seek(std::io::SeekFrom::End(0)).await?;
        f
    } else {
        resume_offset = 0;
        tokio::fs::File::create(&part_path)
            .await
            .wrap_err_with(|| format!("failed to create {}", part_path.display()))?
    };

    // SHA256: if resuming, we can't incrementally hash — we'll verify the whole file at the end.
    // If fresh download, hash as we go.
    let mut hasher = if !resuming { Some(Sha256::new()) } else { None };

    let mut bytes_downloaded = resume_offset;
    let mut stream = resp.bytes_stream();

    // Speed tracking: sliding window
    let mut speed_tracker = SpeedTracker::new();

    loop {
        tokio::select! {
            biased;

            _ = cancel.cancelled() => {
                drop(file);
                bail!("download cancelled");
            }

            // A mirror that accepts the connection and then stops sending must
            // not hold the installation open forever: connect_timeout does not
            // cover an already-established transfer, so bound the wait for each
            // chunk. The .part file is kept so the retry resumes from here.
            chunk = tokio::time::timeout(config.idle_timeout, stream.next()) => {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(_) => {
                        drop(file);
                        bail!(
                            "no data from {mirror} for {:?} ({bytes_downloaded} of {total_size} bytes)",
                            config.idle_timeout
                        );
                    }
                };

                match chunk {
                    Some(Ok(bytes)) => {
                        let len = bytes.len() as u64;
                        if let Some(ref mut h) = hasher {
                            h.update(&bytes);
                        }
                        file.write_all(&bytes).await
                            .wrap_err("failed to write chunk")?;

                        bytes_downloaded += len;
                        speed_tracker.record(len);

                        // Update progress state
                        shared.update_package(index, PackageState::Downloading {
                            filename: filename.to_string(),
                            downloaded: bytes_downloaded,
                            total: total_size,
                            speed_bps: speed_tracker.speed_bps(),
                            mirror: mirror.to_string(),
                            attempt,
                        });
                    }
                    Some(Err(e)) => {
                        drop(file);
                        // Don't delete .part — we can resume
                        return Err(e).wrap_err("download stream error");
                    }
                    None => break,
                }
            }
        }
    }

    file.flush().await?;
    drop(file);

    // Verify SHA256
    if let Some(expected) = expected_sha256 {
        shared.update_package(
            index,
            PackageState::Verifying {
                filename: filename.to_string(),
                bytes: bytes_downloaded,
            },
        );

        let actual = if let Some(hasher) = hasher {
            // Fresh download — use incremental hash
            hex::encode(hasher.finalize())
        } else {
            // Resumed download — hash the whole file
            sha256_file_async(&part_path).await?
        };

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

    Ok(bytes_downloaded)
}

// ── Speed tracker ─────────────────────────────────────

/// Sliding window speed tracker over the last 5 seconds.
struct SpeedTracker {
    window: Vec<(Instant, u64)>,
    window_duration: Duration,
}

impl SpeedTracker {
    fn new() -> Self {
        Self {
            window: Vec::new(),
            window_duration: Duration::from_secs(5),
        }
    }

    fn record(&mut self, bytes: u64) {
        let now = Instant::now();
        self.window.push((now, bytes));
        // Trim entries older than window
        let cutoff = now - self.window_duration;
        self.window.retain(|(t, _)| *t >= cutoff);
    }

    fn speed_bps(&self) -> u64 {
        if self.window.len() < 2 {
            return 0;
        }
        let total_bytes: u64 = self.window.iter().map(|(_, b)| b).sum();
        let elapsed = self
            .window
            .last()
            .unwrap()
            .0
            .duration_since(self.window.first().unwrap().0);
        if elapsed.is_zero() {
            return 0;
        }
        (total_bytes as f64 / elapsed.as_secs_f64()) as u64
    }
}

// ── Utility functions ─────────────────────────────────

fn part_file_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

/// Buffer used when hashing a file. Large enough to keep syscall overhead
/// negligible, small enough that several concurrent hashes cost nothing.
const HASH_CHUNK_BYTES: usize = 1024 * 1024;

/// SHA256 of a file, read in chunks.
///
/// Reading the file whole would hold a package in memory in full — some are
/// several hundred megabytes, several are hashed at once, and on a live ISO
/// the filesystem those bytes come from is itself RAM.
fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::{BufRead, BufReader};

    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::with_capacity(HASH_CHUNK_BYTES, file);
    let mut hasher = Sha256::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        hasher.update(chunk);
        let consumed = chunk.len();
        reader.consume(consumed);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// SHA256 of a file without occupying the async runtime while it reads.
async fn sha256_file_async(path: &Path) -> Result<String> {
    let owned = path.to_path_buf();
    tokio::task::spawn_blocking(move || sha256_file(&owned))
        .await?
        .wrap_err_with(|| format!("failed to read {} for SHA256", path.display()))
}

/// Whether a cached package is present and matches its expected digest.
/// A task without a digest trusts the cached file, as libalpm would.
async fn is_cached_and_valid(path: &Path, expected: Option<&str>) -> bool {
    if !path.exists() {
        return false;
    }
    let Some(expected) = expected else {
        return true;
    };
    match sha256_file_async(path).await {
        Ok(actual) => actual == expected,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn cached_file_matching_its_digest_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pkg");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"hello world").unwrap();
        drop(f);

        let expected = hex::encode(Sha256::digest(b"hello world"));
        assert!(is_cached_and_valid(&path, Some(&expected)).await);
    }

    #[tokio::test]
    async fn cached_file_with_a_wrong_digest_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pkg");
        std::fs::write(&path, b"hello world").unwrap();

        assert!(!is_cached_and_valid(&path, Some("0000bad")).await);
    }

    #[tokio::test]
    async fn cached_file_without_a_digest_is_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pkg");
        std::fs::write(&path, b"anything").unwrap();

        assert!(is_cached_and_valid(&path, None).await);
    }

    #[tokio::test]
    async fn missing_cached_file_is_not_valid() {
        assert!(!is_cached_and_valid(Path::new("/nonexistent"), Some("abc")).await);
        // Absent digest must not make a missing file look cached.
        assert!(!is_cached_and_valid(Path::new("/nonexistent"), None).await);
    }

    #[test]
    fn chunked_hash_matches_a_whole_file_digest_across_buffer_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.pkg");
        // Larger than one hash buffer, so the chunked loop runs several times.
        let data: Vec<u8> = (0..HASH_CHUNK_BYTES * 2 + 12345)
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(&path, &data).unwrap();

        assert_eq!(
            sha256_file(&path).unwrap(),
            hex::encode(Sha256::digest(&data))
        );
    }

    #[test]
    fn test_part_file_path() {
        let dest = Path::new("/cache/linux-6.8.1-x86_64.pkg.tar.zst");
        let part = part_file_path(dest);
        assert_eq!(
            part,
            Path::new("/cache/linux-6.8.1-x86_64.pkg.tar.zst.part")
        );
    }

    #[test]
    fn test_speed_tracker() {
        let mut tracker = SpeedTracker::new();
        assert_eq!(tracker.speed_bps(), 0);

        tracker.record(1000);
        // Single entry — can't compute speed
        assert_eq!(tracker.speed_bps(), 0);
    }

    #[test]
    fn test_download_progress_eta() {
        let progress = PackageProgress::Downloading {
            packages: vec![PackageState::Downloading {
                filename: "test".into(),
                downloaded: 50,
                total: 100,
                speed_bps: 10,
                mirror: "mirror".into(),
                attempt: 1,
            }],
            total_bytes: 100,
            downloaded_bytes: 50,
            active_downloads: 1,
            completed: 0,
            failed: 0,
        };

        assert_eq!(progress.total_speed_bps(), 10);
        assert_eq!(progress.eta(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn test_download_progress_eta_zero_speed() {
        let progress = PackageProgress::Downloading {
            packages: vec![PackageState::Queued],
            total_bytes: 100,
            downloaded_bytes: 0,
            active_downloads: 0,
            completed: 0,
            failed: 0,
        };

        assert!(progress.eta().is_none());
    }

    /// A `SharedProgress` over `count` queued packages, plus its receiver.
    fn progress_for(
        count: usize,
        total_bytes: u64,
    ) -> (SharedProgress, watch::Receiver<PackageProgress>) {
        let (tx, rx) = watch::channel(PackageProgress::Downloading {
            packages: vec![PackageState::Queued; count],
            total_bytes,
            downloaded_bytes: 0,
            active_downloads: 0,
            completed: 0,
            failed: 0,
        });
        (SharedProgress { tx: Arc::new(tx) }, rx)
    }

    fn counters(progress: &PackageProgress) -> (usize, usize, usize, u64) {
        match progress {
            PackageProgress::Downloading {
                active_downloads,
                completed,
                failed,
                downloaded_bytes,
                ..
            } => (*active_downloads, *completed, *failed, *downloaded_bytes),
            other => panic!("expected a downloading phase, got {other:?}"),
        }
    }

    #[test]
    fn a_package_found_in_the_cache_counts_as_completed() {
        // Queued straight to Done, with no Downloading in between — the common
        // case when an install is retried and the cache is warm.
        let (shared, rx) = progress_for(2, 300);
        shared.update_package(
            0,
            PackageState::Done {
                filename: "cached.pkg".into(),
                bytes: 100,
            },
        );

        let (active, completed, failed, bytes) = counters(&rx.borrow());
        assert_eq!((active, completed, failed), (0, 1, 0));
        assert_eq!(bytes, 100);
    }

    #[test]
    fn a_retry_after_a_failed_digest_keeps_the_counters_sane() {
        // Downloading -> Verifying -> (mismatch) -> Downloading -> Done is the
        // sequence that used to underflow active_downloads on completion.
        let (shared, rx) = progress_for(1, 100);
        let downloading = |downloaded| PackageState::Downloading {
            filename: "pkg".into(),
            downloaded,
            total: 100,
            speed_bps: 0,
            mirror: "https://mirror.example".into(),
            attempt: 1,
        };

        shared.update_package(0, downloading(100));
        assert_eq!(counters(&rx.borrow()).0, 1);

        shared.update_package(
            0,
            PackageState::Verifying {
                filename: "pkg".into(),
                bytes: 100,
            },
        );
        assert_eq!(counters(&rx.borrow()).0, 0);

        // Digest mismatch: the file is downloaded again from scratch.
        shared.update_package(0, downloading(40));
        shared.update_package(
            0,
            PackageState::Done {
                filename: "pkg".into(),
                bytes: 100,
            },
        );

        let (active, completed, failed, bytes) = counters(&rx.borrow());
        assert_eq!((active, completed, failed), (0, 1, 0));
        assert_eq!(bytes, 100, "a retried package must not be counted twice");
    }

    #[test]
    fn bytes_never_exceed_the_total_when_a_mirror_is_retried() {
        let (shared, rx) = progress_for(1, 100);
        let attempt_bytes = |downloaded, attempt| PackageState::Downloading {
            filename: "pkg".into(),
            downloaded,
            total: 100,
            speed_bps: 0,
            mirror: "https://mirror.example".into(),
            attempt,
        };

        // First mirror stalls after 80 bytes, second starts over.
        shared.update_package(0, attempt_bytes(80, 1));
        shared.update_package(0, attempt_bytes(30, 2));

        let (_, _, _, bytes) = counters(&rx.borrow());
        assert_eq!(bytes, 30, "the abandoned attempt's bytes must not linger");
    }

    #[test]
    fn test_download_config_default() {
        let config = DownloadConfig::default();
        assert_eq!(config.concurrency, 5);
        assert_eq!(config.retries_per_mirror, 3);
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
        assert_eq!(config.idle_timeout, Duration::from_secs(30));
    }
}
