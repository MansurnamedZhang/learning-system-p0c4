#!/usr/bin/env python3
"""Root-only fresh PG18 restore-target provisioning; result stays quarantined.

This creates no birth attestation, CompleteBackup, restored rows, or service
admission. The local Docker daemon and its operators are trusted. Run only
after reviewing the source, from a root-owned path on an isolated Linux host.
"""

import argparse
import contextlib
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import stat
import subprocess
import uuid


class AdmissionError(RuntimeError):
    pass


DOCKER = "/usr/bin/docker"
IP = "/usr/sbin/ip"
IMAGE = "postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
HEX_ID = re.compile(r"[0-9a-f]{64}\Z")


def require(condition, message):
    if not condition:
        raise AdmissionError(message)


def identity_for(batch_id):
    try:
        parsed = uuid.UUID(batch_id)
    except (ValueError, AttributeError) as error:
        raise AdmissionError("canonical UUIDv4 required") from error
    require(parsed.version == 4 and str(parsed) == batch_id,
            "canonical UUIDv4 required")
    project = f"learning-system-p0c4-restore-{batch_id}"
    return {"project": project, "database": f"learning_restore_c4_{batch_id}",
            "network": project + "_test", "volume": project + "_pg",
            "image": IMAGE}


def _network(value):
    try:
        return ipaddress.ip_network(value, strict=True)
    except ValueError as error:
        raise AdmissionError("invalid explicit subnet") from error


def admit_fresh(identity, subnet, snapshot):
    """Admission is absence *now*, not a historical never-existed claim."""
    chosen = _network(subnet)
    require(chosen.version == 4 and chosen.prefixlen >= 24 and
            chosen.is_private and not chosen.is_loopback and
            not chosen.is_link_local, "private explicit /24-or-smaller subnet required")
    require(snapshot.get("daemon_id"), "Docker daemon identity unavailable")
    project = identity["project"]
    for item in snapshot["containers"]:
        labels = item.get("Config", {}).get("Labels") or {}
        name = item.get("Name", "").lstrip("/")
        require(labels.get("com.docker.compose.project") != project and
                name != project and not name.startswith(project + "-"),
                "project container already exists")
    for item in snapshot["networks"]:
        labels = item.get("Labels") or {}
        require(item.get("Name") != identity["network"] and
                labels.get("com.docker.compose.project") != project,
                "project network already exists")
    for item in snapshot["volumes"]:
        labels = item.get("Labels") or {}
        require(item.get("Name") != identity["volume"] and
                labels.get("com.docker.compose.project") != project,
                "project volume already exists")
    occupied = list(snapshot["routes"])
    for item in snapshot["networks"]:
        occupied.extend(row.get("Subnet") for row in
                        (item.get("IPAM") or {}).get("Config") or [] if row.get("Subnet"))
    for raw in occupied:
        if raw in (None, "default"):
            continue
        try:
            other = ipaddress.ip_network(raw, strict=False)
        except ValueError as error:
            raise AdmissionError("host route or Docker subnet unreadable") from error
        require(other.version != chosen.version or not chosen.overlaps(other),
                "requested subnet overlaps host route or Docker network")


def compose_document(identity, subnet, target, initdb):
    project = identity["project"]
    return {
        "name": project,
        "services": {"pg": {
            "image": identity["image"], "pull_policy": "never",
            "cpus": 2, "mem_limit": "4g",
            "environment": {"POSTGRES_USER": "postgres", "POSTGRES_DB": "postgres",
                            "POSTGRES_PASSWORD_FILE": "/run/secrets/postgres_password",
                            "C4_TARGET_DATABASE": identity["database"]},
            "secrets": ["postgres_password", "admin_password"],
            "volumes": [identity["volume"] + ":/var/lib/postgresql",
                        str(initdb) + ":/docker-entrypoint-initdb.d/10-restore.sh:ro"],
            "networks": ["test"],
            "healthcheck": {"test": ["CMD-SHELL", "pg_isready -U postgres -d postgres"],
                            "interval": "2s", "timeout": "3s", "retries": 30},
        }},
        "networks": {"test": {"name": identity["network"], "internal": True,
                              "ipam": {"config": [{"subnet": subnet}]}}},
        "volumes": {identity["volume"]: {"name": identity["volume"]}},
        "secrets": {key: {"file": str(target / "secrets" / key)}
                    for key in ("postgres_password", "admin_password")},
    }


