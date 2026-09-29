"""Local mock gates for the isolated C4 restore-target provisioner."""

import copy
import contextlib
import json
import stat
import tempfile
from types import SimpleNamespace
import unittest
from pathlib import Path
from unittest.mock import patch

from p0c4_restore_target import (AdmissionError, admit_fresh, compose_document,
                                 identity_for, verify_created, quarantine, provision,
                                 _postgres_uid, probe_initdb)


ID = "550e8400-e29b-41d4-a716-446655440000"
SUBNET = "10.251.219.0/24"


def empty_snapshot():
    return {"containers": [], "networks": [], "volumes": [],
            "routes": ["192.168.0.0/16"], "daemon_id": "daemon-1"}


def created(identity):
    project = identity["project"]
    network = identity["network"]
    volume = identity["volume"]
    return {
        "containers": [{"Id": "a" * 64, "Name": "/" + project + "-pg-1",
                        "Config": {"Image": identity["image"], "Labels": {
                            "com.docker.compose.project": project,
                            "com.docker.compose.service": "pg"}},
                        "Image": "sha256:" + "d" * 64,
                        "State": {"Running": True,
                                  "StartedAt": "2026-09-29T00:00:00Z",
                                  "Health": {"Status": "healthy"}},
                        "HostConfig": {"NetworkMode": network, "PortBindings": {}},
                        "NetworkSettings": {"Ports": {}, "Networks": {network: {"NetworkID": "b" * 64}}},
                        "Mounts": [{"Type": "volume", "Name": volume,
                                    "Source": "/var/lib/docker/volumes/new/_data",
                                    "RW": True, "Destination": "/var/lib/postgresql"}]}],
        "networks": [{"Id": "b" * 64, "Name": network, "Internal": True,
                      "Labels": {"com.docker.compose.project": project},
                      "IPAM": {"Config": [{"Subnet": SUBNET, "Gateway": "10.251.219.1"}]} }],
        "volumes": [{"Name": volume, "Mountpoint": "/var/lib/docker/volumes/new/_data",
                     "Labels": {"com.docker.compose.project": project}}],
        "images": [{"Id": "sha256:" + "d" * 64,
                    "RepoDigests": ["postgres@" + identity["image"].split("@", 1)[1]]}],
        "daemon_id": "daemon-1",
    }


