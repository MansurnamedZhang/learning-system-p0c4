"""Finite current case routing; local contracts confer no native/PG credit."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import p0c4_controlled_import_acceptance as runner
from p0c4_completion import controlled_fixture as fixture
import test_p0c4_controlled_fixture_context as context_tests
import test_p0c4_controlled_import_acceptance as import_tests


CASES = {
    'ready-eof': 'live_controlled_import_ready_eof',
    'precommit-eof': 'live_controlled_import_precommit_eof',
    'precommit-cancel': 'live_controlled_import_cancel_before_commit',
    'sql-error': 'live_controlled_import_sql_error',
    'copy-truncated': 'live_controlled_import_copy_truncated',
    'attempt-sync-failure': 'live_controlled_import_attempt_sync_failure',
    'commit-intent-sync-failure': 'live_controlled_import_commit_intent_sync_failure',
    'commit-unknown': 'live_controlled_import_commit_confirmation_lost',
}
PREFIX = 'restore_preflight::target_binding::controlled_import::live_tests::'


def plan(case):
    value = context_tests.ContextPlanTests().plan()
    if case != 'success':
        value.update(capability='c4_current11_two_role_case_plan_v1', case=case,
                     budget_profile='controlled_fixture_two_role_fixed_case_v1')
    return value


class FiniteCaseTests(unittest.TestCase):
    def test_issuer_unknown_or_nonstring_case_rejects_before_filesystem(self):
        for case in ('wrong-endpoint', 'unknown', True, []):
            with self.subTest(case=case), patch.object(fixture, 'Path', side_effect=AssertionError('filesystem reached')), \
                 self.assertRaisesRegex(fixture.ControlledRejected, 'CONTROLLED_FIXED_CASE_ONLY'):
                fixture._mint_plan_from_install(case)

    def test_each_negative_needs_exact_zero_exit_receipt_and_confirmed_isolation(self):
        samples = import_tests.ImportPhaseTests()
        expected = {'ready-eof': ('Protocol', True), 'precommit-eof': ('Protocol', True),
            'precommit-cancel': ('Cancelled', True), 'sql-error': ('Exit', False),
            'copy-truncated': ('Exit', False), 'attempt-sync-failure': ('Journal', False),
            'commit-intent-sync-failure': ('Journal', False), 'commit-unknown': ('CommitUnknown', False)}
        for case, (failure, rollback) in expected.items():
            record = samples.receipt(case); output = samples.output(case, record)
            observed = runner._import_observations(output, case, 0)
            self.assertEqual(observed['failure'], failure)
            self.assertIs(observed['rollback_verified'], rollback)
            self.assertIs(observed['commit_attempted'], case == 'commit-unknown')
            self.assertIs(observed['retry_allowed'], False); self.assertIsNone(observed['content_sha256'])
            for code in (1, -9, True, None):
                with self.subTest(case=case, code=code), self.assertRaises(runner.ImportRejected):
                    runner._import_observations(output, case, code)
            for bad in (samples.output(case, samples.receipt('success')),
                    output.replace(b'1 passed', b'0 passed'), output.replace(b'0 ignored', b'1 ignored'),
                    output + b'thread panicked\n', output + output,
                    samples.output(case, dict(record, retry_allowed=True)),
                    samples.output(case, dict(record, schema=True)),
                    samples.output(case, dict(record, checkpoints=record['checkpoints']+['EXTRA']))):
                with self.subTest(case=case, malformed=True), self.assertRaises(runner.ImportRejected):
                    runner._import_observations(bad, case, 0)
            for key in ('source_stopped', 'target_stopped', 'volumes_retained'):
                isolation = dict(source_stopped=True, target_stopped=True, volumes_retained=True)
                isolation[key] = False
                with self.subTest(case=case, key=key), self.assertRaises(runner.ImportRejected):
                    runner._validate_import_isolation(dict(case=case, isolation=isolation))

    def test_install_accepts_two_exact_profiles_and_rejects_unknown_before_issuance(self):
        value = dict(format_version=1, capability='c4_current11_install_v1',
            **{k: plan('ready-eof')[k] for k in ('source_commit', 'public_archive_sha256',
                'public_manifest_sha256', 'legacy_archive_sha256', 'legacy_manifest_sha256',
                'builder', 'postgres', 'budget_profile')},
            adapter_sha256='a'*64, docker_sha256='b'*64, compose_sha256='c'*64)
        for profile in ('controlled_fixture_two_role_first_success_v1', 'controlled_fixture_two_role_fixed_case_v1'):
            value['budget_profile'] = profile
            with patch.object(fixture, '_private_bytes', return_value=(fixture._canonical(value), {})):
                try: actual, _ = fixture._read_install(Path('/private/suite'))
                except fixture.ControlledRejected as error: self.fail('reviewed install profile rejected: '+str(error))
                self.assertEqual(actual, value)
        value['budget_profile'] = 'unbounded'
        with patch.object(fixture, '_private_bytes', return_value=(fixture._canonical(value), {})), self.assertRaises(fixture.ControlledRejected):
            fixture._read_install(Path('/private/suite'))

    def test_new_lane_accepts_exact_eight_and_preserves_success_lane(self):
        for case in ('success', *CASES):
            with self.subTest(case=case):
                value = plan(case)
                try: actual = fixture._validate_plan(value)
                except fixture.ControlledRejected as error: self.fail('fixed reviewed case rejected: '+str(error))
                self.assertEqual(actual, value)
        for case in ('success', *CASES):
            value = plan(case)
            for field, wrong in (('case', 'ready-restart'), ('case', 'wrong-endpoint'),
                    ('case', 'full-import'), ('case', 'unknown'), ('case', True), ('case', []),
                    ('budget_profile', 'unbounded'), ('format_version', True)):
                with self.subTest(case=case, field=field, wrong=wrong), self.assertRaises(fixture.ControlledRejected):
                    fixture._validate_plan(dict(value, **{field: wrong}))
            for field in ('command', 'test', 'env', 'sql', 'root'):
                with self.assertRaises(fixture.ControlledRejected):
                    fixture._validate_plan(dict(value, **{field: 'caller'}))
        for value in (dict(plan('success'), case='ready-eof'), dict(plan('ready-eof'), case='success'),
                      dict(plan('ready-eof'), budget_profile='controlled_fixture_two_role_first_success_v1')):
            with self.assertRaises(fixture.ControlledRejected): fixture._validate_plan(value)

    def test_actor_accepts_only_closed_fixed_operations_and_exact_arguments(self):
        suite = Path('/private/suite'); adapter = str(suite/'current11/adapter.py')
        for operation in ('--fixed-prepare-case', '--fixed-run-case'):
            for case in CASES:
                argv = [adapter, operation, case]
                with self.subTest(argv=argv), patch.object(sys, 'argv', argv):
                    try: actual = fixture._actor_main_contract(suite)
                    except fixture.ControlledRejected as error: self.fail('fixed argv rejected: '+str(error))
                    self.assertEqual(actual, ('python3', ['-B', '-u', *argv]))
            for tail in ([], ['success'], ['wrong-endpoint'], ['unknown'],
                         ['ready-eof', '--env'], ['ready-eof', '/plan.json']):
                with patch.object(sys, 'argv', [adapter, operation, *tail]), self.assertRaises(fixture.ControlledRejected):
                    fixture._actor_main_contract(suite)
        with patch.object(sys, 'argv', ['/foreign/adapter.py', '--fixed-run-case', 'ready-eof']), self.assertRaises(fixture.ControlledRejected):
            fixture._actor_main_contract(suite)

    def test_backend_wrong_plan_case_rejects_before_runtime_and_budget(self):
        class RuntimeReached(Exception): pass
        backend = runner.ImportBackend.__new__(runner.ImportBackend)
        context = fixture.ControlledFixtureContext.__new__(fixture.ControlledFixtureContext)
        backend._controlled_context = context
        for selected in ('success', *CASES):
            context._plan = plan(selected)
            with patch.object(fixture.ControlledFixtureContext, '_runtime', side_effect=RuntimeReached), \
                 patch.object(backend, '_budget', side_effect=AssertionError('budget reached before binding')):
                with self.subTest(selected=selected), self.assertRaises(RuntimeReached):
                    try: backend.execute_import(selected)
                    except fixture.ControlledRejected as error: self.fail('matching case rejected: '+str(error))
                for wrong in ('success', *CASES, 'ready-restart', 'wrong-endpoint', 'unknown', True, []):
                    if wrong == selected: continue
                    with self.subTest(selected=selected, wrong=wrong), self.assertRaises(fixture.ControlledRejected):
                        try: backend.execute_import(wrong)
                        except RuntimeReached: self.fail('wrong plan case reached runtime')

    def test_native_consumer_uses_pinned_case_for_exact_command_and_clean_environment(self):
        consume = getattr(fixture, '_run_native_body', None)
        self.assertTrue(callable(consume), 'missing finite native body consumer')
        for case, suffix in {'success': 'live_controlled_import_commits_fixture', **CASES}.items():
            value = plan(case); batch = Path('/private/batch'); binary = batch/'probe'
            calls = []
            def run(command, **kwargs):
                calls.append((command, kwargs))
                return subprocess.CompletedProcess(command, 0, b'bounded transcript', b'')
            result = consume(runner, SimpleNamespace(_run_bounded=run), batch/'source', batch, value, binary)
            self.assertEqual(result.returncode, 0)
            self.assertEqual(calls, [([str(binary), '--exact', PREFIX+suffix, '--ignored', '--nocapture', '--test-threads=1'],
                dict(cwd=batch/'source', timeout=360, limit=65536, env={
                    'PATH': '/usr/bin:/bin', 'HOME': str(batch), 'LC_ALL': 'C',
                    'TMPDIR': str(batch/'tmp'), 'TMP': str(batch/'tmp'), 'TEMP': str(batch/'tmp'),
                    'KNOWWEAVE_C4_IMPORT_CASE': case,
                    'KNOWWEAVE_C4_IMPORT_SOURCE_ROOT': str(batch/'source-control'/'targets'/value['source_case_id']),
                    'KNOWWEAVE_C4_IMPORT_TARGET_ROOT': str(batch/'target-control'/'targets'/value['target_case_id']),
                    'KNOWWEAVE_C4_IMPORT_ARTIFACT_ROOT': str(batch/'artifacts')}))])

    def test_dispatch_records_case_exact_test_binary_and_is_once_only(self):
        with tempfile.TemporaryDirectory() as folder:
            batch = Path(folder)
            def write(path, data):
                with path.open('xb') as stream: stream.write(data)
            context = fixture.ControlledFixtureContext.__new__(fixture.ControlledFixtureContext)
            context._plan = plan('commit-unknown'); context._body_used = False
            context._backend = SimpleNamespace(batch=batch, helper=SimpleNamespace(_private_write=write))
            context._sides = {'source': {'status': {'birth_sha256': 'a'*64}}, 'target': {'status': {'birth_sha256': 'b'*64}}}
            context._compiled = {'live': dict(source_birth_sha256='a'*64, target_birth_sha256='b'*64, binary_sha256='c'*64)}
            with patch.object(fixture.ControlledFixtureContext, '_alive'), \
                 patch.object(fixture.ControlledFixtureContext, '_native', return_value='transport'):
                self.assertEqual(context._execute_body(), 'transport')
                raw = (batch/'body.dispatched').read_bytes(); record = json.loads(raw)
                self.assertEqual(record.get('case'), 'commit-unknown')
                self.assertEqual(record.get('exact_test'), PREFIX+'live_controlled_import_commit_confirmation_lost')
                self.assertEqual(record['binary_sha256'], 'c'*64)
                with self.assertRaises(fixture.ControlledRejected): context._execute_body()
                self.assertEqual((batch/'body.dispatched').read_bytes(), raw)


if __name__ == '__main__': unittest.main()
