#!/usr/bin/env python3
"""Root-only single-host PG18 acceptance of a NEW C4 target birth issuer.

This runner creates a fresh UUIDv4 Compose project and volume. It never opens
a CompleteBackup or restores data. A passing run deliberately makes its new
target DIRTY_UNUSABLE, stops only its inspected container, and retains its
volume. A birth produced here is NOT a reviewed-build pin or restore authority.
Linux/Docker/PG18 behavior is unverified until this exact reviewed runner is
executed on an isolated host with a separately authorized source archive.
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
import uuid
import zipfile


BASE = Path("/var/lib/knowweave-c4")
ENTRY = "scripts/p0c4_restore_birth_acceptance.py"
PROVISIONER = "scripts/p0c4_restore_target.py"
ISSUER = "scripts/p0c4_restore_target_birth.py"
INITDB = "deploy/p0c4_restore_initdb.sh"
REQUIRED = {ENTRY, PROVISIONER, ISSUER, INITDB}
HEX64 = re.compile(r"[0-9a-f]{64}\Z")
HEX40 = re.compile(r"[0-9a-f]{40}\Z")
MAX_ARCHIVE = 32 * 1024 * 1024
MAX_EXPANDED = 64 * 1024 * 1024
BIRTH_KEYS = ("format_version", "project_name", "pg_volume_name",
              "database_name", "database_oid", "pg_system_identifier",
              "control_dev", "control_ino", "asset_dev", "asset_ino",
              "creation_nonce", "template_database", "baseline_cast_count",
              "public_schema")
PUBLIC_KEYS = ("owner_oid", "owner_name", "acl_is_null", "acl")
ACL_KEYS = ("grantor_oid", "grantee_oid", "privilege", "grantable")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical_v4(value):
    try:
        parsed = uuid.UUID(value)
    except (ValueError, AttributeError, TypeError):
        return False
    return parsed.version == 4 and str(parsed) == value


def _json_bytes(value):
    return json.dumps(value, ensure_ascii=False,
                      separators=(",", ":")).encode("utf-8")


def _unique_json(data):
    def unique(pairs):
        output = {}
        for key, value in pairs:
            require(key not in output, "duplicate JSON key")
            output[key] = value
        return output
    return json.loads(data, object_pairs_hook=unique)


def _canonical_birth_bytes(birth):
    require(type(birth) is dict and set(birth) == set(BIRTH_KEYS),
            "birth JSON fields differ")
    public = birth["public_schema"]
    require(type(public) is dict and set(public) == set(PUBLIC_KEYS) and
            type(public["owner_oid"]) is int and public["owner_oid"] > 0 and
            type(public["owner_name"]) is str and
            type(public["acl_is_null"]) is bool and
            type(public["acl"]) is list, "birth public schema fields differ")
    acl = []
    for entry in public["acl"]:
        require(type(entry) is dict and set(entry) == set(ACL_KEYS) and
                type(entry["grantor_oid"]) is int and entry["grantor_oid"] > 0 and
                type(entry["grantee_oid"]) is int and entry["grantee_oid"] >= 0 and
                type(entry["privilege"]) is str and
                type(entry["grantable"]) is bool,
                "birth ACL entry fields differ")
        acl.append({key: entry[key] for key in ACL_KEYS})
    order = lambda row: tuple(row[key] for key in ACL_KEYS)
    require(all(order(left) < order(right) for left, right in zip(acl, acl[1:])),
            "birth ACL entries not sorted uniquely")
    canonical_public = {key: (acl if key == "acl" else public[key])
                        for key in PUBLIC_KEYS}
    canonical = {key: (canonical_public if key == "public_schema" else birth[key])
                 for key in BIRTH_KEYS}
    return _json_bytes(canonical)


def validate_candidate_records(identity, batch_id, birth_bytes, state, success, evidence):
    """Independent byte/identity gate after issuer exit 0; no live proof here."""
    require(canonical_v4(batch_id) and type(birth_bytes) is bytes and
            0 < len(birth_bytes) <= 4096, "candidate batch or birth bytes invalid")
    birth = _unique_json(birth_bytes)
    require(_canonical_birth_bytes(birth) == birth_bytes,
            "birth JSON differs from canonical issuer bytes")
    require(type(birth["format_version"]) is int and
            birth["format_version"] == 1 and
            birth["project_name"] == identity["project"] and
            birth["pg_volume_name"] == identity["volume"] and
            birth["database_name"] == identity["database"] and
            canonical_v4(birth["creation_nonce"]) and
            birth["template_database"] == "template0" and
            all(type(birth[key]) is int and birth[key] > 0 for key in
                ("database_oid", "control_dev", "control_ino", "asset_dev",
                 "asset_ino", "baseline_cast_count")) and
            type(birth["pg_system_identifier"]) is str and
            re.fullmatch(r"[1-9][0-9]*", birth["pg_system_identifier"]),
            "birth identity fields differ")
    require(type(state) is dict and set(state) == {
        "state", "batch_id", "project", "database", "network", "subnet",
        "container_id", "network_id", "volume_name", "volume_mountpoint"} and
            state["state"] == "CREATED_QUARANTINED" and
            state["batch_id"] == batch_id and
            state["project"] == identity["project"] and
            state["database"] == identity["database"] and
            state["network"] == identity["network"] and
            state["volume_name"] == identity["volume"] and
            HEX64.fullmatch(state["container_id"]) and
            HEX64.fullmatch(state["network_id"]),
            "creation state differs")
    require(type(success) is dict and set(success) == {
        "format_version", "state", "batch_id", "project_name", "database_name",
        "birth_sha256", "container_id", "network_id", "pg_volume_name",
        "image_id", "volume_mountpoint", "volume_mount_dev", "volume_mount_ino"} and
            type(success["format_version"]) is int and
            success["format_version"] == 1 and
            success["state"] == "BIRTH_ISSUED_NOT_RESTORE_ACCEPTANCE" and
            success["batch_id"] == batch_id and
            success["project_name"] == identity["project"] and
            success["database_name"] == identity["database"] and
            success["birth_sha256"] == digest(birth_bytes) and
            success["container_id"] == state["container_id"] and
            success["network_id"] == state["network_id"] and
            success["pg_volume_name"] == state["volume_name"] and
            success["volume_mountpoint"] == state["volume_mountpoint"] and
            isinstance(success["image_id"], str) and
            HEX64.fullmatch(success["image_id"].removeprefix("sha256:")) and
            type(success["volume_mount_dev"]) is int and
            success["volume_mount_dev"] > 0 and
            type(success["volume_mount_ino"]) is int and
            success["volume_mount_ino"] > 0,
            "issuance success seal differs")
    require(type(evidence) is dict and
            all(evidence.get(key) == value for key, value in {
                "birth_sha256": digest(birth_bytes),
                "container_id": success["container_id"],
                "network_id": success["network_id"],
                "volume_name": success["pg_volume_name"],
                "image_id": success["image_id"],
                "volume_mountpoint": success["volume_mountpoint"],
                "volume_mount_dev": success["volume_mount_dev"],
                "volume_mount_ino": success["volume_mount_ino"],
            }.items()), "root-private birth evidence differs")
    return birth, state, success


def validate_live_docker(identity, subnet, before, live, expected, target, initdb,
                         *, allow_starting=False):
    """Inspect Docker fields directly, in addition to provisioner verification."""
    project = identity["project"]
    require(live.get("daemon_id") == before.get("daemon_id"),
            "Docker daemon changed")
    containers = [c for c in live["containers"] if
                  (c.get("Config", {}).get("Labels") or {}).get(
                      "com.docker.compose.project") == project]
    networks = [n for n in live["networks"] if
                (n.get("Labels") or {}).get("com.docker.compose.project") == project]
    volumes = [v for v in live["volumes"] if
               (v.get("Labels") or {}).get("com.docker.compose.project") == project]
    require(len(containers) == len(networks) == len(volumes) == 1,
            "project Docker object count differs")
    pg, network, volume = containers[0], networks[0], volumes[0]
    require(pg.get("Id") == expected["container_id"] and
            (pg.get("Config", {}).get("Labels") or {}).get(
                "com.docker.compose.service") == "pg" and
            network.get("Id") == expected["network_id"] and
            volume.get("Name") == expected["volume_name"] and
            volume.get("Mountpoint") == expected["volume_mountpoint"] and
            volume.get("Driver") == "local" and volume.get("Scope") == "local" and
            volume.get("Options") in (None, {}) and
            pg.get("Config", {}).get("Image") == identity["image"] and
            pg.get("State", {}).get("Running") is True and
            (allow_starting or
             pg.get("State", {}).get("Health", {}).get("Status") == "healthy"),
            "Docker immutable identity or health differs")
    images = live.get("images") or []
    repo_digests = images[0].get("RepoDigests") if len(images) == 1 else None
    require(len(images) == 1 and images[0].get("Id") == pg.get("Image") and
            type(pg.get("Image")) is str and
            HEX64.fullmatch(pg["Image"].removeprefix("sha256:")) and
            type(repo_digests) is list and
            all(type(value) is str for value in repo_digests) and
            any(value.endswith("@" + identity["image"].split("@", 1)[1])
                for value in repo_digests),
            "PG image digest differs")
    network_configs = (network.get("IPAM") or {}).get("Config") or []
    require(network.get("Name") == identity["network"] and
            network.get("Internal") is True and
            len(network_configs) == 1 and
            network_configs[0].get("Subnet") == subnet and
            set((pg.get("NetworkSettings") or {}).get("Networks") or {}) ==
            {identity["network"]} and
            (pg.get("NetworkSettings") or {}).get("Networks", {}).get(
                identity["network"], {}).get("NetworkID") == network["Id"] and
            (pg.get("HostConfig") or {}).get("NetworkMode") == identity["network"] and
            not any(((pg.get("NetworkSettings") or {}).get("Ports") or {}).values()) and
            not any((pg.get("HostConfig", {}).get("PortBindings") or {}).values()),
            "PG network isolation or published port differs")
    expected_mounts = {
        "/var/lib/postgresql": ("volume", volume["Mountpoint"], True),
        "/docker-entrypoint-initdb.d/10-restore.sh": ("bind", str(initdb), False),
        "/run/secrets/postgres_password": (
            "bind", str(target / "secrets" / "postgres_password"), False),
        "/run/secrets/admin_password": (
            "bind", str(target / "secrets" / "admin_password"), False),
    }
    mounts = pg.get("Mounts") or []
    require(len(mounts) == 4 and {m.get("Destination") for m in mounts} ==
            set(expected_mounts), "PG has extra or missing mount")
    for mount in mounts:
        require((mount.get("Type"), mount.get("Source"), mount.get("RW")) ==
                expected_mounts[mount["Destination"]], "PG mount differs")
    data_mount = next(mount for mount in mounts if
                      mount.get("Destination") == "/var/lib/postgresql")
    require(data_mount.get("Name") == identity["volume"],
            "PG named data volume differs")
    return pg, network, volume


def stop_verified_pg(provisioner, identity, container_id):
    """Never stop a guessed, replaced, foreign, or uninspected container."""
    require(isinstance(container_id, str) and HEX64.fullmatch(container_id),
            "verified immutable PG ID required for stop")
    live = provisioner.snapshot()
    owned = [c for c in live["containers"] if c.get("Id") == container_id and
             (c.get("Config", {}).get("Labels") or {}).get("com.docker.compose.project")
             == identity["project"] and
             (c.get("Config", {}).get("Labels") or {}).get("com.docker.compose.service")
             == "pg" and c.get("Config", {}).get("Image") == identity["image"] and
             any(m.get("Name") == identity["volume"] and
                 m.get("Destination") == "/var/lib/postgresql" and
                 m.get("Type") == "volume" for m in c.get("Mounts", []))]
    require(len(owned) == 1, "cannot independently identify own PG container")
    if owned[0].get("State", {}).get("Running") is True:
        provisioner._docker("stop", "--time", "1", container_id)
    final = provisioner.snapshot()
    matches = [c for c in final["containers"] if c.get("Id") == container_id]
    require(len(matches) == 1 and matches[0].get("State", {}).get("Running") is False and
            any(v.get("Name") == identity["volume"] and
                (v.get("Labels") or {}).get("com.docker.compose.project") ==
                identity["project"] for v in final["volumes"]),
            "own PG stop or retained volume unconfirmed")
    return {"confirmed": True, "container_id": container_id,
            "volume_retained": True}


def stop_early_owned_pg(provisioner, identity, target, subnet, before, initdb):
    """Stop a newly observed exact project without trusting a missing state."""
    require(target.name == identity["database"].removeprefix(
        "learning_restore_c4_"), "early cleanup target differs")
    _require_private_dir(target)
    provisioner.admit_fresh(identity, subnet, before)
    live = provisioner.snapshot()
    live["images"] = provisioner._inspect("image", [identity["image"]])
    containers = [item for item in live["containers"] if
                  (item.get("Config", {}).get("Labels") or {}).get(
                      "com.docker.compose.project") == identity["project"]]
    networks = [item for item in live["networks"] if
                (item.get("Labels") or {}).get(
                    "com.docker.compose.project") == identity["project"]]
    volumes = [item for item in live["volumes"] if
               (item.get("Labels") or {}).get(
                   "com.docker.compose.project") == identity["project"]]
    require(len(containers) == len(networks) == len(volumes) == 1,
            "cannot uniquely identify fresh batch Docker objects")
    pg, network, volume = containers[0], networks[0], volumes[0]
    require(volume.get("Name") == identity["volume"] and
            type(pg.get("Id")) is str and HEX64.fullmatch(pg["Id"]) and
            type(network.get("Id")) is str and HEX64.fullmatch(network["Id"]) and
            pg["Id"] not in {item.get("Id") for item in before["containers"]} and
            network["Id"] not in {item.get("Id") for item in before["networks"]} and
            volume.get("Name") not in
            {item.get("Name") for item in before["volumes"]},
            "early cleanup Docker identity predates batch")
    expected = {"container_id": pg["Id"], "network_id": network["Id"],
                "volume_name": volume.get("Name"),
                "volume_mountpoint": volume.get("Mountpoint")}
    has_state = os.path.lexists(target / "state.json")
    if has_state:
        recorded = _unique_json(_private_read(target / "state.json"))
        require(type(recorded) is dict and
                recorded.get("state") == "CREATED_QUARANTINED" and
                recorded.get("batch_id") == target.name and
                recorded.get("project") == identity["project"] and
                recorded.get("database") == identity["database"] and
                recorded.get("network") == identity["network"] and
                recorded.get("subnet") == subnet and
                all(recorded.get(key) == value for key, value in
                    expected.items()), "private creation state differs")
    validate_live_docker(identity, subnet, before, live, expected,
                         target, initdb, allow_starting=True)
    stopped = stop_verified_pg(provisioner, identity, pg["Id"])
    return {**stopped, "basis": ("private-state-and-live-docker" if has_state
                                else "fresh-snapshot-no-state")}


def accept_issuer_process(process, target, identity, batch_id):
    require(process.returncode == 0, "issuer exit nonzero")
    lines = process.stdout.splitlines()
    require(len(lines) == 1, "issuer stdout shape differs")
    summary = _unique_json(lines[0])
    require(type(summary) is dict and
            summary.get("state") == "BIRTH_ISSUED_NOT_RESTORE_ACCEPTANCE",
            "issuer did not report issued birth")
    birth, status, success = read_candidate(target, identity, batch_id)
    require(summary.get("batch_id") == batch_id and
            summary.get("birth_sha256") == success["birth_sha256"] and
            summary.get("issuance_sha256") == digest(_private_read(
                target / "issuance-success.json")),
            "issuer stdout differs from durable success seal")
    return birth, status, success


def _trusted_path(path, *, file=False):
    path = Path(path)
    require(path.is_absolute(), "absolute root-owned path required")
    for item in reversed((path, *path.parents)):
        meta = os.lstat(item)
        expected = stat.S_ISREG if file and item == path else stat.S_ISDIR
        require(expected(meta.st_mode) and meta.st_uid == 0 and
                stat.S_IMODE(meta.st_mode) & 0o022 == 0,
                "untrusted source or control path")


def _sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def _private_dir(path):
    require(not os.path.lexists(path), "batch path already exists")
    path.mkdir(mode=0o700)
    _trusted_path(path)
    require(stat.S_IMODE(os.lstat(path).st_mode) == 0o700,
            "batch directory is not root-private")
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


def _private_read(path, limit=4096):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as stream:
        meta = os.fstat(stream.fileno())
        require(stat.S_ISREG(meta.st_mode) and meta.st_uid == 0 and
                stat.S_IMODE(meta.st_mode) == 0o600 and meta.st_nlink == 1 and
                0 < meta.st_size <= limit, "unsafe private issuance record")
        content = stream.read(limit + 1)
    require(0 < len(content) <= limit, "private issuance record exceeds limit")
    return content


def _safe_member(name):
    require(isinstance(name, str) and name and not name.startswith("/") and
            "\\" not in name and ":" not in name and "\x00" not in name and
            all(part not in ("", ".", "..") for part in name.split("/")) and
            str(PurePosixPath(name)) == name, "unsafe archive member")
    return name.split("/")


def _file_digest(path):
    hashed = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hashed.update(block)
    return hashed.hexdigest()


def verify_archive(path, archive_sha256, manifest_sha256, commit):
    require(all(isinstance(value, str) and pattern.fullmatch(value) for
                value, pattern in ((archive_sha256, HEX64), (manifest_sha256, HEX64),
                                (commit, HEX40))), "exact approved source identity required")
    require(path.is_absolute() and path.is_relative_to(BASE / "incoming"),
            "archive must be in private incoming directory")
    _require_private_dir(BASE / "incoming")
    _trusted_path(path, file=True)
    archive_meta = os.lstat(path)
    require(stat.S_IMODE(archive_meta.st_mode) == 0o400 and
            archive_meta.st_nlink == 1, "archive must be root-private 0400")
    _trusted_path(Path(__file__).resolve(strict=True), file=True)
    runner_meta = os.lstat(Path(__file__))
    require(stat.S_IMODE(runner_meta.st_mode) == 0o500 and
            runner_meta.st_nlink == 1, "installed runner must be root-only 0500")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as stream:
        content = stream.read(MAX_ARCHIVE + 1)
    require(len(content) <= MAX_ARCHIVE and digest(content) == archive_sha256,
            "archive bytes differ")
    with zipfile.ZipFile(io.BytesIO(content)) as archive:
        infos = archive.infolist()
        names = [info.filename for info in infos]
        require(REQUIRED.issubset(names) and len(names) == len(set(names)) and
                "SOURCE_MANIFEST.json" in names and len(names) < 1000 and
                sum(info.file_size for info in infos) <= MAX_EXPANDED,
                "archive inventory differs")
        for info in infos:
            _safe_member(info.filename)
            require(not info.is_dir() and info.flag_bits & 1 == 0 and
                    (info.external_attr >> 16) & 0o170000 == stat.S_IFREG and
                    info.file_size <= MAX_EXPANDED,
                    "archive contains non-regular or encrypted member")
        manifest_bytes = archive.read("SOURCE_MANIFEST.json")
        require(digest(manifest_bytes) == manifest_sha256,
                "manifest bytes differ")
        manifest = _unique_json(manifest_bytes)
        require(_json_bytes({key: manifest[key] for key in sorted(manifest)}) ==
                manifest_bytes and type(manifest) is dict and
                set(manifest) == {"format_version", "commit", "files"} and
                manifest["format_version"] == 1 and manifest["commit"] == commit,
                "manifest identity or canonical bytes differ")
        files = manifest["files"]
        require(type(files) is list and len(files) == len(names) - 1 and
                [item["path"] for item in files] ==
                sorted(set(names) - {"SOURCE_MANIFEST.json"}),
                "manifest file inventory differs")
        for entry in files:
            require(type(entry) is dict and set(entry) ==
                    {"path", "sha256", "size"} and
                    HEX64.fullmatch(entry["sha256"]) and
                    type(entry["size"]) is int and
                    0 <= entry["size"] <= MAX_EXPANDED,
                    "manifest entry differs")
            payload = archive.read(entry["path"])
            require(len(payload) == entry["size"] and
                    digest(payload) == entry["sha256"],
                    "reviewed source member bytes differ")
        expected_runner = next(entry["sha256"] for entry in files if
                               entry["path"] == ENTRY)
        require(_file_digest(Path(__file__)) == expected_runner,
                "installed birth runner differs from approved source")
    return manifest, content


def extract_verified(content, manifest, source):
    _private_dir(source)
    with zipfile.ZipFile(io.BytesIO(content)) as archive:
        for entry in manifest["files"]:
            target = source.joinpath(*_safe_member(entry["path"]))
            stack = []
            parent = target.parent
            while parent != source and not parent.exists():
                stack.append(parent)
                parent = parent.parent
            for directory in reversed(stack):
                _private_dir(directory)
            _private_write(target, archive.read(entry["path"]), 0o400)
    return source_digest(source, manifest)


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


def _load_reviewed(source):
    """Bind issuer's import to the same extracted and hashed provisioner."""
    previous = sys.modules.get("p0c4_restore_target")
    try:
        provisioner_spec = importlib.util.spec_from_file_location(
            "p0c4_restore_target", source / PROVISIONER)
        provisioner = importlib.util.module_from_spec(provisioner_spec)
        sys.modules["p0c4_restore_target"] = provisioner
        provisioner_spec.loader.exec_module(provisioner)
        issuer_spec = importlib.util.spec_from_file_location(
            "p0c4_restore_target_birth_reviewed", source / ISSUER)
        issuer = importlib.util.module_from_spec(issuer_spec)
        issuer_spec.loader.exec_module(issuer)
        require(issuer.target_provisioner is provisioner,
                "issuer imported a different provisioner")
        return provisioner, issuer
    finally:
        if previous is None:
            sys.modules.pop("p0c4_restore_target", None)
        else:
            sys.modules["p0c4_restore_target"] = previous


