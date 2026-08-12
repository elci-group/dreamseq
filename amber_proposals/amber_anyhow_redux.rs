//! amber_anyhow — Minimal replacement for `anyhow`
//!
//! Provides basic error chaining without the proc-macro overhead.

use std::error::Error;
use std::fmt;

/// A generic error type with context
#[derive(Debug)]
pub struct AmberError {
    message: String,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl fmt::Display for AmberError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for AmberError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref() as _)
    }
}

/// Create an error from a string
pub fn anyhow(msg: impl Into<String>) -> AmberError {
    AmberError {
        message: msg.into(),
        source: None,
    }
}

// Re-export commonly used types
pub type Result<T> = std::result::Result<T, AmberError>;

/// Add context to a result
pub trait Context<T> {
    fn context(self, msg: impl Into<String>) -> std::result::Result<T, AmberError>;
    fn with_context<F>(self, f: F) -> std::result::Result<T, AmberError>
    where
        F: FnOnce() -> String;
}

impl<T, E: Error + Send + Sync + 'static> Context<T> for std::result::Result<T, E> {
    fn context(self, msg: impl Into<String>) -> std::result::Result<T, AmberError> {
        self.map_err(|e| AmberError {
            message: msg.into(),
            source: Some(Box::new(e)),
        })
    }

    fn with_context<F>(self, f: F) -> std::result::Result<T, AmberError>
    where
        F: FnOnce() -> String,
    {
        self.map_err(|e| AmberError {
            message: f(),
            source: Some(Box::new(e)),
        })
    }
}

/// Macro for early returns with ?
#[macro_export]
macro_rules! bail {
    ($msg:literal) => { return Err($crate::anyhow($msg)) };
    ($fmt:expr, $($arg:tt)*) => { return Err($crate::anyhow(format!($fmt, $($arg)*))) };
}

/// Ensure a condition is true
#[macro_export]
macro_rules! ensure {
    ($cond:expr, $msg:literal) => {
        if !$cond { $crate::bail!($msg) }
    };
    ($cond:expr, $fmt:expr, $($arg:tt)*) => {
        if !$cond { $crate::bail!($fmt, $($arg)*) }
    };
}

