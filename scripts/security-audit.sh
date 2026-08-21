#!/usr/bin/env bash
# Copyright (c) 2026 Dreamsequence Ltd
# SPDX-License-Identifier: MIT
set -euo pipefail

report="$(mktemp)"
trap 'rm -f "$report"' EXIT

cargo audit --json >"$report"

# These warnings are inherited from Tauri's Linux GTK3/WebKit stack. Any new
# advisory fails the gate; removing one also fails so this reviewed exception
# list cannot silently become stale.
allowed='[
  "RUSTSEC-2024-0370",
  "RUSTSEC-2024-0411",
  "RUSTSEC-2024-0412",
  "RUSTSEC-2024-0413",
  "RUSTSEC-2024-0414",
  "RUSTSEC-2024-0415",
  "RUSTSEC-2024-0416",
  "RUSTSEC-2024-0417",
  "RUSTSEC-2024-0418",
  "RUSTSEC-2024-0419",
  "RUSTSEC-2024-0420",
  "RUSTSEC-2024-0429",
  "RUSTSEC-2025-0075",
  "RUSTSEC-2025-0080",
  "RUSTSEC-2025-0081",
  "RUSTSEC-2025-0098",
  "RUSTSEC-2025-0100"
]'

jq -e --argjson allowed "$allowed" '
  ([.warnings[][]?.advisory.id] | unique | sort) as $actual |
  ($allowed | unique | sort) as $expected |
  if .vulnerabilities.found then
    error("RustSec vulnerabilities were found")
  elif $actual != $expected then
    error("RustSec warning set changed; review docs/dependency-risk-policy.md and update the allowlist")
  else
    {vulnerabilities: 0, reviewed_warnings: ($actual | length), advisories: $actual}
  end
' "$report"
