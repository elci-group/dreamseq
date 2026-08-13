#!/usr/bin/env python3
"""Regression tests for CI-safe Amber JSON extraction."""

import unittest

from extract_amber_json import extract_payload


class ExtractAmberJsonTests(unittest.TestCase):
    def test_accepts_plain_json(self) -> None:
        self.assertEqual(extract_payload('{"results":[]}'), {"results": []})

    def test_ignores_progress_and_trailing_output(self) -> None:
        payload = extract_payload(
            'INFO indexing\n  ✓ dependencies indexed\n{"results":[{"crate":"serde"}]}\ncomplete\n'
        )
        self.assertEqual(payload["results"], [{"crate": "serde"}])

    def test_rejects_unrelated_or_malformed_objects(self) -> None:
        with self.assertRaises(ValueError):
            extract_payload('INFO {not-json}\n{"status":"done"}')


if __name__ == "__main__":
    unittest.main()
