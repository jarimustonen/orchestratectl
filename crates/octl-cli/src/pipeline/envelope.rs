//! The decision envelope — the structured audit record stamped on every
//! orchestrator decision (design.md §2), and the tier invariant that makes the
//! coordinator/decider split auditable (design §0.2).
//!
//! A run is causally replayable because every [`Action`] the orchestrator
//! emits is recorded not as prose but as a [`DecisionEnvelope`]: who decided,
//! from what inputs, why, **at which tier**, and with which model + prompt
//! version. The one genuinely new risk the tiering adds is a fast coordinator
//! mislabelling a consequential decision as routine — so
//! [`DecisionEnvelope::validate_for`] turns "a consequential action stamped
//! `coordinator`" into a catchable invariant violation.

use serde::{Deserialize, Serialize};

use super::action::{Action, DecisionClass};

/// Which model tier actually made a decision (design §0.2). Stamped on every
/// [`DecisionEnvelope`]; the invariant is that a
/// [`Consequential`](DecisionClass::Consequential) action must carry
/// [`Decider`](DecisionTier::Decider).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionTier {
    /// The fast, cheap coordinator — routine coordination only.
    Coordinator,
    /// The expensive decider (Opus) — the recorded authority for final /
    /// consequential judgments.
    Decider,
}

/// The structured audit record for one orchestrator decision (design §2:
/// "Decisions are recorded as structured envelopes (actor, input artifact IDs,
/// reason summary, decision tier, model + prompt version), not prose").
///
/// One envelope accompanies one [`Action`]. It is serde-serializable because it
/// is persisted as run provenance; T5 appends it to the event log through the
/// `LockedRun` witness (state-integrity invariant 1), never by writing a
/// projection directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionEnvelope {
    /// The role that made the call (e.g. `coordinator`, `decider`). Human-facing
    /// label; [`decision_tier`](DecisionEnvelope::decision_tier) is the machine
    /// field the invariant checks.
    pub actor: String,
    /// Ids of the artifacts this decision consumed (verify report id, `plan_rev`,
    /// `intent_rev`, chunk ids) — the causal inputs, so the decision is replayable.
    pub input_artifacts: Vec<String>,
    /// A short reason summary. Not prose driving the loop — a human-readable note
    /// on an otherwise machine-typed record.
    pub reason: String,
    /// The tier that decided. The audit field: a
    /// [`Consequential`](DecisionClass::Consequential) action stamped
    /// [`Coordinator`](DecisionTier::Coordinator) is an invariant violation.
    pub decision_tier: DecisionTier,
    /// The concrete model that produced the decision (e.g. the fast coordinator
    /// model, or Opus).
    pub model: String,
    /// The prompt/contract version the tier ran under (provenance; design §7
    /// "prompt version recorded on every attempt").
    pub prompt_version: String,
}

impl DecisionEnvelope {
    /// Check the tier invariant for `action` (design §0.2): a
    /// [`Consequential`](DecisionClass::Consequential) action MUST be stamped
    /// [`Decider`](DecisionTier::Decider). A consequential action stamped
    /// [`Coordinator`](DecisionTier::Coordinator) is an audit-catchable bug — the
    /// fast tier emitted a final decision it was not allowed to make.
    ///
    /// A routine action may be stamped either tier (the decider is free to handle
    /// anything; the constraint is only one-directional).
    ///
    /// # Errors
    ///
    /// Returns a [`TierViolation`] when `action` is consequential but this
    /// envelope is coordinator-tier.
    pub fn validate_for(&self, action: &Action) -> Result<(), TierViolation> {
        if action.decision_class() == DecisionClass::Consequential
            && self.decision_tier == DecisionTier::Coordinator
        {
            return Err(TierViolation {
                action: action.name().to_string(),
                actor: self.actor.clone(),
                reason: self.reason.clone(),
            });
        }
        Ok(())
    }
}

/// A recorded tier-invariant breach: a consequential [`Action`] carried a
/// coordinator-tier [`DecisionEnvelope`] (design §0.2). The driver flags these
/// rather than executing the mis-tiered decision, so the run's audit trail names
/// exactly which fast-tier call over-reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("tier violation: consequential action `{action}` was stamped coordinator-tier (actor `{actor}`, reason: {reason})")]
pub struct TierViolation {
    /// The consequential action's [`Action::name`].
    pub action: String,
    /// The envelope's [`DecisionEnvelope::actor`].
    pub actor: String,
    /// The envelope's [`DecisionEnvelope::reason`], for the audit note.
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::action::SpinoffScope;

    fn envelope(tier: DecisionTier) -> DecisionEnvelope {
        DecisionEnvelope {
            actor: match tier {
                DecisionTier::Coordinator => "coordinator".into(),
                DecisionTier::Decider => "decider".into(),
            },
            input_artifacts: vec!["verify:v1".into()],
            reason: "test".into(),
            decision_tier: tier,
            model: "test-model".into(),
            prompt_version: "v1".into(),
        }
    }

    #[test]
    fn consequential_on_coordinator_is_a_violation() {
        let env = envelope(DecisionTier::Coordinator);
        let err = env.validate_for(&Action::DeclareConverged).unwrap_err();
        assert_eq!(err.action, "declare_converged");
        assert_eq!(err.actor, "coordinator");
    }

    #[test]
    fn consequential_on_decider_is_ok() {
        let env = envelope(DecisionTier::Decider);
        assert!(env.validate_for(&Action::DeclareConverged).is_ok());
    }

    #[test]
    fn routine_on_either_tier_is_ok() {
        let action = Action::AcceptChunk {
            chunk_id: "c1".into(),
        };
        assert!(envelope(DecisionTier::Coordinator)
            .validate_for(&action)
            .is_ok());
        assert!(envelope(DecisionTier::Decider)
            .validate_for(&action)
            .is_ok());
    }

    #[test]
    fn trivial_spinoff_on_coordinator_is_ok_but_nontrivial_is_not() {
        let trivial = Action::ProposeSpinoff {
            title: "t".into(),
            kind: "improvement".into(),
            rationale: "r".into(),
            scope: SpinoffScope::Trivial,
        };
        assert!(envelope(DecisionTier::Coordinator)
            .validate_for(&trivial)
            .is_ok());

        let nontrivial = Action::ProposeSpinoff {
            title: "t".into(),
            kind: "refactor".into(),
            rationale: "r".into(),
            scope: SpinoffScope::Substantial,
        };
        assert!(envelope(DecisionTier::Coordinator)
            .validate_for(&nontrivial)
            .is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = envelope(DecisionTier::Decider);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["decision_tier"], serde_json::json!("decider"));
        let back: DecisionEnvelope = serde_json::from_value(v).unwrap();
        assert_eq!(back, env);
    }
}
