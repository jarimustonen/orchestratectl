//! V4 stress test: 50 threads × 1000 iterations of `flock` + append.
//!
//! Validates `design.md` §4 / `validation.md` V4 — `fs4` flock on
//! macOS APFS correctly serializes concurrent short-lived writers without
//! livelock, starvation, or torn lines.
//!
//! Lives in-crate (rather than under `tests/`) because it exercises the
//! `pub(crate)` raw append primitive [`append_event_with_seq`] directly —
//! the lowest-level lock+seq path, below the canonical
//! [`append_and_apply_event`](crate::append_and_apply_event). That keeps the
//! V4 lock-acquire-latency measurement honest (no reducer work per iter).
//!
//! Verifies:
//! - final `events.jsonl` has exactly `50_000` lines
//! - every `seq` `1..=50_000` appears exactly once
//! - no torn lines (every line is valid JSON)
//! - prints 99th-percentile lock-acquisition latency

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use serde_json::json;
use tempfile::TempDir;

use crate::events::{append_event_with_seq, read_all_events, recover_last_seq};
use crate::{ensure_root, run_dir, RunId, RunLock, RunPaths};

const THREADS: usize = 50;
const ITERS_PER_THREAD: usize = 1000;
const TOTAL: usize = THREADS * ITERS_PER_THREAD;

// Slow (~200s release / multi-minute debug). Run explicitly with:
//   cargo test -p octl-core --release stress_tests -- --nocapture --ignored
#[test]
#[ignore = "expensive — run explicitly with --ignored"]
fn flock_stress_50_threads_1000_iters() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    ensure_root(root).unwrap();
    let run_id = "01jxstress0000000000000000".to_string();
    let dir = run_dir(root, &RunId::parse_str(&run_id).unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let paths = Arc::new(RunPaths::new(dir, run_id).unwrap());

    // Touch the lock and events file once so each thread sees the same root.
    RunLock::with_lock(&paths, |_lock| Ok(())).unwrap();

    let latencies = Arc::new(std::sync::Mutex::new(Vec::<u128>::with_capacity(TOTAL)));

    let start = Instant::now();
    let handles: Vec<_> = (0..THREADS)
        .map(|tid| {
            let paths = Arc::clone(&paths);
            let latencies = Arc::clone(&latencies);
            std::thread::spawn(move || {
                let mut local = Vec::<u128>::with_capacity(ITERS_PER_THREAD);
                for i in 0..ITERS_PER_THREAD {
                    let t0 = Instant::now();
                    RunLock::with_lock(&paths, |lock| {
                        let acq_ns = t0.elapsed().as_nanos();
                        local.push(acq_ns);
                        let last = recover_last_seq(&paths.events())?;
                        let seq = last + 1;
                        append_event_with_seq(
                            lock,
                            &paths,
                            seq,
                            "stress.tick",
                            None,
                            None,
                            json!({"tid": tid, "i": i}),
                        )
                    })
                    .unwrap();
                }
                latencies.lock().unwrap().extend(local);
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let elapsed = start.elapsed();

    // Validate: every seq 1..=TOTAL present, monotonic, no torn lines.
    let events = read_all_events(&paths.events()).unwrap();
    assert_eq!(
        events.len(),
        TOTAL,
        "expected {TOTAL} events, got {}",
        events.len()
    );
    let mut seqs = BTreeSet::new();
    let mut prev = 0u64;
    for ev in &events {
        assert!(
            ev.seq > prev,
            "seq not monotonic: prev={prev} curr={}",
            ev.seq
        );
        assert!(seqs.insert(ev.seq), "duplicate seq {}", ev.seq);
        prev = ev.seq;
    }
    assert_eq!(*seqs.iter().next().unwrap(), 1);
    assert_eq!(*seqs.iter().next_back().unwrap(), TOTAL as u64);

    // p99 latency.
    let mut lats = latencies.lock().unwrap().clone();
    lats.sort_unstable();
    let p50 = lats[lats.len() / 2];
    let p99 = lats[(lats.len() * 99) / 100];
    let max = *lats.last().unwrap();
    eprintln!(
        "V4 flock stress: {THREADS} threads × {ITERS_PER_THREAD} iters = {TOTAL} ops in {elapsed:.2?}"
    );
    eprintln!(
        "V4 flock stress: lock-acquire latency  p50={:.3}ms  p99={:.3}ms  max={:.3}ms",
        p50 as f64 / 1_000_000.0,
        p99 as f64 / 1_000_000.0,
        max as f64 / 1_000_000.0,
    );
}
