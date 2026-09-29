"""Local contract tests for opt-in, same-lock C4 target birth issuance."""

import copy
import contextlib
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import p0c4_restore_target_birth as birth_module
from p0c4_restore_target import (AdmissionError, identity_for, provision,
                                 verify_created)
from p0c4_restore_target_birth import (canonical_birth, validate_pg_facts,
                                       verify_birth_docker, _publish_birth,
                                       issue_birth, probe_pg_facts,
                                       finalize_birth_issuance)
from test_p0c4_restore_target import ID, SUBNET, created, empty_snapshot


def clean_facts():
    return {
        "database_oid": 16385,
        "pg_system_identifier": "7361082129910479001",
        "cast_count": 203,
        "public_schema": {
            "owner_oid": 6171, "owner_name": "pg_database_owner",
            "acl_is_null": False,
            "acl": [
                {"grantor_oid": 6171, "grantee_oid": 6171,
                 "privilege": "CREATE", "grantable": False},
                {"grantor_oid": 6171, "grantee_oid": 6171,
                 "privilege": "USAGE", "grantable": False},
                {"grantor_oid": 6171, "grantee_oid": 0,
                 "privilege": "USAGE", "grantable": False},
            ],
        },
        "runtime_can_create_public": False,
        "other_roles": 0,
        "app_role_memberships": 0,
        "public_database_grants": 0,
        "nonowner_database_grants": 0,
        "runtime_can_use_database": False,
        "role_flags_secure": True,
        "dirty_counts": {name: 0 for name in (
            "relations", "schemas", "routines", "types", "extensions",
            "event_triggers", "publications", "large_objects", "collations",
            "conversions", "operators", "operator_classes", "operator_families",
            "text_search_objects", "default_acls", "foreign_objects",
            "custom_languages", "custom_access_methods", "global_ddl")},
        "other_sessions": 0,
    }


