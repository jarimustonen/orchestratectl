//! Error envelope shared by every subcommand.
//!
//! Per `AGENTS-AI-FIRST-CLI.md` §10, failures emit a structured object
//! to **stderr**. Exit codes follow §2: 0 success, 1 user/validation,
//! 2 refused-but-actionable (system/IO).

use octl_core::SCHEMA_VERSION;
use serde::Serialize;

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
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<serde_json::Value>,
}

#[derive(Debug)]
pub struct CliError {
    pub kind: ExitKind,
    pub code: String,
    pub message: String,
    pub invalid_value: Option<String>,
    pub expected: Option<serde_json::Value>,
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
        }
    }

    pub fn system(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: ExitKind::System,
            code: code.into(),
            message: message.into(),
            invalid_value: None,
            expected: None,
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

    fn payload(&self) -> ErrorPayload {
        ErrorPayload {
            schema_version: SCHEMA_VERSION,
            error: ErrorBody {
                code: self.code.clone(),
                message: self.message.clone(),
                invalid_value: self.invalid_value.clone(),
                expected: self.expected.clone(),
            },
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
