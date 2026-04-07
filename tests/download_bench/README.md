# Download bench harness

Local nginx-based mirror lab for benchmarking `download_packages()` against
a deterministic, controlled environment. No real Arch mirrors, no network
noise — A/B comparisons between download strategies are meaningful.

## What's here

```
tests/download_bench/
├── docker-compose.yml      4 nginx containers, ports 18001..18004
├── nginx/
│   ├── nginx.conf          minimal HTTP/2 static-file server
│   └── entrypoint.sh       applies tc qdisc rules then exec nginx
├── generate-packages.sh    creates ~50 synthetic .pkg.tar.zst files
├── packages/               (gitignored) generated files + manifest.json
└── README.md               this file
```

The actual benchmark code lives in `core/benches/download.rs` and runs via
`cargo bench`.

## Mirror containers

Each container serves the **same** package set on a different port, with a
different `tc qdisc` shaping rule:

| Port  | Container       | Shaping              | Protocol  | Models                           |
|-------|-----------------|----------------------|-----------|----------------------------------|
| 18001 | mirror-fast     | none                 | HTTP/2    | top-of-list reflector mirror     |
| 18002 | mirror-medium   | tbf rate=10mbit      | HTTP/2    | mid-list mirror                  |
| 18003 | mirror-slow     | tbf rate=500kbit     | HTTP/2    | bottom-of-list mirror            |
| 18004 | mirror-flaky    | netem loss=30%       | HTTP/2    | lossy / unstable mirror          |
| 18005 | mirror-fast2    | none                 | HTTP/2    | second fast host (multi-host)    |
| 18006 | mirror-fast3    | none                 | HTTP/2    | third fast host (multi-host)     |
| 18007 | mirror-http1    | none                 | HTTP/1.1  | HTTP/1.1 baseline                |
| 18008 | mirror-laggy    | netem delay=50ms     | HTTP/2    | geographically distant mirror    |
| 18999 | (none)          | n/a                  | n/a       | dead mirror — connection refused |

The "dead mirror" scenario is modeled by simply pointing the bench at a
port nothing listens on.

`mirror-http1` uses a separate nginx config (`nginx/nginx-http1.conf`)
with `http2 on;` removed so the bench can compare HTTP/2 multiplexing
vs HTTP/1.1 keep-alive on a single host.

`mirror-fast2` and `mirror-fast3` are deliberately on different ports so
reqwest sees them as distinct origins (scheme + host + port) and opens
**separate TCP connections** to each. Without the port difference HTTP/2
would multiplex everything over one connection and the multi-host bench
group would degenerate to single-host.

Containers need `NET_ADMIN` to apply tc rules. The compose file adds the
capability; tc rules are applied at container startup by `nginx/entrypoint.sh`.

## One-time setup

```sh
# Generate ~200 MB of synthetic packages and a manifest
./generate-packages.sh

# Start the mirror containers
docker compose up -d

# Sanity check that mirror-fast is reachable
curl http://127.0.0.1:18001/healthz   # → ok
```

The packages are random bytes sized to match the distribution of a real
Arch install (sampled from a 6121-package cache):

- ~32 % < 100 KB (small headers, themes)
- ~35 % 100 KB – 1 MB (typical libs)
- ~21 % 1 MB – 10 MB (medium apps)
- ~12 % 10 MB – 50 MB (firmware-like; capped at 50 MB for bench speed)

Total ~200 MB across ~50 files. Generation takes a few seconds.

## Running benchmarks

```sh
# All benchmark groups
cargo bench -p archinstall-zfs-core --bench download

# Single group (criterion's filter)
cargo bench -p archinstall-zfs-core --bench download -- full_install

# Establish a baseline before refactoring
cargo bench -p archinstall-zfs-core --bench download -- --save-baseline before

# Apply changes, compare against the saved baseline
cargo bench -p archinstall-zfs-core --bench download -- --baseline before
```

The bench fails fast with clear instructions if the docker harness isn't
reachable on `127.0.0.1:18001`.

## Benchmark groups

### `full_install`

Three end-to-end scenarios at concurrency 5:

- `all_mirrors_concurrency_5` — list `[fast, medium, slow, flaky]`. Tests
  default behavior with a realistic mirror mix.
- `single_fast_concurrency_5` — only the fast mirror in the list. Measures
  the upper bound: HTTP/2 multiplexing on one well-behaved host.
- `dead_first_then_fast` — `[dead, fast]`. Measures how much wall-time the
  retry/fallback path costs when a mirror at the top of the list is
  unreachable. Uses a 5-package subset to bound the exponential-backoff
  cost (otherwise this group would take 10+ minutes per sample).

### `concurrency_sweep`

Single fast mirror, sweeping concurrency = 1, 3, 5, 10, 20. Shows the curve
of where parallelism stops helping for our package mix.

### `multi_host`

Three distinct fast hosts (`fast`, `fast2`, `fast3`) in the mirror list,
each on a separate port = separate origin = separate TCP connection. On
`main` the current "always try mirrors[0] first" code only ever uses the
first host, so this group establishes the **baseline** any future
multi-host strategy (round-robin / spread / slowness-failover) must
beat.

- `three_fast_concurrency_5` — three fast hosts, conc=5
- `three_fast_concurrency_10` — three fast hosts, conc=10

### `protocol`

HTTP/2 multiplexing vs HTTP/1.1 keep-alive on the same single host
otherwise. Both servers identical except for the protocol negotiated.

- `http2_single_host` — `[fast]` (HTTP/2)
- `http1_single_host` — `[http1]` (HTTP/1.1)

### `latency`

Geographic-distance mirror modeled with `tc qdisc netem delay 50ms`.
Every request pays a 50 ms RTT for the TCP handshake plus per-stream
HTTP/2 setup. Tests whether parallelism still helps when the bottleneck
is RTT instead of bandwidth.

- `low_latency` — reference point on `[fast]` (no delay), conc=5
- `geo_50ms_concurrency_5` — `[laggy]` at 50 ms RTT, conc=5
- `geo_50ms_concurrency_10` — `[laggy]` at 50 ms RTT, conc=10

The latency group has `measurement_time = 120s` (vs 60s for the others)
because each iter is RTT-dominated and slower.

### `size_profile`

- `small_only` — the 16 small files (~30-90 KB each). Slow-start cost
  dominates per task; this is where parallelism helps the most.
- `huge_only` — the 5 large files (15-50 MB each). Single-connection
  throughput dominates; extra parallelism mostly doesn't help.

## Tearing down

```sh
docker compose down
```

The `packages/` directory survives across runs. Re-run
`generate-packages.sh` if you want to change sizes / counts; it wipes and
recreates.

## Notes / caveats

- This measures `download_packages()` end-to-end including SHA-256
  verification. SHA-256 is not the bottleneck (~500 MB/s software, faster
  with SHA-NI), but it's part of the real code path so we don't fake it
  out.
- The harness models *transport* characteristics (rate, loss, dead host)
  but not CDN-specific behavior like edge caching, geographic routing, or
  per-IP throttling. For end-to-end validation against real mirrors, run a
  manual install via `cargo xtask test-install` on a fresh disk.
- Network conditions inside Docker on Linux are very close to bare-metal
  for HTTP — the `tc qdisc` rules apply at the container's eth0 interface.
  If you see suspicious results, sanity-check with `docker exec
  bench-mirror-slow tc qdisc show dev eth0`.