def read_candidate(target, identity, batch_id):
    require(target.name == batch_id and canonical_v4(batch_id),
            "target directory is not the new batch")
    _trusted_path(target)
    require(stat.S_IMODE(os.lstat(target).st_mode) == 0o700,
            "target must remain private")
    require(not os.path.lexists(target / "failure.json"),
            "issuer failure evidence present")
    destination, control, assets = (target / name for name in
                                    ("destination", "control", "assets"))
    for directory in (destination, control, assets):
        _trusted_path(directory)
        require(stat.S_IMODE(os.lstat(directory).st_mode) == 0o700,
                "restore root not private")
    require(not any(destination.iterdir()) and not any(assets.iterdir()) and
            [p.name for p in control.iterdir()] ==
            [identity["database"] + ".birth.json"],
            "new restore roots not clean")
    birth_bytes = _private_read(control / (identity["database"] + ".birth.json"))
    state = _unique_json(_private_read(target / "state.json"))
    success = _unique_json(_private_read(target / "issuance-success.json"))
    evidence = _unique_json(_private_read(target / "birth-evidence.json"))
    birth, state, success = validate_candidate_records(
        identity, batch_id, birth_bytes, state, success, evidence)
    for path, dev, ino in ((control, birth["control_dev"], birth["control_ino"]),
                           (assets, birth["asset_dev"], birth["asset_ino"])):
        meta = os.lstat(path)
        require(meta.st_dev == dev and meta.st_ino == ino,
                "restore root inode changed after birth")
    return birth, state, success


