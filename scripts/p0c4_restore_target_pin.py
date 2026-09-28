#!/usr/bin/env python3
"""Read-only P0-C4 fresh-target pin eligibility check on an isolated Linux host.

The digest is a review input for a separate build. It is never a restore
authorization, a CompleteBackup receipt, or proof of independent fault domains.
"""

import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import stat

import p0c4_restore_birth_acceptance as acceptance
import p0c4_restore_target as target_provisioner
import p0c4_restore_target_birth as issuer


AdmissionError = target_provisioner.AdmissionError
require = target_provisioner.require
EVIDENCE_KEYS = {
    "state", "project", "batch_id", "docker_daemon_id", "container_id",
    "network_id", "volume_name", "volume_mountpoint", "volume_mount_dev",
    "volume_mount_ino", "image_id", "destination_dev", "destination_ino",
    "control_dev", "control_ino", "asset_dev", "asset_ino", "birth_sha256",
}


def _digest(content):
    return hashlib.sha256(content).hexdigest()


def _json_bytes(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False,
                      separators=(",", ":")).encode("utf-8")


def _trusted_private_dir(path):
    require(os.geteuid() == 0 and path.is_absolute(),
            "root-only absolute path required")
    for ancestor in reversed((path, *path.parents)):
        meta = os.lstat(ancestor)
        require(stat.S_ISDIR(meta.st_mode) and meta.st_uid == 0 and
                stat.S_IMODE(meta.st_mode) & 0o022 == 0,
                "untrusted target path")
    require(stat.S_IMODE(os.lstat(path).st_mode) == 0o700,
            "root-private target directory required")


