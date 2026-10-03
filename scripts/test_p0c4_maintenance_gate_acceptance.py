"""Pure admission tests; never execute the root Linux entrypoint."""
import copy
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import warnings
import zipfile

spec = importlib.util.spec_from_file_location("maintenance_gate", Path(__file__).with_name("p0c4_maintenance_gate_acceptance.py"))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)

UUID = "18da437a-891e-4f21-b508-a25dcaf175ef"
OTHER = "9b5719d9-a4e0-4f63-bda9-0ecf6296d36c"
PROJECT = "learning-system-p0c4-maintenance-18da437a891e4f21b508a25dcaf175ef"
DB = "learning_backup_c4_task3_" + UUID
SUBNET = "172.29.101.0/24"
ROOT = Path("/var/lib/knowweave-c4/maintenance-gates") / UUID
CASE = ROOT / "cases" / UUID
PG_IMAGE = "postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
IMAGE_ID = "sha256:" + "c" * 64
TEST_NAME = "source::linux_gate_tests::release_journal_failure_recloses_runtime_connect"


def pg_fixture():
    return {
        "Id": "a" * 64, "Name": "/" + PROJECT + "-pg-1", "Image": IMAGE_ID,
        "Config": {"Image": PG_IMAGE, "Cmd": ["postgres", "-c", "max_prepared_transactions=16"],
                   "Labels": {"com.docker.compose.project": PROJECT, "com.docker.compose.service": "pg"},
                   "Env": ["POSTGRES_USER=postgres", "POSTGRES_DB=postgres", "POSTGRES_PASSWORD_FILE=/run/secrets/postgres_password", "POSTGRES_INITDB_ARGS=--auth-host=scram-sha-256", "C4_DATABASE=" + DB, "C4_OTHER_DATABASE=" + "learning_backup_c4_task3_" + OTHER]},
        "State": {"Running": True},
        "HostConfig": {"Privileged": False, "CapAdd": None, "CapDrop": None, "SecurityOpt": ["no-new-privileges"], "NetworkMode": PROJECT + "_test", "PortBindings": {}, "NanoCpus": 2000000000, "Memory": 4294967296, "MemorySwap": 4294967296, "PidMode": "", "IpcMode": "private", "Devices": [], "Binds": None, "AutoRemove": False},
        "NetworkSettings": {"Ports": {"5432/tcp": None}, "Networks": {PROJECT + "_test": {"NetworkID": "b" * 64, "IPAddress": "172.29.101.2"}}},
        "Mounts": [
            {"Type": "volume", "Name": PROJECT + "_pg", "Source": "/var/lib/docker/volumes/" + PROJECT + "_pg/_data", "Destination": "/var/lib/postgresql", "RW": True},
            {"Type": "bind", "Source": str(CASE / "initdb.sh"), "Destination": "/docker-entrypoint-initdb.d/10-maintenance.sh", "RW": False},
            *[{"Type": "bind", "Source": str(CASE / "secrets/pg" / name), "Destination": "/run/secrets/" + name, "RW": False} for name in ("postgres_password", "admin_password", "runtime_password")]],
    }


def resources():
    return {
        "containers": [pg_fixture()],
        "networks": [{"Id": "b" * 64, "Name": PROJECT + "_test", "Internal": True, "Driver": "bridge", "Labels": {"com.docker.compose.project": PROJECT}, "IPAM": {"Config": [{"Subnet": SUBNET}]}, "Containers": {"a" * 64: {"Name": PROJECT + "-pg-1"}}}],
        "volumes": [{"Name": PROJECT + "_pg", "Driver": "local", "Mountpoint": "/var/lib/docker/volumes/" + PROJECT + "_pg/_data", "Labels": {"com.docker.compose.project": PROJECT}}],
    }


