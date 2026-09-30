"""Single-host pin acceptance never promotes a failed or replayed batch."""

import json
import hashlib
import contextlib
import io
from pathlib import Path
import stat
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile

import p0c4_restore_pin_acceptance as runner
import p0c4_task3_transfer_acceptance as transfer_runner
from test_p0c4_restore_target import ID, SUBNET


class PinAcceptance(unittest.TestCase):
    def test_clone_phase_rejects_unapproved_or_sensitive_values(self):
        result = {"clone_phase": "primary-recheck"}
        with self.assertRaises(ValueError):
            runner._mark_clone_phase(result, "sensitive DSN")
        self.assertEqual(result["clone_phase"], "primary-recheck")

    def test_bound_probe_uses_existing_reviewed_builder_image(self):
        self.assertEqual(runner.BUILDER_IMAGE_ID,
                         transfer_runner.BUILDER_IMAGE_ID)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.batch = self.base / "batch"
        self.batch.mkdir()
        for name in ("source", "control", "evidence"):
            (self.batch / name).mkdir()
        (self.batch / "control" / "targets").mkdir()
        self.args = SimpleNamespace(batch_id=ID, subnet=SUBNET,
                                    archive_sha256="a" * 64,
                                    manifest_sha256="b" * 64,
                                    source_commit="c" * 40,
                                    runner_sha256="3" * 64)
        sync = patch.object(runner, "_sync_dir")
        sync.start()
        self.addCleanup(sync.stop)

    def _dependencies(self):
        identity = {"project": "new-project", "database": "new-database",
                    "volume": "new-volume", "image": "postgres:18"}
        snapshot = {"daemon_id": "new-daemon", "containers": [],
                    "networks": [], "volumes": [], "routes": []}
        provisioner = SimpleNamespace(identity_for=lambda _: identity,
                                      snapshot=lambda: snapshot,
                                      admit_fresh=lambda *_: None)
        birth = {"database_oid": 42}
        state = {"subnet": SUBNET}
        success = {"birth_sha256": "d" * 64, "container_id": "e" * 64}
        birth_evidence = json.dumps({
            "container_id": "e" * 64,
            "container_started_at": "2026-09-29T00:00:00Z"}).encode()
        acceptance = SimpleNamespace(
            _run_issuer=lambda *_: SimpleNamespace(returncode=0, stdout="issued"),
            accept_issuer_process=lambda *_: (birth, state, success),
            _independent_docker_gate=lambda *_: {"container_id": "e" * 64},
            stop_verified_pg=lambda *_: {"confirmed": True, "volume_retained": True},
            stop_early_owned_pg=lambda *_: {"confirmed": True, "volume_retained": True},
            read_issuer_diagnostic=lambda *_: {"status": "UNAVAILABLE"},
            _private_read_diagnostic=lambda path, limit=262144: (
                birth_evidence if path.name == "birth-evidence.json" else
                path.read_bytes()),
            _unique_json=lambda payload: json.loads(payload),
        )
        projection = {"daemon_id": "new-daemon", "containers": [],
                      "networks": [], "volumes": [], "images": []}
        pg = {"runtime_create": False, "user_relations": 0}
        issuer_pg = {"dirty_counts": {}}
        payload = {"format_version": 1,
                   "state": "PIN_CANDIDATE_NOT_RESTORE_AUTHORITY",
                   "batch_id": ID, "control_root": str(self.batch / "control"),
                   "precreation_sha256": "1" * 64,
                   "birth_sha256": "d" * 64,
                   "creation_state_sha256": "a" * 64,
                   "issuance_success_sha256": "b" * 64,
                   "birth_evidence_sha256": hashlib.sha256(
                       birth_evidence).hexdigest(),
                   "live": {"container_id": "e" * 64,
                            "docker_projection": projection,
                            "docker_sha256": hashlib.sha256(
                                runner._json_bytes(projection)).hexdigest(),
                            "pg_observation": pg,
                            "pg_sha256": hashlib.sha256(
                                runner._json_bytes(pg)).hexdigest(),
                            "issuer_pg_observation": issuer_pg,
                            "issuer_pg_sha256": hashlib.sha256(
                                runner._json_bytes(issuer_pg)).hexdigest()}}
        candidate = {"birth_sha256": "d" * 64,
                     "inspection_evidence_sha256": hashlib.sha256(
                         runner._json_bytes(payload)).hexdigest()}
        pin = SimpleNamespace(inspect_candidate=lambda *_: (candidate, payload))
        prepare = SimpleNamespace(prepare=lambda *_: "1" * 64)
        return provisioner, acceptance, pin, prepare

    def _run(self, deps=None, *, private_writes=False):
        deps = deps or self._dependencies()
        with patch.object(runner, "extract_and_load", return_value=(*deps, Path("/initdb"), "2" * 64)), \
             patch.object(runner, "source_digest", return_value="2" * 64), \
             patch.object(runner, "_file_digest", return_value="3" * 64), \
             (contextlib.nullcontext() if private_writes else
              patch.object(runner, "_private_write", side_effect=lambda path, data: path.write_bytes(data))):
            return runner._run_batch(self.args, {}, b"zip", self.batch)

    def _child_diagnostic_failure(self, kind, *, stop_confirmed=True):
        """Keep driver, bounded capture and durable publication real; fake Docker/child I/O."""
        self.args.child_read_only_restart = True
        new_batch = self.base / ID
        self.batch.rename(new_batch)
        self.batch = new_batch
        binary = self.batch / "test-binary"
        binary.write_bytes(b"binary")
        sentinel = b"untrusted-" + b"sensitive-sentinel"
        first = b"CHILD_READ_ONLY_ATTESTED_NOT_RESTORE\n"
        output = b"running 1 test\n" + sentinel + b"\n"
        exit_code = 101
        if kind == "after-first":
            output += first + b"CHILD_DIAG_CHECKPOINT_RESTART_COMPLETED\n"
            output += b"CHILD_DIAG_REJECTION_FAILURE_Session\n"
            output += b"CHILD_DIAG_REJECTION_ISOLATION_UNCONFIRMED_UNUSABLE\n"
        elif kind == "parser":
            output += first
            exit_code = 0
        elif kind == "noise":
            output += b"NOISE_CHILD_READ_ONLY_ATTESTED_NOT_RESTORE\n"
            output += b"CHILD_DIAG_REJECTION_FAILURE_" + sentinel + b"\n"
            output += b"CHILD_DIAG_CHECKPOINT_" + sentinel + b"\n"
        elif kind == "duplicate":
            output += first * 3
            exit_code = 0
        def process(command, *, stdout, stderr, **kwargs):
            stdout.write(output)
            stderr.write(sentinel)
            if kind in ("timeout", "compiler-timeout"):
                raise subprocess.TimeoutExpired(command, kwargs["timeout"],
                                                output=sentinel, stderr=sentinel)
            if kind == "output-limit":
                stdout.write(b"x" * (16 * 1024 * 1024))
            if kind in ("compile-then-exit", "artifact-invalid") and command[0] == "/usr/bin/docker":
                return SimpleNamespace(returncode=0)
            return SimpleNamespace(returncode=exit_code)
        deps = self._dependencies()
        stopped = []
        def stop(*args):
            stopped.append(args[-1])
            return {"confirmed": stop_confirmed, "volume_retained": True}
        deps[1].stop_verified_pg = stop
        compile_patch = (contextlib.nullcontext() if kind in
                         ("compiler", "compiler-timeout", "compile-then-exit", "artifact-invalid") else
                         patch.object(runner, "_compile_bound_probe",
                                      return_value=(binary, "3" * 64)))
        artifact_patch = (patch.object(runner, "_builder_artifact", return_value=binary) if
                          kind == "compile-then-exit" else contextlib.nullcontext())
        captured = io.StringIO()
        with patch.object(runner, "_preflight_probe_builder", return_value={}), \
             patch.object(runner, "_private_dir", side_effect=lambda path: path.mkdir()), \
             patch.object(runner, "_builder_container_ids", return_value=[]), \
             patch.object(runner, "_cleanup_builder"), compile_patch, artifact_patch, \
             patch.object(runner.subprocess, "run", side_effect=process), \
             patch.object(runner.os, "O_NOFOLLOW", 0, create=True), \
             patch.object(runner.os, "chmod", wraps=runner.os.chmod) as chmod, \
             contextlib.redirect_stdout(captured), contextlib.redirect_stderr(captured):
            code, result = self._run(deps, private_writes=True)
        final = self.batch / "evidence" / "result.json"
        payload = final.read_bytes()
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.CHILD_FAILED)
        self.assertEqual(stopped, ["e" * 64])
        self.assertEqual(result["child_isolation"],
                         "STOPPED" if stop_confirmed else "UNCONFIRMED_UNUSABLE")
        self.assertTrue(result["stop"]["volume_retained"])
        self.assertFalse(result["target_reuse_permitted"])
        self.assertEqual(result["source_before_sha256"], result["source_after_sha256"])
        self.assertFalse((final.parent / "result.pending.json").exists())
        self.assertTrue(sentinel not in payload and sentinel.decode() not in captured.getvalue(),
                        "untrusted output must remain absent from durable/public diagnostics")
        self.assertEqual(json.loads(payload), result)
        self.assertIn(((final.parent / "result.pending.json", 0o600), {}),
                      [(call.args, call.kwargs) for call in chmod.call_args_list])
        self.assertIn("child_diagnostics", result,
                      "child process failure observations must survive result finalization")
        return result["child_diagnostics"]

    def test_child_diagnostics_compiler_failure_is_durable_after_exact_stop(self):
        diagnostics = self._child_diagnostic_failure("compiler")
        self.assertEqual(diagnostics["format_version"], 1)
        step = diagnostics["steps"][-1]
        self.assertEqual((step["phase"], step["reason"], step["exit_code"]),
                         ("LIVE_COMPILE", "EXIT_NONZERO", 101))
        self.assertTrue(step["checks"]["builder_cleanup_confirmed"])

    def test_child_diagnostics_compiler_timeout_still_records_builder_cleanup(self):
        step = self._child_diagnostic_failure("compiler-timeout")["steps"][-1]
        self.assertEqual((step["phase"], step["reason"], step["exit_code"]),
                         ("LIVE_COMPILE", "TIMEOUT", None))
        self.assertTrue(step["checks"]["builder_cleanup_confirmed"])

    def test_child_diagnostics_compiler_artifact_parser_failure_has_zero_exit(self):
        step = self._child_diagnostic_failure("artifact-invalid")["steps"][-1]
        self.assertEqual((step["phase"], step["reason"], step["exit_code"]),
                         ("LIVE_COMPILE", "ARTIFACT_INVALID", 0))

    def test_child_diagnostics_keep_successful_compile_when_later_execution_fails(self):
        steps = self._child_diagnostic_failure("compile-then-exit")["steps"]
        self.assertEqual([(step["phase"], step["reason"], step["exit_code"]) for step in steps],
                         [("LIVE_COMPILE", "COMPLETE", 0),
                          ("LIVE_EXECUTE", "EXIT_NONZERO", 101)])

    def test_child_diagnostics_exit_before_first_marker_is_execution_failure(self):
        diagnostics = self._child_diagnostic_failure("before-first")
        step = diagnostics["steps"][-1]
        self.assertEqual((step["phase"], step["reason"], step["exit_code"]),
                         ("LIVE_EXECUTE", "EXIT_NONZERO", 101))
        self.assertEqual(step["checks"]["first_attestation_count"], 0)
        self.assertEqual(step["checks"]["child_reason"], "UNKNOWN")

    def test_child_diagnostics_exit_after_first_marker_retains_fixed_rejection_observations(self):
        step = self._child_diagnostic_failure("after-first")["steps"][-1]
        self.assertEqual((step["phase"], step["reason"], step["exit_code"]),
                         ("LIVE_EXECUTE", "EXIT_NONZERO", 101))
        self.assertEqual(step["checks"]["first_attestation_count"], 1)
        self.assertEqual(step["checks"]["restart_rejection_count"], 0)
        self.assertEqual(step["checks"]["child_reason"], "Session")
        self.assertEqual(step["checks"]["child_isolation"], "UNCONFIRMED_UNUSABLE")
        self.assertEqual(step["checks"]["checkpoint_counts"]["RESTART_COMPLETED"], 1)

    def test_child_diagnostics_exit_zero_parser_failure_stays_failed(self):
        step = self._child_diagnostic_failure("parser")["steps"][-1]
        self.assertEqual((step["phase"], step["reason"], step["exit_code"]),
                         ("LIVE_OUTPUT_VALIDATE", "OUTPUT_INVALID", 0))
        self.assertEqual(step["checks"]["first_attestation_count"], 1)
        self.assertEqual(step["checks"]["passing_one_test_count"], 0)

    def test_child_diagnostics_timeout_survives_capture_cleanup_and_stop(self):
        step = self._child_diagnostic_failure("timeout")["steps"][-1]
        self.assertEqual((step["phase"], step["reason"], step["exit_code"]),
                         ("LIVE_EXECUTE", "TIMEOUT", None))
        self.assertTrue(step["checks"]["stdout_present"])

    def test_child_diagnostics_output_limit_retains_known_exit_without_raw_output(self):
        step = self._child_diagnostic_failure("output-limit")["steps"][-1]
        self.assertEqual((step["phase"], step["reason"], step["exit_code"]),
                         ("LIVE_EXECUTE", "STDOUT_LIMIT", 101))

    def test_child_diagnostics_noise_remains_unknown_when_exact_stop_unconfirmed(self):
        step = self._child_diagnostic_failure("noise", stop_confirmed=False)["steps"][-1]
        self.assertEqual(step["checks"]["child_reason"], "UNKNOWN")
        self.assertEqual(step["checks"]["first_attestation_count"], 0)
        self.assertTrue(all(count == 0 for count in step["checks"]["checkpoint_counts"].values()))

    def test_child_diagnostics_marker_counts_saturate_and_do_not_relax_parser(self):
        step = self._child_diagnostic_failure("duplicate")["steps"][-1]
        self.assertEqual(step["reason"], "OUTPUT_INVALID")
        self.assertEqual(step["checks"]["first_attestation_count"], 2)

    def test_child_diagnostics_only_enumerated_reason_checkpoint_and_isolation_lines_survive(self):
        name = "restore_preflight::target_binding::tests::live_read_only_child_restart_rejection"
        for phase in ("FIRST", "RESTART", "REJECTION"):
            for reason in ("Session", "Identity", "Protocol", "Version", "Deadline",
                           "StdoutLimit", "StderrLimit", "Exit", "Stderr", "Io", "Unusable"):
                with self.subTest(phase=phase, reason=reason):
                    observations = runner._child_observations(
                        ("test " + name + " ... CHILD_DIAG_" + phase +
                         "_FAILURE_" + reason + "\n").encode(), name)
                    self.assertEqual(observations["child_reason"], reason)
                    self.assertEqual(observations["child_failure_phase"], phase)
        checkpoints = ("FIRST_ATTESTATION", "RESTART_BEGIN", "RESTART_COMPLETED",
                       "REJECTION_BEGIN", "REJECTION_OBSERVED", "REASON_ACCEPTED",
                       "ISOLATION_STOPPED", "GUARD_REUSE_REJECTED", "FINAL_SNAPSHOT",
                       "FINAL_ASSERTIONS_PASSED")
        output = ("\n".join("CHILD_DIAG_CHECKPOINT_" + code for code in checkpoints) +
                  "\nCHILD_DIAG_REJECTION_ISOLATION_STOPPED\n").encode()
        observations = runner._child_observations(output, name)
        self.assertEqual(observations["checkpoint_counts"], dict.fromkeys(checkpoints, 1))
        self.assertEqual(observations["child_isolation"], "STOPPED")
        observations = runner._child_observations(
            b"CHILD_DIAG_FIRST_FAILURE_Session\nCHILD_DIAG_FIRST_FAILURE_Identity\n"
            b"CHILD_DIAG_FIRST_ISOLATION_STOPPED\n"
            b"CHILD_DIAG_FIRST_ISOLATION_UNCONFIRMED_UNUSABLE\n", name)
        self.assertEqual(observations["child_reason"], "UNKNOWN")
        self.assertEqual(observations["child_isolation"], "UNKNOWN")

    def test_child_diagnostics_stderr_limits_and_io_failures_store_only_fixed_process_facts(self):
        for stdout_bytes, stderr_bytes, want in ((b"", b"xxxx", "STDERR_LIMIT"),
                                               (b"xxxx", b"xxxx", "OUTPUT_LIMIT")):
            with self.subTest(reason=want):
                step = {"phase": "LIVE_EXECUTE", "checks": {}}
                def process(_, *, stdout, stderr, **kwargs):
                    stdout.write(stdout_bytes)
                    stderr.write(stderr_bytes)
                    return SimpleNamespace(returncode=-9)
                with patch.object(runner.subprocess, "run", side_effect=process):
                    with self.assertRaises(ValueError):
                        runner._run_bounded(["unused"], limit=3, diagnostic=step)
                self.assertEqual((step["reason"], step["exit_code"]), (want, -9))
        step = {"phase": "LIVE_EXECUTE", "checks": {}}
        with patch.object(runner.subprocess, "run", side_effect=OSError("untrusted noise")):
            with self.assertRaises(OSError):
                runner._run_bounded(["unused"], diagnostic=step)
        self.assertEqual((step["reason"], step["exit_code"]), ("IO", None))

    def test_child_diagnostics_reject_arbitrary_phase_reason_status_and_exception_names(self):
        diagnostics = {"format_version": 1, "steps": []}
        with self.assertRaises(ValueError):
            runner._child_step(diagnostics, "untrusted noise")
        self.assertEqual(diagnostics["steps"], [])
        step = runner._child_step(diagnostics, "LIVE_COMPILE")
        for reason, exit_code in (("untrusted noise", 0), ("COMPLETE", True),
                                  ("COMPLETE", "untrusted noise")):
            with self.assertRaises(ValueError):
                runner._child_process_fact(step, reason, exit_code)
        self.assertEqual((step["reason"], step["exit_code"]), ("UNKNOWN", None))
        arbitrary = type("untrusted_class_name", (ValueError,), {})
        self.assertEqual(runner._child_exception_type(arbitrary("untrusted noise")), "UNKNOWN")

    def test_child_diagnostics_durable_failure_omits_arbitrary_exception_attributes(self):
        self.args.child_read_only_restart = True
        sentinel = "untrusted-" + "sensitive-sentinel"
        arbitrary = type("untrusted_class_name", (ValueError,), {})
        error = arbitrary(sentinel)
        error.clone_helper_cleanup = {"reason": sentinel}
        captured = io.StringIO()
        with patch.object(runner, "_preflight_probe_builder", return_value={}), \
             patch.object(runner, "_run_bound_probe", side_effect=error), \
             contextlib.redirect_stdout(captured):
            code, result = self._run()
        payload = (self.batch / "evidence" / "result.json").read_text()
        self.assertEqual(code, 1)
        self.assertEqual(result["failure_type"], "UNKNOWN")
        self.assertTrue(sentinel not in payload and sentinel not in captured.getvalue(),
                        "exception attributes cannot become child diagnostics")
        self.assertNotIn("clone_helper_cleanup", result)

    def test_child_diagnostics_checkpoints_cannot_replace_acceptance_markers(self):
        binary = self.batch / "test-binary"
        binary.write_bytes(b"binary")
        diagnostics = {"format_version": 1, "steps": []}
        checkpoints = (b"CHILD_DIAG_CHECKPOINT_FIRST_ATTESTATION\n"
                       b"CHILD_DIAG_CHECKPOINT_RESTART_COMPLETED\n"
                       b"CHILD_DIAG_CHECKPOINT_FINAL_ASSERTIONS_PASSED\n")
        with patch.object(runner, "_compile_bound_probe", return_value=(binary, "f" * 64)), \
             patch.object(runner, "_file_digest", return_value="f" * 64), \
             patch.object(runner, "_run_bounded", return_value=SimpleNamespace(
                 returncode=0, stdout=checkpoints, stderr=b"")):
            with self.assertRaises(ValueError):
                runner._run_bound_probe(self.batch / "source", self.batch,
                    self.batch / "control" / "targets" / ID, "new-database", "d" * 64,
                    child=True, diagnostics=diagnostics)
        step = diagnostics["steps"][-1]
        self.assertEqual(step["reason"], "OUTPUT_INVALID")
        self.assertEqual(step["checks"]["first_attestation_count"], 0)
        self.assertEqual(step["checks"]["checkpoint_counts"]["FINAL_ASSERTIONS_PASSED"], 1)

    def test_clean_candidate_only_after_exact_stop(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code, result = self._run()
        self.assertEqual(code, 0, result)
        self.assertEqual(result["status"], runner.PASSED)
        self.assertNotIn("clone_phase", result)
        self.assertEqual(result["birth_sha256"], "d" * 64)
        record = (self.batch / "evidence" / "pin-inspection.json").read_bytes()
        self.assertEqual(result["inspection_evidence_sha256"],
                         hashlib.sha256(record).hexdigest())
        self.assertEqual(result["inspection_record_sha256"],
                         hashlib.sha256(record).hexdigest())
        self.assertEqual(json.loads(record)["live"]["container_id"], "e" * 64)
        self.assertTrue(result["stop"]["confirmed"])
        final = self.batch / "evidence" / "result.json"
        self.assertEqual(json.loads(final.read_bytes())["status"], runner.PASSED)
        self.assertFalse((self.batch / "evidence" / "result.pending.json").exists())
        summary = json.loads(output.getvalue())
        self.assertEqual(summary["result_sha256"],
                         hashlib.sha256(final.read_bytes()).hexdigest())
        self.assertEqual(summary["inspection_evidence_sha256"],
                         hashlib.sha256(record).hexdigest())

    def test_opt_in_bound_probe_runs_before_stop_and_has_distinct_status(self):
        self.args.bound_probe = True
        sequence = []
        deps = self._dependencies()
        issuer = deps[1]._run_issuer
        deps[1]._run_issuer = lambda *args: (sequence.append("issuer") or
                                             issuer(*args))
        deps[1].stop_verified_pg = lambda *_: (sequence.append("stop") or
            {"confirmed": True, "volume_retained": True})
        with patch.object(runner, "_preflight_probe_builder",
                          side_effect=lambda *_: sequence.append("preflight") or
                          {"builder_image_id": runner.BUILDER_IMAGE_ID}), \
             patch.object(runner, "_run_bound_probe",
                          side_effect=lambda *_: sequence.append("probe") or
                          {"state": "BOUND_TARGET_READ_ONLY_PG18_PASSED_NOT_RESTORE"}):
            code, result = self._run(deps)
        self.assertEqual(code, 0, result)
        self.assertEqual(sequence, ["preflight", "issuer", "probe", "stop"])
        self.assertEqual(result["status"], runner.BOUND_PASSED)
        self.assertTrue(result["stop"]["confirmed"])

    def test_guard_mode_selects_guard_before_exact_stop(self):
        self.args.bound_guard = True
        sequence = []
        deps = self._dependencies()
        issuer = deps[1]._run_issuer
        deps[1]._run_issuer = lambda *args: (sequence.append("issuer") or issuer(*args))
        deps[1].stop_verified_pg = lambda *_: (sequence.append("stop") or
            {"confirmed": True, "volume_retained": True})
        def preflight(*args, **kwargs):
            self.assertEqual(kwargs, {"guard": True})
            sequence.append("preflight")
            return {"builder_image_id": runner.BUILDER_IMAGE_ID}
        def guard(*args, **kwargs):
            self.assertEqual(kwargs, {"guard": True})
            sequence.append("guard")
            return {"state": "BOUND_TARGET_GUARD_READ_ONLY_PG18_PASSED_NOT_RESTORE"}
        with patch.object(runner, "_preflight_probe_builder", side_effect=preflight), \
             patch.object(runner, "_run_bound_probe", side_effect=guard):
            code, result = self._run(deps)
        self.assertEqual(sequence, ["preflight", "issuer", "guard", "stop"])
        self.assertEqual(code, 0, result)
        self.assertEqual(result["status"],
            "BOUND_TARGET_GUARD_READ_ONLY_SINGLE_HOST_PG18_PASSED_QUARANTINED_NOT_RESTORE")
        self.assertIn("bound_guard", result)
        self.assertNotIn("bound_probe", result)

    def test_sql_session_mode_selects_one_test_and_distinct_read_only_status(self):
        self.args.sql_session_binding = True
        sequence = []
        deps = self._dependencies()
        issuer = deps[1]._run_issuer
        deps[1]._run_issuer = lambda *args: (sequence.append("issuer") or issuer(*args))
        deps[1].stop_verified_pg = lambda *_: (sequence.append("stop") or
            {"confirmed": True, "volume_retained": True})
        def preflight(*args, **kwargs):
            self.assertEqual(kwargs, {"session": True})
            sequence.append("preflight")
            return {"builder_image_id": runner.BUILDER_IMAGE_ID}
        def session(*args, **kwargs):
            self.assertEqual(kwargs, {"session": True})
            sequence.append("session")
            return {"state": "SQL_SESSION_BINDING_READ_ONLY_PG18_PASSED_NOT_RESTORE"}
        with patch.object(runner, "_preflight_probe_builder", side_effect=preflight), \
             patch.object(runner, "_run_bound_probe", side_effect=session):
            code, result = self._run(deps)
        self.assertEqual((code, sequence),
                         (0, ["preflight", "issuer", "session", "stop"]))
        self.assertEqual(result["status"], runner.SESSION_PASSED)
        self.assertIn("sql_session_binding", result)
        self.assertTrue(result["status"].endswith("NOT_RESTORE"))

    def test_child_restart_mode_preflights_then_persists_isolation_and_reuse_rejection(self):
        self.args.child_read_only_restart = True
        sequence = []
        deps = self._dependencies()
        issuer = deps[1]._run_issuer
        deps[1]._run_issuer = lambda *args: (sequence.append("issuer") or issuer(*args))
        deps[1].stop_verified_pg = lambda *_: (sequence.append("stop") or
            {"confirmed": True, "volume_retained": True})
        def preflight(*args, **kwargs):
            self.assertTrue(kwargs["child"])
            self.assertEqual(kwargs["diagnostics"], {"format_version": 1, "steps": []})
            sequence.append("preflight")
            return {"host_test_listing_confirmed": True}
        def probe(*args, **kwargs):
            self.assertTrue(kwargs["child"])
            self.assertEqual(kwargs["diagnostics"], {"format_version": 1, "steps": []})
            sequence.append("probe")
            return {"state": runner.CHILD_PASSED, "first_attestation":
                    "CHILD_READ_ONLY_ATTESTED_NOT_RESTORE", "restart_rejection":
                    "CHILD_SAME_GUARD_RESTART_REJECTED_READ_ONLY_NOT_RESTORE",
                    "reason": "Session", "isolation": "STOPPED", "guard_reuse_rejected": True,
                    "exit_code": 0, "birth_sha256": "d" * 64}
        with patch.object(runner, "_preflight_probe_builder", side_effect=preflight), \
             patch.object(runner, "_run_bound_probe", side_effect=probe):
            code, result = self._run(deps)
        self.assertEqual((code, sequence),
                         (0, ["preflight", "issuer", "probe", "stop"]))
        self.assertEqual(result["status"], runner.CHILD_PASSED)
        self.assertEqual(result["child_read_only"]["isolation"], "STOPPED")
        self.assertFalse(result["target_reuse_permitted"])
        self.assertEqual(json.loads((self.batch / "evidence" / "result.json").read_bytes())
                         ["child_read_only"]["guard_reuse_rejected"], True)

    def test_child_restart_failure_is_durable_and_never_reuses_batch(self):
        self.args.child_read_only_restart = True
        deps = self._dependencies()
        with patch.object(runner, "_preflight_probe_builder", return_value={}), \
             patch.object(runner, "_run_bound_probe", side_effect=ValueError("bad child")):
            code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.CHILD_FAILED)
        self.assertEqual(result["child_isolation"], "STOPPED")
        self.assertFalse(result["target_reuse_permitted"])
        self.assertEqual(json.loads((self.batch / "evidence" / "result.json").read_bytes())
                         ["status"], runner.CHILD_FAILED)

    def test_child_restart_parser_requires_both_markers_reason_and_one_test(self):
        name = "restore_preflight::target_binding::tests::live_read_only_child_restart_rejection"
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "testbin"
            binary.write_bytes(b"binary")
            listing = (name + ": test\n").encode()
            passed = ("running 1 test\nCHILD_READ_ONLY_ATTESTED_NOT_RESTORE\n"
                      "CHILD_RESTART_FAILURE_Session\n"
                      "CHILD_RESTART_ISOLATION_STOPPED_GUARD_REUSE_REJECTED\n"
                      "CHILD_SAME_GUARD_RESTART_REJECTED_READ_ONLY_NOT_RESTORE\n"
                      "test " + name + " ... ok\n"
                      "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;\n").encode()
            with patch.object(runner, "_compile_bound_probe", return_value=(binary, "f" * 64)), \
                 patch.object(runner, "_file_digest", return_value="f" * 64), \
                 patch.object(runner, "_run_bounded", return_value=SimpleNamespace(
                     returncode=0, stdout=passed, stderr=b"")) as execute:
                found = runner._run_bound_probe(self.batch / "source", self.batch,
                    self.batch / "control" / "targets" / ID, "new-database", "d" * 64,
                    child=True)
                self.assertEqual(found["reason"], "Session")
                self.assertEqual(execute.call_args.args[0][1:4],
                                 [name, "--exact", "--ignored"])
                for damaged in (passed.replace(b"CHILD_READ_ONLY_ATTESTED_NOT_RESTORE", b""),
                                passed.replace(b"CHILD_RESTART_FAILURE_Session", b""),
                                passed.replace(b"CHILD_SAME_GUARD_RESTART_REJECTED_READ_ONLY_NOT_RESTORE",
                                               b"NOISE_CHILD_SAME_GUARD_RESTART_REJECTED_READ_ONLY_NOT_RESTORE"),
                                passed.replace(b"1 passed", b"0 passed")):
                    execute.return_value = SimpleNamespace(returncode=0,
                                                            stdout=damaged, stderr=b"")
                    with self.assertRaises(ValueError):
                        runner._run_bound_probe(self.batch / "source", self.batch,
                            self.batch / "control" / "targets" / ID,
                            "new-database", "d" * 64, child=True)

    def test_child_preflight_requires_exact_listed_test_before_target_birth(self):
        binary = self.batch / "test-binary"
        binary.write_bytes(b"binary")
        name = b"restore_preflight::target_binding::tests::live_read_only_child_restart_rejection: test\n"
        with patch.object(runner, "_trusted_path"), \
             patch.object(runner, "_compile_bound_probe",
                          return_value=(binary, runner._file_digest(binary))), \
             patch.object(runner, "_probe_docker", return_value=SimpleNamespace(
                 returncode=0, stdout=(runner.BUILDER_IMAGE_ID + "\n").encode())), \
             patch.object(runner, "_run_bounded", return_value=SimpleNamespace(
                 returncode=0, stdout=name, stderr=b"")) as execute:
            self.assertTrue(runner._preflight_probe_builder(
                self.batch / "source", self.batch,
                child=True)["host_test_listing_confirmed"])
            execute.return_value = SimpleNamespace(returncode=0, stdout=b"", stderr=b"")
            with self.assertRaises(ValueError):
                runner._preflight_probe_builder(self.batch / "source", self.batch,
                                                child=True)

    def test_failed_sql_session_stops_only_exact_target_and_cannot_pass(self):
        self.args.sql_session_binding = True
        deps = self._dependencies()
        stopped = []
        deps[1].stop_verified_pg = lambda *args: (stopped.append(args[-1]) or
            {"confirmed": True, "volume_retained": True})
        with patch.object(runner, "_preflight_probe_builder", return_value={}), \
             patch.object(runner, "_run_bound_probe", side_effect=ValueError("session failed")):
            code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.SESSION_FAILED)
        self.assertEqual(stopped, ["e" * 64])

    def test_clone_mode_admits_second_project_before_primary_birth(self):
        self.args.sql_session_clone_negative = True
        self.args.clone_batch_id = "b27f4d57-1165-4b17-92c1-4ddf9a178eaa"
        self.args.clone_subnet = "10.251.229.0/24"
        deps = self._dependencies()
        clone = {"project": "clone-project", "database": "clone-database",
                 "volume": "clone-volume", "network": "clone-network",
                 "image": "postgres:18"}
        original = deps[0].identity_for
        deps[0].identity_for = lambda batch_id: (
            clone if batch_id == self.args.clone_batch_id else original(batch_id))
        admitted = []
        deps[0].admit_fresh = lambda identity, subnet, before: admitted.append(
            (identity["project"], subnet))
        deps[1]._run_issuer = lambda *_: self.fail("birth before clone admission")
        with patch.object(runner, "_preflight_probe_builder", return_value={}), \
             patch.object(runner, "_admit_clone_pair", side_effect=ValueError(
                 "second project occupied")):
            code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.CLONE_FAILED)
        self.assertEqual(result["target_condition"], "NO_TARGET_CREATED")
        self.assertEqual(result["clone_project"], "clone-project")

    def test_clone_negative_failure_stops_both_exact_ids(self):
        self.args.sql_session_clone_negative = True
        self.args.clone_batch_id = "b27f4d57-1165-4b17-92c1-4ddf9a178eaa"
        self.args.clone_subnet = "10.251.229.0/24"
        deps = self._dependencies()
        clone = {"project": "clone-project", "database": "clone-database",
                 "volume": "clone-volume", "network": "clone-network",
                 "image": "postgres:18"}
        original = deps[0].identity_for
        deps[0].identity_for = lambda batch_id: (
            clone if batch_id == self.args.clone_batch_id else original(batch_id))
        stopped = []
        deps[1].stop_verified_pg = lambda _provisioner, identity, container_id: (
            stopped.append((identity["project"], container_id)) or
            {"confirmed": True, "volume_retained": True})
        snapshot = deps[0].snapshot()
        seen = []
        def snapshots():
            seen.append(1)
            if len(seen) == 1:
                return snapshot
            return dict(snapshot, containers=[{
                "Id": "e" * 64,
                "State": {"Running": True,
                          "StartedAt": "2026-09-29T00:00:00Z"}}])
        deps[0].snapshot = snapshots
        with patch.object(runner, "_preflight_probe_builder", return_value={}), \
             patch.object(runner, "_admit_clone_pair", return_value=True), \
             patch.object(runner, "_primary_still_pinned", return_value=True), \
             patch.object(runner, "_prepare_physical_clone", return_value={
                 "container_id": "f" * 64, "backup_verified": True,
                 "no_standby": True, "volume_retained": True}):
            code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.CLONE_FAILED)
        self.assertEqual(stopped, [("clone-project", "f" * 64),
                                   ("new-project", "e" * 64)])
        self.assertEqual(result["clone"]["container_id"], "f" * 64)
        self.assertTrue(result["stop"]["confirmed"])

    def test_clone_failure_publishes_only_fixed_last_phase(self):
        self.args.sql_session_clone_negative = True
        self.args.clone_batch_id = "b27f4d57-1165-4b17-92c1-4ddf9a178eaa"
        self.args.clone_subnet = "10.251.229.0/24"
        deps = self._dependencies()
        clone = {"project": "clone-project", "database": "clone-database",
                 "volume": "clone-volume", "network": "clone-network",
                 "image": "postgres:18"}
        original = deps[0].identity_for
        deps[0].identity_for = lambda batch_id: (
            clone if batch_id == self.args.clone_batch_id else original(batch_id))
        def fail_copy(*args, **kwargs):
            kwargs["phase"]("basebackup-copy")
            raise ValueError("sensitive DSN must not be recorded")
        with patch.object(runner, "_preflight_probe_builder", return_value={}), \
             patch.object(runner, "_admit_clone_pair", return_value=True), \
             patch.object(runner, "_primary_still_pinned", return_value=True), \
             patch.object(runner, "_prepare_physical_clone", side_effect=fail_copy):
            code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.CLONE_FAILED)
        self.assertEqual(result["clone_phase"], "basebackup-copy")
        payload = (self.batch / "evidence" / "result.json").read_text()
        self.assertEqual(json.loads(payload)["clone_phase"], "basebackup-copy")
        self.assertNotIn("sensitive DSN", payload)

    def test_clone_probe_rechecks_birth_started_primary_after_probe_before_dual_stop(self):
        self.args.sql_session_clone_negative = True
        self.args.clone_batch_id = "b27f4d57-1165-4b17-92c1-4ddf9a178eaa"
        self.args.clone_subnet = "10.251.229.0/24"
        deps = self._dependencies()
        clone = {"project": "clone-project", "database": "clone-database",
                 "volume": "clone-volume", "network": "clone-network",
                 "image": "postgres:18"}
        original = deps[0].identity_for
        deps[0].identity_for = lambda batch_id: (
            clone if batch_id == self.args.clone_batch_id else original(batch_id))
        sequence = []
        def recheck(*args):
            self.assertEqual(args[-1], "2026-09-29T00:00:00Z")
            sequence.append("recheck")
            return True
        def probe(*args):
            self.assertEqual(args[-3:], ("f" * 64, "a" * 64, "10.251.229.0/24"))
            sequence.append("probe")
            return {"state": runner.CLONE_PASSED}
        with patch.object(runner, "_preflight_probe_builder", return_value={}), \
             patch.object(runner, "_admit_clone_pair", return_value=True), \
             patch.object(runner, "_primary_still_pinned", side_effect=recheck), \
             patch.object(runner, "_prepare_physical_clone", return_value={
                 "container_id": "f" * 64, "network_id": "a" * 64,
                 "backup_verified": True, "no_standby": True,
                 "volume_retained": True}), \
             patch.object(runner, "_run_clone_negative_probe", side_effect=probe), \
             patch.object(runner, "_stop_clone_pair", side_effect=lambda *_:
                          sequence.append("stop") or
                          {"confirmed": True, "volume_retained": True}), \
             patch.object(runner, "_clone_quarantine_evidence", return_value={
                 "primary_stopped": True, "clone_stopped": True,
                 "primary_volume_retained": True, "clone_volume_retained": True}):
            code, result = self._run(deps)
        self.assertEqual(code, 0, result)
        self.assertEqual(result["status"], runner.CLONE_PASSED)
        self.assertEqual(result["clone_phase"], "complete")
        self.assertEqual(sequence[-3:], ["probe", "recheck", "stop"])

    def test_clone_rejects_restart_before_first_runner_docker_observation(self):
        self.args.sql_session_clone_negative = True
        self.args.clone_batch_id = "b27f4d57-1165-4b17-92c1-4ddf9a178eaa"
        self.args.clone_subnet = "10.251.229.0/24"
        deps = self._dependencies()
        clone = {"project": "clone-project", "database": "clone-database",
                 "volume": "clone-volume", "network": "clone-network",
                 "image": "postgres:18"}
        original = deps[0].identity_for
        deps[0].identity_for = lambda batch_id: (
            clone if batch_id == self.args.clone_batch_id else original(batch_id))
        changed = [False]
        snapshot = deps[0].snapshot()
        def snapshots():
            if not changed[0] and not snapshots.initial:
                snapshots.initial = True
                return snapshot
            return dict(snapshot, containers=[{
                "Id": "e" * 64,
                "State": {"Running": True, "StartedAt": (
                    "2026-09-29T00:01:00Z" if changed[0] else
                    "2026-09-29T00:00:00Z")}}])
        snapshots.initial = False
        deps[0].snapshot = snapshots
        issue = deps[1]._run_issuer
        def restart_after_issuer(*args):
            changed[0] = True
            return issue(*args)
        deps[1]._run_issuer = restart_after_issuer
        with patch.object(runner, "_preflight_probe_builder", return_value={}), \
             patch.object(runner, "_admit_clone_pair", return_value=True), \
             patch.object(runner, "_prepare_physical_clone",
                          side_effect=ValueError("clone should not begin")) as prepare_clone:
            code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.CLONE_FAILED)
        prepare_clone.assert_not_called()

    def test_sql_session_requires_exact_listing_and_one_marker_without_secret_env(self):
        binary = self.batch / "test-binary"
        binary.write_bytes(b"binary")
        name = "restore_preflight::target_binding::tests::live_read_only_sql_session_binding"
        marker = "SQL_SESSION_BINDING_READ_ONLY_PG18_PASSED_NOT_RESTORE"
        listing = (name + ": test\n").encode()
        with patch.object(runner, "_trusted_path"), \
             patch.object(runner, "_compile_bound_probe",
                          return_value=(binary, runner._file_digest(binary))), \
             patch.object(runner, "_probe_docker", return_value=SimpleNamespace(
                 returncode=0, stdout=(runner.BUILDER_IMAGE_ID + "\n").encode())), \
             patch.object(runner, "_run_bounded", return_value=SimpleNamespace(
                 returncode=0, stdout=listing, stderr=b"")):
            self.assertTrue(runner._preflight_probe_builder(
                self.batch / "source", self.batch,
                session=True)["host_test_listing_confirmed"])
        with patch.object(runner, "_trusted_path"), \
             patch.object(runner, "_compile_bound_probe",
                          return_value=(binary, runner._file_digest(binary))), \
             patch.object(runner, "_probe_docker", return_value=SimpleNamespace(
                 returncode=0, stdout=(runner.BUILDER_IMAGE_ID + "\n").encode())), \
             patch.object(runner, "_run_bounded", return_value=SimpleNamespace(
                 returncode=0, stdout=listing.replace(b"sql_session_binding", b"bound_target_guard"),
                 stderr=b"")):
            with self.assertRaises(ValueError):
                runner._preflight_probe_builder(self.batch / "source", self.batch,
                                                 session=True)
        good = ("running 1 test\n" + marker + "\ntest " + name +
                " ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;\n").encode()
        for output, accepted in [(good, True), (good + marker.encode(), False),
                                 (good.replace(marker.encode(), b"BOUND_TARGET_READ_ONLY_PG18_PASSED_NOT_RESTORE"), False)]:
            def execute(command, **kwargs):
                self.assertEqual(command[1:], [name, "--exact", "--ignored", "--nocapture"])
                self.assertEqual(set(kwargs["env"]), {
                    "HOME", "PATH", "KNOWWEAVE_C4_PROBE_DESTINATION_ROOT",
                    "KNOWWEAVE_C4_PROBE_CONTROL_ROOT",
                    "KNOWWEAVE_C4_PROBE_ASSET_ROOT",
                    "KNOWWEAVE_C4_PROBE_EXPECTED_DATABASE"})
                return SimpleNamespace(returncode=0, stdout=output, stderr=b"")
            with patch.object(runner, "_compile_bound_probe",
                              return_value=(binary, runner._file_digest(binary))), \
                 patch.object(runner, "_run_bounded", side_effect=execute):
                invoke = lambda: runner._run_bound_probe(
                    self.batch / "source", self.batch,
                    self.batch / "control" / "targets" / ID,
                    "learning_restore_c4_" + ID, "d" * 64, session=True)
                if accepted:
                    self.assertEqual(invoke()["state"], marker)
                else:
                    with self.assertRaises(ValueError):
                        invoke()

    def test_failed_guard_preflight_never_creates_pg_and_guard_failure_still_stops(self):
        self.args.bound_guard = True
        deps = self._dependencies()
        deps[1]._run_issuer = lambda *_: self.fail("issuer reached")
        with patch.object(runner, "_preflight_probe_builder", side_effect=ValueError("unavailable")):
            code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.GUARD_FAILED)
        self.assertEqual(result["target_condition"], "NO_TARGET_CREATED")
        for name in ("attempt.json", "result.json"):
            (self.batch / "evidence" / name).unlink()
        deps = self._dependencies()
        stopped = []
        deps[1].stop_verified_pg = lambda *args: (stopped.append(args[-1]) or
            {"confirmed": True, "volume_retained": True})
        with patch.object(runner, "_preflight_probe_builder", return_value={}), \
             patch.object(runner, "_run_bound_probe", side_effect=ValueError("guard failed")):
            code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.GUARD_FAILED)
        self.assertEqual(stopped, ["e" * 64])
        self.assertTrue(result["stop"]["volume_retained"])

    def test_guard_requires_exact_single_guard_test_and_unchanged_binary(self):
        binary = self.batch / "test-binary"
        binary.write_bytes(b"binary")
        test_name = "restore_preflight::target_binding::tests::live_read_only_bound_target_guard"
        marker = "BOUND_TARGET_GUARD_READ_ONLY_PG18_PASSED_NOT_RESTORE"
        good = ("running 1 test\n" + marker + "\ntest " + test_name +
                " ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;\n").encode()
        bad = [b"test result: ok. 0 passed; 0 failed; 0 ignored;",
               good.replace(marker.encode(), b"BOUND_TARGET_READ_ONLY_PG18_PASSED_NOT_RESTORE"),
               good.replace(test_name.encode(), b"another_test"),
               good + marker.encode(),
               good.replace(marker.encode(), (marker + "_WRONG").encode()),
               good.replace(marker.encode(), ("WRONG_" + marker).encode()),
               good.replace(b"running 1 test", b"running 2 tests"),
               good.replace(b"1 passed", b"2 passed")]
        for output, code, changed in [(good, 0, False), *((out, 0, False) for out in bad),
                                      (good, 1, False), (good, 0, True)]:
            with self.subTest(output=output, code=code, changed=changed):
                binary.write_bytes(b"binary")
                def execute(command, **kwargs):
                    self.assertEqual(command[1:], [test_name, "--exact", "--ignored", "--nocapture"])
                    self.assertEqual(kwargs["env"]["KNOWWEAVE_C4_PROBE_EXPECTED_DATABASE"], "learning_restore_c4_" + ID)
                    self.assertNotIn("PGPASSWORD", kwargs["env"])
                    if changed:
                        binary.write_bytes(b"changed")
                    return SimpleNamespace(returncode=code, stdout=output, stderr=b"")
                with patch.object(runner, "_compile_bound_probe", return_value=(binary, runner._file_digest(binary))), \
                     patch.object(runner, "_run_bounded", side_effect=execute):
                    args = (self.batch / "source", self.batch, self.batch / "control" / "targets" / ID,
                            "learning_restore_c4_" + ID, "d" * 64)
                    if output == good and code == 0 and not changed:
                        result = runner._run_bound_probe(*args, guard=True)
                        self.assertEqual(result["state"], marker)
                        self.assertEqual(result["birth_sha256"], "d" * 64)
                    else:
                        with self.assertRaises(ValueError):
                            runner._run_bound_probe(*args, guard=True)

    def test_guard_preflight_requires_guard_listing_not_probe_listing(self):
        binary = self.batch / "test-binary"
        binary.write_bytes(b"binary")
        for name, valid in [("live_read_only_bound_target_guard", True),
                            ("live_read_only_bound_target_probe", False),
                            ("live_read_only_bound_target_guard_extra", False)]:
            listing = ("restore_preflight::target_binding::tests::" + name + ": test\n").encode()
            with patch.object(runner, "_trusted_path"), \
                 patch.object(runner, "_compile_bound_probe", return_value=(binary, runner._file_digest(binary))), \
                 patch.object(runner, "_probe_docker", return_value=SimpleNamespace(returncode=0,
                     stdout=(runner.BUILDER_IMAGE_ID + "\n").encode())), \
                 patch.object(runner, "_run_bounded", return_value=SimpleNamespace(returncode=0,
                     stdout=listing, stderr=b"")):
                if valid:
                    self.assertTrue(runner._preflight_probe_builder(self.batch / "source", self.batch,
                        guard=True)["host_test_listing_confirmed"])
                else:
                    with self.assertRaises(ValueError):
                        runner._preflight_probe_builder(self.batch / "source", self.batch, guard=True)

    def test_failed_bound_probe_still_stops_and_cannot_pass(self):
        self.args.bound_probe = True
        sequence = []
        deps = self._dependencies()
        deps[1].stop_verified_pg = lambda *_: (sequence.append("stop") or
            {"confirmed": True, "volume_retained": True})
        def fail_probe(*_):
            sequence.append("probe")
            raise RuntimeError("probe failed")
        with patch.object(runner, "_preflight_probe_builder",
                          return_value={"builder_image_id": runner.BUILDER_IMAGE_ID}), \
             patch.object(runner, "_run_bound_probe",
                          side_effect=fail_probe):
            code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(sequence, ["probe", "stop"])
        self.assertEqual(result["status"], runner.BOUND_FAILED)
        self.assertTrue(result["stop"]["confirmed"])

    def test_failed_builder_preflight_never_issues_target(self):
        self.args.bound_probe = True
        deps = self._dependencies()
        deps[1]._run_issuer = lambda *_: self.fail("issuer reached")
        with patch.object(runner, "_preflight_probe_builder",
                          side_effect=ValueError("builder unavailable")):
            code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.BOUND_FAILED)
        self.assertEqual(result["target_condition"], "NO_TARGET_CREATED")

    def test_builder_artifact_requires_exact_test_executable(self):
        build = self.batch / "build"
        binary = build / "debug" / "deps" / ("learning_backup-" + "a" * 16)
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"binary")
        binary.chmod(0o555)
        artifact = {"reason": "compiler-artifact",
                    "manifest_path": "/reviewed/crates/learning-backup/Cargo.toml",
                    "target": {"name": "learning_backup", "kind": ["lib"]},
                    "profile": {"test": True},
                    "executable": "/target/debug/deps/" + binary.name}
        with patch.object(runner, "_trusted_path"), \
             patch.object(runner.stat, "S_IMODE", return_value=0o755):
            self.assertEqual(runner._builder_artifact(
                runner._json_bytes(artifact), build), binary)
            artifact["executable"] = "/target/debug/deps/other"
            with self.assertRaises(ValueError):
                runner._builder_artifact(runner._json_bytes(artifact), build)

    def test_preflight_requires_host_loader_before_target_birth(self):
        binary = self.batch / "test-binary"
        binary.write_bytes(b"binary")
        with patch.object(runner, "_trusted_path"), \
             patch.object(runner, "_compile_bound_probe",
                          return_value=(binary, runner._file_digest(binary))), \
             patch.object(runner, "_probe_docker", return_value=SimpleNamespace(
                 returncode=0,
                 stdout=(runner.BUILDER_IMAGE_ID + "\n").encode())), \
             patch.object(runner, "_run_bounded", return_value=SimpleNamespace(
                 returncode=1, stdout=b"", stderr=b"loader failed")):
            with self.assertRaises(ValueError):
                runner._preflight_probe_builder(self.batch / "source", self.batch)

    def test_preflight_lists_host_test_without_target_environment(self):
        binary = self.batch / "test-binary"
        binary.write_bytes(b"binary")
        listing = (b"restore_preflight::target_binding::tests::"
                   b"live_read_only_bound_target_probe: test\n")
        with patch.object(runner, "_trusted_path"), \
             patch.object(runner, "_compile_bound_probe",
                          return_value=(binary, runner._file_digest(binary))), \
             patch.object(runner, "_probe_docker", return_value=SimpleNamespace(
                 returncode=0,
                 stdout=(runner.BUILDER_IMAGE_ID + "\n").encode())), \
             patch.object(runner, "_run_bounded", return_value=SimpleNamespace(
                 returncode=0, stdout=listing,
                 stderr=b"")) as process:
            result = runner._preflight_probe_builder(
                self.batch / "source", self.batch)
        self.assertTrue(result["host_test_listing_confirmed"])
        self.assertEqual(process.call_args.args[0],
                         [str(binary), "--list"])
        self.assertNotIn("KNOWWEAVE_C4_PROBE_CONTROL_ROOT",
                         process.call_args.kwargs["env"])

    def test_builder_compile_is_pinned_offline_and_never_uses_host_cargo(self):
        binary = self.batch / "test-binary"
        binary.write_bytes(b"binary")
        fresh_batch = self.base / ID
        fresh_batch.mkdir()
        with patch.object(runner, "_private_dir"), \
             patch.object(runner, "_builder_artifact", return_value=binary), \
             patch.object(runner, "_builder_container_ids", return_value=[]), \
             patch.object(runner, "_cleanup_builder"), \
             patch.object(runner, "_run_bounded", return_value=SimpleNamespace(
                 returncode=0, stdout=b"{}\n", stderr=b"")) as process:
            runner._compile_bound_probe(self.batch / "source", fresh_batch,
                                        "probe-preflight-build", "0" * 64)
        command = process.call_args.args[0]
        self.assertEqual(command[0], "/usr/bin/docker")
        self.assertIn(runner.BUILDER_IMAGE_ID, command)
        self.assertIn("none", command)
        self.assertIn("ALL", command)
        self.assertIn("--name", command)
        self.assertIn("com.knowweave.bound-probe.batch=" + ID, command)
        self.assertIn("RUSTUP_AUTO_INSTALL=0", command)
        self.assertIn("KNOWWEAVE_C4_TARGET_BIRTH_SHA256=" + "0" * 64,
                      command)
        self.assertIn("--locked --offline", command[-1])
        self.assertNotIn("/root/.cargo/bin/cargo", command)

    def test_bound_probe_rejects_zero_test_binary_result(self):
        binary = self.batch / "test-binary"
        binary.write_bytes(b"binary")
        with patch.object(runner, "_compile_bound_probe",
                          return_value=(binary, runner._file_digest(binary))), \
             patch.object(runner, "_run_bounded", return_value=SimpleNamespace(
                 returncode=0, stdout=b"test result: ok. 0 passed; 0 failed; 0 ignored;",
                 stderr=b"")) as process:
            with self.assertRaises(ValueError):
                runner._run_bound_probe(self.batch / "source", self.batch,
                                        self.batch / "control" / "targets" / ID,
                                        "learning_restore_c4_" + ID, "d" * 64)
        self.assertIn("--exact", process.call_args.args[0])
        self.assertIn("--ignored", process.call_args.args[0])

    def test_interrupted_builder_is_removed_only_after_exact_identity(self):
        fresh_batch = self.base / ID
        fresh_batch.mkdir()
        container_id = "e" * 64
        name = f"knowweave-c4-bound-{ID}-preflight"
        facts = [{"Id": container_id, "Name": "/" + name,
                  "Image": runner.BUILDER_IMAGE_ID,
                  "Config": {"Labels": {
                      "com.knowweave.bound-probe.batch": ID}}}]
        with patch.object(runner, "_builder_container_ids",
                          side_effect=[[container_id], []]), \
             patch.object(runner, "_probe_docker", side_effect=[
                 SimpleNamespace(returncode=0,
                                 stdout=json.dumps(facts).encode()),
                 SimpleNamespace(returncode=0, stdout=b"")]) as docker:
            runner._cleanup_builder(fresh_batch, "preflight")
        self.assertEqual(docker.call_args_list[1].args[0],
                         ["container", "rm", "-f", container_id])
        facts[0]["Config"]["Labels"]["com.knowweave.bound-probe.batch"] = "foreign"
        with patch.object(runner, "_builder_container_ids",
                          return_value=[container_id]), \
             patch.object(runner, "_probe_docker", return_value=SimpleNamespace(
                 returncode=0, stdout=json.dumps(facts).encode())) as docker:
            with self.assertRaises(ValueError):
                runner._cleanup_builder(fresh_batch, "preflight")
        self.assertEqual(docker.call_count, 1)

    def test_captured_builder_cleanup_removes_only_expected_id(self):
        batch = self.base / ID
        captured = 'a' * 64
        name, _ = runner._builder_identity(batch, 'preflight')
        facts = [{'Id': captured, 'Name': '/' + name, 'Image': runner.BUILDER_IMAGE_ID,
                  'Config': {'Labels': {'com.knowweave.bound-probe.batch': ID}}}]
        with patch.object(runner, '_builder_container_ids', side_effect=[[captured], []]), \
             patch.object(runner, '_probe_docker', side_effect=[
                 SimpleNamespace(returncode=0, stdout=json.dumps(facts).encode()),
                 SimpleNamespace(returncode=0, stdout=b'')]) as docker:
            runner._cleanup_builder(batch, 'preflight', expected_id=captured)
        self.assertEqual([call.args[0] for call in docker.call_args_list],
            [['container', 'inspect', captured], ['container', 'rm', '-f', captured]])

    def test_captured_builder_absent_with_replacement_is_never_substituted(self):
        with patch.object(runner, '_builder_container_ids', return_value=['b' * 64]), \
             patch.object(runner, '_probe_docker') as docker:
            with self.assertRaisesRegex(ValueError, 'captured ID differs'):
                runner._cleanup_builder(self.base / ID, 'preflight', expected_id='a' * 64)
        docker.assert_not_called()

    def test_builder_auto_removed_is_confirmed_for_captured_and_legacy_callers(self):
        for expected in ({'expected_id': 'a' * 64}, {}):
            with self.subTest(expected=expected), \
                 patch.object(runner, '_builder_container_ids', return_value=[]), \
                 patch.object(runner, '_probe_docker') as docker:
                runner._cleanup_builder(self.base / ID, 'preflight', **expected)
            docker.assert_not_called()

    def test_captured_builder_inspection_replacement_never_removes(self):
        captured = 'a' * 64
        name, _ = runner._builder_identity(self.base / ID, 'preflight')
        facts = [{'Id': 'b' * 64, 'Name': '/' + name, 'Image': runner.BUILDER_IMAGE_ID,
                  'Config': {'Labels': {'com.knowweave.bound-probe.batch': ID}}}]
        with patch.object(runner, '_builder_container_ids', return_value=[captured]), \
             patch.object(runner, '_probe_docker', return_value=SimpleNamespace(
                 returncode=0, stdout=json.dumps(facts).encode())) as docker:
            with self.assertRaisesRegex(ValueError, 'cleanup identity differs'):
                runner._cleanup_builder(self.base / ID, 'preflight', expected_id=captured)
        self.assertEqual([call.args[0] for call in docker.call_args_list],
                         [['container', 'inspect', captured]])

    def test_captured_builder_rejects_invalid_expected_id_before_inventory(self):
        for invalid in ('builder', 'a' * 12, 'A' * 64, True):
            with self.subTest(invalid=invalid), patch.object(runner, '_builder_container_ids') as inventory:
                with self.assertRaisesRegex(ValueError, 'expected ID invalid'):
                    runner._cleanup_builder(self.base / ID, 'preflight', expected_id=invalid)
                inventory.assert_not_called()

    def test_command_output_is_bounded_before_read_into_memory(self):
        def write_large(_, *, stdout, stderr, **_kwargs):
            stdout.write(b"too long")
            return SimpleNamespace(returncode=0)
        with patch.object(runner.subprocess, "run", side_effect=write_large):
            with self.assertRaises(ValueError):
                runner._run_bounded(["dummy"], limit=3)

    def test_pin_mismatch_cannot_be_promoted(self):
        deps = self._dependencies()
        candidate, payload = deps[2].inspect_candidate()
        deps[2].inspect_candidate = lambda *_: (
            {**candidate, "birth_sha256": "0" * 64}, payload)
        code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.FAILED)
        self.assertNotIn("inspection_evidence_sha256", result)
        self.assertTrue(result["stop"]["confirmed"])

    def test_missing_or_corrupt_inspection_record_cannot_be_promoted(self):
        for mode in ("missing", "corrupt"):
            with self.subTest(mode=mode):
                (self.batch / "evidence" / "result.json").unlink(missing_ok=True)
                (self.batch / "evidence" / "attempt.json").unlink(missing_ok=True)
                (self.batch / "evidence" / "pin-inspection.json").unlink(missing_ok=True)
                deps = self._dependencies()
                def write(path, data):
                    if path.name == "pin-inspection.json":
                        if mode == "corrupt":
                            path.write_bytes(b"{}")
                    else:
                        path.write_bytes(data)
                with patch.object(runner, "extract_and_load", return_value=(
                         *deps, Path("/initdb"), "2" * 64)), \
                     patch.object(runner, "source_digest", return_value="2" * 64), \
                     patch.object(runner, "_file_digest", return_value="3" * 64), \
                     patch.object(runner, "_private_write", side_effect=write):
                    code, result = runner._run_batch(
                        self.args, {}, b"zip", self.batch)
                self.assertEqual(code, 1)
                self.assertEqual(result["status"], runner.FAILED)
                self.assertTrue(result["stop"]["confirmed"])
                self.assertNotIn("inspection_evidence_sha256", result)

    def test_stop_failure_cannot_be_promoted(self):
        deps = self._dependencies()
        def fail(*_):
            raise OSError("sensitive docker output")
        deps[1].stop_verified_pg = fail
        code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.FAILED)
        self.assertFalse(result["stop"]["confirmed"])
        self.assertNotIn("inspection_evidence_sha256", result)
        self.assertTrue((self.batch / "evidence" / "pin-inspection.json").exists())
        self.assertNotIn("sensitive", (self.batch / "evidence" / "result.json").read_text())

    def test_changed_source_cannot_be_promoted(self):
        deps = self._dependencies()
        with patch.object(runner, "extract_and_load", return_value=(
                 *deps, Path("/initdb"), "2" * 64)), \
             patch.object(runner, "source_digest", return_value="0" * 64), \
             patch.object(runner, "_file_digest", return_value="3" * 64), \
             patch.object(runner, "_private_write", side_effect=lambda path, data: path.write_bytes(data)):
            code, result = runner._run_batch(self.args, {}, b"zip", self.batch)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.FAILED)
        self.assertTrue(result["stop"]["confirmed"])
        self.assertNotIn("inspection_evidence_sha256", result)

    def test_pre_result_crash_leaves_observation_but_no_candidate(self):
        deps = self._dependencies()
        def write(path, data):
            if path.name == "result.pending.json":
                raise SystemExit(137)
            path.write_bytes(data)
        output = io.StringIO()
        with patch.object(runner, "extract_and_load", return_value=(
                 *deps, Path("/initdb"), "2" * 64)), \
             patch.object(runner, "source_digest", return_value="2" * 64), \
             patch.object(runner, "_file_digest", return_value="3" * 64), \
             patch.object(runner, "_private_write", side_effect=write), \
             contextlib.redirect_stdout(output):
            with self.assertRaises(SystemExit):
                runner._run_batch(self.args, {}, b"zip", self.batch)
        self.assertTrue((self.batch / "evidence" / "pin-inspection.json").exists())
        self.assertFalse((self.batch / "evidence" / "result.json").exists())
        self.assertNotIn(runner.PASSED, output.getvalue())

    def test_final_directory_sync_failure_cannot_print_candidate(self):
        deps = self._dependencies()
        output = io.StringIO()
        with patch.object(runner, "extract_and_load", return_value=(
                 *deps, Path("/initdb"), "2" * 64)), \
             patch.object(runner, "source_digest", return_value="2" * 64), \
             patch.object(runner, "_file_digest", return_value="3" * 64), \
             patch.object(runner, "_private_write",
                          side_effect=lambda path, data: path.write_bytes(data)), \
             patch.object(runner, "_sync_dir", side_effect=OSError("fsync failed")), \
             contextlib.redirect_stdout(output):
            with self.assertRaises(OSError):
                runner._run_batch(self.args, {}, b"zip", self.batch)
        self.assertTrue((self.batch / "evidence" / "pin-inspection.json").exists())
        self.assertTrue((self.batch / "evidence" / "result.pending.json").exists())
        self.assertNotIn(runner.PASSED, output.getvalue())

    def test_source_change_during_stop_cannot_be_promoted(self):
        deps = self._dependencies()
        with patch.object(runner, "extract_and_load", return_value=(
                 *deps, Path("/initdb"), "2" * 64)), \
             patch.object(runner, "source_digest", side_effect=["2" * 64,
                                                                "0" * 64,
                                                                "0" * 64]), \
             patch.object(runner, "_file_digest", return_value="3" * 64), \
             patch.object(runner, "_private_write", side_effect=lambda path, data: path.write_bytes(data)):
            code, result = runner._run_batch(self.args, {}, b"zip", self.batch)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.FAILED)
        self.assertTrue(result["stop"]["confirmed"])
        self.assertNotIn("inspection_evidence_sha256", result)

    def test_issuer_crash_keeps_failure_evidence_and_quarantine(self):
        deps = self._dependencies()
        def crash(*_):
            raise KeyboardInterrupt("sensitive")
        deps[1]._run_issuer = crash
        code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.FAILED)
        self.assertTrue(result["stop"]["confirmed"])
        self.assertTrue((self.batch / "evidence" / "attempt.json").exists())
        self.assertNotIn("sensitive", (self.batch / "evidence" / "result.json").read_text())

    def test_extraction_crash_still_leaves_one_attempt_record(self):
        with patch.object(runner, "extract_and_load", side_effect=KeyboardInterrupt), \
             patch.object(runner, "_file_digest", return_value="3" * 64), \
             patch.object(runner, "_private_write", side_effect=lambda path, data: path.write_bytes(data)):
            code, result = runner._run_batch(self.args, {}, b"zip", self.batch)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.FAILED)
        attempt = json.loads((self.batch / "evidence" / "attempt.json").read_bytes())
        self.assertEqual(attempt["batch_id"], ID)
        self.assertNotIn("inspection_evidence_sha256", result)

    def test_old_dirty_batch_and_replay_are_rejected_before_issuer(self):
        with patch.object(runner, "BASE", self.base), \
             patch.object(runner, "_require_private_dir"), \
             patch.object(runner, "_private_dir", side_effect=lambda p: p.mkdir()), \
             patch.object(runner, "_run_batch") as execute:
            root = self.base / "pin-acceptance"
            (root / "batches" / ID).mkdir(parents=True)
            (root / "batches" / ID / "dirty-result.json").write_text("{}")
            with self.assertRaises((FileExistsError, ValueError)):
                runner._prepare_batch(root, ID)
            execute.assert_not_called()


