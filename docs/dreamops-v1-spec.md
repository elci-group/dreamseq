# `dreamops` v1 specification

**Decision:** build a read-only CLI first. It composes current repositories, scripts, receipts, and approved live checks; it does not deploy, mutate tenants, trigger billing, or execute Dreamboat journeys.

## Product contract

`dreamops` answers the operator’s ten questions in report question 91: deployed identity, health, drift, qualification, evidence freshness, pilot funnel, failed-run diagnosis, customer value, billing readiness, and approval needs. Every answer is a check record, never an unqualified dashboard number.

## Commands

| Command | v1 behavior | Primary sources | Required output |
|---|---|---|---|
| `snapshot` | Assemble one timestamped product/operations record. | Git, release manifests, readiness/pilot endpoints, receipts, CI artifacts. | Stable JSON; optional redacted human view. |
| `doctor` | Run deterministic local and approved remote checks. | Scripts, OpenAPI, health/ready, security and evidence ledgers. | One record per check with pass/warn/fail/unknown/blocked. |
| `drift` | Compare normalized contracts and claims. | Routes, OpenAPI, React calls, CLI manifest, migrations, docs, live tuple. | Pairwise differences with allowlist, owner, remediation. |
| `qualify` | Wrap the ordered enterprise gate. | `scripts/ci-enterprise.sh`, `deliver.enterprise.toml`, smoke tests. | JSON and JUnit, named expected/actual, cleanup/timing/environment. |
| `evidence` | Build the readiness scorecard from receipts. | `evidence/`, eval results, restore/rollback receipts, pilot ledger. | Freshness-aware scorecard; plans/prose rejected as observations. |
| `pilot` | Read privacy-safe funnel and outcome aggregates. | Protected pilot endpoints/ledger and application operations. | Counts/rates/ages only; no tenant content or identifiers. |

## Check schema

```json
{
  "schema_version": 1,
  "check_id": "contracts.weekly-pilot-evidence",
  "status": "fail",
  "observed_at": "2026-09-07T00:00:00Z",
  "source": "dreamsequence-api/tests/smoke.sh",
  "source_revision": "d2d7604",
  "expected": "runs_succeeded_observed=2",
  "actual": "runs_succeeded_observed=0",
  "evidence_ref": "sha256:<bounded-redacted-receipt>",
  "evidence_kind": "executable_test",
  "freshness_class": "pilot_outcome",
  "expires_at": null,
  "remediation": "reconcile weekly aggregation with review outcome storage",
  "owner_role": "api-owner",
  "decision_deadline": "2026-09-10"
}
```

Allowed statuses are `pass`, `warn`, `fail`, `unknown`, and `blocked`. `unknown` means the source is unavailable or the fact is not evidenced; `blocked` means a required prerequisite or credential is intentionally absent. Exit codes are stable: 0 all pass, 10 warnings only, 20 fail, 30 unknown/blocked without fail, 40 configuration or schema error.

## Snapshot minimum

`observed_at`, source revision, schema version, deployment tuple (frontend hash, API image digest, schema, CLI and Dreamboat versions), endpoint observations, dependency readiness, pilot funnel counts, active incidents, evidence age/expiry, unresolved gates, and hash-only references. Customer content, prompts, source, paths, credentials, tenant IDs, and personal names are prohibited.

## Trust boundaries and freshness

- Local unauthenticated checks: Git, manifests, file contracts, OpenAPI, scripts, static drift, test fixtures.
- Narrowly scoped credentials: live readiness/health, protected pilot aggregates, deployment identity, identity/billing preflights.
- No credential is written to an artifact; evidence references are SHA-256 hashes or approved opaque receipt IDs.
- Proposed freshness windows: deployment and health 24 hours; SLO, security and inference quality 7 days; restore, billing and identity 30 days; pilot outcomes 7 days during an active cohort.
- A stale fact becomes `unknown` or `warn` according to gate criticality; it is never silently reused.

## Adapters

1. `git`: revision, dirty state, changed files, recent commits.
2. `contracts`: route registration, OpenAPI, React calls, CLI/release manifest, Dreamboat schemas.
3. `qualification`: ordered `scripts/ci-enterprise.sh` and Deliver; capture named failures, ports, lease, cleanup and duration.
4. `readiness`: public health/ready checks, explicitly classified as reachability, contract, or transaction.
5. `pilot`: protected aggregate endpoint and weekly evidence contract; preserve expected/actual zero regression.
6. `receipts`: dated JSON under `evidence/`, with schema, source revision, observed time and hash.
7. `deployment`: release manifests, image/artifact digests, migration and rollback records.
8. `docs`: readiness claims and deadlines, compared only after normalization and allowlist review.

## Two-week implementation sequence

1. Freeze JSON schema, exit codes, redaction policy, evidence freshness policy, and the operator question map.
2. Implement `snapshot` from local sources and dated receipts; add deterministic fixture tests.
3. Implement `doctor` for contracts, source cleanliness, readiness classification, evidence age, unresolved rotations, and pilot divergence.
4. Implement `qualify` around the ordered enterprise script with dynamic fixture metadata, cleanup trap, named assertion parsing, JSON/JUnit output, and bounded logs.
5. Implement `drift` for API/OpenAPI/frontend calls/release manifests and documentation claims.
6. Run one operator shadow period, compare current diagnosis time with direct script use, then freeze v1 contracts.

## Non-goals

No deploy/apply/rollback, no billing mutation, no customer-content display, no tenant data export, no model inference, no auto-remediation, and no readiness score uplift from implementation alone. A future mutating surface requires a separate threat model, lease, preview, approval, scoped credentials, immutable artifact, smoke/rollback plan, and append-only audit.

## Traceability

The command set traces to report questions 71-80 and 91-100; the snapshot schema traces to 71-74 and 95; doctor traces to 72-80 and 96; drift traces to 61-64 and 97; qualify traces to 75-77; evidence traces to 35-50 and 78-80; pilot traces to 41-50 and 81-90. The primary implementation sources are listed in the adapter table, so v1 composes existing behavior rather than creating a second control plane.

