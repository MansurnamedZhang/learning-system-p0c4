#!/usr/bin/env python3
"""Root-only, three-project C4 Task3 Linux acceptance for one reviewed ZIP.

This is an acceptance harness, not a production backup scheduler. Execute
only after the exact source ZIP, its SHA-256, and /var/lib/knowweave-c4 batch
path have been separately authorized. The host and Docker operators are
trusted throughout this operational maintenance window.
"""

import argparse
from contextlib import nullcontext
import hashlib
import ipaddress
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
import uuid
import zipfile


BASE = Path("/var/lib/knowweave-c4")
PG_IMAGE = "postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
BUILDER_IMAGE_ID = "sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b"
CASES = ("success", "missing_original", "pg_dump_exit")
SUBNETS = ("10.251.215.0/24", "10.251.216.0/24", "10.251.217.0/24")
SUCCESS_TEST = "real_gate_waits_for_old_runtime_session_and_rejects_new_runtime_login"
FAILURE_TEST = "missing_original_or_pg_dump_failure_keeps_gate_closed"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_digest(path):
    hasher = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def private_dir(path):
    path = Path(path)
    path.mkdir(mode=0o700, exist_ok=False)
    path.chmod(0o700)
    return path


def private_file(path, payload, mode=0o600):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    with os.fdopen(fd, "wb") as output:
        output.write(payload)
        output.flush()
        os.fsync(output.fileno())
    os.chmod(path, mode)


def assert_root_path(path):
    path = Path(path).absolute()
    for item in (path, *path.parents):
        info = item.lstat()
        require(info.st_uid == 0 and not stat.S_IMODE(info.st_mode) & 0o022,
                "root-owned non-writable path required")


def case_identity(batch_id, kind):
    require(kind in CASES, "unknown acceptance case")
    marker = uuid.uuid5(batch_id, kind)
    return {
        "kind": kind,
        "backup_id": str(uuid.uuid5(batch_id, kind + "-backup")),
        "database": "learning_backup_c4_task3_" + str(marker),
        "project": f"learning-system-p0c4-task3-{kind.replace('_', '-')}-{marker.hex[:8]}",
    }


def available_subnets(requested, occupied):
    networks = [ipaddress.ip_network(value, strict=True) for value in requested]
    require(len(networks) == 3 and len(set(networks)) == 3, "three distinct subnets required")
    require(all(not first.overlaps(second) for index, first in enumerate(networks)
                for second in networks[index + 1:]), "requested subnets overlap each other")
    existing = [ipaddress.ip_network(value, strict=False) for value in occupied]
    require(all(not current.overlaps(other) for current in networks for other in existing
                if other.version == current.version), "requested subnet overlaps a host route or Docker network")
    return networks


def verify_archive(archive_path, expected_sha, expected_manifest_sha):
    archive_path = Path(archive_path)
    require(re.fullmatch(r"[0-9a-f]{64}", expected_sha) is not None, "archive hash format")
    require(re.fullmatch(r"[0-9a-f]{64}", expected_manifest_sha) is not None, "manifest hash format")
    archive_bytes = archive_path.read_bytes()
    require(digest(archive_bytes) == expected_sha, "reviewed archive SHA-256 differs")
    with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
        infos = archive.infolist()
        names = [info.filename for info in infos]
        require(len(names) == len(set(names)) and "SOURCE_MANIFEST.json" in names,
                "duplicate or missing archive entry")
        require(len(infos) < 1000 and sum(info.file_size for info in infos) < 32 * 1024 * 1024,
                "archive budget exceeded")
        for info in infos:
            path = PurePosixPath(info.filename)
            require(not info.is_dir() and not path.is_absolute() and ".." not in path.parts
                    and len(path.parts) > 0 and not any(part in {"", "."} for part in path.parts),
                    "unsafe archive path")
            mode = (info.external_attr >> 16) & 0o170000
            require(mode in {0, 0o100000}, "archive contains non-regular entry")
        manifest_bytes = archive.read("SOURCE_MANIFEST.json")
        require(digest(manifest_bytes) == expected_manifest_sha, "source manifest SHA-256 differs")
        manifest = json.loads(manifest_bytes)
        require(manifest.get("format_version") == 1 and re.fullmatch(r"[0-9a-f]{40}", manifest.get("commit", "")),
                "source manifest identity invalid")
        files = manifest.get("files")
        require(isinstance(files, list) and files and len(files) == len(names) - 1,
                "manifest file count differs")
        require([item["path"] for item in files] == sorted(set(names) - {"SOURCE_MANIFEST.json"}),
                "manifest paths differ from archive")
        for item in files:
            data = archive.read(item["path"])
            require(item["size"] == len(data) and item["sha256"] == digest(data),
                    "manifest entry differs from archive bytes")
    return manifest


