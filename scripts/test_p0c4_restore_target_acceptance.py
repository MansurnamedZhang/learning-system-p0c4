"""Local mock gates for the root-only C4 PG18 target acceptance runner."""

import copy
import hashlib
import io
import json
from pathlib import Path
import stat
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile

import p0c4_restore_target as target
import p0c4_restore_target_acceptance as runner
from test_p0c4_restore_target import ID, SUBNET, created, empty_snapshot


COMMIT = "a" * 40


def package(entries, *, extra=None, override=None):
    manifest = {"format_version": 1, "commit": COMMIT,
                "files": [{"path": name, "sha256": runner.digest(data), "size": len(data)}
                          for name, data in sorted(entries.items())]}
    manifest_bytes = json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w", compression=zipfile.ZIP_STORED) as archive:
        for name, data in sorted(entries.items()):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.external_attr = (override or {}).get(name, stat.S_IFREG | 0o444) << 16
            archive.writestr(info, data)
        for name, data in extra or []:
            info = zipfile.ZipInfo(name)
            info.external_attr = (stat.S_IFREG | 0o444) << 16
            archive.writestr(info, data)
        info = zipfile.ZipInfo("SOURCE_MANIFEST.json")
        info.external_attr = (stat.S_IFREG | 0o444) << 16
        archive.writestr(info, manifest_bytes)
    return stream.getvalue(), manifest_bytes


def source_entries():
    return {runner.ENTRY: b"runner", runner.PROVISIONER: b"provisioner",
            runner.INITDB: b"#!/bin/sh\n", "README.md": b"reviewed source\n"}


class ArchiveGates(unittest.TestCase):
    def check(self, content, manifest_bytes, *, expected_error=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            incoming = root / "incoming"
            incoming.mkdir()
            archive_path = incoming / "approved.zip"
            archive_path.write_bytes(content)
            with (patch.object(runner, "BASE", root),
                  patch.object(runner, "trusted_path"),
                  patch.object(runner, "file_digest", return_value=runner.digest(b"runner"))):
                args = (archive_path, runner.digest(content), runner.digest(manifest_bytes), COMMIT)
                if expected_error:
                    with self.assertRaises((ValueError, zipfile.BadZipFile)):
                        runner.verify_archive(*args)
                else:
                    return runner.verify_archive(*args)

    def test_exact_archive_and_every_tracked_file(self):
        content, manifest_bytes = package(source_entries())
        manifest, checked = self.check(content, manifest_bytes)
        self.assertEqual(checked, content)
        self.assertEqual(len(manifest["files"]), 4)
        with (tempfile.TemporaryDirectory() as directory,
              patch.object(runner, "sync_dir"),
              patch.object(runner, "private_dir", side_effect=lambda path: path.mkdir())):
            source = Path(directory) / "source"
            with patch.object(runner, "source_digest", return_value="verified"):
                self.assertEqual(runner.extract_verified(content, manifest, source), "verified")
            self.assertEqual((source / "README.md").read_bytes(), b"reviewed source\n")
            (source / "README.md").chmod(0o600)
            (source / "README.md").write_bytes(b"changed")
            self.assertNotEqual((source / "README.md").read_bytes(), b"reviewed source\n")

    def test_duplicate_traversal_symlink_and_unlisted_member_rejected(self):
        entries = source_entries()
        cases = [
            package(entries, extra=[("README.md", b"duplicate")]),
            package(entries, extra=[("../escape", b"escape")]),
            package(entries, override={runner.INITDB: stat.S_IFLNK | 0o777}),
            package(entries, extra=[("unlisted.txt", b"unlisted")]),
        ]
        for content, manifest_bytes in cases:
            with self.subTest(content_hash=runner.digest(content)):
                self.check(content, manifest_bytes, expected_error=True)

    def test_wrong_self_or_member_hash_rejected(self):
        content, manifest_bytes = package(source_entries())
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "incoming").mkdir()
            archive_path = root / "incoming" / "approved.zip"
            archive_path.write_bytes(content)
            with (patch.object(runner, "BASE", root),
                  patch.object(runner, "trusted_path"),
                  patch.object(runner, "file_digest", return_value="0" * 64)):
                with self.assertRaisesRegex(ValueError, "installed runner"):
                    runner.verify_archive(archive_path, runner.digest(content),
                                          runner.digest(manifest_bytes), COMMIT)
        rebuilt = io.BytesIO()
        with zipfile.ZipFile(io.BytesIO(content)) as original, zipfile.ZipFile(rebuilt, "w") as changed:
            for info in original.infolist():
                payload = original.read(info.filename)
                changed.writestr(info, b"altered" if info.filename == "README.md" else payload)
        self.check(rebuilt.getvalue(), manifest_bytes, expected_error=True)


