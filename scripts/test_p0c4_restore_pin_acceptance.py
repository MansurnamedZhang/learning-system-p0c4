"""Single-host pin acceptance never promotes a failed or replayed batch."""

import json
import hashlib
import contextlib
import io
from pathlib import Path
import stat
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile

import p0c4_restore_pin_acceptance as runner
from test_p0c4_restore_target import ID, SUBNET


class PinAcceptance(unittest.TestCase):
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
        acceptance = SimpleNamespace(
            _run_issuer=lambda *_: SimpleNamespace(returncode=0, stdout="issued"),
            accept_issuer_process=lambda *_: (birth, state, success),
            _independent_docker_gate=lambda *_: {"container_id": "e" * 64},
            stop_verified_pg=lambda *_: {"confirmed": True, "volume_retained": True},
            stop_early_owned_pg=lambda *_: {"confirmed": True, "volume_retained": True},
            read_issuer_diagnostic=lambda *_: {"status": "UNAVAILABLE"},
            _private_read_diagnostic=lambda path, limit=262144: path.read_bytes(),
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
                   "birth_evidence_sha256": "c" * 64,
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

    def _run(self, deps=None):
        deps = deps or self._dependencies()
        with patch.object(runner, "extract_and_load", return_value=(*deps, Path("/initdb"), "2" * 64)), \
             patch.object(runner, "source_digest", return_value="2" * 64), \
             patch.object(runner, "_file_digest", return_value="3" * 64), \
             patch.object(runner, "_private_write", side_effect=lambda path, data: path.write_bytes(data)):
            return runner._run_batch(self.args, {}, b"zip", self.batch)

    def test_clean_candidate_only_after_exact_stop(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code, result = self._run()
        self.assertEqual(code, 0, result)
        self.assertEqual(result["status"], runner.PASSED)
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
        self.args.bound_probe_cargo = Path("/usr/bin/cargo")
        sequence = []
        deps = self._dependencies()
        deps[1].stop_verified_pg = lambda *_: (sequence.append("stop") or
            {"confirmed": True, "volume_retained": True})
        with patch.object(runner, "_run_bound_probe",
                          side_effect=lambda *_: sequence.append("probe") or
                          {"state": "BOUND_TARGET_READ_ONLY_PG18_PASSED_NOT_RESTORE"}):
            code, result = self._run(deps)
        self.assertEqual(code, 0, result)
        self.assertEqual(sequence, ["probe", "stop"])
        self.assertEqual(result["status"], runner.BOUND_PASSED)
        self.assertTrue(result["stop"]["confirmed"])

    def test_failed_bound_probe_still_stops_and_cannot_pass(self):
        self.args.bound_probe_cargo = Path("/usr/bin/cargo")
        sequence = []
        deps = self._dependencies()
        deps[1].stop_verified_pg = lambda *_: (sequence.append("stop") or
            {"confirmed": True, "volume_retained": True})
        def fail_probe(*_):
            sequence.append("probe")
            raise RuntimeError("probe failed")
        with patch.object(runner, "_run_bound_probe",
                          side_effect=fail_probe):
            code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(sequence, ["probe", "stop"])
        self.assertEqual(result["status"], runner.BOUND_FAILED)
        self.assertTrue(result["stop"]["confirmed"])

    def test_bound_probe_rejects_zero_test_cargo_result(self):
        cargo = self.batch / "cargo"
        cargo.write_bytes(b"binary")
        cargo.chmod(0o555)
        with patch.object(runner, "_trusted_path"), \
             patch.object(runner, "_private_dir"), \
             patch.object(runner.subprocess, "run", return_value=SimpleNamespace(
                 returncode=0, stdout=b"test result: ok. 0 passed; 0 failed; 0 ignored;",
                 stderr=b"")) as process:
            with self.assertRaises(ValueError):
                runner._run_bound_probe(cargo, self.batch / "source", self.batch,
                                        self.batch / "control" / "targets" / ID,
                                        "learning_restore_c4_" + ID, "d" * 64)
        command = process.call_args.args[0]
        self.assertIn("--offline", command)
        self.assertIn("--exact", command)
        self.assertIn("--ignored", command)
        self.assertEqual(process.call_args.kwargs["env"][
            "KNOWWEAVE_C4_TARGET_BIRTH_SHA256"], "d" * 64)

    def test_bound_probe_accepts_exact_one_test_marker(self):
        cargo = self.batch / "cargo"
        cargo.write_bytes(b"binary")
        cargo.chmod(0o555)
        output = (b"BOUND_TARGET_READ_ONLY_PG18_PASSED_NOT_RESTORE\n"
                  b"test result: ok. 1 passed; 0 failed; 0 ignored; 18 filtered out;")
        with patch.object(runner, "_trusted_path"), \
             patch.object(runner, "_private_dir"), \
             patch.object(runner.subprocess, "run", return_value=SimpleNamespace(
                 returncode=0, stdout=output, stderr=b"")):
            result = runner._run_bound_probe(
                cargo, self.batch / "source", self.batch,
                self.batch / "control" / "targets" / ID,
                "learning_restore_c4_" + ID, "d" * 64)
        self.assertEqual(result["birth_sha256"], "d" * 64)
        self.assertEqual(result["state"],
                         "BOUND_TARGET_READ_ONLY_PG18_PASSED_NOT_RESTORE")

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
