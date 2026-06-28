//! Bench: cost of the idempotency dedup scan (`find_prior_with_key`) as the
//! event log grows.
//!
//! `find_prior_with_key` is `pub(crate)`, so this external bench exercises it
//! through its only public trigger: an `append_and_apply_event` carrying an
//! `idempotency_key` that already exists. On a key hit the call returns the
//! prior event *before* any append/reduce, so each timed iteration is exactly
//! one lock acquisition + one full linear scan of the log — the worst case the
//! index in `idempotency-scan-strictness-and-index` would replace.
//!
//! The log is pre-populated by writing envelopes directly (no per-line scan),
//! with the looked-up key placed on the LAST line so every scan walks the whole
//! file. Plain `fn main()` (`harness = false`) — no nightly `#[bench]`.
//!
//! Run: `cargo bench -p octl-core --bench idempotency_scan`

use std::fmt::Write as _;
use std::time::Instant;

use octl_core::{append_and_apply_event, RunPaths};
use serde_json::json;
use tempfile::TempDir;

/// Key placed on the final log line, so a lookup scans every prior line first.
const TARGET_KEY: &str = "BENCH_TARGET";
/// Kind shared by every probe line (the match also keys on kind).
const KIND: &str = "bench.probe";
const RUN_ID: &str = "01jxsnap000000000000000000";

/// Write `n` valid event envelopes straight to `events.jsonl`, the last
/// carrying [`TARGET_KEY`]. Direct write avoids the O(n²) cost of populating
/// via the scanning append path.
fn populate(paths: &RunPaths, n: usize) {
    let mut buf = String::with_capacity(n * 128);
    for i in 0..n {
        let key = if i + 1 == n {
            TARGET_KEY.to_string()
        } else {
            format!("k{i}")
        };
        // A representative envelope: ts/seq/kind/run_id/idempotency_key/data.
        let _ = writeln!(
            buf,
            "{{\"ts\":\"2026-06-12T00:00:00Z\",\"seq\":{seq},\"kind\":\"{KIND}\",\"run_id\":\"{RUN_ID}\",\"idempotency_key\":\"{key}\",\"data\":{{\"status\":\"running\"}}}}",
            seq = i + 1,
        );
    }
    std::fs::write(paths.events(), buf).unwrap();
}

/// Time one full-scan dedup hit. Returns the elapsed duration in microseconds.
fn time_one_scan(paths: &RunPaths) -> f64 {
    let t = Instant::now();
    let r = append_and_apply_event(
        paths,
        KIND,
        None,
        Some(TARGET_KEY),
        json!({"status": "running"}),
    )
    .expect("dedup hit");
    let dt = t.elapsed();
    // Sanity: it must have been a replay (a scan that found the key), not a new
    // append — otherwise the bench is measuring the wrong path.
    assert!(
        r.idempotent_replay,
        "expected an idempotent replay (full scan)"
    );
    dt.as_secs_f64() * 1e6
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx]
}

fn bench_n(n: usize, iters: usize) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join(RUN_ID);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = RunPaths::new(dir, RUN_ID).unwrap();
    populate(&paths, n);

    // Warm the page cache / allocator before sampling.
    for _ in 0..5 {
        time_one_scan(&paths);
    }

    let mut samples: Vec<f64> = Vec::with_capacity(iters);
    for _ in 0..iters {
        samples.push(time_one_scan(&paths));
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let min = samples[0];
    let median = percentile(&samples, 50.0);
    let p99 = percentile(&samples, 99.0);
    let max = *samples.last().unwrap();

    println!(
        "N={n:>7}  iters={iters:>3}  min={min:>9.1}µs  median={median:>9.1}µs  mean={mean:>9.1}µs  p99={p99:>9.1}µs  max={max:>9.1}µs"
    );
}

fn main() {
    println!("find_prior_with_key full-scan dedup hit (worst case: key on last line)");
    println!("---------------------------------------------------------------------");
    bench_n(1_000, 200);
    bench_n(10_000, 200);
    bench_n(100_000, 100);
}
