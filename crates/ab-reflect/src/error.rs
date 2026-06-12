//! Error types for the A=B^Reflect interpreter.

use std::fmt;

/// Result type alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// All recoverable errors the interpreter can emit.
#[derive(Debug)]
pub enum Error {
    /// Syntax or semantic error during parsing.
    Parse {
        /// 1-based line number in the source file.
        line: usize,
        /// Human-readable description.
        message: String,
        /// Optional byte column within the line.
        column: Option<usize>,
    },
    /// Execution step limit exceeded.
    FuelExceeded {
        /// Number of steps executed before hitting the limit.
        steps: u64,
    },
    /// Tape grew beyond the configured size limit.
    TapeOverflow {
        /// Current tape length in bytes.
        len: usize,
        /// Configured limit in bytes.
        limit: usize,
    },
    /// Underlying I/O failure (stdin/stdout/file operations).
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse {
                line,
                message,
                column: Some(col),
            } => write!(f, "parse error at line {line}, column {col}: {message}"),
            Error::Parse {
                line,
                message,
                column: None,
            } => write!(f, "parse error at line {line}: {message}"),
            Error::FuelExceeded { steps } => {
                write!(f, "fuel exhausted after {steps} execution steps")
            }
            Error::TapeOverflow { len, limit } => write!(
                f,
                "tape overflow: {len} bytes exceeds limit of {limit} bytes"
            ),
            Error::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
