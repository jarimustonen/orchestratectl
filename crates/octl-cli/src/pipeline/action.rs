//! The typed action primitives the orchestrator returns, and the
//! routine/consequential classification that decides which tier is allowed to
//! emit each one (design.md §2 primitive list, §0.2 tiering, §8 finding→action
//! table).
//!
//! The orchestrator never speaks natural language back to the supervisor — every
//! decision is one of these discrete, serde-serializable [`Action`]s. The
//! supervisor validates and would-execute each, and records a
//! [`DecisionEnvelope`](crate::pipeline::DecisionEnvelope) alongside it.

use octl_core::plan::Tier;
use serde::{Deserialize, Serialize};

/// A discrete decision the orchestrator hands back to the supervisor (design §2).
///
/// This is the *entire* orchestrator→supervisor vocabulary: no prose, no partial
/// state, no long-running driver. Each variant is a primitive the supervisor
/// knows how to validate, would-execute, and record. Serde-serializable because
/// every emitted action is persisted as run provenance (design §2 "recorded as
/// structured envelopes").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Re-brief and re-run a chunk against verify findings (design §8 `FIX` /
    /// `FIX_WITH_CARE`). The re-coded chunk MUST be re-verified before it can be
    /// accepted — the driver marks it [`ChunkStatus::NeedsReverify`].
    ///
    /// [`ChunkStatus::NeedsReverify`]: crate::pipeline::ChunkStatus::NeedsReverify
    ReCodeChunk {
        /// The chunk to re-code.
        chunk_id: String,
        /// The findings to fold into the re-brief.
        findings: Vec<Finding>,
    },
    /// Spec is flawed against intent: write a new plan revision, then re-code the
    /// affected chunks (design §8 SPEC-FLAW). **Consequential** — a new plan
    /// revision is a final architectural judgment.
    TriggerReSpec {
        /// Why the current spec cannot converge to intent.
        reason: String,
        /// Chunks the re-spec is expected to revert to `Pending`.
        chunk_ids: Vec<String>,
    },
    /// Accept a chunk as done — floor green and verify satisfied (design §7).
    AcceptChunk {
        /// The chunk to accept.
        chunk_id: String,
    },
    /// Promote a chunk to a higher model tier on repeat-fail or verify
    /// self-disagreement (design §3 adaptive promotion, §8). Routine: promotion is
    /// a mechanical response to a stuck chunk, not a final judgment.
    PromoteTier {
        /// The chunk to promote.
        chunk_id: String,
        /// The tier to run the next attempt at.
        tier: Tier,
    },
    /// Open a discussion that bubbles UP toward the front-end (design §8 DISCUSS,
    /// §12 single human locus). Routine: opening the discussion is coordination;
    /// whether it reaches a human is a separate escalation flow.
    OpenDiscussion {
        /// What the discussion is about.
        topic: String,
        /// How urgent it is.
        severity: Severity,
    },
    /// Propose a deferred, non-blocking spin-off (design §8 `SPIN_OFF`). A
    /// [`Substantial`](SpinoffScope::Substantial) spin-off is **consequential**
    /// (deferring real work is a final judgment); a
    /// [`Trivial`](SpinoffScope::Trivial) one is routine — the boundary is the
    /// explicit `scope`, mirroring design §2's "non-trivial `DROP`/`PROPOSE_SPINOFF`".
    ProposeSpinoff {
        /// Proposed issue title.
        title: String,
        /// Proposed issue kind (e.g. `bugfix`, `improvement`).
        kind: String,
        /// Why this is worth a spin-off rather than blocking the feature.
        rationale: String,
        /// Whether deferring this is trivial (routine) or substantial
        /// (consequential). This is a dedicated classification of the *deferred
        /// work*, deliberately distinct from the finding's [`Severity`] — a
        /// low-severity finding can still be substantial to defer, and vice versa.
        scope: SpinoffScope,
    },
    /// The feature converged: no must-fix left AND product matches intent (design
    /// §2, §6 `DECLARE_CONVERGED`). **Consequential** — this is the ship decision.
    DeclareConverged,
    /// Hand control up to the front-end/human, or abort (design §2 `ESCALATE`, §9
    /// circuit-breakers). **Consequential** — a final "I cannot proceed."
    Escalate {
        /// Why the loop is escalating.
        reason: String,
    },
}

