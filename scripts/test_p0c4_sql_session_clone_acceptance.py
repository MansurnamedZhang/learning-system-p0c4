"""Task 3b runner gates; all Docker and Linux work stays at the boundary."""

import contextlib
import hashlib
import io
import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch

import p0c4_restore_pin_acceptance as runner


PRIMARY = "c42e38a1-d61f-4a33-8df7-95836a299253"
CLONE = "b27f4d57-1165-4b17-92c1-4ddf9a178eaa"


class CloneAdmission(unittest.TestCase):
    def setUp(self):
        self.primary = {"project": "primary", "volume": "primary_pg", "database": "primary_db",
                        "network": "primary_test", "image": "postgres@sha256:pin"}
        self.clone = {"project": "clone", "volume": "clone_pg", "database": "clone_db",
                      "network": "clone_test", "image": "postgres@sha256:pin"}
        self.snapshot = {"daemon_id": "daemon", "containers": [],
                         "networks": [], "volumes": [], "routes": []}

    def test_two_projects_and_nonoverlapping_subnets_are_required_before_creation(self):
        admitted = []
        provisioner = SimpleNamespace(admit_fresh=lambda identity, subnet, before:
                                      admitted.append((identity["project"], subnet)))
        runner._admit_clone_pair(provisioner, self.primary, "10.251.228.0/24",
                                 self.clone, "10.251.229.0/24", self.snapshot)
        self.assertEqual(admitted, [("primary", "10.251.228.0/24"),
                                    ("clone", "10.251.229.0/24")])
        for clone, subnet in ((self.primary, "10.251.229.0/24"),
                              (self.clone, "10.251.228.0/24"),
                              (self.clone, "10.251.228.128/25")):
            with self.subTest(clone=clone["project"], subnet=subnet):
                admitted.clear()
                with self.assertRaises(ValueError):
                    runner._admit_clone_pair(provisioner, self.primary,
                                             "10.251.228.0/24", clone,
                                             subnet, self.snapshot)
                self.assertEqual(admitted, [])

    def test_clone_baseline_comes_from_sealed_birth_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            evidence = {"container_id": "a" * 64,
                        "container_started_at": "2026-09-29T00:00:00Z"}
            payload = json.dumps(evidence).encode()
            (target / "birth-evidence.json").write_bytes(payload)
            inspection = {"birth_evidence_sha256": hashlib.sha256(
                payload).hexdigest()}
            acceptance = SimpleNamespace(_private_read_diagnostic=lambda path,
                                         limit: path.read_bytes(),
                                         _unique_json=lambda raw: json.loads(raw))
            self.assertEqual(runner._sealed_primary_started_at(
                acceptance, target, inspection, {"container_id": "a" * 64}),
                "2026-09-29T00:00:00Z")
            (target / "birth-evidence.json").write_bytes(
                json.dumps(dict(evidence,
                    container_started_at="2026-09-29T00:01:00Z")).encode())
            with self.assertRaises(ValueError):
                runner._sealed_primary_started_at(
                    acceptance, target, inspection,
                    {"container_id": "a" * 64})

    def test_replication_preflight_rejects_unproven_contract_before_copy(self):
        primary_id = "a" * 64
        rule = {"rule_number": 1, "type": "host",
                "database": ["replication"], "user_name": ["postgres"],
                "address": "127.0.0.1", "netmask": "255.255.255.255",
                "auth_method": "trust", "options": None, "error": None}
        for response in (b"REJECT\n", b"", b"TRUST_LOOPBACK\nSCRAM_LOOPBACK\n"):
            with self.subTest(response=response), patch.object(
                    runner, "_probe_docker", return_value=SimpleNamespace(
                        returncode=0, stdout=response, stderr=b"")):
                with self.assertRaises(ValueError):
                    runner._check_replication_contract(primary_id)
        with patch.object(runner, "_probe_docker", return_value=SimpleNamespace(
                returncode=0, stdout=json.dumps([rule]).encode() + b"\n",
                stderr=b"")) as docker:
            self.assertEqual(runner._check_replication_contract(primary_id),
                             "trust")
        command = docker.call_args.args[0]
        self.assertEqual(command[:4], ["exec", "--user", "postgres", primary_id])
        self.assertNotIn("password", " ".join(command).lower())
        self.assertIn("ORDER BY r.rule_number", command[-1])
        self.assertIn("pg_hba_file_rules", command[-1])
        with patch.object(runner, "_probe_docker", return_value=SimpleNamespace(
                returncode=0, stdout=json.dumps([
                    dict(rule, auth_method="scram-sha-256")]).encode() + b"\n",
                stderr=b"")):
            self.assertEqual(runner._check_replication_contract(primary_id),
                             "scram-sha-256")

    def test_hba_first_effective_match_skips_unrelated_rules_but_rejects_broad_match(self):
        rule = {"rule_number": 3, "type": "host", "database": ["replication"],
                "user_name": ["postgres"], "address": "127.0.0.1",
                "netmask": "255.255.255.255", "auth_method": "trust",
                "options": None}
        unrelated_user = dict(rule, rule_number=1, user_name=["other"])
        unrelated_address = dict(rule, rule_number=2, address="192.0.2.0",
                                 netmask="255.255.255.0")
        self.assertEqual(runner._effective_replication_hba(
            [unrelated_user, unrelated_address, rule]), "trust")
        broad = dict(rule, rule_number=1, address="0.0.0.0", netmask="0.0.0.0")
        with self.assertRaises(ValueError):
            runner._effective_replication_hba([broad, rule])

    def test_ambiguous_helper_create_recovers_only_exact_owned_id(self):
        helper_id = "f" * 64
        name = "knowweave-c4-clone-" + CLONE + "-verify"
        facts = {"Id": helper_id, "Image": "sha256:" + "1" * 64,
                 "Name": "/" + name,
                 "Config": {"Image": "postgres@sha256:pin",
                            "User": "999:999", "Entrypoint": ["/bin/sh"],
                            "Env": [], "Labels": {"com.knowweave.clone.batch": CLONE}},
                 "HostConfig": {"NetworkMode": "none", "CapDrop": ["ALL"],
                                "CapAdd": None,
                                "SecurityOpt": ["no-new-privileges"],
                                "Privileged": False},
                 "Mounts": [{"Type": "volume", "Name": "clone_pg",
                             "Destination": "/var/lib/postgresql", "RW": False}]}
        calls = []
        def docker(command, **_kwargs):
            calls.append(command)
            if command[0] == "create":
                raise TimeoutError("ambiguous create")
            if command[:2] == ["container", "inspect"]:
                self.assertEqual(command[-1], name)
                return SimpleNamespace(returncode=0,
                                       stdout=json.dumps([facts]).encode())
            if command[:2] == ["container", "rm"]:
                return SimpleNamespace(returncode=0, stdout=b"")
            self.fail("unexpected helper command")
        with patch.object(runner, "_probe_docker", side_effect=docker):
            with self.assertRaises(TimeoutError):
                runner._run_clone_helper(["create", "--name", name],
                                         "postgres@sha256:pin", "a" * 64,
                                         "clone_pg", batch_id=CLONE,
                                         uid=999, gid=999,
                                         image_id="sha256:" + "1" * 64,
                                         kind="verify")
        self.assertEqual(calls[-1], ["container", "rm", "-f", helper_id])

    def test_ambiguous_helper_with_foreign_identity_records_unconfirmed(self):
        name = "knowweave-c4-clone-" + CLONE + "-verify"
        calls = []
        def docker(command, **_kwargs):
            calls.append(command)
            if command[0] == "create":
                return SimpleNamespace(returncode=1, stdout=b"")
            if command[:2] == ["container", "inspect"]:
                return SimpleNamespace(returncode=0, stdout=json.dumps([{
                    "Id": "f" * 64, "Name": "/foreign",
                    "Config": {"Labels": {"com.knowweave.clone.batch": CLONE}}
                }]).encode())
            self.fail("foreign helper was removed or started")
        with patch.object(runner, "_probe_docker", side_effect=docker):
            with self.assertRaises(ValueError) as raised:
                runner._run_clone_helper(["create", "--name", name],
                                         "postgres@sha256:pin", "a" * 64,
                                         "clone_pg", batch_id=CLONE,
                                         uid=999, gid=999,
                                         image_id="sha256:" + "1" * 64,
                                         kind="verify")
        self.assertEqual(raised.exception.clone_helper_cleanup, "UNCONFIRMED")
        self.assertEqual(len(calls), 2)

    def test_clone_project_is_created_by_compose_before_copy(self):
        clone = {"project": "clone", "network": "clone_test", "volume": "clone_pg",
                 "image": "postgres:18@sha256:pin"}
        before = {"daemon_id": "daemon", "containers": [], "networks": [],
                  "volumes": [], "routes": []}
        image_id = "sha256:" + "1" * 64
        pg_id, network_id = "b" * 64, "c" * 64
        after = {"daemon_id": "daemon", "containers": [{
            "Id": pg_id, "Image": image_id, "Name": "/clone-pg-1",
            "Config": {"Image": clone["image"], "User": "999:999",
                       "Cmd": ["postgres"],
                       "Env": ["PGDATA=" + runner.CLONE_DATA],
                       "Labels": {"com.docker.compose.project": "clone",
                                  "com.docker.compose.service": "pg"}},
            "HostConfig": {"NetworkMode": "clone_test", "CapDrop": ["ALL"],
                           "SecurityOpt": ["no-new-privileges"],
                           "Privileged": False, "PortBindings": {}},
            "State": {"Running": False, "Status": "created"},
            "Mounts": [{"Type": "volume",
                "Name": "clone_pg", "Source": "/docker/clone_pg",
                "Destination": "/var/lib/postgresql", "RW": True}]}],
            "networks": [{"Id": network_id, "Name": "clone_test", "Internal": True,
                          "IPAM": {"Config": [{"Subnet": "10.251.229.0/24"}]},
                          "Labels": {"com.docker.compose.project": "clone"}}],
            "volumes": [{"Name": "clone_pg", "Driver": "local", "Scope": "local",
                         "Options": None, "Mountpoint": "/docker/clone_pg",
                         "Labels": {"com.docker.compose.project": "clone"}}]}
        seen = []
        provisioner = SimpleNamespace(
            admit_fresh=lambda *_: None, _postgres_uid=lambda: (999, 999),
            _inspect=lambda *_: [{"Id": image_id,
                                  "RepoDigests": [clone["image"]]}],
            snapshot=lambda: (after if any("create" in command for command in seen)
                              else before))
        def docker(command, **_kwargs):
            seen.append(command)
            self.assertEqual(command[:2], ["compose", "-f"])
            return SimpleNamespace(returncode=0, stdout=b"")
        with tempfile.TemporaryDirectory() as directory, \
             patch.object(runner, "_probe_docker", side_effect=docker), \
             patch.object(runner, "_private_write", side_effect=lambda path, data:
                          path.write_bytes(data)):
            resources = runner._create_clone_resources(
                provisioner, Path(directory), clone, "10.251.229.0/24", before)
            document = json.loads((Path(directory) / "clone-compose.json").read_bytes())
        self.assertEqual(resources["container_id"], pg_id)
        self.assertEqual(seen[-1][-6:],
                         ["create", "--no-build", "--pull", "never",
                          "--no-recreate", "pg"])
        self.assertTrue(document["services"]["pg"]["volumes"][0]["volume"]["nocopy"])
        self.assertNotIn("secrets", document)

    def test_copied_pg_starts_only_the_compose_created_exact_id(self):
        clone = {"project": "clone", "network": "clone_test", "volume": "clone_pg",
                 "image": "postgres:18@sha256:pin"}
        clone_id = "b" * 64
        resources = {"container_id": clone_id, "compose_project_created": True,
                     "network_id": "c" * 64, "volume_mountpoint": "/docker/clone_pg",
                     "image_id": "sha256:" + "1" * 64}
        calls = []
        def docker(command, **_kwargs):
            calls.append(command)
            if command[:2] == ["container", "inspect"]:
                running = any(item == ["start", clone_id] for item in calls)
                return SimpleNamespace(returncode=0, stdout=json.dumps([{
                    "State": {"Running": running,
                              "Status": "running" if running else "created"}
                }]).encode())
            if command[0] == "start":
                return SimpleNamespace(returncode=0,
                                       stdout=(clone_id + "\n").encode())
            if "pg_isready" in command:
                return SimpleNamespace(returncode=0, stdout=b"")
            if "psql" in command:
                return SimpleNamespace(returncode=0, stdout=b"123|42\n")
            self.fail("raw PG creation or unexpected Docker command")
        with patch.object(runner, "_probe_docker", side_effect=docker), \
             patch.object(runner, "_verify_clone_pg", return_value=clone_id):
            result = runner._start_clone_pg(
                object(), clone, resources, "a" * 64,
                {"pg_system_identifier": "123", "database_oid": 42},
                "learning_restore_c4_" + PRIMARY, 999, 999)
        self.assertEqual(result["container_id"], clone_id)
        self.assertEqual([c for c in calls if c[0] == "start"],
                         [["start", clone_id]])

    def test_setup_helper_accepts_docker_cap_name_without_relaxing_cap_set(self):
        helper_id = "f" * 64
        image_id = "sha256:" + "1" * 64
        image = "postgres@sha256:pin"
        volume = "clone_pg"
        facts = {"Id": helper_id, "Image": image_id,
                 "Name": "/knowweave-c4-clone-" + CLONE + "-setup",
                 "Config": {"Image": image, "User": "0:0",
                            "Entrypoint": ["/bin/sh"],
                            "Labels": {"com.knowweave.clone.batch": CLONE},
                            "Env": []},
                 "HostConfig": {"NetworkMode": "none", "CapDrop": ["ALL"],
                                "CapAdd": ["CAP_CHOWN"],
                                "SecurityOpt": ["no-new-privileges"],
                                "Privileged": False},
                 "Mounts": [{"Type": "volume", "Name": volume,
                             "Destination": "/var/lib/postgresql", "RW": True}]}
        args = (helper_id, image, "a" * 64, volume)
        options = {"batch_id": CLONE, "uid": 999, "gid": 999,
                   "image_id": image_id, "kind": "setup"}
        for cap_add in (["CHOWN"], ["CAP_CHOWN"]):
            with self.subTest(allowed=cap_add):
                valid = dict(facts, HostConfig=dict(facts["HostConfig"],
                                                    CapAdd=cap_add))
                runner._verify_clone_helper(valid, *args, **options)
        for cap_add in (None, [], ["CAP_CHOWN", "CAP_NET_ADMIN"],
                        ["CHOWN", "CAP_CHOWN"]):
            with self.subTest(rejected=cap_add), self.assertRaises(ValueError):
                invalid = dict(facts, HostConfig=dict(facts["HostConfig"],
                                                      CapAdd=cap_add))
                runner._verify_clone_helper(invalid, *args, **options)

    def test_setup_helper_sets_mode_before_transferring_ownership(self):
        command = runner._clone_setup_command(
            "clone_pg", "postgres@sha256:pin", CLONE, 999, 999)
        self.assertEqual(command[command.index("--network") + 1], "none")
        self.assertEqual(command[command.index("--cap-drop") + 1], "ALL")
        self.assertEqual(command[command.index("--cap-add") + 1], "CHOWN")
        self.assertNotIn("--env", command)
        self.assertEqual(command[-2], "-ec")

        steps = command[-1].split("; ")
        self.assertEqual(steps, [
            f"mkdir -p {runner.CLONE_DATA}",
            f"chmod 0700 {runner.CLONE_DATA}",
            f'entries="$(ls -A {runner.CLONE_DATA})"',
            'test -z "$entries"',
            f"chown 999:999 /var/lib/postgresql/18 {runner.CLONE_DATA}",
        ])
        mode, owner = 0o755, (0, 0)
        for step in steps[1:]:
            if step.startswith("chmod "):
                self.assertEqual(owner, (0, 0),
                                 "root helper lacks CAP_FOWNER after chown")
                mode = 0o700
            elif step.startswith("entries="):
                self.assertEqual(owner, (0, 0),
                                 "root helper cannot read 0700 PGDATA after chown")
            elif step.startswith("chown "):
                owner = (999, 999)
            elif step != 'test -z "$entries"':
                self.fail(f"unexpected setup step: {step}")
        self.assertEqual((mode, owner), (0o700, (999, 999)))

    def test_clone_helper_mount_contract_rejects_extra_or_wrong_volume(self):
        helper_id = "f" * 64
        clone_volume = "clone_pg"
        passfile = Path("/var/lib/knowweave-c4/pin-acceptance/batches/new/passfile")
        expected = [{"Type": "volume", "Name": clone_volume,
                     "Destination": "/var/lib/postgresql", "RW": True},
                    {"Type": "bind", "Source": str(passfile),
                     "Destination": "/run/secrets/replication.pgpass", "RW": False}]
        image_id = "sha256:" + "1" * 64
        facts = {"Id": helper_id, "Image": image_id,
                 "Name": "/knowweave-c4-clone-" + CLONE + "-copy",
                 "Config": {"Image": "postgres@sha256:pin", "User": "999:999",
                            "Entrypoint": ["/bin/sh"],
                            "Labels": {"com.knowweave.clone.batch": CLONE},
                            "Env": ["PGPASSFILE=/run/secrets/replication.pgpass"]},
                 "HostConfig": {"NetworkMode": "container:" + "a" * 64,
                                "CapDrop": ["ALL"], "CapAdd": None,
                                "SecurityOpt": ["no-new-privileges"],
                                "Privileged": False},
                 "Mounts": expected}
        runner._verify_clone_helper(facts, helper_id, "postgres@sha256:pin",
                                    "a" * 64, clone_volume, passfile,
                                    batch_id=CLONE, uid=999, gid=999,
                                    image_id=image_id, kind="copy")
        for mounts in (expected + [{"Type": "bind", "Destination": "/other"}],
                       [dict(expected[0], Name="old_pg"), expected[1]],
                       [expected[0], dict(expected[1], RW=True)]):
            with self.subTest(mounts=mounts), self.assertRaises(ValueError):
                runner._verify_clone_helper(dict(facts, Mounts=mounts),
                                            helper_id, "postgres@sha256:pin",
                                            "a" * 64, clone_volume, passfile,
                                            batch_id=CLONE, uid=999, gid=999,
                                            image_id=image_id, kind="copy")
        for changed in (dict(facts, Image="sha256:" + "2" * 64),
                        dict(facts, Name="/foreign"),
                        dict(facts, Config=dict(facts["Config"], User="0:0")),
                         dict(facts, HostConfig=dict(facts["HostConfig"],
                                                     CapAdd=["CAP_CHOWN"])),
                        dict(facts, HostConfig=dict(facts["HostConfig"],
                                                    Privileged=True))):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                runner._verify_clone_helper(changed, helper_id,
                                            "postgres@sha256:pin", "a" * 64,
                                            clone_volume, passfile,
                                            batch_id=CLONE, uid=999, gid=999,
                                            image_id=image_id, kind="copy")

    def test_failed_helper_inspection_removes_exact_created_helper(self):
        helper_id = "f" * 64
        calls = []
        def docker(command, **_kwargs):
            calls.append(command)
            if command[0] == "create":
                return SimpleNamespace(returncode=0,
                                       stdout=(helper_id + "\n").encode())
            if command[:2] == ["container", "inspect"]:
                return SimpleNamespace(returncode=1, stdout=b"")
            if command[:2] == ["container", "rm"]:
                return SimpleNamespace(returncode=0, stdout=b"")
            self.fail("helper started before inspection")
        with patch.object(runner, "_probe_docker", side_effect=docker):
            with self.assertRaises(ValueError):
                runner._run_clone_helper(
                    ["create", "--name", "knowweave-c4-clone-" + CLONE + "-verify"],
                    "postgres@sha256:pin",
                                         "a" * 64, "clone_pg", batch_id=CLONE,
                                         uid=999, gid=999, image_id="sha256:" +
                                         "1" * 64, kind="verify")
        self.assertEqual(calls[-1], ["container", "rm", "-f", helper_id])

    def test_copied_pg_requires_exact_image_and_single_volume_as_postgres(self):
        clone = {"project": "clone", "network": "clone_test",
                 "volume": "clone_pg", "image": "postgres:18@sha256:pin"}
        facts = {"Id": "b" * 64, "Image": "sha256:" + "1" * 64,
                 "Name": "/clone-pg-1",
                 "Config": {"Image": clone["image"], "Cmd": ["postgres"],
                            "User": "999:999", "Env": ["PGDATA=" + runner.CLONE_DATA],
                            "Labels": {"com.docker.compose.project": "clone",
                                       "com.docker.compose.service": "pg"}},
                 "HostConfig": {"NetworkMode": "clone_test", "CapDrop": ["ALL"],
                                "Privileged": False,
                                "SecurityOpt": ["no-new-privileges"]},
                 "State": {"Running": True},
                 "NetworkSettings": {"Networks": {"clone_test": {
                     "NetworkID": "c" * 64}}, "Ports": {}},
                 "Mounts": [{"Type": "volume", "Name": "clone_pg",
                             "Source": "/docker/volumes/clone_pg/_data",
                             "Destination": "/var/lib/postgresql", "RW": True}]}
        runner._verify_clone_pg(facts, clone, "a" * 64, "c" * 64,
                                "/docker/volumes/clone_pg/_data", running=True,
                                image_id="sha256:" + "1" * 64,
                                uid=999, gid=999)
        for changed in (dict(facts, Image="sha256:" + "2" * 64),
                        dict(facts, Config=dict(facts["Config"], User="0:0")),
                        dict(facts, Mounts=facts["Mounts"] + [{
                            "Type": "bind", "Destination": "/extra"}])):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                runner._verify_clone_pg(changed, clone, "a" * 64, "c" * 64,
                                        "/docker/volumes/clone_pg/_data",
                                        running=True,
                                        image_id="sha256:" + "1" * 64,
                                        uid=999, gid=999)

    def test_both_exact_pg_ids_are_stopped_and_retained_after_failure(self):
        calls = []
        def stop(_provisioner, identity, container_id):
            calls.append((identity["project"], container_id))
            return {"confirmed": True, "volume_retained": True,
                    "container_id": container_id}
        result = runner._stop_clone_pair(SimpleNamespace(stop_verified_pg=stop),
                                         object(), self.primary, "a" * 64,
                                         self.clone, "b" * 64)
        self.assertEqual(calls, [("clone", "b" * 64), ("primary", "a" * 64)])
        self.assertTrue(result["confirmed"])
        self.assertTrue(result["volume_retained"])
        self.assertEqual(result["containers"], ["b" * 64, "a" * 64])
        calls.clear()
        def failing_stop(_provisioner, identity, container_id):
            calls.append(container_id)
            if identity["project"] == "clone":
                raise RuntimeError("stop failed")
            return {"confirmed": True, "volume_retained": True}
        failed = runner._stop_clone_pair(
            SimpleNamespace(stop_verified_pg=failing_stop), object(),
            self.primary, "a" * 64, self.clone, "b" * 64)
        self.assertEqual(calls, ["b" * 64, "a" * 64])
        self.assertFalse(failed["confirmed"])
        self.assertFalse(failed["volume_retained"])

    def test_clone_mode_requires_second_fresh_identity_and_subnet_at_cli(self):
        common = ["--archive", "/private/reviewed.zip", "--archive-sha256", "a" * 64,
                  "--manifest-sha256", "b" * 64, "--source-commit", "c" * 40,
                  "--runner-sha256", "d" * 64, "--batch-id", PRIMARY,
                  "--subnet", "10.251.228.0/24", "--sql-session-clone-negative"]
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(runner.main(common), 1)
        admitted = []
        with patch.object(runner, "run", side_effect=lambda args:
                          admitted.append(args) or 0):
            self.assertEqual(runner.main(common + ["--clone-batch-id", CLONE,
                                                  "--clone-subnet", "10.251.229.0/24"]), 0)
        self.assertEqual(admitted[0].clone_batch_id, CLONE)
        self.assertTrue(admitted[0].sql_session_clone_negative)

    def test_passfile_is_private_short_lived_and_secret_never_enters_command(self):
        with tempfile.TemporaryDirectory() as directory:
            batch = Path(directory) / CLONE
            batch.mkdir()
            secret = batch / "postgres_password"
            secret.write_bytes(b"a" * 64 + b"\n")
            with patch.object(runner, "_read_clone_secret",
                              return_value=b"a" * 64), \
                 patch.object(runner, "_require_private_dir"), \
                 patch.object(runner, "_require_runtime_tmpfs"), \
                 patch.object(runner, "_sync_dir"), \
                 patch.object(runner, "_private_dir", side_effect=lambda path:
                              path.mkdir()), \
                 patch.object(runner, "CLONE_RUNTIME_ROOT", Path(directory)), \
                 patch.object(runner.os, "chown", create=True), \
                 patch.object(runner.stat, "S_IMODE", return_value=0o600), \
                 patch.object(runner, "_private_write", side_effect=lambda path, data:
                              (path.write_bytes(data), path.chmod(0o600))):
                with runner._clone_passfile(batch, secret,
                                            batch.stat().st_uid, 0) as passfile:
                    self.assertEqual(passfile.parent,
                                     Path(directory) / ("knowweave-c4-clone-" + CLONE))
                    self.assertIn(b"127.0.0.1:5432:*:postgres:",
                                  passfile.read_bytes())
                    command = runner._clone_backup_command(
                        "b" * 64, "new_pg", passfile, "postgres:18@sha256:pin",
                        CLONE, 999, 999)
                    joined = " ".join(command)
                    self.assertNotIn("a" * 64, joined)
                    self.assertNotIn("PGPASSWORD=", joined)
                    self.assertIn("-X stream", joined)
                    self.assertIn("container:" + "b" * 64, joined)
                    self.assertIn("readonly", joined)
            self.assertFalse(passfile.exists())
            self.assertFalse(passfile.parent.exists())

    def test_exact_loopback_trust_clone_never_reads_or_mounts_a_secret(self):
        clone = {"project": "clone", "network": "clone_test",
                 "volume": "clone_pg", "image": "postgres:18@sha256:pin"}
        commands = []
        provisioner = SimpleNamespace(_postgres_uid=lambda: (999, 999))
        resources = {"volume_name": "clone_pg",
                     "image_id": "sha256:" + "1" * 64,
                     "postgres_uid": 999, "postgres_gid": 999}
        with patch.object(runner, "_clone_passfile", side_effect=AssertionError(
                    "passfile created under trust")), \
             patch.object(runner, "_read_clone_secret", side_effect=AssertionError(
                    "secret read under trust")), \
             patch.object(runner, "_run_clone_helper", side_effect=lambda command,
                          *_args, **_kwargs: commands.append(command) or True):
            result = runner._copy_primary_volume(
                provisioner, Path("/private/" + CLONE), Path("/private/target"),
                "a" * 64, clone, resources, "trust")
        self.assertEqual(len(commands), 3)
        self.assertIn("-X stream", " ".join(commands[1]))
        self.assertNotIn("PGPASSFILE", " ".join(commands[1]))
        self.assertNotIn("replication.pgpass", " ".join(commands[1]))
        self.assertEqual(result["replication_auth"], "EXACT_LOOPBACK_TRUST")

    def test_clone_builder_requires_distinct_linux_test_before_pg_birth(self):
        with tempfile.TemporaryDirectory() as directory:
            batch = Path(directory) / CLONE
            batch.mkdir()
            binary = batch / "test-binary"
            binary.write_bytes(b"binary")
            clone_test = ("restore_preflight::target_binding::tests::"
                          "live_read_only_same_id_wrong_endpoint_negative: test\n")
            old_test = ("restore_preflight::target_binding::tests::"
                        "live_read_only_sql_session_binding: test\n")
            with patch.object(runner, "_trusted_path"), \
                 patch.object(runner, "_compile_bound_probe",
                              return_value=(binary, runner._file_digest(binary))), \
                 patch.object(runner, "_probe_docker", return_value=SimpleNamespace(
                     returncode=0,
                     stdout=(runner.BUILDER_IMAGE_ID + "\n").encode())):
                with patch.object(runner, "_run_bounded", return_value=SimpleNamespace(
                        returncode=0, stdout=old_test.encode(), stderr=b"")):
                    with self.assertRaises(ValueError):
                        runner._preflight_probe_builder(batch, batch, clone=True)
                with patch.object(runner, "_run_bounded", return_value=SimpleNamespace(
                        returncode=0, stdout=clone_test.encode(), stderr=b"")):
                    self.assertTrue(runner._preflight_probe_builder(
                        batch, batch, clone=True)["host_test_listing_confirmed"])

    def test_clone_probe_requires_exact_one_test_marker_and_only_nonsecret_identity(self):
        test_name = ("restore_preflight::target_binding::tests::"
                     "live_read_only_same_id_wrong_endpoint_negative")
        marker = runner.CLONE_PASSED
        result_line = ("test result: ok. 1 passed; 0 failed; 0 ignored; "
                       "0 measured; 0 filtered out;\n")
        parallel = ("running 1 test\n" + marker + "\ntest " + test_name +
                    " ... ok\n" + result_line).encode()
        serial = ("running 1 test\ntest " + test_name + " ... " + marker +
                  "\nok\n" + result_line).encode()
        with tempfile.TemporaryDirectory() as directory:
            batch = Path(directory) / PRIMARY
            batch.mkdir()
            binary = batch / "test-binary"
            binary.write_bytes(b"binary")
            primary = {"database": "learning_restore_c4_" + PRIMARY,
                       "project": "learning-system-p0c4-restore-" + PRIMARY,
                       "network": "learning-system-p0c4-restore-" + PRIMARY + "_test"}
            clone = {"project": "learning-system-p0c4-restore-" + CLONE,
                     "network": "learning-system-p0c4-restore-" + CLONE + "_test",
                     "volume": "learning-system-p0c4-restore-" + CLONE + "_pg"}
            target = batch / "control" / "targets" / PRIMARY
            def execute(command, **kwargs):
                self.assertEqual(command[1:], [test_name, "--exact", "--ignored", "--nocapture"])
                env = kwargs["env"]
                self.assertEqual(set(env), {"HOME", "PATH", "KNOWWEAVE_C4_PROBE_DESTINATION_ROOT",
                    "KNOWWEAVE_C4_PROBE_CONTROL_ROOT", "KNOWWEAVE_C4_PROBE_ASSET_ROOT",
                    "KNOWWEAVE_C4_PROBE_EXPECTED_DATABASE", "KNOWWEAVE_C4_CLONE_CONTAINER_ID",
                    "KNOWWEAVE_C4_CLONE_NETWORK_ID", "KNOWWEAVE_C4_CLONE_NETWORK_NAME",
                    "KNOWWEAVE_C4_CLONE_PROJECT", "KNOWWEAVE_C4_CLONE_VOLUME_NAME",
                    "KNOWWEAVE_C4_CLONE_SUBNET", "KNOWWEAVE_C4_PRIMARY_CONTAINER_ID"})
                self.assertEqual(env["KNOWWEAVE_C4_CLONE_CONTAINER_ID"], "b" * 64)
                self.assertEqual(env["KNOWWEAVE_C4_PRIMARY_CONTAINER_ID"], "a" * 64)
                return SimpleNamespace(returncode=0, stdout=current, stderr=b"")
            args = (batch / "source", batch, target, primary, clone, "d" * 64,
                    "a" * 64, "b" * 64, "c" * 64, "10.251.229.0/24")
            for current in (parallel, serial):
                with patch.object(runner, "_compile_bound_probe",
                                  return_value=(binary, runner._file_digest(binary))), \
                     patch.object(runner, "_run_bounded", side_effect=execute):
                    self.assertEqual(runner._run_clone_negative_probe(*args)["state"], marker)
            for bad in (parallel + marker.encode(),
                        serial + marker.encode(),
                        parallel.replace(b"running 1 test", b"running 2 tests"),
                        serial.replace(b"\nok\n", b"\nFAILED\n"),
                        serial.replace(b"\nok\n", b"\nok\nok\n"),
                        serial.replace(marker.encode(), b"FORGED_" + marker.encode()),
                        serial.replace(b"test " + test_name.encode(), b"test other"),
                        parallel.replace(b"test " + test_name.encode() + b" ... ok", b"test other ... ok"),
                        parallel.replace(b"0 measured", b"1 measured"),
                        parallel + result_line.encode(),
                        parallel.replace(marker.encode(), b"WRONG")):
                with patch.object(runner, "_compile_bound_probe",
                                  return_value=(binary, runner._file_digest(binary))), \
                     patch.object(runner, "_run_bounded", return_value=SimpleNamespace(
                         returncode=0, stdout=bad, stderr=b"")):
                    with self.assertRaises(ValueError):
                        runner._run_clone_negative_probe(*args)

    def test_failed_copy_evidence_distinguishes_retained_volumes_and_stopped_ids(self):
        primary = {"project": "primary", "volume": "primary_pg"}
        clone = {"project": "clone", "volume": "clone_pg"}
        snapshot = {"containers": [{"Id": "a" * 64,
                     "State": {"Running": False}},
                    {"Id": "b" * 64, "State": {"Running": False}}],
                    "volumes": [{"Name": "primary_pg", "Labels": {
                        "com.docker.compose.project": "primary"}},
                        {"Name": "clone_pg", "Labels": {
                        "com.docker.compose.project": "clone"}}]}
        provisioner = SimpleNamespace(snapshot=lambda: snapshot)
        result = runner._clone_quarantine_evidence(
            provisioner, primary, "a" * 64, clone, "b" * 64)
        self.assertEqual(result, {"primary_stopped": True,
                                  "clone_stopped": True,
                                  "primary_volume_retained": True,
                                  "clone_volume_retained": True})
        snapshot["volumes"].pop()
        self.assertFalse(runner._clone_quarantine_evidence(
            provisioner, primary, "a" * 64, clone,
            "b" * 64)["clone_volume_retained"])

    def test_copy_is_verified_before_second_pg_starts(self):
        clone = {"project": "clone", "network": "clone_test",
                 "volume": "clone_pg", "image": "postgres:18@sha256:pin"}
        order = []
        with patch.object(runner, "_create_clone_resources",
                          side_effect=lambda *_: order.append("resources") or
                          {"network_id": "n" * 64, "volume_name": "clone_pg"}), \
             patch.object(runner, "_check_replication_contract",
                          side_effect=lambda *_: order.append("hba") or "trust"), \
             patch.object(runner, "_copy_primary_volume",
                          side_effect=lambda *_: order.append("basebackup") or
                          {"backup_verified": True, "no_standby": True,
                           "pgdata": runner.CLONE_DATA, "passfile_removed": True,
                           "passfile_state": "NOT_CREATED",
                           "postgres_uid": 999, "postgres_gid": 999,
                           "replication_auth": "EXACT_LOOPBACK_TRUST"}), \
             patch.object(runner, "_start_clone_pg",
                          side_effect=lambda *_: order.append("start") or
                          {"container_id": "b" * 64, "volume_retained": True}):
            result = runner._prepare_physical_clone(
                object(), Path("/private/batch"), Path("/private/target"),
                "a" * 64, clone, "10.251.229.0/24", {},
                {"pg_system_identifier": "123", "database_oid": 42},
                "learning_restore_c4_" + PRIMARY)
        self.assertEqual(order, ["hba", "resources", "basebackup", "start"])
        self.assertEqual(result["container_id"], "b" * 64)
        order.clear()
        with patch.object(runner, "_check_replication_contract",
                          side_effect=ValueError("HBA unverified")), \
             patch.object(runner, "_create_clone_resources",
                          side_effect=lambda *_: self.fail("resources created")):
            with self.assertRaises(ValueError):
                runner._prepare_physical_clone(
                    object(), Path("/private/batch"), Path("/private/target"),
                    "a" * 64, clone, "10.251.229.0/24", {},
                    {"pg_system_identifier": "123", "database_oid": 42},
                    "learning_restore_c4_" + PRIMARY)


if __name__ == "__main__":
    unittest.main()
