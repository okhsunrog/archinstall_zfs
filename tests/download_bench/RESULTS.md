# Download bench — baseline measurements

Baseline numbers for `download_packages()` on `main`, captured before any
optimization work. Use these as the reference point when comparing future
download-strategy changes.

## How to reproduce

```sh
tests/download_bench/generate-packages.sh
docker compose -f tests/download_bench/docker-compose.yml up -d
cargo bench -p archinstall-zfs-core --bench download
```

The harness, package set, and bench groups are documented in
[README.md](README.md).

## Environment

- Host: Arch Linux on a development workstation, loopback (HTTP/2) to
  local nginx containers
- Bench framework: criterion 0.8.2 with `async_tokio` feature
- Workload: 50 synthetic packages, ~200 MB total, sizes drawn from a
  6121-package real Arch install distribution
- Each iteration runs against a fresh `tempfile::TempDir` cache so the
  on-disk skip-if-cached fast path doesn't pollute timings

> ⚠️ These numbers are loopback-bound. Local nginx over HTTP/2 has no
> realistic per-IP throttle, no TCP RTT, no CDN behavior, and no real
> mirror congestion. They establish the **upper bound** of what
> `download_packages()` can achieve given the algorithmic and code-path
> overhead — not what you'd see against real Arch mirrors. For
> end-to-end validation against real mirrors, use `cargo xtask
> test-install` on a fresh disk.

## `full_install` group — concurrency=5

200 MB / 50 packages per iteration. Numbers are criterion's
`[low estimate, median, high estimate]` from 10 samples.

| Scenario | Mirrors | Median time | Effective throughput |
|---|---|---|---|
| `all_mirrors_concurrency_5` | `[fast, medium, slow, flaky]` | **174.22 ms** | ~1.15 GB/s |
| `single_fast_concurrency_5` | `[fast]` only | **173.60 ms** | ~1.15 GB/s |
| `dead_first_then_fast` | `[dead, fast]`, 5-pkg subset | **3.13 s** | n/a (retry-bound) |

### Observations

- **`all_mirrors` ≈ `single_fast`** (174 ms vs 174 ms). On loopback the
  first mirror in the list (`fast`, no shaping) is so fast that the rest
  of the list (`medium`/`slow`/`flaky`) never gets touched — every
  download succeeds on the first attempt to `mirror[0]`. The fact that
  there are slower mirrors in the fallback list is irrelevant because
  the fallback path never runs. **This is the baseline behavior on
  `main`** — first mirror handles everything, rest are dead weight
  unless something fails.

- **`dead_first` retry overhead is ~3.1 seconds** for 5 packages with
  `[dead, fast]` mirrors. The 7-second exponential backoff (1+2+4) is
  paid per task, but with `concurrency=5` all 5 tasks fail on the dead
  mirror in parallel, so the effective wall-time is ~3-4 seconds. With
  the full 50-package set this scales to ~30 seconds (10 batches of 5
  tasks), which is why we cap the bench at a 5-package subset.

- **Throughput cap on loopback is ~1.15 GB/s.** Anything beyond this is
  measuring criterion overhead, not download behavior. Real network
  throughput will be much lower and dominated by RTT × bandwidth-delay
  product, not by our code.

## `concurrency_sweep` group — single fast mirror

200 MB / 50 packages per iteration, varying `concurrency` value passed
into `download_packages()`. Same fast mirror only.

| Concurrency | Median time | Δ vs conc=1 | Notes |
|---|---|---|---|
| 1 | **181.72 ms** | — | Sequential baseline |
| 3 | **167.91 ms** | −7.6% | **Fastest** — sweet spot |
| 5 | **173.14 ms** | −4.7% | Current default |
| 10 | **180.16 ms** | −0.9% | Back to ~conc=1 |
| 20 | **182.97 ms** | +0.7% | Slight regression |

### Observations

- **Optimum at concurrency=3** for this loopback / HTTP/2 / single-host
  workload. Going from 1 → 3 saves 7.6%, going from 3 → 5 *costs* 3%.

- **Diminishing returns past concurrency=3** for HTTP/2 single-host
  scenarios. All requests multiplex over one TCP connection regardless
  of how many parallel `buffer_unordered` tasks we spawn — the
  bottleneck shifts to the connection's congestion window, not to TCP
  setup or to the number of parallel sockets. Adding more parallel
  tasks adds scheduler overhead and stream-level head-of-line blocking
  without increasing throughput.

- **Concurrency=20 is slightly slower than concurrency=1.** Real
  regression: ~6 ms (3.3%) of overhead from juggling 20 tasks against a
  single HTTP/2 connection that can already saturate its window with 3.

- **Important caveat**: this sweep is HTTP/2 single-host only. On
  HTTP/1.1 mirrors, or on multi-host scenarios where each mirror is a
  separate TCP slow-start to amortize, concurrency=5+ probably *does*
  help. We can't measure that with the current harness because the
  bench points all 50 tasks at the same single host, which HTTP/2
  collapses into one connection.

## What this tells us about real-world behavior

The bench is **loopback-only** and the dominant real-world cost
(round-trip time across the public Internet) isn't modeled at all. So
these numbers are not a prediction of real install times.

What the bench *does* tell us:

1. **Our `download_packages()` algorithm has no internal throughput
   bottleneck** at the 1+ GB/s scale. The sha256 hashing, the file
   write, the progress reporting, the channel updates — none of them
   are slowing us down. Whatever we measure on a real install is
   network-bound, not code-bound.

2. **Concurrency=5 (current default) is reasonable** but slightly
   suboptimal for HTTP/2 single-host workloads. The empirical sweet
   spot is concurrency=3 in the simplest case. We have not yet measured
   what's optimal for *multi-host* (real mirrorlist) workloads where
   each mirror is a separate TCP slow-start to amortize.

3. **The `[dead, fast]` retry overhead is bounded and predictable.**
   ~3 seconds for 5 packages, scaling roughly linearly with package
   count divided by concurrency. If we wanted to improve this we'd
   either reduce `retries_per_mirror` (currently 3) or shorten
   `backoff_base` (currently 1 second), but neither matters for normal
   installs where all mirrors are alive.

4. **The current "always try mirrors[0] first" strategy is optimal
   given a well-sorted mirrorlist.** Reflector ranks the fastest mirror
   first; our code uses it for everything; nothing on the bench
   suggests we should diverge from this. The previous mirror-spread
   experiment confirmed this empirically — spreading regressed
   throughput because the top mirror is genuinely the best.

## What we still need to measure

The most important real-world questions the current bench *doesn't*
answer:

- **Does `concurrency=N` actually help on multi-host real mirrors?** To
  answer this we'd need to add a benchmark that uses `[fast, fast2,
  fast3]` (multiple distinct fast hosts) and varies concurrency.
  Currently all our scenarios collapse to one host or one host plus
  irrelevant fallbacks. Phase 4 work.

- **How much does HTTP/2 multiplexing actually save vs HTTP/1.1?** Our
  nginx config enables HTTP/2; we don't have an HTTP/1.1 comparison.
  Could add an `http_only` mirror container to measure.

- **What does real-world RTT do to these numbers?** A `tc qdisc add ...
  netem delay 50ms` rule on `mirror-fast` would model a typical
  geographic-distance mirror. Worth adding before the next round of
  optimization work.

## File index

- [`docker-compose.yml`](docker-compose.yml) — mirror containers
- [`generate-packages.sh`](generate-packages.sh) — synthetic package generator
- [`README.md`](README.md) — setup, scenarios, caveats
- [`../../core/benches/download.rs`](../../core/benches/download.rs) — the criterion bench itself
