"""Pure checks for the root-only C4 maintenance driver; no Docker required."""

import unittest
from unittest.mock import patch
from subprocess import CompletedProcess

from scripts.p0c4_source_isolation import IsolationError, assess_project, probe_runtime_denied, manager_environment, validate_admin_endpoint, _daemon_preflight, _atomic_private_file, _private_capture, _publish_manager_logs, redact_log


PROJECT = "learning-system-p0c4-test-1234"


def container(service, *, running=False, ports=None, mounts=None, env=None, name=None):
    return {
        "Id": service + "-id",
        "Name": "/" + (name or service),
        "Config": {
            "Labels": {
                "com.docker.compose.project": PROJECT,
                "com.docker.compose.service": service,
            },
            "Env": env or [],
        },
        "State": {"Running": running},
        "Mounts": mounts or [],
        "NetworkSettings": {"Ports": ports or {}, "Networks": {PROJECT + "_test": {}}},
    }


def network():
    return {
        "Name": PROJECT + "_test",
        "Internal": True,
        "Labels": {"com.docker.compose.project": PROJECT},
        "Containers": {},
    }


class ProjectInspectionTests(unittest.TestCase):
    def test_installed_native_producer_rejects_healthcheck_and_post_sample_drift(self):
        from scripts import p0c4_source_isolation as source
        import copy,json
        cid='a'*64;nid='b'*64;iid='sha256:'+'c'*64
        pg=container('pg',running=True);pg['Id']=cid
        pg['Config']['Image']=source.MANAGER_IMAGE
        pg['State'].update(Pid=42,Status='running',OOMKilled=False,StartedAt='epoch')
        pg.update(Image=iid,RestartCount=0)
        pg['NetworkSettings']['Networks']={PROJECT+'_test':dict(NetworkID=nid)}
        pg['Mounts']=[dict(Type='volume',Name=name,Source='/volumes/'+name,Destination=target,RW=rw) for name,target,rw in (
            ('pg','/var/lib/postgresql',True),('source','/var/lib/knowweave-source',True),('build','/target',False),('registry','/var/lib/knowweave-c4/registry',True))]
        pg['Mounts'] += [dict(Type='bind',Source='/root/'+name,Destination=target,RW=False) for name,target in (
            ('init','/docker-entrypoint-initdb.d/10-lifecycle.sh'),('pg','/run/secrets/postgres_password'),('admin','/run/secrets/admin_password'))]
        net=network();net['Containers']={cid:{}}
        sample='\n'.join(['pid:[123]','mnt:[124]','1 (postgres) S '+'0 '*18+'55 0','/usr/lib/postgresql/18/bin/postgres','1|2','3|4|999','d'*64+'  /usr/lib/postgresql/18/bin/pg_dump','pg_dump (PostgreSQL) 18.6 (Debian 18.6-1.pgdg12+1)'])+'\n'
        def observe(before,after):
            observations=iter((before,after))
            def docker(*args):
                if args==('info','--format','{{.ID}}'):return 'daemon\n'
                if args==('container','inspect',cid):return json.dumps([next(observations)])
                if args==('network','inspect',nid):return json.dumps([net])
                if args==('image','inspect',iid):return json.dumps([dict(Id=iid,RepoDigests=['postgres@'+source.MANAGER_IMAGE.split('@')[1]])])
                self.assertEqual(args,('exec','--user','999:999',cid,'/usr/bin/env','-i','LC_ALL=C','/bin/sh','-ec',source.SOURCE_NATIVE_SAMPLER))
                return sample
            return source.observe_source_endpoint(cid,PROJECT,'11111111-1111-4111-8111-111111111111',docker=docker)
        self.assertEqual(observe(pg,pg)['native']['postmaster_start_ticks'],55)
        drift=copy.deepcopy(pg);drift['State']['Pid']=43
        with self.assertRaises(IsolationError):observe(pg,drift)
        health=copy.deepcopy(pg);health['Config']['Healthcheck']={'Test':['CMD','psql']}
        with self.assertRaises(IsolationError):observe(health,health)

    def test_native_sampler_requires_actual_pid1_namespace_epoch_socket_owner_and_18_6(self):
        from scripts import p0c4_source_isolation as source
        rows=['pid:[123]','mnt:[124]','1 (postgres) S '+'0 '*18+'55 0','/usr/lib/postgresql/18/bin/postgres','1|2','3|4|999','a'*64+'  /usr/lib/postgresql/18/bin/pg_dump','pg_dump (PostgreSQL) 18.6 (Debian 18.6-1.pgdg12+1)']
        actual=source._source_native_sample('\n'.join(rows)+'\n')
        self.assertEqual(actual['native'],dict(pid_namespace='pid:[123]',mount_namespace='mnt:[124]',postmaster_start_ticks=55,data_dev=1,data_ino=2,socket_dev=3,socket_ino=4))
        for index,value in ((0,'pid:[0]'),(1,'pid:[124]'),(2,'1 (postgres) Z '+'0 '*18+'55 0'),(3,'/bin/sh'),(5,'3|4|0'),(7,'pg_dump (PostgreSQL) 18.7 (Debian 18.7-1.pgdg12+1)')):
            bad=rows.copy();bad[index]=value
            with self.subTest(index=index),self.assertRaises(IsolationError):source._source_native_sample('\n'.join(bad)+'\n')

    def test_native_source_mounts_accept_exact_seven_and_reject_extra_or_writable_secret(self):
        from scripts import p0c4_source_isolation as source
        validate=getattr(source,'_source_mount_projection',None)
        self.assertIsNotNone(validate,'shipping native source mount verifier is missing')
        mounts=[dict(Type='volume',Name=name,Source='/var/lib/docker/volumes/'+name+'/_data',Destination=target,RW=rw) for name,target,rw in (
            ('pg','/var/lib/postgresql',True),('source','/var/lib/knowweave-source',True),('build','/target',False),('registry','/var/lib/knowweave-c4/registry',True))]
        mounts += [dict(Type='bind',Source='/root/case/'+name,Destination=target,RW=False) for name,target in (
            ('initdb.sh','/docker-entrypoint-initdb.d/10-lifecycle.sh'),('postgres_password','/run/secrets/postgres_password'),('admin_password','/run/secrets/admin_password'))]
        self.assertEqual(len(validate(mounts)),7)
        import copy
        for bad in (mounts+[mounts[0]],mounts[:-1]):
            with self.assertRaises(IsolationError):validate(bad)
        bad=copy.deepcopy(mounts);bad[-1]['RW']=True
        with self.assertRaises(IsolationError):validate(bad)
        bad=copy.deepcopy(mounts);bad[1]['Source']='/root/../other'
        with self.assertRaises(IsolationError):validate(bad)

    def test_only_postgres_may_run_and_network_must_be_internal(self):
        facts = assess_project(
            PROJECT,
            [container("pg", running=True), container("runtime"), container("worker")],
            [network()],
        )
        self.assertEqual(facts["runtime_running"], 0)
        self.assertEqual(facts["worker_running"], 0)
        self.assertTrue(facts["network_internal"])
        for service in ("runtime", "worker", "unknown"):
            with self.subTest(service=service):
                with self.assertRaises(IsolationError):
                    assess_project(PROJECT, [container("pg", running=True), container(service, running=True)], [network()])

    def test_published_pg_port_and_external_network_fail(self):
        with self.assertRaises(IsolationError):
            assess_project(PROJECT, [container("pg", running=True, ports={"5432/tcp": [{"HostPort": "15432"}]})], [network()])
        external = network()
        external["Internal"] = False
        with self.assertRaises(IsolationError):
            assess_project(PROJECT, [container("pg", running=True)], [external])

    def test_manager_runtime_credential_mount_or_environment_fails(self):
        mount = {"Destination": "/run/secrets/learning_runtime_password"}
        with self.assertRaises(IsolationError):
            assess_project(PROJECT, [container("pg", running=True), container("c4-manager", mounts=[mount])], [network()])
        with self.assertRaises(IsolationError):
            assess_project(PROJECT, [container("pg", running=True), container("c4-manager", env=["DATABASE_URL=redacted"])], [network()])

    def test_only_driver_named_manager_may_run_during_capture(self):
        manager = container("c4-manager", running=True, name="reviewed-manager")
        with self.assertRaises(IsolationError):
            assess_project(PROJECT, [container("pg", running=True), manager], [network()])
        facts = assess_project(PROJECT, [container("pg", running=True), manager], [network()], allowed_manager_name="reviewed-manager")
        self.assertEqual(facts["other_admin_processes"], 0)

    def test_missing_or_mismatched_project_is_rejected(self):
        with self.assertRaises(IsolationError):
            assess_project(PROJECT, [container("pg", running=True)], [])
        bad = container("pg", running=True)
        bad["Config"]["Labels"]["com.docker.compose.project"] = "other"
        with self.assertRaises(IsolationError):
            assess_project(PROJECT, [bad], [network()])

    def test_external_attachment_fails_even_if_project_network_is_internal(self):
        pg = container("pg", running=True)
        pg["NetworkSettings"]["Networks"]["external"] = {}
        with self.assertRaises(IsolationError):
            assess_project(PROJECT, [pg], [network()])

    @patch("scripts.p0c4_source_isolation.subprocess.run")
    def test_runtime_probe_accepts_only_database_connect_denial(self, run):
        run.return_value = CompletedProcess([], 2, "", "FATAL: permission denied for database \"x\"")
        probe_runtime_denied("pg-id", "learning_backup_c4_task3_1234")
        run.return_value = CompletedProcess([], 2, "", "could not connect to server")
        with self.assertRaises(IsolationError):
            probe_runtime_denied("pg-id", "learning_backup_c4_task3_1234")
        run.return_value = CompletedProcess([], 0, "1", "")
        with self.assertRaises(IsolationError):
            probe_runtime_denied("pg-id", "learning_backup_c4_task3_1234")

    def test_manager_environment_is_explicit_and_rejects_runtime_secrets(self):
        inherited = {
            "TEST_ADMIN_DATABASE_URL": "postgres://admin:secret@127.0.0.1/db",
            "TEST_C4_CONTROL_ROOT": "/root/control",
            "TEST_C4_UNREVIEWED": "not passed",
            "PATH": "/attacker",
        }
        env = manager_environment(inherited)
        self.assertIn("TEST_ADMIN_DATABASE_URL", env)
        self.assertIn("TEST_C4_CONTROL_ROOT", env)
        self.assertNotIn("TEST_C4_UNREVIEWED", env)
        self.assertNotEqual(env["PATH"], "/attacker")
        inherited["TEST_C4_RUNTIME_PASSWORD"] = "secret"
        with self.assertRaises(IsolationError):
            manager_environment(inherited)
        inherited.pop("TEST_C4_RUNTIME_PASSWORD")
        inherited["TEST_DATABASE_URL"] = "postgres://runtime:secret@pg/db"
        with self.assertRaises(IsolationError):
            manager_environment(inherited)

    def test_admin_endpoint_is_inside_pg_namespace_only(self):
        db = "learning_backup_c4_task3_550e8400-e29b-41d4-a716-446655440000"
        validate_admin_endpoint(f"postgres://learning_admin:secret@pg:5432/{db}?application_name=knowweave_c4_manager", db)
        with self.assertRaises(IsolationError):
            validate_admin_endpoint(f"postgres://learning_admin:secret@127.0.0.1:5432/{db}?application_name=knowweave_c4_manager", db)
        with self.assertRaises(IsolationError):
            validate_admin_endpoint("postgres://learning_admin:secret@pg:5432/other?application_name=knowweave_c4_manager", db)

    @patch("scripts.p0c4_source_isolation.subprocess.run")
    def test_daemon_identity_must_be_available(self, run):
        run.return_value = CompletedProcess([], 1, "", "daemon unavailable")
        with self.assertRaises(IsolationError):
            _daemon_preflight()
        self.assertEqual(run.call_count, 1)

    @patch("scripts.p0c4_source_isolation._rename_noreplace")
    def test_evidence_file_is_fully_written_before_no_replace_publication(self, rename):
        import os
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "probe.json"
            def publish(source, destination):
                if Path(destination).exists():
                    raise FileExistsError(destination)
                self.assertEqual(Path(source).read_bytes(), b"complete-proof")
                os.link(source, destination)
                os.unlink(source)
            rename.side_effect = publish
            _atomic_private_file(target, b"complete-proof")
            self.assertEqual(target.read_bytes(), b"complete-proof")
            with self.assertRaises(FileExistsError):
                _atomic_private_file(target, b"different")

    def test_redaction_hides_admin_url_and_password(self):
        raw = b"postgres://learning_admin:secret@pg/db secret harmless"
        self.assertEqual(redact_log(raw, [b"postgres://learning_admin:secret@pg/db", b"secret"]),
                         b"[REDACTED] [REDACTED] harmless")

    @patch("scripts.p0c4_source_isolation._rename_noreplace")
    def test_manager_logs_are_retained_privately_and_redacted_for_evidence(self, rename):
        import os
        import tempfile
        from pathlib import Path
        rename.side_effect = os.replace
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pgpass = root / "admin.pgpass"
            pgpass.write_bytes(b"pg:5432:db:learning_admin:secret\n")
            stdout = _private_capture(root / "manager-id.stdout.raw")
            stdout.write(b"connect postgres://learning_admin:secret@pg/db\n")
            stderr = _private_capture(root / "manager-id.stderr.raw")
            stderr.write(b"password=secret\n")
            _publish_manager_logs(root, "id", {"stdout": stdout, "stderr": stderr}, {
                "TEST_ADMIN_DATABASE_URL": "postgres://learning_admin:secret@pg/db",
                "TEST_C4_PGPASSFILE": str(pgpass),
            })
            self.assertIn(b"[REDACTED]", (root / "manager-id.stdout.redacted.log").read_bytes())
            self.assertNotIn(b"secret", (root / "manager-id.stderr.redacted.log").read_bytes())
            self.assertIn(b"secret", (root / "manager-id.stderr.raw").read_bytes())


if __name__ == "__main__":
    unittest.main()


class CompletionProjectTests(unittest.TestCase):
    def test_fixed_completion_pattern_preserves_legacy_admission(self):
        from scripts.p0c4_source_isolation import accepted_project
        self.assertTrue(accepted_project(PROJECT))
        self.assertTrue(accepted_project('kwc4c-'+'a'*32))
        for value in ('kwc4c-'+'A'*32,'kwc4c-'+'a'*31,'kwc4c-'+'a'*32+'/x','kwc4c-',None):self.assertFalse(accepted_project(value))