def extract_verified(archive_path, manifest, destination):
    private_dir(destination)
    with zipfile.ZipFile(archive_path) as archive:
        for item in manifest["files"]:
            relative = PurePosixPath(item["path"])
            target = destination.joinpath(*relative.parts)
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            for parent in (target.parent, *target.parent.parents):
                if parent == destination.parent:
                    break
                parent.chmod(0o700)
            # The official PG image sources this file after dropping to its
            # postgres UID; Docker resolves the root-owned host ancestor.
            mode = 0o444 if item["path"] == "deploy/initdb.sh" else 0o400
            private_file(target, archive.read(item["path"]), mode)
    require(source_hashes(destination, manifest) == {item["path"]: item["sha256"] for item in manifest["files"]},
            "extracted source differs")


def source_hashes(source, manifest):
    expected = {item["path"] for item in manifest["files"]}
    actual = {}
    for path in source.rglob("*"):
        require(not path.is_symlink(), "source symlink appeared")
        if path.is_file():
            actual[path.relative_to(source).as_posix()] = file_digest(path)
    require(set(actual) == expected, "source file set changed")
    return actual


def assertion_for_case(kind, backup_id, redacted_log, phases, sealed, complete):
    require(kind in CASES, "unknown case")
    name = SUCCESS_TEST if kind == "success" else FAILURE_TEST
    text = redacted_log.decode(errors="replace")
    require(re.search(rf"^test {re.escape(name)} \.\.\. ok$", text, re.M) is not None,
            "expected test name and pass line missing")
    require(re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", text) is not None,
            "test count differs")
    require(not complete, "source produced a complete receipt")
    expected_phase = {"success": "released.json", "missing_original": "dump-and-index-durable.json",
                      "pg_dump_exit": "drained.json"}[kind]
    require(expected_phase in phases, "expected gate phase missing")
    later = {"missing_original": {"pins-durable.json", "release-ready.json", "released.json"},
             "pg_dump_exit": {"dump-and-index-durable.json", "pins-durable.json", "release-ready.json", "released.json"}}
    require(not (phases & later.get(kind, set())), "failure gate advanced after error")
    require(sealed == (kind == "success"), "sealed pin state differs")
    require(str(backup_id) in text or name in text, "test output was not retained")


class Runner:
    def __init__(self, batch, secrets):
        self.batch = batch
        self.secrets = [item for item in secrets if item]
        self.counter = 0

    def redacted(self, data):
        for secret in sorted(set(self.secrets), key=len, reverse=True):
            data = data.replace(secret, b"[REDACTED]")
        return data

    def run(self, label, command, *, env=None, timeout=1200, allowed=(0,), input_path=None):
        self.counter += 1
        prefix = self.batch / "evidence" / f"{self.counter:04d}-{label}"
        private_file(prefix.with_suffix(".argv.json"), self.redacted(json.dumps(command).encode()))
        try:
            with (Path(input_path).open("rb") if input_path else nullcontext(None)) as input_stream:
                result = subprocess.run(command, stdin=input_stream, capture_output=True,
                                        env=env, timeout=timeout, check=False)
        except subprocess.TimeoutExpired as error:
            private_file(prefix.with_suffix(".stdout.log"), self.redacted(error.stdout or b""))
            private_file(prefix.with_suffix(".stderr.log"), self.redacted(error.stderr or b""))
            raise RuntimeError(label + " timed out") from None
        private_file(prefix.with_suffix(".stdout.log"), self.redacted(result.stdout))
        private_file(prefix.with_suffix(".stderr.log"), self.redacted(result.stderr))
        private_file(prefix.with_suffix(".exit"), str(result.returncode).encode())
        require(result.returncode in allowed, label + " failed; see private redacted evidence")
        return result.stdout


