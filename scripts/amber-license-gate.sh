#!/usr/bin/env bash
set -euo pipefail

report="$(mktemp)"
trap 'rm -f "$report"' EXIT

set +e
amber --format json --threshold 100 >"$report"
status=$?
set -e

# Amber returns 1 when it emits ordinary recommendations. Anything worse is
# an analysis failure, while license policy is checked explicitly below.
if [ "$status" -gt 1 ]; then
  exit "$status"
fi

jq -e '
  (.results | length > 0) and
  (all(.results[]; (.metadata.license // "") != "")) and
  ([.results[] | select((.metadata.license // "") | test("GPL|AGPL|LGPL"; "i"))] | length == 0)
' "$report" >/dev/null
