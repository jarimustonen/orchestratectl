//! The supervisor-side loop skeleton (design.md §2, §6) modelled as a pure
//! in-memory state machine.
//!
//! [`drive`] is the inverted loop: the supervisor owns iteration, and at each
//! decision point it calls the stateless [`Orchestrator`], **validates** the
//! returned primitives (tier invariant + chunk preconditions), **would-execute**
//! them (via a stubbed [`ActionExecutor`] — real git/merge/spawn is T5),
//! **records** each decision as an atomic [`DecisionRecord`] (action + envelope +
//! outcome), and applies the effect to an in-memory [`PipelineState`]. Terminal
//! actions (`DeclareConverged`, `Escalate`) halt the loop.
//!
//! Modelling it as a pure state machine over an in-memory state (rather than
//! bolting a new event-append path onto the live reducer) keeps it fully
//! unit-testable and respects the state-integrity invariants: T5 wires the real
//! event log in behind the same shape, appending each [`DecisionRecord`] through
//! the `LockedRun` witness and the `append_and_apply` API (invariant 1) rather
//! than writing projections directly.
//!
//! ## Fail-closed posture
//!
//! Two safety rules are deterministic and do NOT trust the orchestrator:
//! - A **tier-invariant violation** (a consequential action stamped
//!   coordinator-tier) escalates the loop — the fast tier is misaligned, so stop.
//! - A **circuit-breaker trip** ([`DecisionTrigger::CircuitBreakerTripped`])
//!   escalates directly, without asking the orchestrator (design §9: breakers are
//!   supervisor-owned; never trust an LLM to pull the brake).
//!
//! ## Scope (T4) vs. later tasks
//!
//! The scaffold validates the tier invariant and chunk-existence preconditions;
//! it does NOT yet enforce full semantic preconditions (e.g. `DeclareConverged`
//! only when every chunk is Accepted), durable/idempotent execution, or
//! conflict resolution within a multi-action batch. Those are T5 (supervisor
//! state machine) + the deterministic floor (T3) + T6 (breakers). The traits are
//! synchronous and infallible to match the landed `CodeHarness` seam
//! (`harness::CodeHarness`); T5 wraps real model/transport failures at the
//! integration boundary.

use std::collections::BTreeMap;

use octl_core::plan::{Plan, Tier};
use serde::{Deserialize, Serialize};

use super::action::{Action, SpinoffScope};
use super::envelope::{DecisionEnvelope, TierViolation};
use super::orchestrator::{DecisionContext, DecisionTrigger, Orchestrator};

/// A stubbed execution boundary. The scaffold's [`RecordingExecutor`] only
/// records what *would* run; T5 replaces it with the real actor that merges
/// chunks, spawns re-code nodes, writes plan revisions, etc. Keeping execution
/// behind a trait lets the loop's control flow be tested without any side effect.
pub trait ActionExecutor {
    /// Would-execute one validated action. Returns [`ExecError`] if the effect
    /// could not be carried out (in the scaffold, only a scripted failure).
    ///
    /// # Errors
    ///
    /// Returns [`ExecError`] when the (stubbed) execution fails.
    fn execute(&mut self, action: &Action) -> Result<(), ExecError>;
}

/// An execution failure from an [`ActionExecutor`]. In the scaffold this is only
/// produced by a scripted [`RecordingExecutor::failing_on`]; T5 maps real git /
/// merge / spawn failures onto it (and will likely widen `message` into a typed
/// cause so callers can distinguish "merge conflict" from "test failed").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("action execution failed for `{action}`: {message}")]
pub struct ExecError {
    /// The failing action's [`Action::name`].
    pub action: String,
    /// Diagnostic detail.
    pub message: String,
}

/// The scaffold executor: records every action it is asked to run and (for
/// tests) can be scripted to fail on specific action kinds. Never touches git or
/// the event log.
#[derive(Debug, Default)]
pub struct RecordingExecutor {
    /// Actions this executor *attempted* to run, in order. A scripted failure is
    /// still recorded here (the attempt happened) — the authoritative record of
    /// what succeeded vs. failed is the [`PipelineState`] decision trail.
    pub attempted: Vec<Action>,
    /// [`Action::name`]s to fail on (empty = never fail).
    fail_on: Vec<String>,
}

impl RecordingExecutor {
    /// A recording executor that never fails.
    pub fn new() -> Self {
        Self::default()
    }

