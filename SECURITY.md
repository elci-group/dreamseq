# Security Policy

## Scope

Dreamseq processes local AI-agent logs and can send explicitly consented,
redacted excerpts to configured inference services. Credential storage,
redaction, transport validation, and artifact persistence are security-sensitive
components.

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability. Report it privately
to the repository maintainers through the security contact configured for the
`elci-group/dreamseq` repository. Include the affected version, reproduction
steps, impact, and whether sensitive data may have been exposed.

Maintainers should acknowledge reports within five business days, provide an
initial severity assessment within ten business days, and coordinate a fix or
mitigation with the reporter before public disclosure.

## Security boundaries

- Remote analysis is disabled unless `allow_remote_analysis` is enabled.
- Credentials are kept in environment variables or owner-only local files.
- Production endpoints require HTTPS; loopback HTTP is test-only.
- Remote prompt batches are bounded and common credentials, JWTs, emails, and
  home-directory paths are redacted as defense in depth.
- Private artifacts use atomic writes and restrictive permissions on Unix.
- Remote API responses are bounded to 4 MiB.

Redaction is not a guarantee of anonymization. Customers must review log
sources and configure retention and access controls appropriate to their data.

## Release checks

```text
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo audit
cargo deny check advisories bans licenses sources
cargo package --allow-dirty
```
