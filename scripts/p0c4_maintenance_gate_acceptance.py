"""Fresh root-only Linux focused maintenance acceptance; no full backup/restore.

Run only from the user's Linux terminal with reviewed archive/runner pins.
No resume, production resources, runtime services, dump or restore invocation.
Host and local Docker administrators are trusted during this exclusive window.
"""
import argparse
from contextlib import contextmanager
import hashlib
import importlib.util
import io
import ipaddress
import json
import os
from pathlib import Path, PurePosixPath
import re
import secrets
import signal
import stat
import subprocess
import sys
import time
import uuid
import zipfile

class GateError(RuntimeError):
    pass

BASE = Path("/var/lib/knowweave-c4")
ENTRY = "scripts/p0c4_maintenance_gate_acceptance.py"
PG_IMAGE = "postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
BUILDER = "sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b"
PGDUMP = Path("/usr/lib/postgresql/18/bin/pg_dump")
DOCKER = "/usr/bin/docker"
HOST_ENV = {"PATH": "/usr/bin:/bin", "HOME": "/root", "LC_ALL": "C", "DOCKER_HOST": "unix:///var/run/docker.sock"}
HEX64 = re.compile(r"[0-9a-f]{64}\Z")
SECRET_NAMES = ("postgres_password", "admin_password", "runtime_password")
PASSED = "MAINTENANCE_FOCUSED_GATES_PASSED_NOT_FULL_BACKUP_NOT_RESTORE"
TESTS = {
    "prepared_target": "source::linux_gate_tests::prepared_target_transaction_survives_origin_close_and_blocks_source_preflight",
    "prepared_other": "source::linux_gate_tests::prepared_other_database_does_not_block_empty_target",
    "release": "source::linux_gate_tests::release_journal_failure_recloses_runtime_connect",
}