def verify_created(identity, subnet, before, after):
    """Check daemon, immutable IDs, labels, network isolation, and PG data mount."""
    project = identity["project"]
    require(after.get("daemon_id") == before.get("daemon_id"), "Docker daemon changed")
    containers = [c for c in after["containers"] if
                  (c.get("Config", {}).get("Labels") or {}).get("com.docker.compose.project") == project]
    networks = [n for n in after["networks"] if
                (n.get("Labels") or {}).get("com.docker.compose.project") == project]
    volumes = [v for v in after["volumes"] if
               (v.get("Labels") or {}).get("com.docker.compose.project") == project]
    require(len(containers) == len(networks) == len(volumes) == 1,
            "exactly one PG container, network, and volume required")
    pg, network, volume = containers[0], networks[0], volumes[0]
    labels = pg.get("Config", {}).get("Labels") or {}
    require(bool(HEX_ID.fullmatch(pg.get("Id", ""))) and
            bool(HEX_ID.fullmatch(network.get("Id", ""))), "invalid Docker object ID")
    require(labels.get("com.docker.compose.service") == "pg" and
            pg.get("Config", {}).get("Image") == identity["image"] and
            pg.get("State", {}).get("Running") is True and
            pg.get("State", {}).get("Health", {}).get("Status") == "healthy",
            "PG container identity differs")
    expected_digest = identity["image"].split("@", 1)[1]
    images = after.get("images") or []
    require(len(images) == 1 and
            images[0].get("Id") == pg.get("Image") and
            bool(HEX_ID.fullmatch(pg.get("Image", "").removeprefix("sha256:"))) and
            any(value.endswith("@" + expected_digest) for value in
                images[0].get("RepoDigests") or []),
            "PG image ID is not pinned digest")
    config = (network.get("IPAM") or {}).get("Config") or []
    require((network.get("Labels") or {}).get("com.docker.compose.project") == project and
            (volume.get("Labels") or {}).get("com.docker.compose.project") == project and
            network.get("Name") == identity["network"] and
            volume.get("Name") == identity["volume"] and
            network.get("Internal") is True and
            len(config) == 1 and config[0].get("Subnet") == subnet,
            "network or volume identity differs")
    settings = pg.get("NetworkSettings") or {}
    attached = settings.get("Networks") or {}
    require(set(attached) == {identity["network"]} and
            attached[identity["network"]].get("NetworkID") == network["Id"] and
            pg.get("HostConfig", {}).get("NetworkMode") == identity["network"],
            "PG attached outside isolated network")
    require(not any(settings.get("Ports", {}).values()) and
            not any((pg.get("HostConfig", {}).get("PortBindings") or {}).values()),
            "PG published a host port")
    mountpoint = volume.get("Mountpoint")
    require(isinstance(mountpoint, str) and mountpoint.startswith("/") and
            len([m for m in pg.get("Mounts", []) if m.get("Destination") == "/var/lib/postgresql"]) == 1,
            "PG data mount missing")
    data_mount = next(m for m in pg["Mounts"] if m.get("Destination") == "/var/lib/postgresql")
    require(data_mount.get("Type") == "volume" and
            data_mount.get("Name") == identity["volume"] and
            data_mount.get("Source") == mountpoint and
            data_mount.get("RW") is True, "PG data mount identity differs")
    return {"container_id": pg["Id"], "network_id": network["Id"],
            "volume_name": volume["Name"], "volume_mountpoint": mountpoint}


def quarantine(identity, snapshot, stop):
    """Stop only inspected IDs carrying our exact project and PG labels."""
    stopped = []
    for container in snapshot["containers"]:
        labels = container.get("Config", {}).get("Labels") or {}
        container_id = container.get("Id", "")
        if (labels.get("com.docker.compose.project") == identity["project"]
                and labels.get("com.docker.compose.service") == "pg"
                and HEX_ID.fullmatch(container_id)):
            stop("stop", "--time", "1", container_id)
            stopped.append(container_id)
    return stopped


def _command(binary, *args):
    result = subprocess.run([binary, *args], text=True, capture_output=True,
                            check=False, env={"PATH": "/usr/sbin:/usr/bin:/bin",
                                              "DOCKER_HOST": "unix:///var/run/docker.sock"})
    require(result.returncode == 0, "local host or Docker inspection failed")
    return result.stdout


def _docker(*args):
    return _command(DOCKER, *args)


def _inspect(kind, ids):
    if not ids:
        return []
    args = ("inspect", *ids) if kind == "container" else (kind, "inspect", *ids)
    result = json.loads(_docker(*args))
    require(isinstance(result, list), "Docker inspect shape changed")
    return result


def snapshot():
    daemon = _docker("info", "--format", "{{.ID}}").strip()
    container_ids = _docker("ps", "-aq").split()
    network_ids = _docker("network", "ls", "-q").split()
    volume_names = _docker("volume", "ls", "-q").split()
    routes = json.loads(_command(IP, "-j", "-4", "route", "show", "table", "all"))
    require(isinstance(routes, list), "host route shape changed")
    return {"daemon_id": daemon, "containers": _inspect("container", container_ids),
            "networks": _inspect("network", network_ids),
            "volumes": _inspect("volume", volume_names),
            "routes": [entry["dst"] for entry in routes if entry.get("dst", "default") != "default"]}


def _trusted_root(path):
    require(os.geteuid() == 0 and path.is_absolute(), "root-only absolute control root required")
    for ancestor in reversed((path, *path.parents)):
        meta = os.lstat(ancestor)
        require(stat.S_ISDIR(meta.st_mode) and meta.st_uid == 0 and
                not meta.st_mode & 0o022, "untrusted control ancestor")
    require(stat.S_IMODE(os.lstat(path).st_mode) == 0o700,
            "control root must be root-private 0700")


