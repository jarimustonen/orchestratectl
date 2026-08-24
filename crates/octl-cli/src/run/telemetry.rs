//! Compact observational telemetry views for `run show` and `run list`.
//!
//! These helpers only classify stored advisory samples. They return DTOs and
//! bounded counts; lifecycle, wait, attention, outcome, merge, and cleanup code
//! do not import this module.

use std::fmt::Write as _;

use octl_core::{RunPaths, TelemetrySampleStatus, TelemetryView};
use serde::Serialize;

/// CLI Phase 1 row. `requirement` and `support` intentionally remain absent
/// until agent-profile selection records a candidate that can derive them.
#[derive(Debug, Serialize)]
pub struct NodeTelemetryView {
    pub node_id: String,
    #[serde(flatten)]
    pub sample: TelemetryView,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TelemetryCounts {
    pub absent: u64,
    pub current: u64,
    pub stale: u64,
    pub clock_unreliable: u64,
    pub invalid: u64,
}

impl TelemetryCounts {
    fn observe(&mut self, status: TelemetrySampleStatus) {
        match status {
            TelemetrySampleStatus::Absent => self.absent += 1,
            TelemetrySampleStatus::Current => self.current += 1,
            TelemetrySampleStatus::Stale => self.stale += 1,
            TelemetrySampleStatus::ClockUnreliable => self.clock_unreliable += 1,
            TelemetrySampleStatus::Invalid => self.invalid += 1,
        }
    }
}

/// Read one compact telemetry row per projected node through octl-core's
/// one-lock bulk snapshot. Advisory corruption is localized to `invalid` rows.
pub fn read_views(paths: &RunPaths) -> (Vec<NodeTelemetryView>, Option<String>) {
    match octl_core::read_all_telemetry(paths) {
        Ok(rows) => (
            rows.into_iter()
                .map(|(node_id, sample)| NodeTelemetryView {
                    node_id: node_id.to_string(),
                    sample,
                })
                .collect(),
            None,
        ),
        Err(error) => (Vec::new(), Some(error.to_string())),
    }
}

/// Compute only the five bounded `run list` counts. The bulk core API owns the
/// one-lock node/sample snapshot; scan failures remain separately visible.
pub fn read_counts(paths: &RunPaths) -> (TelemetryCounts, Option<String>) {
    match octl_core::read_all_telemetry(paths) {
        Ok(rows) => {
            let mut counts = TelemetryCounts::default();
            for (_, sample) in rows {
                counts.observe(sample.sample);
            }
            (counts, None)
        }
        Err(error) => (TelemetryCounts::default(), Some(error.to_string())),
    }
}

pub fn counts(views: &[NodeTelemetryView]) -> TelemetryCounts {
    let mut counts = TelemetryCounts::default();
    for view in views {
        counts.observe(view.sample.sample);
    }
    counts
}

pub fn text_line(view: &NodeTelemetryView) -> String {
    let mut line = format!(
        "{}: telemetry {}",
        view.node_id,
        sample_name(view.sample.sample)
    );
    if let Some(state) = view.sample.state {
        write!(line, "; last told activity: {}", state_name(state))
            .expect("writing to String cannot fail");
        if let Some(age_ms) = view.sample.age_ms {
            write!(line, " {} ago", duration(age_ms)).expect("writing to String cannot fail");
        } else {
            line.push_str(" (age unavailable: clock unreliable)");
        }
        if let Some(elapsed_ms) = view.sample.state_elapsed_ms {
            write!(line, "; state reported for {}", duration(elapsed_ms))
                .expect("writing to String cannot fail");
        }
        if let Some(attempt) = view.sample.attempt {
            write!(line, "; attempt {attempt}").expect("writing to String cannot fail");
        }
        if let Some(count) = view.sample.active_tool_count {
            write!(line, "; active tools {count}").expect("writing to String cannot fail");
        }
        if let Some(tool) = &view.sample.tool_name {
            write!(line, " ({tool})").expect("writing to String cannot fail");
        }
    }
    line.push_str("; run status unchanged");
    line
}

fn sample_name(sample: TelemetrySampleStatus) -> &'static str {
    match sample {
        TelemetrySampleStatus::Absent => "absent",
        TelemetrySampleStatus::Current => "current",
        TelemetrySampleStatus::Stale => "stale",
        TelemetrySampleStatus::ClockUnreliable => "clock_unreliable",
        TelemetrySampleStatus::Invalid => "invalid",
    }
}