def require(condition, reason):
    if not condition:
        raise GateError(reason)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def json_bytes(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()


def canonical_uuid(value):
    try:
        parsed = uuid.UUID(value)
        return parsed.version == 4 and str(parsed) == value
    except (ValueError, AttributeError, TypeError):
        return False


def identity_for(case_id, other_id):
    require(canonical_uuid(case_id) and canonical_uuid(other_id) and case_id != other_id, "UUID_IDENTITY")
    project = "learning-system-p0c4-maintenance-" + uuid.UUID(case_id).hex
    return {"case_id": case_id, "backup_id": case_id, "project": project,
            "database": "learning_backup_c4_task3_" + case_id,
            "other_database": "learning_backup_c4_task3_" + other_id,
            "network": project + "_test", "volume": project + "_pg", "pg_name": project + "-pg-1"}


def admit_subnets(requested, occupied):
    require(1 <= len(requested) <= 3, "EXPLICIT_SUBNET_REQUIRED")
    try:
        chosen = [ipaddress.ip_network(value, strict=True) for value in requested]
        private = [ipaddress.ip_network(value) for value in ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")]
        require(all(n.version == 4 and n.prefixlen == 24 and any(n.subnet_of(p) for p in private) for n in chosen), "PRIVATE_CANONICAL_SUBNET")
        for index, network in enumerate(chosen):
            require(all(not network.overlaps(other) for other in chosen[:index]), "SUBNET_PAIR_OVERLAP")
            for raw in occupied:
                if raw == "default":
                    continue
                other = ipaddress.ip_network(raw, strict=False)
                require(other.version != 4 or not network.overlaps(other), "SUBNET_HOST_OVERLAP")
    except ValueError:
        raise GateError("SUBNET_UNREADABLE") from None
    return [str(n) for n in chosen]


def admit_fresh(identities, snapshot, builder_name):
    projects = {i["project"] for i in identities}
    require(len(projects) == len(identities), "CASE_ID_REUSE")
    names = {i["pg_name"] for i in identities} | {builder_name}
    for container in snapshot["containers"]:
        labels = container.get("Config", {}).get("Labels") or {}
        name = container.get("Name", "").lstrip("/")
        require(labels.get("com.docker.compose.project") not in projects and name not in names and not any(name.startswith(p + "-") for p in projects), "EXISTING_CONTAINER")
    for kind, key in (("networks", "network"), ("volumes", "volume")):
        for item in snapshot[kind]:
            require(item.get("Name") not in {i[key] for i in identities} and (item.get("Labels") or {}).get("com.docker.compose.project") not in projects, "EXISTING_RESOURCE")


def validate_capabilities(host, cpus, memory, *, builder=False):
    # Docker versions normalize ALL/CAP_ALL; PG's official root entrypoint
    # needs the default chown/setuid capabilities during brand-new initdb.
    drops = host.get("CapDrop") or []
    require(host.get("Privileged") is False and not host.get("CapAdd") and (drops in (["ALL"], ["CAP_ALL"]) if builder else not drops) and host.get("SecurityOpt") in (["no-new-privileges"], ["no-new-privileges:true"]), "UNSAFE_CAPABILITIES")
    require(host.get("NanoCpus") == cpus * 1000000000 and host.get("Memory") == memory and host.get("MemorySwap") == memory, "RESOURCE_BUDGET")
    require(host.get("PidMode", "") == "" and host.get("IpcMode") == "private" and not host.get("Devices") and not host.get("AutoRemove") and not host.get("VolumesFrom") and not host.get("Links") and not host.get("ExtraHosts"), "EXTERNAL_HOST_CAPABILITY")


def expected_pg_env(identity):
    return {"POSTGRES_USER": "postgres", "POSTGRES_DB": "postgres", "POSTGRES_PASSWORD_FILE": "/run/secrets/postgres_password", "POSTGRES_INITDB_ARGS": "--auth-host=scram-sha-256", "C4_DATABASE": identity["database"], "C4_OTHER_DATABASE": identity["other_database"]}


def validate_env(observed, supplied, baseline):
    parsed = {}
    for value in observed:
        key, separator, item = value.partition("=")
        require(separator and key not in parsed, "CONTAINER_ENV_SHAPE")
        parsed[key] = item
    require(parsed == {**baseline, **supplied}, "CONTAINER_ENV_DIFFERS")


def validate_pg(identity, subnet, case, snapshot, image_id, *, running, image_env=None):
    project = identity["project"]
    containers = [c for c in snapshot["containers"] if (c.get("Config", {}).get("Labels") or {}).get("com.docker.compose.project") == project]
    networks = [n for n in snapshot["networks"] if (n.get("Labels") or {}).get("com.docker.compose.project") == project]
    volumes = [v for v in snapshot["volumes"] if (v.get("Labels") or {}).get("com.docker.compose.project") == project]
    require(len(containers) == len(networks) == len(volumes) == 1, "PG_RESOURCE_COUNT")
    pg, network, volume = containers[0], networks[0], volumes[0]
    require(HEX64.fullmatch(pg.get("Id", "")) and HEX64.fullmatch(network.get("Id", "")), "DOCKER_ID")
    config, host = pg["Config"], pg["HostConfig"]
    require(pg["Name"] == "/" + identity["pg_name"] and pg["Image"] == image_id and config.get("Image") == PG_IMAGE and config["Labels"].get("com.docker.compose.service") == "pg" and config.get("Cmd") == ["postgres", "-c", "max_prepared_transactions=16"] and pg["State"]["Running"] is running, "PG_IDENTITY")
    validate_capabilities(host, 2, 4 * 1024 ** 3)
    validate_env(config.get("Env") or [], expected_pg_env(identity), image_env or {})
    require(network.get("Name") == identity["network"] and network.get("Internal") is True and network.get("Driver") == "bridge" and (network.get("IPAM", {}).get("Config") or [{}])[0].get("Subnet") == subnet and len(network["IPAM"]["Config"]) == 1, "PG_NETWORK")
    require(set(network.get("Containers") or {}) == ({pg["Id"]} if running else set()), "NETWORK_FOREIGN_ATTACHMENT")
    require(volume.get("Name") == identity["volume"] and volume.get("Driver") == "local" and not volume.get("Options"), "PG_VOLUME")
    settings = pg.get("NetworkSettings") or {}
    attached = settings.get("Networks") or {}
    require(set(attached) == {identity["network"]} and attached[identity["network"]].get("NetworkID") in (network["Id"], "") and host.get("NetworkMode") == identity["network"] and not any((settings.get("Ports") or {}).values()) and not any((host.get("PortBindings") or {}).values()), "PG_NETWORK_ATTACHMENT")
    mounts = {(m.get("Destination"), m.get("Type"), m.get("Source"), m.get("RW")) for m in pg.get("Mounts") or []}
    expected = {("/var/lib/postgresql", "volume", volume["Mountpoint"], True), ("/docker-entrypoint-initdb.d/10-maintenance.sh", "bind", str(case / "initdb.sh"), False)}
    expected |= {("/run/secrets/" + name, "bind", str(case / "secrets/pg" / name), False) for name in SECRET_NAMES}
    require(mounts == expected and len(pg["Mounts"]) == len(expected) and next(m for m in pg["Mounts"] if m["Type"] == "volume").get("Name") == identity["volume"], "PG_MOUNT_DIFFERS")
    ip = attached[identity["network"]].get("IPAddress", "")
    if running:
        require(ipaddress.ip_address(ip) in ipaddress.ip_network(subnet) and ipaddress.ip_address(ip) not in (ipaddress.ip_network(subnet).network_address, ipaddress.ip_network(subnet).broadcast_address), "PG_IP_DIFFERS")
    return {"container_id": pg["Id"], "network_id": network["Id"], "volume_name": volume["Name"], "ip": ip}


def validate_secrets(values):
    require(type(values) is dict and set(values) == set(SECRET_NAMES) and all(type(value) is str and HEX64.fullmatch(value) for value in values.values()) and len(set(values.values())) == 3, "SECRET_SHAPE")
    return values


def child_environment(kind, identity, case, host, values):
    validate_secrets(values)
    ip = ipaddress.ip_address(host)
    require(ip.version == 4 and ip.is_private and not ip.is_loopback and not ip.is_link_local and not ip.is_multicast and str(ip) == host, "CHILD_ENDPOINT")
    require(kind in TESTS, "CASE_KIND")
    environment = {"PATH": "/usr/lib/postgresql/18/bin:/usr/bin:/bin", "HOME": "/root", "LC_ALL": "C", "RUST_BACKTRACE": "0"}
    prefix = "TEST_C4_RELEASE_" if kind == "release" else "TEST_C4_PREPARED_"
    url = lambda database: "postgresql://learning_admin:" + values["admin_password"] + "@" + host + ":5432/" + database + "?application_name=knowweave_c4_manager"
    environment.update({prefix + "DATABASE_NAME": identity["database"], prefix + "ADMIN_DATABASE_URL": url(identity["database"]), prefix + "CONTROL_ROOT": str(case / "control")})
    if kind != "release":
        environment.update({prefix + "BACKUP_ID": identity["backup_id"], prefix + "COMPOSE_PROJECT": identity["project"], prefix + "ISOLATION_ATTESTATION": str(case / "isolation" / ("isolation-" + identity["backup_id"] + ".json")), prefix + "PIN_ROOT": str(case / "pin"), prefix + "ASSET_ROOT": str(case / "assets"), prefix + "ASSET_STAGE_ROOT": str(case / "asset-stage"), prefix + "PGDUMP_BIN": str(PGDUMP), prefix + "PGPASSFILE": str(case / "secrets/admin.pgpass"), prefix + "PGHOST": host, prefix + "PGPORT": "5432"})
        if kind == "prepared_other":
            environment.update({prefix + "OTHER_DATABASE_NAME": identity["other_database"], prefix + "OTHER_ADMIN_DATABASE_URL": url(identity["other_database"])})
    return environment


def accept_test_output(name, code, output):
    require(name in TESTS.values() and code == 0, "LIBTEST_EXIT")
    lines = [line.strip() for line in output.splitlines() if line.strip()]
    tests = [line for line in lines if line.startswith("test ") and not line.startswith("test result:")]
    summaries = [line for line in lines if line.startswith("test result:")]
    require(tests == ["test " + name + " ... ok"] and lines.count("running 1 test") == 1 and len(summaries) == 1 and re.fullmatch(r"test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; \d+ filtered out; finished in [0-9.]+s", summaries[0]), "LIBTEST_COMPLETED_COUNT")
    require(len(lines) == 3, "LIBTEST_AMBIGUOUS_OUTPUT")
    return {"test": name, "exit_code": code, "passed": 1, "failed": 0, "ignored": 0, "output_sha256": digest(output.encode())}


def accept_listing(code, output):
    require(code == 0, "LIBTEST_LIST_EXIT")
    names = [line.removesuffix(": test") for line in output.splitlines() if line.endswith(": test")]
    require(all(names.count(name) == 1 for name in TESTS.values()), "LIBTEST_REQUIRED_NAME")
    return len(TESTS)


def select_artifact(output):
    artifacts = []
    for line in output.splitlines():
        row = json.loads(line)
        if row.get("reason") == "compiler-artifact" and row.get("manifest_path") == "/reviewed/crates/learning-backup/Cargo.toml" and row.get("target", {}).get("name") == "learning_backup" and row.get("target", {}).get("kind") == ["lib"] and row.get("profile", {}).get("test") is True:
            artifacts.append(row.get("executable"))
    require(len(artifacts) == 1 and type(artifacts[0]) is str and re.fullmatch(r"/target/debug/deps/learning_backup-[0-9a-f]+", artifacts[0]), "LIB_ARTIFACT_IDENTITY")
    return Path(artifacts[0]).name


def unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "JSON_DUPLICATE_KEY")
        result[key] = value
    return result


def verify_package(content, archive_sha, manifest_sha, commit, runner_sha):
    require(all(type(v) is str and HEX64.fullmatch(v) for v in (archive_sha, manifest_sha, runner_sha)) and re.fullmatch(r"[0-9a-f]{40}", commit or ""), "INPUT_PINS")
    require(len(content) <= 32 * 1024 ** 2 and digest(content) == archive_sha, "ARCHIVE_DIGEST")
    required = {ENTRY, "scripts/p0c4_source_isolation.py", "Cargo.toml", "Cargo.lock", "crates/learning-backup/src/source.rs"}
    with zipfile.ZipFile(io.BytesIO(content)) as archive:
        infos = archive.infolist()
        names = [i.filename for i in infos]
        require(len(names) == len(set(names)) and required.issubset(names) and "SOURCE_MANIFEST.json" in names and len(names) < 1000 and sum(i.file_size for i in infos) <= 64 * 1024 ** 2, "ARCHIVE_INVENTORY")
        for info in infos:
            parts = info.filename.split("/")
            require(not info.is_dir() and not info.flag_bits & 1 and not info.filename.startswith("/") and "\\" not in info.filename and ":" not in info.filename and all(p and p not in (".", "..") for p in parts) and (info.external_attr >> 16) & 0o170000 == stat.S_IFREG, "ARCHIVE_PATH")
        manifest_raw = archive.read("SOURCE_MANIFEST.json")
        require(digest(manifest_raw) == manifest_sha, "MANIFEST_DIGEST")
        manifest = json.loads(manifest_raw, object_pairs_hook=unique_pairs)
        require(type(manifest) is dict and set(manifest) == {"format_version", "commit", "files"} and type(manifest["format_version"]) is int and manifest["format_version"] == 1 and manifest["commit"] == commit and json_bytes(manifest) == manifest_raw, "MANIFEST_IDENTITY")
        files = manifest["files"]
        require(type(files) is list and [i["path"] for i in files] == sorted(set(names) - {"SOURCE_MANIFEST.json"}), "MANIFEST_FILE_SET")
        for item in files:
            require(type(item) is dict and set(item) == {"path", "size", "sha256"} and type(item["size"]) is int and item["size"] >= 0 and type(item["sha256"]) is str and HEX64.fullmatch(item["sha256"]), "MANIFEST_RECORD")
            raw = archive.read(item["path"])
            require(item["size"] == len(raw) and digest(raw) == item["sha256"], "MANIFEST_FILE_BYTES")
        require(digest(archive.read(ENTRY)) == runner_sha, "RUNNER_PACKAGE_DIGEST")
    return manifest, content


def validate_builder(facts, expected, image_env):
    host, config = facts["HostConfig"], facts["Config"]
    require(HEX64.fullmatch(facts.get("Id", "")) and facts.get("Name") == "/" + expected["name"] and facts.get("Image") == BUILDER and config.get("Image") == BUILDER and config.get("Labels") == {"knowweave.c4.maintenance.batch": expected["batch_id"]} and config.get("User") == "0:0" and config.get("WorkingDir") == "/reviewed" and config.get("Entrypoint") == ["/bin/sh"] and config.get("Cmd") == ["-ec", expected["shell"]], "BUILDER_IDENTITY")
    validate_capabilities(host, 4, 8 * 1024 ** 3, builder=True)
    validate_env(config.get("Env") or [], expected["env"], image_env)
    settings = facts.get("NetworkSettings") or {}
    require(host.get("NetworkMode") == "none" and set(settings.get("Networks") or {}).issubset({"none"}) and not any((settings.get("Ports") or {}).values()) and not host.get("PortBindings") and host.get("ReadonlyRootfs") is True and host.get("PidsLimit") == 512, "BUILDER_ISOLATION")
    observed = {(m.get("Type"), m.get("Source"), m.get("Destination"), m.get("RW")) for m in facts.get("Mounts") or []}
    require(observed == {("bind", str(expected["source"]), "/reviewed", False), ("bind", str(expected["target"]), "/target", True)} and len(facts["Mounts"]) == 2, "BUILDER_MOUNTS")
    return facts["Id"]


def sync_dir(path):
    if os.name == "nt":
        return
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic_write(path, payload, mode=0o600):
    temporary = path.with_name("." + path.name + "." + uuid.uuid4().hex + ".tmp")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), mode)
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(payload)
            output.flush()
            os.fsync(output.fileno())
        # Same-filesystem hard link is atomic and cannot replace an existing name.
        os.link(temporary, path, follow_symlinks=False)
        temporary.unlink()
        sync_dir(path.parent)
    finally:
        if temporary.exists():
            temporary.unlink()


