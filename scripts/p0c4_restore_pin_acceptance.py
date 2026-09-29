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
import subprocess
import sys
import tempfile
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
BOUND_PASSED = "BOUND_TARGET_READ_ONLY_SINGLE_HOST_PG18_PASSED_QUARANTINED_NOT_RESTORE"
BOUND_FAILED = "BOUND_TARGET_READ_ONLY_FAILED_QUARANTINED_NOT_RESTORE_NOT_PIN"
GUARD_PASSED = "BOUND_TARGET_GUARD_READ_ONLY_SINGLE_HOST_PG18_PASSED_QUARANTINED_NOT_RESTORE"
GUARD_FAILED = "BOUND_TARGET_GUARD_READ_ONLY_FAILED_QUARANTINED_NOT_RESTORE_NOT_PIN"
SESSION_PASSED = "FOCUSED_SQL_SESSION_GATES_PASSED_NOT_FULL_ENDPOINT_ACCEPTANCE_READ_ONLY_NOT_RESTORE"
SESSION_FAILED = "SQL_SESSION_BINDING_READ_ONLY_FAILED_QUARANTINED_NOT_RESTORE_NOT_PIN"
BUILDER_IMAGE_ID = "sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b"
HEX64 = re.compile(r"[0-9a-f]{64}\Z")
HEX40 = re.compile(r"[0-9a-f]{40}\Z")
MAX_ARCHIVE = 32 * 1024 * 1024
MAX_EXPANDED = 64 * 1024 * 1024
MAX_INSPECTION = 256 * 1024


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


def _publish_result(evidence, payload):
    """Publish only a complete synced result; never overwrite a prior batch."""
    pending = evidence / "result.pending.json"
    final = evidence / "result.json"
    _private_write(pending, payload)
    # link() is atomic and fails if the final name already exists. A crash can
    # leave both names, but the caller still has no exit-0 completion receipt.
    os.link(pending, final)
    _sync_dir(evidence)
    os.unlink(pending)
    _sync_dir(evidence)


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


def _builder_artifact(stdout, build):
    """Select only the reviewed crate's Linux library test executable."""
    binaries = []
    for line in stdout.splitlines():
        row = json.loads(line)
        if (row.get("reason") == "compiler-artifact" and
                row.get("manifest_path") ==
                "/reviewed/crates/learning-backup/Cargo.toml" and
                row.get("target", {}).get("name") == "learning_backup" and
                row.get("target", {}).get("kind") == ["lib"] and
                row.get("profile", {}).get("test") is True):
            binaries.append(row.get("executable"))
    require(len(binaries) == 1 and type(binaries[0]) is str and
            re.fullmatch(r"/target/debug/deps/learning_backup-[0-9a-f]+",
                         binaries[0]),
            "exact Linux bound probe test executable absent")
    binary = build / "debug" / "deps" / Path(binaries[0]).name
    _trusted_path(binary, file=True)
    meta = os.lstat(binary)
    require(stat.S_IMODE(meta.st_mode) & 0o111 != 0 and meta.st_nlink == 1 and
            binary == binary.resolve(strict=True),
            "bound probe binary unsafe")
    return binary


def _run_bounded(command, *, cwd=None, env=None, timeout=60, limit=16 * 1024 * 1024):
    """Bound subprocess output in private temporary files, never in a pipe."""
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        process = subprocess.run(command, cwd=cwd, env=env, stdout=stdout,
                                 stderr=stderr, timeout=timeout, check=False)
        require(stdout.tell() <= limit and stderr.tell() <= limit,
                "bound probe command output exceeded limit")
        stdout.seek(0)
        stderr.seek(0)
        return subprocess.CompletedProcess(command, process.returncode,
                                           stdout.read(), stderr.read())


def _probe_docker(args, *, timeout=60):
    return _run_bounded(
        ["/usr/bin/docker", *args], timeout=timeout,
        env={"PATH": "/usr/bin:/bin", "HOME": "/root",
             "DOCKER_HOST": "unix:///var/run/docker.sock"})


def _builder_identity(batch, stage):
    require(_canonical_v4(batch.name) and stage in
            ("preflight", "live"), "bound builder batch identity invalid")
    return (f"knowweave-c4-bound-{batch.name}-{stage}",
            f"com.knowweave.bound-probe.batch={batch.name}")


def _builder_container_ids(label):
    row = _probe_docker(["container", "ls", "-aq", "--no-trunc",
                         "--filter", f"label={label}"])
    require(row.returncode == 0, "bound builder inventory failed")
    ids = row.stdout.decode("ascii").splitlines()
    require(all(HEX64.fullmatch(value) for value in ids),
            "bound builder inventory invalid")
    return ids


