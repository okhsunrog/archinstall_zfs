//! Download throughput benchmark.
//!
//! Measures `download_packages()` against a local nginx-based mirror
//! harness defined in `tests/download_bench/`. Run the harness first:
//!
//! ```sh
//! tests/download_bench/generate-packages.sh        # one-time, ~200MB
//! docker compose -f tests/download_bench/docker-compose.yml up -d
//! cargo bench -p archinstall-zfs-core --bench download
//! ```
//!
//! Each criterion group exercises a different scenario (mirror mix,
//! concurrency, package-size profile). The bench fails fast with a clear
//! error if the docker harness isn't reachable on `localhost:18001`.
//!
//! ## Why we measure this way
//!
//! - Synthetic packages of realistic size distribution mean we test the
//!   real `DownloadTask` code path including SHA-256 verification.
//! - Local nginx + tc qdisc gives deterministic mirror behavior — no
//!   real-world network noise — so A/B comparisons between download
//!   strategies are meaningful.
//! - Each iteration uses a fresh `TempDir` cache, so the on-disk
//!   skip-if-cached fast path doesn't pollute timings.

use std::path::PathBuf;
use std::time::Duration;

use archinstall_zfs_core::system::async_download::{DownloadTask, download_packages};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use serde::Deserialize;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

// ── Mirror endpoints from tests/download_bench/docker-compose.yml ──

const MIRROR_FAST: &str = "http://127.0.0.1:18001/pkgs";
const MIRROR_MEDIUM: &str = "http://127.0.0.1:18002/pkgs";
const MIRROR_SLOW: &str = "http://127.0.0.1:18003/pkgs";
const MIRROR_FLAKY: &str = "http://127.0.0.1:18004/pkgs";
const MIRROR_FAST2: &str = "http://127.0.0.1:18005/pkgs";
const MIRROR_FAST3: &str = "http://127.0.0.1:18006/pkgs";
const MIRROR_HTTP1: &str = "http://127.0.0.1:18007/pkgs";
const MIRROR_LAGGY: &str = "http://127.0.0.1:18008/pkgs";
/// A port nothing listens on — used to model "dead mirror at top of list".
const MIRROR_DEAD: &str = "http://127.0.0.1:18999/pkgs";

const HEALTH_URL: &str = "http://127.0.0.1:18001/healthz";

// ── Manifest loading ──

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // size is read by serde and used by .size below; manifest field is the source of truth.
struct ManifestEntry {
    filename: String,
    size: i64,
    sha256: String,
}

fn manifest_path() -> PathBuf {
    // CARGO_MANIFEST_DIR points at core/ when this bench runs.
    let core_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    core_dir
        .parent()
        .expect("core/ has a parent")
        .join("tests/download_bench/packages/manifest.json")
}

fn load_manifest() -> Vec<ManifestEntry> {
    let path = manifest_path();
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "Failed to read {}: {e}\n\n\
             Run `tests/download_bench/generate-packages.sh` first to \
             create the synthetic packages and manifest.",
            path.display()
        )
    });
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "Failed to parse {}: {e}\n\
             Re-run generate-packages.sh — manifest may be corrupt.",
            path.display()
        )
    })
}

fn check_harness_up(rt: &Runtime) {
    let ok = rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .expect("build reqwest client");
        client
            .get(HEALTH_URL)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    });
    if !ok {
        panic!(
            "\n\nDownload bench harness is not reachable on {HEALTH_URL}.\n\n\
             Start it with:\n\
             \n\
             \x20\x20\x20 docker compose -f tests/download_bench/docker-compose.yml up -d\n\n\
             Then re-run `cargo bench -p archinstall-zfs-core --bench download`.\n"
        );
    }
}

// ── Task construction ──
//
// `DownloadTask` is not `Clone`, and `download_packages` consumes the
// `Vec<DownloadTask>`. Each criterion iteration therefore has to build a
// fresh task list. We accept that overhead — it's negligible compared to
// the network transfer the bench is actually measuring.

fn make_tasks(manifest: &[ManifestEntry], mirrors: &[&str]) -> Vec<DownloadTask> {
    manifest
        .iter()
        .map(|entry| DownloadTask {
            filename: entry.filename.clone(),
            servers: mirrors.iter().map(|s| (*s).to_string()).collect(),
            sha256: Some(entry.sha256.clone()),
            size: entry.size,
        })
        .collect()
}

// ── The runner ──

/// Run one download_packages call against a fresh tempdir cache. Returns
/// the wall-clock time the criterion harness reports as the iter time.
async fn run_download(tasks: Vec<DownloadTask>, concurrency: usize) {
    let cache = tempfile::tempdir().expect("tempdir");
    let cancel = CancellationToken::new();
    download_packages(tasks, cache.path().to_path_buf(), concurrency, cancel, None)
        .await
        .expect("download_packages succeeded");
    // tempdir drops here, cleaning up the cache for the next iteration
}

// ── Benchmark groups ──

fn bench_full_install(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    check_harness_up(&rt);
    let manifest = load_manifest();

    let mut group = c.benchmark_group("full_install");
    // Each iteration downloads ~200 MB; keep sample count low so the bench
    // finishes in reasonable time.
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    // Baseline: all four live mirrors, default concurrency.
    group.bench_function("all_mirrors_concurrency_5", |b| {
        let mirrors = [MIRROR_FAST, MIRROR_MEDIUM, MIRROR_SLOW, MIRROR_FLAKY];
        b.to_async(&rt)
            .iter(|| run_download(make_tasks(&manifest, &mirrors), 5));
    });

    // Single fast mirror — measures HTTP/2 multiplexing benefit.
    group.bench_function("single_fast_concurrency_5", |b| {
        b.to_async(&rt)
            .iter(|| run_download(make_tasks(&manifest, &[MIRROR_FAST]), 5));
    });

    // Dead mirror first — measures retry/fallback overhead. Uses a tiny
    // 5-package subset because every task pays a fixed ~7s exponential
    // backoff (1+2+4) before falling through to MIRROR_FAST. With the
    // full 50-package set this group would take 10+ minutes per sample.
    group.bench_function("dead_first_then_fast", |b| {
        let mirrors = [MIRROR_DEAD, MIRROR_FAST];
        let subset: Vec<ManifestEntry> = manifest.iter().take(5).cloned().collect();
        b.to_async(&rt)
            .iter(|| run_download(make_tasks(&subset, &mirrors), 5));
    });

    group.finish();
}