def _trusted_initdb(path):
    require(path.is_absolute(), "reviewed initdb path must be absolute")
    for ancestor in reversed(path.parents):
        meta = os.lstat(ancestor)
        require(stat.S_ISDIR(meta.st_mode) and meta.st_uid == 0 and
                not meta.st_mode & 0o022, "untrusted initdb ancestor")
    meta = os.lstat(path)
    require(stat.S_ISREG(meta.st_mode) and meta.st_uid == 0 and
            stat.S_IMODE(meta.st_mode) == 0o444, "root-owned readable initdb required")


@contextlib.contextmanager
def _locked_root(root):
    import fcntl  # Linux-only; local unit tests exercise pure gates on Windows.
    _trusted_root(root)
    fd = os.open(root / ".restore-target.lock",
                 os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    try:
        meta = os.fstat(fd)
        require(meta.st_uid == 0 and stat.S_ISREG(meta.st_mode) and
                stat.S_IMODE(meta.st_mode) == 0o600, "unsafe root lock")
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield
    finally:
        os.close(fd)


def _private_write(path, payload):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as file:
        file.write(payload)
        file.flush()
        os.fsync(file.fileno())
    directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def _postgres_uid():
    # Ephemeral, no-network and no-secret identity probe; pinned image must
    # already exist locally and no project resource is created by this probe.
    output = _docker("run", "--pull=never", "--rm", "--network", "none", "--entrypoint", "sh",
                     IMAGE, "-c", "id -u postgres; id -g postgres")
    lines = output.splitlines()
    require(len(lines) == 2 and all(line.isdecimal() for line in lines),
            "pinned image postgres identity unavailable")
    uid, gid = (int(line) for line in lines)
    require(uid > 0 and gid > 0, "pinned image postgres identity invalid")
    return uid, gid


def provision(root, batch_id, subnet, initdb):
    identity = identity_for(batch_id)
    _trusted_initdb(initdb)
    with _locked_root(root):  # Held before first Docker call through quarantine.
        before = snapshot()
        admit_fresh(identity, subnet, before)
        target = root / "targets" / batch_id
        targets_meta = os.lstat(root / "targets")
        require(stat.S_ISDIR(targets_meta.st_mode) and targets_meta.st_uid == 0 and
                stat.S_IMODE(targets_meta.st_mode) == 0o700 and not target.exists(),
                "root-private target parent and new target required")
        uid, gid = _postgres_uid()
        old_umask = os.umask(0o077)
        try:
            target.mkdir(mode=0o700)
            secret_dir = target / "secrets"
            secret_dir.mkdir(mode=0o700)
            for key in ("postgres_password", "admin_password"):
                secret_path = secret_dir / key
                _private_write(secret_path, (secrets.token_hex(32) + "\n").encode())
                os.chown(secret_path, uid, gid)
            compose = compose_document(identity, subnet, target, initdb)
            compose_path = target / "compose.json"
            _private_write(compose_path, json.dumps(compose, sort_keys=True,
                                                    separators=(",", ":")).encode())
            try:
                _docker("compose", "-f", str(compose_path), "config", "-q")
                _docker("compose", "-f", str(compose_path), "up", "-d", "--wait",
                        "--no-build", "--no-deps", "pg")
                after = snapshot()
                after["images"] = _inspect("image", [identity["image"]])
                ids = verify_created(identity, subnet, before, after)
                status = {"state": "CREATED_QUARANTINED", "batch_id": batch_id,
                          "project": identity["project"], "database": identity["database"],
                          "network": identity["network"], "subnet": subnet, **ids}
            except Exception:
                cleanup_error = None
                stopped = []
                try:
                    live = snapshot()
                    stopped = quarantine(identity, live, _docker)
                    stopped_live = snapshot()
                    require(not any((c.get("Config", {}).get("Labels") or {}).get(
                        "com.docker.compose.project") == identity["project"] and
                        c.get("State", {}).get("Running") is True
                        for c in stopped_live["containers"]),
                        "project container remains running after stop")
                except Exception as error:
                    cleanup_error = type(error).__name__
                finally:
                    _private_write(target / "state.json", json.dumps({
                        "state": "FAILED_QUARANTINE_ATTEMPTED", "batch_id": batch_id,
                        "project": identity["project"], "volume": identity["volume"],
                        "container_stop_confirmed": cleanup_error is None,
                        "cleanup_error": cleanup_error,
                    }, sort_keys=True).encode())
                raise
            _private_write(target / "state.json", json.dumps(status, sort_keys=True).encode())
            return status
        finally:
            os.umask(old_umask)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--batch-id", required=True)
    parser.add_argument("--subnet", required=True)
    parser.add_argument("--initdb", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = provision(args.root, args.batch_id, args.subnet, args.initdb)
    except (AdmissionError, OSError, ValueError, subprocess.SubprocessError):
        print("RESTORE_TARGET_QUARANTINED_OR_ADMISSION_REJECTED", flush=True)
        return 1
    print(json.dumps({"state": result["state"], "batch_id": result["batch_id"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
