#!/usr/bin/env python3
"""Root-only, single-host acceptance of a NEW clean P0-C4 pin candidate.

No CompleteBackup, restore, ACL mutation, build pin, or service admission occurs.
The isolated PG18 volume remains quarantined after its exact container is stopped.
"""

import argparse
import contextlib
import hashlib
import ipaddress
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
import time
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
CLONE_FAILED = "SAME_ID_WRONG_ENDPOINT_FAILED_QUARANTINED_NOT_RESTORE_NOT_PIN"
CLONE_PASSED = "SAME_ID_WRONG_ENDPOINT_REJECTED_READ_ONLY_NOT_RESTORE"
CLONE_PASSFILE = "/run/secrets/replication.pgpass"
CLONE_DATA = "/var/lib/postgresql/18/docker"
CLONE_RUNTIME_ROOT = Path("/run")
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


def _admit_clone_pair(provisioner, primary, primary_subnet, clone,
                      clone_subnet, before):
    """Admit both new projects against the same precreation daemon snapshot."""
    require(all(primary[key] != clone[key] for key in
                ("project", "network", "volume", "database")),
            "clone and primary identities must be distinct")
    try:
        left = ipaddress.ip_network(primary_subnet, strict=True)
        right = ipaddress.ip_network(clone_subnet, strict=True)
    except ValueError as error:
        raise ValueError("clone subnet invalid") from error
    require(left.version == right.version == 4 and not left.overlaps(right),
            "clone and primary subnets overlap")
    provisioner.admit_fresh(primary, primary_subnet, before)
    provisioner.admit_fresh(clone, clone_subnet, before)
    return True


def _effective_replication_hba(rows):
    """Evaluate the first HBA record that could accept postgres at IPv4 loopback."""
    require(type(rows) is list, "physical replication HBA rows unavailable")
    previous = 0
    for rule in rows:
        require(type(rule) is dict and type(rule.get("rule_number")) is int and
                rule["rule_number"] > previous and rule.get("error") is None,
                "physical replication HBA order or parse differs")
        previous = rule["rule_number"]
        if rule.get("type") not in ("host", "hostssl", "hostnossl") or \
                "replication" not in (rule.get("database") or []):
            continue
        users = rule.get("user_name")
        require(type(users) is list and all(type(u) is str for u in users),
                "physical replication HBA user unreadable")
        # Group, regex and @file membership cannot be disproved from this view.
        if not any(u in ("all", "postgres") or u.startswith(("+", "/", "@"))
                   for u in users):
            continue
        address, mask = rule.get("address"), rule.get("netmask")
        try:
            network = ipaddress.ip_network((address, mask), strict=False)
        except (ValueError, TypeError):
            # Hostnames and special HBA addresses may match loopback.
            network = None
        if network is not None and ipaddress.ip_address("127.0.0.1") not in network:
            continue
        require(rule.get("type") == "host" and
                rule.get("database") == ["replication"] and
                users in (["postgres"], ["all"]) and
                address == "127.0.0.1" and mask == "255.255.255.255" and
                not rule.get("options") and
                rule.get("auth_method") in ("trust", "scram-sha-256"),
                "first effective physical replication HBA rule is not exact loopback")
        return rule["auth_method"]
    raise ValueError("physical replication HBA rule unavailable")


def _check_replication_contract(container_id):
    """Admit only the first effective exact loopback physical-replication rule."""
    require(type(container_id) is str and HEX64.fullmatch(container_id),
            "exact primary container required")
    sql = """SELECT CASE WHEN current_setting('server_version_num')::int >= 180000
AND current_setting('server_version_num')::int < 190000
AND current_setting('wal_level') IN ('replica', 'logical')
AND current_setting('max_wal_senders')::int >= 2
AND EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'postgres'
            AND rolsuper AND rolreplication)
AND pg_catalog.pg_conf_load_time() >=
    (pg_catalog.pg_stat_file(current_setting('hba_file'))).modification
AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_hba_file_rules
                WHERE error IS NOT NULL OR file_name IS NULL
                   OR file_name <> current_setting('hba_file'))
THEN (SELECT COALESCE(json_agg(row_to_json(r) ORDER BY r.rule_number),
                      '[]'::json)::text
      FROM pg_catalog.pg_hba_file_rules r
      WHERE r.type IN ('host', 'hostssl', 'hostnossl')
        AND r.database @> ARRAY['replication']::text[])
ELSE 'REJECT' END;"""
    row = _probe_docker(["exec", "--user", "postgres", container_id,
                         "psql", "-XAt", "-v", "ON_ERROR_STOP=1",
                         "--dbname", "postgres", "-c", sql])
    require(row.returncode == 0 and 0 < len(row.stdout) <= 1024 * 1024 and
            row.stdout != b"REJECT\n",
            "pinned PG18 replication permission or HBA contract unverified")
    try:
        return _effective_replication_hba(json.loads(row.stdout))
    except (ValueError, TypeError, json.JSONDecodeError) as error:
        raise ValueError("pinned PG18 physical replication HBA unverified") from error


