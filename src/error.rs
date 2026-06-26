//! Error type and exit-code strategy.
//!
//! Every error path currently exits with code 1; `CliError` has room to grow
//! (e.g. an HTTP error-envelope variant) without changing call sites.

use std::fmt;

pub type Result<T> = std::result::Result<T, CliError>;

#[derive(Debug)]
pub enum CliError {
    /// A user-facing error message printed before exiting.
    Message(String),
}

impl CliError {
    /// Process exit code for this error. All current paths exit 1.
    pub fn exit_code(&self) -> u8 {
        match self {
            CliError::Message(_) => 1,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Message(msg) => f.write_str(msg),
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
}