impl Action {
    /// A stable, human-readable discriminant name for logs, envelopes, and
    /// executor routing.
    ///
    /// These MUST match the serde `#[serde(tag = "type", rename_all =
    /// "snake_case")]` variant names — the `name_matches_serde_tag` test guards
    /// the two against drift.
    pub fn name(&self) -> &'static str {
        match self {
            Action::ReCodeChunk { .. } => "re_code_chunk",
            Action::TriggerReSpec { .. } => "trigger_re_spec",
            Action::AcceptChunk { .. } => "accept_chunk",
            Action::PromoteTier { .. } => "promote_tier",
            Action::OpenDiscussion { .. } => "open_discussion",
            Action::ProposeSpinoff { .. } => "propose_spinoff",
            Action::DeclareConverged => "declare_converged",
            Action::Escalate { .. } => "escalate",
        }
    }

    /// The chunk ids this action references and that must exist in the plan for it
    /// to be applicable. The driver checks these as a precondition **before**
    /// would-executing the action, so an action naming an unknown chunk is
    /// rejected without a side effect (rather than executed and then noticed).
    pub fn referenced_chunks(&self) -> Vec<&str> {
        match self {
            Action::ReCodeChunk { chunk_id, .. }
            | Action::AcceptChunk { chunk_id }
            | Action::PromoteTier { chunk_id, .. } => vec![chunk_id.as_str()],
            Action::TriggerReSpec { chunk_ids, .. } => {
                chunk_ids.iter().map(String::as_str).collect()
            }
            Action::OpenDiscussion { .. }
            | Action::ProposeSpinoff { .. }
            | Action::DeclareConverged
            | Action::Escalate { .. } => Vec::new(),
        }
    }

    /// Whether this primitive is a routine coordination step (a fast coordinator
    /// may emit it) or a final/consequential judgment (only the expensive decider
    /// may emit it). This is the encoded classification table (design §0.2) — the
    /// split is explicit here so the tier invariant is testable rather than
    /// buried in prose.
    ///
    /// Consequential (design §2): `DeclareConverged`, `TriggerReSpec`, `Escalate`,
    /// and a [`Substantial`](SpinoffScope::Substantial) `ProposeSpinoff`.
    /// Everything else is routine.
    ///
    /// Note the boundary is *syntactic* — it reads a field the coordinator itself
    /// supplied (`scope`). Per design §2 this is deliberate: the coordinator
    /// classifies, and the mitigation for a mislabel is the auditable
    /// [`decision_tier`](crate::pipeline::DecisionEnvelope::decision_tier) plus the
    /// deterministic floor gating the merge — not preventing the model from
    /// choosing.
    pub fn decision_class(&self) -> DecisionClass {
        match self {
            Action::DeclareConverged | Action::TriggerReSpec { .. } | Action::Escalate { .. } => {
                DecisionClass::Consequential
            }
            Action::ProposeSpinoff { scope, .. } => match scope {
                SpinoffScope::Trivial => DecisionClass::Routine,
                SpinoffScope::Substantial => DecisionClass::Consequential,
            },
            Action::ReCodeChunk { .. }
            | Action::AcceptChunk { .. }
            | Action::PromoteTier { .. }
            | Action::OpenDiscussion { .. } => DecisionClass::Routine,
        }
    }
}

/// Which tier is permitted to make a decision (design §0.2). The audit invariant
/// ties this to [`DecisionTier`](crate::pipeline::DecisionTier): a
/// [`Consequential`](DecisionClass::Consequential) action stamped
/// [`coordinator`](crate::pipeline::DecisionTier::Coordinator) is a violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionClass {
    /// Obvious mechanical coordination the fast coordinator may emit directly.
    Routine,
    /// A final/consequential judgment that must be made by the expensive decider.
    Consequential,
}

/// How much work a [`Action::ProposeSpinoff`] defers — the explicit signal that
/// decides whether the proposal is routine or consequential (design §2
/// "non-trivial `DROP`/`PROPOSE_SPINOFF`"). Deliberately separate from
/// [`Severity`]: triviality is a judgment about the *deferred work*, not about the
/// severity of the finding that motivated it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpinoffScope {
    /// Deferring this is inconsequential — a fast-tier call is fine.
    Trivial,
    /// Deferring this is real work — the expensive decider must own the call.
    Substantial,
}

/// Severity of a verify finding / discussion. A description of *impact*, distinct
/// from the [`SpinoffScope`] classification that governs tiering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Cosmetic / trivial.
    Low,
    /// Meaningful but not blocking.
    Medium,
    /// Serious.
    High,
}

/// One verify finding the orchestrator triages into an [`Action`] (design §8).
/// Carried in [`Action::ReCodeChunk`] and in a
/// [`DecisionTrigger::VerifyReport`](crate::pipeline::DecisionTrigger::VerifyReport)
/// so the decision record shows what evidence drove the call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable id within the verify report (the join key for the audit trail).
    pub id: String,
    /// Human-readable summary of the finding.
    pub summary: String,
    /// How the finding was triaged.
    pub verdict: FindingVerdict,
    /// How serious the finding is.
    pub severity: Severity,
}

