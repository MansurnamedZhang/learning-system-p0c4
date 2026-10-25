"""The packager's Git-byte archive must pass the unchanged birth verifier."""

import hashlib
import json
from pathlib import Path
import stat
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile

import package_p0c4_task3 as packager
import p0c4_controlled_import_acceptance as contract
import p0c4_restore_birth_acceptance as receiver


REPOSITORY = Path(__file__).resolve().parents[1]


class PackageContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Exercise real committed Git blobs in a private fixture. Source-only
        # verification archives intentionally contain no caller's .git history.
        temporary=tempfile.TemporaryDirectory()
        cls.addClassCleanup(temporary.cleanup)
        cls.repository=Path(temporary.name)/'repository';cls.repository.mkdir()
        template=Path(temporary.name)/'empty-template';template.mkdir()
        for name in sorted(set(contract.REQUIRED)|{'docs/architecture/KnowWeave架构设计.html'}):
            path=cls.repository/name;path.parent.mkdir(parents=True,exist_ok=True)
            path.write_bytes((REPOSITORY/name).read_bytes())
        def git(*args):
            return subprocess.check_output(['git','-c','core.autocrlf=false','-c','commit.gpgsign=false','-c','core.hooksPath='+str(template),'-c','user.name=Package fixture','-c','user.email=fixture@example.invalid',*args],cwd=cls.repository,stderr=subprocess.PIPE)
        git('init','--quiet','--template='+str(template))
        git('add','--all');git('commit','--quiet','-m','Synthetic packaging test fixture')
        cls.commit, cls.entries = packager.tracked_snapshot(cls.repository)

    def test_tracked_snapshot_uses_committed_blobs_despite_dirty_worktree(self):
        path=self.repository/contract.ENTRY;original=path.read_bytes()
        untracked=self.repository/'untracked-private-fixture.txt'
        try:
            path.write_bytes(b'uncommitted bytes\n');untracked.write_bytes(b'untracked fixture\n')
            self.assertEqual(packager.tracked_snapshot(self.repository),(self.commit,self.entries))
        finally:
            path.write_bytes(original);untracked.unlink()

    def package_snapshot(self, destination, entries):
        with patch.object(packager, "tracked_snapshot", return_value=(self.commit, entries)):
            return packager.package(self.repository, destination)

    def verify_with_unchanged_receiver(self, root, package_identity):
        installed = root / Path(contract.ENTRY).name
        installed.write_bytes(self.entries[contract.ENTRY])
        archive = Path(package_identity["archive"])
        def fake_lstat(path):
            if Path(path) not in {installed, archive}:
                raise AssertionError("unexpected platform metadata lookup")
            mode = 0o500 if Path(path) == installed else 0o400
            return SimpleNamespace(st_mode=stat.S_IFREG | mode, st_nlink=1)
        platform_os = SimpleNamespace(
            lstat=fake_lstat, open=receiver.os.open, fdopen=receiver.os.fdopen,
            O_RDONLY=receiver.os.O_RDONLY, O_NOFOLLOW=0, O_CLOEXEC=0,
        )
        # Only Linux ownership/mode/no-follow and this fixture's entry inventory
        # are adapted. All canonicalization, inventory and hash checks stay real.
        with patch.object(receiver, "BASE", root), \
             patch.object(receiver, "ENTRY", contract.ENTRY), \
             patch.object(receiver, "REQUIRED", contract.REQUIRED), \
             patch.object(receiver, "__file__", str(installed)), \
             patch.object(receiver, "_require_private_dir"), \
             patch.object(receiver, "_trusted_path"), \
             patch.object(receiver, "os", platform_os):
            return receiver.verify_archive(
                archive, package_identity["archive_sha256"],
                package_identity["manifest_sha256"], self.commit,
            )

    def test_unicode_git_path_package_passes_unchanged_receiver(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            identity = self.package_snapshot(root / "incoming" / "source.zip", self.entries)
            with zipfile.ZipFile(identity["archive"]) as archive:
                manifest_bytes = archive.read("SOURCE_MANIFEST.json")
            self.assertIn("docs/architecture/KnowWeave架构设计.html".encode("utf-8"),
                          manifest_bytes)
            manifest, content = self.verify_with_unchanged_receiver(root, identity)
            self.assertEqual(manifest["commit"], self.commit)
            self.assertEqual(len(manifest["files"]), len(self.entries))
            self.assertEqual(hashlib.sha256(content).hexdigest(),
                             identity["archive_sha256"])

    def test_escaped_unicode_manifest_is_rejected_with_matching_seals(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            identity = self.package_snapshot(root / "incoming" / "source.zip", self.entries)
            escaped_archive = root / "incoming" / "escaped.zip"
            with zipfile.ZipFile(identity["archive"]) as source, \
                 zipfile.ZipFile(escaped_archive, "w") as destination:
                original = source.read("SOURCE_MANIFEST.json")
                escaped = json.dumps(json.loads(original), sort_keys=True,
                                     separators=(",", ":")).encode("utf-8")
                self.assertTrue(original != escaped,
                                "packager must emit literal UTF-8 paths")
                for info in source.infolist():
                    payload = escaped if info.filename == "SOURCE_MANIFEST.json" else source.read(info)
                    destination.writestr(info, payload)
            tampered_identity = dict(identity,
                archive=str(escaped_archive),
                archive_sha256=hashlib.sha256(escaped_archive.read_bytes()).hexdigest(),
                manifest_sha256=hashlib.sha256(escaped).hexdigest())
            with self.assertRaisesRegex(ValueError, "manifest identity or canonical bytes differ"):
                self.verify_with_unchanged_receiver(root, tampered_identity)

    def test_ascii_only_git_bytes_remain_stable_and_accepted(self):
        entries = {path: self.entries[path] for path in contract.REQUIRED}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = self.package_snapshot(root / "incoming" / "one.zip", entries)
            second = self.package_snapshot(root / "incoming" / "two.zip", entries)
            self.assertEqual(Path(first["archive"]).read_bytes(),
                             Path(second["archive"]).read_bytes())
            manifest, _ = self.verify_with_unchanged_receiver(root, first)
            self.assertEqual({item["path"] for item in manifest["files"]}, set(entries))


if __name__ == "__main__":
    unittest.main()
