//! Deterministic, supervisor-owned resource circuit-breakers (design.md §9).
//!
//! Distinct from quality judgment (design §0.1 / principle 1): these are
//! **mechanical** ceilings the supervisor enforces regardless of the model's
//! convergence verdict, so the verify→triage→fix loop can never run away on cost,
//! time, disk, spawned processes, or a failure that keeps recurring. They force
//! the loop to a terminal `circuit_breaker` state — never gated on an LLM's
//! judgment, which is the whole point of §9 (an agent is never trusted to "pull
//! the brake" on itself).
//!
//! The counting/config here is a **pure** value type ([`ResourceBudget`]) plus a
//! **pure** accumulator ([`ResourceMeter`]) so every ceiling can be unit-tested
//! without git, a model, or a clock — the driver ([`super`]) feeds the meter the
//! real [`Usage`], a measured elapsed [`Duration`], and a measured storage figure,
//! then asks [`ResourceMeter::breach`] whether any ceiling was crossed.
//!
//! The five breakers the issue calls for (all deterministic):
//! - **cost / token** ceilings with a kill-switch — a per-run spend tally fed from
//!   the harness [`Usage`]; on breach the loop stops before the next model call,
//!   so no further spend accrues.
//! - **wall-time** ceiling — total elapsed since the run started.
//! - **process-count** ceiling — how many agent invocations were spawned.
//! - **storage** ceiling — bytes under the run's scratch workdir.
//! - **repeated-identical-failure** breaker — the same failure recurring N times
//!   aborts instead of looping (design §9), keyed on a stable fingerprint so a
//!   floor block that reproduces identically does not burn the whole re-code
//!   budget re-generating the same failure.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;

use crate::harness::Usage;

/// Deterministic resource ceilings for one feature run (design §9). Every ceiling
/// is `Option`: `None` = unbounded (that breaker is off). A ceiling that is
/// crossed forces the loop to abort with a `circuit_breaker` terminal status,
/// regardless of the model's convergence judgment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourceBudget {
    /// Hard cap on total tokens (design §9 token ceiling), summed across every
    /// metered agent invocation. Uses the provider's combined total when reported,
    /// else input+output.
    pub max_total_tokens: Option<u64>,
    /// Hard cap on total cost in USD (design §9 cost ceiling + kill-switch),
    /// target ≤ ~2× an all-Opus run.
    pub max_cost_usd: Option<f64>,
    /// Hard cap on total wall-clock time since the run started (design §9
    /// wall-time ceiling).
    pub max_wall_time: Option<Duration>,
    /// Hard cap on the number of agent invocations spawned across the run (design
    /// §9 process-count ceiling).
    pub max_processes: Option<u32>,
    /// Hard cap on bytes under the run's scratch workdir (design §9 storage
    /// ceiling).
    pub max_storage_bytes: Option<u64>,
    /// The same failure (fingerprinted by chunk + status + findings) recurring
    /// this many times aborts the loop (design §9 repeated-identical-failure).
    /// `None` or `0` disables it.
    pub max_identical_failures: Option<u32>,
}

impl ResourceBudget {
    /// Whether a just-incremented identical-failure `count` for a fingerprint has
    /// crossed the repeated-identical-failure ceiling (design §9). Returns the
    /// `circuit_breaker` message on breach. `None`/`0` ceiling = off.
    #[must_use]
    pub fn identical_failure_breach(&self, count: u32) -> Option<String> {
        self.max_identical_failures.and_then(|cap| {
            (cap > 0 && count >= cap).then(|| {
                format!(
                    "repeated-identical-failure breaker: the same failure recurred {count} time(s) \
                     (ceiling {cap})"
                )
            })
        })
    }
}

impl ResourceBudget {
    /// Every ceiling off — the pre-T6 behaviour (only the count-based
    /// [`FixLoopConfig`](super::fixloop::FixLoopConfig) bounds apply). Used as the
    /// default in tests that predate the resource breakers so they stay meaningful.
    pub const UNLIMITED: ResourceBudget = ResourceBudget {
        max_total_tokens: None,
        max_cost_usd: None,
        max_wall_time: None,
        max_processes: None,
        max_storage_bytes: None,
        max_identical_failures: None,
    };