class RuntimeGates(unittest.TestCase):
    def test_live_mounts_secrets_and_official_pg_env(self):
        identity = target.identity_for(ID)
        live = created(identity)
        pg = live["containers"][0]
        initdb = Path("/var/lib/knowweave-c4/batches/new/initdb.sh")
        target_dir = Path("/var/lib/knowweave-c4/batches/new/control/targets") / ID
        for destination, source in (
                ("/docker-entrypoint-initdb.d/10-restore.sh", initdb),
                ("/run/secrets/postgres_password", target_dir / "secrets/postgres_password"),
                ("/run/secrets/admin_password", target_dir / "secrets/admin_password")):
            pg["Mounts"].append({"Type": "bind", "Source": str(source),
                                 "RW": False, "Destination": destination})
        pg["Config"]["Env"] = ["PATH=/usr/local/bin:/usr/bin", "PG_MAJOR=18",
                                "PG_VERSION=18.6-1.pgdg13+1", "PGDATA=/var/lib/postgresql/18/docker",
                                "POSTGRES_USER=postgres", "POSTGRES_DB=postgres",
                                "POSTGRES_PASSWORD_FILE=/run/secrets/postgres_password",
                                "C4_TARGET_DATABASE=" + identity["database"]]
        fake = SimpleNamespace(snapshot=lambda: copy.deepcopy(live),
                               _inspect=lambda *_: live["images"],
                               verify_created=target.verify_created)
        expected = target.verify_created(identity, SUBNET, empty_snapshot(), live)
        runner.verified_live(fake, identity, SUBNET, empty_snapshot(), expected,
                             initdb, target_dir)
        pg["Config"]["Env"].append("POSTGRES_PASSWORD=plaintext")
        with self.assertRaisesRegex(ValueError, "credential"):
            runner.verified_live(fake, identity, SUBNET, empty_snapshot(), expected,
                                 initdb, target_dir)

    def test_negative_probe_requires_explicit_reject(self):
        identity = target.identity_for(ID)

        class Fake:
            AdmissionError = target.AdmissionError

            def __init__(self, probe_output):
                self.commands = []
                self.probe_output = probe_output

            def _docker(self, *args):
                self.commands.append(args)
                return "GRANT\n" if "GRANT CREATE ON SCHEMA public TO learning_runtime" in args else self.probe_output

            def probe_initdb(self, _identity, _container):
                result = self._docker("exec", "--user", "postgres", "id",
                                      "psql", "-c", "SELECT reviewed probe")
                if result != "OK\n":
                    raise self.AdmissionError("dedicated PG initdb facts differ")

        fake = Fake("REJECT\n")
        self.assertEqual(runner.negative_acl_probe(fake, identity, "a" * 64),
                         hashlib.sha256(b"REJECT\n").hexdigest())
        self.assertEqual(len(fake.commands), 2)
        for wrong in ("OK\n", "connection failed\n"):
            with self.subTest(wrong=wrong), self.assertRaises(ValueError):
                runner.negative_acl_probe(Fake(wrong), identity, "a" * 64)

    def test_stop_only_exact_inspected_pg_and_retains_volume(self):
        identity = target.identity_for(ID)
        live = created(identity)
        stopped = copy.deepcopy(live)
        stopped["containers"][0]["State"]["Running"] = False
        snapshots = iter([live, stopped])
        stop_calls = []
        fake = SimpleNamespace(snapshot=lambda: next(snapshots),
                               quarantine=target.quarantine,
                               _docker=lambda *args: stop_calls.append(args))
        proof = runner.safe_stop(fake, identity, "a" * 64)
        self.assertEqual(stop_calls, [("stop", "--time", "1", "a" * 64)])
        self.assertTrue(proof["volume_retained"])
        foreign = created(identity)
        foreign["containers"][0]["Config"]["Labels"]["com.docker.compose.project"] = "foreign"
        fake.snapshot = lambda: foreign
        with self.assertRaisesRegex(ValueError, "cannot safely"):
            runner.safe_stop(fake, identity, "a" * 64)


if __name__ == "__main__":
    unittest.main()
