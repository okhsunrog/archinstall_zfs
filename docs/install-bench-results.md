# Real-install benchmark — parallel_downloads concurrency sweep

Measures end-to-end install time at different `parallel_downloads` values
using the xtask QEMU harness with a KDE Plasma profile (574 packages, ~1.3 GB).

## How to reproduce

```sh
cargo build --release
cargo xtask bench-downloads \
    --config xtask/configs/qemu_full_disk_kde.json \
    --concurrency 3,5,10,20 \
    --samples 3 \
    --out-dir bench-results-kde \
    --tmpfs
cargo xtask analyze-metrics --dir bench-results-kde
```

## Environment

- Host: Arch Linux development workstation, KVM acceleration
- Network: residential broadband, Fastly CDN mirrors
- Config: KDE Plasma (`plasma-desktop`, `konsole`, `kate`, `dolphin`, `ark`,
  `plasma-workspace`) + `linux-lts` + dracut + ZFS precompiled
- Packages: 574 total — 187 base system (Phase 4), ~5 ZFS (Phase 6),
  387 KDE + deps (Phase 9)
- 3 samples per concurrency level; median reported for download wall time

## Results

| conc | pkgs | total_MB | dl_med_s | dl_min_s | dl_max_s | avg_MBps | max_MBps | install_s |
|-----:|-----:|---------:|---------:|---------:|---------:|---------:|---------:|----------:|
|    3 |  574 |   1296.1 |     38.6 |     37.9 |     43.7 |     8.53 |    32.08 |      70.2 |
|    5 |  574 |   1296.1 |     32.4 |     32.3 |     33.2 |     7.10 |    24.76 |      57.8 |
|   10 |  574 |   1296.1 |     29.3 |     29.3 |     29.4 |     4.46 |    24.49 |      47.2 |
|   20 |  574 |   1296.1 |     28.5 |     28.4 |     29.6 |     2.48 |    24.80 |      44.4 |

`dl_med_s` = median Phase 4 wall time across 3 samples. `install_s` = sum of
all `trans_commit` durations (libalpm package extraction, hooks, depmod).

## Phase timings (seconds, median sample)

| conc | Ph4 base | Ph5 config | Ph6 ZFS | Ph7 initramfs | Ph9 KDE | Ph10 addl |
|-----:|---------:|-----------:|--------:|--------------:|--------:|----------:|
|    3 |    38.6s |       1.0s |    8.3s |          3.3s |   52.4s |      1.2s |
|    5 |    32.4s |       1.0s |    8.1s |          3.1s |   42.8s |      1.2s |
|   10 |    29.3s |       1.0s |    8.1s |          3.1s |   33.5s |      1.2s |
|   20 |    28.5s |       1.0s |   11.1s |          3.0s |   30.6s |      1.2s |

## Analysis

**conc=10 is the sweet spot.** Total download time (Ph4 + Ph9) vs conc=3:

| conc | Ph4+Ph9 | vs conc=3 |
|-----:|--------:|----------:|
|    3 |   91.0s | baseline  |
|    5 |   75.2s |      −17% |
|   10 |   62.8s |      −31% |
|   20 |   59.1s |      −35% |

3→5 and 5→10 each yield ~16% improvement. 10→20 saves only 3.7 s more (−6%)
while opening twice as many connections to mirrors — firmly diminishing returns.

`trans_commit` time also improves with concurrency (70 s → 44 s): packages
downloaded in a burst are still hot in the OS page cache when libalpm reads
them for extraction, so higher concurrency indirectly speeds up installation too.

`avg_MBps` decreasing at higher concurrency is expected — it is the mean
per-connection throughput, not total throughput. Wall time is the relevant metric.

**Bug fixed in this branch:** `parallel_downloads` from config was previously
ignored by the async downloader — `AlpmContext` hardcoded concurrency=5
everywhere. After the fix, every code path that creates an `AlpmContext`
receives an explicit `DownloadConfig`; the compiler enforces this.

**Default raised from 5 → 10** based on these results.
