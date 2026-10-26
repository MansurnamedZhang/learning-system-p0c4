"""Read-only Task9 inventory. This report cannot admit signing or recovery.

Run through an existing authorized Linux session. No key/trust/registry contents
are read, no privilege escalation is attempted, and nothing is installed.
Permission refusal is UNKNOWN, never evidence that an installation is absent.
"""
import json
import os
import platform
import stat
import subprocess


PATHS = (
    "/var/lib/knowweave-c4",
    "/var/lib/knowweave-c4/registry",
    "/var/lib/knowweave-c4/registry/authority.json",
    "/var/lib/knowweave-c4/verifier",
    "/var/lib/knowweave-c4/verifier/trust.json",
    "/var/lib/knowweave-c4/verifier/destination.pk8",
)


def observe_path(path, lstat=os.lstat):
    """Only metadata; never follow a known symlink or read file contents."""
    if path not in PATHS:
        raise ValueError("path outside fixed Task9 metadata inventory")
    parts = path.strip("/").split("/")
    for count in range(1, len(parts) + 1):
        current = "/" + "/".join(parts[:count])
        try:
            item = lstat(current)
        except PermissionError:
            return {"path": path, "state": "UNKNOWN_PERMISSION_DENIED"}
        except FileNotFoundError:
            return {"path": path, "state": "ABSENT_AT_OBSERVATION"}
        except OSError:
            return {"path": path, "state": "UNKNOWN_METADATA_ERROR"}
        if stat.S_ISLNK(item.st_mode):
            return {"path": path, "state": "REFUSED_SYMLINK_COMPONENT"}
        if count != len(parts) and not stat.S_ISDIR(item.st_mode):
            return {"path": path, "state": "REFUSED_NON_DIRECTORY_COMPONENT"}
    return {
        "path": path,
        "state": "METADATA_OBSERVED_NOT_CONTENT_VALIDATED",
        "kind": "directory" if stat.S_ISDIR(item.st_mode) else
                "regular" if stat.S_ISREG(item.st_mode) else "other",
        "mode": format(stat.S_IMODE(item.st_mode), "04o"),
        "uid": item.st_uid,
        "gid": item.st_gid,
        "device": item.st_dev,
        "inode": item.st_ino,
        "links": item.st_nlink,
        "bytes": item.st_size,
    }


def observe_mount(run=subprocess.run):
    # Deliberately omit SOURCE, OPTIONS and mount credentials. This is a
    # containing-mount observation, never a physical failure-domain proof.
    try:
        result = run(
            ["/usr/bin/findmnt", "--json", "--target", PATHS[0],
             "--output", "TARGET,FSTYPE,MAJ:MIN"],
            stdin=subprocess.DEVNULL, capture_output=True, timeout=10,
            check=False, env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"},
        )
        if result.returncode or len(result.stdout) > 16384:
            return {"state": "UNKNOWN_CONTAINING_MOUNT"}
        data = json.loads(result.stdout)
        rows = data.get("filesystems")
        if not isinstance(rows, list) or len(rows) != 1:
            return {"state": "UNKNOWN_CONTAINING_MOUNT"}
        row = rows[0]
        if not isinstance(row, dict):
            return {"state": "UNKNOWN_CONTAINING_MOUNT"}
        selected = {key: row.get(key) for key in ("target", "fstype", "maj:min")}
        if any(not isinstance(v, str) or len(v) > 256 for v in selected.values()):
            return {"state": "UNKNOWN_CONTAINING_MOUNT"}
        return {"state": "CONTAINING_MOUNT_METADATA_ONLY", **selected}
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return {"state": "UNKNOWN_CONTAINING_MOUNT"}


def collect(lstat=os.lstat, run=subprocess.run):
    if platform.system() != "Linux":
        return {
            "schema": 1,
            "status": "TASK9_METADATA_PROBE_UNSUPPORTED_NOT_ACCEPTANCE",
            "task9_accepted": False,
        }
    return {
        "schema": 1,
        "status": "TASK9_READ_ONLY_METADATA_OBSERVATIONS_NOT_ACCEPTANCE",
        "uid": os.geteuid(),
        "paths": [observe_path(path, lstat=lstat) for path in PATHS],
        "containing_mount": observe_mount(run=run),
        "independent_target": "UNVERIFIED",
        "private_key_custody": "UNVERIFIED_NO_CONTENT_READ",
        "installed_trust_and_registry_validity": "UNVERIFIED_NO_CONTENT_READ",
        "reviewed_build_key_pin": "UNVERIFIED",
        "cross_operation_first_sign_and_publish": "NOT_TESTED",
        "task9_accepted": False,
        "complete_backup_issued": False,
        "credentials_read_or_written": False,
        "privilege_escalation_attempted": False,
        "resources_modified": False,
        "limitations": [
            "Metadata is a point-in-time advisory observation, not a trusted admission or stable-open proof.",
            "Different device, UUID, mount, container or readable key metadata cannot prove an independent failure domain or custody.",
            "Unknown permission states must remain unknown; this probe does not retry through another privilege path.",
        ],
    }


def main():
    import sys
    if len(sys.argv) != 1:
        raise SystemExit("no paths, credentials, key data or caller overrides accepted")
    payload = json.dumps(collect(), sort_keys=True, separators=(",", ":"))
    if len(payload.encode()) > 32768:
        raise SystemExit("metadata report bound exceeded")
    print(payload)


if __name__ == "__main__":
    main()
