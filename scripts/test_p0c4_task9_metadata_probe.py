import json
import stat
import subprocess
import types
import unittest
from unittest.mock import patch

import p0c4_task9_metadata_probe as probe


def metadata(mode=stat.S_IFDIR | 0o755):
    return types.SimpleNamespace(st_mode=mode, st_uid=0, st_gid=0,
                                 st_dev=1, st_ino=2, st_nlink=1, st_size=10)


class Task9ReadOnlyMetadataTests(unittest.TestCase):
    def test_key_probe_never_reads_key_contents_or_makes_authority(self):
        with patch("builtins.open", side_effect=AssertionError("file content opened")):
            value = probe.observe_path(probe.PATHS[-1], lstat=lambda path: metadata(
                stat.S_IFREG | 0o600 if path == probe.PATHS[-1] else stat.S_IFDIR | 0o755))
        self.assertEqual(value["state"], "METADATA_OBSERVED_NOT_CONTENT_VALIDATED")
        self.assertNotIn("sha256", value)
        self.assertNotIn("key", value)

    def test_permission_denial_is_unknown_and_not_absence(self):
        def denied(_):
            raise PermissionError()
        self.assertEqual(probe.observe_path(probe.PATHS[-1], lstat=denied)["state"],
                         "UNKNOWN_PERMISSION_DENIED")

    def test_symlink_ancestor_stops_before_traversal(self):
        paths = []
        def linked(path):
            paths.append(path)
            return metadata(stat.S_IFLNK | 0o777) if path == "/var/lib" else metadata()
        self.assertEqual(probe.observe_path(probe.PATHS[-1], lstat=linked)["state"],
                         "REFUSED_SYMLINK_COMPONENT")
        self.assertEqual(paths, ["/var", "/var/lib"])

    def test_arbitrary_or_session_paths_rejected_before_metadata_access(self):
        with self.assertRaises(ValueError):
            probe.observe_path("/unrelated/session-fixture", lstat=lambda _: self.fail("metadata read"))

    def test_mount_report_discards_unrequested_fields(self):
        def fake_run(argv, **kwargs):
            self.assertEqual(argv[-1], "TARGET,FSTYPE,MAJ:MIN")
            data = {"filesystems": [{"target": "/", "fstype": "ext4", "maj:min": "8:1",
                                       "source": "not-for-report", "options": "not-for-report"}]}
            return subprocess.CompletedProcess(argv, 0, json.dumps(data).encode(), b"")
        result = probe.observe_mount(run=fake_run)
        self.assertEqual(set(result), {"state", "target", "fstype", "maj:min"})

    def test_complete_report_cannot_claim_readiness_from_metadata(self):
        with patch.object(probe.platform, "system", return_value="Linux"), \
             patch.object(probe.os, "geteuid", return_value=1000, create=True):
            result = probe.collect(lstat=lambda _: metadata(),
                                   run=lambda argv, **_: subprocess.CompletedProcess(argv, 1, b"", b""))
        self.assertFalse(result["task9_accepted"])
        self.assertFalse(result["resources_modified"])
        self.assertFalse(result["credentials_read_or_written"])
        self.assertFalse(result["privilege_escalation_attempted"])
        self.assertEqual(result["independent_target"], "UNVERIFIED")
        self.assertEqual(result["private_key_custody"], "UNVERIFIED_NO_CONTENT_READ")


if __name__ == "__main__":
    unittest.main()
