//! Mirror selection measured the way packages are fetched.
//!
//! reflector rates a mirror by one small download, which cannot tell a path
//! that is throttled after its first few hundred kilobytes from a fast one;
//! the installer then spent minutes on mirrors that had rated best. Here the
//! candidates from the official status list are first sorted by how quickly
//! they answer, and the closest are then timed on a sustained transfer of the
//! same kind the package downloader performs. The result is written as a
//! pacman mirrorlist and is the order the downloader starts from.

use std::path::Path;
use std::time::{Duration, Instant};

use color_eyre::eyre::{Context, Result, bail};
use futures::stream::{self, StreamExt};
use serde::Deserialize;

pub const STATUS_URL: &str = "https://archlinux.org/mirrors/status/json/";
/// A small object every mirror serves; only its headers are awaited.
const PROBE_PATH: &str = "core/os/x86_64/core.db";
/// A large object every mirror serves; its first megabytes are sampled.
const SAMPLE_PATH: &str = "extra/os/x86_64/extra.db";
/// A sample shorter than this measured nothing worth ranking.
const MIN_SAMPLE_BYTES: u64 = 64 * 1024;

/// One entry of the mirror status list.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MirrorStatus {
    pub url: String,
    pub protocol: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub country_code: String,
    #[serde(default)]
    pub active: bool,
    /// Share of recent checks the mirror passed, 0..=1.
    #[serde(default)]
    pub completion_pct: f64,
    /// Seconds behind the upstream master; missing when never synced. The
    /// status list reports small negative values for mirrors that are ahead
    /// of the check, so this is signed.
    #[serde(default)]
    pub delay: Option<i64>,
}

#[derive(Deserialize)]
struct StatusFile {
    urls: Vec<MirrorStatus>,
}

#[derive(Debug, Clone)]
pub struct RankOptions {
    /// Country names or codes to restrict to; empty means anywhere.
    pub countries: Vec<String>,
    /// Mirrors further behind upstream than this are not candidates.
    pub max_delay: Duration,
    /// Minimum share of passed checks, 0..=1.
    pub min_completion: f64,
    /// How long a mirror may take to answer the probe.
    pub probe_timeout: Duration,
    /// How many of the quickest-answering mirrors get a throughput sample.
    pub sample_count: usize,
    /// How much of the sample object to request.
    pub sample_bytes: u64,
    /// How long one sample may run; shorter when the bytes arrive first.
    pub sample_time: Duration,
    /// Parallel probes and parallel samples.
    pub probe_concurrency: usize,
    pub sample_concurrency: usize,
    /// How many mirrors the written list keeps.
    pub keep: usize,
}