def _verify_clone_helper(facts, helper_id, image, primary_id, volume,
                         passfile=None, *, batch_id, uid, gid, image_id,
                         kind):
    """Check the stopped helper before starting any copy or verification."""
    require(kind in ("setup", "copy", "verify") and
            _canonical_v4(batch_id) and
            type(helper_id) is str and HEX64.fullmatch(helper_id) and
            facts.get("Id") == helper_id and
            facts.get("Image") == image_id and
            facts.get("Name") ==
                f"/knowweave-c4-clone-{batch_id}-{kind}" and
            facts.get("Config", {}).get("Image") == image and
            facts.get("HostConfig", {}).get("NetworkMode") ==
            ("container:" + primary_id if kind == "copy" else "none") and
            (facts.get("Config", {}).get("Labels") or {}).get(
                "com.knowweave.clone.batch") == batch_id and
            facts.get("Config", {}).get("User") ==
                ("0:0" if kind == "setup" else f"{uid}:{gid}") and
            facts.get("Config", {}).get("Entrypoint") == ["/bin/sh"] and
            facts.get("HostConfig", {}).get("CapDrop") == ["ALL"] and
            facts.get("HostConfig", {}).get("CapAdd") ==
                (["CHOWN"] if kind == "setup" else None) and
            facts.get("HostConfig", {}).get("SecurityOpt") ==
                ["no-new-privileges"] and
            facts.get("HostConfig", {}).get("Privileged") is False,
            "clone helper identity or network differs")
    mounts = facts.get("Mounts") or []
    expected = {"/var/lib/postgresql": ("volume", volume,
                                         kind != "verify")}
    if passfile:
        expected[CLONE_PASSFILE] = ("bind", str(passfile), False)
    require(len(mounts) == len(expected) and
            {item.get("Destination") for item in mounts} == set(expected),
            "clone helper mount topology differs")
    for item in mounts:
        kind, source, write = expected[item["Destination"]]
        require(item.get("Type") == kind and item.get("RW") is write and
                item.get("Name" if kind == "volume" else "Source") == source,
                "clone helper mount identity differs")
    env = facts.get("Config", {}).get("Env") or []
    require(all(not value.startswith(("PGPASSWORD=", "POSTGRES_PASSWORD="))
                for value in env) and
            (f"PGPASSFILE={CLONE_PASSFILE}" in env if passfile else
             not any(value.startswith("PGPASSFILE=") for value in env)),
            "clone helper secret environment differs")
    return True


def _stop_clone_pair(acceptance, provisioner, primary, primary_id,
                     clone, clone_id):
    """Attempt both exact-ID stops even if the first one fails."""
    outcome = {"confirmed": False, "volume_retained": False,
               "containers": []}
    failures = []
    for identity, container_id in ((clone, clone_id), (primary, primary_id)):
        if not container_id:
            failures.append("MissingExactId")
            continue
        try:
            stopped = acceptance.stop_verified_pg(provisioner, identity,
                                                   container_id)
            require(stopped.get("confirmed") is True and
                    stopped.get("volume_retained") is True,
                    "exact PG stop or volume retention unconfirmed")
            outcome["containers"].append(container_id)
        except BaseException as error:
            failures.append(type(error).__name__)
    outcome["confirmed"] = not failures
    outcome["volume_retained"] = not failures
    if failures:
        outcome["failure_types"] = failures
    return outcome


def _clone_quarantine_evidence(provisioner, primary, primary_id, clone,
                               clone_id):
    live = provisioner.snapshot()
    def stopped(container_id):
        if not container_id:
            return False
        matches = [c for c in live["containers"] if c.get("Id") == container_id]
        return len(matches) == 1 and matches[0].get("State", {}).get(
            "Running") is False
    def retained(identity):
        matches = [v for v in live["volumes"] if
                   v.get("Name") == identity["volume"]]
        return len(matches) == 1 and (matches[0].get("Labels") or {}).get(
            "com.docker.compose.project") == identity["project"]
    return {"primary_stopped": stopped(primary_id),
            "clone_stopped": stopped(clone_id),
            "primary_volume_retained": retained(primary),
            "clone_volume_retained": retained(clone)}


def _require_runtime_tmpfs():
    """Never put a replication passfile on a persistent /run filesystem."""
    _trusted_path(CLONE_RUNTIME_ROOT)
    with Path("/proc/self/mountinfo").open("r", encoding="ascii") as stream:
        entries = [line.strip().split(" - ", 1) for line in stream]
    require(any(len(parts) == 2 and
                len(parts[0].split()) >= 5 and
                parts[0].split()[4] == str(CLONE_RUNTIME_ROOT) and
                parts[1].split()[0] == "tmpfs" for parts in entries),
            "root-private replication passfile requires /run tmpfs")


def _read_clone_secret(path, uid):
    _require_private_dir(path.parent)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC |
                 os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        meta = os.fstat(stream.fileno())
        require(stat.S_ISREG(meta.st_mode) and meta.st_uid == uid and
                stat.S_IMODE(meta.st_mode) == 0o600 and meta.st_nlink == 1 and
                meta.st_size == 65, "new-batch postgres secret metadata differs")
        value = stream.read(66)
    require(re.fullmatch(rb"[0-9a-f]{64}\n", value) is not None,
            "new-batch postgres secret format differs")
    return value[:-1]


@contextlib.contextmanager
def _clone_passfile(batch, secret_path, uid, gid):
    _require_private_dir(batch)
    require(_canonical_v4(batch.name), "fresh clone runtime batch invalid")
    _require_runtime_tmpfs()
    runtime = CLONE_RUNTIME_ROOT / ("knowweave-c4-clone-" + batch.name)
    _private_dir(runtime)
    passfile = runtime / "replication.pgpass"
    try:
        password = _read_clone_secret(secret_path, uid)
        _private_write(passfile,
                       b"127.0.0.1:5432:*:postgres:" + password + b"\n")
        os.chown(passfile, uid, gid)
        require(stat.S_IMODE(os.lstat(passfile).st_mode) == 0o600 and
                os.lstat(passfile).st_uid == uid,
                "replication passfile permissions differ")
        yield passfile
    finally:
        if os.path.lexists(passfile):
            os.unlink(passfile)
            _sync_dir(runtime)
        os.rmdir(runtime)
        _sync_dir(CLONE_RUNTIME_ROOT)


def _clone_backup_command(primary_id, volume, passfile, image, batch_id,
                          uid, gid):
    require(HEX64.fullmatch(primary_id) and _canonical_v4(batch_id),
            "clone helper identity invalid")
    command = ["create", "--pull=never",
            "--name", f"knowweave-c4-clone-{batch_id}-copy",
            "--label", f"com.knowweave.clone.batch={batch_id}",
            "--network", f"container:{primary_id}",
            "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
            "--user", f"{uid}:{gid}",
            "--mount", f"type=volume,src={volume},dst=/var/lib/postgresql,volume-nocopy"]
    if passfile is not None:
        command.extend(["--mount",
                        f"type=bind,src={passfile},dst={CLONE_PASSFILE},readonly",
                        "--env", f"PGPASSFILE={CLONE_PASSFILE}"])
    command.extend(["--entrypoint", "/bin/sh", image, "-ec",
            f"test -d {CLONE_DATA} && test -z \"$(ls -A {CLONE_DATA})\"; "
            f"exec pg_basebackup -D {CLONE_DATA} "
            "-F plain -X stream --no-password -h 127.0.0.1 -p 5432 -U postgres"])
    return command


