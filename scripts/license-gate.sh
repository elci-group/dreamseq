#!/usr/bin/env bash
set -euo pipefail

test -f LICENSE
test -f lwoodz.toml
test -f .lwoodz/manifest.json
grep -Fq 'Copyright (c) 2026 Dreamsequence Ltd' LICENSE
grep -Fq 'copyright_holder = "Dreamsequence Ltd"' lwoodz.toml

missing=0
while IFS= read -r file; do
  if ! grep -q 'SPDX-License-Identifier: MIT' "$file"; then
    printf 'missing SPDX header: %s\n' "$file" >&2
    missing=1
  fi
done < <(find src tests -type f -name '*.rs' -print | sort)
test "$missing" -eq 0

cargo deny check licenses sources
bash scripts/amber-license-gate.sh