class ReviewedArchive(unittest.TestCase):
    def test_full_manifest_and_exact_installed_runner_hash_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            incoming = root / "incoming"
            incoming.mkdir()
            path = incoming / "reviewed.zip"
            files = {name: b"reviewed" for name in runner.REQUIRED}
            runner_sha = hashlib.sha256(Path(runner.__file__).read_bytes()).hexdigest()
            files[runner.ENTRY] = Path(runner.__file__).read_bytes()
            manifest = {"format_version": 1, "commit": "c" * 40,
                        "files": [{"path": name,
                                   "sha256": hashlib.sha256(data).hexdigest(),
                                   "size": len(data)} for name, data in sorted(files.items())]}
            manifest_bytes = runner._json_bytes(manifest)
            with zipfile.ZipFile(path, "w") as archive:
                for name, data in files.items():
                    info = zipfile.ZipInfo(name)
                    info.external_attr = 0o100444 << 16
                    archive.writestr(info, data)
                info = zipfile.ZipInfo("SOURCE_MANIFEST.json")
                info.external_attr = 0o100444 << 16
                archive.writestr(info, manifest_bytes)
            archive_sha = hashlib.sha256(path.read_bytes()).hexdigest()
            manifest_sha = hashlib.sha256(manifest_bytes).hexdigest()
            regular = SimpleNamespace(st_mode=stat.S_IFREG | 0o400, st_nlink=1)
            executable = SimpleNamespace(st_mode=stat.S_IFREG | 0o500, st_nlink=1)
            def fake_lstat(item):
                return executable if Path(item) == Path(runner.__file__) else regular
            with patch.object(runner, "BASE", root), \
                 patch.object(runner, "_trusted_path"), \
                 patch.object(runner, "_require_private_dir"), \
                 patch.object(runner.os, "O_NOFOLLOW", 0, create=True), \
                 patch.object(runner.os, "O_CLOEXEC", 0, create=True), \
                 patch.object(runner.os, "lstat", side_effect=fake_lstat):
                returned, _ = runner.verify_archive(path, archive_sha,
                                                    manifest_sha, "c" * 40,
                                                    runner_sha)
                self.assertEqual(returned, manifest)
                with self.assertRaisesRegex(ValueError, "runner differs"):
                    runner.verify_archive(path, archive_sha, manifest_sha,
                                          "c" * 40, "0" * 64)
                with patch.object(runner, "_file_digest", return_value="0" * 64):
                    with self.assertRaisesRegex(ValueError, "runner differs"):
                        runner.verify_archive(path, archive_sha,
                                              manifest_sha, "c" * 40,
                                              runner_sha)
                with self.assertRaisesRegex(ValueError, "manifest bytes differ"):
                    runner.verify_archive(path, archive_sha, "0" * 64,
                                          "c" * 40, runner_sha)
                with self.assertRaisesRegex(ValueError, "private incoming"):
                    runner.verify_archive(incoming / ".." / "outside.zip",
                                          archive_sha, manifest_sha,
                                          "c" * 40, runner_sha)
            self.assertEqual(runner_sha, manifest["files"][[entry["path"] for entry in
                manifest["files"]].index(runner.ENTRY)]["sha256"])