@contextlib.contextmanager
def _existing_creation_lock(root):
    """Take the provisioner's *existing* lock without creating any file."""
    import fcntl  # Linux-only; pure gates have local Windows tests.
    _trusted_private_dir(root)
    fd = os.open(root / ".restore-target.lock",
                 os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    try:
        meta = os.fstat(fd)
        require(stat.S_ISREG(meta.st_mode) and meta.st_uid == 0 and
                stat.S_IMODE(meta.st_mode) == 0o600 and meta.st_nlink == 1,
                "unsafe existing creation lock")
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield
    finally:
        os.close(fd)


def _private_read(path):
    # The nonblocking variant rejects FIFOs before attempting a read.
    return acceptance._private_read_diagnostic(path)


def _empty_roots(target, identity, birth, evidence):
    destination, control, assets = (target / name for name in
                                    ("destination", "control", "assets"))
    for path, keys in ((destination, ("destination_dev", "destination_ino")),
                       (control, ("control_dev", "control_ino")),
                       (assets, ("asset_dev", "asset_ino"))):
        _trusted_private_dir(path)
        meta = os.lstat(path)
        require((meta.st_dev, meta.st_ino) ==
                (evidence[keys[0]], evidence[keys[1]]),
                "private restore root identity changed")
    require((evidence["control_dev"], evidence["control_ino"]) ==
            (birth["control_dev"], birth["control_ino"]) and
            (evidence["asset_dev"], evidence["asset_ino"]) ==
            (birth["asset_dev"], birth["asset_ino"]),
            "birth restore root identity changed")
    require(not any(destination.iterdir()) and not any(assets.iterdir()) and
            [item.name for item in control.iterdir()] ==
            [identity["database"] + ".birth.json"],
            "restore roots are not empty and untouched")


def read_records(target, identity, batch_id):
    """Re-read durable local records; failure markers have first precedence."""
    require(target.name == batch_id,
            "target is not an eligible new batch")
    _trusted_private_dir(target)
    require(not os.path.lexists(target / "failure.json") and
            not os.path.lexists(target / "issuer-diagnostic.json"),
            "failure evidence present")
    for name in ("destination", "control", "assets"):
        _trusted_private_dir(target / name)
    birth_bytes = _private_read(target / "control" /
                                (identity["database"] + ".birth.json"))
    state_bytes = _private_read(target / "state.json")
    success_bytes = _private_read(target / "issuance-success.json")
    evidence_bytes = _private_read(target / "birth-evidence.json")
    state, success, evidence = (acceptance._unique_json(content) for content in
                                (state_bytes, success_bytes, evidence_bytes))
    birth, state, success = acceptance.validate_candidate_records(
        identity, batch_id, birth_bytes, state, success, evidence)
    require(type(evidence) is dict and set(evidence) == EVIDENCE_KEYS and
            evidence["state"] ==
            "BIRTH_SOURCE_REINSPECTED_NOT_RESTORE_ACCEPTANCE" and
            evidence["project"] == identity["project"] and
            evidence["batch_id"] == batch_id and
            type(evidence["docker_daemon_id"]) is str and
            bool(evidence["docker_daemon_id"]),
            "root-private issuance evidence differs")
    _empty_roots(target, identity, birth, evidence)
    return (birth, state, success, evidence, birth_bytes, state_bytes,
            success_bytes, evidence_bytes)


def inspect_live(identity, state, success, evidence, birth, target, initdb):
    """Check current Docker, exact mounts, PG18 identity, ACL and empty catalog."""
    live = target_provisioner.snapshot()
    live["images"] = target_provisioner._inspect("image", [identity["image"]])
    expected = {"daemon_id": evidence["docker_daemon_id"]}
    pg, network, volume = acceptance.validate_live_docker(
        identity, state["subnet"], expected, live, state, target, initdb)
    acceptance.verify_live_mount_inode(volume["Mountpoint"], success)
    require(pg["Image"] == success["image_id"] and
            network["Id"] == success["network_id"] and
            volume["Name"] == success["pg_volume_name"],
            "Docker differs from issuance seal")
    issuer_facts = issuer.probe_pg_facts(identity, pg["Id"])
    issuer.validate_pg_facts(issuer_facts)
    observed = acceptance.observe_pg(target_provisioner, identity, pg["Id"])
    acceptance.validate_pg_against_birth(observed, birth, issuer_facts)
    # Reinspect after SQL to reject an object swap during the check.
    again = target_provisioner.snapshot()
    again["images"] = target_provisioner._inspect("image", [identity["image"]])
    acceptance.validate_live_docker(
        identity, state["subnet"], expected, again, state, target, initdb)
    require(_docker_projection(live, identity) ==
            _docker_projection(again, identity),
            "Docker changed during pin check")
    acceptance.verify_live_mount_inode(volume["Mountpoint"], success)
    return {
        "docker_daemon_id": live["daemon_id"],
        "container_id": pg["Id"], "network_id": network["Id"],
        "volume_name": volume["Name"], "volume_mountpoint": volume["Mountpoint"],
        "image_id": pg["Image"], "docker_sha256": _digest(
            _json_bytes(_docker_projection(live, identity))),
        "pg_sha256": _digest(_json_bytes(observed)),
        "issuer_pg_sha256": _digest(_json_bytes(issuer_facts)),
    }


def _docker_projection(live, identity):
    project = identity["project"]
    containers = [c for c in live["containers"] if
                  (c.get("Config", {}).get("Labels") or {}).get(
                      "com.docker.compose.project") == project]
    networks = [n for n in live["networks"] if
                (n.get("Labels") or {}).get("com.docker.compose.project") ==
                project]
    volumes = [v for v in live["volumes"] if
               (v.get("Labels") or {}).get("com.docker.compose.project") ==
               project]
    return {
        "daemon_id": live["daemon_id"],
        "containers": [{
            "Id": c.get("Id"), "Image": c.get("Image"),
            "Config": {"Image": c.get("Config", {}).get("Image"),
                       "Labels": c.get("Config", {}).get("Labels")},
            "State": {"Running": c.get("State", {}).get("Running"),
                      "Status": c.get("State", {}).get("Status"),
                      "HealthStatus": c.get("State", {}).get(
                          "Health", {}).get("Status")},
            "HostConfig": {key: c.get("HostConfig", {}).get(key) for key in
                           ("NetworkMode", "PortBindings")},
            "NetworkSettings": {key: c.get("NetworkSettings", {}).get(key)
                                for key in ("Networks", "Ports")},
            "Mounts": c.get("Mounts"),
        } for c in containers],
        "networks": [{key: n.get(key) for key in
                      ("Id", "Name", "Labels", "Internal", "IPAM")}
                     for n in networks],
        "volumes": [{key: v.get(key) for key in
                     ("Name", "Labels", "Mountpoint", "Driver", "Scope",
                      "Options")} for v in volumes],
        "images": [{"Id": image.get("Id"),
                    "RepoDigests": image.get("RepoDigests")}
                   for image in live["images"]],
    }


def inspection_evidence_digest(batch_id, root, birth_bytes, state_bytes,
                               success_bytes, evidence_bytes, checked):
    payload = {
        "format_version": 1, "state": "PIN_CANDIDATE_NOT_RESTORE_AUTHORITY",
        "batch_id": batch_id, "control_root": root,
        "birth_sha256": _digest(birth_bytes),
        "creation_state_sha256": _digest(state_bytes),
        "issuance_success_sha256": _digest(success_bytes),
        "birth_evidence_sha256": _digest(evidence_bytes),
        "live": checked,
    }
    return _digest(_json_bytes(payload))


def pin(root, batch_id, initdb):
    identity = target_provisioner.identity_for(batch_id)
    target_provisioner._trusted_initdb(initdb)
    with _existing_creation_lock(root):
        _trusted_private_dir(root / "targets")
        target = root / "targets" / batch_id
        (birth, state, success, evidence, birth_bytes, state_bytes,
         success_bytes, evidence_bytes) = read_records(target, identity,
                                                       batch_id)
        checked = inspect_live(identity, state, success, evidence, birth,
                               target, initdb)
        return {
            "birth_sha256": _digest(birth_bytes),
            "inspection_evidence_sha256": inspection_evidence_digest(
                batch_id, str(root), birth_bytes, state_bytes,
                success_bytes, evidence_bytes, checked),
        }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--batch-id", required=True)
    parser.add_argument("--initdb", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        digests = pin(args.root, args.batch_id, args.initdb)
    except Exception:
        print("PIN_CANDIDATE_REJECTED", flush=True)
        return 1
    print(json.dumps({"state": "PIN_CANDIDATE_NOT_RESTORE_AUTHORITY",
                      **digests}, separators=(",", ":")),
          flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
