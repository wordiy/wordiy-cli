//! Error type and exit-code strategy.
//!
//! Every error path currently exits with code 1. `CliError` distinguishes a plain
//! user-facing message, a typed API error envelope (`{code, params}`), and a
//! transport failure, so call sites can stay uniform while messages differ.

use std::fmt;

pub type Result<T> = std::result::Result<T, CliError>;

#[derive(Debug)]
pub enum CliError {
    /// A user-facing error message printed before exiting.
    Message(String),

    /// A typed API error: the HTTP `status`, the server's `code`, and a
    /// human-readable `message` derived from that code.
    Api {
        status: u16,
        code: String,
        message: String,
    },

    /// The request could not be completed (connection refused, DNS, TLS, …).
    Transport(String),
}

impl CliError {
    /// Process exit code for this error. All current paths exit 1.
    pub fn exit_code(&self) -> u8 {
        match self {
            CliError::Message(_) | CliError::Api { .. } | CliError::Transport(_) => 1,
        }
    }

    /// Extra diagnostic detail (HTTP status + server code), shown in verbose mode.
    pub fn detail(&self) -> Option<String> {
        match self {
            CliError::Api { status, code, .. } => Some(format!("HTTP {status} ({code})")),
            _ => None,
        }
    }

    /// The server error `code`, when this is an API error. Used in tests.
    #[cfg(test)]
    pub fn code(&self) -> Option<&str> {
        match self {
            CliError::Api { code, .. } => Some(code),
            _ => None,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Message(msg) => f.write_str(msg),
            CliError::Api { message, .. } => f.write_str(message),
            CliError::Transport(msg) => write!(f, "could not reach the wordiy API: {msg}"),
        }
    }
}

impl std::error::Error for CliError {}

/// Build a `Message` error result in one call: `return fail("missing --path")`.
pub fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(CliError::Message(msg.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_errors_exit_with_code_1() {
        let err = CliError::Message("boom".into());
        assert_eq!(err.exit_code(), 1);
        assert_eq!(err.to_string(), "boom");
    }

    #[test]
    fn api_error_displays_its_message_and_exposes_its_code() {
        let err = CliError::Api {
            status: 400,
            code: "no_exported_result".into(),
            message: "nothing matched".into(),
        };
        assert_eq!(err.to_string(), "nothing matched");
        assert_eq!(err.code(), Some("no_exported_result"));
        assert_eq!(err.exit_code(), 1);
    }
}
