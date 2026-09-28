#!/usr/bin/env python3
"""Root-only, single-project PG18 acceptance of the C4 restore target.

Install this exact file root-owned under /var/lib/knowweave-c4/tools after
reviewing its SHA-256. Supply an exact package_p0c4_task3.py HEAD archive,
manifest hash, commit and an unused RFC1918 subnet. No source/production data,
birth attestation, CompleteBackup or restore is used. Every invocation gets a
new UUIDv4 and a new private evidence directory; a batch is never resumed.
"""

import argparse
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
ENTRY = "scripts/p0c4_restore_target_acceptance.py"
PROVISIONER = "scripts/p0c4_restore_target.py"
INITDB = "deploy/p0c4_restore_initdb.sh"
MAX_ARCHIVE = 32 * 1024 * 1024
MAX_EXPANDED = 64 * 1024 * 1024
NOFOLLOW = getattr(os, "O_NOFOLLOW", 0)  # main requires Linux.
HEX64 = re.compile(r"[0-9a-f]{64}\Z")
HEX40 = re.compile(r"[0-9a-f]{40}\Z")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_digest(path):
    hashed = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hashed.update(chunk)
    return hashed.hexdigest()


def trusted_path(path, *, regular=False):
    path = Path(path).absolute()
    for item in reversed((path, *path.parents)):
        meta = os.lstat(item)
        kind = stat.S_ISREG if regular and item == path else stat.S_ISDIR
        require(kind(meta.st_mode) and meta.st_uid == 0 and
                not stat.S_IMODE(meta.st_mode) & 0o022,
                "root-owned, non-writable, non-symlink path required")


def private_dir(path):
    path = Path(path)
    path.mkdir(mode=0o700)
    trusted_path(path)
    require(stat.S_IMODE(path.stat().st_mode) == 0o700,
            "private directory must be 0700")
    sync_dir(path.parent)


def ensure_private_dir(path):
    if path.exists():
        trusted_path(path)
        require(stat.S_IMODE(path.stat().st_mode) == 0o700,
                "existing control directory must be 0700")
    else:
        private_dir(path)


def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def private_file(path, content, mode=0o600):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | NOFOLLOW, mode)
    with os.fdopen(fd, "wb") as stream:
        stream.write(content)
        stream.flush()
        os.fsync(stream.fileno())
    os.chmod(path, mode)
    sync_dir(path.parent)


def archive_name(name):
    require(isinstance(name, str) and name and "\\" not in name and
            "\x00" not in name and ":" not in name and not name.startswith("/"),
            "unsafe source path")
    parts = name.split("/")
    require(all(part not in ("", ".", "..") for part in parts) and
            str(PurePosixPath(name)) == name, "unsafe source path")
    return parts


def verify_archive(path, archive_sha, manifest_sha, commit):
    require(all(isinstance(value, str) and pattern.fullmatch(value)
                for value, pattern in ((archive_sha, HEX64), (manifest_sha, HEX64),
                                       (commit, HEX40))), "exact source identity required")
    require(path.is_absolute() and path.is_relative_to(BASE / "incoming"),
            "archive must be inside private incoming directory")
    trusted_path(path, regular=True)
    fd = os.open(path, os.O_RDONLY | NOFOLLOW)
    with os.fdopen(fd, "rb") as stream:
        content = stream.read(MAX_ARCHIVE + 1)
    require(len(content) <= MAX_ARCHIVE and digest(content) == archive_sha,
            "archive bytes differ")
    with zipfile.ZipFile(io.BytesIO(content)) as archive:
        infos = archive.infolist()
        names = [item.filename for item in infos]
        require(1 < len(infos) < 1000 and len(names) == len(set(names)) and
                "SOURCE_MANIFEST.json" in names and
                sum(item.file_size for item in infos) <= MAX_EXPANDED,
                "archive inventory or expansion budget differs")
        for info in infos:
            archive_name(info.filename)
            require(not info.is_dir() and info.flag_bits & 1 == 0 and
                    (info.external_attr >> 16) & 0o170000 == stat.S_IFREG and
                    info.file_size <= MAX_EXPANDED,
                    "non-regular or encrypted archive member")
        manifest_bytes = archive.read("SOURCE_MANIFEST.json")
        require(digest(manifest_bytes) == manifest_sha, "manifest bytes differ")
        manifest = json.loads(manifest_bytes)
        require(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()
                == manifest_bytes and manifest.get("format_version") == 1 and
                manifest.get("commit") == commit and
                set(manifest) == {"format_version", "commit", "files"},
                "noncanonical or unexpected manifest")
        files = manifest.get("files")
        require(isinstance(files, list) and files and
                all(isinstance(entry, dict) for entry in files) and
                [item.filename for item in infos if item.filename != "SOURCE_MANIFEST.json"]
                == sorted(names[:-1]) and
                [entry.get("path") for entry in files] ==
                sorted(set(names) - {"SOURCE_MANIFEST.json"}),
                "manifest inventory differs")
        require(all(name in names for name in (ENTRY, PROVISIONER, INITDB)),
                "required reviewed source missing")
        for entry in files:
            require(isinstance(entry, dict) and set(entry) == {"path", "sha256", "size"}
                    and isinstance(entry["sha256"], str) and
                    HEX64.fullmatch(entry["sha256"]) and
                    type(entry["size"]) is int and 0 <= entry["size"] <= MAX_EXPANDED,
                    "invalid manifest entry")
            payload = archive.read(entry["path"])
            require(len(payload) == entry["size"] and digest(payload) == entry["sha256"],
                    "tracked source file differs")
        expected_self = next(entry["sha256"] for entry in files if entry["path"] == ENTRY)
        require(file_digest(Path(__file__)) == expected_self,
                "installed runner differs from approved source")
    return manifest, content


