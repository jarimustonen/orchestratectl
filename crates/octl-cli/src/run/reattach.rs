//! `run reattach` — record the reattach request so the supervisor (once
//! it lands in `supervisor-process`) can replay from a known anchor.
//!
//! MVP scope per the issue: the supervisor doesn't exist yet, so this
//! command writes a `supervisor.reattach-requested` event into the run's
//! `events.jsonl` and exits 0. The reducer ignores unknown event kinds
//! (see `octl_core::reducer::apply_event`), so projections are untouched
//! until the supervisor catches up later.

use serde::Serialize;
use serde_json::Value;

use octl_core::{append_and_apply, read_manifest_opt};

use crate::error::CliError;
use crate::output;
use crate::run::{from_core, run_paths};

#[derive(Serialize)]
struct ReattachPayload<'a> {
    run_id: &'a str,
    action: &'static str,
    note: &'static str,
}

pub fn run(run_id: &str, json: bool, warnings: &[String]) -> Result<(), CliError> {
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, run_id);
    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run-not-found", format!("no run with id {run_id}"))
                .with_invalid_value(run_id),
        );
    }
    append_and_apply(
        &paths,
        "supervisor.reattach-requested",
        None,
        None,
        Value::Object(Default::default()),
    )
    .map_err(from_core)?;
    let payload = ReattachPayload {
        run_id,
        action: "reattach-requested",
        note: "supervisor not yet implemented; event recorded for replay",
    };
    if json {
        output::emit_json(&payload, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        println!("reattach requested for run {} (event recorded)", run_id);
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}
