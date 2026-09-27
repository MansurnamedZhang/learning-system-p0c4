import json
import pathlib
import re


SECRET_NAMES = ("postgres_password", "admin_password", "runtime_password")


def check_docker_identity(options):
    if not isinstance(options, list) or any("rootless" in item or "userns" in item for item in options):
        raise ValueError("acceptance requires rootful Docker without user namespace remapping")


def check_secret_metadata(info, owner):
    import stat
    if not stat.S_ISREG(info.st_mode) or stat.S_IMODE(info.st_mode) != 0o600 or (info.st_uid, info.st_gid) != owner or info.st_nlink != 1:
        raise ValueError("secret must be a single regular 0600 file with the required uid/gid")


def check_secret_pair(client, postgres):
    import hmac
    if not re.fullmatch(b"[0-9a-f]{64}", client) or not re.fullmatch(b"[0-9a-f]{64}", postgres) or not hmac.compare_digest(client, postgres):
        raise ValueError("secret copies must contain identical 64 lowercase hex bytes, without newline")


def load_secrets(root):
    """Read locally before recording any command output; never serialize values."""
    import os
    import stat
    if os.geteuid() != 0:
        raise ValueError("host driver requires root to verify both private owner-specific copies")
    directory = root / ".runtime/secrets"
    for path in [root / ".runtime", directory, directory / "pg"]:
        info = path.lstat()
        if not stat.S_ISDIR(info.st_mode) or stat.S_IMODE(info.st_mode) != 0o700 or info.st_uid != 0:
            raise ValueError("private secret directories must be root-owned 0700 ordinary directories")
    values, metadata = [], {}
    for name in SECRET_NAMES:
        pair = []
        for prefix in ["", "pg"]:
            path = directory / prefix / name
            initial = path.lstat()
            owner = (65532, 65532) if not prefix else (initial.st_uid, initial.st_gid)
            check_secret_metadata(initial, owner)
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            with os.fdopen(fd, "rb") as source:
                info = os.fstat(source.fileno())
                # PG ownership is verified after the no-network fixed-image identity probe.
                check_secret_metadata(info, owner)
                data = source.read(65)
            pair.append(data)
            metadata[path.relative_to(root).as_posix()] = info
        check_secret_pair(*pair)
        values.extend(pair)
    if len(set(values)) != 3:
        raise ValueError("each database role must have a distinct password")
    return values, metadata


def redact(data, secrets):
    for secret in sorted(set(secrets), key=len, reverse=True):
        data = data.replace(secret, b"[REDACTED]")
    return data


def image_source_manifest(root):
    """Resolve this Dockerfile's deliberately simple COPY grammar and deny-all context."""
    import fnmatch
    import hashlib
    import shlex
    rules = (root / ".dockerignore").read_text().splitlines()
    def included(relative):
        allow = True
        for rule in rules:
            if not rule or rule.startswith("#"): continue
            negated = rule.startswith("!")
            pattern = rule.lstrip("!").rstrip("/")
            if fnmatch.fnmatchcase(relative, pattern) or relative.startswith(pattern + "/"):
                allow = negated
        return allow
    result = {}
    for line in (root / "deploy/Dockerfile.test").read_text().splitlines():
        if not line.startswith("COPY "): continue
        parts = shlex.split(line)[1:]
        if len(parts) < 2 or any(x.startswith("--") or any(c in x for c in "*?[") for x in parts):
            raise ValueError("unsupported COPY grammar: update provenance verifier first")
        destination = pathlib.PurePosixPath(parts[-1])
        if not destination.is_absolute(): destination = pathlib.PurePosixPath("/app") / destination
        for source in parts[:-1]:
            path = root / source
            if not included(source) or not path.exists() or path.is_symlink():
                raise ValueError(f"COPY input absent from Docker context: {source}")
            if path.is_dir():
                files = sorted(p for p in path.rglob("*") if p.is_file())
                for p in files:
                    if not included(p.relative_to(root).as_posix()): continue
                    if p.is_symlink(): raise ValueError("symlink COPY input")
                    result[str(destination / p.relative_to(path).as_posix())] = hashlib.sha256(p.read_bytes()).hexdigest()
            else:
                target = destination / path.name if parts[-1].endswith("/") or len(parts) > 2 else destination
                result[str(target)] = hashlib.sha256(path.read_bytes()).hexdigest()
    if not result: raise ValueError("no image source inputs")
    return result


