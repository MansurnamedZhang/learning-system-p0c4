"""Static/mock transfer acceptance checks; never invokes Docker or sudo."""

import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import uuid
import zipfile

from p0c4_task3_transfer_acceptance import (
    GATES, assert_test_output, cleanup_project, private_dir, redact, run_gate, source_hashes,
    verify_archive,
)


COMMIT = "a" * 40


def fixture(path, *, linked=False):
    content = b"reviewed"
    manifest = json.dumps({"format_version": 1, "commit": COMMIT,
                           "files": [{"path": "source.txt", "sha256": hashlib.sha256(content).hexdigest(),
                                      "size": len(content)}]}, sort_keys=True,
                          separators=(",", ":")).encode()
    with zipfile.ZipFile(path, "w") as archive:
        entry = zipfile.ZipInfo("source.txt")
        entry.external_attr = (0o120777 if linked else 0o100444) << 16
        archive.writestr(entry, content)
        archive.writestr("SOURCE_MANIFEST.json", manifest)
    return hashlib.sha256(path.read_bytes()).hexdigest(), hashlib.sha256(manifest).hexdigest()


class TransferAcceptanceTests(unittest.TestCase):
    def test_exact_archive_and_manifest_are_required_before_extraction(self):
        with tempfile.TemporaryDirectory() as tmp:
            archive = Path(tmp) / "source.zip"
            archive_sha, manifest_sha = fixture(archive)
            manifest, data = verify_archive(archive, archive_sha, manifest_sha, COMMIT)
            self.assertEqual(manifest["commit"], COMMIT)
            self.assertEqual(hashlib.sha256(data).hexdigest(), archive_sha)
            with self.assertRaises(ValueError):
                verify_archive(archive, "0" * 64, manifest_sha, COMMIT)
            with self.assertRaises(ValueError):
                verify_archive(archive, archive_sha, "0" * 64, COMMIT)
            with self.assertRaises(ValueError):
                verify_archive(archive, archive_sha, manifest_sha, "b" * 40)
            archive.write_bytes(archive.read_bytes() + b"changed")
            with self.assertRaises(ValueError):
                verify_archive(archive, archive_sha, manifest_sha, COMMIT)

    def test_linked_archive_entry_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            archive = Path(tmp) / "source.zip"
            archive_sha, manifest_sha = fixture(archive, linked=True)
            with self.assertRaises(ValueError):
                verify_archive(archive, archive_sha, manifest_sha, COMMIT)

    def test_exact_default_build_test_counts_and_names(self):
        cases = {label: expected for label, _, expected in GATES if expected is not None}
        self.assertEqual({name: case[0] for name, case in cases.items()},
                         {"sealed-linux": 7, "transfer-interruption": 1,
                          "complete-disabled": 2, "witness-contract": 3})
        for label, (count, names) in cases.items():
            output = ("\n".join(f"test {name} ... ok" for name in names)
                      + f"\ntest result: ok. {count} passed; 0 failed; 0 ignored; 0 measured;").encode()
            assert_test_output(label, output, (count, names))
            with self.assertRaises(ValueError):
                assert_test_output(label, output.replace(b"0 failed", b"1 failed"), (count, names))
            with self.assertRaises(ValueError):
                assert_test_output(label, output.replace(names[0].encode(), b"wrong"), (count, names))

    def test_mock_docker_command_has_no_network_or_credentials(self):
        with tempfile.TemporaryDirectory() as tmp:
            batch = Path(tmp)
            private_dir(batch / "evidence")
            source = private_dir(batch / "source")
            target = private_dir(batch / "target")
            project = "learning-system-p0c4-task3-transfer-" + uuid.uuid4().hex[:12]
            completed = subprocess.CompletedProcess([], 0, b"", b"")
            with patch("p0c4_task3_transfer_acceptance.subprocess.run", return_value=completed) as called:
                run_gate(batch, project, source, target, "a" * 64, COMMIT, GATES[0])
            command = called.call_args.args[0]
            self.assertEqual(command[0:2], ["/usr/bin/docker", "run"])
            self.assertEqual(command[command.index("--network") + 1], "none")
            self.assertIn(f"type=bind,src={source},dst=/reviewed,readonly", command)
            self.assertIn("--no-default-features", GATES[2][1])
            self.assertNotIn("PGPASSWORD", " ".join(command))
            self.assertNotIn("PGPASSFILE", " ".join(command))
            self.assertEqual((batch / "evidence/format.exit").read_text(), "0")

    def test_hash_inventory_and_redaction(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "source.txt").write_bytes(b"reviewed")
            manifest = {"files": [{"path": "source.txt", "sha256": hashlib.sha256(b"reviewed").hexdigest()}]}
            self.assertEqual(source_hashes(root, manifest)["source.txt"], manifest["files"][0]["sha256"])
            (root / "extra.txt").write_text("extra")
            with self.assertRaises(ValueError):
                source_hashes(root, manifest)
            (root / "extra.txt").unlink()
            (root / "empty").mkdir()
            with self.assertRaises(ValueError):
                source_hashes(root, manifest)
        self.assertNotIn(b"secret-value", redact(b"password=secret-value"))

    def test_cleanup_never_removes_same_name_without_batch_label(self):
        project = "learning-system-p0c4-task3-transfer-abcdef123456"
        commands = []

        def fake_run(command, **_kwargs):
            commands.append(command)
            if command[1] == "inspect":
                facts = {"Id": "a" * 64, "Name": "/" + command[-1],
                         "Config": {"Labels": {"com.knowweave.acceptance.project": "another-project"}}}
                return subprocess.CompletedProcess(command, 0, json.dumps(facts).encode(), b"")
            raise AssertionError("foreign container must not be removed")

        with patch("p0c4_task3_transfer_acceptance.subprocess.run", side_effect=fake_run), \
                patch("p0c4_task3_transfer_acceptance.subprocess.check_output", return_value=b""):
            cleanup = cleanup_project(project)
        self.assertEqual(len(cleanup["errors"]), len(GATES))
        self.assertFalse(any(command[1] == "rm" for command in commands))


if __name__ == "__main__":
    unittest.main()
