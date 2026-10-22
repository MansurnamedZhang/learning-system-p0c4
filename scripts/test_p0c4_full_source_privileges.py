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



class FullCSourceTempFixtureBehavior(unittest.TestCase):
    def context(self, case, full_c=True):
        from scripts.p0c4_completion import full_import as full
        plan=dict(case_name=case,case_id=DATABASES[0].removeprefix('learning_backup_c4_task3_'))
        if full_c:plan['schema_profile']='snapshot_c0_d1_v1'
        if case in full.SNAPSHOT_CASE_TESTS:
            plan.update(format_version=3,snapshot_profile='root_snapshot_broker_v2')
        else:plan['format_version']=2 if full_c else 1
        return full.FullRehearsalContext(full._TOKEN,plan)

    def record(self):
        ident=dict(IDENTITY,case_id=DATABASES[0].removeprefix('learning_backup_c4_task3_'))
        return ident,dict(identity=ident.copy(),container_id=CONTAINER,stage='independent-consumer-build')

    def prepare(self, runner, context, ident=None, record=None):
        if ident is None:ident,record=self.record()
        return DRIVER._prepare_fresh_full_c_source_role(runner,CONTAINER,ident,context,record),record

    def test_all_sixteen_fresh_full_c_cases_pay_owner_temp_transaction_before_broker_create(self):
        from scripts.p0c4_completion import full_import as full
        cases=[*full.RECOVERY_CASE_TESTS,*full.SNAPSHOT_CASE_TESTS]
        self.assertEqual(len(cases),16)
        for case in cases:
            with self.subTest(case=case):
                runner=RecordingRunner([(0,b'0\n',b''),(0,b'FULL_C_SOURCE_TEMP_READY\n',b''),(0,b'',b'')])
                prepared,record=self.prepare(runner,self.context(case))
                self.assertTrue(prepared)
                self.assertEqual(len(runner.calls),3,'owner TEMP transaction must precede broker creation')
                self.assertEqual(runner.replies,[])
                self.assertEqual(runner.calls[0][0],command('postgres','postgres'))
                self.assertIn(b'SELECT count(*)',runner.calls[0][1]['stdin'])
                argv,options=runner.calls[1]
                self.assertEqual(argv,command(DATABASES[0],'learning_admin'))
                self.assertEqual(options['timeout'],5)
                self.assertEqual(options['allowed'],(0,))
                statement=options['stdin'].decode()
                self.assertTrue(statement.startswith('BEGIN;'))
                self.assertTrue(statement.endswith('COMMIT;'))
                self.assertIn("current_database()<>'"+DATABASES[0]+"'",statement)
                self.assertNotIn(DATABASES[1],statement)
                for guard in ("session_user<>'learning_admin'","current_user<>'learning_admin'",'pg_catalog.pg_class','pg_catalog.pg_proc',"rolname='learning_snapshot_claim_broker'","current_setting('server_version_num')::int/10000<>18",'d.datdba','has_database_privilege','TEMPORARY'):
                    self.assertIn(guard,statement)
                grant='GRANT TEMPORARY ON DATABASE %I TO learning_admin,learning_runtime'
                revoke='REVOKE TEMPORARY ON DATABASE %I FROM PUBLIC'
                self.assertLess(statement.index(grant),statement.index(revoke))
                self.assertLess(statement.index(revoke),statement.index('FULL_C_SOURCE_TEMP_READY'))
                for wide in ('GRANT ALL','WITH GRANT OPTION','GRANT CREATE','GRANT CONNECT','ALTER ROLE','GRANT EXECUTE'):
                    self.assertNotIn(wide,statement)
                self.assertEqual(statement.count('GRANT '),1)
                self.assertEqual(statement.count('REVOKE '),1)
                self.assertEqual(statement.count('a.grantor<>d.datdba'),2)
                self.assertEqual(statement.count('a.is_grantable'),2)
                import re
                self.assertEqual(re.findall(r"EXECUTE format\('([^']+)',current_database\(\)\);",statement),[grant,revoke])
                self.assertEqual(runner.calls[2][0],command('postgres','postgres'))
                self.assertEqual(runner.calls[2][1]['stdin'],b'CREATE ROLE learning_snapshot_claim_broker LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 4;')
                self.assertTrue(record['full_c_source_role_created_fresh'])
                self.assertEqual(record['stage'],'source-full-c-fresh-privileges')

    def test_legacy_none_and_non_c_full_do_not_execute_temp_or_broker_sql(self):
        for context in (None,self.context('recovery-leases',False)):
            runner=RecordingRunner([])
            prepared,record=self.prepare(runner,context)
            self.assertFalse(prepared)
            self.assertEqual(runner.calls,[])
            self.assertEqual(record['stage'],'independent-consumer-build')
            self.assertNotIn('full_c_source_role_created_fresh',record)

    def test_existing_broker_refuses_before_owner_acl_transaction(self):
        for output in (b'1\n',b'2\n',b'',b'0|1\n'):
            with self.subTest(output=output):
                runner=RecordingRunner([(0,output,b'')])
                with self.assertRaisesRegex(DRIVER.GateError,'FULL_C_SOURCE_ROLE_MUST_BE_ABSENT'):
                    self.prepare(runner,self.context('snapshot-p6'))
                self.assertEqual(len(runner.calls),1)
                self.assertNotIn(b'GRANT ',runner.calls[0][1]['stdin'])

    def test_unknown_nonfresh_or_owner_transaction_failure_never_creates_broker(self):
        replies=[(1,b'',b'fresh database required'),(0,b'',b''),(0,b'FULL_C_SOURCE_TEMP_READY\n',b'warning'),(0,b'other\n',b'')]
        for reply in replies:
            with self.subTest(reply=reply):
                runner=RecordingRunner([(0,b'0\n',b''),reply])
                ident,record=self.record()
                with self.assertRaisesRegex(DRIVER.GateError,'FULL_C_SOURCE_TEMP_PRIVILEGES'):
                    self.prepare(runner,self.context('snapshot-p6'),ident,record)
                self.assertEqual(len(runner.calls),2)
                self.assertEqual(runner.calls[1][0],command(DATABASES[0],'learning_admin'))
                self.assertNotIn('full_c_source_role_created_fresh',record)
                self.assertEqual(record['stage'],'source-full-c-fresh-privileges')

    def test_wrong_current_database_case_or_container_binding_refuses_without_sql(self):
        for change in ('database','context_case','container'):
            with self.subTest(change=change):
                ident,record=self.record();context=self.context('snapshot-p6')
                if change=='database':ident['database']=DATABASES[1];record['identity']=ident.copy()
                elif change=='context_case':context._plan['case_id']=DATABASES[1].removeprefix('learning_backup_c4_task3_')
                else:record['container_id']='2'*64
                runner=RecordingRunner([])
                with self.assertRaisesRegex(DRIVER.GateError,'FULL_C_SOURCE_FRESH_BINDING'):
                    self.prepare(runner,context,ident,record)
                self.assertEqual(runner.calls,[])


if __name__ == '__main__':
    unittest.main()