    /// A recording executor that returns [`ExecError`] for any action whose
    /// [`Action::name`] is in `names` (for exercising the driver's failure path).
    pub fn failing_on(names: &[&str]) -> Self {
        Self {
            attempted: Vec::new(),
            fail_on: names.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

impl ActionExecutor for RecordingExecutor {
    fn execute(&mut self, action: &Action) -> Result<(), ExecError> {
        self.attempted.push(action.clone());
        if self.fail_on.iter().any(|n| n == action.name()) {
            return Err(ExecError {
                action: action.name().to_string(),
                message: "scripted failure".to_string(),
            });
        }
        Ok(())
    }
}

/// What happened to one emitted decision — recorded atomically with the action
/// and envelope in a [`DecisionRecord`], so the audit trail is self-contained
/// (design §2 causal replayability) rather than scattered across parallel lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DecisionOutcome {
    /// The action passed validation, executed, and its effect was applied.
    Applied,
    /// A consequential action was stamped coordinator-tier — rejected and the
    /// loop escalated (fail-closed). Carries the flagged violation.
    RejectedTierViolation(TierViolation),
    /// The action referenced a chunk not in the plan — rejected without a side
    /// effect (the precondition is checked *before* execution). Non-halting.
    RejectedPrecondition {
        /// Why the precondition failed.
        reason: String,
    },
    /// Execution failed; the loop escalated.
    ExecutionFailed(ExecError),
    /// The decision was emitted after the loop already reached a terminal state in
    /// the same batch — recorded for audit completeness, never executed.
    Superseded,
}

/// One atomic decision in the causal audit trail: the [`Action`] the orchestrator
/// emitted, the [`DecisionEnvelope`] recording who decided it, and the
/// [`DecisionOutcome`] of what the supervisor did with it. Keeping the three
/// together is what makes the run replayable — a reader can re-run
/// [`DecisionEnvelope::validate_for`] and see the effect without correlating
/// separate lists by index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    /// The emitted action.
    pub action: Action,
    /// The decision envelope stamped by the orchestrator.
    pub envelope: DecisionEnvelope,
    /// What the supervisor did with the decision.
    pub outcome: DecisionOutcome,
}

/// Where the loop stands (design §6). Only [`Running`](LoopStatus::Running) keeps
/// iterating; the two terminal states halt it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopStatus {
    /// Still processing decision points.
    Running,
    /// `DeclareConverged` fired — the feature is done (design §6 VAIHE 4).
    Converged,
    /// The loop escalated: an `Escalate` action, a circuit-breaker trip, a
    /// tier-invariant violation, or an execution failure. The
    /// [`escalation`](PipelineState::escalation) reason names which.
    Escalated,
}

/// Per-chunk status the loop tracks in memory (design §7 lifecycle). A coarse
/// model of the chunk lifecycle sufficient for the scaffold; T5 refines it
/// against the real DAG scheduler (with attempt ids, commits, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStatus {
    /// Not yet started, reverted by a re-spec, or promoted for a fresh attempt.
    Pending,
    /// A code node committed it; awaiting verify.
    AwaitingVerify,
    /// Re-coded against findings; MUST be re-verified before it can be accepted
    /// (design §8 "FIX-class MUST be re-verified before close").
    NeedsReverify,
    /// Accepted — floor green and verify satisfied.
    Accepted,
}

/// In-memory state of one chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkState {
    /// Lifecycle status.
    pub status: ChunkStatus,
    /// Current model tier (bumped by `PromoteTier`).
    pub tier: Tier,
}

/// A recorded discussion the loop opened (design §8 DISCUSS). Non-executing in
/// the scaffold — a bubble-up record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionRecord {
    /// The discussion topic.
    pub topic: String,
    /// Its severity.
    pub severity: super::action::Severity,
}

/// A recorded spin-off proposal (design §8 `SPIN_OFF`) — deferred, non-blocking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpinoffRecord {
    /// Proposed issue title.
    pub title: String,
    /// Proposed issue kind.
    pub kind: String,
    /// Rationale for deferring.
    pub rationale: String,
    /// The scope that governed whether this needed decider authority — retained
    /// so the audit shows the classification, not just the proposal.
    pub scope: SpinoffScope,
}

