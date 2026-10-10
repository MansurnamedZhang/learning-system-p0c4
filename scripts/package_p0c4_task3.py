#!/usr/bin/env python3
"""Package exactly HEAD's tracked bytes for a separately approved C4 Task3 run."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import zipfile

# Explicit current Task1 source inclusion inventory. tracked_snapshot still
# packages exact tracked HEAD bytes; no private/historical evidence is added.
TASK1_SOURCE_FILES = (
    "crates/learning-backup/src/registry.rs", "crates/learning-backup/src/protection.rs",
    "crates/learning-backup/src/protection_tests.rs", "scripts/p0c4_storage_registry.py",
    "scripts/p0c4_completion_acceptance.py", "scripts/test_p0c4_storage_registry.py",
    "scripts/test_p0c4_completion_acceptance.py", "scripts/p0c4_completion/__init__.py", "scripts/p0c4_completion/plan.py",
    "scripts/p0c4_completion/resources.py", "scripts/p0c4_completion/evidence.py",
    "scripts/test_p0c4_completion_resources.py",
)


def tracked_snapshot(repository):
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repository, text=True).strip()
    paths = subprocess.check_output(["git", "ls-tree", "-r", "-z", "HEAD"], cwd=repository)
    entries = {}
    for raw in paths.split(b"\0"):
        if raw:
            header, name = raw.split(b"\t", 1)
            if header.split(b" ", 1)[0] not in {b"100644", b"100755"}:
                raise ValueError("tracked non-regular entry cannot be packaged")
            path = name.decode("utf-8")
            entries[path] = subprocess.check_output(["git", "show", f"HEAD:{path}"], cwd=repository)
    return commit, entries


def package(repository, destination):
    commit, entries = tracked_snapshot(repository)
    manifest = {
        "format_version": 1,
        "commit": commit,
        "files": [{"path": path, "sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}
                  for path, data in sorted(entries.items())],
    }
    manifest_bytes = json.dumps(manifest, ensure_ascii=False, sort_keys=True,
                                separators=(",", ":")).encode("utf-8")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_STORED) as archive:
        for path, data in sorted(entries.items()):
            info = zipfile.ZipInfo(path, date_time=(1980, 1, 1, 0, 0, 0))
            info.external_attr = 0o100444 << 16
            archive.writestr(info, data)
        info = zipfile.ZipInfo("SOURCE_MANIFEST.json", date_time=(1980, 1, 1, 0, 0, 0))
        info.external_attr = 0o100444 << 16
        archive.writestr(info, manifest_bytes)
    return {
        "archive": str(destination.resolve()),
        "archive_sha256": hashlib.sha256(destination.read_bytes()).hexdigest(),
        "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
        "source_commit": commit,
        "tracked_file_count": len(entries),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(package(args.repository, args.output), sort_keys=True))


if __name__ == "__main__":
    main()
