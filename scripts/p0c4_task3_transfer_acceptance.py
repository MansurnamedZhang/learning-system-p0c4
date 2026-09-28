#!/usr/bin/env python3
"""Default-build Task3 transfer smoke test; single-host evidence only.

Run only after the exact tracked-source ZIP and root path are separately
approved. No PostgreSQL service, credentials, network, or Complete receipt is
used. A passing result does not attest an independent failure domain.
"""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import uuid
import zipfile


BASE = Path("/var/lib/knowweave-c4")
BUILDER_IMAGE_ID = "sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b"
NOFOLLOW = getattr(os, "O_NOFOLLOW", 0)  # main() requires Linux; zero supports local mock tests.
GATES = (
    ("format", "cargo fmt --all -- --check", None),
    ("clippy", "cargo clippy --locked --offline --workspace --all-targets -- -D warnings", None),
    ("workspace-compile", "cargo test --locked --offline --workspace --no-run --no-default-features", None),
    ("sealed-linux", "cargo test --locked --offline -p learning-backup --test sealed --no-default-features -- --nocapture",
     (7, ("destination_transfer_rehashes_all_bytes_and_stays_sealed",
          "destination_transfer_rejects_corrupt_source_and_existing_target"))),
    ("transfer-interruption", "cargo test --locked --offline -p learning-backup --lib sealed::fault_tests::interrupted_destination_stream_cannot_publish_sealed_or_complete --no-default-features -- --nocapture",
     (1, ("sealed::fault_tests::interrupted_destination_stream_cannot_publish_sealed_or_complete",))),
    ("complete-disabled", "cargo test --locked --offline -p learning-backup --lib complete::linux_tests --no-default-features -- --nocapture",
     (2, ("complete::linux_tests::same_device_can_never_publish_complete",
          "complete::linux_tests::default_build_cannot_open_or_publish_complete"))),
    ("witness-contract", "cargo test --locked --offline -p learning-backup --test complete_contract --no-default-features -- --nocapture",
     (3, ("same_host_or_storage_and_invalid_key_are_refused",
          "signed_witness_is_bound_to_exact_manifest_control_and_destination",
          "verifier_config_rejects_local_identity_and_noncanonical_key_or_paths"))),
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def file_sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def private_dir(path):
    path = Path(path)
    path.mkdir(mode=0o700, exist_ok=False)
    path.chmod(0o700)
    return path


def private_file(path, data, mode=0o600):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | NOFOLLOW, mode)
    with os.fdopen(fd, "wb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())
    os.chmod(path, mode)


def root_owned(path):
    path = Path(path).absolute()
    for item in (path, *path.parents):
        metadata = item.lstat()
        require(not stat.S_ISLNK(metadata.st_mode) and metadata.st_uid == 0
                and not stat.S_IMODE(metadata.st_mode) & 0o022,
                "root-owned non-writable ancestor required")


def verify_archive(path, archive_sha, manifest_sha, source_commit):
    require(all(re.fullmatch(r"[0-9a-f]{%d}" % length, value) for value, length in
                ((archive_sha, 64), (manifest_sha, 64), (source_commit, 40))),
            "exact source identity required")
    fd = os.open(path, os.O_RDONLY | NOFOLLOW)
    with os.fdopen(fd, "rb") as stream:
        data = stream.read(32 * 1024 * 1024 + 1)
    require(len(data) <= 32 * 1024 * 1024 and sha256(data) == archive_sha,
            "approved archive hash or size differs")
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        infos = archive.infolist()
        names = [item.filename for item in infos]
        require(len(names) == len(set(names)) and "SOURCE_MANIFEST.json" in names,
                "duplicate or missing archive entry")
        require(len(infos) < 1000 and sum(item.file_size for item in infos) < 32 * 1024 * 1024,
                "archive expansion budget exceeded")
        for item in infos:
            name = PurePosixPath(item.filename)
            mode = (item.external_attr >> 16) & 0o170000
            require(not item.is_dir() and not name.is_absolute() and len(name.parts) > 0
                    and ".." not in name.parts and not any(part in {"", "."} for part in name.parts)
                    and "\\" not in item.filename and "\x00" not in item.filename
                    and mode in {0, 0o100000}, "unsafe archive entry")
        manifest_bytes = archive.read("SOURCE_MANIFEST.json")
        require(sha256(manifest_bytes) == manifest_sha, "source manifest hash differs")
        manifest = json.loads(manifest_bytes)
        require(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode() == manifest_bytes,
                "source manifest is not canonical")
        require(manifest.get("format_version") == 1 and manifest.get("commit") == source_commit,
                "source commit differs")
        files = manifest.get("files")
        require(isinstance(files, list) and files
                and [entry["path"] for entry in files] == sorted(set(names) - {"SOURCE_MANIFEST.json"}),
                "manifest inventory differs")
        for entry in files:
            payload = archive.read(entry["path"])
            require(entry["size"] == len(payload) and entry["sha256"] == sha256(payload),
                    "source entry hash differs")
    return manifest, data


def extract_verified(archive_bytes, manifest, destination):
    private_dir(destination)
    with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
        for entry in manifest["files"]:
            path = destination.joinpath(*PurePosixPath(entry["path"]).parts)
            path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            private_file(path, archive.read(entry["path"]), 0o400)
    require(source_hashes(destination, manifest) == expected_hashes(manifest),
            "extracted source differs from reviewed archive")


def expected_hashes(manifest):
    return {entry["path"]: entry["sha256"] for entry in manifest["files"]}


def source_hashes(source, manifest):
    found = {}
    directories = set()
    for path in source.rglob("*"):
        require(not path.is_symlink(), "source symlink appeared")
        if path.is_file():
            found[path.relative_to(source).as_posix()] = file_sha256(path)
        elif path.is_dir():
            directories.add(path.relative_to(source).as_posix())
        else:
            raise ValueError("special source entry appeared")
    require(set(found) == set(expected_hashes(manifest)), "source file set changed")
    expected_dirs = {str(parent) for entry in manifest["files"]
                     for parent in PurePosixPath(entry["path"]).parents if str(parent) != "."}
    require(directories == expected_dirs, "source directory set changed")
    return found


def redact(data):
    text = data.decode("utf-8", errors="replace")
    for pattern in (r"(?i)(?:postgres(?:ql)?://)[^\s'\"]+",
                    r"(?i)(?:password|token|secret|PGPASSWORD)\s*[=:]\s*[^\s'\"]+",
                    r"(?i)Bearer\s+[^\s'\"]+"):
        text = re.sub(pattern, "[REDACTED]", text)
    return text.encode("utf-8")


def assert_test_output(label, output, expected):
    count, names = expected
    text = output.decode("utf-8", errors="replace")
    require(re.search(rf"test result: ok\. {count} passed; 0 failed; 0 ignored;", text),
            label + " test count differs")
    for name in names:
        require(re.search(rf"^test {re.escape(name)} \.\.\. ok$", text, re.M),
                label + " expected test did not pass: " + name)


def docker_preflight(project):
    require(not os.environ.get("DOCKER_HOST"), "Docker endpoint override forbidden")
    require(not os.environ.get("DOCKER_CONTEXT"), "Docker context override forbidden")
    endpoint = json.loads(subprocess.check_output(
        ["/usr/bin/docker", "context", "inspect", "--format", "{{json .Endpoints.docker.Host}}"]
    ))
    require(endpoint == "unix:///var/run/docker.sock", "local Docker socket required")
    actual = subprocess.check_output(["/usr/bin/docker", "image", "inspect", BUILDER_IMAGE_ID,
                                      "--format", "{{.Id}}"], text=True).strip()
    require(actual == BUILDER_IMAGE_ID, "reviewed builder image is unavailable")
    labels = subprocess.check_output(["/usr/bin/docker", "ps", "-aq", "--filter",
                                      f"label=com.knowweave.acceptance.project={project}"])
    require(not labels.strip(), "project label already in use")
    for label, _, _ in GATES:
        name = f"{project}-{label}"
        require(subprocess.run(["/usr/bin/docker", "inspect", name], capture_output=True).returncode != 0,
                "project container name already exists")


def run_gate(batch, project, source, target, archive_sha, source_commit, gate):
    label, shell, expected = gate
    container = f"{project}-{label}"
    command = ["/usr/bin/docker", "run", "--rm", "--name", container,
               "--label", f"com.knowweave.acceptance.project={project}",
               "--network", "none", "--cap-drop", "ALL",
               "--security-opt", "no-new-privileges", "--user", "0:0",
               "--workdir", "/reviewed", "--tmpfs", "/tmp:rw,nosuid,nodev,size=1g",
               "--mount", f"type=bind,src={source},dst=/reviewed,readonly",
               "--mount", f"type=bind,src={target},dst=/target",
               "--env", "CARGO_TARGET_DIR=/target", "--env", "CARGO_BUILD_JOBS=4",
               "--env", "CARGO_TERM_COLOR=never",
               "--env", f"KNOWWEAVE_SOURCE_COMMIT={source_commit}",
               "--env", f"KNOWWEAVE_BUILD_ID_SHA256={archive_sha}",
               "--entrypoint", "/bin/sh", BUILDER_IMAGE_ID, "-ec",
               "unset KNOWWEAVE_C4_VERIFIER_KEY_SHA256; " + shell]
    evidence = batch / "evidence"
    try:
        result = subprocess.run(command, capture_output=True, timeout=7200, check=False,
                                env={"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/root"})
    except subprocess.TimeoutExpired as error:
        private_file(evidence / f"{label}.stdout.log", redact(error.stdout or b""))
        private_file(evidence / f"{label}.stderr.log", redact(error.stderr or b""))
        private_file(evidence / f"{label}.exit", b"timeout")
        raise RuntimeError(label + " timed out; inspect root-only evidence") from None
    stdout, stderr = redact(result.stdout), redact(result.stderr)
    private_file(evidence / f"{label}.stdout.log", stdout)
    private_file(evidence / f"{label}.stderr.log", stderr)
    private_file(evidence / f"{label}.exit", str(result.returncode).encode())
    require(result.returncode == 0, label + " failed; inspect root-only evidence")
    if expected is not None:
        assert_test_output(label, stdout + b"\n" + stderr, expected)
    return {"gate": label, "exit": result.returncode,
            "stdout_sha256": sha256(stdout), "stderr_sha256": sha256(stderr),
            "tests": expected[0] if expected else None}


def cleanup_project(project):
    names = [f"{project}-{label}" for label, _, _ in GATES]
    errors = []
    for name in names:
        inspection = subprocess.run(
            ["/usr/bin/docker", "inspect", "--format", "{{json .}}", name],
            capture_output=True)
        if inspection.returncode != 0:
            stderr = inspection.stderr.lower().strip()
            absent = any(stderr == prefix + name.encode() for prefix in (
                b"error: no such object: ",
                b"error response from daemon: no such object: ",
                b"error: no such container: ",
                b"error response from daemon: no such container: ",
            ))
            if not absent:
                errors.append(name + ":inspect-failed")
            continue
        try:
            facts = json.loads(inspection.stdout)
            identity = facts["Id"]
            matching = (facts.get("Name") == "/" + name
                        and ((facts.get("Config") or {}).get("Labels") or {}).get(
                            "com.knowweave.acceptance.project") == project
                        and re.fullmatch(r"[0-9a-f]{64}", identity) is not None)
        except (ValueError, KeyError, TypeError, AttributeError):
            matching = False
        if not matching:
            errors.append(name + ":foreign-label-not-removed")
            continue
        # Remove the inspected immutable ID, never a name that may be reused.
        result = subprocess.run(["/usr/bin/docker", "rm", "-f", identity], capture_output=True)
        if result.returncode != 0:
            errors.append(name + ":remove-failed")
    remaining = subprocess.check_output(["/usr/bin/docker", "ps", "-aq", "--filter",
                                         f"label=com.knowweave.acceptance.project={project}"])
    if remaining.strip():
        errors.append("project-label-containers-remain")
    return {"container_names": names, "remaining": remaining.decode().split(), "errors": errors,
            "scratch_retained_root_only": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--archive-sha256", required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--batch-id", required=True)
    args = parser.parse_args()
    require(os.name == "posix" and NOFOLLOW != 0 and os.geteuid() == 0,
            "root Linux with no-follow filesystem support required")
    os.umask(0o077)
    root_owned(Path(__file__))
    require(BASE.is_dir() and stat.S_IMODE(BASE.stat().st_mode) == 0o700,
            "root-only C4 base absent")
    root_owned(BASE)
    batch_id = uuid.UUID(args.batch_id)
    require(str(batch_id) == args.batch_id and batch_id.version == 4, "fresh canonical batch UUIDv4 required")
    project = f"learning-system-p0c4-task3-transfer-{batch_id.hex[:12]}"
    manifest, approved_bytes = verify_archive(args.archive, args.archive_sha256,
                                               args.manifest_sha256, args.source_commit)
    docker_preflight(project)
    batches = BASE / "batches"
    if not batches.exists():
        private_dir(batches)
    root_owned(batches)
    require(stat.S_IMODE(batches.stat().st_mode) == 0o700, "batch parent must be 0700")
    batch = private_dir(batches / f"transfer-{args.archive_sha256[:12]}-{batch_id}")
    private_dir(batch / "evidence")
    private_file(batch / "approved-source.zip", approved_bytes, 0o400)
    manifest, root_bytes = verify_archive(batch / "approved-source.zip", args.archive_sha256,
                                          args.manifest_sha256, args.source_commit)
    source = batch / "source"
    extract_verified(root_bytes, manifest, source)
    before = source_hashes(source, manifest)
    target = private_dir(batch / "target")
    status, passed, failure = "FAILED", [], None
    try:
        for gate in GATES:
            passed.append(run_gate(batch, project, source, target, args.archive_sha256,
                                   args.source_commit, gate))
        status = "SINGLE_HOST_TRANSFER_GATES_PASSED_NO_COMPLETE"
    except Exception as error:
        failure = {"type": type(error).__name__, "message": str(error)}
    finally:
        try:
            unchanged = source_hashes(source, manifest) == before
        except Exception:
            unchanged = False
        cleanup = cleanup_project(project)
        if not unchanged or cleanup["errors"]:
            status = "FAILED"
        result = {"status": status, "source_commit": args.source_commit,
                  "archive_sha256": args.archive_sha256,
                  "manifest_sha256": args.manifest_sha256,
                  "project": project, "source_unchanged": unchanged, "gates": passed,
                  "failure": failure, "cleanup": cleanup, "evidence": str(batch / "evidence")}
        private_file(batch / "result.json", json.dumps(result, sort_keys=True, indent=2).encode())
        print(json.dumps({"status": status, "result_sha256": file_sha256(batch / "result.json"),
                          "evidence": str(batch / "result.json")}, sort_keys=True))
    return 0 if status == "SINGLE_HOST_TRANSFER_GATES_PASSED_NO_COMPLETE" else 1


if __name__ == "__main__":
    raise SystemExit(main())
