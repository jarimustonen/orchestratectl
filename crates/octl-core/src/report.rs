//! §7.3 terminal-report payload validation (design.md §7.3).
//!
//! Validates the structural shape of a `node.report` payload — `success`
//! required; optional `summary`, `cancelled`/`reason`, `discussion_items`,
//! `spinoff_proposals`, `wrap_up_recommendations` — before the reducer
//! ever projects it. Lives in `octl-core` (not the CLI) so the supervisor
//! can validate child reports with the same rules it would consume
//! (design.md §7.3 step 3), rather than copying the validator or depending
//! on the CLI crate.
//!
//! Errors are domain-typed ([`ReportValidationError`]); the CLI maps them
//! to its `CliError` envelope at the boundary.

use serde_json::Value;

use crate::schema::Kind;

/// A §7.3 report payload failed structural validation.
///
/// Every variant describes one schema violation. The CLI renders these as
/// a `schema_violation` error; [`ReportValidationError::expected`] supplies
/// the machine-readable `expected` hint for the variants that carry one.
#[derive(Debug, thiserror::Error)]
pub enum ReportValidationError {
    /// The payload root was not a JSON object.
    #[error("report payload must be a JSON object")]
    NotObject,

    /// The required `success` field was absent.
    #[error("report payload missing required field `success`")]
    MissingSuccess,

    /// `success` was present but not a boolean.
    #[error("field `success` must be a boolean")]
    SuccessNotBoolean,

    /// `summary` was present but not a string (or null).
    #[error("field `summary` must be a string")]
    SummaryNotString,

    /// `cancelled` was present but not a boolean.
    #[error("field `cancelled` must be a boolean")]
    CancelledNotBoolean,

    /// `reason` was present but not a string.
    #[error("field `reason` must be a string")]
    ReasonNotString,

    /// `cancelled: true` was paired with `success: true` (§7.7 forbids it).
    #[error("`cancelled: true` requires `success: false`")]
    CancelledRequiresSuccessFalse,

    /// `cancelled: true` lacked a non-empty `reason` string (§7.7).
    #[error("`cancelled: true` requires a non-empty `reason` string")]
    CancelledRequiresReason,

    /// `discussion_items` was present but not an array.
    #[error("field `discussion_items` must be an array")]
    DiscussionItemsNotArray,

    /// A `discussion_items` element was not a JSON object.
    #[error("discussion_items[{index}] must be a JSON object")]
    DiscussionItemNotObject {
        /// Index of the offending element.
        index: usize,
    },

    /// A `discussion_items` element lacked a non-empty `topic` string.
    #[error("discussion_items[{index}].topic must be a non-empty string")]
    DiscussionItemTopicMissing {
        /// Index of the offending element.
        index: usize,
    },

    /// A `discussion_items` element's `severity` was not a string.
    #[error("discussion_items[{index}].severity must be a string")]
    DiscussionItemSeverityNotString {
        /// Index of the offending element.
        index: usize,
    },

    /// `spinoff_proposals` was present but not an array.
    #[error("field `spinoff_proposals` must be an array")]
    SpinoffProposalsNotArray,

    /// A `spinoff_proposals` element was not a JSON object.
    #[error("spinoff_proposals[{index}] must be a JSON object")]
    SpinoffProposalNotObject {
        /// Index of the offending element.
        index: usize,
    },

    /// A `spinoff_proposals` element lacked a non-empty `proposed_title`.
    #[error("spinoff_proposals[{index}].proposed_title must be a non-empty string")]
    SpinoffProposalTitleMissing {
        /// Index of the offending element.
        index: usize,
    },

    /// A `spinoff_proposals` element's `proposed_kind` was not a string.
    #[error("spinoff_proposals[{index}].proposed_kind must be a string")]
    SpinoffProposalKindNotString {
        /// Index of the offending element.
        index: usize,
    },

    /// A `spinoff_proposals` element's `proposed_kind` was not a known [`Kind`].
    #[error("spinoff_proposals[{index}].proposed_kind `{kind}` is not a known kind")]
    SpinoffProposalKindUnknown {
        /// Index of the offending element.
        index: usize,
        /// The rejected kind string.
        kind: String,
    },

    /// A `spinoff_proposals` element's `rationale` was not a string (or null).
    #[error("spinoff_proposals[{index}].rationale must be a string")]
    SpinoffProposalRationaleNotString {
        /// Index of the offending element.
        index: usize,
    },

