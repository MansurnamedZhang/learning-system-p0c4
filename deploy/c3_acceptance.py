import json
import pathlib
import re


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
    evidence.mkdir(mode=0o700, parents=False, exist_ok=False)
    env = os.environ.copy()
    # Initial parsing needs a job variable before the business seed exists.
    env["C3_JOB"] = str(uuid.UUID(int=1))
    base = ["docker", "compose", "-p", project, "-f", str(root / "deploy/compose.test.yaml")]
    compose = base + ["-f", str(root / "deploy/compose.c3-acceptance.yaml")]
    counter = 0
    records = {}
    containers = []

    def command(label, argv, allowed=(0,)):
        nonlocal counter
        counter += 1
        prefix = evidence / f"{counter:04}-{label}"
        records[label] = prefix.name
        prefix.with_suffix(".command.json").write_text(json.dumps(argv))
        try:
            process = subprocess.run(argv, cwd=root, env=env, capture_output=True, timeout=7200)
        except subprocess.TimeoutExpired as error:
            prefix.with_suffix(".stdout.log").write_bytes(error.stdout or b"")
            prefix.with_suffix(".stderr.log").write_bytes(error.stderr or b"")
            prefix.with_suffix(".exit").write_text("TIMEOUT")
            raise
        prefix.with_suffix(".stdout.log").write_bytes(process.stdout)
        prefix.with_suffix(".stderr.log").write_bytes(process.stderr)
        prefix.with_suffix(".exit").write_text(str(process.returncode))
        if process.returncode not in allowed:
            raise RuntimeError(f"{label} exit {process.returncode}; raw evidence retained")
        return process.stdout.decode()

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
    started = False
    outcome = "FAILED"
    try:
        for image in [env["C3_IMAGE"],env["C3_RUNTIME_IMAGE"]]:
            command("local-fixed-image", ["docker", "image", "inspect", image])
        command("config", compose + ["config"])
        command("initialize-private-volumes", compose + ["run", "--rm", "--no-deps", "c3-init"])
        command("postgres-start", base + ["up", "-d", "--wait", "pg"])
        started = True
        test = f"{project}-workspace"
        command("workspace-start", compose + ["run", "-d", "--no-deps", "--name", test, "c3-test"])
        containers.append(test)
        code = command("workspace-wait", ["docker", "wait", test]).strip()
        stdout = command("workspace-logs", ["docker", "logs", test])
        command("workspace-evidence", ["docker", "cp", f"{test}:/evidence", str(evidence / "workspace")])
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
    finally:
        if started:
            # Stop only this new project's services. Never down -v or prune.
            try:
                for name in containers:
                    info = inspect(name)
                    if info["Config"]["Labels"].get("com.docker.compose.project") != project:
                        raise RuntimeError("refusing to stop a foreign container")
                    if info["State"]["Running"]:
                        command("stop-owned-container", ["docker", "stop", name])
                    command("retained-container-logs", ["docker", "logs", name])
                    inspect(name)
                command("stop-new-project", base + ["stop", "pg"])
                pg = inspect(f"{project}-pg-1")
                if pg["State"]["Running"] or pg["State"]["ExitCode"] != 0:
                    outcome = "FAILED_POSTGRES_STOP"
            except (RuntimeError, subprocess.TimeoutExpired):
                outcome = "FAILED_STOP"
        after = source_hashes(root)
        (evidence / "source-after.json").write_text(json.dumps(after, sort_keys=True))
        if after != before:
            outcome = "FAILED_SOURCE_CHANGED"
        (evidence / "result.json").write_text(json.dumps({"status":outcome,"source_unchanged":after==before,"project":project}, indent=2))
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