def docker_json(command):
    return json.loads(subprocess.check_output(["/usr/bin/docker", *command]))


def preflight_projects(identities, subnets):
    require(not os.environ.get("DOCKER_HOST"), "DOCKER_HOST override forbidden")
    require(Path("/var/run/docker.sock").exists(), "local Docker daemon unavailable")
    endpoint = json.loads(subprocess.check_output(
        ["/usr/bin/docker", "context", "inspect", "--format", "{{json .Endpoints.docker.Host}}"]
    ))
    require(endpoint == "unix:///var/run/docker.sock", "local Docker socket required")
    networks = docker_json(["network", "inspect", *subprocess.check_output(
        ["/usr/bin/docker", "network", "ls", "-q"], text=True).split()])
    occupied = [entry["Subnet"] for item in networks for entry in (item.get("IPAM") or {}).get("Config") or []
                if entry.get("Subnet")]
    routes = json.loads(subprocess.check_output(["/usr/bin/ip", "-j", "-4", "route", "show", "table", "all"]))
    occupied += [item["dst"] for item in routes if item.get("dst") and item["dst"] != "default"]
    available_subnets(subnets, occupied)
    for item in identities:
        project = item["project"]
        for kind, args in (("ps", ["ps", "-aq"]), ("network", ["network", "ls", "-q"]),
                           ("volume", ["volume", "ls", "-q"])):
            require(not subprocess.check_output(["/usr/bin/docker", *args,
                                                 "--filter", f"label=com.docker.compose.project={project}"]).strip(),
                    f"fresh project {kind} resource already exists")
        for command in (["network", "inspect", f"{project}_test"],
                        ["volume", "inspect", f"{project}_pg"]):
            require(subprocess.run(["/usr/bin/docker", *command], capture_output=True).returncode != 0,
                    "named project resource already exists without label")
        for name in (f"{project}-pg-1", f"{project}-c4-manager-{item['backup_id'][:8]}"):
            require(subprocess.run(["/usr/bin/docker", "inspect", name], capture_output=True).returncode != 0,
                    "named project container already exists without label")
    for image in (PG_IMAGE, BUILDER_IMAGE_ID):
        actual = subprocess.check_output(["/usr/bin/docker", "image", "inspect", image,
                                          "--format", "{{.Id}}"], text=True).strip()
        require(actual.startswith("sha256:"), "pinned image unavailable")
        if image.startswith("sha256:"):
            require(actual == image, "builder image identity differs")


def prepare_secrets(case_dir, runner):
    import secrets
    secret_dir = private_dir(case_dir / "secrets")
    pg_dir = private_dir(secret_dir / "pg")
    identity = runner.run("pg-uid", ["/usr/bin/docker", "run", "--rm", "--network", "none",
                                     "--entrypoint", "sh", PG_IMAGE, "-c", "id -u postgres; id -g postgres"])
    uid, gid = [int(value) for value in identity.splitlines()]
    values = {key: secrets.token_hex(32) for key in ("postgres_password", "admin_password", "runtime_password")}
    require(len(set(values.values())) == 3, "secret collision")
    for name, value in values.items():
        path = pg_dir / name
        private_file(path, value.encode())
        os.chown(path, uid, gid)
        runner.secrets.append(value.encode())
    pgpass = secret_dir / "admin.pgpass"
    return values, pgpass, pg_dir