/// The triage verdict for a verify finding (design §8 verdict column). This is
/// the *classification* of a finding; the orchestrator maps it to a concrete
/// [`Action`]. Kept distinct from [`Action`] so a report can record verdicts even
/// for findings the orchestrator ends up dropping.
///
/// Note [`Drop`](FindingVerdict::Drop) has no dedicated [`Action`] variant: per
/// design §8 a dropped finding is "recorded with rationale" as an envelope, not a
/// primitive. Whether a *non-trivial* drop should nonetheless be a consequential,
/// decider-tier primitive (as design §2 implies) is an open contract question —
/// see issue `pipeline-drop-primitive-underspecified`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingVerdict {
    /// Must fix; re-code and re-verify.
    Fix,
    /// The spec is flawed; re-spec.
    SpecFlaw,
    /// Needs discussion.
    Discuss,
    /// Defer as a spin-off.
    SpinOff,
    /// Dropped with rationale (recorded, non-blocking).
    Drop,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn consequential_primitives_are_classified_consequential() {
        assert_eq!(
            Action::DeclareConverged.decision_class(),
            DecisionClass::Consequential
        );
        assert_eq!(
            Action::TriggerReSpec {
                reason: "spec flaw".into(),
                chunk_ids: vec!["c1".into()],
            }
            .decision_class(),
            DecisionClass::Consequential
        );
        assert_eq!(
            Action::Escalate {
                reason: "stuck".into(),
            }
            .decision_class(),
            DecisionClass::Consequential
        );
    }

    #[test]
    fn routine_primitives_are_classified_routine() {
        assert_eq!(
            Action::ReCodeChunk {
                chunk_id: "c1".into(),
                findings: vec![],
            }
            .decision_class(),
            DecisionClass::Routine
        );
        assert_eq!(
            Action::AcceptChunk {
                chunk_id: "c1".into(),
            }
            .decision_class(),
            DecisionClass::Routine
        );
        assert_eq!(
            Action::PromoteTier {
                chunk_id: "c1".into(),
                tier: Tier::High,
            }
            .decision_class(),
            DecisionClass::Routine
        );
        assert_eq!(
            Action::OpenDiscussion {
                topic: "naming".into(),
                severity: Severity::High,
            }
            .decision_class(),
            DecisionClass::Routine
        );
    }

    #[test]
    fn spinoff_class_follows_scope_not_severity() {
        // Design §2: a *substantial* PROPOSE_SPINOFF is consequential; a trivial
        // one is routine. The explicit `scope` — not the finding severity — is the
        // encoded boundary.
        let trivial = Action::ProposeSpinoff {
            title: "tidy docs".into(),
            kind: "improvement".into(),
            rationale: "nice-to-have".into(),
            scope: SpinoffScope::Trivial,
        };
        assert_eq!(trivial.decision_class(), DecisionClass::Routine);

        let substantial = Action::ProposeSpinoff {
            title: "extract module".into(),
            kind: "refactor".into(),
            rationale: "real work".into(),
            scope: SpinoffScope::Substantial,
        };
        assert_eq!(substantial.decision_class(), DecisionClass::Consequential);
    }

    #[test]
    fn referenced_chunks_are_reported_for_chunk_actions() {
        assert_eq!(
            Action::AcceptChunk {
                chunk_id: "c1".into()
            }
            .referenced_chunks(),
            vec!["c1"]
        );
        assert_eq!(
            Action::TriggerReSpec {
                reason: "r".into(),
                chunk_ids: vec!["c1".into(), "c2".into()],
            }
            .referenced_chunks(),
            vec!["c1", "c2"]
        );
        assert!(Action::DeclareConverged.referenced_chunks().is_empty());
    }

    #[test]
    fn action_round_trips_through_serde_tagged() {
        let action = Action::ReCodeChunk {
            chunk_id: "c1".into(),
            findings: vec![Finding {
                id: "f1".into(),
                summary: "off-by-one".into(),
                verdict: FindingVerdict::Fix,
                severity: Severity::High,
            }],
        };
        let v = serde_json::to_value(&action).unwrap();
        assert_eq!(v["type"], json!("re_code_chunk"));
        let back: Action = serde_json::from_value(v).unwrap();
        assert_eq!(back, action);
    }

    #[test]
    fn name_matches_serde_tag() {
        // `Action::name()` is hand-maintained; assert every variant's name equals
        // the serde `type` tag so the two can never drift (a rename that misses
        // one would desync envelopes from serialized actions).
        let samples = [
            Action::ReCodeChunk {
                chunk_id: "c".into(),
                findings: vec![],
            },
            Action::TriggerReSpec {
                reason: "r".into(),
                chunk_ids: vec![],
            },
            Action::AcceptChunk {
                chunk_id: "c".into(),
            },
            Action::PromoteTier {
                chunk_id: "c".into(),
                tier: Tier::Mid,
            },
            Action::OpenDiscussion {
                topic: "t".into(),
                severity: Severity::Low,
            },
            Action::ProposeSpinoff {
                title: "t".into(),
                kind: "k".into(),
                rationale: "r".into(),
                scope: SpinoffScope::Trivial,
            },
            Action::DeclareConverged,
            Action::Escalate { reason: "r".into() },
        ];
        for action in samples {
            let tag = serde_json::to_value(&action).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_string();
            assert_eq!(action.name(), tag, "name()/serde tag drift for {action:?}");
        }
    }
}