def _clone_setup_command(volume, image, batch_id, uid, gid):
    return ["create", "--pull=never",
            "--name", f"knowweave-c4-clone-{batch_id}-setup",
            "--label", f"com.knowweave.clone.batch={batch_id}",
            "--network", "none", "--cap-drop", "ALL", "--cap-add", "CHOWN",
            "--security-opt", "no-new-privileges", "--user", "0:0",
            "--mount", f"type=volume,src={volume},dst=/var/lib/postgresql,volume-nocopy",
            "--entrypoint", "/bin/sh", image, "-ec",
            f"mkdir -p {CLONE_DATA}; chown {uid}:{gid} /var/lib/postgresql/18 "
            f"{CLONE_DATA}; chmod 0700 {CLONE_DATA}; "
            f"test -z \"$(ls -A {CLONE_DATA})\""]


def _clone_verify_command(primary_id, volume, image, batch_id, uid, gid):
    return ["create", "--pull=never",
            "--name", f"knowweave-c4-clone-{batch_id}-verify",
            "--label", f"com.knowweave.clone.batch={batch_id}",
            "--network", "none", "--cap-drop", "ALL",
            "--security-opt", "no-new-privileges", "--user", f"{uid}:{gid}",
            "--mount", f"type=volume,src={volume},dst=/var/lib/postgresql,readonly",
            "--entrypoint", "/bin/sh", image, "-ec",
            f"pg_verifybackup {CLONE_DATA}; test ! -e {CLONE_DATA}/standby.signal; "
            f"test ! -e {CLONE_DATA}/recovery.signal; "
            f"test \"$(cat {CLONE_DATA}/PG_VERSION)\" = 18"]


def _run_clone_helper(command, image, primary_id, volume, passfile=None, *,
                      batch_id, uid, gid, image_id, kind):
    name = f"knowweave-c4-clone-{batch_id}-{kind}"
    require(command[:1] == ["create"] and
            any(command[i:i + 2] == ["--name", name]
                for i in range(len(command) - 1)),
            "clone helper creation name differs")
    try:
        created = _probe_docker(command)
        require(created.returncode == 0, "pinned clone helper creation failed")
        helper_id = created.stdout.decode("ascii").strip()
        require(HEX64.fullmatch(helper_id), "clone helper ID invalid")
    except BaseException as error:
        # Docker may have created the container before its CLI failed. Resolve
        # the deterministic name and remove only a fully verified exact ID.
        error.clone_helper_cleanup = "UNCONFIRMED"
        try:
            recovered = _probe_docker(["container", "inspect", name])
            if recovered.returncode == 0:
                facts = json.loads(recovered.stdout)
                require(type(facts) is list and len(facts) == 1,
                        "ambiguous helper inspect shape differs")
                recovered_id = facts[0].get("Id")
                _verify_clone_helper(facts[0], recovered_id, image,
                                     primary_id, volume, passfile,
                                     batch_id=batch_id, uid=uid, gid=gid,
                                     image_id=image_id, kind=kind)
                removed = _probe_docker(["container", "rm", "-f", recovered_id])
                if removed.returncode == 0:
                    error.clone_helper_cleanup = "EXACT_ID_REMOVED"
        except BaseException:
            pass
        raise
    try:
        inspected = _probe_docker(["container", "inspect", helper_id])
        require(inspected.returncode == 0, "clone helper inspect failed")
        facts = json.loads(inspected.stdout)
        require(type(facts) is list and len(facts) == 1,
                "clone helper inspect shape differs")
        _verify_clone_helper(facts[0], helper_id, image, primary_id, volume,
                             passfile, batch_id=batch_id, uid=uid, gid=gid,
                             image_id=image_id, kind=kind)
        started = _probe_docker(["start", "--attach", helper_id], timeout=1200)
        final = _probe_docker(["container", "inspect", helper_id])
        require(final.returncode == 0, "clone helper final inspect failed")
        state = json.loads(final.stdout)
        require(type(state) is list and len(state) == 1 and
                state[0].get("State", {}).get("Running") is False and
                state[0].get("State", {}).get("ExitCode") == 0 and
                started.returncode == 0,
                "physical clone helper failed")
        _verify_clone_helper(state[0], helper_id, image, primary_id, volume,
                             passfile, batch_id=batch_id, uid=uid, gid=gid,
                             image_id=image_id, kind=kind)
    finally:
        try:
            removed = _probe_docker(["container", "rm", "-f", helper_id])
            require(removed.returncode == 0, "clone helper cleanup unconfirmed")
        except BaseException as cleanup_error:
            cleanup_error.clone_helper_cleanup = "UNCONFIRMED"
            raise
    return True