class AdmissionTests(unittest.TestCase):
    def setUp(self):
        self.identity = gate.identity_for(UUID, OTHER)

    def test_noncanonical_or_reused_identity_cannot_allocate(self):
        self.assertEqual(self.identity["project"], PROJECT)
        for invalid in (UUID.upper(), "../" + UUID, "18da437a-891e-1f21-b508-a25dcaf175ef", "nil"):
            with self.subTest(invalid=invalid), self.assertRaises(gate.GateError):
                gate.identity_for(invalid, OTHER)
        with self.assertRaises(gate.GateError):
            gate.identity_for(UUID, UUID)

    def test_route_and_pair_overlap_reject_before_resources(self):
        self.assertEqual(gate.admit_subnets([SUBNET, "172.29.102.0/24", "172.29.103.0/24"], ["default"]), [SUBNET, "172.29.102.0/24", "172.29.103.0/24"])
        for chosen, occupied in (([SUBNET, SUBNET, "172.29.103.0/24"], []), ([SUBNET, "172.29.102.0/24", "172.29.103.0/24"], ["172.29.0.0/16"]), (["8.8.8.0/24", "172.29.102.0/24", "172.29.103.0/24"], [])):
            with self.assertRaises(gate.GateError):
                gate.admit_subnets(chosen, occupied)

    def test_fresh_name_collision_without_labels_is_rejected(self):
        snap = {"containers": [{"Name": "/" + PROJECT + "-pg-1", "Config": {}}], "networks": [], "volumes": []}
        with self.assertRaises(gate.GateError):
            gate.admit_fresh([self.identity], snap, "builder")

    def test_live_identity_requires_exact_mounts_capabilities_and_budget(self):
        self.assertEqual(gate.validate_pg(self.identity, SUBNET, CASE, resources(), IMAGE_ID, running=True)["ip"], "172.29.101.2")
        mutations = [lambda r: r["containers"][0]["HostConfig"].update(Privileged=True), lambda r: r["containers"][0]["HostConfig"].update(CapAdd=["SYS_ADMIN"]), lambda r: r["containers"][0]["HostConfig"].update(Memory=8589934592), lambda r: r["containers"][0]["NetworkSettings"]["Ports"].update({"5432/tcp": [{"HostPort": "5432"}]}), lambda r: r["containers"][0]["Mounts"].append({"Type": "bind", "Source": "/etc", "Destination": "/host", "RW": False}), lambda r: r["networks"][0]["Containers"].update({"d" * 64: {"Name": "foreign"}}), lambda r: r["containers"][0]["Config"].update(Cmd=["postgres"]), lambda r: r["volumes"][0].update(Name="old_volume"), lambda r: r["containers"][0]["Config"]["Env"].append("PGPASSWORD=leak")]
        for mutate in mutations:
            r = resources()
            mutate(r)
            with self.subTest(mutate=mutate), self.assertRaises(gate.GateError):
                gate.validate_pg(self.identity, SUBNET, CASE, r, IMAGE_ID, running=True)

    def test_secret_shape_prevents_passfile_and_sql_escape(self):
        values = {"postgres_password": "1" * 64, "admin_password": "2" * 64, "runtime_password": "3" * 64}
        self.assertEqual(gate.validate_secrets(values), values)
        for bad in ("\npassword", "x:y", "'", "2" * 63):
            candidate = dict(values, admin_password=bad)
            with self.assertRaises(gate.GateError):
                gate.validate_secrets(candidate)

    def test_child_environment_drops_ambient_secrets_and_binds_endpoint(self):
        values = {"postgres_password": "1" * 64, "admin_password": "2" * 64, "runtime_password": "3" * 64}
        result = gate.child_environment("prepared_target", self.identity, CASE, "172.29.101.2", values)
        self.assertNotIn("PGPASSWORD", result)
        self.assertIn("TEST_C4_PREPARED_PGHOST", result)
        self.assertEqual(result["TEST_C4_PREPARED_PGHOST"], "172.29.101.2")
        self.assertEqual(result["TEST_C4_PREPARED_ADMIN_DATABASE_URL"], "postgresql://learning_admin:" + "2" * 64 + "@172.29.101.2:5432/" + DB + "?application_name=knowweave_c4_manager")
        with self.assertRaises(gate.GateError):
            gate.child_environment("prepared_target", self.identity, CASE, "127.0.0.1", values)

    def test_listing_is_not_execution_and_actual_failure_is_rejected(self):
        good = "\nrunning 1 test\ntest " + TEST_NAME + " ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 17 filtered out; finished in 0.01s\n"
        self.assertEqual(gate.accept_test_output(TEST_NAME, 0, good)["passed"], 1)
        for code, output in ((0, TEST_NAME + ": test\n1 test, 0 benchmarks\n"), (101, good), (0, good.replace("... ok", "... FAILED")), (0, good + good), (0, good.replace("1 passed", "0 passed")), (0, good + "test foreign ... ok\n")):
            with self.assertRaises(gate.GateError):
                gate.accept_test_output(TEST_NAME, code, output)

    def test_listing_requires_every_fixed_name_once(self):
        listing = "\n".join(name + ": test" for name in gate.TESTS.values()) + "\n3 tests, 0 benchmarks\n"
        self.assertEqual(gate.accept_listing(0, listing), 3)
        for code, output in ((1, listing), (0, listing.replace(TEST_NAME + ": test", "unknown: test")), (0, listing + TEST_NAME + ": test\n")):
            with self.assertRaises(gate.GateError):
                gate.accept_listing(code, output)

    def test_build_artifact_rejects_multiple_or_nonlib_executables(self):
        row = {"reason": "compiler-artifact", "manifest_path": "/reviewed/crates/learning-backup/Cargo.toml", "target": {"name": "learning_backup", "kind": ["lib"]}, "profile": {"test": True}, "executable": "/target/debug/deps/learning_backup-a123"}
        self.assertEqual(gate.select_artifact(json.dumps(row)), "learning_backup-a123")
        for value in (json.dumps(row) + "\n" + json.dumps(row), json.dumps(dict(row, executable="/etc/passwd")), json.dumps(dict(row, target={"name": "learning_backup", "kind": ["bin"]}))):
            with self.assertRaises(gate.GateError):
                gate.select_artifact(value)

    def test_source_archive_rejects_traversal_duplicate_and_byte_tamper(self):
        entries = {"Cargo.toml": b"workspace", "Cargo.lock": b"lock", "crates/learning-backup/src/source.rs": b"source", "scripts/p0c4_source_isolation.py": b"isolation", "scripts/p0c4_maintenance_gate_acceptance.py": b"runner"}
        def make_package(values, extra=None):
            manifest = {"format_version": 1, "commit": "a" * 40, "files": [{"path": path, "size": len(data), "sha256": gate.digest(data)} for path, data in sorted(values.items())]}
            raw_manifest = gate.json_bytes(manifest)
            stream = io.BytesIO()
            with zipfile.ZipFile(stream, "w") as archive:
                for path, data in {**values, "SOURCE_MANIFEST.json": raw_manifest}.items():
                    info = zipfile.ZipInfo(path)
                    info.external_attr = 0o100444 << 16
                    archive.writestr(info, data)
                if extra:
                    info = zipfile.ZipInfo(extra)
                    info.external_attr = 0o100444 << 16
                    archive.writestr(info, b"unreviewed")
            return stream.getvalue(), raw_manifest
        package, manifest = make_package(entries)
        self.assertIsInstance(gate.verify_package(package, gate.digest(package), gate.digest(manifest), "a" * 40, gate.digest(b"runner"))[0], dict)
        with self.assertRaises(gate.GateError):
            gate.verify_package(package + b"tamper", gate.digest(package), gate.digest(manifest), "a" * 40, gate.digest(b"runner"))
        stream = io.BytesIO(package)
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", UserWarning)
            with zipfile.ZipFile(stream, "a") as archive:
                duplicate = zipfile.ZipInfo("Cargo.toml")
                duplicate.external_attr = 0o100444 << 16
                archive.writestr(duplicate, b"second")
        with self.assertRaises(gate.GateError):
            gate.verify_package(stream.getvalue(), gate.digest(stream.getvalue()), gate.digest(manifest), "a" * 40, gate.digest(b"runner"))
        for values, extra, runner in ((entries, "../outside", gate.digest(b"runner")), ({**entries, "../outside": b"bad"}, None, gate.digest(b"runner")), (entries, None, gate.digest(b"changed"))):
            package, manifest = make_package(values, extra)
            with self.assertRaises(gate.GateError):
                gate.verify_package(package, gate.digest(package), gate.digest(manifest), "a" * 40, runner)

    def builder_fixture(self):
        facts = {"Id": "e" * 64, "Name": "/builder", "Image": gate.BUILDER, "Config": {"Image": gate.BUILDER, "Labels": {"knowweave.c4.maintenance.batch": UUID}, "Env": ["CARGO_NET_OFFLINE=true"], "User": "0:0", "WorkingDir": "/reviewed", "Entrypoint": ["/bin/sh"], "Cmd": ["-ec", "fixed"]}, "State": {"Running": False}, "HostConfig": {"Privileged": False, "CapAdd": None, "CapDrop": ["ALL"], "SecurityOpt": ["no-new-privileges"], "NanoCpus": 4000000000, "Memory": 8589934592, "MemorySwap": 8589934592, "PidMode": "", "IpcMode": "private", "AutoRemove": False, "NetworkMode": "none", "ReadonlyRootfs": True, "PidsLimit": 512}, "NetworkSettings": {"Networks": {}, "Ports": {}}, "Mounts": [{"Type": "bind", "Source": str(ROOT / "source"), "Destination": "/reviewed", "RW": False}, {"Type": "bind", "Source": str(ROOT / "target"), "Destination": "/target", "RW": True}]}
        expected = {"name": "builder", "batch_id": UUID, "source": ROOT / "source", "target": ROOT / "target", "env": {"CARGO_NET_OFFLINE": "true"}, "shell": "fixed", "image_labels": {}}
        return facts, expected

    def test_builder_inherited_labels_match_exact_pinned_baseline(self):
        facts, expected = self.builder_fixture()
        # A singleton-label check or a subset check breaks this contract.
        baseline = {"synthetic.image.owner": "in-memory-only", "knowweave.c4.maintenance.batch": "image-default"}
        expected["image_labels"] = baseline
        facts["Config"]["Labels"]["synthetic.image.owner"] = "in-memory-only"
        try:
            self.assertEqual(gate.validate_builder(facts, expected, {}), "e" * 64)
        except gate.GateError as error:
            self.fail("exact inherited+batch labels rejected: " + str(error))
        for labels in (
            {"synthetic.image.owner": "in-memory-only", "knowweave.c4.maintenance.batch": UUID, "extra": "foreign"},
            {"knowweave.c4.maintenance.batch": UUID},
            {"synthetic.image.owner": "modified", "knowweave.c4.maintenance.batch": UUID},
            {"synthetic.image.owner": "in-memory-only", "knowweave.c4.maintenance.batch": OTHER},
            {"synthetic.image.owner": "in-memory-only"},
        ):
            modified = copy.deepcopy(facts)
            modified["Config"]["Labels"] = labels
            with self.subTest(labels=labels), self.assertRaisesRegex(gate.GateError, "BUILDER_IDENTITY"):
                gate.validate_builder(modified, expected, {})
        self.assertEqual(baseline["knowweave.c4.maintenance.batch"], "image-default")

    def test_image_label_baseline_validates_optional_map_without_coercion(self):
        for config in ({}, {"Labels": None}, {"Labels": {}}):
            with self.subTest(config=config):
                baseline = gate.image_labels({"Config": config})
                self.assertEqual(baseline, {})
                facts, expected = self.builder_fixture()
                expected["image_labels"] = baseline
                self.assertEqual(gate.validate_builder(facts, expected, {}), "e" * 64)
        for labels in (False, True, [], ["k=v"], "", "k=v", 0, {1: "v"}, {"k": None}, {"k": False}, {"k": []}):
            with self.subTest(labels=labels), self.assertRaisesRegex(gate.GateError, "BUILDER_IMAGE_LABELS"):
                gate.image_labels({"Config": {"Labels": labels}})
        self.assertEqual(gate.image_labels({"Config": {"Labels": {"k": "", "other": "v"}}}), {"k": "", "other": "v"})

    def test_builder_cannot_receive_secret_mount_or_network(self):
        facts, expected = self.builder_fixture()
        expected["image_labels"] = {"synthetic.image.owner": "in-memory-only"}
        facts["Config"]["Labels"]["synthetic.image.owner"] = "in-memory-only"
        self.assertEqual(gate.validate_builder(facts, expected, {}), "e" * 64)
        for change in (lambda f: f["Config"]["Env"].append("TEST_ADMIN_DATABASE_URL=secret"), lambda f: f["HostConfig"].update(NetworkMode="bridge"), lambda f: f["Mounts"].append({"Type": "bind", "Source": "/root/.pgpass", "Destination": "/secret", "RW": False}), lambda f: f["HostConfig"].update(AutoRemove=True)):
            modified = copy.deepcopy(facts)
            change(modified)
            with self.assertRaises(gate.GateError):
                gate.validate_builder(modified, expected, {})

    def test_atomic_evidence_refuses_replacement_and_reads_complete_bytes(self):
        temporary_root = Path(__file__).resolve().parents[1] / ".test-tmp"
        temporary_root.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temporary_root) as directory:
            target = Path(directory) / "result.json"
            gate.atomic_write(target, b"complete")
            self.assertTrue(target.exists())
            self.assertEqual(target.read_bytes(), b"complete")
            with self.assertRaises((gate.GateError, FileExistsError)):
                gate.atomic_write(target, b"overwrite")
            self.assertEqual(target.read_bytes(), b"complete")

    def test_fresh_source_tree_rejects_extra_file_and_modified_bytes(self):
        temporary_root = Path(__file__).resolve().parents[1] / ".test-tmp"
        temporary_root.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temporary_root) as directory:
            source = Path(directory)
            (source / "a").write_bytes(b"reviewed")
            manifest = {"files": [{"path": "a", "size": 8, "sha256": gate.digest(b"reviewed")}]}
            self.assertRegex(gate.source_digest(source, manifest, ownership=False), r"^[0-9a-f]{64}$")
            (source / "a").write_bytes(b"changed!")
            with self.assertRaises(gate.GateError):
                gate.source_digest(source, manifest, ownership=False)
            (source / "a").write_bytes(b"reviewed")
            (source / "extra").write_bytes(b"injected")
            with self.assertRaises(gate.GateError):
                gate.source_digest(source, manifest, ownership=False)

    def test_global_volume_inventory_accepts_legal_names_without_skipping_collisions(self):
        class InventoryRunner:
            def run(self, label, command, **kwargs):
                self.command = command
                return subprocess.CompletedProcess(command, 0, json.dumps([{"Name": "Legacy.DB", "Labels": {"com.docker.compose.project": None}}, {"Name": PROJECT + "_pg", "Labels": {"com.docker.compose.project": None}}]).encode(), b"")
            def observe(self, label, command, parser, projection, **kwargs):
                self.command = command
                return parser('\n'.join(json.dumps(row) for row in [{"Name": "Legacy.DB", "Project": None}, {"Name": PROJECT + "_pg", "Project": None}]).encode())
        runner = InventoryRunner()
        try:
            found = gate.inspect_many(runner, "volume", ["Legacy.DB", PROJECT + "_pg"], inventory=True)
        except gate.GateError as error:
            self.fail("legal inventory refused: " + str(error))
        self.assertEqual([r["Name"] for r in found], ["Legacy.DB", PROJECT + "_pg"])
        with self.assertRaises(gate.GateError):
            gate.admit_fresh([self.identity], {"containers": [], "networks": [], "volumes": found}, "builder")
        with self.assertRaises(gate.GateError):
            gate.inspect_many(runner, "volume", ["Legacy.DB"], inventory=True)
        for unsafe in ("../escape", "-option", "with space", "x\ny", "x/y", "", "é"):
            with self.subTest(unsafe=unsafe), self.assertRaises(gate.GateError):
                gate.inspect_many(runner, "volume", [unsafe], inventory=True)

    def test_stop_refuses_foreign_endpoint_and_changed_retained_identity(self):
        def stopped_resources():
            current = resources()
            current["containers"][0]["State"]["Running"] = False
            current["networks"][0]["Containers"] = {}
            return current
        class RetentionRunner:
            def __init__(self, current):
                self.current = current
                self.mutations = []
            def run(self, label, command, **kwargs):
                if "stop" in command or "disconnect" in command or "rm" in command:
                    self.mutations.append(command)
                kind = "volumes" if "volume" in command else "networks" if "network" in command else "containers"
                return subprocess.CompletedProcess(command, 0, json.dumps(self.current[kind]).encode(), b"")
            def observe(self, label, command, parser, projection, **kwargs):
                return parser(self.run(label, command).stdout)
        def record():
            return {"container_id": "a" * 64, "network_id": "b" * 64, "volume_name": PROJECT + "_pg", "identity": self.identity, "case_path": str(CASE), "image_id": IMAGE_ID, "volume_mountpoint": "/var/lib/docker/volumes/" + PROJECT + "_pg/_data", "stop_verified": False, "condition": "UNVERIFIED_UNUSABLE"}
        valid = record()
        gate.stop_pg(RetentionRunner(stopped_resources()), valid)
        self.assertTrue(valid["stop_verified"])
        mutations = [lambda r: r["networks"][0]["Containers"].update({"f" * 64: {"Name": "foreign"}}), lambda r: r["networks"][0].update(Internal=False), lambda r: r["networks"][0].update(Name="foreign_network"), lambda r: r["networks"][0].update(Id="c" * 64), lambda r: r["networks"][0]["Labels"].update({"com.docker.compose.project": "foreign"}), lambda r: r["volumes"][0]["Labels"].update({"com.docker.compose.project": "foreign"}), lambda r: r["volumes"][0].update(Mountpoint="/foreign"), lambda r: r["containers"][0].update(Name="/foreign"), lambda r: r["containers"][0].update(Image="sha256:" + "d" * 64)]
        for mutate in mutations:
            current = stopped_resources()
            mutate(current)
            runner = RetentionRunner(current)
            candidate = record()
            candidate["stop_verified"] = True
            with self.subTest(mutate=mutate), self.assertRaises(gate.GateError):
                gate.stop_pg(runner, candidate)
            self.assertFalse(candidate["stop_verified"])
            self.assertEqual(runner.mutations, [])

    def test_unknown_observation_secrets_never_reach_durable_files(self):
        temporary_root = Path(__file__).resolve().parents[1] / ".test-tmp"
        temporary_root.mkdir(exist_ok=True)
        marker = "SYNTHETIC_UNKNOWN_OLD_SECRET_7ab98d3"
        facts = [{"Id": "a" * 64, "Name": "/legacy", "Config": {"Env": ["OLD_TOKEN=" + marker], "Cmd": [marker], "Labels": {"UNKNOWN_LABEL": marker}}, "Mounts": [{"Source": "/" + marker}], "State": {"Running": False}}]
        with tempfile.TemporaryDirectory(dir=temporary_root) as directory:
            root = Path(directory)
            runner = gate.Runner(root)
            command = [sys.executable, "-I", "-B", "-c", "import sys; sys.stdout.write(sys.argv[1]); sys.stderr.write(sys.argv[2])", json.dumps(facts), marker]
            observed = runner.observe("unknown-observation", command, lambda raw: json.loads(raw), gate.observation_projection, env={}, timeout=10)
            self.assertEqual(observed[0]["Config"]["Env"], ["OLD_TOKEN=" + marker])
            persisted = b"".join(path.read_bytes() for path in root.iterdir())
            self.assertGreater(len(persisted), 0)
            self.assertNotIn(marker.encode(), persisted)
            self.assertNotIn(b"OLD_TOKEN", persisted)
            self.assertNotIn(b"UNKNOWN_LABEL", persisted)
            self.assertNotIn(b"Mounts", persisted)

    def test_global_inventory_parser_rejects_unexpected_secret_fields(self):
        safe = b'{"Id":"' + b"a" * 64 + b'","Name":"/legacy","Project":null}\n'
        parsed = gate.parse_inventory("container", safe)
        self.assertEqual(parsed[0]["Name"], "/legacy")
        for raw in (safe.replace(b'"Project":null', b'"Project":null,"Env":["SYNTHETIC_TOKEN"]'), safe + safe, b'[]', safe.replace(b'"Name":"/legacy"', b'"Name":"../invalid"')):
            with self.subTest(raw=raw), self.assertRaises(gate.GateError):
                gate.parse_inventory("container", raw)

    def test_global_network_names_accept_docker_names_but_keep_exact_collision_gate(self):
        def observation(name):
            return gate.json_bytes({"Id": "b" * 64, "Name": name, "Project": None, "Subnets": ["172.29.101.0/24"]})
        for name in ("Legacy network", "旧项目网络", "-legacy-network", "  Legacy network  "):
            with self.subTest(name=name):
                try:
                    found = gate.parse_inventory("network", observation(name))
                except gate.GateError as error:
                    self.fail("Docker-valid network name refused: " + str(error))
                self.assertEqual(found[0]["Name"], name)
                gate.admit_fresh([self.identity], {"containers": [], "networks": found, "volumes": []}, "builder")
        found = gate.parse_inventory("network", observation(PROJECT + "_test"))
        with self.assertRaises(gate.GateError):
            gate.admit_fresh([self.identity], {"containers": [], "networks": found, "volumes": []}, "builder")
        for name in ("", " \t\n", None, 12, [], {}):
            with self.subTest(name=name), self.assertRaises(gate.GateError):
                gate.parse_inventory("network", observation(name))


if __name__ == "__main__":
    unittest.main()
