#!/usr/bin/env bash
set -euo pipefail

report="$(mktemp)"
normalized_report="$(mktemp)"
trap 'rm -f "$report" "$normalized_report"' EXIT

set +e
amber --format json --threshold 100 >"$report"
status=$?
set -e

# Amber returns 1 when it emits ordinary recommendations. Anything worse is
# an analysis failure, while license policy is checked explicitly below.
if [ "$status" -gt 1 ]; then
  exit "$status"
fi

# Amber may write human-readable progress to stdout before the JSON document
# when it detects a CI terminal. Normalize the first complete JSON object before
# applying the policy so the gate behaves identically locally and in CI.
python3 scripts/extract_amber_json.py "$report" "$normalized_report"

jq -e '
  (.results | length > 0) and
  (all(.results[]; (.metadata.license // "") != "")) and
  ([.results[] | select((.metadata.license // "") | test("GPL|AGPL|LGPL"; "i"))] | length == 0)
' "$normalized_report" >/dev/null
