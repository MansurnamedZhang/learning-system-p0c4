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
        )
        pin = SimpleNamespace(pin=lambda *_: {
            "birth_sha256": "d" * 64,
            "inspection_evidence_sha256": "f" * 64})
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
        code, result = self._run()
        self.assertEqual(code, 0, result)
        self.assertEqual(result["status"], runner.PASSED)
        self.assertEqual(result["birth_sha256"], "d" * 64)
        self.assertEqual(result["inspection_evidence_sha256"], "f" * 64)
        self.assertTrue(result["stop"]["confirmed"])
        self.assertEqual(json.loads((self.batch / "evidence" / "result.json").read_bytes())["status"],
                         runner.PASSED)

    def test_pin_mismatch_cannot_be_promoted(self):
        deps = self._dependencies()
        deps[2].pin = lambda *_: {"birth_sha256": "0" * 64,
                                  "inspection_evidence_sha256": "f" * 64}
        code, result = self._run(deps)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], runner.FAILED)
        self.assertNotIn("inspection_evidence_sha256", result)
        self.assertTrue(result["stop"]["confirmed"])

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


if __name__ == "__main__":
    unittest.main()
