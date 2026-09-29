"""Local, fake-Docker tests for the isolated clone setup diagnostic."""

import json
import contextlib
import io
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import p0c4_clone_setup_diagnostic as diagnostic
import p0c4_restore_pin_acceptance as runner


RUNNER = Path(runner.__file__)
BATCH = "6adeb4f9-94bb-4105-9505-67592af77d11"
IMAGE = "postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
IMAGE_ID = "sha256:" + "b" * 64
HELPER_ID = "c" * 64
VOLUME = "knowweave-c4-clone-setup-diag-" + BATCH
NAME = "knowweave-c4-clone-" + BATCH + "-setup"


def reply(code=0, value=b"", error=b""):
    return SimpleNamespace(returncode=code, stdout=value, stderr=error)


class DockerFake:
    def __init__(self, *, wrong_identity=False, start_code=0, volume_exists=False,
                 image_inspect_fails=False, create_raises=False):
        self.calls = []
        self.wrong_identity = wrong_identity
        self.start_code = start_code
        self.volume_exists = volume_exists
        self.image_inspect_fails = image_inspect_fails
        self.create_raises = create_raises

    def __call__(self, args, **kwargs):
        self.calls.append(args)
        if args[:2] == ["image", "inspect"]:
            if self.image_inspect_fails:
                return reply(code=1, error=b"secret-in-docker-stderr")
            return reply(value=json.dumps([{"Id": IMAGE_ID,
                "RepoDigests": [IMAGE]}]).encode())
        if args[:2] == ["volume", "ls"]:
            return reply(value=(VOLUME + "\n").encode() if self.volume_exists else b"")
        if args[:2] == ["container", "ls"]:
            return reply(value=b"")
        if args[:2] == ["volume", "create"]:
            return reply(value=(VOLUME + "\n").encode())
        if args[:2] == ["volume", "inspect"]:
            return reply(value=json.dumps([{"Name": VOLUME, "Driver": "local",
                "Scope": "local", "Options": None,
                "Labels": {"com.knowweave.clone.setup-diag": BATCH}}]).encode())
        if args[:1] == ["create"]:
            if self.create_raises:
                raise TimeoutError("secret-in-exception")
            return reply(value=(HELPER_ID + "\n").encode())
        if args[:2] == ["container", "inspect"]:
            return reply(value=json.dumps([self.facts(
                running=False, exit_code=self.start_code if self.started else 0)]).encode())
        if args[:2] == ["start", "--attach"]:
            self.started = True
            return reply(code=self.start_code, error=b"secret-in-docker-stderr")
        if args[:3] == ["container", "rm", "-f"]:
            return reply(value=(HELPER_ID + "\n").encode())
        raise AssertionError("unexpected Docker call: " + repr(args))

    started = False

    def facts(self, *, running, exit_code):
        return {"Id": HELPER_ID, "Image": IMAGE_ID,
            "Name": "/" + NAME,
            "Config": {"Image": IMAGE, "User": "0:0",
                "Entrypoint": ["/bin/sh"],
                "Labels": {"com.knowweave.clone.batch": BATCH},
                "Env": ["UNRELATED_SECRET=secret-in-inspect-env"]},
            "HostConfig": {"NetworkMode": "none", "CapDrop": ["ALL"],
                "CapAdd": [] if self.wrong_identity else ["CAP_CHOWN"],
                "SecurityOpt": ["no-new-privileges"],
                "Privileged": False},
            "Mounts": [{"Type": "volume", "Name": VOLUME,
                "Destination": "/var/lib/postgresql", "RW": True}],
            "State": {"Running": running, "ExitCode": exit_code}}