def source_digest(source, manifest, *, ownership=True):
    observed = {}
    for path in source.rglob("*"):
        metadata = path.lstat()
        if ownership:
            require(metadata.st_uid == 0 and not stat.S_IMODE(metadata.st_mode) & 0o022, "SOURCE_OWNERSHIP")
        if stat.S_ISREG(metadata.st_mode):
            require(metadata.st_nlink == 1, "SOURCE_HARDLINK")
            raw = path.read_bytes()
            observed[path.relative_to(source).as_posix()] = {"size": len(raw), "sha256": digest(raw)}
        else:
            require(stat.S_ISDIR(metadata.st_mode), "SOURCE_SPECIAL_FILE")
    expected = {i["path"]: {"size": i["size"], "sha256": i["sha256"]} for i in manifest["files"]}
    require(observed == expected, "SOURCE_CHANGED")
    return digest(json_bytes(observed))


def trusted(path, *, mode=None, directory=False):
    require(path.is_absolute() and ".." not in path.parts, "TRUSTED_PATH")
    metadata = path.lstat()
    require(metadata.st_uid == 0 and (stat.S_ISDIR(metadata.st_mode) if directory else stat.S_ISREG(metadata.st_mode)) and not stat.S_IMODE(metadata.st_mode) & 0o022, "TRUSTED_OWNERSHIP")
    require(mode is None or stat.S_IMODE(metadata.st_mode) == mode, "PRIVATE_MODE")
    if not directory:
        require(metadata.st_nlink == 1, "PRIVATE_HARDLINK")
    for parent in path.parents:
        meta = parent.lstat()
        require(meta.st_uid == 0 and stat.S_ISDIR(meta.st_mode) and not stat.S_IMODE(meta.st_mode) & 0o022, "TRUSTED_ANCESTOR")


def private_dir(path):
    path.mkdir(mode=0o700)
    trusted(path, mode=0o700, directory=True)
    sync_dir(path.parent)
    return path


@contextmanager
def exclusive_lock(path, *, new=False):
    import fcntl
    flags = os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC
    if new:
        flags |= os.O_EXCL
    fd = os.open(path, flags, 0o600)
    try:
        trusted(path, mode=0o600)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield
    finally:
        os.close(fd)


