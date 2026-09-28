#!/usr/bin/env python3
"""Root-only, single-host acceptance of a NEW clean P0-C4 pin candidate.

No CompleteBackup, restore, ACL mutation, build pin, or service admission occurs.
The isolated PG18 volume remains quarantined after its exact container is stopped.
"""

import argparse
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import sys
import uuid
import zipfile


BASE = Path("/var/lib/knowweave-c4")
ENTRY = "scripts/p0c4_restore_pin_acceptance.py"
HELPER = "scripts/p0c4_restore_birth_acceptance.py"
PROVISIONER = "scripts/p0c4_restore_target.py"
ISSUER = "scripts/p0c4_restore_target_birth.py"
PIN = "scripts/p0c4_restore_target_pin.py"
PREPARE = "scripts/p0c4_restore_target_pin_prepare.py"
INITDB = "deploy/p0c4_restore_initdb.sh"
REQUIRED = {ENTRY, HELPER, PROVISIONER, ISSUER, PIN, PREPARE, INITDB}
PASSED = "PIN_CANDIDATE_SINGLE_HOST_PG18_PASSED_QUARANTINED_NOT_RESTORE"
FAILED = "PIN_CANDIDATE_FAILED_QUARANTINED_NOT_RESTORE_NOT_PIN"
HEX64 = re.compile(r"[0-9a-f]{64}\Z")
HEX40 = re.compile(r"[0-9a-f]{40}\Z")
MAX_ARCHIVE = 32 * 1024 * 1024
MAX_EXPANDED = 64 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def _json_bytes(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False).encode("utf-8")


def _file_digest(path):
    hashed = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hashed.update(block)
    return hashed.hexdigest()


def _trusted_path(path, *, file=False):
    path = Path(path)
    require(path.is_absolute() and os.geteuid() == 0,
            "root-owned absolute path required")
    for item in reversed((path, *path.parents)):
        meta = os.lstat(item)
        expected = stat.S_ISREG if file and item == path else stat.S_ISDIR
        require(expected(meta.st_mode) and meta.st_uid == 0 and
                stat.S_IMODE(meta.st_mode) & 0o022 == 0,
                "untrusted control or source path")


def _require_private_dir(path):
    _trusted_path(path)
    require(stat.S_IMODE(os.lstat(path).st_mode) == 0o700,
            "root-private directory required")


def _sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def _private_dir(path):
    require(not os.path.lexists(path), "batch already exists")
    path.mkdir(mode=0o700)
    _require_private_dir(path)
    _sync_dir(path.parent)


def _private_write(path, payload, mode=0o600):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                 mode)
    with os.fdopen(fd, "wb") as stream:
        stream.write(payload)
        stream.flush()
        os.fsync(stream.fileno())
    os.chmod(path, mode)
    _sync_dir(path.parent)


def _safe_member(name):
    require(type(name) is str and name and not name.startswith("/") and
            "\\" not in name and ":" not in name and "\x00" not in name and
            all(part not in ("", ".", "..") for part in name.split("/")) and
            str(PurePosixPath(name)) == name, "unsafe source path")
    return name.split("/")


