//! Inverted control loop — the code-pipeline's architectural core (design.md §2
//! + §0.2, breakdown T4).
//!
//! # The inversion (design §2)
//!
//! The panel's load-bearing correction: the non-LLM **supervisor owns the event
//! loop**, and the **orchestrator is a stateless pure function** invoked *per
//! decision point*. It never runs as a long-lived LLM driver (which would
//! exhaust context and hallucinate state transitions) and never speaks natural
//! language back to the loop — it returns **discrete, typed action primitives**
//! ([`Action`]). The supervisor validates each primitive, would-execute it, and
//! records a structured [`DecisionEnvelope`] so a run is causally replayable.
//!
//! # The tiering (design §0.2)
//!
//! The orchestrator function is itself **tiered**. A fast, cheap *coordinator*
//! emits the obvious mechanical primitives (a clear `RE_CODE_CHUNK`, dispatch,
//! progress) and classifies each decision [`Routine`](DecisionClass::Routine) vs
//! [`Consequential`](DecisionClass::Consequential). Every *final/consequential*
//! primitive — `DECLARE_CONVERGED`, `TRIGGER_RE_SPEC`, `ESCALATE`, a non-trivial
//! `PROPOSE_SPINOFF` — is deferred to an expensive *decider* (Opus) whose verdict
//! is the one recorded. The classification boundary is the one genuinely new risk
//! this refinement adds (a fast model mislabelling a consequential decision as
//! routine), so [`DecisionEnvelope::decision_tier`] makes every such call
//! auditable: **a consequential action stamped `coordinator` is an invariant
//! violation** the driver flags ([`DecisionEnvelope::validate_for`]).
//!
//! # Layout
//!
//! - [`action`] — the typed [`Action`] primitives + their
//!   [`routine/consequential`](DecisionClass) classification.
//! - [`envelope`] — the [`DecisionEnvelope`] audit record + the tier invariant.
//! - [`orchestrator`] — the [`Orchestrator`] trait, the [`TieredOrchestrator`]
//!   wrapper, and deterministic scripted [`Coordinator`]/[`Decider`] stubs.
//! - [`driver`] — [`drive`], a pure in-memory state machine modelling the
//!   supervisor side of the loop.
//!
//! **This module is behind the seam and not wired into any live path.** Nothing
//! in `run create` / the live supervisor constructs an [`Orchestrator`] or calls
//! [`drive`] yet; staged rollout (design §14) plugs it in at T5, which replaces
//! the in-memory state machine's execution stub with the real event log
//! (`LockedRun` + `append_and_apply`, state-integrity invariant 1) and the real
//! git/merge/spawn actions. It lands as unused-by-default scaffolding + tests;
//! the `mod pipeline;` declaration carries `#[allow(dead_code)]` for that reason.

pub mod action;
pub mod driver;
pub mod envelope;
pub mod orchestrator;

pub use action::{Action, DecisionClass, Finding, FindingVerdict, Severity, SpinoffScope};
pub use driver::{
    drive, ActionExecutor, ChunkState, ChunkStatus, DecisionOutcome, DecisionRecord,
    DiscussionRecord, ExecError, LoopStatus, PipelineState, RecordingExecutor, SpinoffRecord,
};
pub use envelope::{DecisionEnvelope, DecisionTier, TierViolation};
pub use orchestrator::{
    Coordinator, CoordinatorProposal, Decider, DeciderVerdict, DecisionContext, DecisionTrigger,
    Orchestrator, ScriptedCoordinator, ScriptedDecider, TieredOrchestrator,
};
