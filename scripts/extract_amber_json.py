#!/usr/bin/env python3
"""Extract Amber's JSON payload from output that may include progress text."""

from __future__ import annotations

import json
import sys
from pathlib import Path


def extract_payload(raw: str) -> dict[str, object]:
    decoder = json.JSONDecoder()
    for offset, character in enumerate(raw):
        if character != "{":
            continue
        try:
            payload, _ = decoder.raw_decode(raw[offset:])
        except json.JSONDecodeError:
            continue
        if isinstance(payload, dict) and isinstance(payload.get("results"), list):
            return payload
    raise ValueError("Amber output did not contain a JSON object with a results array")


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: extract_amber_json.py INPUT OUTPUT", file=sys.stderr)
        return 2
    try:
        payload = extract_payload(Path(sys.argv[1]).read_text(encoding="utf-8"))
    except (OSError, UnicodeError, ValueError) as error:
        print(f"failed to normalize Amber report: {error}", file=sys.stderr)
        return 1
    Path(sys.argv[2]).write_text(
        json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