def verify_archive(path, archive_sha256, manifest_sha256, commit,
                   runner_sha256):
    """Verify the full reviewed snapshot and this installed runner's bytes."""
    require(all(type(value) is str and pattern.fullmatch(value) for
                value, pattern in ((archive_sha256, HEX64),
                (manifest_sha256, HEX64), (commit, HEX40),
                (runner_sha256, HEX64))),
            "exact reviewed source identity required")
    require(path.is_absolute() and ".." not in path.parts and
            path.is_relative_to(BASE / "incoming"),
            "source must be under private incoming")
    _require_private_dir(BASE / "incoming")
    _trusted_path(path, file=True)
    require(path == path.resolve(strict=True),
            "source must be a direct private incoming file")
    meta = os.lstat(path)
    require(stat.S_IMODE(meta.st_mode) == 0o400 and meta.st_nlink == 1,
            "source must be root-private 0400")
    _trusted_path(Path(__file__), file=True)
    runner_meta = os.lstat(__file__)
    require(stat.S_IMODE(runner_meta.st_mode) == 0o500 and
            runner_meta.st_nlink == 1, "runner must be root-only 0500")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as stream:
        content = stream.read(MAX_ARCHIVE + 1)
    require(len(content) <= MAX_ARCHIVE and digest(content) == archive_sha256,
            "source bytes differ")
    with zipfile.ZipFile(io.BytesIO(content)) as archive:
        infos = archive.infolist()
        names = [item.filename for item in infos]
        require(REQUIRED.issubset(names) and
                "SOURCE_MANIFEST.json" in names and
                len(names) == len(set(names)) and len(names) < 1000 and
                sum(item.file_size for item in infos) <= MAX_EXPANDED,
                "source inventory differs")
        for item in infos:
            _safe_member(item.filename)
            require(not item.is_dir() and not (item.flag_bits & 1) and
                    (item.external_attr >> 16) & 0o170000 == stat.S_IFREG and
                    item.file_size <= MAX_EXPANDED,
                    "source member is not regular plaintext")
        manifest_bytes = archive.read("SOURCE_MANIFEST.json")
        require(digest(manifest_bytes) == manifest_sha256,
                "source manifest bytes differ")
        manifest = json.loads(manifest_bytes,
                              object_pairs_hook=_unique_pairs)
        require(type(manifest) is dict and set(manifest) ==
                {"format_version", "commit", "files"} and
                type(manifest["format_version"]) is int and
                manifest["format_version"] == 1 and
                manifest["commit"] == commit and
                _json_bytes(manifest) == manifest_bytes,
                "source manifest identity differs")
        files = manifest["files"]
        require(type(files) is list and len(files) == len(names) - 1 and
                [entry["path"] for entry in files] ==
                sorted(set(names) - {"SOURCE_MANIFEST.json"}),
                "source file list differs")
        for entry in files:
            require(type(entry) is dict and set(entry) ==
                    {"path", "sha256", "size"} and
                    type(entry["sha256"]) is str and
                    HEX64.fullmatch(entry["sha256"]) and
                    type(entry["size"]) is int and
                    0 <= entry["size"] <= MAX_EXPANDED,
                    "source file record differs")
            member = archive.read(entry["path"])
            require(len(member) == entry["size"] and
                    digest(member) == entry["sha256"],
                    "source file bytes differ")
        reviewed_runner = next(entry["sha256"] for entry in files if
                               entry["path"] == ENTRY)
        require(_file_digest(__file__) == reviewed_runner == runner_sha256,
                "installed runner differs from reviewed source")
    return manifest, content


def _unique_pairs(pairs):
    value = {}
    for key, item in pairs:
        require(key not in value, "duplicate source JSON key")
        value[key] = item
    return value


def source_digest(source, manifest):
    expected = {entry["path"]: entry["sha256"] for entry in manifest["files"]}
    found = {}
    for path in source.rglob("*"):
        meta = os.lstat(path)
        require(meta.st_uid == 0 and stat.S_IMODE(meta.st_mode) & 0o022 == 0,
                "extracted source ownership changed")
        if stat.S_ISREG(meta.st_mode):
            found[path.relative_to(source).as_posix()] = _file_digest(path)
        else:
            require(stat.S_ISDIR(meta.st_mode), "source special file appeared")
    require(found == expected, "extracted source changed")
    return digest(_json_bytes({key: found[key] for key in sorted(found)}))


def _load(source, name, relative):
    spec = importlib.util.spec_from_file_location(name, source / relative)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def extract_and_load(manifest, package, batch):
    source = batch / "source"
    _private_dir(source)
    with zipfile.ZipFile(io.BytesIO(package)) as archive:
        for entry in manifest["files"]:
            path = source.joinpath(*_safe_member(entry["path"]))
            stack = []
            parent = path.parent
            while parent != source and not parent.exists():
                stack.append(parent)
                parent = parent.parent
            for directory in reversed(stack):
                _private_dir(directory)
            _private_write(path, archive.read(entry["path"]), 0o400)
    before = source_digest(source, manifest)
    initdb = batch / "initdb.sh"
    _private_write(initdb, (source / INITDB).read_bytes(), 0o444)
    require(_file_digest(initdb) == next(entry["sha256"] for entry in
                 manifest["files"] if entry["path"] == INITDB),
            "installed initdb changed")
    sys.dont_write_bytecode = True
    previous = {name: sys.modules.get(name) for name in (
        "p0c4_restore_target", "p0c4_restore_target_birth",
        "p0c4_restore_birth_acceptance", "p0c4_restore_target_pin",
        "p0c4_restore_target_pin_prepare")}
    try:
        provisioner = _load(source, "p0c4_restore_target", PROVISIONER)
        _load(source, "p0c4_restore_target_birth", ISSUER)
        acceptance = _load(source, "p0c4_restore_birth_acceptance", HELPER)
        pin = _load(source, "p0c4_restore_target_pin", PIN)
        prepare = _load(source, "p0c4_restore_target_pin_prepare", PREPARE)
        require(pin.target_provisioner is provisioner and
                prepare.target_provisioner is provisioner and
                prepare.pin is pin, "reviewed module graph differs")
        return provisioner, acceptance, pin, prepare, initdb, before
    finally:
        for name, module in previous.items():
            if module is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = module


