"""Read-only, local contract tests for a fresh C4 target pin candidate."""

import contextlib
import copy
import hashlib
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import p0c4_restore_target_pin as pin
from p0c4_restore_target import AdmissionError, identity_for
from test_p0c4_restore_birth_acceptance import records
from test_p0c4_restore_target import ID, created
from test_p0c4_restore_target_birth import clean_facts


def live_docker(identity, target, initdb):
    live = created(identity)
    for destination, source in (
        ("/docker-entrypoint-initdb.d/10-restore.sh", initdb),
        ("/run/secrets/postgres_password", target / "secrets/postgres_password"),
        ("/run/secrets/admin_password", target / "secrets/admin_password"),
    ):
        live["containers"][0]["Mounts"].append({
            "Type": "bind", "Source": str(source), "RW": False,
            "Destination": destination})
    live["volumes"][0].update(Driver="local", Scope="local", Options=None)
    return live


def observed_pg(birth):
    return {
        "server_version_num": 180006, "database_oid": birth["database_oid"],
        "pg_system_identifier": birth["pg_system_identifier"],
        "cast_count": birth["baseline_cast_count"],
        "database_owner": "learning_admin", "runtime_create": False,
        "other_sessions": 0, "user_relations": 0, "public_collations": 0,
        "default_acls": 0, "public_schema": birth["public_schema"],
    }


