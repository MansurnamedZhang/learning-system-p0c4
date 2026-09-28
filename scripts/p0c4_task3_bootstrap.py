#!/usr/bin/env python3
"""Root-owned bootstrap for an exact approved C4 Task3 archive.

Install this file at /var/lib/knowweave-c4/tools/bootstrap.py and verify its
own separately recorded SHA-256 before running it. This bootstrap verifies the
archive and manifest bytes before installing the acceptance runner from ZIP.
"""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import zipfile


BASE = Path("/var/lib/knowweave-c4")
RUNNER_ENTRY = "scripts/p0c4_task3_acceptance.py"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def trusted(path):
    path = Path(path).resolve()
    require(path.is_relative_to(BASE), "bootstrap must be installed under root controls")
    for item in (path, *path.parents):
        metadata = item.lstat()
        require(metadata.st_uid == 0 and not stat.S_IMODE(metadata.st_mode) & 0o022,
                "bootstrap path or ancestor is not root-owned and private")


def install_runner(archive_path, archive_sha, manifest_sha):
    require(len(archive_sha) == 64 and len(manifest_sha) == 64, "exact digest required")
    fd = os.open(archive_path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, "rb") as stream:
        data = stream.read(32 * 1024 * 1024 + 1)
    require(len(data) <= 32 * 1024 * 1024 and hashlib.sha256(data).hexdigest() == archive_sha,
            "approved archive bytes differ")
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        manifest_bytes = archive.read("SOURCE_MANIFEST.json")
        require(hashlib.sha256(manifest_bytes).hexdigest() == manifest_sha,
                "approved manifest bytes differ")
        manifest = json.loads(manifest_bytes)
        entry = next((item for item in manifest["files"] if item["path"] == RUNNER_ENTRY), None)
        require(entry is not None, "reviewed runner absent")
        runner_bytes = archive.read(RUNNER_ENTRY)
        require(entry["size"] == len(runner_bytes)
                and entry["sha256"] == hashlib.sha256(runner_bytes).hexdigest(),
                "runner differs from reviewed source manifest")
    tools = BASE / "tools"
    target = tools / f"task3-{archive_sha[:12]}-acceptance.py"
    require(not target.exists(), "reviewed runner already installed")
    fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o500)
    with os.fdopen(fd, "wb") as output:
        output.write(runner_bytes)
        output.flush()
        os.fsync(output.fileno())
    os.chmod(target, 0o500)
    return target


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--archive-sha256", required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--batch-id", required=True)
    args = parser.parse_args()
    require(os.name == "posix" and os.geteuid() == 0, "root Linux required")
    os.umask(0o077)
    trusted(Path(__file__))
    require(BASE.is_dir() and (BASE / "tools").is_dir(), "root control paths not prepared")
    trusted(BASE / "tools")
    require(stat.S_IMODE(BASE.stat().st_mode) == 0o700
            and stat.S_IMODE((BASE / "tools").stat().st_mode) == 0o700,
            "root control directories must be 0700")
    runner = install_runner(args.archive, args.archive_sha256, args.manifest_sha256)
    command = [sys.executable, "-B", str(runner), "--archive", str(args.archive),
               "--archive-sha256", args.archive_sha256,
               "--manifest-sha256", args.manifest_sha256,
               "--batch-id", args.batch_id]
    return subprocess.call(command, env={"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/root"})


if __name__ == "__main__":
    raise SystemExit(main())