    /// A declared string-array field was not an array.
    #[error("field `{field}` must be an array")]
    FieldNotArray {
        /// The offending field name.
        field: String,
    },

    /// An element of a declared string-array field was not a string.
    #[error("{field}[{index}] must be a string")]
    FieldElementNotString {
        /// The offending field name.
        field: String,
        /// Index of the offending element.
        index: usize,
    },

    /// A nested path expected to hold a string array was not an array.
    #[error("{path} must be an array")]
    PathNotArray {
        /// Dotted/indexed path to the offending value.
        path: String,
    },

    /// An element at a nested string-array path was not a string.
    #[error("{path}[{index}] must be a string")]
    PathElementNotString {
        /// Dotted/indexed path to the offending array.
        path: String,
        /// Index of the offending element.
        index: usize,
    },
}

impl ReportValidationError {
    /// The machine-readable `expected` hint for this error, if any.
    ///
    /// Mirrors the `with_expected(...)` payloads the CLI previously
    /// attached inline, so callers can surface the same structured hint.
    #[must_use]
    pub fn expected(&self) -> Option<Value> {
        match self {
            Self::MissingSuccess | Self::SuccessNotBoolean => {
                Some(serde_json::json!({"field": "success", "type": "boolean"}))
            }
            Self::SpinoffProposalKindUnknown { .. } => Some(serde_json::json!([
                "code",
                "spinoff",
                "orchestrated",
                "research",
                "technical-decision",
                "make-skill",
                "fan-out",
                "bugfix"
            ])),
            _ => None,
        }
    }
}

/// Validate a §7.3 report payload's structural shape.
///
/// Rejects anything obviously not a report before the reducer ever sees
/// it, so the caller can name the offending field instead of bubbling a
/// generic `CorruptEventLog`. Keeps the current validation logic verbatim;
/// this is a relocation, not a tightening.
///
/// # Errors
///
/// Returns a [`ReportValidationError`] describing the first schema
/// violation found.
pub fn validate_report_payload(data: &Value) -> Result<(), ReportValidationError> {
    let obj = data
        .as_object()
        .ok_or(ReportValidationError::NotObject)?;

    // `success` is the one strictly required field per §7.3. A cancel-
    // synthesized report (§7.7) may carry `cancelled: true` AND
    // `success: false` — both are still booleans on the wire.
    let success = obj
        .get("success")
        .ok_or(ReportValidationError::MissingSuccess)?;
    if !success.is_boolean() {
        return Err(ReportValidationError::SuccessNotBoolean);
    }

    if let Some(v) = obj.get("summary") {
        if !v.is_string() && !v.is_null() {
            return Err(ReportValidationError::SummaryNotString);
        }
    }
    let cancelled = match obj.get("cancelled") {
        None | Some(Value::Null) => false,
        Some(v) => v
            .as_bool()
            .ok_or(ReportValidationError::CancelledNotBoolean)?,
    };
    let reason = match obj.get("reason") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_str()
                .ok_or(ReportValidationError::ReasonNotString)?,
        ),
    };

    // §7.7: a cancel-synthesized report carries `cancelled: true,
    // success: false, reason: <non-empty>`. Allowing `success: true`
    // alongside `cancelled: true` would persist a contradiction (the
    // reducer prioritizes `cancelled`, so the node would be cancelled
    // while `last_report.success == true`).
    if cancelled {
        if success.as_bool().unwrap_or(false) {
            return Err(ReportValidationError::CancelledRequiresSuccessFalse);
        }
        match reason {
            Some(s) if !s.trim().is_empty() => {}
            _ => return Err(ReportValidationError::CancelledRequiresReason),
        }
    }

    validate_discussion_items(obj.get("discussion_items"))?;
    validate_spinoff_proposals(obj.get("spinoff_proposals"))?;
    validate_string_array(
        obj.get("wrap_up_recommendations"),
        "wrap_up_recommendations",
    )?;
    Ok(())
}

