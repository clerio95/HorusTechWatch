//! Per-command response payload parsers.
//!
//! Each parser takes the `payload` bytes (everything after the 2-char
//! command-index echo) and returns a structured value plus an `Err`
//! describing what went wrong if it cannot. Parsers are defensive:
//! they never panic, and they preserve as much partial information as
//! possible when later fields are malformed.

pub mod clock;
pub mod device_info;
pub mod diagnostics;
pub mod status;
pub mod wireless;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParseError {}

impl ParseError {
    pub fn new(s: impl Into<String>) -> Self {
        ParseError(s.into())
    }
}
