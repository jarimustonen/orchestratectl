//! The orchestrator as a stateless pure function (design.md §2) and its tiered
//! implementation (design §0.2).
//!
//! The supervisor calls [`Orchestrator::decide`] at each decision point with a
//! [`DecisionContext`] and gets back a list of `(Action, DecisionEnvelope)`
//! pairs. The orchestrator holds no loop state of its own — everything it needs
//! is in the context (state lives in the supervisor's event log, design §3).
//!
//! [`TieredOrchestrator`] is the concrete wrapper: a fast [`Coordinator`]
//! proposes actions and classifies each; every
//! [`Consequential`](super::action::DecisionClass::Consequential) proposal is
//! deferred to an expensive [`Decider`], whose verdict is the recorded one. By
//! construction the wrapper **never** emits a consequential action stamped
//! coordinator-tier — the tier invariant holds without the driver having to
//! reject anything on the happy path.
//!
//! [`ScriptedCoordinator`] / [`ScriptedDecider`] are deterministic, in-process
//! stubs (no model, no network) so the whole loop is unit-testable.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::action::{Action, DecisionClass, Finding};
use super::driver::ChunkState;
use super::envelope::{DecisionEnvelope, DecisionTier};

/// What just happened in the pipeline that requires a decision (design §6 loop
/// steps). The supervisor builds a [`DecisionContext`] around one of these and
/// invokes the orchestrator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "trigger", rename_all = "snake_case")]
pub enum DecisionTrigger {
    /// The spec node produced a plan revision; proceed or (optionally) escalate an
    /// architectural question (design §6 VAIHE 1).
    SpecReady,
    /// A code node committed its chunk branch (design §6 VAIHE 2).
    ChunkCommitted {
        /// The chunk that was committed.
        chunk_id: String,
    },
    /// A verify pass produced findings to triage (design §6 VAIHE 3, §8).
    VerifyReport {
        /// The verify report's id (an input artifact for the envelope).
        report_id: String,
        /// The findings to triage into actions.
        findings: Vec<Finding>,
    },
    /// A resource circuit-breaker tripped (design §9). This is **not** routed to
    /// the orchestrator: a breaker is deterministic and supervisor-owned, so the
    /// driver escalates the loop directly rather than trusting an LLM to "pull the
    /// brake." The trigger exists so the loop can consume it; the driver never
    /// asks the orchestrator what to do about it.
    CircuitBreakerTripped {
        /// Which ceiling was breached (cost, wall-time, repeated-failure, …).
        reason: String,
    },
}

/// Everything the stateless orchestrator sees at one decision point (design §2:
/// `Triage(verify_report, plan_rev, intent_rev) -> Action[]`). It carries no
/// mutable loop state — the supervisor owns that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionContext {
    /// The run this decision belongs to (causal id).
    pub run_id: String,
    /// The plan revision currently in force (design §7 immutable `plan_rev`).
    pub plan_rev: u32,
    /// The intent revision the plan targets (design §1 orchestrator-owned intent).
    pub intent_rev: u32,
    /// A read-only snapshot of every chunk's status/tier at this decision point.
    /// The orchestrator is stateless — it holds no loop state of its own — so the
    /// supervisor projects the facts it needs to decide (e.g. "are all chunks
    /// accepted?" before `DeclareConverged`) into the context. It is a *snapshot*:
    /// the orchestrator reads it, never mutates it. T5 will widen this into a
    /// fuller trusted projection (DAG, acceptance state, attempt counts, budget);
    /// the chunk map is the minimum a triage decision needs.
    pub chunks: BTreeMap<String, ChunkState>,
    /// What triggered this decision point.
    pub trigger: DecisionTrigger,
}