PG_OBSERVATION_SQL = """SELECT pg_catalog.json_build_object(
 'server_version_num',pg_catalog.current_setting('server_version_num')::integer,
 'database_oid',d.oid::bigint,
 'pg_system_identifier',pcs.system_identifier::text,
 'cast_count',(SELECT count(*) FROM pg_catalog.pg_cast),
 'database_owner',pg_catalog.pg_get_userbyid(d.datdba)::text,
 'runtime_create',pg_catalog.has_schema_privilege('learning_runtime',n.oid,'CREATE'),
 'other_sessions',(SELECT count(*) FROM pg_catalog.pg_stat_activity
    WHERE datname=pg_catalog.current_database() AND pid<>pg_catalog.pg_backend_pid()),
 'user_relations',(SELECT count(*) FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace ns ON ns.oid=c.relnamespace
    WHERE ns.nspname NOT IN ('pg_catalog','information_schema')
      AND ns.nspname NOT LIKE 'pg_toast%' AND ns.nspname NOT LIKE 'pg_temp%'),
 'public_collations',(SELECT count(*) FROM pg_catalog.pg_collation c
    WHERE c.collnamespace=n.oid),
 'default_acls',(SELECT count(*) FROM pg_catalog.pg_default_acl),
 'public_schema',pg_catalog.json_build_object(
    'owner_oid',n.nspowner::bigint,
    'owner_name',pg_catalog.pg_get_userbyid(n.nspowner)::text,
    'acl_is_null',n.nspacl IS NULL,
    'acl',(SELECT COALESCE(pg_catalog.json_agg(pg_catalog.json_build_object(
       'grantor_oid',a.grantor::bigint,'grantee_oid',a.grantee::bigint,
       'privilege',a.privilege_type::text,'grantable',a.is_grantable)), '[]'::json)
       FROM pg_catalog.aclexplode(COALESCE(n.nspacl,
         pg_catalog.acldefault('n',n.nspowner))) a)))
 FROM pg_catalog.pg_database d CROSS JOIN pg_catalog.pg_control_system() pcs
 JOIN pg_catalog.pg_namespace n ON n.nspname='public'
 WHERE d.datname=pg_catalog.current_database();"""