fn validate_discussion_items(v: Option<&Value>) -> Result<(), ReportValidationError> {
    let arr = match v {
        Some(Value::Array(a)) => a,
        Some(_) => return Err(ReportValidationError::DiscussionItemsNotArray),
        None => return Ok(()),
    };
    for (i, item) in arr.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or(ReportValidationError::DiscussionItemNotObject { index: i })?;
        let topic = obj.get("topic").and_then(Value::as_str);
        if topic.is_none_or(|t| t.trim().is_empty()) {
            return Err(ReportValidationError::DiscussionItemTopicMissing { index: i });
        }
        if let Some(sev) = obj.get("severity") {
            if !sev.is_string() {
                return Err(ReportValidationError::DiscussionItemSeverityNotString { index: i });
            }
            // §7.3 example lists "discuss|critical" but the design
            // calls for forward-compatibility — accept any string and
            // let the supervisor interpret unknown severities. (A
            // CLI-side closed-set check would deadlock agents shipped
            // ahead of a CLI release; see review #2/DeepSeek and #15
            // /Claude.)
        }
        if let Some(opts) = obj.get("options") {
            validate_string_array_at(opts, &format!("discussion_items[{i}].options"))?;
        }
    }
    Ok(())
}

fn validate_spinoff_proposals(v: Option<&Value>) -> Result<(), ReportValidationError> {
    let arr = match v {
        Some(Value::Array(a)) => a,
        Some(_) => return Err(ReportValidationError::SpinoffProposalsNotArray),
        None => return Ok(()),
    };
    for (i, item) in arr.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or(ReportValidationError::SpinoffProposalNotObject { index: i })?;
        let title = obj.get("proposed_title").and_then(Value::as_str);
        if title.is_none_or(|t| t.trim().is_empty()) {
            return Err(ReportValidationError::SpinoffProposalTitleMissing { index: i });
        }
        let kind_str = obj
            .get("proposed_kind")
            .and_then(Value::as_str)
            .ok_or(ReportValidationError::SpinoffProposalKindNotString { index: i })?;
        // Reject unknown kinds at the boundary so the supervisor never
        // has to translate a generic `CorruptEventLog` for the user.
        // Mirrors the `Kind` enum's `rename_all = "kebab-case"` serde
        // routing.
        if serde_json::from_value::<Kind>(Value::String(kind_str.to_string())).is_err() {
            return Err(ReportValidationError::SpinoffProposalKindUnknown {
                index: i,
                kind: kind_str.to_string(),
            });
        }
        if let Some(rationale) = obj.get("rationale") {
            if !rationale.is_string() && !rationale.is_null() {
                return Err(ReportValidationError::SpinoffProposalRationaleNotString { index: i });
            }
        }
    }
    Ok(())
}

/// Path-aware string-array validator. Used for nested fields where the
/// caller wants to embed an index in the error message.
fn validate_string_array_at(v: &Value, path: &str) -> Result<(), ReportValidationError> {
    let arr = v.as_array().ok_or_else(|| ReportValidationError::PathNotArray {
        path: path.to_string(),
    })?;
    for (i, item) in arr.iter().enumerate() {
        if !item.is_string() {
            return Err(ReportValidationError::PathElementNotString {
                path: path.to_string(),
                index: i,
            });
        }
    }
    Ok(())
}