class BirthContract(unittest.TestCase):
    def test_unexpected_issuer_failure_has_only_generic_stdout(self):
        secret = "postgres_password=hidden"
        with (patch("p0c4_restore_target.provision",
                    side_effect=RuntimeError(secret)),
              patch("sys.argv", ["birth", "--root", "/private",
                                 "--batch-id", ID, "--subnet", SUBNET,
                                 "--initdb", "/reviewed/initdb.sh"]),
              patch("builtins.print") as output):
            self.assertEqual(birth_module.main(), 1)
        output.assert_called_once_with(
            "BIRTH_NOT_ISSUED_TARGET_QUARANTINED_OR_ADMISSION_REJECTED",
            flush=True)

    def test_issuer_failure_diagnostic_uses_only_bounded_codes(self):
        identity = identity_for(ID)
        secret = "postgres_password=hidden postgresql://user:pass@host/db"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "targets" / ID
            target.mkdir(parents=True)
            status = {"batch_id": ID}
            written = {}
            def capture(path, content):
                written[path.name] = content
            with (patch("p0c4_restore_target_birth._verify_creation_state",
                        side_effect=AdmissionError(secret)),
                  patch("p0c4_restore_target._trusted_root"),
                  patch("p0c4_restore_target._private_write",
                        side_effect=capture)):
                with self.assertRaises(AdmissionError):
                    issue_birth(root, target, identity, SUBNET, empty_snapshot(),
                                {}, status, Path("/reviewed/initdb.sh"))
            payload = written["issuer-diagnostic.json"]
            self.assertNotIn(secret.encode(), payload)
            self.assertNotIn(b"postgres_password", payload)
            diagnostic = json.loads(payload)
            self.assertEqual(diagnostic["phase"], "CREATION_STATE")
            self.assertEqual(diagnostic["reason_code"],
                             "CREATION_STATE_REJECTED")
            self.assertEqual(diagnostic["exception_class"], "AdmissionError")
            self.assertEqual(diagnostic["batch_id"], ID)

    def test_pg_failure_phase_callback_separates_execute_parse_validate(self):
        identity = identity_for(ID)
        secret = "postgres_password=hidden"
        cases = (
            (RuntimeError(secret), "PG_SQL_EXECUTION"),
            ("not-json\n", "PG_SQL_PARSE"),
            (json.dumps({**clean_facts(), "runtime_can_create_public": True})
             + "\n", "PG_FACTS_VALIDATION"),
        )
        for output, expected in cases:
            phases = []
            with self.subTest(expected=expected), \
                 patch("p0c4_restore_target._docker",
                       side_effect=output if isinstance(output, BaseException)
                       else None, return_value=output if isinstance(output, str)
                       else None):
                with self.assertRaises((AdmissionError, RuntimeError)):
                    probe_pg_facts(identity, "a" * 64, _phase=phases.append)
            self.assertEqual(phases[-1], expected)

    def test_non_rfc_version_nibble_is_not_a_python_uuid_v4(self):
        non_rfc = "550e8400-e29b-41d4-0716-446655440000"
        with self.assertRaises(AdmissionError):
            identity_for(non_rfc)
        with self.assertRaises(AdmissionError):
            canonical_birth(identity_for(ID), clean_facts(), (42, 100),
                            (43, 200), non_rfc)

    def test_interruption_after_birth_has_no_success_seal(self):
        identity = identity_for(ID)
        payload = canonical_birth(identity, clean_facts(), (42, 100), (43, 200),
                                  "550e8400-e29b-41d4-a716-446655440001")
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            control = target / "control"
            control.mkdir()
            status = {"batch_id": ID}
            ids = {"container_id": "a" * 64, "network_id": "b" * 64,
                   "volume_name": identity["volume"],
                   "volume_mountpoint": "/var/lib/docker/volumes/new/_data"}
            def publish(folder, name, content, _nonce):
                (folder / name).write_bytes(content)
                return hashlib.sha256(content).hexdigest()
            with (patch("p0c4_restore_target_birth._publish_birth",
                        side_effect=publish),
                  patch("p0c4_restore_target_birth._read_published_birth",
                        side_effect=KeyboardInterrupt)):
                with self.assertRaises(KeyboardInterrupt):
                    finalize_birth_issuance(target, control, identity, status, ids,
                                            "sha256:" + "d" * 64, (9, 10),
                                            payload, "550e8400-e29b-41d4-a716-446655440001")
            self.assertTrue((control / (identity["database"] + ".birth.json")).exists())
            self.assertFalse((target / "issuance-success.json").exists())

    def test_fsync_and_rollback_failure_leave_birth_but_never_seal(self):
        identity = identity_for(ID)
        payload = b"birth content"
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            control = target / "control"
            control.mkdir()
            real_unlink = os.unlink
            def fail_final_unlink(path):
                if str(path).endswith(".birth.json"):
                    raise OSError("rollback unlink failed")
                return real_unlink(path)
            with (patch("p0c4_restore_target._private_write",
                        side_effect=lambda path, data: path.write_bytes(data)),
                  patch("p0c4_restore_target._sync_directory",
                        side_effect=OSError("fsync failed")),
                  patch("p0c4_restore_target_birth.os.unlink",
                        side_effect=fail_final_unlink),
                  patch("p0c4_restore_target_birth._read_published_birth") as read):
                with self.assertRaises(OSError):
                    finalize_birth_issuance(target, control, identity,
                                            {"batch_id": ID}, {}, "sha256:" + "d" * 64,
                                            (9, 10), payload,
                                            "550e8400-e29b-41d4-a716-446655440001")
            self.assertTrue((control / (identity["database"] + ".birth.json")).exists())
            self.assertFalse((target / "issuance-success.json").exists())
            read.assert_not_called()

    @patch("p0c4_restore_target._docker")
    def test_pg_observation_uses_verified_container_and_catalog_families(self, docker):
        facts = clean_facts()
        docker.return_value = json.dumps(facts) + "\n"
        self.assertEqual(probe_pg_facts(identity_for(ID), "a" * 64), facts)
        command = docker.call_args.args
        self.assertEqual(command[:4], ("exec", "--user", "postgres", "a" * 64))
        self.assertEqual(command[4:8], ("psql", "-XAt", "-v", "ON_ERROR_STOP=1"))
        self.assertIn(identity_for(ID)["database"], command)
        sql = command[-1]
        for catalog in ("pg_control_system", "pg_collation", "pg_default_acl",
                        "pg_auth_members", "nspacl", "pg_stat_activity"):
            self.assertIn(catalog, sql)
        docker.return_value = json.dumps({**facts, "other_sessions": 1}) + "\n"
        with self.assertRaises(AdmissionError):
            probe_pg_facts(identity_for(ID), "a" * 64)

    def test_issuer_reinspects_and_creates_three_disjoint_empty_roots(self):
        identity = identity_for(ID)
        before = empty_snapshot()
        after = created(identity)
        after["containers"][0]["State"]["StartedAt"] = "2026-09-29T00:00:00Z"
        ids = verify_created(identity, SUBNET, before, after)
        status = {"state": "CREATED_QUARANTINED", "batch_id": ID,
                  "project": identity["project"], "database": identity["database"],
                  "volume_name": identity["volume"]}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "targets" / ID
            target.mkdir(parents=True)
            def create_root(path):
                path.mkdir()
                return (42, {"destination": 90, "control": 100, "assets": 200}[path.name])
            def publish(control, name, payload, _nonce):
                if name.endswith(".birth.json"):
                    self.assertEqual(control, target / "control")
                    self.assertEqual(name, identity["database"] + ".birth.json")
                    self.assertEqual(json.loads(payload)["asset_ino"], 200)
                else:
                    self.assertEqual(control, target)
                    self.assertEqual(name, "issuance-success.json")
                    self.assertEqual(json.loads(payload)["birth_sha256"],
                                     hashlib.sha256((target / "control" /
                                         (identity["database"] + ".birth.json")).read_bytes()).hexdigest())
                (control / name).write_bytes(payload)
                return hashlib.sha256(payload).hexdigest()
            with (patch("p0c4_restore_target_birth._verify_creation_state"),
                  patch("p0c4_restore_target_birth._trusted_volume_mount",
                        return_value=(9, 10)),
                  patch("p0c4_restore_target.probe_initdb"),
                  patch("p0c4_restore_target_birth.probe_pg_facts",
                        return_value=clean_facts()),
                  patch("p0c4_restore_target_birth._new_private_root",
                        side_effect=create_root),
                  patch("p0c4_restore_target_birth._publish_birth", side_effect=publish),
                  patch("p0c4_restore_target_birth._read_published_birth",
                        side_effect=lambda path: path.read_bytes()),
                  patch("p0c4_restore_target._private_write",
                        side_effect=lambda path, data: path.write_bytes(data)),
                  patch("p0c4_restore_target.snapshot", return_value=after) as snapshot,
                  patch("p0c4_restore_target._inspect", return_value=after["images"]),
                  patch("p0c4_restore_target_birth.verify_birth_docker",
                        return_value=ids) as docker):
                result = issue_birth(root, target, identity, SUBNET, before,
                                     ids, status, Path("/reviewed/initdb.sh"))
            snapshot.assert_called_once_with()
            docker.assert_called_once()
            roots = [Path(result[name]) for name in
                     ("destination_root", "control_root", "asset_root")]
            self.assertEqual(len(set(roots)), 3)
            self.assertTrue(all(not any(path.iterdir()) for path in
                                (roots[0], roots[2])))
            evidence = json.loads((target / "birth-evidence.json").read_bytes())
            self.assertEqual(evidence["birth_sha256"], result["birth_sha256"])
            self.assertEqual(evidence["volume_mount_ino"], 10)
            self.assertEqual(evidence["container_started_at"],
                             "2026-09-29T00:00:00Z")
            seal = json.loads((target / "issuance-success.json").read_bytes())
            self.assertEqual(seal["birth_sha256"], result["birth_sha256"])
            self.assertEqual(seal["container_id"], ids["container_id"])

    def test_issuer_requires_start_time_on_verified_exact_id_before_publication(self):
        identity = identity_for(ID)
        before = empty_snapshot()
        after = created(identity)
        ids = verify_created(identity, SUBNET, before, after)
        del after["containers"][0]["State"]["StartedAt"]
        status = {"state": "CREATED_QUARANTINED", "batch_id": ID,
                  "project": identity["project"], "database": identity["database"],
                  "volume_name": identity["volume"]}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "targets" / ID
            target.mkdir(parents=True)
            with patch("p0c4_restore_target_birth._verify_creation_state"), \
                 patch("p0c4_restore_target.snapshot", return_value=after), \
                 patch("p0c4_restore_target._inspect", return_value=after["images"]), \
                 patch("p0c4_restore_target_birth.verify_birth_docker",
                       return_value=ids), \
                 patch("p0c4_restore_target_birth._trusted_volume_mount") as mount, \
                 patch("p0c4_restore_target_birth._publish_birth") as publish, \
                 patch("p0c4_restore_target_birth._write_failure_diagnostic"):
                with self.assertRaises(AdmissionError):
                    issue_birth(root, target, identity, SUBNET, before,
                                ids, status, Path("/reviewed/initdb.sh"))
            mount.assert_not_called()
            publish.assert_not_called()

    def test_publish_is_no_replace_and_sync_failure_removes_candidate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with (patch("p0c4_restore_target_birth._verify_birth_file"),
                  patch("p0c4_restore_target._sync_directory"),
                  patch("p0c4_restore_target._private_write",
                        side_effect=lambda path, data: path.write_bytes(data))):
                digest = _publish_birth(root, "new.birth.json", b'{"valid":true}', ID)
            self.assertEqual(digest, hashlib.sha256(b'{"valid":true}').hexdigest())
            self.assertEqual((root / "new.birth.json").read_bytes(), b'{"valid":true}')
            with (patch("p0c4_restore_target_birth._verify_birth_file"),
                  patch("p0c4_restore_target._sync_directory"),
                  patch("p0c4_restore_target._private_write",
                        side_effect=lambda path, data: path.write_bytes(data))):
                with self.assertRaises(AdmissionError):
                    _publish_birth(root, "new.birth.json", b"other", ID)
            self.assertEqual((root / "new.birth.json").read_bytes(), b'{"valid":true}')
            sync_calls = 0
            def fail_second(_path):
                nonlocal sync_calls
                sync_calls += 1
                if sync_calls == 1:
                    raise OSError("directory sync failed after publication")
            with (patch("p0c4_restore_target_birth._verify_birth_file"),
                  patch("p0c4_restore_target._sync_directory", side_effect=fail_second),
                  patch("p0c4_restore_target._private_write",
                        side_effect=lambda path, data: path.write_bytes(data))):
                with self.assertRaises(OSError):
                    _publish_birth(root, "failed.birth.json", b"never admitted", ID)
            self.assertFalse((root / "failed.birth.json").exists())

    def test_birth_hook_runs_under_creation_lock_and_failure_quarantines(self):
        identity = identity_for(ID)
        live = created(identity)
        stopped = copy.deepcopy(live)
        stopped["containers"][0]["State"]["Running"] = False
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "targets").mkdir()
            locked = False
            calls = []
            @contextlib.contextmanager
            def lock(_root):
                nonlocal locked
                locked = True
                try:
                    yield
                finally:
                    locked = False
            def issue(_root, target, _identity, _subnet, _before, _ids, status, _initdb):
                self.assertTrue(locked)
                self.assertEqual(json.loads((target / "state.json").read_text()), status)
                raise AdmissionError("issuer refused dirty target")
            with (patch("p0c4_restore_target._trusted_initdb"),
                  patch("p0c4_restore_target._locked_root", side_effect=lock),
                  patch("p0c4_restore_target._postgres_uid", return_value=(999, 999)),
                  patch("p0c4_restore_target._sync_directory"),
                  patch("p0c4_restore_target.os.chown", create=True),
                  patch("p0c4_restore_target.os.lstat", return_value=SimpleNamespace(
                      st_mode=stat.S_IFDIR | 0o700, st_uid=0)),
                  patch("p0c4_restore_target._private_write",
                        side_effect=lambda path, data: path.write_bytes(data)),
                  patch("p0c4_restore_target.snapshot",
                        side_effect=[empty_snapshot(), live, live, stopped]),
                  patch("p0c4_restore_target._inspect", return_value=live["images"]),
                  patch("p0c4_restore_target._docker",
                        side_effect=lambda *args: calls.append(args) or "OK\n")):
                with self.assertRaises(AdmissionError):
                    provision(root, ID, SUBNET, Path("/reviewed/initdb.sh"),
                              _birth_issuer=issue)
            self.assertFalse(locked)
            self.assertIn(("stop", "--time", "1", "a" * 64), calls)
            self.assertEqual(json.loads((root / "targets" / ID /
                                         "failure.json").read_text())["state"],
                             "FAILED_QUARANTINE_ATTEMPTED")

    def test_issuer_refuses_existing_failure_marker_before_inspection(self):
        identity = identity_for(ID)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "targets" / ID
            target.mkdir(parents=True)
            (target / "failure.json").write_text("{}")
            status = {"state": "CREATED_QUARANTINED", "batch_id": ID,
                      "project": identity["project"],
                      "database": identity["database"],
                      "volume_name": identity["volume"]}
            with patch("p0c4_restore_target.snapshot") as snapshot:
                with self.assertRaises(AdmissionError):
                    issue_birth(root, target, identity, SUBNET, empty_snapshot(),
                                {}, status, Path("/reviewed/initdb.sh"))
                snapshot.assert_not_called()

    def test_issuer_refuses_preexisting_private_root_before_inspection(self):
        identity = identity_for(ID)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "targets" / ID
            (target / "assets").mkdir(parents=True)
            status = {"state": "CREATED_QUARANTINED", "batch_id": ID,
                      "project": identity["project"],
                      "database": identity["database"],
                      "volume_name": identity["volume"]}
            with (patch("p0c4_restore_target_birth._verify_creation_state"),
                  patch("p0c4_restore_target.snapshot") as snapshot):
                with self.assertRaises(AdmissionError):
                    issue_birth(root, target, identity, SUBNET, empty_snapshot(),
                                {}, status, Path("/reviewed/initdb.sh"))
                snapshot.assert_not_called()

    def test_python_birth_bytes_match_committed_rust_fixture(self):
        identity = identity_for(ID)
        facts = clean_facts()
        validate_pg_facts(facts)
        payload = canonical_birth(identity, facts, (42, 100), (43, 200),
                                  "550e8400-e29b-41d4-a716-446655440001")
        fixture = (Path(__file__).resolve().parent.parent / "crates" /
                   "learning-backup" / "tests" / "fixtures" /
                   "c4_birth_python.json").read_bytes()
        self.assertEqual(payload, fixture)
        self.assertTrue(payload.startswith(b'{"format_version":1'))
        self.assertNotIn(b"\n", payload)
        self.assertEqual(hashlib.sha256(payload).hexdigest(),
                         hashlib.sha256(fixture).hexdigest())

    def test_dirty_or_mutated_acl_never_becomes_a_birth_baseline(self):
        facts = clean_facts()
        for mutation in (
            lambda f: f["dirty_counts"].__setitem__("collations", 1),
            lambda f: f["dirty_counts"].__setitem__("default_acls", 1),
            lambda f: f["public_schema"].update(owner_name="learning_runtime"),
            lambda f: f["public_schema"]["acl"].append(
                {"grantor_oid": 6171, "grantee_oid": 0,
                 "privilege": "CREATE", "grantable": False}),
            lambda f: f.update(other_sessions=1),
            lambda f: f.update(public_database_grants=1),
            lambda f: f.update(runtime_can_use_database=True),
            lambda f: f.update(role_flags_secure=False),
            lambda f: f.update(runtime_can_create_public=True),
            lambda f: f.update(pg_system_identifier="00042"),
        ):
            with self.subTest(mutation=mutation):
                changed = copy.deepcopy(facts)
                mutation(changed)
                with self.assertRaises(AdmissionError):
                    validate_pg_facts(changed)

    def test_birth_requires_current_immutable_docker_mount_and_image(self):
        identity = identity_for(ID)
        before = empty_snapshot()
        after = created(identity)
        target = Path("/var/lib/knowweave-c4/targets") / ID
        initdb = Path("/var/lib/knowweave-c4/tools/initdb.sh")
        pg = after["containers"][0]
        pg["Mounts"].extend([
            {"Type": "bind", "Source": str(initdb),
             "Destination": "/docker-entrypoint-initdb.d/10-restore.sh", "RW": False},
            {"Type": "bind", "Source": str(target / "secrets" / "postgres_password"),
             "Destination": "/run/secrets/postgres_password", "RW": False},
            {"Type": "bind", "Source": str(target / "secrets" / "admin_password"),
             "Destination": "/run/secrets/admin_password", "RW": False},
        ])
        after["volumes"][0].update(Driver="local", Scope="local", Options=None)
        ids = verify_created(identity, SUBNET, before, after)
        verify_birth_docker(identity, SUBNET, before, after, ids, target, initdb)
        for mutation in (
            lambda s: s["containers"][0].update(Id="c" * 64),
            lambda s: s["volumes"][0].update(Driver="remote"),
            lambda s: s["containers"][0]["Mounts"][1].update(Source="/old/initdb.sh"),
            lambda s: s["containers"][0]["Mounts"].append(
                {"Type": "bind", "Source": "/old", "Destination": "/other", "RW": True}),
        ):
            changed = copy.deepcopy(after)
            mutation(changed)
            with self.subTest(mutation=mutation), self.assertRaises(AdmissionError):
                verify_birth_docker(identity, SUBNET, before, changed, ids, target, initdb)


if __name__ == "__main__":
    unittest.main()