/// The whole in-memory state of one feature's loop — the audit-bearing result of
/// [`drive`]. The [`decisions`](PipelineState::decisions) trail is the canonical
/// record (each entry pairs an action with its envelope and outcome); the other
/// fields are reduced projections a T5 wiring would persist alongside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineState {
    /// The run this loop belongs to.
    pub run_id: String,
    /// The plan revision in force (bumped by `TriggerReSpec`).
    pub plan_rev: u32,
    /// The intent revision the plan targets. Write-once in the scaffold: no T4
    /// primitive changes intent (design §1 intent is orchestrator-owned, revised
    /// upstream of this loop); T5 surfaces intent changes as a re-spec input.
    pub intent_rev: u32,
    /// Where the loop stands.
    pub status: LoopStatus,
    /// Chunk states keyed by chunk id (ordered for deterministic inspection).
    pub chunks: BTreeMap<String, ChunkState>,
    /// The canonical, ordered audit trail: every emitted decision with its
    /// envelope and outcome (design §2).
    pub decisions: Vec<DecisionRecord>,
    /// Actions that referenced a chunk id not in the plan (a driver-level
    /// anomaly; empty on any well-formed run). Mirrors the
    /// [`DecisionOutcome::RejectedPrecondition`] entries for quick inspection.
    pub anomalies: Vec<String>,
    /// Opened discussions (bubble-up records).
    pub discussions: Vec<DiscussionRecord>,
    /// Proposed spin-offs (deferred backlog).
    pub spinoffs: Vec<SpinoffRecord>,
    /// The escalation reason, once the loop escalated.
    pub escalation: Option<String>,
}

impl PipelineState {
    /// Seed the state from a validated plan: every chunk starts
    /// [`Pending`](ChunkStatus::Pending) at its declared tier, and the plan/intent
    /// revisions are taken from the plan.
    fn from_plan(run_id: &str, plan: &Plan) -> Self {
        let chunks = plan
            .chunks
            .iter()
            .map(|c| {
                (
                    c.id.clone(),
                    ChunkState {
                        status: ChunkStatus::Pending,
                        tier: c.tier,
                    },
                )
            })
            .collect();
        Self {
            run_id: run_id.to_string(),
            plan_rev: plan.plan_rev,
            intent_rev: plan.intent_rev,
            status: LoopStatus::Running,
            chunks,
            decisions: Vec::new(),
            anomalies: Vec::new(),
            discussions: Vec::new(),
            spinoffs: Vec::new(),
            escalation: None,
        }
    }

    /// Count of decisions with a given outcome discriminant — a convenience for
    /// inspection/tests over the canonical [`decisions`](PipelineState::decisions)
    /// trail.
    pub fn tier_violations(&self) -> impl Iterator<Item = &TierViolation> {
        self.decisions.iter().filter_map(|d| match &d.outcome {
            DecisionOutcome::RejectedTierViolation(v) => Some(v),
            _ => None,
        })
    }

    /// Execution failures in decision order.
    pub fn exec_failures(&self) -> impl Iterator<Item = &ExecError> {
        self.decisions.iter().filter_map(|d| match &d.outcome {
            DecisionOutcome::ExecutionFailed(e) => Some(e),
            _ => None,
        })
    }

    /// Actions that were applied (side effect + state effect took hold).
    pub fn applied_actions(&self) -> impl Iterator<Item = &Action> {
        self.decisions.iter().filter_map(|d| match &d.outcome {
            DecisionOutcome::Applied => Some(&d.action),
            _ => None,
        })
    }

    /// Stage-outcome bookkeeping applied *before* the decision (design §6: the
    /// supervisor consumes stage outcomes). A committed chunk moves to
    /// `AwaitingVerify`; the other triggers carry no pre-decision state change.
    fn absorb_trigger(&mut self, trigger: &DecisionTrigger) {
        if let DecisionTrigger::ChunkCommitted { chunk_id } = trigger {
            self.set_chunk_status(chunk_id, ChunkStatus::AwaitingVerify);
        }
    }

    /// Check the chunk-existence precondition for `action` *before* it is
    /// executed, so an action naming an unknown chunk is rejected without a side
    /// effect (rather than executed and only then noticed). Returns the reason on
    /// the first unknown chunk.
    fn check_preconditions(&self, action: &Action) -> Result<(), String> {
        for chunk_id in action.referenced_chunks() {
            if !self.chunks.contains_key(chunk_id) {
                return Err(format!(
                    "action `{}` referenced unknown chunk `{chunk_id}`",
                    action.name()
                ));
            }
        }
        Ok(())
    }

