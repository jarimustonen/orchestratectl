//! The supervisor-side loop skeleton (design.md §2, §6) modelled as a pure
//! in-memory state machine.
//!
//! [`drive`] is the inverted loop: the supervisor owns iteration, and at each
//! decision point it calls the stateless [`Orchestrator`], **validates** the
//! returned primitives against the tier invariant, **would-execute** them (via a
//! stubbed [`ActionExecutor`] — real git/merge/spawn is T5), **records** each
//! [`DecisionEnvelope`], and applies the effect to an in-memory [`PipelineState`].
//! Terminal actions (`DeclareConverged`, `Escalate`) halt the loop.
//!
//! Modelling it as a pure state machine over an in-memory state (rather than
//! bolting a new event-append path onto the live reducer) keeps it fully
//! unit-testable and respects the state-integrity invariants: T5 wires the real
//! event log in behind the same shape, appending through the `LockedRun` witness
//! and the `append_and_apply` API (invariant 1) rather than writing projections
//! directly.

use std::collections::BTreeMap;

use octl_core::plan::{Plan, Tier};
use serde::{Deserialize, Serialize};

use super::action::Action;
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
/// merge / spawn failures onto it.
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
    /// Actions this executor was asked to run, in order.
    pub executed: Vec<Action>,
    /// [`Action::name`]s to fail on (empty = never fail).
    fail_on: Vec<&'static str>,
}

impl RecordingExecutor {
    /// A recording executor that never fails.
    pub fn new() -> Self {
        Self::default()
    }

    /// A recording executor that returns [`ExecError`] for any action whose
    /// [`Action::name`] is in `names` (for exercising the driver's failure path).
    pub fn failing_on(names: &[&'static str]) -> Self {
        Self {
            executed: Vec::new(),
            fail_on: names.to_vec(),
        }
    }
}

impl ActionExecutor for RecordingExecutor {
    fn execute(&mut self, action: &Action) -> Result<(), ExecError> {
        if self.fail_on.contains(&action.name()) {
            return Err(ExecError {
                action: action.name().to_string(),
                message: "scripted failure".to_string(),
            });
        }
        self.executed.push(action.clone());
        Ok(())
    }
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
    /// `Escalate` fired (or an execution failure) — control hands up (design §9).
    Escalated,
}

/// Per-chunk status the loop tracks in memory (design §7 lifecycle). A coarse
/// model of the chunk lifecycle sufficient for the scaffold; T5 refines it
/// against the real DAG scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStatus {
    /// Not yet started (or reverted by a re-spec).
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// The whole in-memory state of one feature's loop — the audit-bearing result of
/// [`drive`]. Everything a T5 wiring would persist to the event log lives here:
/// the chunk states, the ordered [`envelopes`](PipelineState::envelopes) audit
/// trail, the flagged tier [`violations`](PipelineState::violations), and the
/// deferred discussion/spin-off records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineState {
    /// The run this loop belongs to.
    pub run_id: String,
    /// The plan revision in force (bumped by `TriggerReSpec`).
    pub plan_rev: u32,
    /// The intent revision the plan targets.
    pub intent_rev: u32,
    /// Where the loop stands.
    pub status: LoopStatus,
    /// Chunk states keyed by chunk id (ordered for deterministic inspection).
    pub chunks: BTreeMap<String, ChunkState>,
    /// Every recorded decision envelope, in order — the causal audit trail
    /// (design §2). A flagged violation's envelope is recorded here too.
    pub envelopes: Vec<DecisionEnvelope>,
    /// Tier-invariant breaches the driver caught and refused to execute.
    pub violations: Vec<TierViolation>,
    /// Execution failures reported by the [`ActionExecutor`].
    pub exec_errors: Vec<ExecError>,
    /// Actions that targeted a chunk id not in the plan (a driver-level anomaly;
    /// empty on any well-formed run).
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
            envelopes: Vec::new(),
            violations: Vec::new(),
            exec_errors: Vec::new(),
            anomalies: Vec::new(),
            discussions: Vec::new(),
            spinoffs: Vec::new(),
            escalation: None,
        }
    }

    /// Stage-outcome bookkeeping applied *before* the decision (design §6: the
    /// supervisor consumes stage outcomes). A committed chunk moves to
    /// `AwaitingVerify`; the other triggers carry no pre-decision state change.
    fn absorb_trigger(&mut self, trigger: &DecisionTrigger) {
        if let DecisionTrigger::ChunkCommitted { chunk_id } = trigger {
            self.set_chunk_status(chunk_id, ChunkStatus::AwaitingVerify);
        }
    }

    /// Apply the *effect* of an executed action to the in-memory state. Called
    /// only after the action passed tier validation and was would-executed.
    fn apply(&mut self, action: &Action) {
        match action {
            Action::ReCodeChunk { chunk_id, .. } => {
                self.set_chunk_status(chunk_id, ChunkStatus::NeedsReverify);
            }
            Action::AcceptChunk { chunk_id } => {
                self.set_chunk_status(chunk_id, ChunkStatus::Accepted);
            }
            Action::PromoteTier { chunk_id, tier } => {
                if let Some(chunk) = self.chunks.get_mut(chunk_id) {
                    chunk.tier = *tier;
                } else {
                    self.note_unknown_chunk(action, chunk_id);
                }
            }
            Action::TriggerReSpec { chunk_ids, .. } => {
                self.plan_rev += 1;
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
                ..
            } => {
                self.spinoffs.push(SpinoffRecord {
                    title: title.clone(),
                    kind: kind.clone(),
                    rationale: rationale.clone(),
                });
            }
            Action::DeclareConverged => {
                self.status = LoopStatus::Converged;
            }
            Action::Escalate { reason } => {
                self.status = LoopStatus::Escalated;
                self.escalation = Some(reason.clone());
            }
        }
    }

    /// Set a chunk's status, recording an anomaly if the id is not in the plan.
    fn set_chunk_status(&mut self, chunk_id: &str, status: ChunkStatus) {
        if let Some(chunk) = self.chunks.get_mut(chunk_id) {
            chunk.status = status;
        } else {
            self.anomalies
                .push(format!("action referenced unknown chunk `{chunk_id}`"));
        }
    }

    /// Record an anomaly for an action targeting an unknown chunk.
    fn note_unknown_chunk(&mut self, action: &Action, chunk_id: &str) {
        self.anomalies.push(format!(
            "action `{}` referenced unknown chunk `{chunk_id}`",
            action.name()
        ));
    }
}