impl Default for RankOptions {
    fn default() -> Self {
        Self {
            countries: Vec::new(),
            max_delay: Duration::from_secs(12 * 3600),
            min_completion: 0.95,
            probe_timeout: Duration::from_secs(3),
            sample_count: 24,
            sample_bytes: 8 * 1024 * 1024,
            sample_time: Duration::from_secs(5),
            probe_concurrency: 32,
            sample_concurrency: 6,
            keep: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RankedMirror {
    pub url: String,
    pub country: String,
    pub latency: Duration,
    pub bytes_per_sec: u64,
}

/// Mirrors from `status` that are worth measuring, in status-list order.
pub fn candidates(status: &[MirrorStatus], opts: &RankOptions) -> Vec<MirrorStatus> {
    let wanted: Vec<String> = opts
        .countries
        .iter()
        .map(|c| c.trim().to_ascii_lowercase())
        .filter(|c| !c.is_empty())
        .collect();
    status
        .iter()
        .filter(|m| m.active && m.protocol == "https" && m.url.ends_with('/'))
        .filter(|m| m.completion_pct >= opts.min_completion)
        .filter(|m| {
            m.delay
                .is_some_and(|d| Duration::from_secs(d.max(0) as u64) <= opts.max_delay)
        })
        .filter(|m| {
            wanted.is_empty()
                || wanted.contains(&m.country.to_ascii_lowercase())
                || wanted.contains(&m.country_code.to_ascii_lowercase())
        })
        .cloned()
        .collect()
}

pub async fn fetch_status(client: &reqwest::Client) -> Result<Vec<MirrorStatus>> {
    let file: StatusFile = client
        .get(STATUS_URL)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .wrap_err("cannot reach the Arch Linux mirror status")?
        .error_for_status()
        .wrap_err("mirror status request failed")?
        .json()
        .await
        .wrap_err("cannot read the mirror status")?;
    Ok(file.urls)
}

/// Time to the response headers of a small object.
async fn probe(
    client: &reqwest::Client,
    mirror: &MirrorStatus,
    opts: &RankOptions,
) -> Option<Duration> {
    let started = Instant::now();
    let response = client
        .get(format!("{}{PROBE_PATH}", mirror.url))
        .header(reqwest::header::RANGE, "bytes=0-0")
        .timeout(opts.probe_timeout)
        .send()
        .await
        .ok()?;
    response.status().is_success().then(|| started.elapsed())
}

/// Bytes per second over a bounded slice of a large object.
async fn sample(
    client: &reqwest::Client,
    mirror: &MirrorStatus,
    opts: &RankOptions,
) -> Option<u64> {
    let started = Instant::now();
    let response = client
        .get(format!("{}{SAMPLE_PATH}", mirror.url))
        .header(
            reqwest::header::RANGE,
            format!("bytes=0-{}", opts.sample_bytes - 1),
        )
        .timeout(opts.probe_timeout)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut received = 0u64;
    let mut body = response.bytes_stream();
    loop {
        let remaining = opts.sample_time.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, body.next()).await {
            Ok(Some(Ok(chunk))) => received += chunk.len() as u64,
            Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
        }
    }
    if received < MIN_SAMPLE_BYTES {
        return None;
    }
    let seconds = started.elapsed().as_secs_f64().max(0.25);
    Some((received as f64 / seconds) as u64)
}

/// Probe every candidate, sample the quickest to answer, and return the
/// mirrors that delivered, fastest first.
pub async fn rank(
    client: &reqwest::Client,
    candidates: Vec<MirrorStatus>,
    opts: &RankOptions,
) -> Vec<RankedMirror> {
    tracing::info!(count = candidates.len(), "probing mirrors");
    let mut probed: Vec<(Duration, MirrorStatus)> = stream::iter(candidates)
        .map(|m| async move { probe(client, &m, opts).await.map(|latency| (latency, m)) })
        .buffer_unordered(opts.probe_concurrency.max(1))
        .filter_map(|r| async move { r })
        .collect()
        .await;
    probed.sort_by_key(|(latency, _)| *latency);
    probed.truncate(opts.sample_count);
    tracing::info!(count = probed.len(), "measuring mirror throughput");
    let mut ranked: Vec<RankedMirror> = stream::iter(probed)
        .map(|(latency, m)| async move {
            let bytes_per_sec = sample(client, &m, opts).await?;
            tracing::debug!(url = %m.url, bytes_per_sec, latency_ms = latency.as_millis() as u64, "mirror measured");
            Some(RankedMirror {
                url: m.url,
                country: m.country,
                latency,
                bytes_per_sec,
            })
        })
        .buffer_unordered(opts.sample_concurrency.max(1))
        .filter_map(|r| async move { r })
        .collect()
        .await;
    ranked.sort_by(|a, b| {
        b.bytes_per_sec
            .cmp(&a.bytes_per_sec)
            .then(a.latency.cmp(&b.latency))
    });
    ranked
}

/// A pacman mirrorlist of the first `keep` mirrors.
pub fn mirrorlist(ranked: &[RankedMirror], keep: usize) -> String {
    let mut out = String::from(
        "# Written by archinstall_zfs: mirrors measured on a sustained transfer,\n# fastest first.\n",
    );
    for m in ranked.iter().take(keep) {
        out.push_str(&format!(
            "# {} — {} KiB/s, {} ms\nServer = {}$repo/os/$arch\n",
            if m.country.is_empty() {
                "?"
            } else {
                &m.country
            },
            m.bytes_per_sec / 1024,
            m.latency.as_millis(),
            m.url
        ));
    }
    out
}

/// Fetch the status, rank, and replace the mirrorlist at `path` atomically.
/// The existing file is left alone when nothing could be measured.
pub async fn refresh(path: &Path, opts: &RankOptions) -> Result<Vec<RankedMirror>> {
    let client = reqwest::Client::builder()
        .user_agent("archinstall-zfs-rs")
        .connect_timeout(opts.probe_timeout)
        .build()
        .wrap_err("failed to create HTTP client")?;
    let status = fetch_status(&client).await?;
    let candidates = candidates(&status, opts);
    if candidates.is_empty() {
        bail!("no mirror in the status list matches {:?}", opts.countries);
    }
    let ranked = rank(&client, candidates, opts).await;
    if ranked.is_empty() {
        bail!("no mirror delivered data; the current mirrorlist is kept");
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("azfs.tmp");
    std::fs::write(&temporary, mirrorlist(&ranked, opts.keep))
        .wrap_err_with(|| format!("cannot write {}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .wrap_err_with(|| format!("cannot replace {}", path.display()))?;
    tracing::info!(
        best = %ranked[0].url,
        kib_per_sec = ranked[0].bytes_per_sec / 1024,
        kept = ranked.len().min(opts.keep),
        "mirrorlist written"
    );
    Ok(ranked)
}

pub const LIVE_MIRRORLIST: &str = "/etc/pacman.d/mirrorlist";

/// Rank mirrors for the live medium once per process: the wizard does it
/// while the user is still on the welcome step, and the pipeline's own call
/// then costs nothing. A failure keeps the medium's list and is retried by
/// the next caller.
pub fn refresh_live_once() -> Result<()> {
    static DONE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if DONE.get().is_some() {
        return Ok(());
    }
    refresh_blocking(Path::new(LIVE_MIRRORLIST), RankOptions::default())?;
    let _ = DONE.set(());
    Ok(())
}

/// [`refresh`] for callers without an async context. Runs on its own
/// thread with its own runtime, so it is safe inside `spawn_blocking`.
pub fn refresh_blocking(path: &Path, opts: RankOptions) -> Result<Vec<RankedMirror>> {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .wrap_err("cannot start a runtime for mirror ranking")?
            .block_on(refresh(&path, &opts))
    })
    .join()
    .map_err(|_| color_eyre::eyre::eyre!("mirror ranking thread panicked"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(url: &str, country: &str, code: &str) -> MirrorStatus {
        MirrorStatus {
            url: url.into(),
            protocol: "https".into(),
            country: country.into(),
            country_code: code.into(),
            active: true,
            completion_pct: 1.0,
            delay: Some(600),
        }
    }

    #[test]
    fn candidates_keep_only_current_https_mirrors_of_the_wanted_countries() {
        let mut stale = status("https://stale.example/", "Germany", "DE");
        stale.delay = Some(2 * 24 * 3600);
        let mut flaky = status("https://flaky.example/", "Germany", "DE");
        flaky.completion_pct = 0.5;
        let mut plain = status("http://plain.example/", "Germany", "DE");
        plain.protocol = "http".into();
        let mut never = status("https://never.example/", "Germany", "DE");
        never.delay = None;
        let list = vec![
            status("https://de.example/", "Germany", "DE"),
            status("https://nl.example/", "Netherlands", "NL"),
            stale,
            flaky,
            plain,
            never,
        ];
        let opts = RankOptions::default();
        let urls = |v: Vec<MirrorStatus>| v.into_iter().map(|m| m.url).collect::<Vec<_>>();
        assert_eq!(
            urls(candidates(&list, &opts)),
            ["https://de.example/", "https://nl.example/"]
        );
        let by_name = RankOptions {
            countries: vec!["netherlands".into()],
            ..RankOptions::default()
        };
        assert_eq!(urls(candidates(&list, &by_name)), ["https://nl.example/"]);
        let by_code = RankOptions {
            countries: vec!["de".into()],
            ..RankOptions::default()
        };
        assert_eq!(urls(candidates(&list, &by_code)), ["https://de.example/"]);
    }

    #[test]
    fn the_status_file_parses_the_fields_the_ranking_needs() {
        let json = r#"{"cutoff": 86400, "urls": [
            {"url": "https://mirror.example/archlinux/", "protocol": "https", "country": "Finland",
             "country_code": "FI", "active": true, "completion_pct": 1.0, "delay": 1234,
             "score": 1.5, "isos": true, "ipv4": true, "ipv6": true, "details": "x"},
            {"url": "https://ahead.example/", "protocol": "https", "country": "Finland",
             "country_code": "FI", "active": true, "completion_pct": 1.0, "delay": -59},
            {"url": "rsync://mirror.example/archlinux/", "protocol": "rsync", "country": "",
             "country_code": "", "active": true, "completion_pct": 1.0, "delay": null}
        ]}"#;
        let file: StatusFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.urls.len(), 3);
        assert_eq!(file.urls[0].country_code, "FI");
        assert_eq!(file.urls[2].delay, None);
        // The rsync entry is dropped; the one reported ahead of the check stays.
        assert_eq!(candidates(&file.urls, &RankOptions::default()).len(), 2);
    }

    #[test]
    fn the_mirrorlist_keeps_the_fastest_and_pacman_syntax() {
        let ranked = vec![
            RankedMirror {
                url: "https://fast.example/archlinux/".into(),
                country: "Finland".into(),
                latency: Duration::from_millis(40),
                bytes_per_sec: 20 * 1024 * 1024,
            },
            RankedMirror {
                url: "https://slow.example/".into(),
                country: String::new(),
                latency: Duration::from_millis(300),
                bytes_per_sec: 100 * 1024,
            },
        ];
        let text = mirrorlist(&ranked, 1);
        assert!(text.contains("Server = https://fast.example/archlinux/$repo/os/$arch\n"));
        assert!(!text.contains("slow.example"));
        assert!(text.contains("# Finland — 20480 KiB/s, 40 ms"));
        assert!(mirrorlist(&ranked, 5).contains("# ? — 100 KiB/s, 300 ms"));
    }
}