/// The orchestrator: a **stateless pure function** invoked per decision point
/// (design §2). Returns each chosen [`Action`] paired with the
/// [`DecisionEnvelope`] recording who decided it and at which tier.
pub trait Orchestrator {
    /// Decide what to do at this decision point. May return zero, one, or several
    /// actions (e.g. re-code two chunks and open a discussion). Each pair's
    /// envelope must satisfy the tier invariant
    /// ([`DecisionEnvelope::validate_for`]); the driver re-checks it defensively.
    fn decide(&self, ctx: &DecisionContext) -> Vec<(Action, DecisionEnvelope)>;
}

/// One action a [`Coordinator`] proposes, with the provenance the tiered wrapper
/// needs to stamp an envelope (minus the tier — the wrapper sets that from which
/// path handled the proposal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinatorProposal {
    /// The proposed action. Its [`Action::decision_class`] decides whether the
    /// wrapper keeps it (routine) or defers it to the decider (consequential).
    pub action: Action,
    /// Reason summary for the envelope.
    pub reason: String,
    /// Input artifact ids for the envelope.
    pub input_artifacts: Vec<String>,
}

/// The fast, cheap coordinator tier (design §0.2, §3 "coordinator (PM)"). Emits
/// routine primitives directly and *proposes* consequential ones, which the
/// tiered wrapper routes to the [`Decider`]. Stateless: it reads the context and
/// proposes.
pub trait Coordinator {
    /// Propose actions for this decision point.
    fn coordinate(&self, ctx: &DecisionContext) -> Vec<CoordinatorProposal>;
    /// The concrete model this coordinator runs on (for the envelope).
    fn model(&self) -> String;
    /// The prompt/contract version (for the envelope).
    fn prompt_version(&self) -> String;
    /// The actor label recorded for coordinator-tier decisions.
    fn actor(&self) -> String {
        "coordinator".to_string()
    }
}

/// The authoritative verdict a [`Decider`] returns for one consequential
/// proposal. The decider may **confirm** the coordinator's proposal, **replace**
/// it with a different action, or **soften** it — its action is the recorded one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeciderVerdict {
    /// The authoritative action (may differ from the coordinator's proposal).
    pub action: Action,
    /// Reason summary for the envelope.
    pub reason: String,
    /// Input artifact ids for the envelope.
    pub input_artifacts: Vec<String>,
}

/// The expensive decider tier (design §0.2, §3 "decider [Opus]"). Invoked by the
/// coordinator (via the tiered wrapper) for every final/consequential decision;
/// its verdict is the recorded authority.
pub trait Decider {
    /// Rule on a consequential proposal from the coordinator.
    fn decide_consequential(
        &self,
        ctx: &DecisionContext,
        proposed: &CoordinatorProposal,
    ) -> DeciderVerdict;
    /// The concrete model this decider runs on (for the envelope, e.g. Opus).
    fn model(&self) -> String;
    /// The prompt/contract version (for the envelope).
    fn prompt_version(&self) -> String;
    /// The actor label recorded for decider-tier decisions.
    fn actor(&self) -> String {
        "decider".to_string()
    }
}

/// The tiered orchestrator (design §0.2): a fast [`Coordinator`] `C` proposes
/// actions; every consequential proposal is deferred to an expensive [`Decider`]
/// `D`. The wrapper stamps each resulting envelope with the tier that actually
/// decided, so the tier invariant holds by construction.
pub struct TieredOrchestrator<C, D> {
    coordinator: C,
    decider: D,
}

impl<C: Coordinator, D: Decider> TieredOrchestrator<C, D> {
    /// Wrap a coordinator + decider into a tiered orchestrator.
    pub fn new(coordinator: C, decider: D) -> Self {
        Self {
            coordinator,
            decider,
        }
    }
}