def _create_clone_resources(provisioner, batch, clone, subnet, before):
    provisioner.admit_fresh(clone, subnet, provisioner.snapshot())
    uid, gid = provisioner._postgres_uid()
    require(type(uid) is int and type(gid) is int and uid > 0 and gid > 0,
            "pinned image postgres UID unavailable")
    image = provisioner._inspect("image", [clone["image"]])
    require(type(image) is list and len(image) == 1 and
            type(image[0].get("Id")) is str and
            image[0]["Id"].startswith("sha256:") and
            HEX64.fullmatch(image[0]["Id"][7:]) and
            any(value.endswith("@" + clone["image"].split("@", 1)[1])
                for value in image[0].get("RepoDigests") or []),
            "pinned PG18 clone image unavailable")
    # Compose owns the complete second project. `create` leaves PG stopped;
    # helpers fill its empty no-copy volume before exact-ID start.
    document = {"name": clone["project"],
        "services": {"pg": {"image": clone["image"],
            "pull_policy": "never", "user": f"{uid}:{gid}",
            "command": ["postgres"], "restart": "no",
            "cap_drop": ["ALL"], "security_opt": ["no-new-privileges"],
            "environment": {"PGDATA": CLONE_DATA},
            "volumes": [{"type": "volume", "source": clone["volume"],
                         "target": "/var/lib/postgresql",
                         "volume": {"nocopy": True}}],
            "networks": ["test"]}},
        "networks": {"test": {"name": clone["network"], "internal": True,
            "ipam": {"config": [{"subnet": subnet}]}}},
        "volumes": {clone["volume"]: {"name": clone["volume"]}}}
    compose_path = batch / "clone-compose.json"
    _private_write(compose_path, _json_bytes(document))
    prefix = ["compose", "-f", str(compose_path), "-p", clone["project"]]
    config = _probe_docker(prefix + ["config", "-q"])
    require(config.returncode == 0, "clone Compose configuration invalid")
    precreate = provisioner.snapshot()
    require(precreate["daemon_id"] == before["daemon_id"],
            "Docker daemon changed before clone Compose creation")
    provisioner.admit_fresh(clone, subnet, precreate)
    created = _probe_docker(prefix + ["create", "--no-build", "--pull",
                                     "never", "--no-recreate", "pg"])
    require(created.returncode == 0, "clone Compose project creation failed")
    live = provisioner.snapshot()
    require(live["daemon_id"] == before["daemon_id"],
            "Docker daemon changed during clone Compose creation")
    project_containers = [c for c in live["containers"] if
        (c.get("Config", {}).get("Labels") or {}).get(
            "com.docker.compose.project") == clone["project"]]
    require(len(project_containers) == 1,
            "clone Compose project container count differs")
    network_matches = [n for n in live["networks"] if
                       n.get("Name") == clone["network"]]
    require(len(network_matches) == 1,
            "clone Compose network count differs")
    network_id = network_matches[0].get("Id")
    require(type(network_id) is str and HEX64.fullmatch(network_id),
            "clone Compose network ID invalid")
    networks = network_matches
    volumes = [v for v in live["volumes"] if v.get("Name") == clone["volume"]]
    project_networks = [n for n in live["networks"] if
                        (n.get("Labels") or {}).get(
                            "com.docker.compose.project") == clone["project"]]
    project_volumes = [v for v in live["volumes"] if
                       (v.get("Labels") or {}).get(
                           "com.docker.compose.project") == clone["project"]]
    # Verify the fields without assuming Docker's gateway choice.
    require(len(networks) == len(volumes) == 1 and
            len(project_networks) == len(project_volumes) == 1 and
            project_networks[0] == networks[0] and
            project_volumes[0] == volumes[0] and
            networks[0].get("Name") == clone["network"] and
            networks[0].get("Internal") is True and
            len((networks[0].get("IPAM") or {}).get("Config") or []) == 1 and
            networks[0]["IPAM"]["Config"][0].get("Subnet") == subnet and
            (networks[0].get("Labels") or {}).get(
                "com.docker.compose.project") == clone["project"] and
            (volumes[0].get("Labels") or {}).get(
                "com.docker.compose.project") == clone["project"] and
            volumes[0].get("Driver") == "local" and
            volumes[0].get("Scope") == "local" and
            volumes[0].get("Options") in (None, {}) and
            type(volumes[0].get("Mountpoint")) is str and
            volumes[0]["Mountpoint"].startswith("/") and
            volumes[0]["Name"] not in
                {v.get("Name") for v in before["volumes"]} and
            network_id not in {n.get("Id") for n in before["networks"]},
            "new clone network or volume identity differs")
    pg = project_containers[0]
    require(pg.get("Id") not in {c.get("Id") for c in precreate["containers"]}
            and pg.get("State", {}).get("Status") == "created",
            "clone PG started before physical copy")
    clone_id = _verify_clone_pg(
        pg, clone, None, network_id, volumes[0]["Mountpoint"],
        running=False, image_id=image[0]["Id"], uid=uid, gid=gid)
    return {"network_id": network_id, "volume_name": clone["volume"],
            "volume_mountpoint": volumes[0].get("Mountpoint"),
            "image_id": image[0]["Id"], "container_id": clone_id,
            "postgres_uid": uid, "postgres_gid": gid,
            "compose_project_created": True}


def _copy_primary_volume(provisioner, batch, target, primary_id, clone,
                         resources, auth_method):
    require(resources["volume_name"] == clone["volume"],
            "clone copy destination changed")
    uid, gid = resources["postgres_uid"], resources["postgres_gid"]
    require(type(uid) is int and type(gid) is int and uid > 0 and gid > 0,
            "pinned image postgres UID unavailable")
    setup = _clone_setup_command(clone["volume"], clone["image"],
                                  batch.name, uid, gid)
    _run_clone_helper(setup, clone["image"], primary_id, clone["volume"],
                      batch_id=batch.name, uid=uid, gid=gid,
                      image_id=resources["image_id"], kind="setup")
    require(auth_method in ("trust", "scram-sha-256"),
            "unverified replication authentication")
    passfile_context = (_clone_passfile(
        batch, target / "secrets" / "postgres_password", uid, gid)
        if auth_method == "scram-sha-256" else contextlib.nullcontext(None))
    with passfile_context as passfile:
        command = _clone_backup_command(primary_id, clone["volume"], passfile,
                                         clone["image"], batch.name, uid, gid)
        _run_clone_helper(command, clone["image"], primary_id,
                          clone["volume"], passfile, batch_id=batch.name,
                          uid=uid, gid=gid, image_id=resources["image_id"],
                          kind="copy")
    verify = _clone_verify_command(primary_id, clone["volume"],
                                    clone["image"], batch.name, uid, gid)
    _run_clone_helper(verify, clone["image"], primary_id, clone["volume"],
                      batch_id=batch.name, uid=uid, gid=gid,
                      image_id=resources["image_id"], kind="verify")
    return {"backup_verified": True, "no_standby": True,
            "pgdata": CLONE_DATA, "passfile_removed": True,
            "passfile_state": ("NOT_CREATED" if auth_method == "trust" else
                               "REMOVED_FROM_TMPFS"),
            "postgres_uid": uid, "postgres_gid": gid,
            "replication_auth": ("EXACT_LOOPBACK_TRUST" if auth_method == "trust"
                                 else "EXACT_LOOPBACK_SCRAM_PASSFILE")}