fn bench_concurrency_sweep(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    check_harness_up(&rt);
    let manifest = load_manifest();

    let mut group = c.benchmark_group("concurrency_sweep");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    // Single fast mirror, vary concurrency. Shows where parallelism stops
    // helping for our package mix.
    for &conc in &[1usize, 3, 5, 10, 20] {
        group.bench_with_input(BenchmarkId::new("fast_mirror", conc), &conc, |b, &conc| {
            b.to_async(&rt)
                .iter(|| run_download(make_tasks(&manifest, &[MIRROR_FAST]), conc));
        });
    }

    group.finish();
}

/// `multi_host` — exercises multiple distinct origins (separate TCP
/// connections to fast2/fast3 in addition to fast). On `main` the current
/// "always try mirrors[0] first" code only ever hits the first mirror,
/// so these scenarios serve as the **baseline against which any future
/// multi-host strategy** (round-robin, spread, slowness-failover) must
/// be compared.
fn bench_multi_host(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    check_harness_up(&rt);
    let manifest = load_manifest();

    let mut group = c.benchmark_group("multi_host");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    // Three distinct fast hosts in the mirror list. Different ports →
    // different origins → reqwest opens separate TCP connections to
    // each, instead of multiplexing everything over one HTTP/2 session.
    // Current code always picks mirror[0] (fast), so this measures the
    // *upper bound* a single-host strategy can achieve when the list
    // would have allowed parallelism. Any future spread / round-robin
    // implementation must beat this baseline to be worth shipping.
    let three_fast = [MIRROR_FAST, MIRROR_FAST2, MIRROR_FAST3];

    group.bench_function("three_fast_concurrency_5", |b| {
        b.to_async(&rt)
            .iter(|| run_download(make_tasks(&manifest, &three_fast), 5));
    });

    group.bench_function("three_fast_concurrency_10", |b| {
        b.to_async(&rt)
            .iter(|| run_download(make_tasks(&manifest, &three_fast), 10));
    });

    group.finish();
}

/// `protocol` — HTTP/2 multiplexing vs HTTP/1.1 keep-alive on the same
/// single host. Both servers are otherwise identical (same nginx, same
/// loopback, no shaping). Difference is purely the protocol negotiated.
fn bench_protocol(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    check_harness_up(&rt);
    let manifest = load_manifest();

    let mut group = c.benchmark_group("protocol");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    group.bench_function("http2_single_host", |b| {
        b.to_async(&rt)
            .iter(|| run_download(make_tasks(&manifest, &[MIRROR_FAST]), 5));
    });

    group.bench_function("http1_single_host", |b| {
        b.to_async(&rt)
            .iter(|| run_download(make_tasks(&manifest, &[MIRROR_HTTP1]), 5));
    });

    group.finish();
}

/// `latency` — RTT-bound scenarios. The `mirror-laggy` container has
/// `tc qdisc netem delay 50ms` on its eth0, so every request pays a
/// 50ms RTT for the TCP handshake plus per-stream HTTP/2 setup. Models
/// a geographically distant mirror.
///
/// The interesting question this group answers: does parallelism still
/// help when the bottleneck is RTT instead of bandwidth?
fn bench_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    check_harness_up(&rt);
    let manifest = load_manifest();

    let mut group = c.benchmark_group("latency");
    group.sample_size(10);
    // Latency-bound iters take longer; budget more time per sample.
    group.measurement_time(Duration::from_secs(120));

    // Reference point: same workload on the no-shaping mirror so we can
    // read off the pure RTT cost as the delta.
    group.bench_function("low_latency", |b| {
        b.to_async(&rt)
            .iter(|| run_download(make_tasks(&manifest, &[MIRROR_FAST]), 5));
    });

    group.bench_function("geo_50ms_concurrency_5", |b| {
        b.to_async(&rt)
            .iter(|| run_download(make_tasks(&manifest, &[MIRROR_LAGGY]), 5));
    });

    // Higher concurrency to test whether parallelism amortizes RTT.
    // With HTTP/2 multiplexing all streams share one connection, so
    // they share the RTT cost — concurrency should help less here than
    // you'd naively expect.
    group.bench_function("geo_50ms_concurrency_10", |b| {
        b.to_async(&rt)
            .iter(|| run_download(make_tasks(&manifest, &[MIRROR_LAGGY]), 10));
    });

    group.finish();
}

// NOTE: a `size_profile` group (small_only / huge_only subsets) was tried
// and removed because criterion's auto-tuned iteration count for
// sub-100ms operations makes the bench fire thousands of network requests
// per sample, exhausting client ephemeral ports and tripping nginx
// connection resets. If you want to re-add it, use `iter_custom` with a
// fixed iteration count instead of `iter`. The realistic ~200MB workload
// in `full_install` covers the same code paths.

criterion_group!(
    benches,
    bench_full_install,
    bench_concurrency_sweep,
    bench_multi_host,
    bench_protocol,
    bench_latency
);
criterion_main!(benches);