fn state_name(state: octl_core::TelemetryState) -> &'static str {
    match state {
        octl_core::TelemetryState::AgentActive => "agent_active",
        octl_core::TelemetryState::ToolRunning => "tool_running",
        octl_core::TelemetryState::Settled => "settled",
        octl_core::TelemetryState::Shutdown => "shutdown",
    }
}

fn duration(milliseconds: i64) -> String {
    let seconds = milliseconds.max(0) / 1_000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m{}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h{}m", seconds / 3_600, (seconds % 3_600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare(sample: TelemetrySampleStatus) -> NodeTelemetryView {
        NodeTelemetryView {
            node_id: "n-0001".to_string(),
            sample: TelemetryView {
                sample,
                state: None,
                age_ms: None,
                state_elapsed_ms: None,
                attempt: None,
                active_tool_count: None,
                tool_name: None,
            },
        }
    }

    #[test]
    fn counts_are_bounded_by_sample_state() {
        let rows = [
            TelemetrySampleStatus::Absent,
            TelemetrySampleStatus::Current,
            TelemetrySampleStatus::Stale,
            TelemetrySampleStatus::ClockUnreliable,
            TelemetrySampleStatus::Invalid,
        ]
        .into_iter()
        .map(bare)
        .collect::<Vec<_>>();
        assert_eq!(
            counts(&rows),
            TelemetryCounts {
                absent: 1,
                current: 1,
                stale: 1,
                clock_unreliable: 1,
                invalid: 1,
            }
        );
    }

    #[test]
    fn telemetry_row_pins_bounded_wire_shape() {
        let row = NodeTelemetryView {
            node_id: "n-0001".to_string(),
            sample: TelemetryView {
                sample: TelemetrySampleStatus::Current,
                state: Some(octl_core::TelemetryState::ToolRunning),
                age_ms: Some(12_200),
                state_elapsed_ms: Some(481_000),
                attempt: Some(2),
                active_tool_count: Some(1),
                tool_name: Some("bash".to_string()),
            },
        };
        assert_eq!(
            serde_json::to_value(row).unwrap(),
            serde_json::json!({
                "node_id": "n-0001", "sample": "current", "state": "tool_running",
                "age_ms": 12_200, "state_elapsed_ms": 481_000, "attempt": 2,
                "active_tool_count": 1, "tool_name": "bash"
            })
        );
    }

    #[test]
    fn every_text_state_is_observational_and_duration_boundaries_are_stable() {
        for sample in [
            TelemetrySampleStatus::Absent,
            TelemetrySampleStatus::Current,
            TelemetrySampleStatus::Stale,
            TelemetrySampleStatus::ClockUnreliable,
            TelemetrySampleStatus::Invalid,
        ] {
            let line = text_line(&bare(sample));
            assert!(line.contains("run status unchanged"));
            for forbidden in [
                "healthy", "progress", "success", "failure", "wedged", "stuck",
            ] {
                assert!(!line.contains(forbidden), "{sample:?}: {line}");
            }
        }
        for (milliseconds, expected) in [
            (-1, "0s"),
            (59_999, "59s"),
            (60_000, "1m0s"),
            (3_599_999, "59m59s"),
            (3_600_000, "1h0m"),
        ] {
            assert_eq!(duration(milliseconds), expected);
        }
    }
}