impl<C: Coordinator, D: Decider> Orchestrator for TieredOrchestrator<C, D> {
    fn decide(&self, ctx: &DecisionContext) -> Vec<(Action, DecisionEnvelope)> {
        let mut out = Vec::new();
        for proposal in self.coordinator.coordinate(ctx) {
            match proposal.action.decision_class() {
                DecisionClass::Routine => {
                    // The fast tier is allowed to emit routine primitives directly.
                    let envelope = DecisionEnvelope {
                        actor: self.coordinator.actor(),
                        input_artifacts: proposal.input_artifacts,
                        reason: proposal.reason,
                        decision_tier: DecisionTier::Coordinator,
                        model: self.coordinator.model(),
                        prompt_version: self.coordinator.prompt_version(),
                    };
                    out.push((proposal.action, envelope));
                }
                DecisionClass::Consequential => {
                    // Defer to the expensive tier; its verdict is what we record.
                    let verdict = self.decider.decide_consequential(ctx, &proposal);
                    let envelope = DecisionEnvelope {
                        actor: self.decider.actor(),
                        input_artifacts: verdict.input_artifacts,
                        reason: verdict.reason,
                        decision_tier: DecisionTier::Decider,
                        model: self.decider.model(),
                        prompt_version: self.decider.prompt_version(),
                    };
                    out.push((verdict.action, envelope));
                }
            }
        }
        out
    }
}

/// A deterministic, scripted [`Coordinator`] stub (no model, no network). Each
/// [`coordinate`](Coordinator::coordinate) call pops the next scripted batch of
/// proposals; an exhausted script yields an empty batch (no proposals). Interior
/// mutability ([`RefCell`]) lets it advance the script behind the `&self` the
/// stateless-orchestrator contract requires.
pub struct ScriptedCoordinator {
    script: RefCell<VecDeque<Vec<CoordinatorProposal>>>,
    model: String,
    prompt_version: String,
}

impl ScriptedCoordinator {
    /// Build a coordinator that returns `batches[i]` on its `i`-th call.
    pub fn new(batches: Vec<Vec<CoordinatorProposal>>) -> Self {
        Self {
            script: RefCell::new(batches.into()),
            model: "stub-coordinator".to_string(),
            prompt_version: "stub-v1".to_string(),
        }
    }
}

impl Coordinator for ScriptedCoordinator {
    fn coordinate(&self, _ctx: &DecisionContext) -> Vec<CoordinatorProposal> {
        self.script.borrow_mut().pop_front().unwrap_or_default()
    }
    fn model(&self) -> String {
        self.model.clone()
    }
    fn prompt_version(&self) -> String {
        self.prompt_version.clone()
    }
}

/// A deterministic, scripted [`Decider`] stub (no model, no network). Each
/// [`decide_consequential`](Decider::decide_consequential) call pops the next
/// scripted verdict; an exhausted script **confirms** the coordinator's proposal
/// (the safe default — the decider ratifies what was proposed). Interior
/// mutability advances the script behind `&self`.
pub struct ScriptedDecider {
    script: RefCell<VecDeque<DeciderVerdict>>,
    model: String,
    prompt_version: String,
}

impl ScriptedDecider {
    /// Build a decider that returns `verdicts[i]` on its `i`-th call and confirms
    /// the proposal once the script runs dry.
    pub fn new(verdicts: Vec<DeciderVerdict>) -> Self {
        Self {
            script: RefCell::new(verdicts.into()),
            model: "stub-decider".to_string(),
            prompt_version: "stub-v1".to_string(),
        }
    }

    /// A decider with no scripted verdicts — it confirms every proposal. Handy
    /// for tests that only care that consequential proposals get decider-tier
    /// stamping, not that the action changes.
    pub fn confirming() -> Self {
        Self::new(Vec::new())
    }
}

