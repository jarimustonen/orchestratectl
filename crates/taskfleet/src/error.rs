//! Error envelope shared by every subcommand.
//!
//! Per `AGENTS-AI-FIRST-CLI.md` §10, failures emit a structured object
//! to **stderr**. Exit codes follow §2: 0 success, 1 user/validation,
//! 2 refused-but-actionable (system/IO).

use serde::Serialize;
use taskfleet_core::SCHEMA_VERSION;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum ExitKind {
    User = 1,
    System = 2,
}

#[derive(Debug, Serialize)]
pub struct ErrorPayload {
    pub schema_version: u32,
    pub error: ErrorBody,
    /// Process-cumulative count of log events dropped by the lossy
    /// non-blocking appender (buffer overflow). Mirrors the success
    /// envelope's `dropped_log_events` field — without it, a command
    /// that drops `error!`/`warn!` events and then fails would emit
    /// the error with no signal that logs were lost (the worst case).
    /// Omitted when zero so the field is purely additive
    /// (issue: passably-shaggy-parent; AGENTS-AI-FIRST-CLI §10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped_log_events: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<serde_json::Value>,
    /// Command-specific observed context, distinct from valid replacement
    /// values in `expected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Box<serde_json::Value>>,
}

#[derive(Debug)]
pub struct CliError {
    pub kind: ExitKind,
    pub code: String,
    pub message: String,
    pub invalid_value: Option<String>,
    pub expected: Option<serde_json::Value>,
    pub details: Option<Box<serde_json::Value>>,
}

impl CliError {
    #[allow(dead_code)] // Used by later subcommand issues.
    pub fn user(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: ExitKind::User,
            code: code.into(),
            message: message.into(),
            invalid_value: None,
            expected: None,
            details: None,
        }
    }

    pub fn system(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: ExitKind::System,
            code: code.into(),
            message: message.into(),
            invalid_value: None,
            expected: None,
            details: None,
        }
    }

    #[allow(dead_code)] // Used by later subcommand issues.
    pub fn with_invalid_value(mut self, value: impl Into<String>) -> Self {
        self.invalid_value = Some(value.into());
        self
    }

    #[allow(dead_code)] // Used by later subcommand issues.
    pub fn with_expected(mut self, expected: serde_json::Value) -> Self {
        self.expected = Some(expected);
        self
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(Box::new(details));
        self
    }

    fn payload(&self) -> ErrorPayload {
        let dropped = crate::cli::dropped_log_events();
        ErrorPayload {
            schema_version: SCHEMA_VERSION,
            error: ErrorBody {
                code: self.code.clone(),
                message: self.message.clone(),
                invalid_value: self.invalid_value.clone(),
                expected: self.expected.clone(),
                details: self.details.clone(),
            },
            dropped_log_events: (dropped > 0).then_some(dropped),
        }
    }

    /// Print the error envelope to stderr. Always uses the JSON envelope —
    /// `AGENTS-AI-FIRST-CLI.md` §10 makes the envelope the contract regardless
    /// of stdout `--format`, since the AI caller must parse failures the same
    /// way every time.
    pub fn emit(&self) {
        let payload = self.payload();
        match serde_json::to_string(&payload) {
            Ok(s) => eprintln!("{s}"),
            Err(_) => eprintln!(
                "{{\"schema_version\":{SCHEMA_VERSION},\"error\":{{\"code\":\"internal_serialize\",\"message\":\"failed to serialize error envelope\"}}}}"
            ),
        }
    }
}
