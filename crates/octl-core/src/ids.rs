//! Identifier generators and types per design.md §1.1.

use ulid::Ulid;

/// Generate a new lowercase-ULID `run_id`.
pub fn new_run_id() -> String {
    Ulid::new().to_string().to_lowercase()
}

/// Format a monotonic per-run `node_id` (e.g. `n-0001`).
pub fn format_node_id(n: u32) -> String {
    format!("n-{n:04}")
}
