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
