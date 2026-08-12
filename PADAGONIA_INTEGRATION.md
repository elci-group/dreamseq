# PADAGONIA Integration Roadmap

Implement the common contract in `/home/sal/padagonia/docs/enterprise-integration-directives.md`.

## Modules

- `event_normalizer`: convert Bound/harness events to a versioned privacy-
  reduced schema.
- `redactor`: remove raw prompts, credentials, personal data, and high-risk
  payloads before graph writes; retain hashes and safe references.
- `batch_writer`: use idempotent transaction batches and bounded offline replay.
- `trend_reader`: query recurring tools, models, friction, interventions, and
  directives by cohort, time, and project namespace.
- `directive_lineage`: connect evidence patterns to recommendations and later
  outcomes.
- `retention`: enforce deletion, expiry, and non-linkability policies.

## Acceptance gates

Cohort-only reporting, deterministic reruns, deletion verification, conflict
visibility, and no raw sensitive content in Padagonia by default.