    /// The ceilings the live `pipeline run` command uses by default. Generous
    /// enough that a normal feature never trips one, real enough to stop a runaway
    /// (design §9: a backstop, not a tight budget — the count-based fix-loop bounds
    /// already keep an ordinary run short). Every ceiling is individually
    /// overridable on the CLI, and a value of `0` disables it.
    #[must_use]
    pub const fn live_default() -> Self {
        Self {
            // ~2M tokens is far past a single amortized feature (design §11: spend
            // is Opus-dominated but bounded); a runaway loop blows past it.
            max_total_tokens: Some(2_000_000),
            // Target ≤ ~2× an all-Opus feature (design §9). $10 is a deliberately
            // generous ceiling for one feature.
            max_cost_usd: Some(10.0),
            // One unattended feature should not run for an hour.
            max_wall_time: Some(Duration::from_secs(3600)),
            // Chunk attempts + spec/verify spawns; the fix-loop bounds keep a sane
            // run well under this, so hitting it means something is spinning.
            max_processes: Some(50),
            // 2 GiB of scratch worktrees/artifacts is already a lot for one feature.
            max_storage_bytes: Some(2 * 1024 * 1024 * 1024),
            // The same failure three times over is a loop, not progress.
            max_identical_failures: Some(3),
        }
    }
}

/// A stable fingerprint for a blocked chunk attempt, used by the
/// repeated-identical-failure breaker. Combines the chunk id, the block status,
/// and the (order-insensitive) findings — the floor-violation lines that describe
/// *what* failed — so an identical floor block reproduces the same key across
/// re-code attempts. Volatile detail (commit oids in a harness reason) naturally
/// varies the key, so those blocks fall to the re-code-budget bound instead, which
/// is the intended behaviour (they are not "the same failure").
#[must_use]
pub fn failure_fingerprint(chunk_id: &str, status: &str, findings: &[String]) -> String {
    let mut parts: Vec<&str> = findings.iter().map(String::as_str).collect();
    parts.sort_unstable();
    // `\u{1}` (SOH) is not a byte any of the components legitimately contain, so it
    // is an unambiguous field separator.
    format!("{chunk_id}\u{1}{status}\u{1}{}", parts.join("\u{1}"))
}

/// The live per-run resource accumulator (design §9 cost instrumentation). Fed by
/// the driver after each metered agent run and each measured round boundary; the
/// numeric fields are surfaced in the report so the tally is auditable.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ResourceMeter {
    /// Total tokens summed across every metered agent invocation.
    pub total_tokens: u64,
    /// Total cost in USD summed across every metered agent invocation.
    pub cost_usd: f64,
    /// Number of agent invocations spawned (chunk attempts + spec/verify calls).
    pub processes: u32,
    /// The largest measured scratch-workdir size in bytes (design §9 storage).
    pub storage_bytes: u64,
    /// Per-fingerprint identical-failure counts (design §9 repeated-failure). Not
    /// serialized — it is internal breaker bookkeeping, not a reportable total.
    #[serde(skip)]
    failure_counts: BTreeMap<String, u32>,
}

impl ResourceMeter {
    /// A fresh, all-zero meter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one agent invocation and fold in its [`Usage`] (design §9 per-node
    /// cost tally). Every invocation bumps the process count; token/cost totals
    /// grow only when the provider reported them (spec/verify do not surface usage
    /// through their trait today, so they contribute a process but no tokens — a
    /// documented follow-up). Saturating so an absurd provider figure can never
    /// overflow the tally.
    pub fn record_agent_run(&mut self, usage: Option<&Usage>) {
        self.processes = self.processes.saturating_add(1);
        if let Some(u) = usage {
            let tokens = u.total_tokens.unwrap_or_else(|| {
                u.input_tokens
                    .unwrap_or(0)
                    .saturating_add(u.output_tokens.unwrap_or(0))
            });
            self.total_tokens = self.total_tokens.saturating_add(tokens);
            if let Some(c) = u.cost_usd {
                // Ignore a negative/NaN cost rather than corrupt the tally.
                if c.is_finite() && c > 0.0 {
                    self.cost_usd += c;
                }
            }
        }
    }

    /// Record the measured scratch-workdir size (design §9 storage), keeping the
    /// high-water mark so a mid-run cleanup can't hide a peak that already
    /// breached.
    pub fn observe_storage_bytes(&mut self, bytes: u64) {
        self.storage_bytes = self.storage_bytes.max(bytes);
    }

    /// Record one occurrence of a blocked-attempt fingerprint and return the new
    /// count for that fingerprint (design §9 repeated-identical-failure).
    pub fn record_failure(&mut self, fingerprint: &str) -> u32 {
        let c = self
            .failure_counts
            .entry(fingerprint.to_string())
            .or_insert(0);
        *c = c.saturating_add(1);
        *c
    }