def _cleanup_builder(batch, stage):
    name, label = _builder_identity(batch, stage)
    ids = _builder_container_ids(label)
    require(len(ids) <= 1, "extra bound builder container")
    if not ids:
        return
    row = _probe_docker(["container", "inspect", ids[0]])
    require(row.returncode == 0, "bound builder inspect failed")
    facts = json.loads(row.stdout)
    require(type(facts) is list and len(facts) == 1 and
            facts[0].get("Id") == ids[0] and
            facts[0].get("Name") == "/" + name and
            facts[0].get("Image") == BUILDER_IMAGE_ID and
            facts[0].get("Config", {}).get("Labels", {}).get(
                "com.knowweave.bound-probe.batch") == batch.name,
            "bound builder cleanup identity differs")
    removed = _probe_docker(["container", "rm", "-f", ids[0]])
    require(removed.returncode == 0 and not _builder_container_ids(label),
            "bound builder cleanup unconfirmed")


def _compile_bound_probe(source, batch, build_name, birth_sha256):
    require(type(birth_sha256) is str and HEX64.fullmatch(birth_sha256),
            "bound probe compile digest invalid")
    build = batch / build_name
    _private_dir(build)
    stage = "preflight" if build_name == "probe-preflight-build" else "live"
    require(build_name in ("probe-preflight-build", "probe-live-build"),
            "bound probe build stage invalid")
    name, label = _builder_identity(batch, stage)
    require(not _builder_container_ids(label),
            "prior bound builder container remains")
    command = ["/usr/bin/docker", "run", "--rm", "--pull", "never",
               "--name", name, "--label", label,
               "--network", "none", "--cap-drop", "ALL",
               "--security-opt", "no-new-privileges", "--user", "0:0",
               "--workdir", "/reviewed", "--tmpfs", "/tmp:rw,nosuid,nodev,size=1g",
               "--mount", f"type=bind,src={source},dst=/reviewed,readonly",
               "--mount", f"type=bind,src={build},dst=/target",
               "--env", "CARGO_TARGET_DIR=/target",
               "--env", "CARGO_NET_OFFLINE=true",
               "--env", "RUSTUP_AUTO_INSTALL=0",
               "--env", "CARGO_TERM_COLOR=never",
               "--env", f"KNOWWEAVE_C4_TARGET_BIRTH_SHA256={birth_sha256}",
               "--entrypoint", "/bin/sh", BUILDER_IMAGE_ID, "-ec",
               "cargo test --locked --offline -p learning-backup --lib --no-run --message-format=json"]
    try:
        process = _run_bounded(
            command, timeout=7200,
            env={"PATH": "/usr/bin:/bin", "HOME": "/root",
                 "DOCKER_HOST": "unix:///var/run/docker.sock"})
    finally:
        _cleanup_builder(batch, stage)
    require(process.returncode == 0,
            "offline pinned builder failed")
    binary = _builder_artifact(process.stdout, build)
    return binary, _file_digest(binary)


def _preflight_probe_builder(source, batch, *, guard=False, session=False):
    """Prove the pinned offline Linux toolchain is ready before PG birth."""
    _trusted_path(Path("/usr/bin/docker"), file=True)
    image = _probe_docker(["image", "inspect", BUILDER_IMAGE_ID,
                           "--format", "{{.Id}}"])
    require(image.returncode == 0 and image.stdout ==
            (BUILDER_IMAGE_ID + "\n").encode(),
            "pinned offline builder image unavailable")
    binary, placeholder_sha = _compile_bound_probe(
        source, batch, "probe-preflight-build", "0" * 64)
    listing = _run_bounded(
        [str(binary), "--list"], cwd=source,
        env={"PATH": "/usr/bin:/bin", "HOME": "/root"},
        timeout=60)
    expected = (b"restore_preflight::target_binding::tests::"
                b"live_read_only_bound_target_probe: test")
    if session:
        expected = (b"restore_preflight::target_binding::tests::"
                    b"live_read_only_sql_session_binding: test")
    elif guard:
        expected = (b"restore_preflight::target_binding::tests::"
                    b"live_read_only_bound_target_guard: test")
    if guard or session:
        require(listing.stdout.splitlines().count(expected) == 1,
                "exact Linux read-only test absent")
    require(listing.returncode == 0 and expected in listing.stdout and
            _file_digest(binary) == placeholder_sha,
            "host cannot execute pinned builder test binary")
    return {"builder_image_id": BUILDER_IMAGE_ID,
            "placeholder_binary_sha256": placeholder_sha,
            "host_test_listing_confirmed": True}


