#!/usr/bin/env python3
"""Build and verify deterministic Dreamseq release artifacts."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
from pathlib import Path
import tarfile
import zipfile


EPOCH_ZIP = (1980, 1, 1, 0, 0, 0)


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def release_files(binary: Path, readme: Path, license_file: Path) -> list[tuple[Path, str, int]]:
    return [
        (binary, binary.name, 0o755),
        (license_file, "LICENSE", 0o644),
        (readme, "README.md", 0o644),
    ]


def package_tar(output: Path, files: list[tuple[Path, str, int]]) -> None:
    with output.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for source, name, mode in sorted(files, key=lambda item: item[1]):
                    data = source.read_bytes()
                    info = tarfile.TarInfo(name)
                    info.size = len(data)
                    info.mode = mode
                    info.mtime = 0
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    archive.addfile(info, io.BytesIO(data))


def package_zip(output: Path, files: list[tuple[Path, str, int]]) -> None:
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for source, name, mode in sorted(files, key=lambda item: item[1]):
            info = zipfile.ZipInfo(name, EPOCH_ZIP)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.create_system = 3
            info.external_attr = (mode & 0xFFFF) << 16
            archive.writestr(info, source.read_bytes(), compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)


def package(args: argparse.Namespace) -> None:
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    files = release_files(Path(args.binary), Path(args.readme), Path(args.license))
    missing = [str(path) for path, _, _ in files if not path.is_file()]
    if missing:
        raise SystemExit("missing release input: " + ", ".join(missing))
    if args.format == "tar.gz":
        package_tar(output, files)
    else:
        package_zip(output, files)


def manifest(args: argparse.Namespace) -> None:
    paths = sorted((Path(value) for value in args.files), key=lambda path: path.name)
    if len({path.name for path in paths}) != len(paths):
        raise SystemExit("manifest inputs must have unique basenames")
    lines = []
    for path in paths:
        if not path.is_file():
            raise SystemExit(f"manifest input does not exist: {path}")
        lines.append(f"{digest(path)}  {path.name}\n")
    Path(args.output).write_text("".join(lines), encoding="ascii", newline="\n")


def verify(args: argparse.Namespace) -> None:
    manifest_path = Path(args.manifest)
    base = manifest_path.parent
    seen: set[str] = set()
    for number, line in enumerate(manifest_path.read_text(encoding="ascii").splitlines(), 1):
        parts = line.split("  ", 1)
        if len(parts) != 2 or len(parts[0]) != 64:
            raise SystemExit(f"invalid manifest line {number}")
        expected, name = parts
        if name in seen or name != Path(name).name:
            raise SystemExit(f"unsafe or duplicate manifest path on line {number}")
        seen.add(name)
        artifact = base / name
        if not artifact.is_file() or digest(artifact) != expected:
            raise SystemExit(f"checksum mismatch: {name}")
    if not seen:
        raise SystemExit("manifest is empty")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    commands = root.add_subparsers(dest="command", required=True)
    build = commands.add_parser("package")
    build.add_argument("--binary", required=True)
    build.add_argument("--readme", default="README.md")
    build.add_argument("--license", default="LICENSE")
    build.add_argument("--format", choices=("tar.gz", "zip"), required=True)
    build.add_argument("--output", required=True)
    build.set_defaults(action=package)
    create = commands.add_parser("manifest")
    create.add_argument("--output", required=True)
    create.add_argument("files", nargs="+")
    create.set_defaults(action=manifest)
    check = commands.add_parser("verify")
    check.add_argument("--manifest", required=True)
    check.set_defaults(action=verify)
    return root


if __name__ == "__main__":
    arguments = parser().parse_args()
    arguments.action(arguments)