class PinRecords(unittest.TestCase):
    def test_acceptance_batch_root_is_structurally_ineligible(self):
        root = (Path("/var/lib/knowweave-c4/birth-acceptance/batches") /
                ID / "control")
        with patch.object(pin.target_provisioner, "_trusted_initdb"), \
             patch.object(pin, "_existing_creation_lock") as locked:
            with self.assertRaises(AdmissionError):
                pin.pin(root, ID, Path("/reviewed/initdb.sh"))
            locked.assert_not_called()

    def test_empty_private_roots_require_original_inodes_and_no_assets(self):
        identity, birth_bytes, _, _, evidence = records()
        birth = json.loads(birth_bytes)
        evidence.update(destination_dev=44, destination_ino=300,
                        control_dev=birth["control_dev"],
                        control_ino=birth["control_ino"],
                        asset_dev=birth["asset_dev"],
                        asset_ino=birth["asset_ino"])
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            for name in ("destination", "control", "assets"):
                (target / name).mkdir()
            (target / "control" / (identity["database"] + ".birth.json")).write_bytes(
                birth_bytes)
            ids = {"destination": (44, 300), "control": (42, 100),
                   "assets": (43, 200)}
            def directory_stat(path):
                dev, ino = ids[Path(path).name]
                return SimpleNamespace(st_dev=dev, st_ino=ino)
            with patch.object(pin, "_trusted_private_dir"), \
                 patch.object(pin.os, "lstat", side_effect=directory_stat):
                pin._empty_roots(target, identity, birth, evidence)
                ids["assets"] = (43, 201)
                with self.assertRaises(AdmissionError):
                    pin._empty_roots(target, identity, birth, evidence)
                ids["assets"] = (43, 200)
                (target / "assets" / "unapproved").write_text("x")
                with self.assertRaises(AdmissionError):
                    pin._empty_roots(target, identity, birth, evidence)

    def test_existing_creation_lock_never_creates_a_file(self):
        lock = SimpleNamespace(st_mode=stat.S_IFREG | 0o600, st_uid=0,
                               st_nlink=1)
        fake_fcntl = SimpleNamespace(LOCK_EX=2, LOCK_NB=4, flock=lambda *_: None)
        with patch.dict(sys.modules, {"fcntl": fake_fcntl}), \
             patch.object(pin, "_trusted_private_dir"), \
             patch.object(pin.os, "O_NOFOLLOW", 0x1000000, create=True), \
             patch.object(pin.os, "O_CLOEXEC", 0x2000000, create=True), \
             patch.object(pin.os, "O_NONBLOCK", 0x4000000, create=True), \
             patch.object(pin.os, "open", return_value=12) as opened, \
             patch.object(pin.os, "fstat", return_value=lock), \
             patch.object(pin.os, "close"):
            with pin._existing_creation_lock(Path("/private")):
                pass
        self.assertEqual(opened.call_count, 1)
        self.assertFalse(opened.call_args.args[1] & os.O_CREAT)

    def test_failure_marker_wins_before_any_private_record_read(self):
        identity = identity_for(ID)
        target = Path("/private/targets") / ID
        with patch.object(pin, "_trusted_private_dir"), \
             patch.object(pin.os.path, "lexists", return_value=True), \
             patch.object(pin, "_private_read") as read:
            with self.assertRaises((AdmissionError, ValueError)):
                pin.read_records(target, identity, ID)
            read.assert_not_called()

    def test_control_symlink_is_rejected_before_following_record_path(self):
        identity = identity_for(ID)
        target = Path("/private/targets") / ID
        def trust(path):
            if path == target / "control":
                raise AdmissionError("untrusted control symlink")
        with patch.object(pin, "_trusted_private_dir", side_effect=trust), \
             patch.object(pin.os.path, "lexists", return_value=False), \
             patch.object(pin, "_private_read") as read:
            with self.assertRaises(AdmissionError):
                pin.read_records(target, identity, ID)
            read.assert_not_called()

    def test_missing_or_mismatched_success_seal_cannot_be_a_candidate(self):
        identity, birth_bytes, state, success, evidence = records()
        target = Path("/private/targets") / ID
        payloads = {
            identity["database"] + ".birth.json": birth_bytes,
            "state.json": json.dumps(state).encode(),
            "issuance-success.json": json.dumps(success).encode(),
            "birth-evidence.json": json.dumps(evidence).encode(),
        }
        def private_read(path):
            return payloads[Path(path).name]
        with patch.object(pin, "_trusted_private_dir"), \
             patch.object(pin.os.path, "lexists", return_value=False), \
             patch.object(pin, "_empty_roots"), \
             patch.object(pin, "_private_read", side_effect=private_read):
            del payloads["issuance-success.json"]
            with self.assertRaises(KeyError):
                pin.read_records(target, identity, ID)
            payloads["issuance-success.json"] = json.dumps(
                {**success, "birth_sha256": "0" * 64}).encode()
            with self.assertRaises((AdmissionError, ValueError)):
                pin.read_records(target, identity, ID)

    def test_private_root_rejects_symlink_and_writable_ancestor(self):
        path = Path("/private/targets") / ID
        regular = SimpleNamespace(st_mode=stat.S_IFDIR | 0o700, st_uid=0)
        symlink = SimpleNamespace(st_mode=stat.S_IFLNK | 0o777, st_uid=0)
        writable = SimpleNamespace(st_mode=stat.S_IFDIR | 0o777, st_uid=0)
        for bad in (symlink, writable):
            with self.subTest(mode=bad.st_mode), \
                 patch.object(pin.os, "lstat", side_effect=lambda p: bad if p == path else regular), \
                 patch.object(pin.os, "geteuid", return_value=0, create=True):
                with self.assertRaises(AdmissionError):
                    pin._trusted_private_dir(path)