def compose_document(identity, subnet, case_dir, source):
    project = identity["project"]
    return {
        "name": project,
        "services": {"pg": {
            "image": PG_IMAGE, "cpus": 2, "mem_limit": "4g",
            "environment": {"POSTGRES_USER": "postgres", "POSTGRES_DB": "postgres",
                            "POSTGRES_PASSWORD_FILE": "/run/secrets/postgres_password"},
            "secrets": ["postgres_password", "admin_password", "runtime_password"],
            "volumes": [f"{project}_pg:/var/lib/postgresql",
                        f"{source / 'deploy/initdb.sh'}:/docker-entrypoint-initdb.d/10-learning.sh:ro"],
            "networks": ["test"],
            "healthcheck": {"test": ["CMD-SHELL", "pg_isready -U postgres -d postgres"],
                            "interval": "2s", "timeout": "3s", "retries": 30},
        },
            # Sentinels carry the same named runtime credential boundary and
            # prove the root driver actually stops both project services.
            "runtime": {"image": PG_IMAGE, "entrypoint": ["/bin/sleep"], "command": ["3600"],
                        "user": "postgres", "secrets": ["runtime_password"], "networks": ["test"]},
            "worker": {"image": PG_IMAGE, "entrypoint": ["/bin/sleep"], "command": ["3600"],
                       "user": "postgres", "secrets": ["runtime_password"], "networks": ["test"]},
        },
        "networks": {"test": {"name": f"{project}_test", "internal": True,
                              "ipam": {"config": [{"subnet": str(subnet)}]}}},
        "volumes": {f"{project}_pg": {"name": f"{project}_pg"}},
        "secrets": {key: {"file": str(case_dir / "secrets/pg" / key)}
                    for key in ("postgres_password", "admin_password", "runtime_password")},
    }


def build_reviewed_binaries(batch, source, manifest, archive_sha, runner):
    target = private_dir(batch / "target")
    binaries = private_dir(batch / "bin")
    command = ["/usr/bin/docker", "run", "--rm", "--network", "none",
               "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--user", "0:0",
               "--workdir", "/reviewed", "--tmpfs", "/tmp:rw,nosuid,nodev,size=1g",
               "--mount", f"type=bind,src={source},dst=/reviewed,readonly",
               "--mount", f"type=bind,src={target},dst=/target",
               "--env", "CARGO_TARGET_DIR=/target", "--env", "CARGO_BUILD_JOBS=4",
               "--env", f"KNOWWEAVE_SOURCE_COMMIT={manifest['commit']}",
               "--env", f"KNOWWEAVE_BUILD_ID_SHA256={archive_sha}",
               "--entrypoint", "/bin/sh", BUILDER_IMAGE_ID, "-ec"]
    for label, shell in (
        ("linux-format", "cargo fmt --all -- --check"),
        ("linux-strict-clippy", "cargo clippy --locked --offline --workspace --all-targets -- -D warnings"),
        ("linux-workspace-compile", "cargo test --locked --offline --workspace --no-run"),
        ("linux-task3-build", "cargo test --locked --offline -p learning-backup --test maintenance_pg --no-run && "
                              "cargo build --locked --offline -p learning-backup --example c4_task3_migrate"),
    ):
        runner.run(label, command + [shell], timeout=7200)
    tests = [path for path in (target / "debug/deps").glob("maintenance_pg-*")
             if path.is_file() and os.access(path, os.X_OK)]
    require(len(tests) == 1, "exactly one reviewed maintenance_pg binary required")
    example = target / "debug/examples/c4_task3_migrate"
    require(example.is_file(), "reviewed migration helper binary missing")
    for name, path in (("maintenance_pg", tests[0]), ("c4_task3_migrate", example)):
        shutil.copyfile(path, binaries / name)
        (binaries / name).chmod(0o500)
    return binaries