def _run_bound_probe(source, batch, target, database, birth_sha256, *, guard=False,
                     session=False):
    """Rebuild with sealed birth digest, then execute on the Linux host."""
    require(type(birth_sha256) is str and HEX64.fullmatch(birth_sha256),
            "sealed birth digest required for bound probe")
    binary, binary_sha = _compile_bound_probe(
        source, batch, "probe-live-build", birth_sha256)
    test_name = ("restore_preflight::target_binding::tests::"
                 "live_read_only_bound_target_probe")
    if session:
        test_name = ("restore_preflight::target_binding::tests::"
                     "live_read_only_sql_session_binding")
    elif guard:
        test_name = ("restore_preflight::target_binding::tests::"
                     "live_read_only_bound_target_guard")
    env = {"HOME": "/root", "PATH": "/usr/bin:/bin",
           "KNOWWEAVE_C4_PROBE_DESTINATION_ROOT": str(target / "destination"),
           "KNOWWEAVE_C4_PROBE_CONTROL_ROOT": str(target / "control"),
           "KNOWWEAVE_C4_PROBE_ASSET_ROOT": str(target / "assets"),
           "KNOWWEAVE_C4_PROBE_EXPECTED_DATABASE": database}
    require(_file_digest(binary) == binary_sha,
            "bound probe binary changed before execution")
    process = _run_bounded(
        [str(binary), test_name, "--exact", "--ignored", "--nocapture"],
        cwd=source, env=env, timeout=180)
    output = process.stdout + b"\n" + process.stderr
    marker = ("SQL_SESSION_BINDING_READ_ONLY_PG18_PASSED_NOT_RESTORE" if session else
              "BOUND_TARGET_GUARD_READ_ONLY_PG18_PASSED_NOT_RESTORE" if guard else
              "BOUND_TARGET_READ_ONLY_PG18_PASSED_NOT_RESTORE")
    if guard or session:
        marker_line = (rb"(?m)^(?:test " + re.escape(test_name.encode()) +
                       rb" \.\.\. )?" + marker.encode() + rb"\r?$")
        require(len(re.findall(marker_line, output)) == 1 and
                output.count(marker.encode()) == 1 and
                output.splitlines().count(b"running 1 test") == 1 and
                len(re.findall(rb"(?m)^test result: ok\. 1 passed; 0 failed; 0 ignored;", output)) == 1 and
                ("test " + test_name + " ... ").encode() in output,
                "exact one-test read-only success absent")
    require(_file_digest(binary) == binary_sha and process.returncode == 0 and
            marker.encode() in output and
            b"test result: ok. 1 passed; 0 failed; 0 ignored;" in output,
            "read-only bound probe failed")
    return {"state": marker,
            "birth_sha256": birth_sha256, "binary_sha256": binary_sha,
            "builder_image_id": BUILDER_IMAGE_ID, "exit_code": process.returncode}


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
    bound_probe = getattr(args, "bound_probe", False)
    bound_guard = getattr(args, "bound_guard", False)
    sql_session = getattr(args, "sql_session_binding", False)
    require(sum((bound_probe, bound_guard, sql_session)) <= 1,
            "bound modes are mutually exclusive")
    if sql_session:
        result["status"] = SESSION_FAILED
    elif bound_guard:
        result["status"] = GUARD_FAILED
    elif bound_probe:
        result["status"] = BOUND_FAILED
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
        if bound_probe or bound_guard or sql_session:
            result["stage"] = "offline-builder-preflight"
            options = ({"session": True} if sql_session else
                       {"guard": True} if bound_guard else {})
            key = ("session_toolchain_preflight" if sql_session else
                   "guard_toolchain_preflight" if bound_guard else
                   "probe_toolchain_preflight")
            result[key] = _preflight_probe_builder(source, batch, **options)
            require(source_digest(source, manifest) ==
                    result["source_before_sha256"],
                    "reviewed source changed during builder preflight")
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
        candidate, inspection = pin.inspect_candidate(
            control, args.batch_id, initdb)
        require(type(candidate) is dict and set(candidate) ==
                {"birth_sha256", "inspection_evidence_sha256"} and
                all(type(value) is str and HEX64.fullmatch(value)
                    for value in candidate.values()) and
                candidate["birth_sha256"] == success["birth_sha256"],
                "pin checker differs from sealed birth")
        require(type(inspection) is dict and set(inspection) == {
            "format_version", "state", "batch_id", "control_root",
            "precreation_sha256", "birth_sha256", "creation_state_sha256",
            "issuance_success_sha256", "birth_evidence_sha256", "live"} and
            inspection["format_version"] == 1 and
            inspection["state"] == "PIN_CANDIDATE_NOT_RESTORE_AUTHORITY" and
            inspection["batch_id"] == args.batch_id and
            inspection["control_root"] == str(control) and
            inspection["birth_sha256"] == candidate["birth_sha256"] and
            inspection["precreation_sha256"] == precreation_sha and
            type(inspection["live"]) is dict and
            inspection["live"].get("container_id") == confirmed_id,
            "pin inspection identity differs")
        live_record = inspection["live"]
        for observed_key, hash_key in (
                ("docker_projection", "docker_sha256"),
                ("pg_observation", "pg_sha256"),
                ("issuer_pg_observation", "issuer_pg_sha256")):
            require(type(live_record.get(observed_key)) is dict and
                    live_record.get(hash_key) == digest(
                        _json_bytes(live_record[observed_key])),
                    "pin inspection observation digest differs")
        inspection_bytes = _json_bytes(inspection)
        require(len(inspection_bytes) <= MAX_INSPECTION and
                digest(inspection_bytes) ==
                candidate["inspection_evidence_sha256"],
                "pin inspection payload digest differs")
        inspection_path = evidence / "pin-inspection.json"
        result["stage"] = "durable-inspection-evidence"
        _private_write(inspection_path, inspection_bytes)
        require(acceptance._private_read_diagnostic(
                    inspection_path, limit=MAX_INSPECTION) == inspection_bytes,
                "pin inspection record differs after write")
        result["stage"] = "source-reinspection"
        result["source_after_sha256"] = source_digest(source, manifest)
        require(result["source_after_sha256"] ==
                result["source_before_sha256"],
                "reviewed source changed during run")
        if bound_probe or bound_guard or sql_session:
            result["stage"] = ("read-only-sql-session" if sql_session else
                               "read-only-bound-guard" if bound_guard else
                               "read-only-bound-probe")
            key = ("sql_session_binding" if sql_session else
                   "bound_guard" if bound_guard else "bound_probe")
            result[key] = _run_bound_probe(
                source, batch, target, identity["database"],
                success["birth_sha256"], **options)
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
        require(acceptance._private_read_diagnostic(
                    inspection_path, limit=MAX_INSPECTION) == inspection_bytes,
                "pin inspection record changed during stop")
        result.update(candidate)
        result["inspection_record_sha256"] = digest(inspection_bytes)
        result["inspection_record_file"] = inspection_path.name
        result["target_condition"] = "CLEAN_STOPPED_QUARANTINED_NOT_RESTORE"
        result["status"] = (SESSION_PASSED if sql_session else
                            GUARD_PASSED if bound_guard else
                            BOUND_PASSED if bound_probe else PASSED)
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
    _publish_result(evidence, payload)
    summary = {"status": result["status"], "result_sha256": digest(payload),
               "evidence": str(evidence), "not_restore": True}
    if result["status"] in (PASSED, BOUND_PASSED, GUARD_PASSED, SESSION_PASSED):
        summary.update(birth_sha256=result["birth_sha256"],
                       inspection_evidence_sha256=result[
                           "inspection_evidence_sha256"])
    print(json.dumps(summary, sort_keys=True), flush=True)
    return (0 if result["status"] in (PASSED, BOUND_PASSED, GUARD_PASSED,
                                      SESSION_PASSED) else 1), result


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
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--bound-probe", action="store_true",
                        help="opt-in pinned offline builder preflight and read-only Rust bound probe before exact PG stop")
    modes.add_argument("--bound-guard", action="store_true",
                       help="opt-in internal guard lock-lifetime check on a new isolated PG18 target before exact stop")
    modes.add_argument("--sql-session-binding", action="store_true",
                       help="opt-in root-private SQLx session proof on a new isolated PG18 target before exact stop")
    try:
        args = parser.parse_args(argv)
    except SystemExit as error:
        if error.code == 0:
            return 0
        print("PIN_CANDIDATE_ACCEPTANCE_ADMISSION_REJECTED", flush=True)
        return 1
    try:
        return run(args)
    except BaseException:
        print("PIN_CANDIDATE_ACCEPTANCE_ADMISSION_REJECTED", flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
