#!/usr/bin/env python3
"""Isolate the P0-C4 physical-clone setup helper on a NEW local Docker volume.

This diagnostic uses the exact authorized runner bytes, creates no PostgreSQL
server, has no network or credentials, and never touches an acceptance batch.
The diagnostic volume is retained even on success for inspection/quarantine.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import types
import uuid


AUTHORIZED_RUNNER_SHA256 = (
    "b298f6814d50eff3a6fc1b6e322492264183e5fd263486a6a871cce3b5a5b97a"
)
PINNED_IMAGE = (
    "postgres:18.6-bookworm@sha256:"
    "9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
)
IMAGE_ID = re.compile(r"sha256:[0-9a-f]{64}\Z")
HEX64 = re.compile(r"[0-9a-f]{64}\Z")
ZERO_ID = "0" * 64  # Setup uses --network none; no primary ID is referenced.


def _read_runner(path):
    """Execute only bytes that exactly match the authorized, installed runner."""
    data = Path(path).read_bytes()
    if hashlib.sha256(data).hexdigest() != AUTHORIZED_RUNNER_SHA256:
        raise ValueError("runner hash mismatch")
    module = types.ModuleType("authorized_p0c4_restore_pin_acceptance")
    module.__file__ = str(path)
    exec(compile(data, str(path), "exec"), module.__dict__)
    return module


def _one_json(process):
    if process.returncode != 0:
        raise ValueError("Docker inspect failed")
    value = json.loads(process.stdout)
    if type(value) is not list or len(value) != 1 or type(value[0]) is not dict:
        raise ValueError("Docker inspect shape differs")
    return value[0]


def _projection(facts, helper_id, image, image_id, name, batch_id, volume):
    """Compare untrusted inspect fields without echoing any of their values."""
    config = facts.get("Config")
    config = config if type(config) is dict else {}
    host = facts.get("HostConfig")
    host = host if type(host) is dict else {}
    state = facts.get("State")
    state = state if type(state) is dict else {}
    labels = config.get("Labels")
    labels = labels if type(labels) is dict else {}
    mounts = facts.get("Mounts")
    mount_count = len(mounts) if type(mounts) is list else None
    mount = mounts[0] if type(mounts) is list and mounts and type(mounts[0]) is dict else {}
    cap_add = host.get("CapAdd")
    cap_add_class = ("CAP_CHOWN" if cap_add == ["CAP_CHOWN"] else
                     "CHOWN" if cap_add == ["CHOWN"] else "OTHER")
    env = config.get("Env")
    env = env if type(env) is list else []
    running = state.get("Running")
    exit_code = state.get("ExitCode")
    return {
        "id_matches": facts.get("Id") == helper_id,
        "image_id_matches": facts.get("Image") == image_id,
        "name_matches": facts.get("Name") == "/" + name,
        "image_ref_matches": config.get("Image") == image,
        "batch_label_matches": labels.get("com.knowweave.clone.batch") == batch_id,
        "network_none": host.get("NetworkMode") == "none",
        "root_user_matches": config.get("User") == "0:0",
        "entrypoint_matches": config.get("Entrypoint") == ["/bin/sh"],
        "cap_drop_matches": host.get("CapDrop") == ["ALL"],
        "cap_add_class": cap_add_class,
        "security_opt_matches": host.get("SecurityOpt") == ["no-new-privileges"],
        "privileged_false": host.get("Privileged") is False,
        "mount_count": mount_count,
        "mount_type_volume": mount.get("Type") == "volume",
        "mount_name_matches": mount.get("Name") == volume,
        "mount_destination_matches": mount.get("Destination") == "/var/lib/postgresql",
        "mount_rw": mount.get("RW") is True,
        "state_running": running if type(running) is bool else None,
        "exit_code": exit_code if type(exit_code) is int else None,
        "password_env_present": any(
            type(value) is str and value.startswith(("PGPASSWORD=", "POSTGRES_PASSWORD=", "PGPASSFILE="))
            for value in env),
    }


def diagnose(runner_path, image, uid, gid, *, batch_id=None, docker=None):
    """Return secret-free, fixed-stage evidence from one fresh setup attempt."""
    batch_id = batch_id or str(uuid.uuid4())
    volume = "knowweave-c4-clone-setup-diag-" + str(batch_id)
    name = "knowweave-c4-clone-" + str(batch_id) + "-setup"
    result = {"code": "INPUT_REJECTED", "stages": [], "batch_id": batch_id,
              "volume": volume, "helper_name": name, "helper_id": None,
              "cleanup": "NOT_CREATED", "volume_retained": None,
              "observed": {}, "failure_type": None}
    try:
        runner = _read_runner(runner_path)
    except Exception as error:
        result["code"] = "RUNNER_HASH_REJECTED"
        result["failure_type"] = type(error).__name__
        return result
    result["stages"].append("RUNNER_VERIFIED")
    try:
        valid_uuid = uuid.UUID(str(batch_id))
        if (valid_uuid.version != 4 or str(valid_uuid) != batch_id or
                type(uid) is not int or type(gid) is not int or
                uid <= 0 or gid <= 0 or
                image != PINNED_IMAGE):
            return result
    except (ValueError, TypeError, AttributeError):
        return result
    docker = docker or runner._probe_docker
    helper_id = None
    verified_identity = False
    volume_attempted = False
    try:
        result["code"] = "IMAGE_INSPECT_FAILED"
        image_facts = _one_json(docker(["image", "inspect", image]))
        image_id = image_facts.get("Id")
        if (type(image_id) is not str or not IMAGE_ID.fullmatch(image_id) or
                not any(type(ref) is str and
                        ref.endswith("@" + image.split("@", 1)[1])
                        for ref in image_facts.get("RepoDigests") or [])):
            result["code"] = "IMAGE_REJECTED"
            return result
        result["image_id"] = image_id
        result["stages"].append("IMAGE_VERIFIED")
        result["code"] = "FRESH_NAMES_INSPECT_FAILED"
        volumes = docker(["volume", "ls", "--format", "{{.Name}}",
                          "--filter", "name=" + volume])
        containers = docker(["container", "ls", "-a", "--format", "{{.Names}}",
                             "--filter", "name=" + name])
        if (volumes.returncode != 0 or containers.returncode != 0 or
                volume in volumes.stdout.decode("utf-8").splitlines() or
                name in containers.stdout.decode("utf-8").splitlines()):
            result["code"] = "FRESH_NAMES_REJECTED"
            return result
        result["stages"].append("FRESH_NAMES_VERIFIED")
        result["code"] = "VOLUME_CREATE_FAILED"
        volume_attempted = True
        created_volume = docker(["volume", "create", "--driver", "local",
                                 "--label", "com.knowweave.clone.setup-diag=" + batch_id,
                                 "--name", volume])
        if created_volume.returncode != 0 or created_volume.stdout.decode("ascii").strip() != volume:
            result["code"] = "VOLUME_CREATE_FAILED"
            return result
        result["stages"].append("VOLUME_CREATED")
        result["code"] = "VOLUME_INSPECT_FAILED"
        volume_facts = _one_json(docker(["volume", "inspect", volume]))
        if not _volume_matches(volume_facts, volume, batch_id):
            result["code"] = "VOLUME_REJECTED"
            return result
        result["stages"].append("VOLUME_VERIFIED")
        result["code"] = "SETUP_COMMAND_FAILED"
        command = runner._clone_setup_command(volume, image, batch_id, uid, gid)
        result["code"] = "HELPER_CREATE_FAILED"
        # Docker can create the container and then lose the CLI response.
        # Without an inspected exact ID, leave it untouched for review.
        result["cleanup"] = "UNCONFIRMED"
        created_helper = docker(command)
        if created_helper.returncode != 0:
            result["code"] = "HELPER_CREATE_FAILED"
            result["cleanup"] = "UNCONFIRMED"
            return result
        helper_id = created_helper.stdout.decode("ascii").strip()
        if not HEX64.fullmatch(helper_id):
            result["code"] = "HELPER_ID_REJECTED"
            result["cleanup"] = "UNCONFIRMED"
            return result
        result["helper_id"] = helper_id
        result["cleanup"] = "UNCONFIRMED"
        result["stages"].append("HELPER_CREATED")
        result["code"] = "PRESTART_INSPECT_FAILED"
        prestart = _one_json(docker(["container", "inspect", helper_id]))
        result["observed"]["prestart"] = _projection(
            prestart, helper_id, image, image_id, name, batch_id, volume)
        result["stages"].append("PRESTART_INSPECTED")
        result["code"] = "IDENTITY_REJECTED"
        runner._verify_clone_helper(prestart, helper_id, image, ZERO_ID, volume,
                                    batch_id=batch_id, uid=uid, gid=gid,
                                    image_id=image_id, kind="setup")
        if (prestart.get("State") or {}).get("Running") is not False:
            return result
        verified_identity = True
        result["stages"].append("IDENTITY_VERIFIED")
        result["code"] = "HELPER_START_FAILED"
        started = docker(["start", "--attach", helper_id], timeout=1200)
        result["start_cli_exit_code"] = started.returncode
        result["stages"].append("HELPER_STARTED")
        result["code"] = "FINAL_INSPECT_FAILED"
        final = _one_json(docker(["container", "inspect", helper_id]))
        result["observed"]["final"] = _projection(
            final, helper_id, image, image_id, name, batch_id, volume)
        result["code"] = "FINAL_IDENTITY_REJECTED"
        runner._verify_clone_helper(final, helper_id, image, ZERO_ID, volume,
                                    batch_id=batch_id, uid=uid, gid=gid,
                                    image_id=image_id, kind="setup")
        state = final.get("State") or {}
        if (state.get("Running") is not False or
                state.get("ExitCode") != 0 or started.returncode != 0):
            result["code"] = "HELPER_EXIT_FAILED"
            return result
        result["stages"].append("EXIT_VERIFIED")
        result["code"] = "CLONE_SETUP_DIAG_PASSED"
    except Exception as error:
        result["failure_type"] = type(error).__name__
    finally:
        if verified_identity:
            try:
                removed = docker(["container", "rm", "-f", helper_id])
                if removed.returncode == 0:
                    result["cleanup"] = "EXACT_ID_REMOVED"
                    result["stages"].append("EXACT_ID_REMOVED")
                else:
                    result["cleanup"] = "UNCONFIRMED"
                    if result["code"] == "CLONE_SETUP_DIAG_PASSED":
                        result["code"] = "EXACT_ID_CLEANUP_FAILED"
            except Exception:
                result["cleanup"] = "UNCONFIRMED"
                if result["code"] == "CLONE_SETUP_DIAG_PASSED":
                    result["code"] = "EXACT_ID_CLEANUP_FAILED"
        if volume_attempted:
            try:
                retained = _one_json(docker(["volume", "inspect", volume]))
                result["volume_retained"] = _volume_matches(retained, volume, batch_id)
                if result["volume_retained"]:
                    result["stages"].append("VOLUME_RETAINED")
                elif result["code"] == "CLONE_SETUP_DIAG_PASSED":
                    result["code"] = "VOLUME_RETENTION_UNCONFIRMED"
            except Exception:
                if result["code"] == "CLONE_SETUP_DIAG_PASSED":
                    result["code"] = "VOLUME_RETENTION_UNCONFIRMED"
    return result


def _volume_matches(facts, volume, batch_id):
    return (facts.get("Name") == volume and facts.get("Driver") == "local" and
            facts.get("Scope") == "local" and facts.get("Options") in (None, {}) and
            (facts.get("Labels") or {}).get("com.knowweave.clone.setup-diag") == batch_id)


def _batch_uuid(value):
    try:
        parsed = uuid.UUID(value)
    except (ValueError, TypeError, AttributeError):
        parsed = None
    if parsed is None or parsed.version != 4 or str(parsed) != value:
        raise argparse.ArgumentTypeError("canonical new UUIDv4 required")
    return value


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runner", type=Path, required=True,
                        help="root-installed runner whose exact SHA-256 is pinned in this script")
    parser.add_argument("--image", required=True,
                        help="already local pinned PostgreSQL 18 image digest")
    parser.add_argument("--batch-id", type=_batch_uuid, required=True,
                        help="new, unused UUIDv4 naming the diagnostic volume and helper")
    parser.add_argument("--uid", required=True, type=int)
    parser.add_argument("--gid", required=True, type=int)
    args = parser.parse_args(argv)
    if os.name != "posix" or os.geteuid() != 0 or not Path("/var/run/docker.sock").is_socket():
        result = {"code": "LOCAL_ROOTFUL_DOCKER_REQUIRED", "stages": [],
                  "cleanup": "NOT_CREATED", "volume_retained": None}
    else:
        result = diagnose(args.runner, args.image, args.uid, args.gid,
                          batch_id=args.batch_id)
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0 if result["code"] == "CLONE_SETUP_DIAG_PASSED" else 1


if __name__ == "__main__":
    sys.exit(main())