def observe_pg(provisioner, identity, container_id):
    require(HEX64.fullmatch(container_id), "verified PG container ID required")
    output = provisioner._docker(
        "exec", "--user", "postgres", container_id, "psql", "-XAt",
        "-v", "ON_ERROR_STOP=1", "--dbname", identity["database"],
        "-c", PG_OBSERVATION_SQL)
    require(output.endswith("\n") and len(output.splitlines()) == 1,
            "independent PG observation shape changed")
    facts = _unique_json(output)
    require(type(facts) is dict and set(facts) == {
        "server_version_num", "database_oid", "pg_system_identifier",
        "cast_count", "database_owner", "runtime_create", "other_sessions",
        "user_relations", "public_collations", "default_acls", "public_schema"},
            "independent PG fields differ")
    return facts


def validate_pg_against_birth(facts, birth, issuer_facts):
    require(type(facts["server_version_num"]) is int and
            180000 <= facts["server_version_num"] < 190000 and
            facts["database_oid"] == birth["database_oid"] ==
            issuer_facts["database_oid"] and
            facts["pg_system_identifier"] == birth["pg_system_identifier"] ==
            issuer_facts["pg_system_identifier"] and
            facts["cast_count"] == birth["baseline_cast_count"] ==
            issuer_facts["cast_count"] and
            facts["database_owner"] == "learning_admin" and
            facts["runtime_create"] is False and
            all(type(facts[key]) is int and facts[key] == 0 for key in
                ("other_sessions", "user_relations", "public_collations",
                 "default_acls")),
            "PG18 identity or clean database differs")
    public = facts["public_schema"]
    require(type(public) is dict and set(public) ==
            {"owner_oid", "owner_name", "acl_is_null", "acl"} and
            public["owner_name"] == "pg_database_owner" and
            public["acl_is_null"] is False and
            sorted(public["acl"], key=lambda row: (
                row["grantor_oid"], row["grantee_oid"], row["privilege"],
                row["grantable"])) == birth["public_schema"]["acl"] ==
            sorted(issuer_facts["public_schema"]["acl"], key=lambda row: (
                row["grantor_oid"], row["grantee_oid"], row["privilege"],
                row["grantable"])) and
            public["owner_oid"] == birth["public_schema"]["owner_oid"] and
            public["owner_oid"] == issuer_facts["public_schema"]["owner_oid"],
            "PG18 public schema owner or ACL differs")


