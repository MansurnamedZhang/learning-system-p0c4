"""Mock-only gates for the independent C4 birth issuer acceptance runner."""

import copy
from contextlib import ExitStack
import hashlib
import json
from pathlib import Path
import stat
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch

import p0c4_restore_birth_acceptance as runner
import p0c4_restore_target_birth as issuer
from p0c4_restore_target import identity_for
from test_p0c4_restore_target import ID, SUBNET, created, empty_snapshot


def records():
    identity = identity_for(ID)
    birth = {
        "format_version": 1, "project_name": identity["project"],
        "pg_volume_name": identity["volume"],
        "database_name": identity["database"],
        "database_oid": 16385,
        "pg_system_identifier": "7361082129910479001",
        "control_dev": 42, "control_ino": 100,
        "asset_dev": 43, "asset_ino": 200,
        "creation_nonce": "550e8400-e29b-41d4-a716-446655440001",
        "template_database": "template0", "baseline_cast_count": 203,
        "public_schema": {"owner_oid": 6171, "owner_name": "pg_database_owner",
                          "acl_is_null": False,
                          "acl": [{"grantor_oid": 6171, "grantee_oid": 0,
                                   "privilege": "USAGE", "grantable": False},
                                  {"grantor_oid": 6171, "grantee_oid": 6171,
                                   "privilege": "CREATE", "grantable": False},
                                  {"grantor_oid": 6171, "grantee_oid": 6171,
                                   "privilege": "USAGE", "grantable": False}]}}
    birth_bytes = json.dumps(birth, separators=(",", ":")).encode()
    digest = hashlib.sha256(birth_bytes).hexdigest()
    status = {"state": "CREATED_QUARANTINED", "batch_id": ID,
              "project": identity["project"], "database": identity["database"],
              "network": identity["network"], "subnet": SUBNET,
              "container_id": "a" * 64, "network_id": "b" * 64,
              "volume_name": identity["volume"],
              "volume_mountpoint": "/var/lib/docker/volumes/new/_data"}
    success = {"format_version": 1,
               "state": "BIRTH_ISSUED_NOT_RESTORE_ACCEPTANCE",
               "batch_id": ID, "project_name": identity["project"],
               "database_name": identity["database"], "birth_sha256": digest,
               "container_id": "a" * 64, "network_id": "b" * 64,
               "pg_volume_name": identity["volume"],
               "image_id": "sha256:" + "d" * 64,
               "volume_mountpoint": status["volume_mountpoint"],
               "volume_mount_dev": 9, "volume_mount_ino": 10}
    evidence = {"birth_sha256": digest, "container_id": "a" * 64,
                "network_id": "b" * 64, "volume_name": identity["volume"],
                "image_id": "sha256:" + "d" * 64,
                "volume_mountpoint": status["volume_mountpoint"],
                "volume_mount_dev": 9, "volume_mount_ino": 10}
    return identity, birth_bytes, status, success, evidence


