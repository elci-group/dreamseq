// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT

/// Create a correlation identifier for one end-to-end Dreamseq operation.
///
/// Public async entry points create an ID when callers do not provide one;
/// callers that span multiple operations can use the corresponding
/// `*_with_trace_id` methods to preserve one trace across the whole workflow.
#[must_use]
pub fn new_trace_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_trace_ids_are_unique_uuid_values() {
        let first = new_trace_id();
        let second = new_trace_id();

        assert_ne!(first, second);
        assert!(uuid::Uuid::parse_str(&first).is_ok());
        assert!(uuid::Uuid::parse_str(&second).is_ok());
    }
}