def verify_live_mount_inode(mountpoint, success):
    path = Path(mountpoint)
    require(path.is_absolute(), "Docker volume mount must be absolute")
    for ancestor in reversed((path, *path.parents)):
        meta = os.lstat(ancestor)
        require(stat.S_ISDIR(meta.st_mode) and
                stat.S_IMODE(meta.st_mode) & 0o022 == 0 and
                (ancestor == path or meta.st_uid == 0),
                "Docker volume mount ancestor differs")
    meta = os.lstat(path)
    require((meta.st_dev, meta.st_ino) ==
            (success["volume_mount_dev"], success["volume_mount_ino"]),
            "live Docker volume inode differs from issuance")


def _run_issuer(source, control, batch_id, subnet, initdb):
    command = ["/usr/bin/python3", "-B", str(source / ISSUER),
               "--root", str(control), "--batch-id", batch_id,
               "--subnet", subnet, "--initdb", str(initdb)]
    return subprocess.run(command, text=True, capture_output=True,
                          timeout=900, check=False,
                          env={"PATH": "/usr/sbin:/usr/bin:/bin",
                               "DOCKER_HOST": "unix:///var/run/docker.sock"})


def _negative_acl(provisioner, issuer, identity, container_id):
    output = provisioner._docker(
        "exec", "--user", "postgres", container_id, "psql", "-XAt",
        "-v", "ON_ERROR_STOP=1", "--dbname", identity["database"],
        "-c", "GRANT CREATE ON SCHEMA public TO learning_runtime")
    require(output.strip() == "GRANT", "negative ACL grant failed")
    dirty = observe_pg(provisioner, identity, container_id)
    require(dirty["runtime_create"] is True,
            "negative ACL mutation not independently visible")
    rejected = False
    try:
        issuer.probe_pg_facts(identity, container_id)
    except issuer.AdmissionError:
        rejected = True
    require(rejected, "issuer PG fact probe accepted deliberately dirty target")
    return {"runtime_create": True,
            "issuer_probe_rejected": True,
            "pg_observation_sha256": digest(_json_bytes(dirty))}


