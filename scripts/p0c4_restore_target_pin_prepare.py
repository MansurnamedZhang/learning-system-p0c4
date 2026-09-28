#!/usr/bin/env python3
"""Issue pin-only provenance before a fresh P0-C4 target is created.

Run as root against a new private control root with an empty targets directory.
This command records absence under the same lock used by target provisioning;
it does not create a target, issue birth, restore data, or pin a build.
"""

import argparse
import json
import os
from pathlib import Path

import p0c4_restore_target as target_provisioner
import p0c4_restore_target_pin as pin


require = target_provisioner.require


def _targets_empty(targets):
    return not any(targets.iterdir())


def _root_has_only_setup(root):
    return {item.name for item in root.iterdir()} == {
        "targets", ".restore-target.lock"}


def _initdb_digest(path):
    return pin._initdb_digest(path)


def prepare(root, batch_id, subnet, initdb):
    """O_EXCL marker publication is the sole persistent change after locking."""
    identity = target_provisioner.identity_for(batch_id)
    pin.reject_acceptance_root(root)
    target_provisioner._trusted_initdb(initdb)
    with target_provisioner._locked_root(root):
        targets = root / "targets"
        pin._trusted_private_dir(targets)
        require(not os.path.lexists(targets / batch_id) and
                not os.path.lexists(root / "pin-precreation.json") and
                _root_has_only_setup(root) and
                _targets_empty(targets),
                "pin root or target already used")
        before = target_provisioner.snapshot()
        target_provisioner.admit_fresh(identity, subnet, before)
        root_meta, targets_meta = os.lstat(root), os.lstat(targets)
        record = {
            "format_version": 1,
            "state": "PIN_ONLY_PRECREATION_NOT_RESTORE_AUTHORITY",
            "target_absent_at_precreation": True,
            "batch_id": batch_id,
            "project": identity["project"],
            "database": identity["database"],
            "network": identity["network"],
            "volume": identity["volume"],
            "subnet": subnet,
            "root_path": str(root),
            "root_dev": root_meta.st_dev,
            "root_ino": root_meta.st_ino,
            "targets_dev": targets_meta.st_dev,
            "targets_ino": targets_meta.st_ino,
            "docker_daemon_id": before["daemon_id"],
            "initdb_path": str(initdb),
            "initdb_sha256": _initdb_digest(initdb),
        }
        payload = pin._json_bytes(record)
        require(len(payload) <= 4096, "pin precreation record too large")
        old_umask = os.umask(0o077)
        try:
            target_provisioner._private_write(
                root / "pin-precreation.json", payload)
        finally:
            os.umask(old_umask)
    return pin._digest(payload)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--batch-id", required=True)
    parser.add_argument("--subnet", required=True)
    parser.add_argument("--initdb", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        digest = prepare(args.root, args.batch_id, args.subnet, args.initdb)
    except Exception:
        print("PIN_PRECREATION_REJECTED", flush=True)
        return 1
    print(json.dumps({"state": "PIN_PRECREATION_RECORDED_NOT_RESTORE_AUTHORITY",
                      "precreation_sha256": digest}, separators=(",", ":")),
          flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
