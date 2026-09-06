//! Read surfaces for a durable, non-terminal human-decision request.
//!
//! A worker opens this state explicitly with `node.awaiting_input`; no pane,
//! stdin, activity, or liveness inference is involved. The event timestamp is
//! projected onto [`taskfleet_core::AwaitingInput`] and anchors a fixed grace window,
//! so a supervisor restart cannot restart the clock. Before the grace expires
//! the request is visible but does not settle `run wait` or page the parent.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use taskfleet_core::AwaitingInput;

/// Default delay before an unresolved question propagates to the parent.
pub const GRACE_SECS: i64 = 180;
/// Operational override for the propagation grace. Whole seconds; invalid or
/// negative values fall back to [`GRACE_SECS`].
pub const GRACE_ENV: &str = "TASKFLEET_AWAITING_INPUT_GRACE_SECS";
/// Stable machine reason emitted when an escalated request settles `run wait`.
pub const AWAITING_INPUT_REASON: &str = "worker is awaiting a human decision";

/// Effective propagation grace.
#[must_use]
pub fn grace() -> Duration {
    let secs = std::env::var(GRACE_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|v| *v >= 0)
        .unwrap_or(GRACE_SECS);
    Duration::seconds(secs)
}

/// True once the durable open timestamp is strictly older than the grace.
/// Strict `>` prevents paging exactly on the boundary.
#[must_use]
pub fn is_escalated(opened_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now.signed_duration_since(opened_at) > grace()
}

/// JSON/human context shared by `run show`, `run list`, and `run wait`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AwaitingInputView {
    /// Generation fence required by `node.input_resolved`.
    pub event_seq: u64,
    pub opened_at: DateTime<Utc>,
    pub pending_age_secs: i64,
    pub escalated: bool,
    pub open_discussion_count: usize,
    pub discussion_items: Vec<serde_json::Value>,
}

impl AwaitingInputView {
    #[must_use]
    pub fn build(open: &AwaitingInput, now: DateTime<Utc>) -> Self {
        Self {
            event_seq: open.event_seq,
            opened_at: open.opened_at,
            pending_age_secs: now
                .signed_duration_since(open.opened_at)
                .num_seconds()
                .max(0),
            escalated: is_escalated(open.opened_at, now),
            open_discussion_count: open.discussion_items.len(),
            discussion_items: open.discussion_items.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn grace_boundary_is_strict() {
        let opened: DateTime<Utc> = "2026-08-16T12:00:00Z".parse().unwrap();
        let boundary = opened + Duration::seconds(GRACE_SECS);
        // Tests do not mutate the process-global override, so parallel tests
        // cannot race this boundary assertion.
        assert!(!is_escalated(opened, boundary));
        assert!(is_escalated(opened, boundary + Duration::seconds(1)));
    }

    #[test]
    fn view_preserves_discussion_and_clamps_clock_skew() {
        let opened: DateTime<Utc> = "2026-08-16T12:00:00Z".parse().unwrap();
        let open = AwaitingInput {
            opened_at: opened,
            event_seq: 7,
            discussion_items: vec![json!({
                "topic": "Which API?",
                "options": ["small", "large"],
                "recommended_default": "small"
            })],
        };
        let view = AwaitingInputView::build(&open, opened - Duration::seconds(1));
        assert_eq!(view.pending_age_secs, 0);
        assert_eq!(view.open_discussion_count, 1);
        assert_eq!(view.discussion_items, open.discussion_items);
    }
}