class RestoreTargetGates(unittest.TestCase):
    def test_initdb_uses_template0_and_scopes_function_grant_to_target_db(self):
        script = (Path(__file__).resolve().parent.parent / "deploy" /
                  "p0c4_restore_initdb.sh").read_bytes()
        self.assertNotIn(b"\r", script)
        self.assertIn(b'CREATE DATABASE :"dbname" OWNER learning_admin TEMPLATE template0;', script)
        target_block = script.split(b'--dbname "$C4_TARGET_DATABASE"', 1)[1]
        self.assertIn(b"REVOKE ALL ON FUNCTION pg_catalog.pg_control_system() FROM PUBLIC;", target_block)
        self.assertIn(b"GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO learning_admin;", target_block)

    def test_admission_requires_uuid_absence_and_unused_explicit_subnet(self):
        identity = identity_for(ID)
        admit_fresh(identity, SUBNET, empty_snapshot())
        for field, object_name in (("containers", identity["project"]),
                                   ("networks", identity["network"]),
                                   ("volumes", identity["volume"])):
            with self.subTest(field=field):
                snapshot = empty_snapshot()
                snapshot[field] = [{"Name": object_name, "Labels": {}}]
                with self.assertRaises(AdmissionError):
                    admit_fresh(identity, SUBNET, snapshot)
        for bad in ("192.168.1.0/24", "127.0.0.0/24", "0.0.0.0/0",
                    "198.18.0.0/24", "203.0.113.0/24", "not-a-subnet"):
            with self.subTest(subnet=bad), self.assertRaises(AdmissionError):
                admit_fresh(identity, bad, empty_snapshot())
        snapshot = empty_snapshot()
        snapshot["networks"] = [{"Name": "unrelated", "Labels": {},
                                 "IPAM": {"Config": [{"Subnet": "10.251.219.0/25"}]}}]
        with self.assertRaises(AdmissionError):
            admit_fresh(identity, SUBNET, snapshot)

    def test_compose_has_only_pinned_pg18_and_private_explicit_network(self):
        identity = identity_for(ID)
        doc = compose_document(identity, SUBNET, Path("/trusted/new"),
                               Path("/reviewed/initdb.sh"))
        self.assertEqual(list(doc["services"]), ["pg"])
        pg = doc["services"]["pg"]
        self.assertEqual(pg["image"], identity["image"])
        self.assertEqual(pg["pull_policy"], "never")
        self.assertNotIn("ports", pg)
        self.assertEqual(pg["environment"]["POSTGRES_DB"], "postgres")
        self.assertEqual(pg["environment"]["C4_TARGET_DATABASE"], identity["database"])
        self.assertIn("-h 127.0.0.1", pg["healthcheck"]["test"][1])
        self.assertIn(identity["database"], pg["healthcheck"]["test"][1])
        self.assertEqual(doc["networks"]["test"]["ipam"]["config"][0]["subnet"], SUBNET)
        self.assertTrue(doc["networks"]["test"]["internal"])
        self.assertEqual(pg["volumes"][0], {
            "type": "volume", "source": identity["volume"],
            "target": "/var/lib/postgresql", "volume": {"nocopy": True}})
        self.assertNotIn("runtime", str(doc))
        self.assertNotIn("worker", str(doc))

    def test_new_pg_volume_disables_image_copy_up_without_changing_other_mounts(self):
        identity = identity_for(ID)
        target = Path("/trusted/new")
        initdb = Path("/reviewed/initdb.sh")
        doc = compose_document(identity, SUBNET, target, initdb)
        pg = doc["services"]["pg"]
        self.assertEqual(pg["volumes"][0], {
            "type": "volume",
            "source": identity["volume"],
            "target": "/var/lib/postgresql",
            "volume": {"nocopy": True},
        })
        self.assertEqual(pg["volumes"][1],
                         str(initdb) + ":/docker-entrypoint-initdb.d/10-restore.sh:ro")
        self.assertEqual(pg["secrets"], ["postgres_password", "admin_password"])
        self.assertEqual(doc["secrets"]["postgres_password"]["file"],
                         str(target / "secrets" / "postgres_password"))
        self.assertEqual(doc["secrets"]["admin_password"]["file"],
                         str(target / "secrets" / "admin_password"))

    def test_created_identity_requires_exact_labels_ids_mount_and_no_ports(self):
        identity = identity_for(ID)
        expected = created(identity)
        verify_created(identity, SUBNET, empty_snapshot(), expected)
        mutations = [
            lambda s: s["containers"][0]["Mounts"][0].update(Name="old-volume"),
            lambda s: s["containers"][0]["Mounts"][0].update(Source="/other"),
            lambda s: s["containers"][0]["Mounts"][0].update(RW=False),
            lambda s: s["volumes"][0].update(Mountpoint="/different"),
            lambda s: s["containers"][0].update(Id="malformed"),
            lambda s: s["containers"][0]["NetworkSettings"]["Networks"].update(external={}),
            lambda s: s["containers"][0]["NetworkSettings"]["Ports"].update({"5432/tcp": [{"HostPort": "15432"}]}),
            lambda s: s["containers"][0]["Config"]["Labels"].update({"com.docker.compose.service": "worker"}),
            lambda s: s["containers"][0]["State"]["Health"].update(Status="starting"),
            lambda s: s["containers"][0].update(Image="sha256:" + "e" * 64),
            lambda s: s["images"][0].update(RepoDigests=[]),
            lambda s: s["networks"][0].update(Id=""),
            lambda s: s["networks"][0].update(Id="malformed"),
            lambda s: s["networks"][0]["IPAM"]["Config"][0].update(Subnet="10.251.220.0/24"),
            lambda s: s["volumes"][0]["Labels"].update({"com.docker.compose.project": "other"}),
            lambda s: s.update(daemon_id="different"),
            lambda s: s["containers"].append(copy.deepcopy(s["containers"][0])),
        ]
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                live = copy.deepcopy(expected)
                mutate(live)
                with self.assertRaises(AdmissionError):
                    verify_created(identity, SUBNET, empty_snapshot(), live)

    def test_failure_quarantine_stops_only_exact_labeled_new_container_id(self):
        identity = identity_for(ID)
        live = created(identity)
        live["containers"].append({"Id": "c" * 64, "Config": {"Labels": {
            "com.docker.compose.project": "other", "com.docker.compose.service": "pg"}}})
        commands = []
        quarantine(identity, live, lambda *args: commands.append(args))
        self.assertEqual(commands, [("stop", "--time", "1", "a" * 64)])

    @patch("p0c4_restore_target._docker", return_value="999\n999\n")
    def test_postgres_uid_probe_uses_local_pinned_image_without_network(self, docker):
        self.assertEqual(_postgres_uid(), (999, 999))
        args = docker.call_args.args
        self.assertEqual(args[:5], ("run", "--pull=never", "--rm", "--network", "none"))
        self.assertIn(identity_for(ID)["image"], args)

    @patch("p0c4_restore_target._docker", return_value="OK\n")
    def test_post_initdb_probe_uses_verified_id_and_checks_role_acl_gates(self, docker):
        identity = identity_for(ID)
        probe_initdb(identity, "a" * 64)
        args = docker.call_args.args
        self.assertEqual(args[:4], ("exec", "--user", "postgres", "a" * 64))
        self.assertEqual(args[4:8], ("psql", "-XAt", "-v", "ON_ERROR_STOP=1"))
        self.assertIn(identity["database"], args)
        sql = args[-1]
        for clause in ("learning_admin", "learning_runtime", "pg_database_owner",
                       "has_database_privilege", "has_schema_privilege",
                       "pg_control_system", "aclexplode"):
            self.assertIn(clause, sql)
        docker.return_value = "REJECT\n"
        with self.assertRaises(AdmissionError):
            probe_initdb(identity, "a" * 64)

    def test_driver_waits_for_health_before_recording_quarantined_target(self):
        identity = identity_for(ID)
        live = created(identity)
        calls = []
        events = []
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "targets").mkdir()
            def write(path, payload):
                path.write_bytes(payload)
            with (patch("p0c4_restore_target._trusted_initdb"),
                  patch("p0c4_restore_target._locked_root", return_value=contextlib.nullcontext()),
                  patch("p0c4_restore_target._postgres_uid", side_effect=lambda: events.append("docker-uid") or (999, 999)),
                  patch("p0c4_restore_target._sync_directory", side_effect=lambda path: events.append(("fsync", path))) as sync,
                  patch("p0c4_restore_target.os.chown", create=True),
                  patch("p0c4_restore_target.os.lstat", return_value=SimpleNamespace(
                      st_mode=stat.S_IFDIR | 0o700, st_uid=0)),
                  patch("p0c4_restore_target._private_write", side_effect=write),
                  patch("p0c4_restore_target.snapshot", side_effect=[empty_snapshot(), live]),
                  patch("p0c4_restore_target._inspect", return_value=live["images"]),
                  patch("p0c4_restore_target._docker", side_effect=lambda *args: calls.append(args) or "OK\n")):
                result = provision(root, ID, SUBNET, Path("/reviewed/initdb.sh"))
            self.assertEqual(result["state"], "CREATED_QUARANTINED")
            self.assertIn(("compose", "-f", str(root / "targets" / ID / "compose.json"),
                           "up", "-d", "--wait", "--no-build", "--no-deps", "pg"), calls)
            sync.assert_any_call(root / "targets")
            self.assertLess(events.index(("fsync", root / "targets")), events.index("docker-uid"))
            self.assertTrue(any(call[0] == "exec" for call in calls))
            self.assertEqual(json.loads((root / "targets" / ID / "state.json").read_text())["state"],
                             "CREATED_QUARANTINED")

    def test_failed_stop_records_unconfirmed_quarantine_without_claiming_stopped(self):
        identity = identity_for(ID)
        live = created(identity)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "targets").mkdir()
            def docker(*args):
                if args[0] == "stop":
                    raise AdmissionError("stop failed")
                if "up" in args:
                    raise AdmissionError("compose failed")
                return ""
            with (patch("p0c4_restore_target._trusted_initdb"),
                  patch("p0c4_restore_target._locked_root", return_value=contextlib.nullcontext()),
                  patch("p0c4_restore_target._postgres_uid", return_value=(999, 999)),
                  patch("p0c4_restore_target._sync_directory"),
                  patch("p0c4_restore_target.os.chown", create=True),
                  patch("p0c4_restore_target.os.lstat", return_value=SimpleNamespace(
                      st_mode=stat.S_IFDIR | 0o700, st_uid=0)),
                  patch("p0c4_restore_target._private_write", side_effect=lambda path, data: path.write_bytes(data)),
                  patch("p0c4_restore_target.snapshot", side_effect=[empty_snapshot(), live]),
                  patch("p0c4_restore_target._docker", side_effect=docker)):
                with self.assertRaises(AdmissionError):
                    provision(root, ID, SUBNET, Path("/reviewed/initdb.sh"))
            state = json.loads((root / "targets" / ID / "failure.json").read_text())
            self.assertEqual(state["state"], "FAILED_QUARANTINE_ATTEMPTED")
            self.assertFalse(state["container_stop_confirmed"])
            self.assertEqual(state["cleanup_error"], "AdmissionError")

    def test_malformed_project_container_id_cannot_claim_stopped(self):
        identity = identity_for(ID)
        live = created(identity)
        live["containers"][0]["Id"] = "uninspectable"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "targets").mkdir()
            def docker(*args):
                if "up" in args:
                    raise AdmissionError("compose failed")
                if args[0] == "stop":
                    self.fail("malformed ID must never be sent to docker stop")
                return ""
            with (patch("p0c4_restore_target._trusted_initdb"),
                  patch("p0c4_restore_target._locked_root", return_value=contextlib.nullcontext()),
                  patch("p0c4_restore_target._postgres_uid", return_value=(999, 999)),
                  patch("p0c4_restore_target._sync_directory"),
                  patch("p0c4_restore_target.os.chown", create=True),
                  patch("p0c4_restore_target.os.lstat", return_value=SimpleNamespace(
                      st_mode=stat.S_IFDIR | 0o700, st_uid=0)),
                  patch("p0c4_restore_target._private_write", side_effect=lambda path, data: path.write_bytes(data)),
                  patch("p0c4_restore_target.snapshot", side_effect=[empty_snapshot(), live, live]),
                  patch("p0c4_restore_target._docker", side_effect=docker)):
                with self.assertRaises(AdmissionError):
                    provision(root, ID, SUBNET, Path("/reviewed/initdb.sh"))
            state = json.loads((root / "targets" / ID / "failure.json").read_text())
            self.assertFalse(state["container_stop_confirmed"])
            self.assertEqual(state["cleanup_error"], "AdmissionError")

    def test_post_up_state_write_interrupt_stops_exact_container_id(self):
        identity = identity_for(ID)
        live = created(identity)
        stopped = copy.deepcopy(live)
        stopped["containers"][0]["State"]["Running"] = False
        calls = []
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "targets").mkdir()
            state_writes = 0
            def write(path, data):
                nonlocal state_writes
                if path.name == "state.json":
                    state_writes += 1
                    if state_writes == 1:
                        raise KeyboardInterrupt()
                path.write_bytes(data)
            with (patch("p0c4_restore_target._trusted_initdb"),
                  patch("p0c4_restore_target._locked_root", return_value=contextlib.nullcontext()),
                  patch("p0c4_restore_target._postgres_uid", return_value=(999, 999)),
                  patch("p0c4_restore_target._sync_directory"),
                  patch("p0c4_restore_target.os.chown", create=True),
                  patch("p0c4_restore_target.os.lstat", return_value=SimpleNamespace(
                      st_mode=stat.S_IFDIR | 0o700, st_uid=0)),
                  patch("p0c4_restore_target._private_write", side_effect=write),
                  patch("p0c4_restore_target.snapshot", side_effect=[empty_snapshot(), live, live, stopped]),
                  patch("p0c4_restore_target._inspect", return_value=live["images"]),
                  patch("p0c4_restore_target._docker", side_effect=lambda *args: calls.append(args) or "OK\n")):
                with self.assertRaises(KeyboardInterrupt):
                    provision(root, ID, SUBNET, Path("/reviewed/initdb.sh"))
            self.assertIn(("stop", "--time", "1", "a" * 64), calls)
            state = json.loads((root / "targets" / ID / "failure.json").read_text())
            self.assertEqual(state["state"], "FAILED_QUARANTINE_ATTEMPTED")
            self.assertTrue(state["container_stop_confirmed"])


if __name__ == "__main__":
    unittest.main()