def _verify_clone_pg(facts, clone, primary_id, network_id, mountpoint,
                     *, running, image_id, uid, gid):
    pg_id = facts.get("Id")
    labels = facts.get("Config", {}).get("Labels") or {}
    require(type(pg_id) is str and HEX64.fullmatch(pg_id) and
            pg_id != primary_id and
            facts.get("Name") == "/" + clone["project"] + "-pg-1" and
            labels.get("com.docker.compose.project") == clone["project"] and
            labels.get("com.docker.compose.service") == "pg" and
            facts.get("Config", {}).get("Image") == clone["image"] and
            facts.get("Config", {}).get("Cmd") == ["postgres"] and
            facts.get("Image") == image_id and
            facts.get("Config", {}).get("User") == f"{uid}:{gid}" and
            facts.get("HostConfig", {}).get("NetworkMode") == clone["network"] and
            facts.get("HostConfig", {}).get("CapDrop") == ["ALL"] and
            facts.get("HostConfig", {}).get("SecurityOpt") ==
                ["no-new-privileges"] and
            facts.get("HostConfig", {}).get("Privileged") is False and
            not any((facts.get("HostConfig", {}).get("PortBindings") or
                     {}).values()) and
            facts.get("State", {}).get("Running") is running,
            "copied PG exact identity differs")
    mounts = facts.get("Mounts") or []
    require(len(mounts) == 1 and mounts[0].get("Type") == "volume" and
            mounts[0].get("Name") == clone["volume"] and
            mounts[0].get("Source") == mountpoint and
            mounts[0].get("Destination") == "/var/lib/postgresql" and
            mounts[0].get("RW") is True,
            "copied PG volume mount differs")
    env = facts.get("Config", {}).get("Env") or []
    require("PGDATA=" + CLONE_DATA in env and
            not any(value.startswith(("PGPASSWORD=", "POSTGRES_PASSWORD="))
                    for value in env),
            "copied PG environment differs")
    if running:
        networks = (facts.get("NetworkSettings") or {}).get("Networks") or {}
        require(set(networks) == {clone["network"]} and
                networks[clone["network"]].get("NetworkID") == network_id and
                not any(((facts.get("NetworkSettings") or {}).get("Ports") or
                         {}).values()),
                "copied PG network attachment differs")
    return pg_id


def _start_clone_pg(provisioner, clone, resources, primary_id, birth,
                    database, uid, gid):
    require(type(database) is str and re.fullmatch(
        r"learning_restore_c4_[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-"
        r"[89ab][0-9a-f]{3}-[0-9a-f]{12}", database),
        "dedicated primary database name invalid")
    clone_id = resources["container_id"]
    require(type(clone_id) is str and HEX64.fullmatch(clone_id) and
            resources.get("compose_project_created") is True,
            "copied PG Compose identity unavailable")
    inspected = _probe_docker(["container", "inspect", clone_id])
    require(inspected.returncode == 0, "copied PG inspect failed")
    facts = json.loads(inspected.stdout)
    require(type(facts) is list and len(facts) == 1,
            "copied PG inspect shape differs")
    _verify_clone_pg(facts[0], clone, primary_id, resources["network_id"],
                     resources["volume_mountpoint"], running=False,
                     image_id=resources["image_id"], uid=uid, gid=gid)
    require(facts[0].get("State", {}).get("Status") == "created",
            "copied PG started before backup verified")
    started = _probe_docker(["start", clone_id])
    require(started.returncode == 0 and
            started.stdout == (clone_id + "\n").encode(),
            "copied PG start failed")
    ready = False
    for _ in range(30):
        probe = _probe_docker(["exec", "--user", "postgres", clone_id,
                               "pg_isready", "-h", "127.0.0.1", "-U",
                               "postgres", "-d", database])
        if probe.returncode == 0:
            ready = True
            break
        time.sleep(2)
    require(ready, "copied PG18 did not become ready")
    inspected = _probe_docker(["container", "inspect", clone_id])
    require(inspected.returncode == 0, "live copied PG inspect failed")
    facts = json.loads(inspected.stdout)
    require(type(facts) is list and len(facts) == 1,
            "live copied PG inspect shape differs")
    _verify_clone_pg(facts[0], clone, primary_id, resources["network_id"],
                     resources["volume_mountpoint"], running=True,
                     image_id=resources["image_id"], uid=uid, gid=gid)
    sql = ("SELECT (pg_catalog.pg_control_system()).system_identifier::text "
           "|| '|' || d.oid::text FROM pg_catalog.pg_database d "
           "WHERE d.datname = current_database() AND NOT pg_is_in_recovery() "
           "AND current_setting('server_version_num')::int >= 180000 "
           "AND current_setting('server_version_num')::int < 190000")
    row = _probe_docker(["exec", "--user", "postgres", clone_id, "psql",
                         "-XAt", "-v", "ON_ERROR_STOP=1", "--dbname",
                         database, "-c", sql])
    expected = (str(birth["pg_system_identifier"]) + "|" +
                str(birth["database_oid"]) + "\n").encode()
    require(row.returncode == 0 and row.stdout == expected,
            "copied PG18 physical system or database identity differs")
    return {"container_id": clone_id, "network_id": resources["network_id"],
            "volume_name": clone["volume"], "volume_retained": True,
            "same_system_identifier": True, "same_database_oid": True}