class CloneSetupDiagnosticTests(unittest.TestCase):
    def test_cli_requires_explicit_new_batch_uuid(self):
        with contextlib.redirect_stderr(io.StringIO()), \
             contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaises(SystemExit) as raised:
                diagnostic.main(["--runner", str(RUNNER), "--image", IMAGE,
                                 "--uid", "999", "--gid", "999"])
        self.assertEqual(raised.exception.code, 2)

    def test_cli_rejects_noncanonical_batch_uuid(self):
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as raised:
                diagnostic.main(["--runner", str(RUNNER), "--image", IMAGE,
                                 "--uid", "999", "--gid", "999",
                                 "--batch-id", "old-batch"])
        self.assertEqual(raised.exception.code, 2)

    def test_rejects_unapproved_runner_before_docker(self):
        docker = DockerFake()
        result = diagnostic.diagnose(Path(__file__), IMAGE, 999, 999,
                                     batch_id=BATCH, docker=docker)
        self.assertEqual(result["code"], "RUNNER_HASH_REJECTED")
        self.assertEqual(docker.calls, [])

    def test_rejects_other_pinned_image_before_docker(self):
        docker = DockerFake()
        with patch.object(diagnostic, "_read_runner", return_value=runner):
            result = diagnostic.diagnose(RUNNER,
                                         "postgres:18.6-bookworm@sha256:" + "f" * 64,
                                         999, 999, batch_id=BATCH, docker=docker)
        self.assertEqual(result["code"], "INPUT_REJECTED")
        self.assertEqual(docker.calls, [])

    def test_image_inspect_failure_has_fixed_code_without_raw_error(self):
        docker = DockerFake(image_inspect_fails=True)
        with patch.object(diagnostic, "_read_runner", return_value=runner):
            result = diagnostic.diagnose(RUNNER, IMAGE, 999, 999,
                                         batch_id=BATCH, docker=docker)
        self.assertEqual(result["code"], "IMAGE_INSPECT_FAILED")
        self.assertEqual(result["stages"], ["RUNNER_VERIFIED"])
        self.assertNotIn("secret-in-", json.dumps(result))

    def test_ambiguous_helper_create_leaves_cleanup_unconfirmed(self):
        docker = DockerFake(create_raises=True)
        with patch.object(diagnostic, "_read_runner", return_value=runner):
            result = diagnostic.diagnose(RUNNER, IMAGE, 999, 999,
                                         batch_id=BATCH, docker=docker)
        self.assertEqual(result["code"], "HELPER_CREATE_FAILED")
        self.assertEqual(result["cleanup"], "UNCONFIRMED")
        self.assertEqual(result["failure_type"], "TimeoutError")
        self.assertFalse(any(c[:2] == ["container", "rm"] for c in docker.calls))
        self.assertTrue(result["volume_retained"])
        self.assertNotIn("secret-in-", json.dumps(result))

    def test_fresh_setup_records_stages_removes_exact_helper_and_retains_volume(self):
        docker = DockerFake()
        with patch.object(diagnostic, "_read_runner", return_value=runner):
            result = diagnostic.diagnose(RUNNER, IMAGE, 999, 999,
                                         batch_id=BATCH, docker=docker)
        self.assertEqual(result["code"], "CLONE_SETUP_DIAG_PASSED")
        self.assertEqual(result["stages"], ["RUNNER_VERIFIED", "IMAGE_VERIFIED",
            "FRESH_NAMES_VERIFIED", "VOLUME_CREATED", "VOLUME_VERIFIED",
            "HELPER_CREATED", "PRESTART_INSPECTED", "IDENTITY_VERIFIED",
            "HELPER_STARTED", "EXIT_VERIFIED", "EXACT_ID_REMOVED",
            "VOLUME_RETAINED"])
        self.assertIn(["container", "rm", "-f", HELPER_ID], docker.calls)
        self.assertNotIn(["volume", "rm", VOLUME], docker.calls)
        self.assertEqual(result["cleanup"], "EXACT_ID_REMOVED")
        self.assertEqual(result["observed"]["prestart"]["cap_add"], ["CAP_CHOWN"])
        self.assertNotIn("secret-in-", json.dumps(result))

    def test_wrong_identity_is_not_started_or_blindly_removed(self):
        docker = DockerFake(wrong_identity=True)
        with patch.object(diagnostic, "_read_runner", return_value=runner):
            result = diagnostic.diagnose(RUNNER, IMAGE, 999, 999,
                                         batch_id=BATCH, docker=docker)
        self.assertEqual(result["code"], "IDENTITY_REJECTED")
        self.assertEqual(result["cleanup"], "UNCONFIRMED")
        self.assertFalse(any(c[:1] == ["start"] for c in docker.calls))
        self.assertFalse(any(c[:2] == ["container", "rm"] for c in docker.calls))
        self.assertEqual(result["observed"]["prestart"]["cap_add"], [])
        self.assertNotIn("secret-in-", json.dumps(result))

    def test_failed_start_still_removes_verified_exact_helper(self):
        docker = DockerFake(start_code=17)
        with patch.object(diagnostic, "_read_runner", return_value=runner):
            result = diagnostic.diagnose(RUNNER, IMAGE, 999, 999,
                                         batch_id=BATCH, docker=docker)
        self.assertEqual(result["code"], "HELPER_EXIT_FAILED")
        self.assertEqual(result["cleanup"], "EXACT_ID_REMOVED")
        self.assertIn(["container", "rm", "-f", HELPER_ID], docker.calls)
        self.assertIn("VOLUME_RETAINED", result["stages"])
        self.assertNotIn("secret-in-", json.dumps(result))

    def test_existing_volume_fails_before_create(self):
        docker = DockerFake(volume_exists=True)
        with patch.object(diagnostic, "_read_runner", return_value=runner):
            result = diagnostic.diagnose(RUNNER, IMAGE, 999, 999,
                                         batch_id=BATCH, docker=docker)
        self.assertEqual(result["code"], "FRESH_NAMES_REJECTED")
        self.assertFalse(any(c[:2] == ["volume", "create"] for c in docker.calls))
        self.assertFalse(any(c[:1] == ["create"] for c in docker.calls))


if __name__ == "__main__":
    unittest.main()
