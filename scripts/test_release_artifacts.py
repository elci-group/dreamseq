#!/usr/bin/env python3

from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile


SCRIPT = Path(__file__).with_name("release_artifacts.py")


class ReleaseArtifactsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.binary = self.root / "dreamseq"
        self.binary.write_bytes(b"binary\x00payload")
        (self.root / "README.md").write_text("readme\n", encoding="utf-8")
        (self.root / "LICENSE").write_text("license\n", encoding="utf-8")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def run_tool(self, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        return subprocess.run([sys.executable, str(SCRIPT), *args], check=check, text=True, capture_output=True)

    def build(self, format_name: str, output: Path) -> None:
        self.run_tool(
            "package", "--binary", str(self.binary), "--readme", str(self.root / "README.md"),
            "--license", str(self.root / "LICENSE"), "--format", format_name, "--output", str(output),
        )

    def test_tar_and_zip_are_reproducible_and_normalized(self) -> None:
        for format_name, suffix in (("tar.gz", ".tar.gz"), ("zip", ".zip")):
            first, second = self.root / f"first{suffix}", self.root / f"second{suffix}"
            self.build(format_name, first)
            self.build(format_name, second)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            if format_name == "tar.gz":
                with tarfile.open(first, "r:gz") as archive:
                    self.assertEqual([item.name for item in archive.getmembers()], ["LICENSE", "README.md", "dreamseq"])
                    self.assertTrue(all(item.mtime == 0 and item.uid == 0 and item.gid == 0 for item in archive.getmembers()))
            else:
                with zipfile.ZipFile(first) as archive:
                    self.assertEqual(archive.namelist(), ["LICENSE", "README.md", "dreamseq"])
                    self.assertTrue(all(item.date_time == (1980, 1, 1, 0, 0, 0) for item in archive.infolist()))

    def test_manifest_verification_rejects_tampering(self) -> None:
        artifact = self.root / "dreamseq.tar.gz"
        self.build("tar.gz", artifact)
        manifest = self.root / "SHA256SUMS"
        self.run_tool("manifest", "--output", str(manifest), str(artifact))
        self.run_tool("verify", "--manifest", str(manifest))
        artifact.write_bytes(artifact.read_bytes() + b"tamper")
        result = self.run_tool("verify", "--manifest", str(manifest), check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)


if __name__ == "__main__":
    unittest.main()