def _prepare_physical_clone(provisioner, batch, target, primary_id, clone,
                            subnet, before, birth, database):
    auth_method = _check_replication_contract(primary_id)
    require(auth_method in ("trust", "scram-sha-256"),
            "replication authentication unverified")
    resources = _create_clone_resources(provisioner, batch, clone, subnet, before)
    copied = _copy_primary_volume(provisioner, batch, target, primary_id,
                                  clone, resources, auth_method)
    require(copied.get("backup_verified") is True and
            copied.get("no_standby") is True and
            copied.get("pgdata") == CLONE_DATA and
            copied.get("passfile_removed") is True and
            copied.get("passfile_state") ==
                ("NOT_CREATED" if auth_method == "trust" else
                 "REMOVED_FROM_TMPFS") and
            copied.get("replication_auth") ==
                ("EXACT_LOOPBACK_TRUST" if auth_method == "trust" else
                 "EXACT_LOOPBACK_SCRAM_PASSFILE") and
            type(copied.get("postgres_uid")) is int and
            type(copied.get("postgres_gid")) is int and
            copied["postgres_uid"] > 0 and copied["postgres_gid"] > 0,
            "physical clone verification incomplete")
    started = _start_clone_pg(provisioner, clone, resources, primary_id,
                              birth, database, copied["postgres_uid"],
                              copied["postgres_gid"])
    return {**resources, **copied, **started}


def _primary_still_pinned(provisioner, acceptance, identity, subnet, before,
                          state, success, target, initdb, started_at):
    observed = acceptance._independent_docker_gate(
        provisioner, identity, subnet, before, state, success, target, initdb)
    require(observed.get("container_id") == success["container_id"],
            "primary exact Docker ID changed during clone")
    live = provisioner.snapshot()
    matches = [c for c in live["containers"] if
               c.get("Id") == success["container_id"]]
    require(len(matches) == 1 and
            matches[0].get("State", {}).get("Running") is True and
            matches[0].get("State", {}).get("StartedAt") == started_at,
            "primary PG restarted during clone")
    return True


def _sealed_primary_started_at(acceptance, target, inspection, success):
    """Read only the issuer birth-time start covered by the pin digest."""
    payload = acceptance._private_read_diagnostic(
        target / "birth-evidence.json", limit=4096)
    require(type(payload) is bytes and
            digest(payload) == inspection.get("birth_evidence_sha256"),
            "sealed primary birth evidence hash differs")
    evidence = acceptance._unique_json(payload)
    require(type(evidence) is dict and
            evidence.get("container_id") == success["container_id"] and
            type(evidence.get("container_started_at")) is str and
            evidence["container_started_at"],
            "sealed primary birth start differs")
    return evidence["container_started_at"]


def _find_owned_clone_pg(provisioner, clone, before):
    """Recover only the one new, fully attributed clone ID for failure stop."""
    live = provisioner.snapshot()
    prior = {c.get("Id") for c in before["containers"]}
    matches = [c for c in live["containers"] if
               (c.get("Config", {}).get("Labels") or {}).get(
                   "com.docker.compose.project") == clone["project"]]
    require(len(matches) <= 1, "extra clone project container")
    if not matches:
        return None
    c = matches[0]
    ident = c.get("Id")
    require(type(ident) is str and HEX64.fullmatch(ident) and ident not in prior and
            (c.get("Config", {}).get("Labels") or {}).get(
                "com.docker.compose.service") == "pg" and
            c.get("Config", {}).get("Image") == clone["image"] and
            len(c.get("Mounts") or []) == 1 and
            c["Mounts"][0].get("Type") == "volume" and
            c["Mounts"][0].get("Name") == clone["volume"] and
            c["Mounts"][0].get("Destination") == "/var/lib/postgresql",
            "clone failure container cannot be attributed")
    return ident


def _run_clone_negative_probe(source, batch, target, primary, clone,
                              birth_sha256, primary_id, clone_id,
                              clone_network_id, clone_subnet):
    """Run exactly the ignored live second-endpoint test with public IDs only."""
    require(all(type(value) is str and HEX64.fullmatch(value) for value in
                (birth_sha256, primary_id, clone_id, clone_network_id)) and
            primary_id != clone_id and
            all(type(clone.get(key)) is str and clone[key] for key in
                ("project", "network", "volume")) and
            primary["project"] != clone["project"] and
            primary["network"] != clone["network"] and
            primary["database"].startswith("learning_restore_c4_") and
            type(clone_subnet) is str and clone_subnet,
            "wrong-endpoint probe identities invalid")
    binary, binary_sha = _compile_bound_probe(
        source, batch, "probe-live-build", birth_sha256)
    test_name = ("restore_preflight::target_binding::tests::"
                 "live_read_only_same_id_wrong_endpoint_negative")
    env = {"HOME": "/root", "PATH": "/usr/bin:/bin",
           "KNOWWEAVE_C4_PROBE_DESTINATION_ROOT": str(target / "destination"),
           "KNOWWEAVE_C4_PROBE_CONTROL_ROOT": str(target / "control"),
           "KNOWWEAVE_C4_PROBE_ASSET_ROOT": str(target / "assets"),
           "KNOWWEAVE_C4_PROBE_EXPECTED_DATABASE": primary["database"],
           "KNOWWEAVE_C4_PRIMARY_CONTAINER_ID": primary_id,
           "KNOWWEAVE_C4_CLONE_CONTAINER_ID": clone_id,
           "KNOWWEAVE_C4_CLONE_NETWORK_ID": clone_network_id,
           "KNOWWEAVE_C4_CLONE_NETWORK_NAME": clone["network"],
           "KNOWWEAVE_C4_CLONE_PROJECT": clone["project"],
           "KNOWWEAVE_C4_CLONE_VOLUME_NAME": clone["volume"],
           "KNOWWEAVE_C4_CLONE_SUBNET": clone_subnet}
    require(_file_digest(binary) == binary_sha,
            "wrong-endpoint binary changed before execution")
    process = _run_bounded(
        [str(binary), test_name, "--exact", "--ignored", "--nocapture"],
        cwd=source, env=env, timeout=240)
    output = process.stdout + b"\n" + process.stderr
    marker = CLONE_PASSED.encode()
    lines = output.splitlines()
    test_prefix = ("test " + test_name + " ... ").encode()
    test_lines = [(index, line) for index, line in enumerate(lines) if
                  line.startswith(b"test ") and b" ... " in line]
    parallel = (len(test_lines) == 1 and test_lines[0][1] ==
                test_prefix + b"ok" and lines.count(marker) == 1 and
                lines.count(b"ok") == 0 and
                lines.index(marker) < test_lines[0][0])
    serial = (len(test_lines) == 1 and test_lines[0][1] ==
              test_prefix + marker and test_lines[0][0] + 1 < len(lines) and
              lines[test_lines[0][0] + 1] == b"ok" and
              lines.count(b"ok") == 1)
    result_lines = [line for line in lines if line.startswith(b"test result:")]
    require(process.returncode == 0 and
            _file_digest(binary) == binary_sha and
            (parallel or serial) and
            output.count(marker) == 1 and
            lines.count(b"running 1 test") == 1 and
            len(result_lines) == 1 and
            re.fullmatch(
                rb"test result: ok\. 1 passed; 0 failed; 0 ignored; "
                rb"0 measured; \d+ filtered out;(?: finished in \d+(?:\.\d+)?s)?",
                result_lines[0]) is not None,
            "exact one-test wrong-endpoint negative absent")
    return {"state": CLONE_PASSED, "birth_sha256": birth_sha256,
            "binary_sha256": binary_sha, "builder_image_id": BUILDER_IMAGE_ID,
            "exit_code": process.returncode, "primary_container_id": primary_id,
            "clone_container_id": clone_id, "clone_network_id": clone_network_id}


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


