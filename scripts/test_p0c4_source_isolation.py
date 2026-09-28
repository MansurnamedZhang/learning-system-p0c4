"""Pure checks for the root-only C4 maintenance driver; no Docker required."""

import unittest
from unittest.mock import patch
from subprocess import CompletedProcess

from p0c4_source_isolation import IsolationError, assess_project, probe_runtime_denied, manager_environment, validate_admin_endpoint, _daemon_preflight


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

    @patch("p0c4_source_isolation.subprocess.run")
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

    @patch("p0c4_source_isolation.subprocess.run")
    def test_daemon_identity_must_be_available(self, run):
        run.return_value = CompletedProcess([], 1, "", "daemon unavailable")
        with self.assertRaises(IsolationError):
            _daemon_preflight()
        self.assertEqual(run.call_count, 1)


if __name__ == "__main__":
    unittest.main()