def check_image_sources(expected, output):
    actual = {}
    for line in output.splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  (/[^\r\n]+)", line)
        if not match or match[2] in actual: raise ValueError("invalid image source manifest")
        actual[match[2]] = match[1]
    if actual != expected: raise ValueError("image copied source differs from frozen host source")


def check_pg_config(configuration, root):
    mounted = configuration["services"]["pg"]["secrets"]
    expected = {name: "pg_" + name for name in SECRET_NAMES}
    if len(mounted) != 3 or {item["target"]:item["source"] for item in mounted} != expected:
        raise ValueError("merged PG secrets must contain only the three PG-owned copies")
    for name, source in expected.items():
        if pathlib.Path(configuration["secrets"][source]["file"]).resolve() != (root / ".runtime/secrets/pg" / name).resolve():
            raise ValueError("unexpected PG secret source")


def check_pg_mounts(info, root):
    actual = {item["Destination"]:item for item in info["Mounts"] if item["Destination"].startswith("/run/secrets/")}
    if set(actual) != {"/run/secrets/" + name for name in SECRET_NAMES}:
        raise ValueError("PG secret mount set differs")
    for name in SECRET_NAMES:
        item = actual["/run/secrets/" + name]
        if item["Type"] != "bind" or item["RW"] or pathlib.Path(item["Source"]).resolve() != (root / ".runtime/secrets/pg" / name).resolve():
            raise ValueError("PG secret bind ownership source differs")


def cleanup_owned(project, command, known):
    """Every operation is independent; a failed inspect never authorizes a stop."""
    errors = []
    names = set(known)
    try:
        names.update(command("cleanup-discover", ["docker", "ps", "-aq", "--filter", f"label=com.docker.compose.project={project}"]).split())
    except Exception as error:
        errors.append(f"discover: {type(error).__name__}")
    for name in sorted(names):
        try:
            info = json.loads(command("cleanup-inspect", ["docker", "inspect", name]))[0]
            if info["Config"]["Labels"].get("com.docker.compose.project") != project:
                raise ValueError("foreign container")
        except Exception as error:
            errors.append(f"inspect {name}: {type(error).__name__}")
            continue
        for label, argv in [("cleanup-stop", ["docker", "stop", name]), ("cleanup-logs", ["docker", "logs", name]), ("cleanup-final-inspect", ["docker", "inspect", name])]:
            try:
                output = command(label, argv)
                if label == "cleanup-final-inspect":
                    final = json.loads(output)[0]
                    if final["State"]["Running"]:
                        raise RuntimeError("container remains running")
                    if final["Config"]["Labels"].get("com.docker.compose.service") == "pg" and final["State"]["ExitCode"] != 0:
                        raise RuntimeError("PostgreSQL did not stop cleanly")
            except Exception as error:
                errors.append(f"{label} {name}: {type(error).__name__}")
    return errors


def save_evidence_tar(data, directory, secrets):
    import io
    import tarfile
    # Never extract raw files, links, devices or paths outside this private evidence tree.
    seen = set()
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        for member in archive:
            path = pathlib.PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or not path.parts or path.parts[0] != "evidence":
                raise ValueError("unexpected evidence archive path")
            target = directory.joinpath(*path.parts[1:])
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True, mode=0o700)
            elif member.isfile() and member.size <= 128 * 1024 * 1024 and target not in seen:
                seen.add(target)
                target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                with target.open("xb") as output:
                    output.write(redact(archive.extractfile(member).read(), secrets))
            else:
                raise ValueError("unexpected evidence archive entry")


def inventory(root):
    return json.loads((root / "deploy/c3-databases.json").read_text(encoding="utf-8-sig"))


def validate_project(name):
    if not re.fullmatch(r"learning-system-p0c3-task7-[a-z0-9][a-z0-9-]{5,48}", name):
        raise ValueError("require a new uniquely suffixed Task 7 project")