    /// The first resource ceiling crossed, if any, as the `circuit_breaker`
    /// message (deterministic order: cost, tokens, wall-time, processes, storage).
    /// `elapsed` is the driver-measured wall-clock since the run started; storage
    /// uses the last [`observe_storage_bytes`](ResourceMeter::observe_storage_bytes)
    /// value. Repeated-identical-failure is checked separately via
    /// [`identical_failure_breach`](ResourceBudget::identical_failure_breach) at the
    /// point a failure is recorded (it needs the just-incremented count).
    #[must_use]
    pub fn breach(&self, budget: &ResourceBudget, elapsed: Duration) -> Option<String> {
        if let Some(cap) = budget.max_cost_usd {
            if cap > 0.0 && self.cost_usd > cap {
                return Some(format!(
                    "cost ceiling exceeded: ${:.4} spent > ${cap:.4} ceiling",
                    self.cost_usd
                ));
            }
        }
        if let Some(cap) = budget.max_total_tokens {
            if cap > 0 && self.total_tokens > cap {
                return Some(format!(
                    "token ceiling exceeded: {} tokens > {cap} ceiling",
                    self.total_tokens
                ));
            }
        }
        if let Some(cap) = budget.max_wall_time {
            if !cap.is_zero() && elapsed > cap {
                return Some(format!(
                    "wall-time ceiling exceeded: {:.1}s elapsed > {}s ceiling",
                    elapsed.as_secs_f64(),
                    cap.as_secs()
                ));
            }
        }
        if let Some(cap) = budget.max_processes {
            if cap > 0 && self.processes > cap {
                return Some(format!(
                    "process-count ceiling exceeded: {} agent invocation(s) > {cap} ceiling",
                    self.processes
                ));
            }
        }
        if let Some(cap) = budget.max_storage_bytes {
            if cap > 0 && self.storage_bytes > cap {
                return Some(format!(
                    "storage ceiling exceeded: {} bytes > {cap} bytes ceiling",
                    self.storage_bytes
                ));
            }
        }
        None
    }
}

