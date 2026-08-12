# Dependency license review

Reviewed 208 resolved Cargo packages with `cargo metadata`, `cargo deny list`,
and `cargo deny check licenses sources` on 2026-08-12.

- 0 unlicensed packages
- 0 incompatible license errors
- 0 unknown registry or git sources
- Allowed expressions are recorded in [`deny.toml`](../deny.toml)
- The complete machine-readable inventory is
  [`dependency-license-review.json`](dependency-license-review.json)

The generated [`THIRD_PARTY_NOTICES`](../THIRD_PARTY_NOTICES) file is retained
for distribution. Lwoodz currently emits `NOASSERTION` for some transitive
packages even though cargo-deny resolves them from registry metadata; the
release gate therefore requires both artifacts and treats cargo-deny as the
compatibility authority.