    /// Apply the *effect* of an executed action to the in-memory state. Called
    /// only after the action passed tier validation + preconditions and was
    /// would-executed, so every chunk lookup here is guaranteed present.
    fn apply(&mut self, action: &Action) {
        match action {
            Action::ReCodeChunk { chunk_id, .. } => {
                self.set_chunk_status(chunk_id, ChunkStatus::NeedsReverify);
            }
            Action::AcceptChunk { chunk_id } => {
                self.set_chunk_status(chunk_id, ChunkStatus::Accepted);
            }
            Action::PromoteTier { chunk_id, tier } => {
                // Promotion bumps the tier AND resets the chunk to Pending: a
                // promote is a response to a stuck chunk (design §3), so the next
                // attempt re-runs at the new tier rather than leaving the old
                // failed state in place.
                if let Some(chunk) = self.chunks.get_mut(chunk_id) {
                    chunk.tier = *tier;
                    chunk.status = ChunkStatus::Pending;
                }
            }
            Action::TriggerReSpec { chunk_ids, .. } => {
                self.plan_rev = self.plan_rev.saturating_add(1);
                for id in chunk_ids {
                    self.set_chunk_status(id, ChunkStatus::Pending);
                }
            }
            Action::OpenDiscussion { topic, severity } => {
                self.discussions.push(DiscussionRecord {
                    topic: topic.clone(),
                    severity: *severity,
                });
            }
            Action::ProposeSpinoff {
                title,
                kind,
                rationale,
                scope,
            } => {
                self.spinoffs.push(SpinoffRecord {
                    title: title.clone(),
                    kind: kind.clone(),
                    rationale: rationale.clone(),
                    scope: *scope,
                });
            }
            Action::DeclareConverged => {
                self.status = LoopStatus::Converged;
            }
            Action::Escalate { reason } => {
                self.escalate(reason.clone());
            }
        }
    }

    /// Set a chunk's status. Preconditions guarantee the id exists on the applied
    /// path; the anomaly branch defends `absorb_trigger` (a stage outcome for an
    /// unknown chunk) which does not go through [`Self::check_preconditions`].
    fn set_chunk_status(&mut self, chunk_id: &str, status: ChunkStatus) {
        if let Some(chunk) = self.chunks.get_mut(chunk_id) {
            chunk.status = status;
        } else {
            self.anomalies.push(format!(
                "stage outcome referenced unknown chunk `{chunk_id}`"
            ));
        }
    }

    /// Transition to the escalated terminal state with a reason (idempotent on the
    /// reason — the first cause wins).
    fn escalate(&mut self, reason: String) {
        self.status = LoopStatus::Escalated;
        if self.escalation.is_none() {
            self.escalation = Some(reason);
        }
    }
}