class PinLiveGate(unittest.TestCase):
    def setUp(self):
        self.identity, self.birth_bytes, self.state, self.success, self.evidence = records()
        self.birth = json.loads(self.birth_bytes)
        self.root = Path("/private")
        self.target = self.root / "targets" / ID
        self.initdb = Path("/reviewed/initdb.sh")
        self.live = live_docker(self.identity, self.target, self.initdb)
        self.evidence.update({
            "state": "BIRTH_SOURCE_REINSPECTED_NOT_RESTORE_ACCEPTANCE",
            "project": self.identity["project"], "batch_id": ID,
            "docker_daemon_id": self.live["daemon_id"],
            "destination_dev": 44, "destination_ino": 300,
            "control_dev": self.birth["control_dev"],
            "control_ino": self.birth["control_ino"],
            "asset_dev": self.birth["asset_dev"],
            "asset_ino": self.birth["asset_ino"],
        })

    def run_pin(self, live=None, pg=None, issuer_facts=None):
        live = self.live if live is None else live
        pg = observed_pg(self.birth) if pg is None else pg
        issuer_facts = clean_facts() if issuer_facts is None else issuer_facts
        with patch.object(pin.target_provisioner, "snapshot", return_value=live), \
             patch.object(pin.target_provisioner, "_inspect", return_value=live["images"]), \
             patch.object(pin.acceptance, "verify_live_mount_inode"), \
             patch.object(pin.acceptance, "observe_pg", return_value=pg), \
             patch.object(pin.issuer, "probe_pg_facts", return_value=issuer_facts):
            return pin.inspect_live(self.identity, self.state, self.success,
                                    self.evidence, self.birth, self.target,
                                    self.initdb)

    def test_positive_candidate_binds_evidence_and_emits_digest_only(self):
        checked = self.run_pin()
        candidate = pin.inspection_evidence_digest(
            ID, str(self.root), b"precreation", self.birth_bytes,
            json.dumps(self.state).encode(), json.dumps(self.success).encode(),
            json.dumps(self.evidence).encode(), checked)
        self.assertRegex(candidate, r"\A[0-9a-f]{64}\Z")
        self.assertNotIn(self.identity["database"], candidate)
        self.assertNotIn(self.state["container_id"], candidate)
        changed = pin.inspection_evidence_digest(
            ID, str(self.root), b"precreation", self.birth_bytes,
            json.dumps(self.state).encode(), json.dumps(self.success).encode(),
            json.dumps(self.evidence).encode(), {**checked, "image_id": "sha256:" + "c" * 64})
        self.assertNotEqual(candidate, changed)

    def test_docker_identity_and_mount_drift_reject(self):
        for change in ("Id", "Image", "Mounts", "network"):
            live = copy.deepcopy(self.live)
            if change == "Id":
                live["containers"][0]["Id"] = "c" * 64
            elif change == "Image":
                live["containers"][0]["Image"] = "sha256:" + "c" * 64
            elif change == "Mounts":
                live["containers"][0]["Mounts"][0]["Source"] = "/other"
            else:
                live["networks"][0]["Id"] = "c" * 64
            with self.subTest(change=change), self.assertRaises((AdmissionError, ValueError)):
                self.run_pin(live=live)

    def test_health_log_timestamp_churn_does_not_invalidate_stable_identity(self):
        changed = copy.deepcopy(self.live)
        changed["containers"][0]["State"]["Health"]["Log"] = [
            {"Start": "2026-09-28T00:00:01Z"}]
        with patch.object(pin.target_provisioner, "snapshot",
                          side_effect=[self.live, changed]), \
             patch.object(pin.target_provisioner, "_inspect",
                          return_value=self.live["images"]), \
             patch.object(pin.acceptance, "verify_live_mount_inode"), \
             patch.object(pin.acceptance, "observe_pg",
                          return_value=observed_pg(self.birth)), \
             patch.object(pin.issuer, "probe_pg_facts",
                          return_value=clean_facts()):
            checked = pin.inspect_live(self.identity, self.state, self.success,
                                       self.evidence, self.birth, self.target,
                                       self.initdb)
        self.assertEqual(checked["container_id"], self.state["container_id"])

    def test_pg_identity_acl_and_dirty_state_reject(self):
        for change in ("database_oid", "pg_system_identifier", "runtime_create",
                       "user_relations", "public_schema"):
            pg = copy.deepcopy(observed_pg(self.birth))
            if change == "public_schema":
                pg[change]["acl"] = []
            elif change == "runtime_create":
                pg[change] = True
            elif change == "pg_system_identifier":
                pg[change] = "123456789"
            else:
                pg[change] += 1
            with self.subTest(change=change), self.assertRaises((AdmissionError, ValueError)):
                self.run_pin(pg=pg)
        dirty_issuer = clean_facts()
        dirty_issuer["dirty_counts"]["extensions"] = 1
        with self.assertRaises((AdmissionError, ValueError)):
            self.run_pin(issuer_facts=dirty_issuer)

    def test_volume_inode_swap_during_pg_check_rejects(self):
        with patch.object(pin.target_provisioner, "snapshot", return_value=self.live), \
             patch.object(pin.target_provisioner, "_inspect",
                          return_value=self.live["images"]), \
             patch.object(pin.acceptance, "verify_live_mount_inode",
                          side_effect=[None, AdmissionError("volume changed")]), \
             patch.object(pin.acceptance, "observe_pg",
                          return_value=observed_pg(self.birth)), \
             patch.object(pin.issuer, "probe_pg_facts",
                          return_value=clean_facts()):
            with self.assertRaises(AdmissionError):
                pin.inspect_live(self.identity, self.state, self.success,
                                 self.evidence, self.birth, self.target,
                                 self.initdb)

    def test_no_candidate_stdout_on_failed_live_gate(self):
        with patch.object(pin, "_existing_creation_lock", return_value=contextlib.nullcontext()), \
             patch.object(pin, "read_records", return_value=(
                 self.birth, self.state, self.success, self.evidence,
                 self.birth_bytes, json.dumps(self.state).encode(),
                 json.dumps(self.success).encode(), json.dumps(self.evidence).encode())), \
             patch.object(pin, "read_precreation",
                          return_value=({"subnet": self.state["subnet"],
                                         "docker_daemon_id": self.evidence[
                                             "docker_daemon_id"]}, b"precreation")), \
             patch.object(pin.target_provisioner, "_trusted_initdb"), \
             patch.object(pin, "inspect_live", side_effect=AdmissionError("secret=hidden")), \
             patch("builtins.print") as output, \
             patch.object(pin.os, "geteuid", return_value=0, create=True):
            self.assertEqual(pin.main(["--root", str(self.root), "--batch-id", ID,
                                       "--initdb", str(self.initdb)]), 1)
            output.assert_called_once_with("PIN_CANDIDATE_REJECTED", flush=True)

    def test_missing_precreation_never_reaches_live_inspection_or_candidate(self):
        payloads = (self.birth, self.state, self.success, self.evidence,
                    self.birth_bytes, json.dumps(self.state).encode(),
                    json.dumps(self.success).encode(),
                    json.dumps(self.evidence).encode())
        with patch.object(pin, "_existing_creation_lock", return_value=contextlib.nullcontext()), \
             patch.object(pin, "_trusted_private_dir"), \
             patch.object(pin, "read_records", return_value=payloads), \
             patch.object(pin, "read_precreation",
                          side_effect=FileNotFoundError) as provenance, \
             patch.object(pin.target_provisioner, "_trusted_initdb"), \
             patch.object(pin, "inspect_live") as inspect, \
             patch("builtins.print") as output:
            self.assertEqual(pin.main(["--root", str(self.root), "--batch-id", ID,
                                       "--initdb", str(self.initdb)]), 1)
        provenance.assert_called_once()
        inspect.assert_not_called()
        output.assert_called_once_with("PIN_CANDIDATE_REJECTED", flush=True)

    def test_successful_entrypoint_prints_only_a_candidate_digest(self):
        payloads = (self.birth, self.state, self.success, self.evidence,
                    self.birth_bytes, json.dumps(self.state).encode(),
                    json.dumps(self.success).encode(),
                    json.dumps(self.evidence).encode())
        with patch.object(pin, "_existing_creation_lock", return_value=contextlib.nullcontext()), \
             patch.object(pin, "_trusted_private_dir"), \
             patch.object(pin, "read_records", return_value=payloads), \
             patch.object(pin, "read_precreation",
                          return_value=({"subnet": self.state["subnet"],
                                         "docker_daemon_id": self.evidence[
                                             "docker_daemon_id"]}, b"precreation")), \
             patch.object(pin.target_provisioner, "_trusted_initdb"), \
             patch.object(pin, "inspect_live", return_value=self.run_pin()), \
             patch("builtins.print") as output:
            self.assertEqual(pin.main(["--root", str(self.root), "--batch-id", ID,
                                       "--initdb", str(self.initdb)]), 0)
        printed = json.loads(output.call_args.args[0])
        self.assertEqual(set(printed), {"state", "birth_sha256",
                                        "inspection_evidence_sha256"})
        self.assertEqual(printed["birth_sha256"],
                         hashlib.sha256(self.birth_bytes).hexdigest())
        self.assertRegex(printed["inspection_evidence_sha256"],
                         r"\A[0-9a-f]{64}\Z")
        self.assertNotEqual(printed["birth_sha256"],
                            printed["inspection_evidence_sha256"])
        self.assertNotIn(self.identity["database"], output.call_args.args[0])


if __name__ == "__main__":
    unittest.main()