def extract_verified(content, manifest, source):
    private_dir(source)
    with zipfile.ZipFile(io.BytesIO(content)) as archive:
        for entry in manifest["files"]:
            target = source.joinpath(*archive_name(entry["path"]))
            stack = []
            parent = target.parent
            while parent != source and not parent.exists():
                stack.append(parent)
                parent = parent.parent
            for directory in reversed(stack):
                private_dir(directory)
            private_file(target, archive.read(entry["path"]), 0o400)
    return source_digest(source, manifest)


def source_digest(source, manifest):
    expected = {entry["path"]: entry["sha256"] for entry in manifest["files"]}
    found = {}
    for path in source.rglob("*"):
        meta = os.lstat(path)
        require(meta.st_uid == 0 and not stat.S_IMODE(meta.st_mode) & 0o022,
                "source ownership or mode changed")
        if stat.S_ISREG(meta.st_mode):
            found[path.relative_to(source).as_posix()] = file_digest(path)
        else:
            require(stat.S_ISDIR(meta.st_mode), "source special file appeared")
    require(found == expected, "extracted source differs from approved manifest")
    return digest(json.dumps(found, sort_keys=True, separators=(",", ":")).encode())


def load_provisioner(path):
    spec = importlib.util.spec_from_file_location("p0c4_restore_target_reviewed", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def verified_live(provisioner, identity, subnet, before, expected, initdb, target):
    live = provisioner.snapshot()
    live["images"] = provisioner._inspect("image", [identity["image"]])
    observed = provisioner.verify_created(identity, subnet, before, live)
    require(observed == expected, "immutable PG object identity changed")
    pg = next(c for c in live["containers"] if c["Id"] == expected["container_id"])
    mounts = {m.get("Destination"): m for m in pg.get("Mounts", [])}
    expected_mounts = {"/var/lib/postgresql", "/docker-entrypoint-initdb.d/10-restore.sh",
                       "/run/secrets/postgres_password", "/run/secrets/admin_password"}
    require(set(mounts) == expected_mounts and len(pg["Mounts"]) == 4,
            "unexpected PG mount or secret")
    for destination, source in (("/docker-entrypoint-initdb.d/10-restore.sh", initdb),
                                ("/run/secrets/postgres_password", target / "secrets" / "postgres_password"),
                                ("/run/secrets/admin_password", target / "secrets" / "admin_password")):
        mount = mounts[destination]
        require(mount.get("Type") == "bind" and mount.get("Source") == str(source) and
                mount.get("RW") is False, "secret or initdb bind differs")
    env = pg.get("Config", {}).get("Env") or []
    required = {"POSTGRES_USER=postgres", "POSTGRES_DB=postgres",
                "POSTGRES_PASSWORD_FILE=/run/secrets/postgres_password",
                "C4_TARGET_DATABASE=" + identity["database"]}
    require(required.issubset(set(env)) and
            not any(re.search(r"PASSWORD|SECRET|TOKEN|DSN", value.split("=", 1)[0], re.I)
                    for value in env if value != "POSTGRES_PASSWORD_FILE=/run/secrets/postgres_password"),
            "PG credential environment differs")
    return live


def safe_stop(provisioner, identity, container_id):
    require(HEX64.fullmatch(container_id), "verified PG ID required for stop")
    live = provisioner.snapshot()
    owned = [c for c in live["containers"] if c.get("Id") == container_id and
             (c.get("Config", {}).get("Labels") or {}).get("com.docker.compose.project")
             == identity["project"] and
             (c.get("Config", {}).get("Labels") or {}).get("com.docker.compose.service") == "pg"]
    require(len(owned) == 1 and owned[0].get("Config", {}).get("Image") == identity["image"] and
            set((owned[0].get("NetworkSettings") or {}).get("Networks") or {}) ==
            {identity["network"]} and
            len([m for m in owned[0].get("Mounts", []) if m.get("Type") == "volume" and
                 m.get("Name") == identity["volume"] and
                 m.get("Destination") == "/var/lib/postgresql"]) == 1,
            "cannot safely identify own PG container")
    if owned[0].get("State", {}).get("Running") is True:
        stopped = provisioner.quarantine(identity, live, provisioner._docker, {container_id})
        require(stopped == [container_id], "own PG stop was not issued")
    final = provisioner.snapshot()
    matches = [c for c in final["containers"] if c.get("Id") == container_id]
    require(len(matches) == 1 and matches[0].get("State", {}).get("Running") is False,
            "own PG did not stop")
    volumes = [v for v in final["volumes"] if v.get("Name") == identity["volume"] and
               (v.get("Labels") or {}).get("com.docker.compose.project") == identity["project"]]
    require(len(volumes) == 1, "quarantine volume missing")
    return {"container_id": container_id, "running": False,
            "volume_name": identity["volume"], "volume_retained": True}


def negative_acl_probe(provisioner, identity, container_id):
    """Prove the reviewed probe emits REJECT after one isolated ACL grant."""
    grant = provisioner._docker("exec", "--user", "postgres", container_id,
                                "psql", "-XAt", "-v", "ON_ERROR_STOP=1",
                                "--dbname", identity["database"],
                                "-c", "GRANT CREATE ON SCHEMA public TO learning_runtime")
    require(grant.strip() == "GRANT", "negative ACL mutation did not apply")
    probe_outputs = []
    original_docker = provisioner._docker

    def capture_probe(*command):
        output = original_docker(*command)
        if command[:2] == ("exec", "--user") and "-c" in command:
            probe_outputs.append(output)
        return output

    provisioner._docker = capture_probe
    rejected = False
    try:
        provisioner.probe_initdb(identity, container_id)
    except provisioner.AdmissionError:
        rejected = True
    finally:
        provisioner._docker = original_docker
    require(rejected and probe_outputs == ["REJECT\n"],
            "negative ACL probe did not explicitly reject")
    return digest(b"REJECT\n")


def run(args):
    require(sys.platform.startswith("linux") and os.geteuid() == 0,
            "root Linux required")
    os.umask(0o077)
    require(not os.environ.get("DOCKER_CONTEXT") and not os.environ.get("DOCKER_HOST"),
            "Docker endpoint override forbidden")
    trusted_path(Path(__file__), regular=True)
    require(Path(__file__).absolute().is_relative_to(BASE / "tools"),
            "runner must be installed in private tools directory")
    trusted_path(BASE)
    require(stat.S_IMODE(BASE.stat().st_mode) == 0o700,
            "control base must be 0700")
    ensure_private_dir(BASE / "tools")
    ensure_private_dir(BASE / "incoming")
    ensure_private_dir(BASE / "batches")
    manifest, archive_bytes = verify_archive(args.archive, args.archive_sha256,
                                              args.manifest_sha256, args.source_commit)
    batch_id = str(uuid.uuid4())
    batch = BASE / "batches" / f"restore-target-{args.archive_sha256[:12]}-{batch_id}"
    private_dir(batch)
    evidence = batch / "evidence"
    private_dir(evidence)
    result = {"state": "FAILED", "batch_id": batch_id,
              "archive_sha256": args.archive_sha256,
              "manifest_sha256": args.manifest_sha256,
              "source_commit": args.source_commit, "subnet": args.subnet,
              "gates": [], "failure_label": None, "stop": None,
              "target_dirty": False,
              "provisioner_state_is_pre_negative_only": True}
    provisioner = None
    identity = None
    container_id = None
    source = batch / "source"
    stage = "extract-reviewed-source"
    try:
        result["source_before_sha256"] = extract_verified(archive_bytes, manifest, source)
        stage = "prepare-private-control"
        control = batch / "control"
        private_dir(control)
        private_dir(control / "targets")
        installed_initdb = batch / "initdb.sh"
        private_file(installed_initdb, (source / INITDB).read_bytes(), 0o444)
        require(file_digest(installed_initdb) == next(e["sha256"] for e in manifest["files"]
                                                       if e["path"] == INITDB),
                "installed initdb differs")
        provisioner = load_provisioner(source / PROVISIONER)
        identity = provisioner.identity_for(batch_id)
        result.update({"project": identity["project"], "database": identity["database"],
                       "network": identity["network"], "volume": identity["volume"]})
        stage = "live-admission"
        before = provisioner.snapshot()
        provisioner.admit_fresh(identity, args.subnet, before)
        stage = "provision-target"
        status = provisioner.provision(control, batch_id, args.subnet, installed_initdb)
        require(status["state"] == "CREATED_QUARANTINED", "target not quarantined")
        container_id = status["container_id"]
        target = control / "targets" / batch_id
        stage = "created-and-isolated"
        verified_live(provisioner, identity, args.subnet, before, status_ids(status),
                      installed_initdb, target)
        result["gates"].append({"name": "created-and-isolated", "exit": 0,
                                "container_id": container_id,
                                "network_id": status["network_id"]})
        stage = "pg18-and-initdb-facts"
        version = provisioner._docker("exec", "--user", "postgres", container_id,
                                      "psql", "-XAt", "-v", "ON_ERROR_STOP=1",
                                      "--dbname", identity["database"],
                                      "-c", "SHOW server_version_num")
        require(version.strip().isdigit() and 180000 <= int(version.strip()) < 190000,
                "dedicated database is not PostgreSQL 18")
        provisioner.probe_initdb(identity, container_id)
        result["gates"].append({"name": "pg18-and-initdb-facts", "exit": 0,
                                "server_version_num": int(version.strip()),
                                "probe_sha256": digest(b"OK\n")})
        stage = "runtime-public-create-rejected"
        # One intentional mutation, confined to the new target's dedicated DB.
        result["target_dirty"] = True  # Conservative even if GRANT returns an error.
        rejection_hash = negative_acl_probe(provisioner, identity, container_id)
        result["gates"].append({"name": "runtime-public-create-rejected", "exit": 0,
                                "probe_sha256": rejection_hash})
        stage = "post-negative-isolation"
        verified_live(provisioner, identity, args.subnet, before, status_ids(status),
                      installed_initdb, target)
        result["gates"].append({"name": "post-negative-isolation", "exit": 0})
        result["source_after_sha256"] = source_digest(source, manifest)
        require(result["source_before_sha256"] == result["source_after_sha256"],
                "reviewed source changed during acceptance")
        stage = "verified-container-stop"
        result["stop"] = safe_stop(provisioner, identity, container_id)
        result["state"] = "PG18_TARGET_ACCEPTANCE_PASSED_DIRTY_QUARANTINED_NOT_RESTORE"
    except BaseException as error:
        result["failure_label"] = type(error).__name__
        result["gates"].append({"name": stage, "exit": 1})
        if provisioner is not None and identity is not None and container_id is not None:
            try:
                result["stop"] = safe_stop(provisioner, identity, container_id)
            except BaseException as stop_error:
                result["stop"] = {"confirmed": False,
                                  "failure_label": type(stop_error).__name__}
        if source.exists():
            try:
                result["source_after_sha256"] = source_digest(source, manifest)
            except BaseException:
                result["source_after_sha256"] = None
    finally:
        encoded = json.dumps(result, sort_keys=True, separators=(",", ":")).encode()
        private_file(evidence / "result.json", encoded)
        print(json.dumps({"state": result["state"], "batch_id": batch_id,
                          "result_sha256": digest(encoded),
                          "evidence": str(evidence)}, sort_keys=True), flush=True)
    return 0 if result["state"] == "PG18_TARGET_ACCEPTANCE_PASSED_DIRTY_QUARANTINED_NOT_RESTORE" else 1


def status_ids(status):
    return {key: status[key] for key in
            ("container_id", "network_id", "volume_name", "volume_mountpoint")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--archive-sha256", required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--subnet", required=True)
    try:
        return run(parser.parse_args())
    except (OSError, ValueError, zipfile.BadZipFile, KeyError, TypeError):
        print("RESTORE_TARGET_ACCEPTANCE_ADMISSION_REJECTED", flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