def check_worker(info, network):
    config, host = info["Config"], info["HostConfig"]
    allowed = {"/run/secrets/runtime_password", "/assets", "/usr/local/bin/learning-worker", "/snapshots", "/gate", "/staging", "/tmp"}
    mounts = {m["Destination"]: m for m in info["Mounts"]}
    project = network.removesuffix("_test")
    if host.get("Privileged") or host.get("Devices") or host.get("DeviceRequests"):
        raise ValueError("Worker privileged/device override")
    for path, suffix in [("/assets","test_assets"),("/snapshots","c3_snapshots")]:
        if mounts.get(path,{}).get("Type") != "volume" or mounts[path].get("Name") != f"{project}_{suffix}":
            raise ValueError("Worker must use only this new project's volumes")
    if set(mounts) - allowed:
        raise ValueError("unexpected Worker mount")
    if config["User"] != "65532:65532" or not host["ReadonlyRootfs"]:
        raise ValueError("Worker identity/rootfs")
    if "ALL" not in host["CapDrop"] or "no-new-privileges:true" not in host["SecurityOpt"]:
        raise ValueError("Worker privilege escalation")
    if host["NanoCpus"] != 2000000000 or host["Memory"] != 2147483648 or host["PidsLimit"] != 128:
        raise ValueError("Worker resource limits")
    if host.get("PortBindings") or info["NetworkSettings"].get("Ports") or set(info["NetworkSettings"]["Networks"]) != {network}:
        raise ValueError("Worker network exposure")
    for name in ["/assets", "/usr/local/bin/learning-worker", "/run/secrets/runtime_password"]:
        if name not in mounts or mounts[name]["RW"]:
            raise ValueError("missing/read-write input mount")
    if mounts.get("/snapshots", {}).get("Type") != "volume" or not mounts["/snapshots"]["RW"]:
        raise ValueError("snapshot storage must be a durable writable volume")
    env = dict(item.split("=", 1) for item in config["Env"])
    if env.get("SNAPSHOT_ROOT") != "/snapshots" or any("DATABASE_URL" in k or "PASSWORD" in k for k in env):
        raise ValueError("credentials may not be in inspect environment")

# Each group requires concrete successful negative cases in raw workspace output.
# The atomic test includes named inline assertions for collision and late SQL rollback.
FAILURES = {
    "interrupted-staging": ["interrupted_input_never_creates_ready_package", "process_exit_before_seal_never_publishes", "rejects_premature_eof_against_declared_manifest_length"],
    "invalid-package": ["rejects_extra_files_corrupt_content_and_reparse_points", "missing_and_corrupt_asset_bytes_never_publish", "malformed_closure_authors_parent_and_body_leave_all_counts_unchanged", "no_originals_requires_same_space_and_asset_id_and_real_bytes", "revoked_and_read_only_root_grants_cannot_preflight_and_copy_is_rejected"],
    "authorization-and-lease": ["integration::revocation_after_plan_or_copy_prevents_result_and_delivery", "integration::old_token_cannot_publish_and_winning_package_survives_restart_until_revocation", "private_overlay_revision_id_never_reports_an_identity_collision"],
    "target-collision-and-rollback": ["preflight_reuses_exact_rows_read_only_and_rejects_identity_metadata_changes", "attention_exact_atomic_roundtrip_two_fresh_databases_and_failure_boundaries"],
}


def failure_evidence(stdout):
    result = {}
    for group, cases in FAILURES.items():
        for case in cases:
            if not re.search(r"^test " + re.escape(case) + r" \.\.\. ok$", stdout, re.M):
                raise ValueError(f"missing actual negative-case pass: {case}")
        result[group] = [{"test":case,"result":"ok","exit":0,"stdout":"workspace/workspace.stdout.log","exit_file":"workspace/workspace.exit"} for case in cases]
    return result