impl Decider for ScriptedDecider {
    fn decide_consequential(
        &self,
        _ctx: &DecisionContext,
        proposed: &CoordinatorProposal,
    ) -> DeciderVerdict {
        self.script
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| DeciderVerdict {
                action: proposed.action.clone(),
                reason: format!("decider confirmed: {}", proposed.reason),
                input_artifacts: proposed.input_artifacts.clone(),
            })
    }
    fn model(&self) -> String {
        self.model.clone()
    }
    fn prompt_version(&self) -> String {
        self.prompt_version.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::action::SpinoffScope;

    fn ctx() -> DecisionContext {
        DecisionContext {
            run_id: "run1".into(),
            plan_rev: 1,
            intent_rev: 1,
            chunks: BTreeMap::new(),
            trigger: DecisionTrigger::SpecReady,
        }
    }

    fn proposal(action: Action) -> CoordinatorProposal {
        CoordinatorProposal {
            action,
            reason: "because".into(),
            input_artifacts: vec!["plan:1".into()],
        }
    }

    #[test]
    fn routine_proposal_is_stamped_coordinator() {
        let coord = ScriptedCoordinator::new(vec![vec![proposal(Action::AcceptChunk {
            chunk_id: "c1".into(),
        })]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());

        let decisions = orch.decide(&ctx());
        assert_eq!(decisions.len(), 1);
        let (action, env) = &decisions[0];
        assert_eq!(action.name(), "accept_chunk");
        assert_eq!(env.decision_tier, DecisionTier::Coordinator);
        // The invariant holds by construction.
        assert!(env.validate_for(action).is_ok());
    }

    #[test]
    fn consequential_proposal_is_deferred_and_stamped_decider() {
        // The coordinator *proposes* a consequential action; the wrapper must
        // route it to the decider and stamp the recorded envelope decider-tier.
        let coord = ScriptedCoordinator::new(vec![vec![proposal(Action::DeclareConverged)]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());

        let decisions = orch.decide(&ctx());
        assert_eq!(decisions.len(), 1);
        let (action, env) = &decisions[0];
        assert_eq!(action.name(), "declare_converged");
        assert_eq!(env.decision_tier, DecisionTier::Decider);
        assert_eq!(env.model, "stub-decider");
        assert!(env.validate_for(action).is_ok());
    }

    #[test]
    fn decider_may_override_the_proposed_action() {
        // Coordinator proposes DeclareConverged; the decider overrides with
        // Escalate. The recorded action is the decider's.
        let coord = ScriptedCoordinator::new(vec![vec![proposal(Action::DeclareConverged)]]);
        let decider = ScriptedDecider::new(vec![DeciderVerdict {
            action: Action::Escalate {
                reason: "not actually done".into(),
            },
            reason: "intent not met".into(),
            input_artifacts: vec!["intent:1".into()],
        }]);
        let orch = TieredOrchestrator::new(coord, decider);

        let decisions = orch.decide(&ctx());
        let (action, env) = &decisions[0];
        assert_eq!(action.name(), "escalate");
        assert_eq!(env.decision_tier, DecisionTier::Decider);
        assert_eq!(env.reason, "intent not met");
    }

    #[test]
    fn nontrivial_spinoff_is_deferred_but_trivial_stays_coordinator() {
        let coord = ScriptedCoordinator::new(vec![vec![
            proposal(Action::ProposeSpinoff {
                title: "trivial".into(),
                kind: "improvement".into(),
                rationale: "r".into(),
                scope: SpinoffScope::Trivial,
            }),
            proposal(Action::ProposeSpinoff {
                title: "big".into(),
                kind: "refactor".into(),
                rationale: "r".into(),
                scope: SpinoffScope::Substantial,
            }),
        ]]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());

        let decisions = orch.decide(&ctx());
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].1.decision_tier, DecisionTier::Coordinator);
        assert_eq!(decisions[1].1.decision_tier, DecisionTier::Decider);
    }

    #[test]
    fn exhausted_coordinator_script_yields_no_decisions() {
        let coord = ScriptedCoordinator::new(vec![]);
        let orch = TieredOrchestrator::new(coord, ScriptedDecider::confirming());
        assert!(orch.decide(&ctx()).is_empty());
    }
}