class CandidateGates(unittest.TestCase):
    def test_sanitized_issuer_diagnostic_or_unavailable_only(self):
        self.assertEqual(runner.ISSUER_FAILURE_REASONS,
                         issuer.ISSUER_FAILURE_REASONS)
        identity = identity_for(ID)
        target = Path("/private/targets") / ID
        safe = {"format_version": 1,
                "state": "BIRTH_ISSUER_DIAGNOSTIC_NOT_ACCEPTANCE",
                "batch_id": ID, "project": identity["project"],
                "phase": "PG_SQL_PARSE",
                "reason_code": "PG_SQL_PARSE_FAILED",
                "exception_class": "AdmissionError"}
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner, "_private_read",
                          return_value=json.dumps(safe).encode()):
            self.assertEqual(runner.read_issuer_diagnostic(target, identity, ID),
                             safe)
        changed = {**safe, "reason_code": "postgres_password=secret"}
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner, "_private_read",
                          return_value=json.dumps(changed).encode()):
            self.assertEqual(runner.read_issuer_diagnostic(target, identity, ID),
                             {"status": "UNAVAILABLE"})
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner, "_private_read",
                          return_value=b'{"phase":"postgres_password=secret"'):
            self.assertEqual(runner.read_issuer_diagnostic(target, identity, ID),
                             {"status": "UNAVAILABLE"})
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner, "_private_read",
                          side_effect=FileNotFoundError):
            self.assertEqual(runner.read_issuer_diagnostic(target, identity, ID),
                             {"status": "UNAVAILABLE"})
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner, "_private_read",
                          side_effect=KeyboardInterrupt):
            self.assertEqual(runner.read_issuer_diagnostic(target, identity, ID),
                             {"status": "UNAVAILABLE"})

    def test_private_candidate_rejects_failure_marker_and_missing_success_seal(self):
        identity, birth_bytes, state, success, evidence = records()
        with tempfile.TemporaryDirectory() as temp:
            target = Path(temp) / ID
            for name in ("destination", "control", "assets"):
                (target / name).mkdir(parents=True, exist_ok=True)
            birth_name = identity["database"] + ".birth.json"
            (target / "control" / birth_name).write_bytes(birth_bytes)
            (target / "failure.json").write_text("{}")
            private = SimpleNamespace(st_mode=stat.S_IFDIR | 0o700,
                                      st_dev=44, st_ino=300)
            with patch.object(runner, "_trusted_path"), \
                 patch.object(runner.os, "lstat", return_value=private):
                with self.assertRaisesRegex(ValueError, "failure evidence"):
                    runner.read_candidate(target, identity, ID)
            (target / "failure.json").unlink()
            def fake_lstat(path):
                if Path(path).name == "control":
                    return SimpleNamespace(st_mode=stat.S_IFDIR | 0o700,
                                           st_dev=42, st_ino=100)
                if Path(path).name == "assets":
                    return SimpleNamespace(st_mode=stat.S_IFDIR | 0o700,
                                           st_dev=43, st_ino=200)
                return SimpleNamespace(st_mode=stat.S_IFDIR | 0o700,
                                       st_dev=44, st_ino=300)
            payloads = {
                birth_name: birth_bytes,
                "state.json": json.dumps(state).encode(),
                "birth-evidence.json": json.dumps(evidence).encode(),
            }
            def fake_read(path):
                return payloads[Path(path).name]
            with patch.object(runner, "_trusted_path"), \
                 patch.object(runner.os, "lstat", side_effect=fake_lstat), \
                 patch.object(runner.os.path, "lexists", return_value=False), \
                 patch.object(runner, "_private_read", side_effect=fake_read):
                with self.assertRaises(KeyError):
                    runner.read_candidate(target, identity, ID)

    def test_direct_pg_observation_is_required_and_dirty_acl_rejects(self):
        identity, birth_bytes, _, _, _ = records()
        birth = json.loads(birth_bytes)
        observed = {
            "server_version_num": 180006, "database_oid": birth["database_oid"],
            "pg_system_identifier": birth["pg_system_identifier"],
            "cast_count": birth["baseline_cast_count"],
            "database_owner": "learning_admin", "runtime_create": False,
            "other_sessions": 0, "user_relations": 0,
            "public_collations": 0, "default_acls": 0,
            "public_schema": birth["public_schema"],
        }
        docker = MagicMock()
        docker._docker.return_value = json.dumps(observed) + "\n"
        facts = runner.observe_pg(docker, identity, "a" * 64)
        self.assertEqual(facts, observed)
        self.assertIn("pg_catalog.pg_control_system()", docker._docker.call_args.args[-1])
        issuer_facts = {"database_oid": birth["database_oid"],
                        "pg_system_identifier": birth["pg_system_identifier"],
                        "cast_count": birth["baseline_cast_count"],
                        "public_schema": birth["public_schema"]}
        runner.validate_pg_against_birth(facts, birth, issuer_facts)
        changed = {**facts, "runtime_create": True}
        with self.assertRaises(ValueError):
            runner.validate_pg_against_birth(changed, birth, issuer_facts)

    def test_exit_zero_is_required_before_any_candidate_read(self):
        process = SimpleNamespace(returncode=2, stdout="{}\n", stderr="sensitive")
        with patch.object(runner, "read_candidate") as read:
            with self.assertRaises(ValueError):
                runner.accept_issuer_process(process, Path("/private/target"),
                                             identity_for(ID), ID)
            read.assert_not_called()
        process.returncode = 0
        process.stdout = '{"state":"BIRTH_ISSUED_NOT_RESTORE_ACCEPTANCE"}\n'
        with patch.object(runner, "read_candidate", side_effect=KeyboardInterrupt) as read:
            with self.assertRaises(KeyboardInterrupt):
                runner.accept_issuer_process(process, Path("/private/target"),
                                             identity_for(ID), ID)
            read.assert_called_once()

    def test_missing_seal_failure_cross_batch_and_wrong_hash_reject(self):
        identity, birth_bytes, state, success, evidence = records()
        runner.validate_candidate_records(identity, ID, birth_bytes, state, success, evidence)
        for mutate in (
            lambda b, s, c, e: c.clear(),
            lambda b, s, c, e: c.update(birth_sha256="0" * 64),
            lambda b, s, c, e: s.update(batch_id="550e8400-e29b-41d4-a716-446655440002"),
            lambda b, s, c, e: e.update(container_id="c" * 64),
        ):
            with self.subTest(mutate=mutate):
                b, s, c, e = copy.deepcopy((birth_bytes, state, success, evidence))
                mutate(b, s, c, e)
                with self.assertRaises(ValueError):
                    runner.validate_candidate_records(identity, ID, b, s, c, e)

    def test_reordered_birth_and_boolean_version_reject_even_with_updated_digest(self):
        identity, birth_bytes, state, success, evidence = records()
        original = json.loads(birth_bytes)
        reordered = {"project_name": original["project_name"],
                     **{key: value for key, value in original.items()
                        if key != "project_name"}}
        for candidate in (reordered, {**original, "format_version": True}):
            payload = json.dumps(candidate, separators=(",", ":")).encode()
            stamped = hashlib.sha256(payload).hexdigest()
            seal = {**success, "birth_sha256": stamped}
            proof = {**evidence, "birth_sha256": stamped}
            with self.subTest(candidate=candidate):
                with self.assertRaises(ValueError):
                    runner.validate_candidate_records(identity, ID, payload,
                                                      state, seal, proof)

    def test_reordered_public_schema_and_acl_reject_with_updated_digest(self):
        identity, birth_bytes, state, success, evidence = records()
        for section in ("public", "acl"):
            candidate = json.loads(birth_bytes)
            if section == "public":
                candidate["public_schema"] = {
                    "acl": candidate["public_schema"]["acl"],
                    **{key: value for key, value in
                       candidate["public_schema"].items() if key != "acl"}}
            else:
                first = candidate["public_schema"]["acl"][0]
                candidate["public_schema"]["acl"][0] = {
                    "privilege": first["privilege"],
                    **{key: value for key, value in first.items()
                       if key != "privilege"}}
            payload = json.dumps(candidate, separators=(",", ":")).encode()
            stamped = hashlib.sha256(payload).hexdigest()
            with self.subTest(section=section):
                with self.assertRaises(ValueError):
                    runner.validate_candidate_records(identity, ID, payload,
                                                      state, {**success, "birth_sha256": stamped},
                                                      {**evidence, "birth_sha256": stamped})

    def test_birth_nested_boolean_type_drift_rejects(self):
        identity, birth_bytes, state, success, evidence = records()
        for field in ("owner_oid", "grantor_oid", "grantable"):
            candidate = json.loads(birth_bytes)
            if field == "owner_oid":
                candidate["public_schema"]["owner_oid"] = True
            else:
                candidate["public_schema"]["acl"][0][field] = (
                    1 if field == "grantable" else True)
            payload = json.dumps(candidate, separators=(",", ":")).encode()
            stamped = hashlib.sha256(payload).hexdigest()
            with self.subTest(field=field):
                with self.assertRaises(ValueError):
                    runner.validate_candidate_records(
                        identity, ID, payload, state,
                        {**success, "birth_sha256": stamped},
                        {**evidence, "birth_sha256": stamped})

    def test_live_docker_reinspection_rejects_changed_ids_and_extra_mount(self):
        identity = identity_for(ID)
        live = created(identity)
        target = Path("/private/targets") / ID
        initdb = Path("/private/initdb.sh")
        for destination, source in (
            ("/docker-entrypoint-initdb.d/10-restore.sh", initdb),
            ("/run/secrets/postgres_password", target / "secrets/postgres_password"),
            ("/run/secrets/admin_password", target / "secrets/admin_password"),
        ):
            live["containers"][0]["Mounts"].append(
                {"Type": "bind", "Source": str(source), "RW": False,
                 "Destination": destination})
        live["volumes"][0].update(Driver="local", Scope="local", Options=None)
        expected = {"container_id": "a" * 64, "network_id": "b" * 64,
                    "volume_name": identity["volume"],
                    "volume_mountpoint": "/var/lib/docker/volumes/new/_data"}
        runner.validate_live_docker(identity, SUBNET, empty_snapshot(), live,
                                    expected, target, initdb)
        changed = copy.deepcopy(live)
        changed["containers"][0]["Id"] = "c" * 64
        with self.assertRaises(ValueError):
            runner.validate_live_docker(identity, SUBNET, empty_snapshot(),
                                        changed, expected, target, initdb)
        changed = copy.deepcopy(live)
        changed["containers"][0]["Mounts"].append(
            {"Type": "bind", "Source": "/other", "RW": True,
             "Destination": "/other"})
        with self.assertRaises(ValueError):
            runner.validate_live_docker(identity, SUBNET, empty_snapshot(),
                                        changed, expected, target, initdb)
        changed = copy.deepcopy(live)
        changed["images"][0]["RepoDigests"] = [
            "unrelated-prefix-" + identity["image"].split("@", 1)[1] + "-suffix"]
        with self.assertRaises(ValueError):
            runner.validate_live_docker(identity, SUBNET, empty_snapshot(),
                                        changed, expected, target, initdb)

    def test_cleanup_failure_never_credits_stop(self):
        identity = identity_for(ID)
        live = created(identity)
        stopped = copy.deepcopy(live)
        stopped["containers"][0]["State"]["Running"] = False
        snapshots = iter([live, stopped])
        docker = SimpleNamespace(snapshot=lambda: next(snapshots),
                                 _docker=lambda *_: None)
        self.assertTrue(runner.stop_verified_pg(docker, identity, "a" * 64)["confirmed"])
        docker.snapshot = lambda: live
        docker._docker = lambda *_: (_ for _ in ()).throw(RuntimeError("stop failed"))
        with self.assertRaises(RuntimeError):
            runner.stop_verified_pg(docker, identity, "a" * 64)

    def test_missing_seal_early_stop_requires_unique_owned_live_container(self):
        identity = identity_for(ID)
        target = Path("/private/targets") / ID
        initdb = Path("/private/initdb.sh")
        before = empty_snapshot()
        state = records()[2]
        live = created(identity)
        for destination, source in (
            ("/docker-entrypoint-initdb.d/10-restore.sh", initdb),
            ("/run/secrets/postgres_password", target / "secrets/postgres_password"),
            ("/run/secrets/admin_password", target / "secrets/admin_password"),
        ):
            live["containers"][0]["Mounts"].append(
                {"Type": "bind", "Source": str(source), "RW": False,
                 "Destination": destination})
        live["volumes"][0].update(Driver="local", Scope="local", Options=None)
        docker = SimpleNamespace(snapshot=lambda: live,
                                 _inspect=lambda *_: live["images"],
                                 admit_fresh=lambda *_: None)
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner.os.path, "lexists", return_value=True), \
             patch.object(runner, "_private_read",
                          return_value=json.dumps(state).encode()), \
             patch.object(runner, "stop_verified_pg",
                          return_value={"confirmed": True}) as stop:
            self.assertTrue(runner.stop_early_owned_pg(
                docker, identity, target, SUBNET, before, initdb)["confirmed"])
            stop.assert_called_once_with(docker, identity, "a" * 64)
        changed = copy.deepcopy(live)
        changed["containers"][0]["Mounts"][0]["Name"] = "foreign_volume"
        docker.snapshot = lambda: changed
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner.os.path, "lexists", return_value=True), \
             patch.object(runner, "_private_read",
                          return_value=json.dumps(state).encode()), \
             patch.object(runner, "stop_verified_pg") as stop:
            with self.assertRaises(ValueError):
                runner.stop_early_owned_pg(
                    docker, identity, target, SUBNET, before, initdb)
            stop.assert_not_called()
        changed = copy.deepcopy(live)
        changed["containers"].append(copy.deepcopy(changed["containers"][0]))
        docker.snapshot = lambda: changed
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner.os.path, "lexists", return_value=True), \
             patch.object(runner, "_private_read",
                          return_value=json.dumps(state).encode()), \
             patch.object(runner, "stop_verified_pg") as stop:
            with self.assertRaises(ValueError):
                runner.stop_early_owned_pg(
                    docker, identity, target, SUBNET, before, initdb)
            stop.assert_not_called()

    def test_already_exited_exact_pg_is_confirmed_without_new_stop(self):
        identity = identity_for(ID)
        target = Path("/private/targets") / ID
        initdb = Path("/private/initdb.sh")
        state = records()[2]
        exited = created(identity)
        exited["containers"][0]["State"]["Running"] = False
        exited["containers"][0]["State"]["Status"] = "exited"
        for destination, source in (
            ("/docker-entrypoint-initdb.d/10-restore.sh", initdb),
            ("/run/secrets/postgres_password", target / "secrets/postgres_password"),
            ("/run/secrets/admin_password", target / "secrets/admin_password"),
        ):
            exited["containers"][0]["Mounts"].append(
                {"Type": "bind", "Source": str(source), "RW": False,
                 "Destination": destination})
        exited["volumes"][0].update(Driver="local", Scope="local", Options=None)
        docker = MagicMock()
        docker.snapshot.side_effect = [exited, exited, exited]
        docker._inspect.return_value = exited["images"]
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner.os.path, "lexists", return_value=True), \
             patch.object(runner, "_private_read",
                          return_value=json.dumps(state).encode()):
            result = runner.stop_early_owned_pg(
                docker, identity, target, SUBNET, empty_snapshot(), initdb)
        self.assertTrue(result["confirmed"])
        self.assertTrue(result["volume_retained"])
        self.assertEqual(result["container_id"], "a" * 64)
        docker._docker.assert_not_called()
        wrong_state = {**state, "container_id": "c" * 64}
        docker.snapshot.side_effect = [exited]
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner.os.path, "lexists", return_value=True), \
             patch.object(runner, "_private_read",
                          return_value=json.dumps(wrong_state).encode()):
            with self.assertRaises(ValueError):
                runner.stop_early_owned_pg(
                    docker, identity, target, SUBNET, empty_snapshot(), initdb)
        docker._docker.assert_not_called()

    def test_timeout_without_state_stops_only_new_isolated_project(self):
        identity = identity_for(ID)
        target = Path("/private/targets") / ID
        initdb = Path("/private/initdb.sh")
        before = empty_snapshot()
        live = created(identity)
        live["containers"][0]["State"]["Health"]["Status"] = "starting"
        for destination, source in (
            ("/docker-entrypoint-initdb.d/10-restore.sh", initdb),
            ("/run/secrets/postgres_password", target / "secrets/postgres_password"),
            ("/run/secrets/admin_password", target / "secrets/admin_password"),
        ):
            live["containers"][0]["Mounts"].append(
                {"Type": "bind", "Source": str(source), "RW": False,
                 "Destination": destination})
        live["volumes"][0].update(Driver="local", Scope="local", Options=None)
        external = copy.deepcopy(live["containers"][0])
        external["Id"] = "e" * 64
        external["Config"]["Labels"]["com.docker.compose.project"] = "other-project"
        live["containers"].append(external)
        stopped = copy.deepcopy(live)
        stopped["containers"][0]["State"]["Running"] = False
        actual = MagicMock()
        actual.snapshot.side_effect = [live, live, stopped]
        actual._inspect.return_value = live["images"]
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner.os.path, "lexists", return_value=False):
            result = runner.stop_early_owned_pg(
                actual, identity, target, SUBNET, before, initdb)
        self.assertTrue(result["confirmed"])
        self.assertFalse(stopped["containers"][0]["State"]["Running"])
        self.assertTrue(stopped["containers"][1]["State"]["Running"])
        actual._docker.assert_called_once_with("stop", "--time", "1", "a" * 64)
        docker = MagicMock()
        docker.snapshot.return_value = live
        docker._inspect.return_value = live["images"]
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner.os.path, "lexists", return_value=False), \
             patch.object(runner, "stop_verified_pg",
                          return_value={"confirmed": True}) as stop:
            result = runner.stop_early_owned_pg(
                docker, identity, target, SUBNET, before, initdb)
            self.assertEqual(result["basis"], "fresh-snapshot-no-state")
            stop.assert_called_once_with(docker, identity, "a" * 64)
        changed = copy.deepcopy(live)
        changed["containers"][0]["Config"]["Image"] = "postgres:unreviewed"
        docker.snapshot.return_value = changed
        with patch.object(runner, "_require_private_dir"), \
             patch.object(runner.os.path, "lexists", return_value=False), \
             patch.object(runner, "stop_verified_pg") as stop:
            with self.assertRaises(ValueError):
                runner.stop_early_owned_pg(
                    docker, identity, target, SUBNET, before, initdb)
            stop.assert_not_called()
        for change in ("daemon", "network", "port", "mount", "image-digest",
                       "preexisting-id"):
            changed = copy.deepcopy(live)
            prior = copy.deepcopy(before)
            if change == "daemon":
                changed["daemon_id"] = "different-daemon"
            elif change == "network":
                changed["networks"][0]["Internal"] = False
            elif change == "port":
                changed["containers"][0]["NetworkSettings"]["Ports"] = {
                    "5432/tcp": [{"HostPort": "5432"}]}
            elif change == "mount":
                changed["containers"][0]["Mounts"][0]["Source"] = "/foreign"
            elif change == "image-digest":
                changed["images"][0]["RepoDigests"] = [
                    "postgres@sha256:" + "0" * 64]
            else:
                prior["containers"].append({"Id": "a" * 64})
            docker.snapshot.return_value = changed
            docker._inspect.return_value = changed["images"]
            with self.subTest(change=change), \
                 patch.object(runner, "_require_private_dir"), \
                 patch.object(runner.os.path, "lexists", return_value=False), \
                 patch.object(runner, "stop_verified_pg") as stop:
                with self.assertRaises(ValueError):
                    runner.stop_early_owned_pg(
                        docker, identity, target, SUBNET, prior, initdb)
                stop.assert_not_called()

    def test_issuer_exit_failure_missing_seal_id_change_interrupt_and_cleanup(self):
        identity, birth_bytes, state, success, evidence = records()
        birth = json.loads(birth_bytes)
        manifest = {"files": [{"path": runner.INITDB, "sha256": "f" * 64}]}
        for failure in ("pass", "issuer-exit", "diagnostic-hostile",
                        "timeout-no-state",
                        "missing-seal", "failure-marker",
                        "id-change", "pg-failure", "interrupted-before-exit",
                        "cleanup-failed", "early-cleanup-failed"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temp:
                batch = Path(temp)
                (batch / "source" / "deploy").mkdir(parents=True)
                (batch / "source" / runner.INITDB).write_bytes(b"initdb")
                (batch / "evidence").mkdir()
                args = SimpleNamespace(batch_id=ID, subnet=SUBNET,
                                       archive_sha256="a" * 64,
                                       manifest_sha256="b" * 64,
                                       source_commit="c" * 40)
                provisioner = MagicMock()
                provisioner.identity_for.return_value = identity
                provisioner.snapshot.return_value = {"daemon_id": "daemon"}
                issuer = MagicMock()
                observed = []
                with ExitStack() as stack:
                    stack.enter_context(patch.object(runner, "extract_verified",
                                                     return_value="before"))
                    stack.enter_context(patch.object(runner, "_private_write"))
                    stack.enter_context(patch.object(runner, "_file_digest",
                                                     return_value="f" * 64))
                    stack.enter_context(patch.object(runner, "_load_reviewed",
                                                     return_value=(provisioner, issuer)))
                    stack.enter_context(patch.object(runner, "source_digest",
                                                     return_value="before"))
                    stack.enter_context(patch.object(runner, "_private_read",
                                                     return_value=b"seal"))
                    stack.enter_context(patch.object(runner, "_write_result",
                                                     side_effect=lambda _, result:
                                                     observed.append(copy.deepcopy(result))))
                    run_issuer = stack.enter_context(patch.object(
                        runner, "_run_issuer", return_value=SimpleNamespace(
                            returncode=2 if failure == "issuer-exit" else 0,
                            stdout="{}\n", stderr="sensitive")))
                    accept = stack.enter_context(patch.object(
                        runner, "accept_issuer_process",
                        return_value=(birth, state, success)))
                    live = stack.enter_context(patch.object(
                        runner, "_independent_docker_gate", return_value={
                            "container_id": "a" * 64, "network_id": "b" * 64}))
                    pg = stack.enter_context(patch.object(
                        runner, "_independent_pg_gate", return_value={
                            "pg_observation_sha256": "c" * 64,
                            "issuer_pg_observation_sha256": "d" * 64}))
                    negative = stack.enter_context(patch.object(
                        runner, "_negative_acl", return_value={"issuer_probe_rejected": True}))
                    stop = stack.enter_context(patch.object(
                        runner, "stop_verified_pg", return_value={"confirmed": True}))
                    early_stop = stack.enter_context(patch.object(
                        runner, "stop_early_owned_pg",
                        return_value={"confirmed": True}))
                    diagnostic = stack.enter_context(patch.object(
                        runner, "read_issuer_diagnostic",
                        return_value={"status": "UNAVAILABLE"}))
                    stack.enter_context(patch.object(runner, "validate_live_docker"))
                    if failure == "issuer-exit":
                        accept.side_effect = ValueError("issuer exit nonzero")
                        diagnostic.return_value = {
                            "format_version": 1,
                            "state": "BIRTH_ISSUER_DIAGNOSTIC_NOT_ACCEPTANCE",
                            "batch_id": ID, "project": identity["project"],
                            "phase": "PG_SQL_PARSE",
                            "reason_code": "PG_SQL_PARSE_FAILED",
                            "exception_class": "AdmissionError"}
                    elif failure == "missing-seal":
                        accept.side_effect = FileNotFoundError("seal")
                    elif failure == "diagnostic-hostile":
                        accept.side_effect = FileNotFoundError("seal")
                        diagnostic.side_effect = RuntimeError(
                            "postgres_password=hidden")
                    elif failure == "early-cleanup-failed":
                        accept.side_effect = FileNotFoundError("seal")
                        early_stop.side_effect = RuntimeError("stop failed")
                    elif failure == "failure-marker":
                        accept.side_effect = ValueError("failure.json")
                    elif failure == "id-change":
                        live.side_effect = ValueError("Docker ID changed")
                    elif failure == "pg-failure":
                        pg.side_effect = ValueError("PG identity changed")
                    elif failure == "interrupted-before-exit":
                        run_issuer.side_effect = KeyboardInterrupt()
                    elif failure == "timeout-no-state":
                        run_issuer.side_effect = TimeoutError()
                    elif failure == "cleanup-failed":
                        negative.side_effect = ValueError("negative probe failed")
                        stop.side_effect = RuntimeError("stop failed")
                    with patch("builtins.print"):
                        self.assertEqual(runner._run_batch(args, manifest, b"zip", batch),
                                         0 if failure == "pass" else 1)
                if failure == "pass":
                    self.assertTrue(observed[0]["status"].startswith(
                        "BIRTH_ISSUER_SINGLE_HOST_PG18_PASSED"))
                    self.assertEqual(observed[0]["target_condition"],
                                     "DIRTY_UNUSABLE_NOT_RESTORE_NOT_PIN")
                    stop.assert_called_once()
                    continue
                self.assertEqual(observed[0]["status"], "FAILED_NOT_RESTORE_NOT_PIN")
                self.assertNotIn("sensitive", json.dumps(observed[0]))
                self.assertIn("issuer_diagnostic", observed[0])
                if failure == "issuer-exit":
                    self.assertEqual(observed[0]["issuer_diagnostic"]["phase"],
                                     "PG_SQL_PARSE")
                else:
                    self.assertEqual(observed[0]["issuer_diagnostic"],
                                     {"status": "UNAVAILABLE"})
                if failure == "cleanup-failed":
                    self.assertEqual(observed[0]["target_condition"],
                                     "DIRTY_UNUSABLE_NOT_RESTORE_NOT_PIN")
                    self.assertEqual(observed[0]["stop"]["failure_type"],
                                     "RuntimeError")
                    stop.assert_called_once()
                elif failure == "pg-failure":
                    self.assertEqual(observed[0]["stage"], "independent-pg18")
                    stop.assert_called_once()
                elif failure == "early-cleanup-failed":
                    early_stop.assert_called_once()
                    self.assertFalse(observed[0]["stop"]["confirmed"])
                    self.assertTrue(observed[0]["stop"]["early_isolation_unconfirmed"])
                    stop.assert_not_called()
                elif failure in ("issuer-exit", "missing-seal",
                                 "failure-marker", "id-change",
                                 "interrupted-before-exit", "timeout-no-state",
                                 "diagnostic-hostile"):
                    early_stop.assert_called_once()
                    self.assertTrue(observed[0]["stop"]["confirmed"])
                    stop.assert_not_called()
                else:
                    stop.assert_not_called()


if __name__ == "__main__":
    unittest.main()