fn validate_string_array(v: Option<&Value>, field: &str) -> Result<(), ReportValidationError> {
    let arr = match v {
        Some(Value::Array(a)) => a,
        Some(_) => {
            return Err(ReportValidationError::FieldNotArray {
                field: field.to_string(),
            })
        }
        None => return Ok(()),
    };
    for (i, item) in arr.iter().enumerate() {
        if !item.is_string() {
            return Err(ReportValidationError::FieldElementNotString {
                field: field.to_string(),
                index: i,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- valid payloads ---

    #[test]
    fn validates_minimal_success_payload() {
        let v = json!({"success": true});
        assert!(validate_report_payload(&v).is_ok());
    }

    #[test]
    fn validates_full_success_payload() {
        let v = json!({
            "success": true,
            "summary": "did the thing",
            "discussion_items": [
                {"topic": "naming", "severity": "discuss", "options": ["a", "b"]},
            ],
            "spinoff_proposals": [
                {"proposed_title": "follow-up", "proposed_kind": "code", "rationale": "later"},
            ],
            "wrap_up_recommendations": ["rebase", "squash"],
        });
        assert!(validate_report_payload(&v).is_ok());
    }

    #[test]
    fn discussion_item_unknown_severity_accepted_for_forward_compat() {
        // Forward-compat: a supervisor may add new severities without a
        // CLI release. The validator only enforces severity is a string.
        let v = json!({
            "success": true,
            "discussion_items": [{"topic": "x", "severity": "info"}],
        });
        assert!(validate_report_payload(&v).is_ok());
    }

    #[test]
    fn cancel_synthesized_report_shape_ok() {
        // Mirror of run cancel's synthesized payload (run/cancel.rs).
        let v = json!({
            "success": false,
            "cancelled": true,
            "reason": "cancelled by user",
            "summary": "Run cancelled before agent reported.",
            "discussion_items": [],
            "spinoff_proposals": [],
            "wrap_up_recommendations": [],
        });
        assert!(validate_report_payload(&v).is_ok());
    }

    // --- invalid payloads ---

    #[test]
    fn non_object_root_rejected() {
        let v = json!([1, 2, 3]);
        assert!(matches!(
            validate_report_payload(&v),
            Err(ReportValidationError::NotObject)
        ));
    }

    #[test]
    fn missing_success_rejected() {
        let v = json!({"summary": "no success field"});
        let err = validate_report_payload(&v).unwrap_err();
        assert!(matches!(err, ReportValidationError::MissingSuccess));
        // Missing `success` carries the structured `expected` hint.
        assert_eq!(
            err.expected(),
            Some(json!({"field": "success", "type": "boolean"}))
        );
    }

    #[test]
    fn non_boolean_success_rejected() {
        let v = json!({"success": "yes"});
        assert!(matches!(
            validate_report_payload(&v),
            Err(ReportValidationError::SuccessNotBoolean)
        ));
    }

    #[test]
    fn discussion_item_missing_topic_rejected() {
        let v = json!({
            "success": true,
            "discussion_items": [{"severity": "discuss"}],
        });
        assert!(matches!(
            validate_report_payload(&v),
            Err(ReportValidationError::DiscussionItemTopicMissing { index: 0 })
        ));
    }

    #[test]
    fn discussion_item_non_string_severity_rejected() {
        let v = json!({
            "success": true,
            "discussion_items": [{"topic": "x", "severity": 42}],
        });
        assert!(matches!(
            validate_report_payload(&v),
            Err(ReportValidationError::DiscussionItemSeverityNotString { index: 0 })
        ));
    }

    #[test]
    fn discussion_item_options_must_be_strings() {
        let v = json!({
            "success": true,
            "discussion_items": [{"topic": "x", "options": [1, 2]}],
        });
        assert!(matches!(
            validate_report_payload(&v),
            Err(ReportValidationError::PathElementNotString { index: 0, .. })
        ));
    }

    #[test]
    fn spinoff_unknown_proposed_kind_rejected() {
        let v = json!({
            "success": true,
            "spinoff_proposals": [{"proposed_title": "x", "proposed_kind": "not-a-kind"}],
        });
        let err = validate_report_payload(&v).unwrap_err();
        assert!(matches!(
            err,
            ReportValidationError::SpinoffProposalKindUnknown { index: 0, .. }
        ));
        // Unknown kind surfaces the closed-set of known kinds.
        assert!(err.expected().is_some());
    }

    #[test]
    fn spinoff_missing_kind_rejected() {
        let v = json!({
            "success": true,
            "spinoff_proposals": [{"proposed_title": "x"}],
        });
        assert!(matches!(
            validate_report_payload(&v),
            Err(ReportValidationError::SpinoffProposalKindNotString { index: 0 })
        ));
    }

    #[test]
    fn cancelled_requires_success_false() {
        let v = json!({"success": true, "cancelled": true, "reason": "x"});
        assert!(matches!(
            validate_report_payload(&v),
            Err(ReportValidationError::CancelledRequiresSuccessFalse)
        ));
    }

    #[test]
    fn cancelled_requires_reason() {
        let v = json!({"success": false, "cancelled": true});
        assert!(matches!(
            validate_report_payload(&v),
            Err(ReportValidationError::CancelledRequiresReason)
        ));
    }

    #[test]
    fn wrap_up_must_be_string_array() {
        let v = json!({
            "success": true,
            "wrap_up_recommendations": ["ok", 42],
        });
        assert!(matches!(
            validate_report_payload(&v),
            Err(ReportValidationError::FieldElementNotString { index: 1, .. })
        ));
    }
}
