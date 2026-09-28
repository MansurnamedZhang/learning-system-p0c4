#!/usr/bin/env python3
"""Root-only, fail-closed host driver for one isolated P0-C4 source capture.

The driver holds a private flock for the entire child process. It never logs
environment values or raw Docker inspect output. The existing Docker daemon
and all its operators are explicitly trusted during this maintenance window;
this is operational fencing, not a defense against a host/Docker operator.
It is
not a production backup scheduler and does not publish a complete receipt.
Install the reviewed script and manager binary in root-owned, non-writable-by-
others paths; use fresh root-owned 0700 evidence/control roots and a dedicated
P0-C4 Compose project. A small manager container based on the pinned PG18
image joins only that project's internal network, without publishing a port.
Linux/Docker/PostgreSQL execution must be
validated in a fresh isolated acceptance project before this can be accepted.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import time
from urllib.parse import urlsplit
import uuid


class IsolationError(RuntimeError):
    pass


DOCKER_BIN = "/usr/bin/docker"
DOCKER_SOCKET = "/var/run/docker.sock"
DOCKER_HOST = "unix://" + DOCKER_SOCKET
MANAGER_IMAGE = "postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
MANAGER_ENV_KEYS = {
    "TEST_ADMIN_DATABASE_URL", "TEST_C4_CONTROL_ROOT", "TEST_C4_PIN_ROOT",
    "TEST_C4_ASSET_ROOT", "TEST_C4_ASSET_STAGE_ROOT", "TEST_C4_PGDUMP_BIN",
    "TEST_C4_PGDUMP_FAIL_BIN", "TEST_C4_PGPASSFILE", "TEST_C4_PGHOST",
    "TEST_C4_PGPORT", "TEST_C4_FAILURE_KIND",
}


def manager_environment(inherited):
    if any(k in {"TEST_DATABASE_URL", "DATABASE_URL", "PGPASSWORD"} or k.startswith("TEST_C4_RUNTIME_") for k in inherited):
        raise IsolationError("runtime credentials present in driver environment")
    result = {k: inherited[k] for k in MANAGER_ENV_KEYS if k in inherited}
    result.update({"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/root"})
    return result


def validate_admin_endpoint(url, database):
    parsed = urlsplit(url)
    if (parsed.scheme not in {"postgres", "postgresql"}
            or parsed.username != "learning_admin" or parsed.hostname != "pg"
            or parsed.port != 5432 or parsed.path != "/" + database
            or parsed.query != "application_name=knowweave_c4_manager" or parsed.fragment):
        raise IsolationError("management DB endpoint must be this PG namespace and database")


def _docker(*args):
    result = subprocess.run(
        [DOCKER_BIN, *args], capture_output=True, text=True, check=False,
        env={"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/root", "DOCKER_HOST": DOCKER_HOST}, timeout=30,
    )
    if result.returncode:
        raise IsolationError("Docker inspection or stop failed")
    return result.stdout


def _inspect_project(project):
    ids = _docker("ps", "-aq", "--filter", f"label=com.docker.compose.project={project}").split()
    if not ids:
        raise IsolationError("project has no containers")
    containers = json.loads(_docker("inspect", *ids))
    network_ids = _docker("network", "ls", "-q", "--filter", f"label=com.docker.compose.project={project}").split()
    if len(network_ids) != 1:
        raise IsolationError("project must have exactly one network")
    networks = json.loads(_docker("network", "inspect", *network_ids))
    return containers, networks


def assess_project(project, containers, networks, allowed_manager_name=None):
    if not project.startswith("learning-system-p0c4-") or not containers or len(networks) != 1:
        raise IsolationError("project identity or network count")
    network = networks[0]
    if network.get("Labels", {}).get("com.docker.compose.project") != project or network.get("Internal") is not True:
        raise IsolationError("project network is not isolated")
    services = {}
    sanitized = []
    for item in containers:
        config = item.get("Config") or {}
        labels = config.get("Labels") or {}
        if labels.get("com.docker.compose.project") != project:
            raise IsolationError("foreign project container")
        service = labels.get("com.docker.compose.service")
        if service not in {"pg", "runtime", "worker", "c4-manager"} or service in services:
            raise IsolationError("unknown or duplicate project service")
        running = item.get("State", {}).get("Running") is True
        if running and service != "pg" and not (service == "c4-manager" and item.get("Name") == "/" + str(allowed_manager_name)):
            raise IsolationError("business or other manager process still running")
        mounts = [str(m.get("Destination", "")) for m in item.get("Mounts") or []]
        env_keys = [str(e).split("=", 1)[0] for e in config.get("Env") or []]
        if service == "c4-manager" and (
            any("runtime" in m.lower() or "database_url" in m.lower() for m in mounts)
            or any("runtime" in k.lower() or k.upper() == "DATABASE_URL" for k in env_keys)
        ):
            raise IsolationError("manager has runtime credentials")
        ports = item.get("NetworkSettings", {}).get("Ports") or {}
        attachments = item.get("NetworkSettings", {}).get("Networks") or {}
        if set(attachments) != {network.get("Name")}:
            raise IsolationError("container attached outside isolated project network")
        if service == "pg" and any(bindings for bindings in ports.values()):
            raise IsolationError("PostgreSQL port published")
        services[service] = item
        sanitized.append({
            "id": str(item.get("Id", "")), "service": service, "running": running,
            "mount_destinations": sorted(mounts), "environment_keys": sorted(env_keys),
            "published_port_count": sum(len(v or []) for v in ports.values()),
        })
    if "pg" not in services or not services["pg"].get("State", {}).get("Running"):
        raise IsolationError("PostgreSQL not running")
    return {
        "runtime_running": 0, "worker_running": 0, "other_admin_processes": 0,
        "postgres_published_ports": 0, "network_internal": True,
        "manager_runtime_secret_mounts": 0, "manager_runtime_env_keys": 0,
        "containers": sorted(sanitized, key=lambda c: c["service"]),
        "network": {"name": network.get("Name"), "internal": True, "project": project},
    }


def _private_root(path):
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or stat.S_IMODE(info.st_mode) != 0o700:
        raise IsolationError("driver root must be root-owned 0700 directory")
    for parent in path.parents:
        info = parent.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or stat.S_IMODE(info.st_mode) & 0o022:
            raise IsolationError("driver root has a writable or untrusted ancestor")


def _trusted_executable(path):
    path = Path(path)
    if not path.is_absolute():
        raise IsolationError("trusted executable path must be absolute")
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid != 0 or stat.S_IMODE(info.st_mode) & 0o022:
        raise IsolationError("trusted executable is not root-owned and immutable to others")
    for parent in path.parents:
        info = parent.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or stat.S_IMODE(info.st_mode) & 0o022:
            raise IsolationError("trusted executable has a writable or untrusted ancestor")


def _private_file(path):
    path = Path(path)
    if not path.is_absolute():
        raise IsolationError("private file path must be absolute")
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid != 0 or stat.S_IMODE(info.st_mode) != 0o600:
        raise IsolationError("private file must be root:root 0600")
    for parent in path.parents:
        info = parent.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or stat.S_IMODE(info.st_mode) & 0o022:
            raise IsolationError("private file has writable or untrusted ancestor")


def _atomic_private_file(path, payload):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as output:
        output.write(payload)
        output.flush()
        os.fsync(output.fileno())
    dirfd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(dirfd)
    finally:
        os.close(dirfd)


def _socket_exclusive():
    info = os.stat(DOCKER_SOCKET, follow_symlinks=False)
    mode = stat.S_IMODE(info.st_mode)
    if not stat.S_ISSOCK(info.st_mode) or info.st_uid != 0 or mode & 0o007:
        raise IsolationError("trusted host Docker socket unavailable")


def _daemon_preflight():
    result = subprocess.run(
        [DOCKER_BIN, "info", "--format", "{{.ID}}"],
        capture_output=True, text=True, check=False, timeout=15,
        env={"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/root", "DOCKER_HOST": DOCKER_HOST},
    )
    if result.returncode or not result.stdout.strip():
        raise IsolationError("Docker daemon identity unavailable")
    return result.stdout.strip()


def _stop_business(project, containers):
    ids = [c["Id"] for c in containers if c.get("Config", {}).get("Labels", {}).get("com.docker.compose.service") in {"runtime", "worker"} and c.get("State", {}).get("Running") is True]
    if ids:
        _docker("stop", "--time", "10", *ids)
    # Inspect again after stop; do not trust a successful stop exit alone.
    return _inspect_project(project)


def probe_runtime_denied(pg_id, database):
    """A real connection attempt, without giving runtime credentials to manager."""
    result = subprocess.run(
        [DOCKER_BIN, "exec", "-e", "LC_ALL=C", pg_id, "psql", "-X", "-q", "-U", "learning_runtime", "-d", database, "-c", "SELECT 1"],
        capture_output=True, text=True, check=False, timeout=15,
        env={"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/root", "DOCKER_HOST": DOCKER_HOST},
    )
    if result.returncode == 0 or "permission denied for database" not in result.stderr.lower():
        raise IsolationError("runtime connect denial could not be proven")


def _start_runtime_transaction(pg_id, database):
    """Hold one real runtime transaction until the source DB gate closes."""
    process = subprocess.Popen(
        [DOCKER_BIN, "exec", "-e", "LC_ALL=C", pg_id, "psql", "-X", "-q", "-U", "learning_runtime", "-d", database,
         "-c", "BEGIN; SELECT pg_sleep(300)"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        env={"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/root", "DOCKER_HOST": DOCKER_HOST},
    )
    for _ in range(60):
        rows = _docker(
            "exec", pg_id, "psql", "-X", "-Atq", "-U", "postgres", "-d", "postgres",
            "-c", f"SELECT pid FROM pg_catalog.pg_stat_activity WHERE datname='{database}' AND usename='learning_runtime' AND query='BEGIN; SELECT pg_sleep(300)'",
        ).split()
        if len(rows) == 1 and rows[0].isdigit():
            return process, int(rows[0])
        if process.poll() is not None:
            break
        time.sleep(0.05)
    process.terminate()
    process.wait(timeout=10)
    raise IsolationError("existing runtime transaction could not be established")


def _end_runtime_transaction(pg_id, database, backend_pid, process):
    result = _docker(
        "exec", pg_id, "psql", "-X", "-Atq", "-U", "postgres", "-d", "postgres",
        "-c", f"SELECT pg_terminate_backend({backend_pid})",
    ).strip()
    if result != "t":
        raise IsolationError("existing runtime transaction did not terminate")
    process.wait(timeout=10)


def _monitor_postgres(pg_id, database, require_closed):
    row = _docker(
        "exec", pg_id, "psql", "-X", "-Atq", "-U", "postgres", "-d", "postgres",
        "-c", f"SELECT has_database_privilege('learning_runtime','{database}','CONNECT')::int,"
              f"(SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='{database}' "
              "AND pid<>pg_backend_pid() AND (usename <> 'learning_admin' OR "
              "application_name NOT IN ('knowweave_c4_manager','knowweave_c4_pg_dump')))"
    ).strip().split("|")
    if len(row) != 2 or row[0] not in {"0", "1"} or not row[1].isdigit():
        raise IsolationError("PostgreSQL live gate monitor returned invalid facts")
    if int(row[1]) != 0 or (require_closed and row[0] != "0"):
        raise IsolationError("PostgreSQL gate or session isolation changed during capture")


def run(args):
    import fcntl
    if os.geteuid() != 0:
        raise IsolationError("root is required")
    _trusted_executable(Path(__file__).absolute())
    _trusted_executable(DOCKER_BIN)
    _socket_exclusive()
    engine_id = _daemon_preflight()
    backup_id = str(uuid.UUID(args.backup_id))
    db_marker = args.database.removeprefix("learning_backup_c4_task3_")
    if (backup_id != args.backup_id or db_marker == args.database
            or str(uuid.UUID(db_marker)) != db_marker):
        raise IsolationError("dedicated UUID backup/database required")
    if not args.project.startswith("learning-system-p0c4-") or not args.manager or not Path(args.manager[0]).is_absolute():
        raise IsolationError("exact isolated project and absolute manager executable required")
    _trusted_executable(args.manager[0])
    root = Path(args.private_root)
    _private_root(root)
    control_root = Path(args.control_root)
    _private_root(control_root)
    if root == control_root:
        raise IsolationError("driver evidence and source controls must be separate")
    lock_path = root / f"isolation-{backup_id}.lock"
    lockfd = os.open(lock_path, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(lockfd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        before, _ = _inspect_project(args.project)
        containers, networks = _stop_business(args.project, before)
        facts = assess_project(args.project, containers, networks)
        pg = next(c for c in containers if c["Config"]["Labels"]["com.docker.compose.service"] == "pg")
        evidence = json.dumps(facts, sort_keys=True, separators=(",", ":")).encode()
        evidence_name = f"inspection-{backup_id}.json"
        _atomic_private_file(root / evidence_name, evidence)
        proof = {
            "format_version": 2, "backup_id": backup_id, "database": args.database,
            "compose_project": args.project, "observed_unix_ms": int(time.time() * 1000),
            "driver_pid": os.getpid(), "inspection_file": evidence_name,
            "docker_inspection_sha256": hashlib.sha256(evidence).hexdigest(),
            **{k: facts[k] for k in (
                "runtime_running", "worker_running", "other_admin_processes",
                "postgres_published_ports", "network_internal",
                "manager_runtime_secret_mounts", "manager_runtime_env_keys")},
        }
        _atomic_private_file(root / f"isolation-{backup_id}.json", json.dumps(proof, sort_keys=True, separators=(",", ":")).encode())
        child_env = manager_environment(os.environ)
        validate_admin_endpoint(child_env.get("TEST_ADMIN_DATABASE_URL", ""), args.database)
        for name in ("TEST_C4_PIN_ROOT", "TEST_C4_ASSET_ROOT", "TEST_C4_ASSET_STAGE_ROOT"):
            if name not in child_env:
                raise IsolationError(f"missing required manager root: {name}")
            _private_root(Path(child_env[name]))
        if "TEST_C4_PGPASSFILE" not in child_env:
            raise IsolationError("missing required private pgpassfile")
        _private_file(child_env["TEST_C4_PGPASSFILE"])
        if child_env.get("TEST_C4_FAILURE_KIND") == "pg_dump_exit":
            _trusted_executable(child_env.get("TEST_C4_PGDUMP_FAIL_BIN", ""))
        child_env.update({"PATH": "/usr/lib/postgresql/18/bin:/usr/local/bin:/usr/bin:/bin", "HOME": "/root", "C4_ISOLATION_ROOT": str(root)})
        child_env.update({
            "TEST_C4_ISOLATION_ATTESTATION": str(root / f"isolation-{backup_id}.json"),
            "TEST_C4_BACKUP_ID": backup_id,
            "TEST_C4_COMPOSE_PROJECT": args.project,
            "TEST_C4_TASK3_DATABASE_NAME": args.database,
            "TEST_C4_PGHOST": "pg",
            "TEST_C4_PGPORT": "5432",
            "TEST_C4_CONTROL_ROOT": str(control_root),
            "TEST_C4_PGDUMP_BIN": "/usr/lib/postgresql/18/bin/pg_dump",
        })
        manager_name = f"{args.project}-c4-manager-{backup_id[:8]}"
        mounts = [(args.manager[0], "/usr/local/bin/knowweave-c4-manager", True),
                  (str(root), str(root), True), (str(control_root), str(control_root), False),
                  (child_env["TEST_C4_PIN_ROOT"], child_env["TEST_C4_PIN_ROOT"], False),
                  (child_env["TEST_C4_ASSET_ROOT"], child_env["TEST_C4_ASSET_ROOT"], False),
                  (child_env["TEST_C4_ASSET_STAGE_ROOT"], child_env["TEST_C4_ASSET_STAGE_ROOT"], False),
                  (child_env["TEST_C4_PGPASSFILE"], child_env["TEST_C4_PGPASSFILE"], True)]
        if child_env.get("TEST_C4_FAILURE_KIND") == "pg_dump_exit":
            failure_bin = child_env["TEST_C4_PGDUMP_FAIL_BIN"]
            mounts.append((failure_bin, failure_bin, True))
        command = [DOCKER_BIN, "run", "--rm", "--pull=never", "--name", manager_name,
                   "--label", f"com.docker.compose.project={args.project}",
                   "--label", "com.docker.compose.service=c4-manager",
                   "--network", facts["network"]["name"], "--read-only", "--cap-drop=ALL",
                   "--security-opt", "no-new-privileges", "--user", "0:0", "--pids-limit", "128",
                   "--memory", "2g", "--cpus", "2", "--tmpfs", "/tmp:rw,nosuid,nodev,size=128m"]
        for source, target, readonly in mounts:
            command.extend(["--mount", f"type=bind,src={source},dst={target}" + (",readonly" if readonly else "")])
        for key in sorted(child_env):
            command.extend(["--env", key])
        command.extend([MANAGER_IMAGE, "/usr/local/bin/knowweave-c4-manager", *args.manager[1:]])
        runtime_process, runtime_backend = _start_runtime_transaction(pg["Id"], args.database)
        try:
            child = subprocess.Popen(
                command, env={**child_env, "DOCKER_HOST": DOCKER_HOST}, close_fds=True,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
        except BaseException:
            _end_runtime_transaction(pg["Id"], args.database, runtime_backend, runtime_process)
            raise
        probe_done = False
        runtime_ended = False
        closed = control_root / f"{backup_id}.control" / "closed.json"
        probe_path = root / f"runtime-connect-denied-{backup_id}.json"
        try:
            while child.poll() is None:
                time.sleep(0.5)
                _socket_exclusive()
                if _daemon_preflight() != engine_id:
                    raise IsolationError("Docker daemon identity changed during capture")
                live, live_networks = _inspect_project(args.project)
                assess_project(args.project, live, live_networks, allowed_manager_name=manager_name)
                if not probe_done and closed.is_file():
                    pg_id = next(c["Id"] for c in live if c["Config"]["Labels"]["com.docker.compose.service"] == "pg")
                    probe_runtime_denied(pg_id, args.database)
                    _atomic_private_file(probe_path, json.dumps({"backup_id": backup_id, "database": args.database, "result": "runtime_connect_denied"}, sort_keys=True, separators=(",", ":")).encode())
                    probe_done = True
                    _end_runtime_transaction(pg["Id"], args.database, runtime_backend, runtime_process)
                    runtime_ended = True
                if runtime_ended:
                    release_ready = control_root / f"{backup_id}.control" / "release-ready.json"
                    _monitor_postgres(pg["Id"], args.database, require_closed=not release_ready.is_file())
            if child.returncode:
                raise IsolationError("manager failed; DB gate requires inspection before reuse")
            if not probe_done:
                raise IsolationError("no actual runtime connect denial observed during closed gate")
            live, live_networks = _inspect_project(args.project)
            final_facts = assess_project(args.project, live, live_networks)
            end_bytes = json.dumps(final_facts, sort_keys=True, separators=(",", ":")).encode()
            _atomic_private_file(root / f"inspection-end-{backup_id}.json", end_bytes)
        finally:
            if child.poll() is None:
                try:
                    _docker("stop", "--time", "1", manager_name)
                except IsolationError:
                    pass
                child.terminate()
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
            if not runtime_ended and runtime_process.poll() is None:
                _end_runtime_transaction(pg["Id"], args.database, runtime_backend, runtime_process)
        _atomic_private_file(root / f"driver-result-{backup_id}.json", json.dumps({
            "status": "SOURCE_CAPTURE_DRIVER_PASSED_NOT_COMPLETE",
            "daemon_id": engine_id,
            "start_inspection_sha256": hashlib.sha256(evidence).hexdigest(),
            "end_inspection_sha256": hashlib.sha256(end_bytes).hexdigest(),
        }, sort_keys=True, separators=(",", ":")).encode())
    except Exception as exc:
        try:
            _atomic_private_file(root / f"driver-result-{backup_id}.json", json.dumps({
                "status": "FAILED", "error_type": type(exc).__name__, "daemon_id": engine_id,
            }, sort_keys=True, separators=(",", ":")).encode())
        except OSError:
            pass
        raise
    finally:
        os.close(lockfd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backup-id", required=True)
    parser.add_argument("--database", required=True)
    parser.add_argument("--project", required=True)
    parser.add_argument("--private-root", required=True)
    parser.add_argument("--control-root", required=True)
    parser.add_argument("manager", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.manager and args.manager[0] == "--":
        args.manager.pop(0)
    try:
        run(args)
    except (IsolationError, OSError, ValueError, subprocess.TimeoutExpired) as exc:
        print(json.dumps({"status": "FAILED", "type": type(exc).__name__, "reason": str(exc)}), file=sys.stderr)
        return 1
    print(json.dumps({"status": "SOURCE_CAPTURE_DRIVER_PASSED_NOT_COMPLETE"}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
