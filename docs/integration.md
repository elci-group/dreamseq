# Integration and Export Guide

Dreamseq is currently a CLI and local-artifact product. Enterprise systems can
consume its stable machine-readable outputs without depending on terminal
formatting.

## JSON completion output

```bash
dreamseq run --json > dreamseq-run.json
dreamseq report 2026-08-07 --json > dreamseq-report.json
```

Treat the generated anthology ID, date, schema fields, and pipeline statistics
as the primary correlation fields. Store artifacts in an approved encrypted
location and apply customer retention policy.

## CI handoff

Use a dedicated workspace and an explicit configuration file. A CI job should
fail closed when the data boundary or remote-analysis consent is not present,
then publish only the JSON summary to the approved artifact store.

## Current integration boundary

There is no native webhook, SIEM exporter, Jira/Linear connector, SSO, RBAC,
or hosted multi-tenant API in this release. These should be implemented only
after a design partner confirms the workflow and required schema.

## Repository drift scores (azusa)

`dreamseq drift --json` emits the tier-limited azusa drift view: for every
cached repository, the remote observation time, the per-device local
observation times, and the azusa drift scores (0–10 composite index), or the
enterprise average across devices. Scores are produced by the external
[azusa](https://github.com/elci-group/azusa) CLI and cached per user in
`~/.config/dreamseq/drift-cache.json`, keyed by origin URL — treat that file
as the correlation source when aggregating drift history across devices.

`dreamseq run` measures drift automatically for the current repo and every
cached repo available locally (`enable_drift`, default on) and, when the
device is paired, uploads fresh observations to `POST /api/v1/drift` as a
`DriftEnvelope` (schema_version 1: device id, CLI version, upload time, and
one record per repo with identity, device, score, direction, risk, the
compared remote ref, and both capture timestamps). Device-local paths are
deliberately not part of the envelope. The service keys observations by
(account, device, repo identity) — re-upload replaces the device's previous
scores for those repos — and applies the account tier's display limits for
the website view (free 1, pro 4, enterprise averaged over up to 16).

## Stack reports

`dreamseq stack-report` detects workstation specs (OS, CPU, memory, OEM),
installed agent harnesses, and a ~490-tool developer-tools registry, and
uploads the result to `POST /api/v1/device/report` when the device is
paired. `--dry-run` prints the composed payload without sending it; treat
that as the schema reference. Detection is entirely local and best-effort —
a missing package manager, an unreadable directory, or a hung `--version`
probe degrades the report rather than failing it. `--batches` narrows
detection to a subset of the devtools registry's staged rollout batches, for
lower-cost partial reports.
