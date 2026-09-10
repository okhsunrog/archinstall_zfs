//! Download benchmark: repeated installs at several concurrency levels and
//! the analysis of the metrics they leave behind.

use std::path::{Path, PathBuf};

use crate::{TestOpts, check_prerequisites, cmd_test_install};

// ── bench-downloads ────────────────────────────────────

/// Patch `parallel_downloads` in a JSON config and write to a temp file.
fn patch_config_concurrency(
    config_path: &Path,
    concurrency: usize,
    dest: &Path,
) -> Result<(), String> {
    let content = std::fs::read_to_string(config_path).map_err(|e| format!("read config: {e}"))?;
    let mut v: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("parse config: {e}"))?;
    v["parallel_downloads"] = serde_json::json!(concurrency);
    let patched = serde_json::to_string_pretty(&v).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(dest, patched).map_err(|e| format!("write patched config: {e}"))?;
    Ok(())
}

pub fn cmd_bench_downloads(
    opts: TestOpts,
    concurrency_spec: &str,
    out_dir: &Path,
    samples: usize,
) -> Result<(), String> {
    check_prerequisites(&opts)?;
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create out_dir: {e}"))?;

    let samples = samples.max(1);

    // Parse comma-separated concurrency values
    let concurrencies: Vec<usize> = concurrency_spec
        .split(',')
        .map(|s| {
            s.trim()
                .parse::<usize>()
                .map_err(|_| format!("invalid concurrency value: '{s}'"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    eprintln!(
        "=== bench-downloads: {} concurrency levels × {} sample(s): {:?} ===",
        concurrencies.len(),
        samples,
        concurrencies
    );

    for &conc in &concurrencies {
        eprintln!("\n--- concurrency={conc} ({samples} sample(s)) ---");

        // Write patched config to a temp file (shared across all samples)
        let patched_config = out_dir.join(format!("config_conc{conc}.json"));
        patch_config_concurrency(&opts.paths.config, conc, &patched_config)?;

        // Track phase4 wall time for each sample to pick the median
        let mut sample_files: Vec<PathBuf> = Vec::new();
        let mut phase4_times: Vec<u64> = Vec::new();

        for s in 1..=samples {
            eprintln!("  sample {s}/{samples}");

            let mut run_opts = opts.clone();
            run_opts.paths.config = patched_config.clone();
            cmd_test_install(run_opts)?;

            let local_metrics = PathBuf::from("/tmp/archinstall-metrics.jsonl");
            let sample_dest = out_dir.join(format!("conc_{conc}_s{s}.jsonl"));
            if local_metrics.exists() {
                std::fs::copy(&local_metrics, &sample_dest)
                    .map_err(|e| format!("copy metrics sample: {e}"))?;
                eprintln!("    saved {}", sample_dest.display());

                // Extract phase 4 wall time from this sample
                let content = std::fs::read_to_string(&sample_dest).unwrap_or_default();
                let ph4_time = phase4_wall_ms_from_jsonl(&content);
                eprintln!("    phase4 wall: {:.1}s", ph4_time as f64 / 1000.0);
                phase4_times.push(ph4_time);
                sample_files.push(sample_dest);
            } else {
                eprintln!("    Warning: metrics file not found");
                phase4_times.push(u64::MAX);
                sample_files.push(PathBuf::new());
            }
        }

        // Pick the median sample (by phase 4 wall time) as the canonical result
        let median_idx = median_index(&phase4_times);
        let canonical = out_dir.join(format!("conc_{conc}.jsonl"));
        if sample_files[median_idx].exists() {
            std::fs::copy(&sample_files[median_idx], &canonical)
                .map_err(|e| format!("copy median sample: {e}"))?;
            eprintln!(
                "  Median sample: s{} ({:.1}s) → {}",
                median_idx + 1,
                phase4_times[median_idx] as f64 / 1000.0,
                canonical.display()
            );
        }
        if samples > 1 {
            let mut sorted = phase4_times.clone();
            sorted.retain(|&t| t != u64::MAX);
            sorted.sort_unstable();
            let min_s = sorted.first().copied().unwrap_or(0) as f64 / 1000.0;
            let max_s = sorted.last().copied().unwrap_or(0) as f64 / 1000.0;
            eprintln!("  Phase4 range: {min_s:.1}s – {max_s:.1}s");
        }
    }

    eprintln!("\n=== bench-downloads: complete ===");
    eprintln!(
        "Run 'cargo xtask analyze-metrics --dir {}' to see results.",
        out_dir.display()
    );
    Ok(())
}

/// Extract the phase-4 wall-clock time (ms) from a JSONL string.
/// Returns phase_5.ts_ms - phase_4.ts_ms, or 0 if either is missing.
fn phase4_wall_ms_from_jsonl(content: &str) -> u64 {
    let mut ts4 = None;
    let mut ts5 = None;
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("event").and_then(|e| e.as_str()) == Some("phase_start") {
            let num = v.get("num").and_then(|n| n.as_u64()).unwrap_or(0);
            let ts = v.get("ts_ms").and_then(|t| t.as_u64()).unwrap_or(0);
            match num {
                4 => ts4 = Some(ts),
                5 => ts5 = Some(ts),
                _ => {}
            }
        }
    }
    match (ts4, ts5) {
        (Some(t4), Some(t5)) if t5 > t4 => t5 - t4,
        _ => 0,
    }
}

/// Return the index of the median value in a slice.
fn median_index(values: &[u64]) -> usize {
    if values.is_empty() {
        return 0;
    }
    let mut indexed: Vec<(usize, u64)> = values.iter().copied().enumerate().collect();
    indexed.sort_by_key(|&(_, v)| v);
    indexed[indexed.len() / 2].0
}

// ── analyze-metrics ────────────────────────────────────

#[derive(Default)]
struct RunStats {
    concurrency: usize,
    pkg_count: u64,
    total_bytes: u64,
    total_dl_ms: u64,
    max_speed_bps: u64,
    avg_speed_bps: u64,
    batch_install_ms: u64,
    phases: Vec<(u32, String, u64)>, // (num, name, ts_ms)
}

pub fn cmd_analyze_metrics(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Err(format!("directory not found: {}", dir.display()));
    }

    // Collect canonical conc_N.jsonl files, sorted by N
    let mut entries: Vec<(usize, PathBuf)> = std::fs::read_dir(dir)
        .map_err(|e| format!("read dir: {e}"))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let stem = name.strip_suffix(".jsonl")?;
            // Only canonical files (conc_N, not conc_N_sM)
            let n = stem.strip_prefix("conc_")?;
            if n.contains('_') {
                return None;
            }
            let n = n.parse::<usize>().ok()?;
            Some((n, e.path()))
        })
        .collect();
    entries.sort_by_key(|(n, _)| *n);

    if entries.is_empty() {
        return Err(format!("no conc_N.jsonl files found in {}", dir.display()));
    }

    // Also check for per-sample files to compute scatter
    let mut sample_map: std::collections::HashMap<usize, Vec<u64>> = Default::default();
    for e in std::fs::read_dir(dir)
        .map_err(|e| format!("read dir: {e}"))?
        .filter_map(|e| e.ok())
    {
        let name = e.file_name().into_string().unwrap_or_default();
        let stem = name.strip_suffix(".jsonl").unwrap_or("");
        // matches conc_N_sM
        if let Some(rest) = stem.strip_prefix("conc_") {
            let parts: Vec<&str> = rest.splitn(2, '_').collect();
            if parts.len() == 2
                && let (Ok(n), Some(_s)) = (parts[0].parse::<usize>(), parts[1].strip_prefix('s'))
                && let Ok(content) = std::fs::read_to_string(e.path())
            {
                let t = phase4_wall_ms_from_jsonl(&content);
                sample_map.entry(n).or_default().push(t);
            }
        }
    }

    let mut rows: Vec<RunStats> = Vec::new();

    for (conc, path) in &entries {
        let content =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;

        let mut stats = RunStats {
            concurrency: *conc,
            ..Default::default()
        };

        let mut phase_ts: Vec<(u32, String, u64)> = Vec::new();
        let mut dl_speeds: Vec<u64> = Vec::new();

        for line in content.lines() {
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let event = v.get("event").and_then(|e| e.as_str()).unwrap_or("");
            let ts_ms = v.get("ts_ms").and_then(|t| t.as_u64()).unwrap_or(0);

            match event {
                "pkg_download" => {
                    let bytes = v.get("bytes").and_then(|b| b.as_u64()).unwrap_or(0);
                    let speed_bps = v.get("speed_bps").and_then(|s| s.as_u64()).unwrap_or(0);
                    stats.pkg_count += 1;
                    stats.total_bytes += bytes;
                    dl_speeds.push(speed_bps);
                    if speed_bps > stats.max_speed_bps {
                        stats.max_speed_bps = speed_bps;
                    }
                }
                "batch_install" => {
                    stats.batch_install_ms +=
                        v.get("duration_ms").and_then(|d| d.as_u64()).unwrap_or(0);
                }
                "phase_start" => {
                    let num = v.get("num").and_then(|n| n.as_u64()).unwrap_or(0) as u32;
                    let name = v
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    phase_ts.push((num, name, ts_ms));
                }
                _ => {}
            }
        }

        if !dl_speeds.is_empty() {
            stats.avg_speed_bps = dl_speeds.iter().sum::<u64>() / dl_speeds.len() as u64;
        }

        // Use phase4→phase5 timestamps for wall-clock download time
        if let Some((_, _, ts4)) = phase_ts.iter().find(|(n, _, _)| *n == 4)
            && let Some((_, _, ts5)) = phase_ts.iter().find(|(n, _, _)| *n == 5)
        {
            stats.total_dl_ms = ts5 - ts4;
        }
        // total_bytes must be measured from pkg_download events (set above, but reset by wall logic)
        // re-sum bytes properly
        stats.total_bytes = 0;
        for line in content.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if v.get("event").and_then(|e| e.as_str()) == Some("pkg_download") {
                stats.total_bytes += v.get("bytes").and_then(|b| b.as_u64()).unwrap_or(0);
            }
        }

        stats.phases = phase_ts;
        rows.push(stats);
    }

    // Print markdown table
    println!("## Download Benchmark Results\n");

    // Check if we have multi-sample scatter
    let has_scatter = sample_map.values().any(|v| v.len() > 1);

    if has_scatter {
        println!(
            "| conc | pkgs | total_MB | dl_med_s | dl_min_s | dl_max_s | avg_MBps | max_MBps | install_s |"
        );
        println!(
            "|-----:|-----:|---------:|---------:|---------:|---------:|---------:|---------:|----------:|"
        );
    } else {
        println!("| conc | pkgs | total_MB | dl_wall_s | avg_MBps | max_MBps | install_s |");
        println!("|-----:|-----:|---------:|----------:|---------:|---------:|----------:|");
    }

    for r in &rows {
        let total_mb = r.total_bytes as f64 / 1_048_576.0;
        let dl_wall_s = r.total_dl_ms as f64 / 1000.0;
        let avg_mbps = r.avg_speed_bps as f64 / 1_048_576.0;
        let max_mbps = r.max_speed_bps as f64 / 1_048_576.0;
        let install_s = r.batch_install_ms as f64 / 1000.0;

        if has_scatter {
            let mut samples = sample_map.get(&r.concurrency).cloned().unwrap_or_default();
            samples.sort_unstable();
            let min_s = samples.first().copied().unwrap_or(0) as f64 / 1000.0;
            let max_s = samples.last().copied().unwrap_or(0) as f64 / 1000.0;
            println!(
                "| {:>4} | {:>4} | {:>8.1} | {:>8.1} | {:>8.1} | {:>8.1} | {:>8.2} | {:>8.2} | {:>9.1} |",
                r.concurrency,
                r.pkg_count,
                total_mb,
                dl_wall_s,
                min_s,
                max_s,
                avg_mbps,
                max_mbps,
                install_s
            );
        } else {
            println!(
                "| {:>4} | {:>4} | {:>8.1} | {:>9.1} | {:>8.2} | {:>8.2} | {:>9.1} |",
                r.concurrency, r.pkg_count, total_mb, dl_wall_s, avg_mbps, max_mbps, install_s
            );
        }
    }

    // Per-phase breakdown for each run
    println!("\n## Phase Timings (seconds)\n");
    let all_phases: Vec<u32> = {
        let mut nums: Vec<u32> = rows
            .iter()
            .flat_map(|r| r.phases.iter().map(|(n, _, _)| *n))
            .collect();
        nums.sort_unstable();
        nums.dedup();
        nums
    };

    if !all_phases.is_empty() {
        let header_phases: String = all_phases
            .iter()
            .map(|n| format!("| Ph{n:>2} "))
            .collect::<Vec<_>>()
            .join("");
        println!("| conc {header_phases}|");
        let sep: String = all_phases
            .iter()
            .map(|_| "|-----:")
            .collect::<Vec<_>>()
            .join("");
        println!("|-----:{sep}|");

        for r in &rows {
            let phase_map: std::collections::HashMap<u32, u64> = r
                .phases
                .windows(2)
                .map(|w| {
                    let (n1, _, ts1) = &w[0];
                    let (_, _, ts2) = &w[1];
                    (*n1, ts2 - ts1)
                })
                .collect();

            let cells: String = all_phases
                .iter()
                .map(|n| {
                    if let Some(&dur) = phase_map.get(n) {
                        format!("| {:>4.1}s", dur as f64 / 1000.0)
                    } else {
                        "|    - ".to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            println!("| {:>4} {cells} |", r.concurrency);
        }
    }

    Ok(())
}