/// Drive the inverted control loop over a sequence of decision triggers (design
/// §2, §6). The supervisor owns this iteration; the `orchestrator` is invoked as
/// a stateless function at each decision point.
///
/// For each trigger, while the loop is still [`Running`](LoopStatus::Running):
/// 1. A [`DecisionTrigger::CircuitBreakerTripped`] escalates deterministically —
///    the orchestrator is not consulted (design §9 fail-closed).
/// 2. Otherwise, absorb the stage outcome, build a [`DecisionContext`] (with a
///    read-only chunk snapshot) from the *current* revisions, and invoke the
///    orchestrator.
/// 3. For each returned `(action, envelope)`, record a [`DecisionRecord`]:
///    - if the loop already reached a terminal state earlier in this batch, the
///      decision is [`Superseded`](DecisionOutcome::Superseded) (recorded, not run);
///    - a tier-invariant violation is [`RejectedTierViolation`](DecisionOutcome::RejectedTierViolation)
///      and **escalates** the loop (fail-closed);
///    - an unknown-chunk precondition failure is
///      [`RejectedPrecondition`](DecisionOutcome::RejectedPrecondition) (non-halting);
///    - otherwise the action is would-executed; an [`ExecError`] escalates, a
///      success is [`Applied`](DecisionOutcome::Applied) and may itself be terminal.
///
/// Once the loop reaches a terminal state, remaining *triggers* are ignored.
/// Returns the final [`PipelineState`], whose [`decisions`](PipelineState::decisions)
/// trail is the run's audit result.
pub fn drive<O: Orchestrator, E: ActionExecutor>(
    run_id: &str,
    plan: &Plan,
    triggers: Vec<DecisionTrigger>,
    orchestrator: &O,
    executor: &mut E,
) -> PipelineState {
    let mut state = PipelineState::from_plan(run_id, plan);

    for trigger in triggers {
        if state.status != LoopStatus::Running {
            break;
        }

        // Circuit breakers are deterministic and supervisor-owned: escalate
        // directly, never ask the orchestrator (design §9).
        if let DecisionTrigger::CircuitBreakerTripped { reason } = &trigger {
            state.escalate(format!("circuit breaker: {reason}"));
            break;
        }

        state.absorb_trigger(&trigger);
        let ctx = DecisionContext {
            run_id: state.run_id.clone(),
            plan_rev: state.plan_rev,
            intent_rev: state.intent_rev,
            chunks: state.chunks.clone(),
            trigger,
        };

        for (action, envelope) in orchestrator.decide(&ctx) {
            // A terminal action earlier in this batch supersedes the rest — still
            // recorded so the audit trail loses nothing (design §2).
            if state.status != LoopStatus::Running {
                state.decisions.push(DecisionRecord {
                    action,
                    envelope,
                    outcome: DecisionOutcome::Superseded,
                });
                continue;
            }

            // Tier invariant: a consequential action stamped coordinator-tier is
            // an authority breach — reject it and escalate (fail-closed).
            if let Err(violation) = envelope.validate_for(&action) {
                state.decisions.push(DecisionRecord {
                    action,
                    envelope,
                    outcome: DecisionOutcome::RejectedTierViolation(violation),
                });
                state.escalate("tier invariant violation".to_string());
                continue;
            }

            // Precondition: referenced chunks must exist, checked before any side
            // effect. Non-halting — a single bad reference rejects that action.
            if let Err(reason) = state.check_preconditions(&action) {
                state.anomalies.push(reason.clone());
                state.decisions.push(DecisionRecord {
                    action,
                    envelope,
                    outcome: DecisionOutcome::RejectedPrecondition { reason },
                });
                continue;
            }

            match executor.execute(&action) {
                Ok(()) => {
                    state.apply(&action);
                    state.decisions.push(DecisionRecord {
                        action,
                        envelope,
                        outcome: DecisionOutcome::Applied,
                    });
                }
                Err(e) => {
                    state.decisions.push(DecisionRecord {
                        action,
                        envelope,
                        outcome: DecisionOutcome::ExecutionFailed(e.clone()),
                    });
                    state.escalate(format!("execution failure: {}", e.action));
                }
            }
        }
    }

    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::action::{Finding, FindingVerdict, Severity, SpinoffScope};
    use crate::pipeline::envelope::DecisionTier;
    use crate::pipeline::orchestrator::{
        CoordinatorProposal, DeciderVerdict, ScriptedCoordinator, ScriptedDecider,
        TieredOrchestrator,
    };
    use serde_json::json;

    /// A two-chunk plan (`c1`, then `c2` depending on `c1`).
    fn plan() -> Plan {
        let v = json!({
            "schema_version": 3, "plan_rev": 1, "intent_rev": 1,
            "feature": {"slug": "f", "source_branch": "main", "integration_branch": "feat/f"},
            "baseline": {"ref": "feat/f@fork", "commit_oid": "0123456789abcdef0123456789abcdef01234567", "toolchain": "rustc 1.97.1", "test_passlist_hash": "h", "clippy_warnings_hash": "h", "enumerated_targets_hash": "h"},
            "acceptance": [{"kind": "check", "desc": "e2e", "run": "cargo test"}],
            "chunks": [
                {"id": "c1", "title": "t", "tier": "code", "brief": "b", "files_touched": ["a.rs"], "checks": [{"desc": "d", "run": "r"}]},
                {"id": "c2", "title": "t", "tier": "code", "brief": "b", "deps": ["c1"], "files_touched": ["b.rs"], "checks": [{"desc": "d", "run": "r"}]},
            ],
        });
        octl_core::plan::parse_and_validate_plan(&v).expect("fixture plan must validate")
    }

    fn proposal(action: Action) -> CoordinatorProposal {
        CoordinatorProposal {
            action,
            reason: "r".into(),
            input_artifacts: vec!["a".into()],
        }
    }

    fn finding(verdict: FindingVerdict) -> Finding {
        Finding {
            id: "f1".into(),
            summary: "s".into(),
            verdict,
            severity: Severity::High,
        }
    }

    fn verify(report_id: &str, findings: Vec<Finding>) -> DecisionTrigger {
        DecisionTrigger::VerifyReport {
            report_id: report_id.into(),
            findings,
        }
    }

    #[test]
    fn routine_fix_loop_recodes_then_accepts() {
        // VerifyReport(Fix) → RE_CODE_CHUNK (routine); then AcceptChunk (routine).
        // Both stay coordinator-tier; the chunk ends Accepted, no violations.
        let coord = ScriptedCoordinator::new(vec![
            vec![proposal(Action::ReCodeChunk {
                chunk_id: "c1".into(),
                findings: vec![finding(FindingVerdict::Fix)],
            })],
            vec![proposal(Action::AcceptChunk {
                chunk_id: "c1".into(),
            })],
        ]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();

        let triggers = vec![
            verify("v1", vec![finding(FindingVerdict::Fix)]),
            verify("v2", vec![]),
        ];
        let state = drive("run1", &plan(), triggers, &orch, &mut exec);

        assert_eq!(state.status, LoopStatus::Running);
        assert_eq!(state.chunks["c1"].status, ChunkStatus::Accepted);
        assert_eq!(exec.attempted.len(), 2);
        assert_eq!(state.applied_actions().count(), 2);
        assert_eq!(state.decisions[0].action.name(), "re_code_chunk");
        assert_eq!(state.decisions[1].action.name(), "accept_chunk");
        assert!(state
            .decisions
            .iter()
            .all(|d| d.envelope.decision_tier == DecisionTier::Coordinator));
        assert_eq!(state.tier_violations().count(), 0);
        assert!(state.anomalies.is_empty());
    }

    #[test]
    fn chunk_committed_trigger_moves_chunk_to_awaiting_verify() {
        let coord = ScriptedCoordinator::new(vec![vec![]]); // no decision
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();

        let state = drive(
            "run1",
            &plan(),
            vec![DecisionTrigger::ChunkCommitted {
                chunk_id: "c1".into(),
            }],
            &orch,
            &mut exec,
        );
        assert_eq!(state.chunks["c1"].status, ChunkStatus::AwaitingVerify);
        assert!(exec.attempted.is_empty());
    }

    #[test]
    fn decision_context_carries_chunk_snapshot() {
        // The stateless orchestrator must see the current chunk states to decide.
        // Capture the snapshot the driver passes.
        use crate::pipeline::envelope::DecisionEnvelope;
        use std::cell::RefCell;

        struct SnoopingOrchestrator {
            seen: RefCell<Vec<BTreeMap<String, ChunkState>>>,
        }
        impl Orchestrator for SnoopingOrchestrator {
            fn decide(&self, ctx: &DecisionContext) -> Vec<(Action, DecisionEnvelope)> {
                self.seen.borrow_mut().push(ctx.chunks.clone());
                Vec::new()
            }
        }

        let orch = SnoopingOrchestrator {
            seen: RefCell::new(Vec::new()),
        };
        let mut exec = RecordingExecutor::new();
        drive(
            "run1",
            &plan(),
            vec![DecisionTrigger::ChunkCommitted {
                chunk_id: "c1".into(),
            }],
            &orch,
            &mut exec,
        );
        let seen = orch.seen.into_inner();
        assert_eq!(seen.len(), 1);
        // c1 was moved to AwaitingVerify by absorb_trigger before the snapshot.
        assert_eq!(seen[0]["c1"].status, ChunkStatus::AwaitingVerify);
        assert_eq!(seen[0]["c2"].status, ChunkStatus::Pending);
    }

    #[test]
    fn declare_converged_is_decider_tier_and_terminal() {
        // The coordinator *proposes* converge; the tiered wrapper defers to the
        // decider, so the recorded envelope is decider-tier. The loop terminates.
        let coord = ScriptedCoordinator::new(vec![vec![proposal(Action::DeclareConverged)]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();

        // A trailing trigger proves terminal states stop the loop.
        let triggers = vec![
            verify("v1", vec![]),
            DecisionTrigger::ChunkCommitted {
                chunk_id: "c1".into(),
            },
        ];
        let state = drive("run1", &plan(), triggers, &orch, &mut exec);

        assert_eq!(state.status, LoopStatus::Converged);
        assert_eq!(state.decisions.len(), 1);
        assert_eq!(
            state.decisions[0].envelope.decision_tier,
            DecisionTier::Decider
        );
        // The trailing ChunkCommitted was ignored — c1 never moved.
        assert_eq!(state.chunks["c1"].status, ChunkStatus::Pending);
    }

    #[test]
    fn escalate_is_decider_tier_and_terminal() {
        let coord = ScriptedCoordinator::new(vec![vec![proposal(Action::Escalate {
            reason: "cannot converge".into(),
        })]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();

        let state = drive(
            "run1",
            &plan(),
            vec![verify("v1", vec![])],
            &orch,
            &mut exec,
        );
        assert_eq!(state.status, LoopStatus::Escalated);
        assert_eq!(state.escalation.as_deref(), Some("cannot converge"));
        assert_eq!(
            state.decisions[0].envelope.decision_tier,
            DecisionTier::Decider
        );
    }

    #[test]
    fn circuit_breaker_escalates_deterministically_without_the_orchestrator() {
        // A tripped breaker must NOT be routed to the orchestrator (design §9).
        // Script a coordinator that would AcceptChunk if asked — it must not run.
        let coord = ScriptedCoordinator::new(vec![vec![proposal(Action::AcceptChunk {
            chunk_id: "c1".into(),
        })]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();

        let state = drive(
            "run1",
            &plan(),
            vec![DecisionTrigger::CircuitBreakerTripped {
                reason: "cost ceiling".into(),
            }],
            &orch,
            &mut exec,
        );
        assert_eq!(state.status, LoopStatus::Escalated);
        assert_eq!(
            state.escalation.as_deref(),
            Some("circuit breaker: cost ceiling")
        );
        // The orchestrator was never consulted — nothing executed, no decisions.
        assert!(exec.attempted.is_empty());
        assert!(state.decisions.is_empty());
    }

    #[test]
    fn trigger_re_spec_bumps_plan_rev_and_reverts_chunks() {
        // SPEC-FLAW → TRIGGER_RE_SPEC (consequential, decider-tier). plan_rev
        // increments; listed chunks revert to Pending.
        let coord = ScriptedCoordinator::new(vec![vec![proposal(Action::TriggerReSpec {
            reason: "spec cannot meet intent".into(),
            chunk_ids: vec!["c1".into(), "c2".into()],
        })]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();

        // Pre-move c1 to AwaitingVerify so the revert-to-Pending is observable.
        let triggers = vec![
            DecisionTrigger::ChunkCommitted {
                chunk_id: "c1".into(),
            },
            verify("v1", vec![finding(FindingVerdict::SpecFlaw)]),
        ];
        let state = drive("run1", &plan(), triggers, &orch, &mut exec);

        assert_eq!(state.plan_rev, 2);
        assert_eq!(state.chunks["c1"].status, ChunkStatus::Pending);
        assert_eq!(state.chunks["c2"].status, ChunkStatus::Pending);
        assert_eq!(
            state.decisions.last().unwrap().envelope.decision_tier,
            DecisionTier::Decider
        );
        assert_eq!(state.status, LoopStatus::Running);
    }

    #[test]
    fn promote_tier_bumps_tier_and_resets_to_pending() {
        let coord = ScriptedCoordinator::new(vec![vec![proposal(Action::PromoteTier {
            chunk_id: "c1".into(),
            tier: Tier::High,
        })]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();

        // Move c1 to AwaitingVerify first, then promote — it must go back to
        // Pending (re-run at the new tier), not stay in the failed state.
        let triggers = vec![
            DecisionTrigger::ChunkCommitted {
                chunk_id: "c1".into(),
            },
            verify("v1", vec![]),
        ];
        let state = drive("run1", &plan(), triggers, &orch, &mut exec);
        assert_eq!(state.chunks["c1"].tier, Tier::High);
        assert_eq!(state.chunks["c1"].status, ChunkStatus::Pending);
    }

    /// A test double that returns a fixed `(Action, DecisionEnvelope)` — used to
    /// feed the driver a *mis-tiered* decision the tiered wrapper would never
    /// produce, so the driver's rejection path is exercised directly.
    struct FixedOrchestrator(Vec<(Action, DecisionEnvelope)>);
    use crate::pipeline::envelope::DecisionEnvelope;
    impl Orchestrator for FixedOrchestrator {
        fn decide(&self, _ctx: &DecisionContext) -> Vec<(Action, DecisionEnvelope)> {
            self.0.clone()
        }
    }

    #[test]
    fn mis_tiered_consequential_action_is_rejected_and_escalates() {
        // A consequential action (DeclareConverged) stamped coordinator-tier is
        // the invariant violation the audit must catch. The driver flags it, does
        // NOT execute it, and fails closed (escalates).
        let bad_envelope = DecisionEnvelope {
            actor: "coordinator".into(),
            input_artifacts: vec![],
            reason: "fast tier over-reached".into(),
            decision_tier: DecisionTier::Coordinator,
            model: "cheap".into(),
            prompt_version: "v1".into(),
        };
        let orch = FixedOrchestrator(vec![(Action::DeclareConverged, bad_envelope)]);
        let mut exec = RecordingExecutor::new();

        let state = drive(
            "run1",
            &plan(),
            vec![verify("v1", vec![])],
            &orch,
            &mut exec,
        );

        assert_eq!(state.tier_violations().count(), 1);
        assert_eq!(
            state.tier_violations().next().unwrap().action,
            "declare_converged"
        );
        // Rejected: never executed; loop failed closed (escalated), NOT converged.
        assert!(exec.attempted.is_empty());
        assert_eq!(state.status, LoopStatus::Escalated);
        assert_eq!(
            state.escalation.as_deref(),
            Some("tier invariant violation")
        );
        // The envelope is still recorded for the audit trail.
        assert_eq!(state.decisions.len(), 1);
    }

    #[test]
    fn post_terminal_actions_in_a_batch_are_recorded_superseded() {
        // A batch [DeclareConverged, ProposeSpinoff] must not silently drop the
        // spinoff — it is recorded Superseded (audit completeness) but not run.
        let coord = ScriptedCoordinator::new(vec![vec![
            proposal(Action::DeclareConverged),
            proposal(Action::ProposeSpinoff {
                title: "later".into(),
                kind: "improvement".into(),
                rationale: "r".into(),
                scope: SpinoffScope::Trivial,
            }),
        ]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();

        let state = drive(
            "run1",
            &plan(),
            vec![verify("v1", vec![])],
            &orch,
            &mut exec,
        );
        assert_eq!(state.status, LoopStatus::Converged);
        assert_eq!(state.decisions.len(), 2);
        assert_eq!(state.decisions[0].outcome, DecisionOutcome::Applied);
        assert_eq!(state.decisions[1].outcome, DecisionOutcome::Superseded);
        // The superseded spinoff was NOT applied.
        assert!(state.spinoffs.is_empty());
    }

    #[test]
    fn unknown_chunk_is_rejected_before_execution() {
        // An action naming a nonexistent chunk is rejected on the precondition,
        // before any side effect — the executor is never called for it.
        let coord = ScriptedCoordinator::new(vec![vec![proposal(Action::AcceptChunk {
            chunk_id: "ghost".into(),
        })]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();

        let state = drive(
            "run1",
            &plan(),
            vec![verify("v1", vec![])],
            &orch,
            &mut exec,
        );
        assert!(exec.attempted.is_empty()); // never executed
        assert_eq!(state.anomalies.len(), 1);
        assert!(matches!(
            state.decisions[0].outcome,
            DecisionOutcome::RejectedPrecondition { .. }
        ));
        // Non-halting: the loop keeps running after a rejected precondition.
        assert_eq!(state.status, LoopStatus::Running);
    }

    #[test]
    fn execution_failure_escalates_the_loop() {
        let coord = ScriptedCoordinator::new(vec![vec![proposal(Action::AcceptChunk {
            chunk_id: "c1".into(),
        })]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::failing_on(&["accept_chunk"]);

        let state = drive(
            "run1",
            &plan(),
            vec![verify("v1", vec![])],
            &orch,
            &mut exec,
        );
        assert_eq!(state.status, LoopStatus::Escalated);
        assert_eq!(state.exec_failures().count(), 1);
        assert_eq!(state.exec_failures().next().unwrap().action, "accept_chunk");
        assert_eq!(state.chunks["c1"].status, ChunkStatus::Pending); // never applied
    }

    #[test]
    fn spinoff_and_discussion_are_recorded_non_blocking() {
        let coord = ScriptedCoordinator::new(vec![vec![
            proposal(Action::OpenDiscussion {
                topic: "api shape".into(),
                severity: Severity::Medium,
            }),
            proposal(Action::ProposeSpinoff {
                title: "extract helper".into(),
                kind: "refactor".into(),
                rationale: "out of scope".into(),
                scope: SpinoffScope::Trivial, // routine → stays in the batch
            }),
        ]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();

        let state = drive(
            "run1",
            &plan(),
            vec![verify(
                "v1",
                vec![
                    finding(FindingVerdict::Discuss),
                    finding(FindingVerdict::SpinOff),
                ],
            )],
            &orch,
            &mut exec,
        );
        assert_eq!(state.discussions.len(), 1);
        assert_eq!(state.discussions[0].topic, "api shape");
        assert_eq!(state.spinoffs.len(), 1);
        assert_eq!(state.spinoffs[0].title, "extract helper");
        assert_eq!(state.spinoffs[0].scope, SpinoffScope::Trivial);
        assert_eq!(state.status, LoopStatus::Running); // non-blocking
    }

    #[test]
    fn state_round_trips_through_serde() {
        let coord = ScriptedCoordinator::new(vec![vec![proposal(Action::AcceptChunk {
            chunk_id: "c1".into(),
        })]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();
        let state = drive(
            "run1",
            &plan(),
            vec![verify("v1", vec![])],
            &orch,
            &mut exec,
        );
        let v = serde_json::to_value(&state).unwrap();
        let back: PipelineState = serde_json::from_value(v).unwrap();
        assert_eq!(back, state);
    }
}