@contextlib.contextmanager
def _acceptance_lock(root):
    import fcntl  # Linux-only; local tests mock host resource operations.
    _trusted_path(root)
    fd = os.open(root / ".birth-acceptance.lock",
                 os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    try:
        meta = os.fstat(fd)
        require(stat.S_ISREG(meta.st_mode) and meta.st_uid == 0 and
                stat.S_IMODE(meta.st_mode) == 0o600 and meta.st_nlink == 1,
                "unsafe acceptance lock")
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield
    finally:
        os.close(fd)


def _require_private_dir(path):
    _trusted_path(path)
    require(stat.S_IMODE(os.lstat(path).st_mode) == 0o700,
            "root-private 0700 directory required")


def _prepare_batch(root, batch_id):
    _require_private_dir(BASE)
    if not os.path.lexists(root):
        _private_dir(root)
    _require_private_dir(root)
    batches = root / "batches"
    if not os.path.lexists(batches):
        _private_dir(batches)
    _require_private_dir(batches)
    batch = batches / batch_id
    _private_dir(batch)
    for name in ("control", "evidence"):
        _private_dir(batch / name)
    _private_dir(batch / "control" / "targets")
    return batch


def _independent_docker_gate(provisioner, identity, subnet, before,
                             state, success, target, initdb):
    live = provisioner.snapshot()
    live["images"] = provisioner._inspect("image", [identity["image"]])
    pg, network, volume = validate_live_docker(
        identity, subnet, before, live, state, target, initdb)
    verify_live_mount_inode(volume["Mountpoint"], success)
    require(pg["Image"] == success["image_id"] and
            network["Id"] == success["network_id"] and
            volume["Name"] == success["pg_volume_name"],
            "live Docker differs from success seal")
    return {"docker_daemon_id": live["daemon_id"],
            "container_id": pg["Id"], "network_id": network["Id"],
            "image_id": pg["Image"], "volume_name": volume["Name"]}


def _independent_pg_gate(provisioner, issuer, identity, container_id, birth):
    facts = observe_pg(provisioner, identity, container_id)
    issuer_facts = issuer.probe_pg_facts(identity, container_id)
    validate_pg_against_birth(facts, birth, issuer_facts)
    return {"pg_observation_sha256": digest(_json_bytes(facts)),
            "issuer_pg_observation_sha256": digest(_json_bytes(issuer_facts)),
            "pg_observation": facts, "issuer_pg_observation": issuer_facts}


def _write_result(evidence, result):
    payload = _json_bytes(result)
    _private_write(evidence / "result.json", payload)
    return {"status": result["status"], "result_sha256": digest(payload),
            "evidence": str(evidence), "not_restore": True, "not_pin": True}


def _run_batch(args, manifest, package, batch):
    evidence = batch / "evidence"
    source = batch / "source"
    result = {"status": "FAILED_NOT_RESTORE_NOT_PIN", "batch_id": args.batch_id,
              "subnet": args.subnet, "stage": "extract",
              "archive_sha256": args.archive_sha256,
              "manifest_sha256": args.manifest_sha256,
              "source_commit": args.source_commit,
              "runner_sha256": _file_digest(Path(__file__)),
              "source_before_sha256": None, "source_after_sha256": None,
              "target_condition": "NO_TARGET_CREATED",
              "target_reuse_permitted": False,
              "stop": {"confirmed": False}, "gates": [], "failure_type": None}
    provisioner = None
    identity = None
    confirmed_id = None
    issuer_started = False
    try:
        result["source_before_sha256"] = extract_verified(package, manifest, source)
        result["stage"] = "install-reviewed-initdb"
        initdb = batch / "initdb.sh"
        _private_write(initdb, (source / INITDB).read_bytes(), 0o444)
        require(_file_digest(initdb) == next(
            item["sha256"] for item in manifest["files"] if item["path"] == INITDB),
            "installed initdb differs from approved source")
        provisioner, issuer = _load_reviewed(source)
        identity = provisioner.identity_for(args.batch_id)
        result.update({"project": identity["project"],
                       "database": identity["database"],
                       "volume": identity["volume"]})
        result["stage"] = "fresh-admission"
        before = provisioner.snapshot()
        provisioner.admit_fresh(identity, args.subnet, before)
        result["stage"] = "issuer"
        result["target_condition"] = "UNVERIFIED_UNUSABLE"
        issuer_started = True
        process = _run_issuer(source, batch / "control", args.batch_id,
                              args.subnet, initdb)
        target = batch / "control" / "targets" / args.batch_id
        result["stage"] = "issuance-records"
        birth, state, success = accept_issuer_process(
            process, target, identity, args.batch_id)
        require(state["subnet"] == args.subnet,
                "issuer network differs from admitted subnet")
        result["gates"].append("issuer-exit0-and-durable-seal")
        result["stage"] = "independent-docker"
        facts = _independent_docker_gate(
            provisioner, identity, args.subnet, before, state,
            success, target, initdb)
        confirmed_id = facts["container_id"]
        result["gates"].append("independent-docker-and-mount")
        result["stage"] = "independent-pg18"
        facts.update(_independent_pg_gate(
            provisioner, issuer, identity, confirmed_id, birth))
        result["gates"].append("independent-pg18-and-issuer-facts")
        result["stage"] = "pre-negative-evidence"
        _private_write(evidence / "clean-birth-observation.json", _json_bytes({
            "birth_sha256": success["birth_sha256"],
            "issuance_sha256": digest(_private_read(
                target / "issuance-success.json")),
            "batch_id": args.batch_id, "project": identity["project"],
            "database": identity["database"], **facts}))
        result["stage"] = "deliberate-negative-acl"
        result["target_condition"] = "DIRTY_UNUSABLE_NOT_RESTORE_NOT_PIN"
        result["negative_acl"] = _negative_acl(
            provisioner, issuer, identity, confirmed_id)
        result["gates"].append("runtime-create-grant-rejected-by-issuer")
        result["stage"] = "source-and-docker-reinspection"
        result["source_after_sha256"] = source_digest(source, manifest)
        require(result["source_after_sha256"] ==
                result["source_before_sha256"],
                "approved source changed during acceptance")
        after = provisioner.snapshot()
        after["images"] = provisioner._inspect("image", [identity["image"]])
        validate_live_docker(identity, args.subnet, before, after, state,
                             target, initdb)
        result["stage"] = "exact-id-stop"
        result["stop"] = stop_verified_pg(
            provisioner, identity, confirmed_id)
        result["status"] = (
            "BIRTH_ISSUER_SINGLE_HOST_PG18_PASSED_DIRTY_UNUSABLE_NOT_RESTORE_NOT_PIN")
    except BaseException as error:
        result["failure_type"] = type(error).__name__
        if provisioner is not None and identity is not None and confirmed_id:
            try:
                result["stop"] = stop_verified_pg(
                    provisioner, identity, confirmed_id)
            except BaseException as stop_error:
                result["stop"] = {"confirmed": False,
                                  "failure_type": type(stop_error).__name__}
        elif provisioner is not None and identity is not None and issuer_started:
            try:
                result["stop"] = stop_early_owned_pg(
                    provisioner, identity,
                    batch / "control" / "targets" / args.batch_id,
                    args.subnet, before, initdb)
            except BaseException as stop_error:
                result["stop"] = {"confirmed": False,
                                  "failure_type": type(stop_error).__name__,
                                  "early_isolation_unconfirmed": True}
        if source.exists():
            try:
                result["source_after_sha256"] = source_digest(source, manifest)
            except BaseException:
                pass
    summary = _write_result(evidence, result)
    print(json.dumps(summary, sort_keys=True), flush=True)
    return 0 if result["status"].startswith(
        "BIRTH_ISSUER_SINGLE_HOST_PG18_PASSED") else 1


def run(args):
    """Never consumes CompleteBackup; success retains a dirty unusable volume."""
    require(os.geteuid() == 0 and canonical_v4(args.batch_id),
            "root and canonical UUIDv4 batch required")
    archive = args.archive.resolve(strict=True)
    manifest, package = verify_archive(
        archive, args.archive_sha256, args.manifest_sha256,
        args.source_commit)
    root = BASE / "birth-acceptance"
    _require_private_dir(BASE)
    if not os.path.lexists(root):
        _private_dir(root)
    with _acceptance_lock(root):
        batch = _prepare_batch(root, args.batch_id)
        return _run_batch(args, manifest, package, batch)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--archive-sha256", required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--batch-id", required=True)
    parser.add_argument("--subnet", required=True)
    try:
        return run(parser.parse_args())
    except (OSError, ValueError, KeyError, TypeError, zipfile.BadZipFile):
        print("BIRTH_ISSUER_ACCEPTANCE_ADMISSION_REJECTED", flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
