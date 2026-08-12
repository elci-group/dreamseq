# Support and Operations

Dreamseq currently provides community-grade support through repository issues
and maintainer channels. Enterprise pilots should establish a named technical
owner, escalation contact, and incident channel before processing production
logs.

## Pilot operating expectations

- Define a supported Dreamseq version and Rust/toolchain baseline.
- Keep a tested backup of anthology and ingestion-report directories.
- Pin inference endpoints and models in configuration management.
- Run with remote analysis disabled until data-flow approval is complete.
- Capture the CLI version, configuration schema version, and run ID in support
  tickets.

## Incident response

1. Disable remote analysis or revoke the paired device credential.
2. Preserve the run ID and local ingestion report.
3. Rotate any credential suspected of appearing in source logs.
4. Restrict access to affected anthology files.
5. Report the event privately using `SECURITY.md` for security incidents.

Formal uptime commitments, response-time SLAs, data residency guarantees, and
24/7 escalation are not currently offered and must be agreed separately for an
enterprise contract.
