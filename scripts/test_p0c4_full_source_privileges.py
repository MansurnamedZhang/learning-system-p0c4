"""Local behavior checks; recording transport is not native PG proof."""
import importlib.util
from pathlib import Path
import unittest


FILE = Path(__file__).with_name('p0c4_source_lifecycle_gate_acceptance.py')
SPEC = importlib.util.spec_from_file_location('c2_lifecycle_driver', FILE)
DRIVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DRIVER)

CONTAINER = '1' * 64
DATABASES = ('learning_backup_c4_task3_11111111-1111-4111-8111-111111111111',
             'learning_backup_c4_task3_22222222-2222-4222-8222-222222222222')
IDENTITY = dict(database=DATABASES[0], other_database=DATABASES[1])
FULL_CONTEXT = object()  # live_case validates its actual context before dispatch.
OK = (0, b't|t\n', b'')


class RecordingRunner:
    def __init__(self, replies):
        self.replies = list(replies)
        self.calls = []

    def run(self, command, **kwargs):
        self.calls.append((command, kwargs))
        if not self.replies:
            raise AssertionError('unexpected native command or fallback')
        reply = self.replies.pop(0)
        if isinstance(reply, Exception):
            raise reply
        return reply


def command(database, role):
    return ['/usr/bin/docker', 'exec', '-i', '--user', '0:0', CONTAINER,
            '/usr/bin/env', '-i', 'PATH=/usr/bin:/bin', 'HOME=/root', 'LC_ALL=C',
            'psql', '-X', '-qAt', '-v', 'ON_ERROR_STOP=1', '-U', role, '-d', database]


class FullSourcePrivilegeBehavior(unittest.TestCase):
    def prepare(self, runner, context=FULL_CONTEXT):
        DRIVER._prepare_source_control_system_privileges(
            runner, CONTAINER, IDENTITY, context)

    def assert_full_call(self, call, database):
        argv, options = call
        self.assertEqual(argv, command(database, 'learning_admin'))
        self.assertEqual(options['timeout'], 5)
        self.assertEqual(options['allowed'], (0,))
        statement = options['stdin']
        self.assertIs(type(statement), bytes)
        self.assertIn(b'BEGIN READ ONLY;', statement)
        self.assertIn(b'pg_catalog.pg_proc', statement)
        self.assertIn(b'proacl IS NULL', statement)
        self.assertIn(b'EXECUTE', statement)
        self.assertIn(b'pg_catalog.pg_control_system()', statement)
        self.assertIn(b'ROLLBACK;', statement)
        for mutation in (b'GRANT ', b'REVOKE ', b'ALTER ', b'CREATE ', b'DROP '):
            self.assertNotIn(mutation, statement)

    def assert_rejected(self, reply, failing_index):
        runner = RecordingRunner([OK] * failing_index + [reply])
        with self.assertRaises(DRIVER.GateError):
            self.prepare(runner)
        self.assertEqual(len(runner.calls), failing_index + 1)
        self.assertEqual(runner.replies, [])
        for call, database in zip(runner.calls, DATABASES):
            self.assert_full_call(call, database)

    def test_legacy_none_keeps_exact_two_database_grant_transcript(self):
        runner = RecordingRunner([(0, b'', b''), (0, b'', b'')])
        self.prepare(runner, None)
        statement = b'GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO learning_admin;'
        expected = [(command(database, 'postgres'),
                     dict(stdin=statement, timeout=5, allowed=(0,)))
                    for database in DATABASES]
        self.assertEqual(runner.calls, expected)
        self.assertEqual(runner.replies, [])

    def test_full_probes_both_databases_as_admin_without_materializing_acl(self):
        runner = RecordingRunner([OK, OK])
        self.prepare(runner)
        self.assertEqual(len(runner.calls), 2)
        self.assertEqual(runner.replies, [])
        for call, database in zip(runner.calls, DATABASES):
            self.assert_full_call(call, database)

    def test_full_refuses_nondefault_acl_on_either_database(self):
        for index in (0, 1):
            with self.subTest(database=DATABASES[index]):
                self.assert_rejected((0, b'f|t\n', b''), index)

    def test_full_refuses_missing_execute_on_either_database(self):
        for index in (0, 1):
            with self.subTest(database=DATABASES[index]):
                self.assert_rejected((0, b't|f\n', b''), index)

    def test_full_refuses_missing_or_malformed_probe_on_either_database(self):
        malformed = (b'', b'|t\n', b't|\n', b't|true\n', b't|t|f\n',
                     b't|t\nt|t\n', b't|t', b't|t\r\n', b' t|t\n', b'\xff\n')
        for index in (0, 1):
            for output in malformed:
                with self.subTest(database=DATABASES[index], output=output):
                    self.assert_rejected((0, output, b''), index)

    def test_full_refuses_nonzero_native_exit_on_either_database(self):
        for index in (0, 1):
            with self.subTest(database=DATABASES[index]):
                self.assert_rejected((2, b't|t\n', b''), index)

    def test_full_refuses_native_stderr_on_either_database(self):
        for index in (0, 1):
            with self.subTest(database=DATABASES[index]):
                self.assert_rejected((0, b't|t\n', b'bounded native failure'), index)

    def test_full_propagates_native_failure_without_grant_or_role_fallback(self):
        for index in (0, 1):
            with self.subTest(database=DATABASES[index]):
                self.assert_rejected(DRIVER.GateError('NATIVE_COMMAND_FAILED'), index)


if __name__ == '__main__':
    unittest.main()
