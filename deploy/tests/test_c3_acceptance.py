"""Pure acceptance-policy checks; no Docker, network or credentials."""
import importlib.util
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("c3", ROOT / "deploy/c3_acceptance.py")
c3 = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(c3)

def worker_inspect():
    return {"Config":{"User":"65532:65532","Env":["SNAPSHOT_ROOT=/snapshots"],"Cmd":["worker"]},
                "HostConfig":{"Privileged":False,"Devices":[],"ReadonlyRootfs":True,"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges:true"],"NanoCpus":2000000000,"Memory":2147483648,"PidsLimit":128,"PortBindings":{}},
                "Mounts":[{"Destination":"/run/secrets/runtime_password","RW":False,"Type":"bind"},{"Destination":"/assets","RW":False,"Type":"volume","Name":"fresh_test_assets"},{"Destination":"/usr/local/bin/learning-worker","RW":False,"Type":"bind"},{"Destination":"/snapshots","RW":True,"Type":"volume","Name":"fresh_c3_snapshots"}],
                "NetworkSettings":{"Networks":{"fresh_test":{}},"Ports":{}}}

class AcceptancePolicy(unittest.TestCase):
    def test_inventory_has_distinct_databases_and_all_frozen_paths(self):
        inventory = c3.inventory(ROOT)
        self.assertEqual(len(set(inventory.values())), len(inventory))
        self.assertEqual(set(inventory), {"TEST", "TEST_UPGRADE", "TEST_B1_UPGRADE", "TEST_B2_UPGRADE", "TEST_B3_SCHEMA_UPGRADE", "TEST_C3_UPGRADE", "TEST_IMPORT_A", "TEST_IMPORT_B", "TEST_IMPORT_UPGRADE", "C3_SOURCE", "C3_TARGET"})
        init = (ROOT / "deploy/initdb.sh").read_text()
        env = (ROOT / "deploy/c3-env.sh").read_text()
        for prefix, db in inventory.items():
            self.assertIn(f"CREATE DATABASE {db} OWNER learning_admin;", init)
            self.assertIn(f"{prefix} {db}", env)
        bootstrap = (ROOT / "deploy/bootstrap-fixtures.sh").read_text()
        for kind in ["p0a", "b1", "b2", "b3-schema"]:
            self.assertIn(kind, bootstrap)

    def test_security_check_rejects_admin_source_socket_and_ephemeral_snapshot(self):
        good = worker_inspect()
        c3.check_worker(good, "fresh_test")
        import copy
        for destination in ["/run/secrets/admin_password", "/app", "/var/run/docker.sock"]:
            bad = copy.deepcopy(good)
            bad["Mounts"].append({"Destination":destination,"RW":False,"Type":"bind"})
            with self.assertRaises(ValueError): c3.check_worker(bad,"fresh_test")
        bad = copy.deepcopy(good)
        bad["Mounts"][-1]["Type"] = "tmpfs"
        with self.assertRaises(ValueError): c3.check_worker(bad,"fresh_test")

    def test_worker_rejects_privileged_override_and_foreign_volume(self):
        good = worker_inspect()
        good["HostConfig"]["Privileged"] = True
        with self.assertRaises(ValueError): c3.check_worker(good,"fresh_test")
        good["HostConfig"]["Privileged"] = False
        good["Mounts"][-1]["Name"] = "old_project_snapshots"
        with self.assertRaises(ValueError): c3.check_worker(good,"fresh_test")

    def test_originals_are_real_png_and_pdf(self):
        import struct, zlib
        directory = ROOT / "crates/learning-worker/examples/c3"
        png = (directory / "attention.png").read_bytes()
        self.assertEqual(png[:8], b"\x89PNG\r\n\x1a\n")
        offset = 8
        kinds = []
        while offset < len(png):
            size = struct.unpack(">I", png[offset:offset+4])[0]
            kind = png[offset+4:offset+8]
            body = png[offset+8:offset+8+size]
            crc = struct.unpack(">I", png[offset+8+size:offset+12+size])[0]
            self.assertEqual(crc, zlib.crc32(kind+body), f"invalid PNG {kind} CRC")
            kinds.append(kind)
            if kind == b"IDAT": self.assertTrue(zlib.decompress(body))
            offset += 12 + size
        self.assertEqual(kinds, [b"IHDR", b"IDAT", b"IEND"])
        pdf = (directory / "attention.pdf").read_bytes()
        start = int(pdf.split(b"startxref\n")[1].splitlines()[0])
        self.assertTrue(pdf[start:].startswith(b"xref\n0 5\n"))
        for i, entry in enumerate(pdf[start:].splitlines()[3:7], 1):
            self.assertTrue(pdf[int(entry[:10]):].startswith(f"{i} 0 obj".encode()))

    def test_failure_groups_require_each_actual_case(self):
        output = "\n".join(f"test {case} ... ok" for cases in c3.FAILURES.values() for case in cases)
        self.assertEqual(set(c3.failure_evidence(output)), set(c3.FAILURES))
        with self.assertRaises(ValueError): c3.failure_evidence(output.replace("interrupted_input_never_creates_ready_package ... ok", "interrupted_input_never_creates_ready_package ... FAILED"))
        with self.assertRaises(ValueError): c3.failure_evidence("compilation succeeded")

    def test_project_validation_is_fail_closed(self):
        for name in ["", "prod", "learning-system-p0c3-", "../old", "learning-system-p0c3-X"]:
            with self.assertRaises(ValueError): c3.validate_project(name)
        c3.validate_project("learning-system-p0c3-task7-abc123")

if __name__ == "__main__": unittest.main()
