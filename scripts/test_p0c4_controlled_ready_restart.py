"""Protected restart routing and cleanup contracts; no native/PG acceptance."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import p0c4_controlled_import_acceptance as runner
import p0c4_restore_birth_acceptance as birth
from p0c4_completion import controlled_fixture as fixture
import test_p0c4_controlled_fixture_context as context_tests
from test_p0c4_restore_target import ID, created, identity_for


CASE = 'ready-restart'
CAPABILITY = 'c4_current11_ready_restart_plan_v1'
PROFILE = 'controlled_fixture_two_role_ready_restart_v1'
TEST = 'restore_preflight::target_binding::controlled_import::live_tests::live_controlled_import_restart_before_ddl'


def plan():
    value = context_tests.ContextPlanTests().plan()
    value.update(case=CASE, capability=CAPABILITY, budget_profile=PROFILE)
    return value


class RestartContextTests(unittest.TestCase):
    def test_restart_needs_its_own_case_profile_and_rejects_cross_lane_plans(self):
        value = plan()
        try: self.assertEqual(fixture._validate_plan(value), value)
        except fixture.ControlledRejected as error: self.fail('protected restart lane absent: '+str(error))
        for updates in (
            {'case': 'success'}, {'case': 'ready-eof'}, {'case': 'wrong-endpoint'},
            {'case': 'full-import'}, {'case': True}, {'case': []}, {'case': 'unknown'},
            {'budget_profile': 'controlled_fixture_two_role_fixed_case_v1'},
            {'budget_profile': 'controlled_fixture_two_role_first_success_v1'},
            {'capability': 'c4_current11_two_role_case_plan_v1'},
            {'capability': 'c4_current11_first_success_plan_v1'}, {'command': 'restart'},
        ):
            with self.subTest(updates=updates), self.assertRaises(fixture.ControlledRejected):
                fixture._validate_plan(dict(value, **updates))

    def test_restart_install_profile_is_exact(self):
        record = dict(format_version=1, capability='c4_current11_install_v1',
            **{key: plan()[key] for key in ('source_commit', 'public_archive_sha256',
                'public_manifest_sha256', 'legacy_archive_sha256', 'legacy_manifest_sha256',
                'builder', 'postgres', 'budget_profile')}, adapter_sha256='a'*64,
            docker_sha256='b'*64, compose_sha256='c'*64)
        with patch.object(fixture, '_private_bytes', return_value=(fixture._canonical(record), {})):
            try: observed, _ = fixture._read_install(Path('/private/suite'))
            except fixture.ControlledRejected as error: self.fail('restart install rejected: '+str(error))
        self.assertEqual(observed, record)
        for unknown in (PROFILE+'-unbounded', True, None):
            with patch.object(fixture, '_private_bytes', return_value=(fixture._canonical(dict(record, budget_profile=unknown)), {})), self.assertRaises(fixture.ControlledRejected):
                fixture._read_install(Path('/private/suite'))

    def test_actor_accepts_only_exact_restart_argv(self):
        suite = Path('/private/suite'); adapter = str(suite/'current11/adapter.py')
        for operation in ('--fixed-prepare-case', '--fixed-run-case'):
            argv = [adapter, operation, CASE]
            with patch.object(sys, 'argv', argv):
                try: self.assertEqual(fixture._actor_main_contract(suite), ('python3', ['-B', '-u', *argv]))
                except fixture.ControlledRejected as error: self.fail('restart actor rejected: '+str(error))
            for tail in ([], ['wrong-endpoint'], [CASE, '--env'], [CASE, '/plan.json'], [CASE, 'a'*64]):
                with patch.object(sys, 'argv', [adapter, operation, *tail]), self.assertRaises(fixture.ControlledRejected):
                    fixture._actor_main_contract(suite)

    def test_restart_backend_case_is_bound_before_runtime_or_budget(self):
        class RuntimeReached(Exception): pass
        context = fixture.ControlledFixtureContext.__new__(fixture.ControlledFixtureContext)
        context._plan = plan()
        backend = runner.ImportBackend.__new__(runner.ImportBackend); backend._controlled_context = context
        with patch.object(fixture.ControlledFixtureContext, '_runtime', side_effect=RuntimeReached), \
             patch.object(backend, '_budget', side_effect=AssertionError('budget before case binding')):
            with self.assertRaises(RuntimeReached):
                try: backend.execute_import(CASE)
                except fixture.ControlledRejected as error: self.fail('bound restart rejected: '+str(error))
            for wrong in ('success', 'ready-eof', 'wrong-endpoint', 'full-import', True, []):
                with self.subTest(wrong=wrong), self.assertRaises(fixture.ControlledRejected): backend.execute_import(wrong)

    def test_native_restart_has_fixed_test_environment_and_no_cid_selector(self):
        value = plan(); batch = Path('/private/batch'); binary = batch/'probe'; calls = []
        def run(command, **kwargs):
            calls.append((command, kwargs)); return subprocess.CompletedProcess(command, 0, b'bounded', b'')
        try: result = fixture._run_native_body(runner, SimpleNamespace(_run_bounded=run), batch/'source', batch, value, binary)
        except fixture.ControlledRejected as error: self.fail('restart body consumer rejected: '+str(error))
        self.assertEqual(result.stdout, b'bounded')
        self.assertEqual(calls, [([str(binary), '--exact', TEST, '--ignored', '--nocapture', '--test-threads=1'],
            dict(cwd=batch/'source', timeout=360, limit=65536, env={
                'PATH': '/usr/bin:/bin', 'HOME': str(batch), 'LC_ALL': 'C',
                'TMPDIR': str(batch/'tmp'), 'TMP': str(batch/'tmp'), 'TEMP': str(batch/'tmp'),
                'KNOWWEAVE_C4_IMPORT_CASE': CASE,
                'KNOWWEAVE_C4_IMPORT_SOURCE_ROOT': str(batch/'source-control'/'targets'/value['source_case_id']),
                'KNOWWEAVE_C4_IMPORT_TARGET_ROOT': str(batch/'target-control'/'targets'/value['target_case_id']),
                'KNOWWEAVE_C4_IMPORT_ARTIFACT_ROOT': str(batch/'artifacts')}))])

    def test_dispatch_requires_distinct_original_births_and_is_once_only(self):
        with tempfile.TemporaryDirectory() as folder:
            batch = Path(folder)
            def write(path, data):
                with path.open('xb') as stream: stream.write(data)
            context = fixture.ControlledFixtureContext.__new__(fixture.ControlledFixtureContext)
            context._plan = plan(); context._body_used = False
            context._backend = SimpleNamespace(batch=batch, helper=SimpleNamespace(_private_write=write))
            context._sides = {'source': {'status': {'birth_sha256': 'a'*64}}, 'target': {'status': {'birth_sha256': 'b'*64}}}
            context._compiled = {'live': dict(source_birth_sha256='a'*64, target_birth_sha256='b'*64, binary_sha256='c'*64)}
            with patch.object(fixture.ControlledFixtureContext, '_alive'), patch.object(fixture.ControlledFixtureContext, '_native', return_value='transport'):
                for bad in ('0'*64, 'a'*64, 'd'*64):
                    context._compiled['live']['target_birth_sha256'] = bad
                    with self.assertRaises(fixture.ControlledRejected): context._execute_body()
                    self.assertFalse((batch/'body.dispatched').exists()); self.assertFalse(context._body_used)
                context._compiled['live']['target_birth_sha256'] = 'b'*64
                try: self.assertEqual(context._execute_body(), 'transport')
                except fixture.ControlledRejected as error: self.fail('restart dispatch rejected: '+str(error))
                raw = (batch/'body.dispatched').read_bytes(); record = json.loads(raw)
                self.assertEqual((record['case'], record['exact_test'], record['binary_sha256']), (CASE, TEST, 'c'*64))
                with self.assertRaises(fixture.ControlledRejected): context._execute_body()
                self.assertEqual((batch/'body.dispatched').read_bytes(), raw)

    def test_owned_cid_cannot_be_replaced_or_borrowed_from_another_side(self):
        context = fixture.ControlledFixtureContext.__new__(fixture.ControlledFixtureContext)
        identity = {'project': 'exact-target'}; cid = 'a'*64
        owner = dict(state='known', id=cid, intent={'name': 'exact-target-pg-1'})
        side = dict(identity=identity, creation=owner, ids={'container_id': cid}, owner=SimpleNamespace(held=lambda: None))
        owners = {'exact-target-pg-1': owner}
        context._runner = SimpleNamespace(ownership=SimpleNamespace(owners=owners))
        with patch.object(fixture.ControlledFixtureContext, '_current', return_value=side):
            context._require_current_container(identity, cid)
            for other in ('b'*64, 'exact-target-pg-1'):
                with self.assertRaises(fixture.ControlledRejected): context._require_current_container(identity, other)
            with self.assertRaises(fixture.ControlledRejected): context._require_current_container({'project': 'source'}, cid)
            owners['exact-target-pg-1'] = copy.deepcopy(owner)
            with self.assertRaises(fixture.ControlledRejected): context._require_current_container(identity, cid)
            owners['exact-target-pg-1'] = owner; owner['state'] = 'sent'
            with self.assertRaises(fixture.ControlledRejected): context._require_current_container(identity, cid)

    def test_restart_receipt_requires_complete_transition_and_never_accepts_process_failure(self):
        points = ['FRESH_SOURCE_FIXED_PRODUCER', 'SNAPSHOT_NOFOLLOW_FROZEN', 'REAL_TOC_VERIFIED',
            'FULL_GOLDEN_VERIFIED', 'TARGET_UNUSABLE', 'EXACT_TARGET_STOPPED', 'READY',
            'RESTART_BEFORE_DDL', 'OLD_GUARD_REJECTED', 'ATTEMPT_ABSENT', 'INTENT_ABSENT', 'DDL_NOT_SENT']
        record = dict(schema=1, case=CASE, failure='Identity', checkpoints=points,
            commit_attempted=False, rollback_verified=False, retry_allowed=False,
            dump_sha256='a'*64, toc_sha256='b'*64, raw_sql_sha256='c'*64,
            transformed_sql_sha256='d'*64, content_sha256=None)
        def output(value):
            return ('\nrunning 1 test\ntest '+TEST+' ... KW_C4_IMPORT|'+json.dumps(value, separators=(',', ':'))+
                '\nok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 123 filtered out; finished in 0.01s\n').encode()
        valid = output(record)
        self.assertEqual(runner._import_observations(valid, CASE, 0), record)
        for missing in points:
            with self.subTest(missing=missing), self.assertRaises(runner.ImportRejected):
                runner._import_observations(output(dict(record, checkpoints=[p for p in points if p != missing])), CASE, 0)
        for updates in ({'case': 'success'}, {'failure': 'Session'}, {'retry_allowed': True},
                {'commit_attempted': True}, {'rollback_verified': True}):
            with self.subTest(updates=updates), self.assertRaises(runner.ImportRejected):
                runner._import_observations(output(dict(record, **updates)), CASE, 0)
        for data, code in ((valid, 101), (valid, -9), (valid+b'thread panicked\n', 0),
                (valid+valid, 0), (valid.replace(b'0 ignored', b'1 ignored'), 0)):
            with self.assertRaises(runner.ImportRejected): runner._import_observations(data, CASE, code)

    def test_restart_markers_must_both_be_absent_on_disk(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); control=root/'targets'/ID/'control'; control.mkdir(parents=True)
            backend=runner.ImportBackend.__new__(runner.ImportBackend)
            target=dict(control=root,batch_id=ID,identity={'database':'exact_db'})
            self.assertEqual(backend._markers(target,{'case':CASE}),{'attempt':{'exists':False},'intent':{'exists':False}})
            for suffix in ('.restore.attempt','.restore.commit-attempt'):
                path=control/('exact_db'+suffix);path.write_bytes(b'unexpected')
                with self.assertRaises(runner.ImportRejected):backend._markers(target,{'case':CASE})
                path.unlink()

    def test_cleanup_accepts_restarted_owned_cid_but_never_stops_foreign_identity(self):
        identity = identity_for(ID); original = created(identity)
        original['containers'][0]['State']['StartedAt'] = '2026-10-18T12:00:01Z'
        cid = original['containers'][0]['Id']; calls = []
        live = copy.deepcopy(original)
        def stop(*words):
            calls.append(words); live['containers'][0]['State']['Running'] = False
        docker = SimpleNamespace(snapshot=lambda: copy.deepcopy(live), _docker=stop)
        self.assertTrue(birth.stop_verified_pg(docker, identity, cid)['confirmed'])
        self.assertEqual(calls, [('stop', '--time', '1', cid)])
        for mutation in ('cid', 'project', 'service', 'image', 'volume'):
            live = copy.deepcopy(original); calls.clear(); pg = live['containers'][0]
            if mutation == 'cid': pg['Id'] = 'f'*64
            elif mutation == 'project': pg['Config']['Labels']['com.docker.compose.project'] = 'foreign'
            elif mutation == 'service': pg['Config']['Labels']['com.docker.compose.service'] = 'foreign'
            elif mutation == 'image': pg['Config']['Image'] = 'foreign'
            else: pg['Mounts'][0]['Name'] = 'foreign'
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): birth.stop_verified_pg(docker, identity, cid)
            self.assertEqual(calls, [])


if __name__ == '__main__': unittest.main()