/// Drive the inverted control loop over a sequence of decision triggers (design
/// §2, §6). The supervisor owns this iteration; the `orchestrator` is invoked as
/// a stateless function at each decision point.
///
/// For each trigger, while the loop is still [`Running`](LoopStatus::Running):
/// 1. absorb the stage outcome into state;
/// 2. build a [`DecisionContext`] from the *current* revisions and invoke the
///    orchestrator;
/// 3. for each returned `(action, envelope)`, record the envelope, then
///    **validate the tier invariant** — a
///    [`TierViolation`](super::envelope::TierViolation) is flagged and the action
///    is **not** executed (a mis-tiered consequential decision is rejected);
/// 4. would-execute the action via `executor`; an [`ExecError`] escalates the
///    loop;
/// 5. apply the effect and stop early on a terminal action.
///
/// Once the loop reaches a terminal state, remaining triggers are ignored (the
/// feature is done or has escalated). Returns the final [`PipelineState`], whose
/// envelope trail + violation list + records are the run's audit result.
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
        state.absorb_trigger(&trigger);
        let ctx = DecisionContext {
            run_id: state.run_id.clone(),
            plan_rev: state.plan_rev,
            intent_rev: state.intent_rev,
            trigger,
        };

        for (action, envelope) in orchestrator.decide(&ctx) {
            // The envelope is always recorded — even a flagged decision is part
            // of the causal audit trail (design §2).
            let violation = envelope.validate_for(&action).err();
            state.envelopes.push(envelope);
            if let Some(v) = violation {
                // A consequential action stamped coordinator-tier: reject it —
                // do not execute a final decision the fast tier was not allowed
                // to make (design §0.2). It stays flagged for audit.
                state.violations.push(v);
                continue;
            }

            match executor.execute(&action) {
                Ok(()) => state.apply(&action),
                Err(e) => {
                    state.exec_errors.push(e);
                    state.status = LoopStatus::Escalated;
                    state.escalation = Some("execution failure".to_string());
                    break;
                }
            }

            if state.status != LoopStatus::Running {
                // A terminal action fired; stop applying the rest of this batch.
                break;
            }
        }
    }

    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::action::{Finding, FindingVerdict, Severity};
    use crate::pipeline::envelope::DecisionTier;
    use crate::pipeline::orchestrator::{
        CoordinatorProposal, DeciderVerdict, ScriptedCoordinator, ScriptedDecider,
        TieredOrchestrator,
    };
    use serde_json::json;

    /// A two-chunk plan (`c1`, then `c2` depending on `c1`).
    fn plan() -> Plan {
        let v = json!({
            "schema_version": 2, "plan_rev": 1, "intent_rev": 1,
            "feature": {"slug": "f", "source_branch": "main", "integration_branch": "feat/f"},
            "baseline": {"ref": "feat/f@fork", "test_passlist_hash": "h", "clippy_warnings_hash": "h"},
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
            DecisionTrigger::VerifyReport {
                report_id: "v1".into(),
                findings: vec![finding(FindingVerdict::Fix)],
            },
            DecisionTrigger::VerifyReport {
                report_id: "v2".into(),
                findings: vec![],
            },
        ];
        let state = drive("run1", &plan(), triggers, &orch, &mut exec);

        assert_eq!(state.status, LoopStatus::Running);
        assert_eq!(state.chunks["c1"].status, ChunkStatus::Accepted);
        assert_eq!(exec.executed.len(), 2);
        assert_eq!(exec.executed[0].name(), "re_code_chunk");
        assert_eq!(exec.executed[1].name(), "accept_chunk");
        assert_eq!(state.envelopes.len(), 2);
        assert!(state
            .envelopes
            .iter()
            .all(|e| e.decision_tier == DecisionTier::Coordinator));
        assert!(state.violations.is_empty());
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
        assert!(exec.executed.is_empty());
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
            DecisionTrigger::VerifyReport {
                report_id: "v1".into(),
                findings: vec![],
            },
            DecisionTrigger::ChunkCommitted {
                chunk_id: "c1".into(),
            },
        ];
        let state = drive("run1", &plan(), triggers, &orch, &mut exec);

        assert_eq!(state.status, LoopStatus::Converged);
        assert_eq!(state.envelopes.len(), 1);
        assert_eq!(state.envelopes[0].decision_tier, DecisionTier::Decider);
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
            vec![DecisionTrigger::CircuitBreakerTripped {
                reason: "cost ceiling".into(),
            }],
            &orch,
            &mut exec,
        );
        assert_eq!(state.status, LoopStatus::Escalated);
        assert_eq!(state.escalation.as_deref(), Some("cannot converge"));
        assert_eq!(state.envelopes[0].decision_tier, DecisionTier::Decider);
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
            DecisionTrigger::VerifyReport {
                report_id: "v1".into(),
                findings: vec![finding(FindingVerdict::SpecFlaw)],
            },
        ];
        let state = drive("run1", &plan(), triggers, &orch, &mut exec);

        assert_eq!(state.plan_rev, 2);
        assert_eq!(state.chunks["c1"].status, ChunkStatus::Pending);
        assert_eq!(state.chunks["c2"].status, ChunkStatus::Pending);
        assert_eq!(
            state.envelopes.last().unwrap().decision_tier,
            DecisionTier::Decider
        );
        assert_eq!(state.status, LoopStatus::Running);
    }

    #[test]
    fn promote_tier_bumps_the_chunk_tier() {
        let coord = ScriptedCoordinator::new(vec![vec![proposal(Action::PromoteTier {
            chunk_id: "c1".into(),
            tier: Tier::High,
        })]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();

        let state = drive(
            "run1",
            &plan(),
            vec![DecisionTrigger::VerifyReport {
                report_id: "v1".into(),
                findings: vec![],
            }],
            &orch,
            &mut exec,
        );
        assert_eq!(state.chunks["c1"].tier, Tier::High);
    }

    /// A test double that returns a fixed `(Action, DecisionEnvelope)` — used to
    /// feed the driver a *mis-tiered* decision the tiered wrapper would never
    /// produce, so the driver's rejection path is exercised directly.
    struct FixedOrchestrator(Vec<(Action, DecisionEnvelope)>);
    impl Orchestrator for FixedOrchestrator {
        fn decide(&self, _ctx: &DecisionContext) -> Vec<(Action, DecisionEnvelope)> {
            self.0.clone()
        }
    }

    #[test]
    fn mis_tiered_consequential_action_is_rejected_not_executed() {
        // A consequential action (DeclareConverged) stamped coordinator-tier is
        // the invariant violation the audit must catch. The driver flags it and
        // does NOT execute it — so the loop never converges on it.
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
            vec![DecisionTrigger::VerifyReport {
                report_id: "v1".into(),
                findings: vec![],
            }],
            &orch,
            &mut exec,
        );

        assert_eq!(state.violations.len(), 1);
        assert_eq!(state.violations[0].action, "declare_converged");
        // Rejected: never executed, loop did NOT converge, but the envelope is
        // still recorded for the audit trail.
        assert!(exec.executed.is_empty());
        assert_eq!(state.status, LoopStatus::Running);
        assert_eq!(state.envelopes.len(), 1);
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
            vec![DecisionTrigger::VerifyReport {
                report_id: "v1".into(),
                findings: vec![],
            }],
            &orch,
            &mut exec,
        );
        assert_eq!(state.status, LoopStatus::Escalated);
        assert_eq!(state.exec_errors.len(), 1);
        assert_eq!(state.exec_errors[0].action, "accept_chunk");
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
                severity: Severity::Low, // trivial → routine, stays in the batch
            }),
        ]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        let mut exec = RecordingExecutor::new();

        let state = drive(
            "run1",
            &plan(),
            vec![DecisionTrigger::VerifyReport {
                report_id: "v1".into(),
                findings: vec![
                    finding(FindingVerdict::Discuss),
                    finding(FindingVerdict::SpinOff),
                ],
            }],
            &orch,
            &mut exec,
        );
        assert_eq!(state.discussions.len(), 1);
        assert_eq!(state.discussions[0].topic, "api shape");
        assert_eq!(state.spinoffs.len(), 1);
        assert_eq!(state.spinoffs[0].title, "extract helper");
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
            vec![DecisionTrigger::VerifyReport {
                report_id: "v1".into(),
                findings: vec![],
            }],
            &orch,
            &mut exec,
        );
        let v = serde_json::to_value(&state).unwrap();
        let back: PipelineState = serde_json::from_value(v).unwrap();
        assert_eq!(back, state);
    }
}
