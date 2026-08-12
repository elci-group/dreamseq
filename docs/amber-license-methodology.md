# Amber license-aware recommendation methodology

Dreamseq treats dependency licensing as a release and replacement constraint.
Replaceability alone must never turn an incompatible or unattributed dependency
into an automated recommendation.

## Recommendation order

1. Resolve the dependency's SPDX expression from Cargo metadata and the
   generated transitive manifest.
2. Reject unknown, incompatible, or undisclosed licenses at the release gate.
3. Prefer replacements with an equal-or-more-permissive compatible license,
   while preserving attribution obligations.
4. Apply Amber's security, maintenance, usage, and testability scores.
5. Downgrade any replacement recommendation to human review when license
   confidence is below the configured threshold or the replacement changes
   notice obligations.

## Decision classes

| License state | Amber recommendation impact |
| --- | --- |
| Known and compatible | Score normally; preserve notices |
| Known but attribution-bearing | Human review; update notices before merge |
| Copyleft or dual-license with policy implications | Block automated replacement; legal review |
| Unknown or missing SPDX expression | Block release and replacement automation |
| Incompatible with project policy | Block dependency introduction |

The local `.amber.toml` raises the security weight to 30%, keeps core runtime
and serialization dependencies required, and forbids the previously removed
unpublished `goblin` dependency. The license manifest and `cargo-deny` remain
the authoritative compatibility gates until Amber gains native license-policy
fields.
