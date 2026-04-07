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

> ⚠️ Most numbers are loopback-bound. Local nginx over HTTP/2 has no
> realistic per-IP throttle and (except in the `latency` group) no
> RTT, no CDN behavior, and no real mirror congestion. The
> non-`latency` numbers establish the **upper bound** of what
> `download_packages()` can achieve given the algorithmic and code-path
> overhead — not what you'd see against real Arch mirrors. The
> `latency` group with `tc qdisc netem delay 50ms` is the closest the
> harness gets to real conditions, and it shows that **RTT dominates
> wall-time on realistic mirrors** by 6-8×. For end-to-end validation
> against real mirrors, use `cargo xtask test-install` on a fresh disk.

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

## `multi_host` group — multiple distinct fast hosts

Three separate fast nginx containers (`fast`, `fast2`, `fast3`) on
different ports = different origins = separate TCP connections from
reqwest's pool. Tests whether having multiple hosts in the mirror list
changes anything for the current code.

| Scenario | Concurrency | Median time | vs `single_fast` |
|---|---|---|---|
| `three_fast_concurrency_5` | 5 | **171.62 ms** | −1% (within noise) |
| `three_fast_concurrency_10` | 10 | **180.80 ms** | +4% |

### Observations

- **`multi_host` ≈ `single_fast`** (172 ms vs 174 ms). Confirms what we
  suspected from `full_install`: the current code only ever uses
  `mirrors[0]`. Adding more hosts to the list doesn't help because the
  fallback path never runs.

- **Concurrency=10 still slightly worse than concurrency=5** even with
  multiple hosts available. With the current single-host strategy this
  is expected (only mirror[0] is used, so HTTP/2 multiplexing limits
  apply same as `concurrency_sweep`). A multi-host strategy that
  actually distributed tasks across hosts should beat this — but we
  haven't built one yet. **This number is the baseline that any future
  spread / round-robin attempt has to beat.**

## `protocol` group — HTTP/2 vs HTTP/1.1 on a single host

| Scenario | Protocol | Median time |
|---|---|---|
| `http2_single_host` | HTTP/2 (h2c) | **172.80 ms** |
| `http1_single_host` | HTTP/1.1 keep-alive | **173.13 ms** |

### Observations

- **HTTP/2 buys us nothing on loopback** (172.80 vs 173.13 ms — same
  within noise). On a high-bandwidth low-latency link reqwest's
  HTTP/1.1 keep-alive serializes 50 small requests over a few pooled
  connections fast enough that multiplexing latency wins are zero.

- **This is loopback-specific.** On real mirrors with TCP slow-start
  and per-connection bandwidth caps, HTTP/2 multiplexing usually wins
  by a substantial margin because all requests share one already-warm
  congestion window. We'd need a `tc qdisc tbf` rate limit on the
  http1 mirror to see the protocol difference manifest. Future work.

- **Useful as a sanity check**: confirms our reqwest client handles
  HTTP/1.1 fallback transparently and doesn't accidentally cripple
  itself when HTTP/2 isn't available.

## `latency` group — RTT-bound scenarios

`mirror-laggy` adds `tc qdisc netem delay 50ms` to model a
geographically distant mirror. Every request pays a 50 ms RTT for the
TCP handshake plus per-stream HTTP/2 setup.

| Scenario | Latency | Concurrency | Median time | vs `low_latency` |
|---|---|---|---|---|
| `low_latency` | ~0 ms (loopback) | 5 | **173.87 ms** | — |
| `geo_50ms_concurrency_5` | 50 ms | 5 | **1.4277 s** | **8.2× slower** |
| `geo_50ms_concurrency_10` | 50 ms | 10 | **1.1565 s** | **6.6× slower** |

### Observations

This is the **most important finding** in the entire bench suite.

- **50 ms RTT costs ~1.25 seconds wall-time per install** on top of the
  baseline. Real mirrors with realistic RTTs (20-200 ms) will be the
  dominant cost in `download_packages()`, dwarfing everything our
  algorithmic choices can do.

- **At 50 ms RTT, concurrency=10 beats concurrency=5 by 19 %.**
  (1.43 s → 1.16 s.) This is the **opposite** of what we saw on
  loopback in `concurrency_sweep`, where conc=10 was a regression.

- **The reason**: when RTT dominates, parallelism amortizes the
  per-stream setup latency. With concurrency=5, only 5 streams have
  their RTT in flight at any moment. With concurrency=10, 10 streams
  share the RTT cost — they all wait for the same network round trip
  in parallel instead of serially. HTTP/2 multiplexes them over one
  TCP connection so the connection-setup RTT is paid once, but each
  stream's first response still needs a round trip.

- **The optimal concurrency depends on RTT, not on bandwidth.** Our
  current default of 5 was chosen for low-RTT loopback-style
  conditions. On real mirrors with 50-200 ms RTT, the optimum is
  probably 10-20.

### Implication for `DownloadConfig::default()`

If we picked the default concurrency value based purely on the
loopback `concurrency_sweep` numbers we'd lower it to 3. **Don't do
that.** The latency group shows that on realistic-RTT mirrors a
default of 5 is already conservative — the right answer is probably
**8-10**, but we'd want to confirm with a real-mirror smoke test
before changing the default.

## What we still need to measure

The most important real-world questions the current bench *doesn't*
answer:

- **Multi-host benefit at high RTT.** All our `multi_host` scenarios
  use loopback (0 ms RTT). The interesting question is whether having
  3 hosts at 50 ms RTT, with conc=10 distributed across them (3-4
  streams per host), beats 1 host at 50 ms RTT with all 10 streams
  multiplexed over one connection. Needs `mirror-fast2-laggy` and
  `mirror-fast3-laggy` containers, plus a multi-host strategy in code
  (round-robin / spread) before it's worth measuring.

- **HTTP/2 advantage at high RTT.** HTTP/2 multiplexing's real benefit
  shows up when RTT × number-of-requests dominates. Adding `tc qdisc
  netem delay 50ms` to `mirror-http1` would let us compare HTTP/2 vs
  HTTP/1.1 under realistic RTT conditions.

- **Real mirror smoke test.** Phase 4 work — a one-shot bench against
  3-5 real Arch mirrors via reflector to validate that everything we
  learned in the lab holds up on the real internet.

## File index

- [`docker-compose.yml`](docker-compose.yml) — mirror containers
- [`generate-packages.sh`](generate-packages.sh) — synthetic package generator
- [`README.md`](README.md) — setup, scenarios, caveats
- [`../../core/benches/download.rs`](../../core/benches/download.rs) — the criterion bench itself