def validate_dump_index_in_fresh_database(identity, pg, pin, case, runner, archive_sha, source_commit):
    """Bounded Task3 source consistency probe, not full Task4 restoration."""
    sealed = pin / f"{identity['backup_id']}.sealed"
    dump = sealed / "database.dump"
    index_path = sealed / "asset-index.json"
    manifest = json.loads((sealed / "manifest.json").read_bytes())
    require(manifest["source"]["application_build_sha256"] == archive_sha
            and manifest["source"]["application_commit"] == source_commit,
            "sealed source identity differs from reviewed ZIP build")
    index_bytes = index_path.read_bytes()
    index = json.loads(index_bytes)
    require(index.get("format_version") == 1 and len(index.get("assets", [])) == 2,
            "success catalog must contain two ready asset rows")
    files = {entry["path"]: entry for entry in manifest["files"]}
    require(files["asset-index.json"]["sha256"] == digest(index_bytes)
            and files["database.dump"]["sha256"] == file_digest(dump)
            and files["database.dump"]["size"] == dump.stat().st_size
            and manifest["logical_asset_count"] == 2,
            "sealed dump/index differs from source manifest")
    require(dump.stat().st_size <= 128 * 1024 * 1024, "Task3 validation dump budget exceeded")
    pg_restore = "/usr/lib/postgresql/18/bin/pg_restore"
    toc = runner.run("success-pg-restore-list", ["/usr/bin/docker", "exec", "-i", "-u", "postgres",
                                                 pg, pg_restore, "--list"], input_path=dump, timeout=120)
    require(b"TABLE DATA public asset" in toc, "PG18 dump TOC lacks asset table data")
    validation_db = "c4v_" + identity["backup_id"]
    runner.run("success-create-validation-db", ["/usr/bin/docker", "exec", "-u", "postgres", pg,
                                                 "psql", "-X", "-v", "ON_ERROR_STOP=1", "-d", "postgres",
                                                 "-c", f'CREATE DATABASE "{validation_db}" OWNER learning_admin'])
    runner.run("success-pg-restore-validation", ["/usr/bin/docker", "exec", "-i", "-u", "postgres", pg,
                                                 pg_restore, "--format=custom", "--exit-on-error",
                                                 "--clean", "--if-exists",
                                                 "--no-owner", "--no-acl", "--dbname=" + validation_db],
               input_path=dump, timeout=900)
    query = ("SELECT COALESCE(json_agg(json_build_object("
             "'space_id',space_id::text,'id',id::text,'sha256',sha256,"
             "'byte_size',byte_size,'storage_key',storage_key) ORDER BY space_id,id),"
             "'[]'::json)::text FROM (SELECT space_id,id,sha256,byte_size,storage_key "
             "FROM public.asset WHERE status='ready' ORDER BY space_id,id LIMIT 3) a")
    output = runner.run("success-restored-asset-query", ["/usr/bin/docker", "exec", "-u", "postgres", pg,
                                                         "psql", "-X", "-Atq", "-d", validation_db,
                                                         "-c", query], timeout=120)
    rows = json.loads(output.strip())
    require(rows == index["assets"], "restored ready rows differ from canonical asset index")
    evidence = {"validation_database": validation_db, "ready_row_count": len(rows),
                "asset_index_sha256": digest(index_bytes), "database_dump_sha256": file_digest(dump),
                "ready_rows_sha256": digest(json.dumps(rows, sort_keys=True, separators=(",", ":")).encode()),
                "scope": "Task3 dump/index consistency only; not full Task4 restore"}
    private_file(case / "dump-index-consistency.json", json.dumps(evidence, sort_keys=True).encode())