def source_hashes(root):
    import hashlib
    import os
    ignored = {".git", ".runtime", ".superpowers", "target", "__pycache__"}
    result = {}
    for directory, dirs, files in os.walk(root):
        dirs[:] = sorted(d for d in dirs if d not in ignored)
        for name in sorted(files):
            p = pathlib.Path(directory) / name
            if name == ".git" or p.suffix == ".pyc": continue
            if p.is_symlink(): raise ValueError("symlink in frozen source")
            result[p.relative_to(root).as_posix()] = hashlib.sha256(p.read_bytes()).hexdigest()
    return result


def run_acceptance(root, evidence):
    """Explicit invocation only, on an authorized fresh Linux isolation project."""
    import hashlib
    import os
    import subprocess
    import time
    import uuid
    project = os.environ["C3_PROJECT"]
    validate_project(project)
    if os.name != "posix":
        raise ValueError("real acceptance requires Linux")
    for key in ["C3_IMAGE", "C3_RUNTIME_IMAGE"]:
        if not re.fullmatch(r"(?:[^\s]+@)?sha256:[0-9a-f]{64}", os.environ[key]):
            raise ValueError("images must be fixed digest/image IDs")
    binary = pathlib.Path(os.environ["C3_WORKER_BIN"])
    if not binary.is_absolute() or not binary.is_file() or binary.is_symlink():
        raise ValueError("explicit ordinary prebuilt binary required")
    evidence = evidence.resolve()
    if evidence == root or root in evidence.parents:
        raise ValueError("evidence directory must be outside frozen source")
    secrets, secret_metadata = load_secrets(root)
    evidence.mkdir(mode=0o700, parents=False, exist_ok=False)
    env = os.environ.copy()
    # Initial parsing needs a job variable before the business seed exists.
    env["C3_JOB"] = str(uuid.UUID(int=1))
    base = ["docker", "compose", "-p", project, "-f", str(root / "deploy/compose.test.yaml")]
    compose = base + ["-f", str(root / "deploy/compose.c3-acceptance.yaml")]
    counter = 0
    records = {}
    containers = []

    def command(label, argv, allowed=(0,), binary_output=False):
        nonlocal counter
        counter += 1
        prefix = evidence / f"{counter:04}-{label}"
        records[label] = prefix.name
        prefix.with_suffix(".command.json").write_bytes(redact(json.dumps(argv).encode(), secrets))
        try:
            process = subprocess.run(argv, cwd=root, env=env, capture_output=True, timeout=7200)
        except subprocess.TimeoutExpired as error:
            prefix.with_suffix(".stdout.log").write_bytes(redact(error.stdout or b"", secrets))
            prefix.with_suffix(".stderr.log").write_bytes(redact(error.stderr or b"", secrets))
            prefix.with_suffix(".exit").write_text("TIMEOUT")
            raise RuntimeError(f"{label} TIMEOUT; redacted evidence retained") from None
        prefix.with_suffix(".stdout.log").write_bytes(b"archive entries individually redacted before storage\n" if binary_output else redact(process.stdout, secrets))
        prefix.with_suffix(".stderr.log").write_bytes(redact(process.stderr, secrets))
        prefix.with_suffix(".exit").write_text(str(process.returncode))
        if process.returncode not in allowed:
            raise RuntimeError(f"{label} exit {process.returncode}; redacted evidence retained")
        return process.stdout if binary_output else redact(process.stdout, secrets).decode()

    def manager(action):
        output = command(action, compose + ["run", "--no-deps", "--rm", "c3-manager", action])
        return json.loads(output.strip().splitlines()[-1])

    def inspect(name):
        return json.loads(command("inspect", ["docker", "inspect", name]))[0]

    def poll(label, predicate, seconds=90):
        until = time.monotonic() + seconds
        while time.monotonic() < until:
            if predicate():
                return
            time.sleep(0.25)
        raise RuntimeError(f"{label} did not reach the required observable state")

    def worker(label):
        name = f"{project}-{label}"
        command(label, compose + ["run", "-d", "--no-deps", "--name", name, "c3-worker"])
        containers.append(name)
        poll("claim gate", lambda: gate_exists(name), 25)
        info = inspect(name)
        check_worker(info, f"{project}_test")
        if info["Config"]["Image"] != env["C3_RUNTIME_IMAGE"]:
            raise RuntimeError("Worker image changed")
        mounted_binary = next(m for m in info["Mounts"] if m["Destination"] == "/usr/local/bin/learning-worker")
        if pathlib.Path(mounted_binary["Source"]).resolve() != binary.resolve():
            raise RuntimeError("Worker binary mount changed")
        command("runtime-boundary", ["docker", "exec", name, "sh", "-eu", "-c", "test ! -e /run/secrets/admin_password; test ! -e /run/secrets/postgres_password; test ! -e /var/run/docker.sock; test ! -e /app; test ! -w /assets; test \"$(stat -c '%a:%u' /snapshots)\" = 700:65532"])
        return name

    def gate_exists(name):
        # Exit status is the evidence; use cat after it exists, never a fixed delay.
        output = command("gate-state", ["docker", "exec", name, "sh", "-c", "if test -f /gate/claimed; then cat /gate/claimed; else printf pending; fi"])
        return output.startswith("pid=")

    for line in (root / "deploy/c3-migrations-0001-0013.sha256").read_text().splitlines():
        digest, relative = line.split("  ", 1)
        if hashlib.sha256((root / relative).read_bytes()).hexdigest() != digest:
            raise ValueError("frozen migration changed")
    before = source_hashes(root)
    (evidence / "source-before.json").write_text(json.dumps(before, sort_keys=True))
    (evidence / "worker.sha256").write_text(hashlib.sha256(binary.read_bytes()).hexdigest())
    # Check ALL object kinds before touching the new project, including stopped containers.
    for kind, args in [("container", ["ps", "-aq"]), ("network", ["network", "ls", "-q"]), ("volume", ["volume", "ls", "-q"])]:
        if command("fresh-" + kind, ["docker"] + args + ["--filter", f"label=com.docker.compose.project={project}"]).strip():
            raise ValueError("project already exists; never reset/reuse its evidence")
    for suffix in ["test_pg", "test_evidence", "test_assets", "test_staging", "c3_snapshots", "c3_control", "c3_destination"]:
        if command("fresh-volume-name", ["docker", "volume", "inspect", f"{project}_{suffix}"], (0,1)).strip() not in ("", "[]"):
            raise ValueError("named volume exists even without project label")
    if command("fresh-network-name", ["docker", "network", "inspect", f"{project}_test"], (0,1)).strip() not in ("", "[]"):
        raise ValueError("named network already exists")
    cleanup_armed = False
    primary_failure = None
    outcome = "FAILED"
    try:
        context_endpoint = json.loads(command("docker-endpoint", ["docker", "context", "inspect", "--format", "{{json .Endpoints.docker.Host}}"]))
        endpoint = context_endpoint if env.get("DOCKER_CONTEXT") else env.get("DOCKER_HOST") or context_endpoint
        if endpoint != "unix:///var/run/docker.sock":
            raise ValueError("use the local rootful Docker socket; remote/rootless endpoints are unsupported")
        check_docker_identity(json.loads(command("docker-security-options", ["docker", "info", "--format", "{{json .SecurityOptions}}"])))
        for image in [env["C3_IMAGE"],env["C3_RUNTIME_IMAGE"]]:
            command("local-fixed-image", ["docker", "image", "inspect", image])
        # Read-only preflight probes have no network; only readability probes mount secrets.
        cleanup_armed = True
        probe = ["docker", "run", "--rm", "--network", "none", "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true", "--pids-limit", "32", "--label", f"com.docker.compose.project={project}"]
        pg_text = (root / "deploy/compose.test.yaml").read_text()
        pg_image = re.search(r"(?m)^    image: (postgres:[^\s]+@sha256:[0-9a-f]{64})$", pg_text)[1]
        command("local-fixed-pg-image", ["docker", "image", "inspect", pg_image])
        identity = command("postgres-image-identity", probe + ["--entrypoint", "sh", pg_image, "-eu", "-c", "id -u postgres; id -g postgres"]).splitlines()
        if len(identity) != 2 or not all(re.fullmatch(r"[0-9]+", x) for x in identity):
            raise ValueError("invalid postgres image identity")
        pg_owner = tuple(int(x) for x in identity)
        for name in SECRET_NAMES:
            check_secret_metadata(secret_metadata[f".runtime/secrets/pg/{name}"], pg_owner)
        for label, image, owner, prefix in [("pg", pg_image, pg_owner, "pg"), ("client", env["C3_RUNTIME_IMAGE"], (65532,65532), "")]:
            mounts = []
            checks = []
            for name in SECRET_NAMES:
                mounts += ["--mount", f"type=bind,src={root / '.runtime/secrets' / prefix / name},dst=/run/secrets/{name},readonly"]
                checks.append(f'test "$(stat -c %a:%u:%g /run/secrets/{name})" = 600:{owner[0]}:{owner[1]}; test "$(wc -c < /run/secrets/{name})" = 64')
            command(f"{label}-secret-readable", probe + ["--user", f"{owner[0]}:{owner[1]}"] + mounts + ["--entrypoint", "sh", image, "-eu", "-c", "; ".join(checks)])
        expected = image_source_manifest(root)
        image_manifest = ""
        paths = sorted(expected)
        for offset in range(0, len(paths), 100):
            image_manifest += command("image-source-sha256", probe + ["--user", "65532:65532", "--entrypoint", "sha256sum", env["C3_IMAGE"]] + paths[offset:offset+100])
        check_image_sources(expected, image_manifest)
        (evidence / "image-source-verified.json").write_text(json.dumps(expected, sort_keys=True))
        configuration = json.loads(command("config", compose + ["config", "--format", "json"]))
        check_pg_config(configuration, root)
        command("initialize-private-volumes", compose + ["run", "--rm", "--no-deps", "c3-init"])
        command("postgres-start", compose + ["up", "-d", "--wait", "pg"])
        check_pg_mounts(inspect(f"{project}-pg-1"), root)
        test = f"{project}-workspace"
        command("workspace-start", compose + ["run", "-d", "--no-deps", "--name", test, "c3-test"])
        containers.append(test)
        code = command("workspace-wait", ["docker", "wait", test]).strip()
        stdout = command("workspace-logs", ["docker", "logs", test])
        archive = command("workspace-evidence", ["docker", "cp", f"{test}:/evidence", "-"], binary_output=True)
        save_evidence_tar(archive, evidence / "workspace", secrets)
        if code != "0":
            raise RuntimeError(f"workspace exit {code}; later gates stopped")
        for case in ["real_p0a_data_and_idempotent_receipt_survive_additive_upgrade", "actual_b1_stores_and_receipts_survive_b2_upgrade", "genuine_five_store_b2_history_and_replays_survive_b3_upgrade", "pre_b3_revisions_are_backfilled_without_changing_old_checksums_or_content"]:
            if not re.search(r"^test " + re.escape(case) + r" \.\.\. ok$", stdout, re.M):
                raise RuntimeError(f"missing frozen legacy upgrade pass: {case}")
        fixtures = evidence / "workspace"
        for stage in ["empty-databases", "workspace", "legacy-upgrades", "fmt", "clippy"]:
            if (fixtures / f"{stage}.exit").read_text().strip() != "0":
                raise RuntimeError(f"required stage {stage} did not pass")
        sums = json.loads((fixtures / "legacy-upgrades.json").read_text())
        if len(sums) != 4 or any(len(rows) != 15 for rows in sums.values()):
            raise RuntimeError("four complete migration checksum sequences required")
        for kind in ["p0a", "b1", "b2", "b3-schema"]:
            manifest = json.loads((fixtures / f"{kind}.json").read_text())
            if manifest["started_empty"] is not True or manifest["source_manifest_sha256"] != "2da005cdf6f2e92ea4fa2f34463b371a71c30e145e6d8927eb75db558b284225":
                raise RuntimeError("frozen producer manifest mismatch")
        (evidence / "failure-groups.json").write_text(json.dumps(failure_evidence(stdout), indent=2))
        if manager("binary")["sha256"] != hashlib.sha256(binary.read_bytes()).hexdigest():
            raise RuntimeError("Worker binary differs from frozen image build")
        seed = manager("seed")
        env["C3_JOB"] = seed["job_id"]
        first = worker("worker-first")
        captured = manager("capture")
        initial = manager("status")
        if initial != {"status":"snapshot_running", "attempt":1, "digest":None,"expired":False,"results":0}:
            raise RuntimeError("first claim state")
        command("real-sigkill", ["docker", "kill", "--signal=KILL", first])
        if command("killed-wait", ["docker", "wait", first]).strip() != "137" or inspect(first)["State"]["OOMKilled"]:
            raise RuntimeError("first worker was not killed by requested SIGKILL")
        poll("database-clock lease expiry", lambda: manager("status")["expired"])
        second = worker("worker-second")
        fenced = manager("fence")
        if captured["token_sha256"] != fenced["token_sha256"]:
            raise RuntimeError("old-token evidence changed")
        command("release-second", ["docker", "exec", second, "touch", "/gate/go"])
        if command("second-wait", ["docker", "wait", second]).strip() != "0":
            raise RuntimeError("replacement worker failed")
        command("first-logs", ["docker", "logs", first]); command("second-logs", ["docker", "logs", second])
        final = manager("status")
        if final["status"] != "succeeded" or final["attempt"] != 2 or final["results"] != 1 or not re.fullmatch("[0-9a-f]{64}", final["digest"] or ""):
            raise RuntimeError("exactly one export result required")
        third = f"{project}-worker-repeat"
        command("repeat-start", compose + ["run", "-d", "--no-deps", "--name", third, "c3-worker"])
        containers.append(third)
        if command("repeat-wait", ["docker", "wait", third]).strip() != "0":
            raise RuntimeError("repeat worker failed")
        check_worker(inspect(third), f"{project}_test")
        if manager("status") != final:
            raise RuntimeError("completed job changed on repeated Worker")
        result = manager("roundtrip")
        if result["manifest_sha256"] != final["digest"]:
            raise RuntimeError("delivered manifest differs from published result")
        if not result.get("wrong_actor_delivery_rejected") or not result.get("wrong_actor_import_rejected"):
            raise RuntimeError("wrong-actor negative assertions missing")
        manager("revoke")
        groups = failure_evidence(stdout)
        for action, case in [("roundtrip", "wrong_actor_delivery_and_import"), ("fence", "real_sigkill_replacement_fences_old_token"), ("revoke", "revoke_before_delivery")]:
            groups["authorization-and-lease"].append({"test":case,"result":"ok","exit":0,"stdout":records[action]+".stdout.log","exit_file":records[action]+".exit"})
        (evidence / "failure-groups.json").write_text(json.dumps(groups,indent=2))
        outcome = "CANDIDATE_GATES_PASSED_NOT_PRODUCTION"
    except Exception as error:
        primary_failure = {"type":type(error).__name__, "message":redact(str(error).encode(), secrets).decode()}
        raise
    finally:
        cleanup_errors = cleanup_owned(project, command, containers + [f"{project}-pg-1"]) if cleanup_armed else []
        if cleanup_errors and outcome == "CANDIDATE_GATES_PASSED_NOT_PRODUCTION":
            outcome = "FAILED_CLEANUP"
        after = source_hashes(root)
        (evidence / "source-after.json").write_text(json.dumps(after, sort_keys=True))
        if after != before:
            outcome = "FAILED_SOURCE_CHANGED"
        (evidence / "result.json").write_text(json.dumps({"status":outcome,"source_unchanged":after==before,"project":project,"primary_failure":primary_failure,"cleanup_errors":cleanup_errors}, indent=2))
        # Relative paths preserve multiple stdout filenames across evidence subdirectories.
        hashes = {p.relative_to(evidence).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(evidence.rglob("*")) if p.is_file()}
        (evidence / "evidence.sha256.json").write_text(json.dumps(hashes, sort_keys=True, indent=2))
    if outcome != "CANDIDATE_GATES_PASSED_NOT_PRODUCTION":
        raise RuntimeError(outcome)


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description="Only run after exact package authorization on a fresh isolated Linux project")
    parser.add_argument("--evidence", type=pathlib.Path, required=True)
    args = parser.parse_args()
    run_acceptance(pathlib.Path(__file__).resolve().parent.parent, args.evidence)
