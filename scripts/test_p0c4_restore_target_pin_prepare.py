"""Pre-creation record must be issued only for a fresh dedicated pin root."""

import contextlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import p0c4_restore_target_pin_prepare as prepare
from p0c4_restore_target import AdmissionError, identity_for
from test_p0c4_restore_target import ID, SUBNET


class Precreation(unittest.TestCase):
    def setUp(self):
        self.root = Path("/private/pin-root")
        self.initdb = Path("/reviewed/initdb.sh")
        self.identity = identity_for(ID)

    def test_existing_target_cannot_acquire_precreation_provenance(self):
        with patch.object(prepare.target_provisioner, "_locked_root",
                          return_value=contextlib.nullcontext()), \
             patch.object(prepare.pin, "_trusted_private_dir"), \
             patch.object(prepare.os.path, "lexists", return_value=True), \
             patch.object(prepare.target_provisioner, "_trusted_initdb"), \
             patch.object(prepare.target_provisioner, "snapshot") as snapshot, \
             patch.object(prepare.target_provisioner, "_private_write") as write:
            with self.assertRaises(AdmissionError):
                prepare.prepare(self.root, ID, SUBNET, self.initdb)
        snapshot.assert_not_called()
        write.assert_not_called()

    def test_root_with_old_evidence_cannot_be_repurposed_as_pin_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "targets").mkdir()
            (root / ".restore-target.lock").write_bytes(b"")
            (root / "old-evidence.json").write_bytes(b"{}")
            with patch.object(prepare.target_provisioner, "_locked_root",
                              return_value=contextlib.nullcontext()), \
                 patch.object(prepare.pin, "_trusted_private_dir"), \
                 patch.object(prepare.target_provisioner, "_trusted_initdb"), \
                 patch.object(prepare.target_provisioner, "snapshot") as snapshot:
                with self.assertRaises(AdmissionError):
                    prepare.prepare(root, ID, SUBNET, self.initdb)
            snapshot.assert_not_called()

    def test_precreation_records_fresh_absence_under_creation_lock(self):
        snapshot = {"daemon_id": "fresh-daemon", "containers": [],
                    "networks": [], "volumes": [], "routes": []}
        written = {}
        def capture(path, content):
            written["path"] = path
            written["content"] = content
        def directory_stat(path):
            if path == self.root:
                return SimpleNamespace(st_dev=8, st_ino=100)
            return SimpleNamespace(st_dev=8, st_ino=101)
        with patch.object(prepare.target_provisioner, "_locked_root",
                          return_value=contextlib.nullcontext()) as locked, \
             patch.object(prepare.pin, "_trusted_private_dir"), \
             patch.object(prepare.os.path, "lexists", return_value=False), \
             patch.object(prepare, "_root_has_only_setup", return_value=True), \
             patch.object(prepare, "_targets_empty", return_value=True), \
             patch.object(prepare.os, "lstat", side_effect=directory_stat), \
             patch.object(prepare.target_provisioner, "_trusted_initdb"), \
             patch.object(prepare.target_provisioner, "snapshot",
                          return_value=snapshot), \
             patch.object(prepare, "_initdb_digest", return_value="a" * 64), \
             patch.object(prepare.target_provisioner, "_private_write",
                          side_effect=capture):
            prepare.prepare(self.root, ID, SUBNET, self.initdb)
        locked.assert_called_once_with(self.root)
        self.assertEqual(written["path"], self.root / "pin-precreation.json")
        record = json.loads(written["content"])
        self.assertEqual(record["batch_id"], ID)
        self.assertEqual(record["docker_daemon_id"], "fresh-daemon")
        self.assertEqual(record["initdb_sha256"], "a" * 64)
        self.assertEqual(record["targets_ino"], 101)
        self.assertNotIn("password", written["content"].decode())
        def entries(path):
            if path == self.root:
                return iter((self.root / "targets",
                             self.root / ".restore-target.lock",
                             self.root / "pin-precreation.json"))
            return iter((self.root / "targets" / ID,))
        with patch.object(prepare.pin, "_trusted_private_dir"), \
             patch.object(prepare.pin, "_private_read",
                          return_value=written["content"]), \
             patch.object(prepare.pin, "_initdb_digest", return_value="a" * 64), \
             patch.object(prepare.pin.os, "lstat", side_effect=directory_stat), \
             patch.object(Path, "iterdir", autospec=True,
                          side_effect=entries):
            checked, raw = prepare.pin.read_precreation(
                self.root, self.identity, ID, self.initdb)
            self.assertEqual(checked, record)
            self.assertEqual(raw, written["content"])

    def test_acceptance_root_cannot_prepare_even_without_a_target(self):
        root = Path("/var/lib/knowweave-c4/birth-acceptance/batches") / ID / "control"
        with patch.object(prepare.target_provisioner, "_locked_root") as locked:
            with self.assertRaises(AdmissionError):
                prepare.prepare(root, ID, SUBNET, self.initdb)
            locked.assert_not_called()

    def test_pin_rejects_new_sibling_dirty_evidence_after_preparation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            targets = root / "targets"
            targets.mkdir()
            (targets / ID).mkdir()
            (root / ".restore-target.lock").write_bytes(b"")
            (root / "pin-precreation.json").write_bytes(b"placeholder")
            root_meta, targets_meta = root.stat(), targets.stat()
            record = {
                "format_version": 1,
                "state": "PIN_ONLY_PRECREATION_NOT_RESTORE_AUTHORITY",
                "target_absent_at_precreation": True,
                "batch_id": ID,
                "project": self.identity["project"],
                "database": self.identity["database"],
                "network": self.identity["network"],
                "volume": self.identity["volume"],
                "subnet": SUBNET,
                "root_path": str(root),
                "root_dev": root_meta.st_dev,
                "root_ino": root_meta.st_ino,
                "targets_dev": targets_meta.st_dev,
                "targets_ino": targets_meta.st_ino,
                "docker_daemon_id": "fresh-daemon",
                "initdb_path": str(self.initdb),
                "initdb_sha256": "a" * 64,
            }
            raw = json.dumps(record, sort_keys=True, ensure_ascii=False,
                             separators=(",", ":")).encode()
            with patch.object(prepare.pin, "_trusted_private_dir"), \
                 patch.object(prepare.pin, "_private_read", return_value=raw), \
                 patch.object(prepare.pin, "_initdb_digest",
                              return_value="a" * 64):
                prepare.pin.read_precreation(root, self.identity, ID,
                                             self.initdb)
                (root / "dirty-result.json").write_bytes(b"{}")
                with self.assertRaises(AdmissionError):
                    prepare.pin.read_precreation(root, self.identity, ID,
                                                 self.initdb)


if __name__ == "__main__":
    unittest.main()