class Runner:
    def __init__(self, evidence):
        self.evidence = evidence
        self.counter = 0
        self.secrets = []

    def observe(self, label, command, parser, projection, *, env=None, timeout=60):
        """Inspect in memory; persist only validated least-data metadata.

        Unknown Docker Env/Cmd/mount values and even error text never enter a
        temporary or durable file. Global inventory also uses a Go whitelist.
        Full own-object/image data is returned only to the in-memory validator.
        """
        self.counter += 1
        prefix = self.evidence / (f"{self.counter:04d}-" + label)
        process = subprocess.Popen(command, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   env=HOST_ENV if env is None else env,
                                   close_fds=True, start_new_session=True)
        timed_out = False
        try:
            try:
                output, errors = process.communicate(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                if os.name == "nt":
                    process.kill()
                else:
                    os.killpg(process.pid, signal.SIGKILL)
                output, errors = process.communicate()
            metadata = {"label": label, "exit_code": process.returncode,
                        "timed_out": timed_out, "stderr_suppressed": bool(errors)}
            if timed_out or process.returncode:
                atomic_write(prefix.with_suffix(".process.json"), json_bytes(metadata))
                raise GateError("OBSERVATION_TIMEOUT" if timed_out else "OBSERVATION_EXIT")
            require(len(output) <= 32 * 1024 ** 2 and len(errors) <= 32 * 1024 ** 2, "OBSERVATION_BUDGET")
            try:
                observed = parser(output)
                safe = json_bytes(projection(observed))
            except BaseException:
                atomic_write(prefix.with_suffix(".process.json"), json_bytes({**metadata, "parse_rejected": True}))
                raise GateError("OBSERVATION_PARSE_REJECTED") from None
            atomic_write(prefix.with_suffix(".stdout.projected.json"), safe)
            atomic_write(prefix.with_suffix(".process.json"), json_bytes({**metadata, "projection_sha256": digest(safe)}))
            return observed
        finally:
            if process.poll() is None:
                if os.name == "nt":
                    process.kill()
                else:
                    os.killpg(process.pid, signal.SIGKILL)
                process.wait()

    def run(self, label, command, *, env=None, timeout=60, stdin=None, allowed=(0,)):
        self.counter += 1
        prefix = self.evidence / (f"{self.counter:04d}-" + label)
        stdout = prefix.with_suffix(".stdout.raw")
        stderr = prefix.with_suffix(".stderr.raw")
        # Capture to private files, kill the whole child process group on timeout.
        handles = []
        process = None
        timed_out = False
        try:
            for path in (stdout, stderr):
                handles.append(os.fdopen(os.open(path, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600), "w+b"))
            process = subprocess.Popen(command, stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL, stdout=handles[0], stderr=handles[1], env=HOST_ENV if env is None else env, close_fds=True, start_new_session=True)
            try:
                process.communicate(input=stdin, timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            captures = []
            for handle in handles:
                handle.flush()
                require(handle.tell() <= 32 * 1024 ** 2, "OUTPUT_BUDGET")
                handle.seek(0)
                data = handle.read()
                for secret in sorted(set(self.secrets), key=len, reverse=True):
                    data = data.replace(secret, b"[REDACTED]")
                handle.seek(0)
                handle.write(data)
                handle.truncate()
                handle.flush()
                os.fsync(handle.fileno())
                captures.append(data)
            code = process.returncode
            atomic_write(prefix.with_suffix(".process.json"), json_bytes({"label": label, "exit_code": code, "timed_out": timed_out, "stdout_sha256": digest(captures[0]), "stderr_sha256": digest(captures[1])}))
            require(not timed_out and code in allowed, "PROCESS_TIMEOUT" if timed_out else "PROCESS_EXIT")
            return subprocess.CompletedProcess(command, code, captures[0], captures[1])
        finally:
            if process is not None and process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            for handle in handles:
                handle.close()


def observation_projection(value):
    rows = value if type(value) is list else [value]
    require(all(type(row) is dict for row in rows), "OBSERVATION_OBJECT_SHAPE")
    ids = []
    for row in rows:
        identifier = row.get("Id")
        if type(identifier) is str and re.fullmatch(r"(?:sha256:)?[0-9a-f]{64}", identifier):
            ids.append(identifier)
    return {"object_count": len(rows), "object_ids": ids, "scope": "METADATA_ONLY_NO_ENV_CMD_MOUNT_VALUES"}


def parse_inventory(kind, raw):
    require(kind in ("container", "network", "volume"), "INVENTORY_KIND")
    found = []
    seen = set()
    for line in raw.splitlines():
        row = json.loads(line, object_pairs_hook=unique_pairs)
        expected = {"Name", "Project"}
        if kind != "volume":
            expected.add("Id")
        if kind == "network":
            expected.add("Subnets")
        require(type(row) is dict and set(row) == expected, "INVENTORY_FIELDS")
        name = row["Name"]
        if kind == "network":
            # Docker networks need only a non-whitespace name. Inspection uses
            # the exact SHA ID; names are kept solely for in-memory collision checks.
            require(type(name) is str and bool(name.strip()) and len(name.encode("utf-8")) <= 32 * 1024 ** 2, "INVENTORY_NAME")
        else:
            require(type(name) is str and re.fullmatch(r"/[a-zA-Z0-9][a-zA-Z0-9_.-]*" if kind == "container" else r"[a-zA-Z0-9][a-zA-Z0-9_.-]*", name), "INVENTORY_NAME")
        project = row["Project"]
        # Label values are arbitrary Docker metadata; compare in memory rather
        # than imposing this batch's project spelling on unrelated labels.
        require(project is None or type(project) is str, "INVENTORY_PROJECT")
        identifier = name if kind == "volume" else row["Id"]
        require(kind == "volume" or type(identifier) is str and HEX64.fullmatch(identifier), "INVENTORY_ID")
        require(identifier not in seen, "INVENTORY_DUPLICATE")
        seen.add(identifier)
        projected = {"Name": name, "Labels": {"com.docker.compose.project": project}}
        if kind != "volume":
            projected["Id"] = identifier
        if kind == "container":
            projected["Config"] = {"Labels": projected.pop("Labels")}
        elif kind == "network":
            require(type(row["Subnets"]) is list and all(type(subnet) is str for subnet in row["Subnets"]), "INVENTORY_SUBNETS")
            for subnet in row["Subnets"]:
                ipaddress.ip_network(subnet, strict=False)
            projected["IPAM"] = {"Config": [{"Subnet": subnet} for subnet in row["Subnets"]]}
        found.append(projected)
    return found


def inspect_many(runner, kind, ids, *, inventory=False):
    require(kind in ("container", "network", "volume"), "INSPECT_KIND")
    require(all(type(i) is str and HEX64.fullmatch(i) for i in ids) if kind != "volume" else all(type(i) is str and re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.-]*", i) for i in ids), "INSPECT_ID_SHAPE")
    if not ids:
        return []
    command = [DOCKER, *( ["inspect"] if kind == "container" else [kind, "inspect"])]
    if inventory:
        # Project only the one collision-relevant label, never arbitrary labels,
        # Config.Env, Cmd or mounts from existing unrelated projects.
        fields = '{"Name":{{json .Name}},"Project":{{json (index '
        fields += '.Config.Labels' if kind == "container" else '.Labels'
        fields += ' "com.docker.compose.project")}}'
        if kind != "volume":
            fields += ',"Id":{{json .Id}}'
        if kind == "network":
            fields += ',"Subnets":[{{range $i,$v := .IPAM.Config}}{{if $i}},{{end}}{{json $v.Subnet}}{{end}}]'
        command.extend(["--format", fields + "}"])
        def parser(raw):
            rows = parse_inventory(kind, raw)
            observed = [row["Name"] if kind == "volume" else row["Id"] for row in rows]
            require(len(observed) == len(ids) and set(observed) == set(ids), "INVENTORY_COVERAGE")
            return rows
    else:
        parser = lambda raw: json.loads(raw, object_pairs_hook=unique_pairs)
    command.extend(ids)
    return runner.observe("inspect-" + kind, command, parser, observation_projection)


def snapshot(runner):
    result = {}
    for kind, command in (("containers", ["ps", "-aq", "--no-trunc"]), ("networks", ["network", "ls", "-q", "--no-trunc"]), ("volumes", ["volume", "ls", "-q"])):
        ids = runner.run("list-" + kind, [DOCKER, *command]).stdout.decode().split()
        result[kind] = inspect_many(runner, {"containers": "container", "networks": "network", "volumes": "volume"}[kind], ids, inventory=True)
    return result


def image_environment(facts):
    return dict(value.split("=", 1) for value in facts["Config"]["Env"])


def host_preflight(runner, identities, subnets, builder_name, batch):
    trusted(Path(DOCKER))
    trusted(Path("/usr/bin/ip"))
    socket = Path("/var/run/docker.sock").lstat()
    require(stat.S_ISSOCK(socket.st_mode) and socket.st_uid == 0 and not socket.st_mode & 0o002, "DOCKER_SOCKET")
    daemon = runner.observe("docker-info", [DOCKER, "info", "--format", '{"ID":{{json .ID}},"OSType":{{json .OSType}},"SecurityOptions":{{json .SecurityOptions}}}'], lambda raw: json.loads(raw, object_pairs_hook=unique_pairs), observation_projection)
    require(daemon.get("ID") and daemon.get("OSType") == "linux" and "rootless" not in str(daemon.get("SecurityOptions", [])), "DOCKER_DAEMON")
    images = {}
    for name in (PG_IMAGE, BUILDER):
        # Baseline image Env is needed in memory for exact container comparison;
        # only IDs/counts are persisted, never the baseline values or stderr.
        image_format = '{"Id":{{json .Id}},"RepoDigests":{{json .RepoDigests}},"Config":{"Env":{{json .Config.Env}},"Volumes":{{json (index .Config "Volumes")}}}}'
        facts = runner.observe("image-inspect", [DOCKER, "image", "inspect", "--format", image_format, name], lambda raw: [json.loads(line, object_pairs_hook=unique_pairs) for line in raw.splitlines()], observation_projection)
        require(len(facts) == 1, "PINNED_IMAGE_COUNT")
        facts = facts[0]
        require((facts["Id"] == name if name == BUILDER else PG_IMAGE.split("@", 1)[1] in [d.split("@", 1)[1] for d in facts.get("RepoDigests", [])]) and re.fullmatch(r"sha256:[0-9a-f]{64}", facts["Id"]), "PINNED_IMAGE")
        if name == PG_IMAGE:
            require(facts["Config"].get("Volumes") == {"/var/lib/postgresql": {}}, "PG_IMAGE_VOLUMES")
        else:
            require(not facts["Config"].get("Volumes"), "BUILDER_IMAGE_VOLUMES")
        images[name] = facts
    routes = json.loads(runner.run("routes", ["/usr/bin/ip", "-j", "-4", "route", "show", "table", "all"]).stdout)
    before = snapshot(runner)
    occupied = [r["dst"] for r in routes if r.get("dst")]
    occupied.extend(c["Subnet"] for n in before["networks"] for c in (n.get("IPAM", {}).get("Config") or []) if c.get("Subnet"))
    admit_subnets(subnets, occupied)
    admit_fresh(identities, before, builder_name)
    available = next(int(line.split()[1]) * 1024 for line in Path("/proc/meminfo").read_text().splitlines() if line.startswith("MemAvailable:"))
    disk = os.statvfs(batch)
    require((os.cpu_count() or 0) >= 4 and available >= 10 * 1024 ** 3 and disk.f_bavail * disk.f_frsize >= 20 * 1024 ** 3, "HOST_CAPACITY")
    atomic_write(batch / "evidence" / (f"preflight-{runner.counter:04d}.json"), json_bytes({"daemon_id": daemon["ID"], "logical_cpu": os.cpu_count(), "memory_available": available, "disk_available": disk.f_bavail * disk.f_frsize, "subnets": subnets, "pg_image_id": images[PG_IMAGE]["Id"], "builder_image_id": BUILDER, "pgdump_scope": "FIXED_CONTAINER_PATH_ONLY_HOST_PATH_UNEXECUTED"}))
    return daemon["ID"], images


INITDB = b'''#!/bin/sh
set -eu
psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname postgres -v "dbname=$C4_DATABASE" -v "other=$C4_OTHER_DATABASE" <<'SQL'
\\set admin_password `cat /run/secrets/admin_password`
\\set runtime_password `cat /run/secrets/runtime_password`
CREATE ROLE learning_admin LOGIN PASSWORD :'admin_password' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
CREATE ROLE learning_runtime LOGIN PASSWORD :'runtime_password' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
CREATE ROLE learning_auth_lock NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
GRANT learning_auth_lock TO learning_admin WITH INHERIT FALSE, SET TRUE;
CREATE DATABASE :"dbname" OWNER learning_admin TEMPLATE template0;
CREATE DATABASE :"other" OWNER learning_admin TEMPLATE template0;
REVOKE ALL ON DATABASE :"dbname" FROM PUBLIC;
REVOKE ALL ON DATABASE :"other" FROM PUBLIC;
GRANT CONNECT, TEMPORARY ON DATABASE :"dbname" TO learning_admin, learning_runtime;
GRANT CONNECT, TEMPORARY ON DATABASE :"other" TO learning_admin, learning_runtime;
SQL
'''


def compose_document(identity, subnet, case):
    return {"name": identity["project"], "services": {"pg": {"image": PG_IMAGE, "pull_policy": "never", "container_name": identity["pg_name"], "command": ["postgres", "-c", "max_prepared_transactions=16"], "cpus": 2, "mem_limit": "4g", "memswap_limit": "4g", "security_opt": ["no-new-privileges"], "environment": expected_pg_env(identity), "secrets": list(SECRET_NAMES), "volumes": [{"type": "volume", "source": identity["volume"], "target": "/var/lib/postgresql", "volume": {"nocopy": True}}, {"type": "bind", "source": str(case / "initdb.sh"), "target": "/docker-entrypoint-initdb.d/10-maintenance.sh", "read_only": True}], "networks": ["test"]}}, "networks": {"test": {"name": identity["network"], "internal": True, "ipam": {"config": [{"subnet": subnet}]}}}, "volumes": {identity["volume"]: {"name": identity["volume"]}}, "secrets": {name: {"file": str(case / "secrets/pg" / name)} for name in SECRET_NAMES}}


def extract_package(content, manifest, batch):
    source = private_dir(batch / "source")
    with zipfile.ZipFile(io.BytesIO(content)) as archive:
        for entry in manifest["files"]:
            path = source.joinpath(*PurePosixPath(entry["path"]).parts)
            missing = []
            parent = path.parent
            while parent != source and not parent.exists():
                missing.append(parent)
                parent = parent.parent
            for directory in reversed(missing):
                private_dir(directory)
            atomic_write(path, archive.read(entry["path"]), 0o400)
    source_digest(source, manifest)
    return source


def load_isolation(source, manifest):
    source_digest(source, manifest)
    spec = importlib.util.spec_from_file_location("reviewed_maintenance_isolation", source / "scripts/p0c4_source_isolation.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def remove_builder(runner, builder_id):
    require(HEX64.fullmatch(builder_id), "BUILDER_CLEANUP_ID")
    facts = inspect_many(runner, "container", [builder_id])[0]
    if facts["State"]["Running"]:
        runner.run("builder-stop", [DOCKER, "stop", "--time", "10", builder_id])
    runner.run("builder-remove", [DOCKER, "rm", builder_id])
    remaining = runner.run("builder-absence", [DOCKER, "ps", "-aq", "--no-trunc"]).stdout.decode().split()
    require(builder_id not in remaining, "BUILDER_REMOVAL_UNCONFIRMED")


def build_binary(runner, batch, source, manifest, archive_sha, image, result):
    target = private_dir(batch / "target")
    environment = {"CARGO_TARGET_DIR": "/target", "CARGO_BUILD_JOBS": "4", "CARGO_NET_OFFLINE": "true", "RUSTUP_AUTO_INSTALL": "0", "CARGO_TERM_COLOR": "never", "KNOWWEAVE_SOURCE_COMMIT": manifest["commit"], "KNOWWEAVE_BUILD_ID_SHA256": archive_sha}
    shell = "unset KNOWWEAVE_C4_VERIFIER_KEY_SHA256; cargo fmt --all -- --check; cargo clippy --locked --offline -p learning-backup --all-targets -- -D warnings; cargo test --locked --offline -p learning-backup --test maintenance_contract --test maintenance_journal; cargo test --locked --offline -p learning-backup --lib --no-run --message-format=json"
    expected = {"name": "knowweave-c4-maintenance-builder-" + batch.name.replace("-", ""), "batch_id": batch.name, "source": source, "target": target, "env": environment, "shell": shell}
    command = [DOCKER, "create", "--pull=never", "--name", expected["name"], "--label", "knowweave.c4.maintenance.batch=" + batch.name, "--network", "none", "--read-only", "--cap-drop=ALL", "--security-opt", "no-new-privileges", "--user", "0:0", "--pids-limit", "512", "--cpus", "4", "--memory", "8g", "--memory-swap", "8g", "--workdir", "/reviewed", "--tmpfs", "/tmp:rw,nosuid,nodev,size=1g", "--mount", "type=bind,src=" + str(source) + ",dst=/reviewed,readonly", "--mount", "type=bind,src=" + str(target) + ",dst=/target"]
    for key, value in sorted(environment.items()):
        command.extend(["--env", key + "=" + value])
    command.extend(["--entrypoint", "/bin/sh", BUILDER, "-ec", shell])
    builder_id = runner.run("builder-create", command).stdout.decode().strip()
    require(HEX64.fullmatch(builder_id), "BUILDER_CREATED_ID")
    result["builder"] = {"container_id": builder_id, "removed": False}
    try:
        facts = inspect_many(runner, "container", [builder_id])[0]
        require(facts["State"]["Running"] is False, "BUILDER_STARTED_EARLY")
        validate_builder(facts, expected, image_environment(image))
        runner.run("builder-start", [DOCKER, "start", builder_id])
        validate_builder(inspect_many(runner, "container", [builder_id])[0], expected, image_environment(image))
        waited = runner.run("builder-wait", [DOCKER, "wait", builder_id], timeout=7200)
        require(waited.stdout.strip() == b"0", "BUILDER_WORK_EXIT")
        logs = runner.run("builder-logs", [DOCKER, "logs", builder_id])
        # cargo's final JSON stream follows normal libtest output from contract gates.
        artifact_stream = "\n".join(line for line in logs.stdout.decode().splitlines() if line.startswith("{"))
        binary = target / "debug/deps" / select_artifact(artifact_stream)
        trusted(binary)
        require(os.access(binary, os.X_OK), "LIB_BINARY_EXECUTABLE")
        result["binary_sha256"] = digest(binary.read_bytes())
        listing = runner.run("lib-list", [str(binary), "--list"], env={"PATH": "/usr/bin:/bin", "HOME": "/root", "LC_ALL": "C"})
        result["listed_required_count"] = accept_listing(listing.returncode, listing.stdout.decode())
        return binary
    finally:
        remove_builder(runner, builder_id)
        result["builder"]["removed"] = True


def setup_case(case, identity, runner):
    private_dir(case)
    for name in ("control", "pin", "assets", "asset-stage", "isolation", "secrets"):
        private_dir(case / name)
    pg = private_dir(case / "secrets/pg")
    values = validate_secrets({name: secrets.token_hex(32) for name in SECRET_NAMES})
    runner.secrets.extend(value.encode() for value in values.values())
    for name, value in values.items():
        atomic_write(case / "secrets" / name, value.encode())
        atomic_write(pg / name, value.encode(), 0o400)
        os.chown(pg / name, 999, 999)
        meta = (pg / name).lstat()
        require(meta.st_uid == 999 and meta.st_gid == 999 and stat.S_IMODE(meta.st_mode) == 0o400 and (pg / name).read_bytes() == value.encode(), "PG_SECRET_COPY")
    require(b"\r" not in INITDB and INITDB.endswith(b"\n"), "INITDB_LF")
    atomic_write(case / "initdb.sh", INITDB, 0o444)
    require((case / "initdb.sh").read_bytes() == INITDB, "INITDB_BYTES")
    return values


def validate_retained_pg(record, containers, networks, volumes):
    require(len(containers) == len(networks) == len(volumes) == 1, "RETAINED_RESOURCE_COUNT")
    container, network, volume = containers[0], networks[0], volumes[0]
    identity = record["identity"]
    require(container.get("Id") == record["container_id"] and container.get("Name") == "/" + identity["pg_name"] and container.get("Image") == record["image_id"] and container.get("Config", {}).get("Image") == PG_IMAGE and container.get("Config", {}).get("Cmd") == ["postgres", "-c", "max_prepared_transactions=16"] and (container.get("Config", {}).get("Labels") or {}).get("com.docker.compose.project") == identity["project"] and (container.get("Config", {}).get("Labels") or {}).get("com.docker.compose.service") == "pg" and container.get("State", {}).get("Running") is False, "RETAINED_CONTAINER_IDENTITY")
    require(network.get("Id") == record["network_id"] and network.get("Name") == identity["network"] and network.get("Internal") is True and network.get("Driver") == "bridge" and (network.get("Labels") or {}).get("com.docker.compose.project") == identity["project"] and type(network.get("Containers")) is dict and network["Containers"] == {}, "RETAINED_NETWORK_NOT_EMPTY_ISOLATED")
    require(volume.get("Name") == record["volume_name"] == identity["volume"] and volume.get("Driver") == "local" and not volume.get("Options") and (volume.get("Labels") or {}).get("com.docker.compose.project") == identity["project"] and volume.get("Mountpoint") == record["volume_mountpoint"], "RETAINED_VOLUME_IDENTITY")
    mounts = container.get("Mounts") or []
    data = [m for m in mounts if m.get("Destination") == "/var/lib/postgresql"]
    require(len(data) == 1 and data[0].get("Type") == "volume" and data[0].get("Name") == record["volume_name"] and data[0].get("Source") == record["volume_mountpoint"] and data[0].get("RW") is True, "RETAINED_CONTAINER_VOLUME_BINDING")


def stop_pg(runner, record):
    record.update(stop_verified=False, retained=False, condition="UNVERIFIED_UNUSABLE_NOT_RESTORE")
    pg_id = record["container_id"]
    require(HEX64.fullmatch(pg_id), "PG_STOP_ID")
    facts = inspect_many(runner, "container", [pg_id])[0]
    require(facts["Id"] == pg_id, "PG_STOP_ID_CHANGED")
    if facts["State"]["Running"]:
        runner.run("pg-stop", [DOCKER, "stop", "--time", "10", pg_id])
    after = inspect_many(runner, "container", [pg_id])[0]
    require(after["State"]["Running"] is False, "PG_STOP_UNCONFIRMED")
    volume = inspect_many(runner, "volume", [record["volume_name"]])
    require(HEX64.fullmatch(record.get("network_id", "")), "RETAINED_NETWORK_UNCONFIRMED")
    network = inspect_many(runner, "network", [record["network_id"]])
    validate_retained_pg(record, [after], network, volume)
    record.update(stop_verified=True, retained=True, condition="STOPPED_RETAINED_UNUSABLE_NOT_RESTORE")


def case_resources(runner, identity):
    ids = runner.run("case-container-ids", [DOCKER, "ps", "-aq", "--no-trunc", "--filter", "label=com.docker.compose.project=" + identity["project"]]).stdout.decode().split()
    return {"containers": inspect_many(runner, "container", ids), "networks": runner.observe("case-network", [DOCKER, "network", "inspect", identity["network"]], lambda raw: json.loads(raw, object_pairs_hook=unique_pairs), observation_projection), "volumes": inspect_many(runner, "volume", [identity["volume"]])}


def run_case(runner, kind, identity, subnet, case, binary, images, isolation, result):
    values = setup_case(case, identity, runner)
    compose = case / "compose.json"
    document = compose_document(identity, subnet, case)
    atomic_write(compose, json_bytes(document))
    result["initdb_sha256"] = digest(INITDB)
    compose_command = [DOCKER, "compose", "--project-name", identity["project"], "-f", str(compose)]
    result["stage"] = "pg-create"
    # Capture a newly created PG ID even if compose fails partway, for exact stop.
    created = runner.run("pg-compose-create", compose_command + ["create", "--no-build", "--pull", "never"], allowed=tuple(range(256)))
    ids = runner.run("created-case-ids", [DOCKER, "ps", "-aq", "--no-trunc", "--filter", "label=com.docker.compose.project=" + identity["project"]]).stdout.decode().split()
    require(len(ids) == 1 and HEX64.fullmatch(ids[0]), "CREATED_PG_ID")
    # Record exact newly observed ID before any network/volume inspection can fail.
    record = {"container_id": ids[0], "network_id": "", "volume_name": identity["volume"], "identity": identity, "image_id": images[PG_IMAGE]["Id"], "volume_mountpoint": "", "stop_verified": False, "retained": True, "condition": "UNVERIFIED_UNUSABLE"}
    result["resource"] = record
    resources = case_resources(runner, identity)
    require(len(resources["networks"]) == 1, "CREATED_NETWORK_ID")
    record["network_id"] = resources["networks"][0]["Id"]
    require(len(resources["volumes"]) == 1, "CREATED_VOLUME_IDENTITY")
    record["volume_mountpoint"] = resources["volumes"][0]["Mountpoint"]
    require(created.returncode == 0, "PG_CREATE_EXIT")
    result["stage"] = "pg-inspect-before-start"
    validate_pg(identity, subnet, case, resources, images[PG_IMAGE]["Id"], running=False, image_env=image_environment(images[PG_IMAGE]))
    runner.run("pg-start", [DOCKER, "start", record["container_id"]])
    result["stage"] = "pg-ready"
    for _ in range(60):
        probe = runner.run("pg-tcp-ready", [DOCKER, "exec", record["container_id"], "/usr/bin/pg_isready", "-h", "127.0.0.1", "-U", "postgres", "-d", identity["database"]], allowed=(0, 1, 2), timeout=10)
        if probe.returncode == 0:
            break
        time.sleep(1)
    else:
        raise GateError("PG_READY_TIMEOUT")
    resources = case_resources(runner, identity)
    current = validate_pg(identity, subnet, case, resources, images[PG_IMAGE]["Id"], running=True, image_env=image_environment(images[PG_IMAGE]))
    require(current["container_id"] == record["container_id"] and current["network_id"] == record["network_id"], "PG_CREATED_ID_CHANGED")
    host = current["ip"]
    version = runner.run("container-pgdump-version", [DOCKER, "exec", record["container_id"], str(PGDUMP), "--version"])
    require(re.fullmatch(rb"pg_dump \(PostgreSQL\) 18\.[0-9]+[^\r\n]*\n", version.stdout), "CONTAINER_PGDUMP_VERSION")
    uid = runner.run("container-postgres-uid", [DOCKER, "exec", record["container_id"], "/usr/bin/id", "-u", "postgres"])
    gid = runner.run("container-postgres-gid", [DOCKER, "exec", record["container_id"], "/usr/bin/id", "-g", "postgres"])
    require(uid.stdout.strip() == gid.stdout.strip() == b"999", "PINNED_PG_UID")
    result["pgdump_config_scope"] = "GENUINE_CONTAINER_PG18_PATH_NOT_HOST_EXECUTED_NO_DUMP"
    passfile = case / "secrets/admin.pgpass"
    atomic_write(passfile, (host + ":5432:" + identity["database"] + ":learning_admin:" + values["admin_password"] + "\n" + host + ":5432:" + identity["other_database"] + ":learning_admin:" + values["admin_password"] + "\n").encode())
    trusted(passfile, mode=0o600)
    environment = child_environment(kind, identity, case, host, values)
    runner.secrets.extend(v.encode() for k, v in environment.items() if k.endswith("DATABASE_URL"))
    result["stage"] = "held-isolation-proof-and-exact-test"
    backup_id = identity["backup_id"]
    with exclusive_lock(case / "isolation" / ("isolation-" + backup_id + ".lock"), new=True):
        resources = case_resources(runner, identity)
        validate_pg(identity, subnet, case, resources, images[PG_IMAGE]["Id"], running=True, image_env=image_environment(images[PG_IMAGE]))
        facts = isolation.assess_project(identity["project"], resources["containers"], resources["networks"])
        evidence = json_bytes(facts)
        inspection = "inspection-" + backup_id + ".json"
        atomic_write(case / "isolation" / inspection, evidence)
        proof = {"format_version": 2, "backup_id": backup_id, "database": identity["database"], "compose_project": identity["project"], "observed_unix_ms": int(time.time() * 1000), "driver_pid": os.getpid(), "inspection_file": inspection, "docker_inspection_sha256": digest(evidence), **{key: facts[key] for key in ("runtime_running", "worker_running", "other_admin_processes", "postgres_published_ports", "network_internal", "manager_runtime_secret_mounts", "manager_runtime_env_keys")}}
        atomic_write(case / "isolation" / ("isolation-" + backup_id + ".json"), json_bytes(proof))
        process = runner.run("exact-" + kind, [str(binary), TESTS[kind], "--exact", "--ignored", "--test-threads=1"], env=environment, timeout=180, allowed=(0, 101))
        result["gate"] = accept_test_output(TESTS[kind], process.returncode, process.stdout.decode())
        resources = case_resources(runner, identity)
        validate_pg(identity, subnet, case, resources, images[PG_IMAGE]["Id"], running=True, image_env=image_environment(images[PG_IMAGE]))
    result["pending_protocol"] = "RELEASE_READY_RECOVERY_OBSERVATION_RECORDED_SYNTHETIC_PRIOR_DIGESTS" if kind == "release" else "PREFLIGHT_ONLY_NO_BACKUP_PROTOCOL"
    result["stage"] = "exact-stop-retain"
    stop_pg(runner, record)
    result["status"] = "PASSED_FOCUSED_ONLY"


def run(args):
    require(sys.platform == "linux" and os.geteuid() == 0 and canonical_uuid(args.batch_id), "ROOT_LINUX_NEW_UUID_REQUIRED")
    trusted(BASE, mode=0o700, directory=True)
    trusted(BASE / "incoming", mode=0o700, directory=True)
    trusted(Path(__file__).absolute(), mode=0o500)
    archive = args.archive
    require(archive.is_absolute() and archive.parent == BASE / "incoming", "PRIVATE_ARCHIVE_PATH")
    trusted(archive, mode=0o400)
    require(digest(Path(__file__).read_bytes()) == args.runner_sha256, "INSTALLED_RUNNER_DIGEST")
    with archive.open("rb") as stream:
        content = stream.read(32 * 1024 ** 2 + 1)
    manifest, content = verify_package(content, args.archive_sha256, args.manifest_sha256, args.source_commit, args.runner_sha256)
    root = BASE / "maintenance-gates"
    if not root.exists():
        private_dir(root)
    trusted(root, mode=0o700, directory=True)
    with exclusive_lock(root / "exclusive-orchestration.lock"):
        batch = private_dir(root / args.batch_id)
        evidence = private_dir(batch / "evidence")
        runner = Runner(evidence)
        result = {"format_version": 1, "batch_id": args.batch_id, "status": "FAILED_CLOSED_NOT_FULL_BACKUP_NOT_RESTORE", "stage": "source-extraction", "inputs": {"archive_sha256": args.archive_sha256, "manifest_sha256": args.manifest_sha256, "source_commit": args.source_commit, "runner_sha256": args.runner_sha256, "pg_image": PG_IMAGE, "builder_image": BUILDER}, "cases": [], "pending_protocol": "RETAIN_STOPPED_RESOURCES_FOR_MANUAL_REVIEW_NO_REPLAY"}
        source = None
        try:
            source = extract_package(content, manifest, batch)
            result["source_before_sha256"] = source_digest(source, manifest)
            isolation = load_isolation(source, manifest)
            identities = [identity_for(str(uuid.uuid4()), str(uuid.uuid4())) for _ in TESTS]
            subnets = admit_subnets(args.subnets, [])
            builder_name = "knowweave-c4-maintenance-builder-" + args.batch_id.replace("-", "")
            result["stage"] = "preflight"
            daemon_id, images = host_preflight(runner, identities, subnets, builder_name, batch)
            result["stage"] = "offline-build"
            binary = build_binary(runner, batch, source, manifest, args.archive_sha256, images[BUILDER], result)
            cases = private_dir(batch / "cases")
            for kind, identity, subnet in zip(TESTS, identities, subnets):
                item = {"kind": kind, "identity": identity, "subnet": subnet, "status": "FAILED_UNUSABLE", "stage": "setup", "pending_protocol": "UNVERIFIED_RETAINED_REVIEW_REQUIRED"}
                result["cases"].append(item)
                result["stage"] = kind
                # Recheck routes/capacity/absence for this still-uncreated case;
                # previous case networks are retained and are not reused.
                require(host_preflight(runner, [identity], [subnet], builder_name, batch)[0] == daemon_id, "DAEMON_CHANGED")
                run_case(runner, kind, identity, subnet, cases / identity["case_id"], binary, images, isolation, item)
            result["source_after_sha256"] = source_digest(source, manifest)
            require(result["source_after_sha256"] == result["source_before_sha256"] and digest(Path(__file__).read_bytes()) == args.runner_sha256 and len(result["cases"]) == 3 and all(c["resource"]["stop_verified"] for c in result["cases"]), "FINAL_EVIDENCE")
            result["status"] = PASSED
            result["stage"] = "complete-focused-gates"
        except BaseException as error:
            result["reason"] = str(error) if isinstance(error, GateError) else type(error).__name__
            for item in result["cases"]:
                if "resource" in item:
                    try:
                        stop_pg(runner, item["resource"])
                    except BaseException as stop_error:
                        item["resource"]["stop_error"] = str(stop_error) if isinstance(stop_error, GateError) else type(stop_error).__name__
            if source is not None:
                try:
                    result["source_after_sha256"] = source_digest(source, manifest)
                except BaseException:
                    result["source_unchanged"] = False
        payload = json_bytes(result)
        atomic_write(evidence / "result.json", payload)
        atomic_write(evidence / "result.sha256", (digest(payload) + "\n").encode())
        print(json.dumps({"status": result["status"], "stage": result["stage"], "result_sha256": digest(payload), "evidence": str(evidence), "completed_cases": sum(c.get("gate", {}).get("passed", 0) for c in result["cases"]), "not_full_backup": True, "not_restore": True}, sort_keys=True), flush=True)
        return 0 if result["status"] == PASSED else 1


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("archive-sha256", "manifest-sha256", "source-commit", "runner-sha256", "batch-id"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--subnets", nargs=3, required=True)
    try:
        return run(parser.parse_args(argv))
    except BaseException:
        print(json.dumps({"status": "MAINTENANCE_ACCEPTANCE_ADMISSION_REJECTED_NOT_RESTORE"}, sort_keys=True), flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
