//! amber_chrono — Replacement for the `chrono` crate
//!
//! For many use-cases `std::time::SystemTime` and `std::time::Duration`
//! are sufficient. For formatting/parsing, a minimal local implementation
//! or the `time` crate may be needed.

use std::time::{SystemTime, UNIX_EPOCH};

/// Current UTC timestamp as seconds since the Unix epoch.
pub fn now_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A minimal RFC 3339 timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timestamp {
    pub secs: u64,
}

impl Timestamp {
    /// Create a timestamp representing the current time.
    #[must_use]
    pub fn now() -> Self {
        Self {
            secs: now_timestamp(),
        }
    }
}