@contextlib.contextmanager
def _acceptance_lock(root):
    import fcntl  # Linux-only; local tests mock the host boundary.
    _require_private_dir(root)
    fd = os.open(root / ".pin-acceptance.lock",
                 os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC,
                 0o600)
    try:
        meta = os.fstat(fd)
        require(stat.S_ISREG(meta.st_mode) and meta.st_uid == 0 and
                stat.S_IMODE(meta.st_mode) == 0o600 and meta.st_nlink == 1,
                "unsafe acceptance lock")
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield
    finally:
        os.close(fd)


def _prepare_batch(root, batch_id):
    _require_private_dir(root)
    batches = root / "batches"
    if not os.path.lexists(batches):
        _private_dir(batches)
    _require_private_dir(batches)
    batch = batches / batch_id
    _private_dir(batch)  # O_EXCL semantics: never replay even after a crash.
    for name in ("control", "evidence"):
        _private_dir(batch / name)
    _private_dir(batch / "control" / "targets")
    return batch


def _run_batch(args, manifest, package, batch):
    evidence = batch / "evidence"
    control = batch / "control"
    source = batch / "source"
    result = {"status": FAILED, "batch_id": args.batch_id,
              "subnet": args.subnet, "stage": "extract",
              "archive_sha256": args.archive_sha256,
              "manifest_sha256": args.manifest_sha256,
              "source_commit": args.source_commit,
              "runner_sha256": _file_digest(__file__),
              "source_before_sha256": None, "source_after_sha256": None,
              "target_condition": "NO_TARGET_CREATED",
              "target_reuse_permitted": False,
              "stop": {"confirmed": False}, "failure_type": None}
    provisioner = acceptance = identity = initdb = before = None
    confirmed_id = None
    issuer_started = False
    _private_write(evidence / "attempt.json", _json_bytes({
        "state": "PIN_ONLY_ATTEMPT_NOT_RESTORE_AUTHORITY",
        "batch_id": args.batch_id, "archive_sha256": args.archive_sha256,
        "manifest_sha256": args.manifest_sha256,
        "runner_sha256": args.runner_sha256,
        "source_commit": args.source_commit}))
    try:
        (provisioner, acceptance, pin, prepare, initdb,
         result["source_before_sha256"]) = extract_and_load(
             manifest, package, batch)
        identity = provisioner.identity_for(args.batch_id)
        result.update(project=identity["project"], volume=identity["volume"])
        result["stage"] = "fresh-admission"
        before = provisioner.snapshot()
        provisioner.admit_fresh(identity, args.subnet, before)
        result["stage"] = "precreation"
        precreation_sha = prepare.prepare(control, args.batch_id,
                                         args.subnet, initdb)
        require(type(precreation_sha) is str and HEX64.fullmatch(precreation_sha),
                "precreation digest differs")
        result["precreation_sha256"] = precreation_sha
        result["stage"] = "birth-issuer"
        result["target_condition"] = "UNVERIFIED_UNUSABLE"
        issuer_started = True
        process = acceptance._run_issuer(
            source, control, args.batch_id, args.subnet, initdb)
        target = control / "targets" / args.batch_id
        result["stage"] = "sealed-birth"
        birth, state, success = acceptance.accept_issuer_process(
            process, target, identity, args.batch_id)
        require(state["subnet"] == args.subnet,
                "birth subnet differs")
        result["stage"] = "independent-docker"
        docker = acceptance._independent_docker_gate(
            provisioner, identity, args.subnet, before, state,
            success, target, initdb)
        confirmed_id = docker["container_id"]
        require(type(confirmed_id) is str and HEX64.fullmatch(confirmed_id) and
                confirmed_id == success["container_id"],
                "confirmed PG ID differs")
        result["stage"] = "read-only-pin-check"
        candidate = pin.pin(control, args.batch_id, initdb)
        require(type(candidate) is dict and set(candidate) ==
                {"birth_sha256", "inspection_evidence_sha256"} and
                all(type(value) is str and HEX64.fullmatch(value)
                    for value in candidate.values()) and
                candidate["birth_sha256"] == success["birth_sha256"],
                "pin checker differs from sealed birth")
        result["stage"] = "source-reinspection"
        result["source_after_sha256"] = source_digest(source, manifest)
        require(result["source_after_sha256"] ==
                result["source_before_sha256"],
                "reviewed source changed during run")
        result["stage"] = "exact-id-stop"
        result["stop"] = acceptance.stop_verified_pg(
            provisioner, identity, confirmed_id)
        require(result["stop"].get("confirmed") is True and
                result["stop"].get("volume_retained") is True,
                "exact PG stop or quarantine unconfirmed")
        result["source_after_sha256"] = source_digest(source, manifest)
        require(result["source_after_sha256"] ==
                result["source_before_sha256"],
                "reviewed source changed during stop")
        require(_file_digest(__file__) == args.runner_sha256,
                "reviewed runner changed during run")
        result.update(candidate)
        result["target_condition"] = "CLEAN_STOPPED_QUARANTINED_NOT_RESTORE"
        result["status"] = PASSED
    except BaseException as error:
        result["failure_type"] = type(error).__name__
        if issuer_started and identity is not None and acceptance is not None:
            try:
                result["issuer_diagnostic"] = acceptance.read_issuer_diagnostic(
                    control / "targets" / args.batch_id, identity, args.batch_id)
            except BaseException:
                result["issuer_diagnostic"] = {"status": "UNAVAILABLE"}
        if provisioner is not None and acceptance is not None and identity is not None:
            try:
                if confirmed_id:
                    result["stop"] = acceptance.stop_verified_pg(
                        provisioner, identity, confirmed_id)
                elif issuer_started:
                    result["stop"] = acceptance.stop_early_owned_pg(
                        provisioner, identity,
                        control / "targets" / args.batch_id,
                        args.subnet, before, initdb)
            except BaseException as stop_error:
                result["stop"] = {"confirmed": False,
                                  "failure_type": type(stop_error).__name__}
        if source.exists():
            try:
                result["source_after_sha256"] = source_digest(source, manifest)
            except BaseException:
                pass
    payload = _json_bytes(result)
    _private_write(evidence / "result.json", payload)
    summary = {"status": result["status"], "result_sha256": digest(payload),
               "evidence": str(evidence), "not_restore": True}
    if result["status"] == PASSED:
        summary.update(birth_sha256=result["birth_sha256"],
                       inspection_evidence_sha256=result[
                           "inspection_evidence_sha256"])
    print(json.dumps(summary, sort_keys=True), flush=True)
    return (0 if result["status"] == PASSED else 1), result


def run(args):
    require(os.geteuid() == 0 and _canonical_v4(args.batch_id),
            "root and new UUIDv4 required")
    archive = args.archive.absolute()
    manifest, package = verify_archive(
        archive, args.archive_sha256, args.manifest_sha256,
        args.source_commit, args.runner_sha256)
    _require_private_dir(BASE)
    root = BASE / "pin-acceptance"
    if not os.path.lexists(root):
        _private_dir(root)
    with _acceptance_lock(root):
        batch = _prepare_batch(root, args.batch_id)
        code, _ = _run_batch(args, manifest, package, batch)
    return code


def _canonical_v4(value):
    try:
        parsed = uuid.UUID(value)
    except (ValueError, AttributeError, TypeError):
        return False
    return parsed.version == 4 and str(parsed) == value


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--archive-sha256", required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--runner-sha256", required=True)
    parser.add_argument("--batch-id", required=True)
    parser.add_argument("--subnet", required=True)
    try:
        return run(parser.parse_args(argv))
    except BaseException:
        print("PIN_CANDIDATE_ACCEPTANCE_ADMISSION_REJECTED", flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