class CommandLine(unittest.TestCase):
    def test_help_exits_cleanly_without_admission_rejection(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(runner.main(["--help"]), 0)
        self.assertIn("--runner-sha256", output.getvalue())
        self.assertNotIn("ADMISSION_REJECTED", output.getvalue())

    def test_guard_flag_is_admitted_and_mutually_exclusive_with_probe(self):
        argv = ["--archive", "/var/lib/knowweave-c4/incoming/REVIEWED.zip",
                "--archive-sha256", "a" * 64, "--manifest-sha256", "b" * 64,
                "--source-commit", "c" * 40, "--runner-sha256", "d" * 64,
                "--batch-id", ID, "--subnet", SUBNET, "--bound-guard"]
        with patch.object(runner, "run", return_value=0) as admitted:
            self.assertEqual(runner.main(argv), 0)
            self.assertTrue(admitted.call_args.args[0].bound_guard)
            admitted.reset_mock()
            self.assertEqual(runner.main(argv + ["--bound-probe"]), 1)
            admitted.assert_not_called()

    def test_documented_runner_sha_is_passed_to_admission(self):
        argv = ["--archive", "/var/lib/knowweave-c4/incoming/REVIEWED.zip",
                "--archive-sha256", "a" * 64,
                "--manifest-sha256", "b" * 64,
                "--source-commit", "c" * 40,
                "--runner-sha256", "d" * 64,
                "--batch-id", ID, "--subnet", SUBNET]
        with patch.object(runner, "run", return_value=0) as admitted:
            self.assertEqual(runner.main(argv), 0)
        self.assertEqual(admitted.call_args.args[0].runner_sha256, "d" * 64)


class FinalResultPublication(unittest.TestCase):
    def test_partial_temp_write_never_appears_as_final_result(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory)
            def partial(path, data):
                path.write_bytes(data[:8])
                raise OSError("interrupted write")
            with patch.object(runner, "_private_write", side_effect=partial), \
                 patch.object(runner, "_sync_dir"):
                with self.assertRaises(OSError):
                    runner._publish_result(evidence, b'{"status":"PASSED"}')
            self.assertFalse((evidence / "result.json").exists())
            self.assertTrue((evidence / "result.pending.json").exists())

    def test_atomic_publish_never_overwrites_an_existing_result(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory)
            final = evidence / "result.json"
            final.write_bytes(b"older evidence")
            with patch.object(runner, "_private_write",
                              side_effect=lambda path, data: path.write_bytes(data)), \
                 patch.object(runner, "_sync_dir"):
                with self.assertRaises(FileExistsError):
                    runner._publish_result(evidence, b'{"status":"PASSED"}')
            self.assertEqual(final.read_bytes(), b"older evidence")


if __name__ == "__main__":
    unittest.main()
