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

| Port  | Container       | Shaping             | Models                       |
|-------|-----------------|---------------------|------------------------------|
| 18001 | mirror-fast     | none                | top-of-list reflector mirror |
| 18002 | mirror-medium   | tbf rate=10mbit     | mid-list mirror              |
| 18003 | mirror-slow     | tbf rate=500kbit    | bottom-of-list mirror        |
| 18004 | mirror-flaky    | netem loss=30%      | lossy / unstable mirror      |
| 18999 | (none)          | n/a                 | dead mirror — connection refused |

The "dead mirror" scenario is modeled by simply pointing the bench at a
port nothing listens on.

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
  unreachable.

### `concurrency_sweep`

Single fast mirror, sweeping concurrency = 1, 3, 5, 10, 20. Shows the curve
of where parallelism stops helping for our package mix.

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
