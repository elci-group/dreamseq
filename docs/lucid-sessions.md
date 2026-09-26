# Ingesting Lucid agent sessions

Lucid, Dreamsequence's agent platform, records every agent action as an
event and publishes a project's events as `lucid.session/v1`, one JSON object
per line, at `GET /api/lucid/v1/projects/{project}/signals/export`. Dreamseq
reads those exports natively, so Lucid tasks go through the same
normalization, segmentation and pattern stages as other harnesses.

## What a record carries

Each line has the event's provenance (tenant, project, task, execution,
attempt, sequence, `event_id`, `occurred_at`) and the agent's role, provider
and model. Lucid has already reduced the payload to an allow-list of
operational fields (`detail`: tool, status, reject reason, usage and so on)
plus `payload_sha256`. Prompts, file contents and paths are not in the feed.

Dreamseq maps each event to one log entry:

- the entry id is the Lucid `event_id`. An event delivered twice
  (overlapping exports, a retried pull) collapses to one entry and is
  reported as a harness warning. Distinct events with identical content
  stay, because repetition is evidence;
- `tool.completed` and `policy.denied` become tool calls with their outcome
  (`applied`, `rejected`, `failed`, `denied`);
- `message.applied` counts as a user steering message and `model.response`
  as an assistant message;
- model and provider come from the event's agent.

Records with another `schema_version` are rejected, not guessed at.

## Pulling exports

```bash
export LUCID_API_TOKEN=…   # a Lucid bearer token for the project; never logged
dreamseq lucid pull --url https://lucid.dreamsequence.pro \
  --project my-project --out ~/.local/share/dreamseq/lucid/my-project
```

The first pull fetches `--days` (default 7, at most 90). Later pulls continue
from where the last one ended; the position is kept in
`.lucid-pull-<project>.json` in the output directory (no credentials). Each
page is written as `lucid-<project>-<first event time>.jsonl`.

## Configuring the harness

Add a harness that reads the output directory:

```json
{ "name": "lucid", "log_path": "~/.local/share/dreamseq/lucid/my-project",
  "log_format": "lucid_session" }
```

Only `*.jsonl` files are read. The contract fixture
`tests/fixtures/lucid/session-v1.jsonl` is shared with Lucid
(`lucid/contracts/v1/fixtures/session-v1.jsonl`), and both repositories test
against it.