def _preflight_probe_builder(source, batch, *, guard=False, session=False,
                             clone=False):
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
    if clone:
        expected = (b"restore_preflight::target_binding::tests::"
                    b"live_read_only_same_id_wrong_endpoint_negative: test")
    elif session:
        expected = (b"restore_preflight::target_binding::tests::"
                    b"live_read_only_sql_session_binding: test")
    elif guard:
        expected = (b"restore_preflight::target_binding::tests::"
                    b"live_read_only_bound_target_guard: test")
    if guard or session or clone:
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
    clone_mode = getattr(args, "sql_session_clone_negative", False)
    require(sum((bound_probe, bound_guard, sql_session, clone_mode)) <= 1,
            "bound modes are mutually exclusive")
    if clone_mode:
        result["status"] = CLONE_FAILED
        result["clone_batch_id"] = args.clone_batch_id
        result["clone_subnet"] = args.clone_subnet
    elif sql_session:
        result["status"] = SESSION_FAILED
    elif bound_guard:
        result["status"] = GUARD_FAILED
    elif bound_probe:
        result["status"] = BOUND_FAILED
    provisioner = acceptance = identity = initdb = before = None
    clone_identity = None
    confirmed_id = None
    clone_id = None
    issuer_started = False
    _private_write(evidence / "attempt.json", _json_bytes({
        "state": "PIN_ONLY_ATTEMPT_NOT_RESTORE_AUTHORITY",
        "batch_id": args.batch_id, "archive_sha256": args.archive_sha256,
        "manifest_sha256": args.manifest_sha256,
        "runner_sha256": args.runner_sha256,
        "source_commit": args.source_commit}))
    try:
        if clone_mode:
            require(not os.path.lexists(batch.parent / args.clone_batch_id),
                    "clone identity already used by an acceptance batch")
        (provisioner, acceptance, pin, prepare, initdb,
         result["source_before_sha256"]) = extract_and_load(
             manifest, package, batch)
        if bound_probe or bound_guard or sql_session or clone_mode:
            result["stage"] = "offline-builder-preflight"
            options = ({"clone": True} if clone_mode else
                       {"session": True} if sql_session else
                       {"guard": True} if bound_guard else {})
            key = ("clone_toolchain_preflight" if clone_mode else
                   "session_toolchain_preflight" if sql_session else
                   "guard_toolchain_preflight" if bound_guard else
                   "probe_toolchain_preflight")
            result[key] = _preflight_probe_builder(source, batch, **options)
            require(source_digest(source, manifest) ==
                    result["source_before_sha256"],
                    "reviewed source changed during builder preflight")
        identity = provisioner.identity_for(args.batch_id)
        result.update(project=identity["project"], volume=identity["volume"])
        if clone_mode:
            clone_identity = provisioner.identity_for(args.clone_batch_id)
            result.update(clone_project=clone_identity["project"],
                          clone_volume=clone_identity["volume"])
        result["stage"] = "fresh-admission"
        before = provisioner.snapshot()
        if clone_mode:
            _admit_clone_pair(provisioner, identity, args.subnet,
                              clone_identity, args.clone_subnet, before)
        else:
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
        if clone_mode:
            primary_started_at = _sealed_primary_started_at(
                acceptance, target, inspection, success)
            _primary_still_pinned(provisioner, acceptance, identity,
                                  args.subnet, before, state, success,
                                  target, initdb, primary_started_at)
        result["stage"] = "source-reinspection"
        result["source_after_sha256"] = source_digest(source, manifest)
        require(result["source_after_sha256"] ==
                result["source_before_sha256"],
                "reviewed source changed during run")
        if clone_mode:
            result["stage"] = "physical-clone-preparation"
            _primary_still_pinned(provisioner, acceptance, identity,
                                  args.subnet, before, state, success,
                                  target, initdb, primary_started_at)
            result["clone"] = _prepare_physical_clone(
                provisioner, batch, target, confirmed_id, clone_identity,
                args.clone_subnet, before, birth, identity["database"])
            clone_id = result["clone"]["container_id"]
            require(type(clone_id) is str and HEX64.fullmatch(clone_id) and
                    clone_id != confirmed_id and
                    result["clone"].get("backup_verified") is True and
                    result["clone"].get("no_standby") is True and
                    result["clone"].get("volume_retained") is True,
                    "physical clone preparation evidence incomplete")
            _primary_still_pinned(provisioner, acceptance, identity,
                                  args.subnet, before, state, success,
                                  target, initdb, primary_started_at)
            result["primary_started_at_unchanged"] = True
            result["stage"] = "same-id-wrong-endpoint-read-only"
            result["same_id_wrong_endpoint"] = _run_clone_negative_probe(
                source, batch, target, identity, clone_identity,
                success["birth_sha256"], confirmed_id, clone_id,
                result["clone"]["network_id"], args.clone_subnet)
            require(type(result["same_id_wrong_endpoint"]) is dict and
                    result["same_id_wrong_endpoint"].get("state") ==
                    CLONE_PASSED,
                    "distinct wrong-endpoint negative marker absent")
            _primary_still_pinned(provisioner, acceptance, identity,
                                  args.subnet, before, state, success,
                                  target, initdb, primary_started_at)
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
        if clone_mode:
            result["stop"] = _stop_clone_pair(
                acceptance, provisioner, identity, confirmed_id,
                clone_identity, clone_id)
            result["quarantine"] = _clone_quarantine_evidence(
                provisioner, identity, confirmed_id, clone_identity,
                clone_id)
            require(all(result["quarantine"].values()),
                    "exact clone quarantine evidence differs")
        else:
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
        result["status"] = (CLONE_PASSED if clone_mode else
                            SESSION_PASSED if sql_session else
                            GUARD_PASSED if bound_guard else
                            BOUND_PASSED if bound_probe else PASSED)
    except BaseException as error:
        result["failure_type"] = type(error).__name__
        if getattr(error, "clone_helper_cleanup", None) is not None:
            result["clone_helper_cleanup"] = error.clone_helper_cleanup
        if issuer_started and identity is not None and acceptance is not None:
            try:
                result["issuer_diagnostic"] = acceptance.read_issuer_diagnostic(
                    control / "targets" / args.batch_id, identity, args.batch_id)
            except BaseException:
                result["issuer_diagnostic"] = {"status": "UNAVAILABLE"}
        if provisioner is not None and acceptance is not None and identity is not None:
            try:
                if clone_mode and confirmed_id and clone_identity is not None:
                    if clone_id is None:
                        try:
                            clone_id = _find_owned_clone_pg(provisioner,
                                                            clone_identity, before)
                        except BaseException as lookup_error:
                            result["clone_stop_lookup_failure_type"] = type(
                                lookup_error).__name__
                    result["stop"] = _stop_clone_pair(
                        acceptance, provisioner, identity, confirmed_id,
                        clone_identity, clone_id)
                elif confirmed_id:
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
        if clone_mode and provisioner is not None and identity is not None and \
                clone_identity is not None:
            try:
                result["quarantine"] = _clone_quarantine_evidence(
                    provisioner, identity, confirmed_id, clone_identity,
                    clone_id)
            except BaseException as quarantine_error:
                result["quarantine"] = {
                    "state": "UNVERIFIED",
                    "failure_type": type(quarantine_error).__name__}
        if source.exists():
            try:
                result["source_after_sha256"] = source_digest(source, manifest)
            except BaseException:
                pass
    payload = _json_bytes(result)
    _publish_result(evidence, payload)
    summary = {"status": result["status"], "result_sha256": digest(payload),
               "evidence": str(evidence), "not_restore": True}
    if result["status"] in (PASSED, BOUND_PASSED, GUARD_PASSED,
                            SESSION_PASSED, CLONE_PASSED):
        summary.update(birth_sha256=result["birth_sha256"],
                       inspection_evidence_sha256=result[
                           "inspection_evidence_sha256"])
    print(json.dumps(summary, sort_keys=True), flush=True)
    return (0 if result["status"] in (PASSED, BOUND_PASSED, GUARD_PASSED,
                                      SESSION_PASSED, CLONE_PASSED) else 1), result