def run_one_case(identity, subnet, batch, source, binaries, runner, archive_sha, source_commit):
    kind = identity["kind"]
    case = private_dir(batch / "cases" / kind)
    control = private_dir(case / "control")
    pin = private_dir(case / "pin")
    assets = private_dir(case / "assets")
    stage = private_dir(case / "stage")
    driver_evidence = private_dir(case / "driver-evidence")
    values, pgpass, _ = prepare_secrets(case, runner)
    database = identity["database"]
    private_file(pgpass, f"pg:5432:{database}:learning_admin:{values['admin_password']}\n".encode())
    url = (f"postgres://learning_admin:{values['admin_password']}@pg:5432/{database}"
           "?application_name=knowweave_c4_manager")
    runner.secrets.append(url.encode())
    compose_path = case / "compose.json"
    private_file(compose_path, json.dumps(compose_document(identity, subnet, case, source),
                                         sort_keys=True, separators=(",", ":")).encode())
    compose = ["/usr/bin/docker", "compose", "-p", identity["project"], "-f", str(compose_path)]
    armed = False
    case_passed = False
    try:
        config = json.loads(runner.run(kind + "-compose-config", compose + ["config", "--format", "json"]))
        pg_config = config["services"]["pg"]
        net_config = config["networks"]["test"]
        require(config["name"] == identity["project"]
                and set(config["services"]) == {"pg", "runtime", "worker"}
                and all(not config["services"][service].get("ports") for service in config["services"])
                and pg_config["image"] == PG_IMAGE
                and not pg_config.get("ports") and len(pg_config["secrets"]) == 3
                and net_config["internal"] is True
                and net_config["name"] == identity["project"] + "_test"
                and net_config["ipam"]["config"][0]["subnet"] == str(subnet)
                and config["volumes"][identity["project"] + "_pg"]["name"] == identity["project"] + "_pg",
                "Compose isolation differs from reviewed project")
        armed = True
        runner.run(kind + "-project-start", compose + ["up", "-d", "--wait", "pg", "runtime", "worker"], timeout=300)
        for service in ("runtime", "worker"):
            inspected = docker_json(["inspect", identity["project"] + "-" + service + "-1"])[0]
            require(inspected["State"]["Running"] is True
                    and inspected["Config"]["Labels"].get("com.docker.compose.project") == identity["project"],
                    "business sentinel did not start in isolated project")
            runner.run(kind + "-" + service + "-secret-readable",
                       ["/usr/bin/docker", "exec", identity["project"] + "-" + service + "-1",
                        "test", "-r", "/run/secrets/runtime_password"])
        pg = identity["project"] + "-pg-1"
        version = runner.run(kind + "-pg-version", ["/usr/bin/docker", "exec", "-u", "postgres", pg,
                                                 "psql", "-X", "-Atq", "-d", "postgres", "-c",
                                                 "SHOW server_version_num"]).strip()
        require(version.startswith(b"18"), "PostgreSQL major version differs")
        for label, sql in (("create-db", f'CREATE DATABASE "{database}" OWNER learning_admin'),
                           ("revoke-public", f'REVOKE ALL ON DATABASE "{database}" FROM PUBLIC'),
                           ("grant-runtime", f'GRANT CONNECT, TEMPORARY ON DATABASE "{database}" TO learning_runtime')):
            runner.run(kind + "-" + label, ["/usr/bin/docker", "exec", "-u", "postgres", pg,
                                            "psql", "-X", "-v", "ON_ERROR_STOP=1", "-d", "postgres", "-c", sql])
        migrate_env = {"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/root",
                       "TEST_ADMIN_DATABASE_URL": url, "TEST_C4_TASK3_DATABASE_NAME": database}
        runner.run(kind + "-migrate", ["/usr/bin/docker", "run", "--rm", "--network",
                                        identity["project"] + "_test", "--read-only", "--cap-drop", "ALL",
                                        "--security-opt", "no-new-privileges", "--user", "0:0",
                                        "--mount", f"type=bind,src={binaries / 'c4_task3_migrate'},dst=/usr/local/bin/c4-migrate,readonly",
                                        "--env", "TEST_ADMIN_DATABASE_URL", "--env", "TEST_C4_TASK3_DATABASE_NAME",
                                        PG_IMAGE, "/usr/local/bin/c4-migrate"], env=migrate_env, timeout=900)
        failure_bin = binaries / "pg_dump_fail"
        if kind == "pg_dump_exit" and not failure_bin.exists():
            private_file(failure_bin, b'#!/bin/sh\nif [ "$1" = "--version" ]; then echo "pg_dump (PostgreSQL) 18.6"; exit 0; fi\nexit 42\n', 0o500)
        env = {"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/root",
               "TEST_ADMIN_DATABASE_URL": url,
               "TEST_C4_PIN_ROOT": str(pin), "TEST_C4_ASSET_ROOT": str(assets),
               "TEST_C4_ASSET_STAGE_ROOT": str(stage), "TEST_C4_PGPASSFILE": str(pgpass)}
        if kind != "success":
            env["TEST_C4_FAILURE_KIND"] = kind
        if kind == "pg_dump_exit":
            env["TEST_C4_PGDUMP_FAIL_BIN"] = str(failure_bin)
        test = SUCCESS_TEST if kind == "success" else FAILURE_TEST
        command = ["/usr/bin/python3", "-B", str(source / "scripts/p0c4_source_isolation.py"),
                   "--backup-id", identity["backup_id"], "--database", database,
                   "--project", identity["project"], "--private-root", str(driver_evidence),
                   "--control-root", str(control), "--", str(binaries / "maintenance_pg"),
                   "--ignored", "--exact", test, "--nocapture", "--test-threads=1"]
        runner.run(kind + "-source-driver", command, env=env, timeout=1200)
        result = json.loads((driver_evidence / f"driver-result-{identity['backup_id']}.json").read_text())
        require(result["status"] == "SOURCE_CAPTURE_DRIVER_PASSED_NOT_COMPLETE",
                "source driver did not attest this case")
        require(result["start_inspection_sha256"] == file_digest(
                    driver_evidence / f"inspection-{identity['backup_id']}.json")
                and result["end_inspection_sha256"] == file_digest(
                    driver_evidence / f"inspection-end-{identity['backup_id']}.json"),
                "driver inspection evidence hash differs")
        for service in ("runtime", "worker"):
            inspected = docker_json(["inspect", identity["project"] + "-" + service + "-1"])[0]
            require(inspected["State"]["Running"] is False,
                    "business sentinel remained running after maintenance driver")
        stdout = (driver_evidence / f"manager-{identity['backup_id']}.stdout.redacted.log").read_bytes()
        stderr = (driver_evidence / f"manager-{identity['backup_id']}.stderr.redacted.log").read_bytes()
        require(all(secret not in stdout + stderr for secret in runner.secrets),
                "manager redacted logs contain a secret")
        journal = control / f"{identity['backup_id']}.control"
        phases = {item.name for item in journal.iterdir()}
        for name in phases:
            require(name.endswith(".json"), "unexpected journal entry")
            entry = json.loads((journal / name).read_bytes())
            require(entry["backup_id"] == identity["backup_id"]
                    and entry["phase"] == name.removesuffix(".json").replace("-", "_"),
                    "journal entry identity or phase differs")
        assertion_for_case(kind, uuid.UUID(identity["backup_id"]), stdout + stderr, phases,
                           (pin / f"{identity['backup_id']}.sealed").is_dir(),
                           (pin / f"{identity['backup_id']}.complete").exists())
        require((driver_evidence / f"inspection-end-{identity['backup_id']}.json").is_file(),
                "end-of-capture Docker inspection missing")
        if kind == "success":
            validate_dump_index_in_fresh_database(identity, pg, pin, case, runner,
                                                  archive_sha, source_commit)
        case_passed = True
        return {"kind": kind, "status": "CASE_GATES_PASSED_NOT_COMPLETE", "project": identity["project"],
                "database": database, "backup_id": identity["backup_id"],
                "driver_result_sha256": file_digest(driver_evidence / f"driver-result-{identity['backup_id']}.json")}
    finally:
        if armed:
            original_error = sys.exc_info()[1]
            cleanup_errors = []
            label = f"label=com.docker.compose.project={identity['project']}"
            # Capture first-cause PostgreSQL and container evidence before any
            # stop/down; failed cases retain the isolated volume for diagnosis.
            for evidence_label, command in (
                ("pg-logs-precleanup", compose + ["logs", "--no-color", "pg"]),
                ("containers-precleanup", ["/usr/bin/docker", "ps", "-a", "--no-trunc", "--filter", label]),
            ):
                try:
                    runner.run(kind + "-" + evidence_label, command, allowed=(0, 1), timeout=60)
                except Exception as error:
                    cleanup_errors.append(evidence_label + ":" + type(error).__name__)
            ids = subprocess.run(["/usr/bin/docker", "ps", "-aq", "--filter", label],
                                 capture_output=True, check=False).stdout.split()
            if ids:
                try:
                    runner.run(kind + "-container-inspect-precleanup",
                               ["/usr/bin/docker", "inspect", *[item.decode() for item in ids]], timeout=60)
                except Exception as error:
                    cleanup_errors.append("container-inspect:" + type(error).__name__)
            volume = identity["project"] + "_pg"
            volume_before_cleanup = subprocess.run(["/usr/bin/docker", "volume", "inspect", volume],
                                                   capture_output=True, check=False).returncode == 0
            if volume_before_cleanup:
                try:
                    runner.run(kind + "-volume-inspect-precleanup",
                               ["/usr/bin/docker", "volume", "inspect", volume], timeout=60)
                except Exception as error:
                    cleanup_errors.append("volume-inspect:" + type(error).__name__)
            try:
                down = compose + ["down", "--remove-orphans"]
                if case_passed:
                    down.append("--volumes")
                runner.run(kind + "-cleanup", down, timeout=300)
            except Exception as error:
                cleanup_errors.append("compose-down:" + type(error).__name__)
            for scope, cmd in (("container", ["ps", "-aq"]), ("network", ["network", "ls", "-q"])):
                try:
                    require(not subprocess.check_output(["/usr/bin/docker", *cmd, "--filter", label]).strip(),
                            "case cleanup left " + scope)
                except Exception as error:
                    cleanup_errors.append(scope + ":" + type(error).__name__)
            volume_after_cleanup = subprocess.run(["/usr/bin/docker", "volume", "inspect", volume],
                                                  capture_output=True, check=False).returncode == 0
            if volume_after_cleanup != (volume_before_cleanup and not case_passed):
                cleanup_errors.append("volume:unexpected-postcleanup-state")
            private_file(case / "cleanup.json", json.dumps({
                "project": identity["project"],
                "volume_present_before_cleanup": volume_before_cleanup,
                "volume_present_after_cleanup": volume_after_cleanup,
                "volume_retained_for_failure": not case_passed and volume_after_cleanup,
                "errors": cleanup_errors,
            }, sort_keys=True).encode())
            if cleanup_errors and original_error is None:
                raise RuntimeError("isolated case cleanup or evidence capture failed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--archive-sha256", required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--batch-id", required=True)
    args = parser.parse_args()
    require(os.name == "posix" and os.geteuid() == 0, "root Linux execution required")
    os.umask(0o077)
    batch_id = uuid.UUID(args.batch_id)
    require(str(batch_id) == args.batch_id, "canonical batch UUID required")
    require(Path(__file__).resolve().is_relative_to(BASE), "run only from reviewed root-owned source")
    assert_root_path(Path(__file__).resolve())
    manifest = verify_archive(args.archive, args.archive_sha256, args.manifest_sha256)
    identities = [case_identity(batch_id, kind) for kind in CASES]
    preflight_projects(identities, SUBNETS)
    require(BASE.is_dir(), "root-owned /var/lib/knowweave-c4 must be prepared")
    assert_root_path(BASE)
    batches = BASE / "batches"
    if not batches.exists():
        private_dir(batches)
    assert_root_path(batches)
    batch = private_dir(batches / (args.archive_sha256[:12] + "-" + str(batch_id)))
    private_dir(batch / "evidence")
    private_dir(batch / "cases")
    archive_copy = batch / "approved-source.zip"
    private_file(archive_copy, args.archive.read_bytes(), 0o400)
    verify_archive(archive_copy, args.archive_sha256, args.manifest_sha256)
    source = batch / "source"
    extract_verified(archive_copy, manifest, source)
    before = source_hashes(source, manifest)
    runner = Runner(batch, [])
    status = "FAILED"
    results = []
    failure = None
    try:
        binaries = build_reviewed_binaries(batch, source, manifest, args.archive_sha256, runner)
        for kind, subnet in zip(CASES, SUBNETS):
            results.append(run_one_case(case_identity(batch_id, kind), subnet, batch, source,
                                        binaries, runner, args.archive_sha256, manifest["commit"]))
        status = "TASK3_CANDIDATE_GATES_PASSED_NOT_PRODUCTION"
    except Exception as error:
        failure = {"type": type(error).__name__, "message": str(error)}
    finally:
        try:
            unchanged = source_hashes(source, manifest) == before
        except Exception:
            unchanged = False
        if not unchanged:
            status = "FAILED_SOURCE_CHANGED"
        record = {"status": status, "archive_sha256": args.archive_sha256,
                  "manifest_sha256": args.manifest_sha256, "source_commit": manifest["commit"],
                  "source_unchanged": unchanged, "cases": results, "failure": failure,
                  "evidence": str(batch / "evidence")}
        private_file(batch / "result.json", json.dumps(record, sort_keys=True, indent=2).encode())
        print(json.dumps({"status": status, "evidence": str(batch / "result.json"),
                          "result_sha256": file_digest(batch / "result.json")}, sort_keys=True))
    return 0 if status == "TASK3_CANDIDATE_GATES_PASSED_NOT_PRODUCTION" else 1


if __name__ == "__main__":
    raise SystemExit(main())
