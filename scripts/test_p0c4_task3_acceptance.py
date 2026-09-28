"""Mock/static tests for the Task3 root acceptance harness; no Docker needed."""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import uuid
import zipfile

from p0c4_task3_acceptance import (
    CASES, assertion_for_case, case_identity, available_subnets, verify_archive,
)


class AcceptancePlanTests(unittest.TestCase):
    def test_three_cases_have_distinct_canonical_database_and_projects(self):
        batch = uuid.UUID("550e8400-e29b-41d4-a716-446655440000")
        identities = [case_identity(batch, kind) for kind in CASES]
        self.assertEqual(len({item["project"] for item in identities}), 3)
        self.assertEqual(len({item["database"] for item in identities}), 3)
        for item in identities:
            self.assertEqual(str(uuid.UUID(item["database"].removeprefix("learning_backup_c4_task3_"))),
                             item["database"].removeprefix("learning_backup_c4_task3_"))

    def test_archive_requires_exact_sha_manifest_and_regular_entries(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "source.zip"
            data = b"reviewed"
            manifest = json.dumps({"format_version": 1, "commit": "a" * 40,
                                   "files": [{"path": "file.txt", "sha256": hashlib.sha256(data).hexdigest(),
                                              "size": len(data)}]}, sort_keys=True,
                                  separators=(",", ":")).encode()
            with zipfile.ZipFile(archive, "w") as output:
                output.writestr("file.txt", data)
                output.writestr("SOURCE_MANIFEST.json", manifest)
            verified = verify_archive(archive, hashlib.sha256(archive.read_bytes()).hexdigest(),
                                      hashlib.sha256(manifest).hexdigest())
            self.assertEqual(verified["files"][0]["path"], "file.txt")
            with self.assertRaises(ValueError):
                verify_archive(archive, "0" * 64, hashlib.sha256(manifest).hexdigest())
            with self.assertRaises(ValueError):
                verify_archive(archive, hashlib.sha256(archive.read_bytes()).hexdigest(), "0" * 64)

    def test_archive_rejects_symlink_even_with_matching_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "source.zip"
            data = b"target"
            manifest = json.dumps({"format_version": 1, "commit": "a" * 40,
                                   "files": [{"path": "link", "sha256": hashlib.sha256(data).hexdigest(),
                                              "size": len(data)}]}, sort_keys=True,
                                  separators=(",", ":")).encode()
            with zipfile.ZipFile(archive, "w") as output:
                entry = zipfile.ZipInfo("link")
                entry.external_attr = 0o120777 << 16
                output.writestr(entry, data)
                output.writestr("SOURCE_MANIFEST.json", manifest)
            with self.assertRaises(ValueError):
                verify_archive(archive, hashlib.sha256(archive.read_bytes()).hexdigest(),
                               hashlib.sha256(manifest).hexdigest())

    def test_subnet_overlap_fails_before_mutation(self):
        requested = ["10.251.215.0/24", "10.251.216.0/24", "10.251.217.0/24"]
        self.assertEqual(len(available_subnets(requested, ["10.251.200.0/24"])), 3)
        with self.assertRaises(ValueError):
            available_subnets(requested, ["10.251.215.128/25"])
        with self.assertRaises(ValueError):
            available_subnets([requested[0], requested[0], requested[2]], [])
        with self.assertRaises(ValueError):
            available_subnets([requested[0], "10.251.215.128/25", requested[2]], [])

    def test_case_evidence_requires_exact_single_test_and_phase(self):
        backup = uuid.uuid4()
        success = "real_gate_waits_for_old_runtime_session_and_rejects_new_runtime_login"
        log = f"test {success} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored;".encode()
        assertion_for_case("success", backup, log, {"released.json"}, True, False)
        with self.assertRaises(ValueError):
            assertion_for_case("success", backup, log.replace(b"1 passed", b"0 passed"),
                               {"released.json"}, True, False)
        with self.assertRaises(ValueError):
            assertion_for_case("missing_original", backup, log, {"dump-and-index-durable.json"},
                               False, False)
        negative = ("test missing_original_or_pg_dump_failure_keeps_gate_closed ... ok\n"
                    "test result: ok. 1 passed; 0 failed; 0 ignored;").encode()
        assertion_for_case("missing_original", backup, negative,
                           {"intent.json", "closed.json", "drained.json", "dump-and-index-durable.json"},
                           False, False)
        assertion_for_case("pg_dump_exit", backup, negative,
                           {"intent.json", "closed.json", "drained.json"}, False, False)
        with self.assertRaises(ValueError):
            assertion_for_case("missing_original", backup, negative,
                               {"dump-and-index-durable.json", "released.json"}, False, False)


if __name__ == "__main__":
    unittest.main()