def run(args):
    require(os.geteuid() == 0 and _canonical_v4(args.batch_id),
            "root and new UUIDv4 required")
    clone_mode = getattr(args, "sql_session_clone_negative", False)
    if clone_mode:
        require(_canonical_v4(getattr(args, "clone_batch_id", None)) and
                args.clone_batch_id != args.batch_id and
                type(getattr(args, "clone_subnet", None)) is str and args.clone_subnet,
                "second new UUIDv4 and subnet required")
    else:
        require(getattr(args, "clone_batch_id", None) is None and
                getattr(args, "clone_subnet", None) is None,
                "clone options require clone mode")
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
    parser.add_argument("--clone-batch-id")
    parser.add_argument("--clone-subnet")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--bound-probe", action="store_true",
                        help="opt-in pinned offline builder preflight and read-only Rust bound probe before exact PG stop")
    modes.add_argument("--bound-guard", action="store_true",
                       help="opt-in internal guard lock-lifetime check on a new isolated PG18 target before exact stop")
    modes.add_argument("--sql-session-binding", action="store_true",
                       help="opt-in root-private SQLx session proof on a new isolated PG18 target before exact stop")
    modes.add_argument("--sql-session-clone-negative", action="store_true",
                       help="opt-in two-project physical clone preparation for a distinct read-only wrong-endpoint gate")
    try:
        args = parser.parse_args(argv)
    except SystemExit as error:
        if error.code == 0:
            return 0
        print("PIN_CANDIDATE_ACCEPTANCE_ADMISSION_REJECTED", flush=True)
        return 1
    try:
        if args.sql_session_clone_negative:
            require(_canonical_v4(args.clone_batch_id) and
                    args.clone_batch_id != args.batch_id and
                    type(args.clone_subnet) is str and args.clone_subnet,
                    "second new UUIDv4 and subnet required")
        else:
            require(args.clone_batch_id is None and args.clone_subnet is None,
                    "clone options require clone mode")
        return run(args)
    except BaseException:
        print("PIN_CANDIDATE_ACCEPTANCE_ADMISSION_REJECTED", flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