/// Total size in bytes of the regular files under `root` (design §9 storage
/// ceiling). Best-effort: unreadable entries and I/O errors are skipped rather
/// than propagated (storage metering must never fail the run), and symlinks are
/// measured by their own metadata (not followed) so a link out of the workdir
/// can't inflate the figure or loop. Returns `0` when `root` does not exist yet.
#[must_use]
pub fn dir_size_bytes(root: &std::path::Path) -> u64 {
    fn walk(dir: &std::path::Path, acc: &mut u64) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            // `DirEntry::metadata` does NOT traverse a symlink (unlike `fs::metadata`)
            // — the link's own size counts and we never recurse through it.
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                walk(&entry.path(), acc);
            } else {
                *acc = acc.saturating_add(meta.len());
            }
        }
    }
    let mut acc = 0;
    walk(root, &mut acc);
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(tokens: u64, cost: f64) -> Usage {
        Usage {
            input_tokens: None,
            output_tokens: None,
            total_tokens: Some(tokens),
            cost_usd: Some(cost),
        }
    }

    #[test]
    fn unlimited_never_breaches() {
        let mut m = ResourceMeter::new();
        m.record_agent_run(Some(&usage(1_000_000, 999.0)));
        m.observe_storage_bytes(u64::MAX);
        assert_eq!(
            m.breach(&ResourceBudget::UNLIMITED, Duration::from_secs(1)),
            None
        );
    }

    #[test]
    fn cost_ceiling_trips_with_kill_switch_ordering() {
        // Cost is checked first: a run that breaches BOTH cost and tokens reports
        // cost (the kill-switch the issue names).
        let mut m = ResourceMeter::new();
        m.record_agent_run(Some(&usage(10_000_000, 25.0)));
        let budget = ResourceBudget {
            max_cost_usd: Some(10.0),
            max_total_tokens: Some(1_000),
            ..ResourceBudget::UNLIMITED
        };
        let msg = m.breach(&budget, Duration::ZERO).expect("cost breach");
        assert!(msg.contains("cost ceiling exceeded"), "{msg}");
    }

    #[test]
    fn token_ceiling_trips() {
        let mut m = ResourceMeter::new();
        m.record_agent_run(Some(&usage(2_500, 0.0)));
        let budget = ResourceBudget {
            max_total_tokens: Some(2_000),
            ..ResourceBudget::UNLIMITED
        };
        assert!(m
            .breach(&budget, Duration::ZERO)
            .unwrap()
            .contains("token ceiling"));
    }

    #[test]
    fn wall_time_ceiling_trips_only_past_the_cap() {
        let m = ResourceMeter::new();
        let budget = ResourceBudget {
            max_wall_time: Some(Duration::from_secs(60)),
            ..ResourceBudget::UNLIMITED
        };
        assert_eq!(
            m.breach(&budget, Duration::from_secs(60)),
            None,
            "at cap is fine"
        );
        assert!(m
            .breach(&budget, Duration::from_secs(61))
            .unwrap()
            .contains("wall-time"));
    }

    #[test]
    fn process_ceiling_counts_every_agent_run_including_no_usage() {
        let mut m = ResourceMeter::new();
        for _ in 0..3 {
            m.record_agent_run(None); // spec/verify style: no usage, still a process
        }
        let budget = ResourceBudget {
            max_processes: Some(2),
            ..ResourceBudget::UNLIMITED
        };
        assert!(m
            .breach(&budget, Duration::ZERO)
            .unwrap()
            .contains("process-count"));
    }

    #[test]
    fn storage_keeps_high_water_mark() {
        let mut m = ResourceMeter::new();
        m.observe_storage_bytes(5_000);
        m.observe_storage_bytes(1_000); // a shrink must not hide the peak
        let budget = ResourceBudget {
            max_storage_bytes: Some(4_000),
            ..ResourceBudget::UNLIMITED
        };
        assert!(m
            .breach(&budget, Duration::ZERO)
            .unwrap()
            .contains("storage ceiling"));
    }

    #[test]
    fn zero_ceiling_disables_the_breaker() {
        let mut m = ResourceMeter::new();
        m.record_agent_run(Some(&usage(9_999, 9_999.0)));
        m.observe_storage_bytes(u64::MAX);
        let budget = ResourceBudget {
            max_total_tokens: Some(0),
            max_cost_usd: Some(0.0),
            max_processes: Some(0),
            max_storage_bytes: Some(0),
            max_wall_time: Some(Duration::ZERO),
            max_identical_failures: Some(0),
        };
        assert_eq!(m.breach(&budget, Duration::from_secs(10_000)), None);
        assert_eq!(budget.identical_failure_breach(100), None);
    }

    #[test]
    fn negative_or_nonfinite_cost_is_ignored() {
        let mut m = ResourceMeter::new();
        m.record_agent_run(Some(&Usage {
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cost_usd: Some(-5.0),
        }));
        m.record_agent_run(Some(&Usage {
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cost_usd: Some(f64::NAN),
        }));
        assert!(
            m.cost_usd.abs() < f64::EPSILON,
            "cost stayed zero: {}",
            m.cost_usd
        );
    }

    #[test]
    fn tokens_fall_back_to_input_plus_output() {
        let mut m = ResourceMeter::new();
        m.record_agent_run(Some(&Usage {
            input_tokens: Some(700),
            output_tokens: Some(300),
            total_tokens: None,
            cost_usd: None,
        }));
        assert_eq!(m.total_tokens, 1_000);
    }

    #[test]
    fn identical_failure_breaker_trips_on_the_nth_recurrence() {
        let mut m = ResourceMeter::new();
        let budget = ResourceBudget {
            max_identical_failures: Some(3),
            ..ResourceBudget::UNLIMITED
        };
        let fp = failure_fingerprint("c1", "chunk_floor_blocked", &["test regressed: t".into()]);
        assert_eq!(m.record_failure(&fp), 1);
        assert_eq!(budget.identical_failure_breach(1), None);
        assert_eq!(m.record_failure(&fp), 2);
        assert_eq!(budget.identical_failure_breach(2), None);
        assert_eq!(m.record_failure(&fp), 3);
        assert!(budget
            .identical_failure_breach(3)
            .unwrap()
            .contains("repeated-identical-failure"));
    }

    #[test]
    fn distinct_failures_do_not_aggregate() {
        let mut m = ResourceMeter::new();
        let a = failure_fingerprint("c1", "chunk_floor_blocked", &["test regressed: a".into()]);
        let b = failure_fingerprint("c1", "chunk_floor_blocked", &["test regressed: b".into()]);
        assert_eq!(m.record_failure(&a), 1);
        assert_eq!(
            m.record_failure(&b),
            1,
            "a different finding is a different key"
        );
    }

    #[test]
    fn fingerprint_is_order_insensitive_over_findings() {
        let a = failure_fingerprint("c1", "s", &["x".into(), "y".into()]);
        let b = failure_fingerprint("c1", "s", &["y".into(), "x".into()]);
        assert_eq!(a, b, "reordered findings must fingerprint identically");
    }
}
