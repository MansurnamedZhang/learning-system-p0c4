"""Behavioral source-binding driver tests. No Docker, PG or remote operations."""
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import unittest
from unittest.mock import patch
import uuid
import zipfile
from scripts.test_p0c4_storage_registry import receipt_fixture

HERE = Path(__file__).parent
spec = importlib.util.spec_from_file_location('binding_driver', HERE / 'p0c4_source_binding_gate_acceptance.py')
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
helper = gate.load_helper(HERE / 'p0c4_source_admission_gate_acceptance.py')


def package(extra=None, patch_manifest=None):
    files = {name: b'fixture' for name in ('Cargo.toml', 'Cargo.lock', 'crates/learning-backup/src/source.rs',
             'crates/learning-backup/src/source/admission_tests.rs', 'scripts/test_p0c4_source_admission_gate_acceptance.py',
             gate.ENTRY, 'scripts/test_p0c4_source_binding_gate_acceptance.py', 'crates/learning-backup/src/source/binding_tests.rs')}
    files[gate.HELPER_ENTRY] = (HERE / Path(gate.HELPER_ENTRY).name).read_bytes()
    files.update(extra or {})
    manifest = {'schema': 1, 'base_commit': '0' * 40, 'snapshot_kind': 'working-tree-red',
                'files': [{'path': name, 'bytes': len(raw), 'sha256': gate.digest(raw)} for name, raw in sorted(files.items())]}
    if patch_manifest:
        patch_manifest(manifest)
    manifest_raw = gate.canonical(manifest)
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, 'w') as archive:
        for name, raw in {**files, gate.MANIFEST: manifest_raw}.items():
            info = zipfile.ZipInfo(name)
            info.external_attr = (stat.S_IFREG | 0o400) << 16
            archive.writestr(info, raw)
    raw = stream.getvalue()
    return raw, gate.digest(raw), gate.digest(manifest_raw), gate.digest(files[gate.ENTRY])


def issuer_row():
    sql = {'database': 'learning_backup_c4_task3_' + str(uuid.uuid4()), 'database_oid': 16384,
           'system_identifier': '7533123456789012345'}
    binding = {'format_version': 1, 'capability': 'source_control_binding_v1', 'binding_id': str(uuid.uuid4()),
               'control_path': gate.CONTROL, 'control_dev': 2049, 'control_ino': 72181, **sql}
    return {'binding': binding, 'binding_sha256': gate.digest(gate.canonical(binding)), 'sql': sql,
            'issuer_euid': 0, 'classification': 'NO_DUMP_SYNTHETIC', 'registry_provisioning':receipt_fixture(gate.digest(gate.canonical(binding)),binding['control_dev'],binding['control_ino'])}


ZERO_TIME = '0001-01-01T00:00:00Z'
STARTED_TIME = '2026-10-04T10:00:00.123456789Z'
FINISHED_TIME = '2026-10-04T10:00:01.123456789Z'
LIBRARY_OUTPUT = b'''running 428 tests
test full_restore::dump_validation_diagnostic_tests::linux_actual_decode_entry_carries_observer_to_frozen_metadata_rejection ... ok
test full_restore::dump_validation_diagnostic_tests::linux_actual_version_and_toc_run_retain_the_native_rejection_predicate ... ok
test full_restore::spool::native_observation::first_version_files_are_private_exact_and_exclusive ... ok
test full_restore::spool::tests::full_dump_spool_immutable_after_source_rewrite ... ok
test result: ok. 348 passed; 0 failed; 80 ignored; 0 measured; 0 filtered out; finished in 0.1s
'''
LIBRARY_STDERR = b'''FULL_REHEARSAL_FAILURE kind=PRIMARY stage=PACKAGE class=INVALID code=FRESH_INDEX_DIFFERS
FULL_REHEARSAL_FAILURE kind=SECONDARY stage=STOP_INSPECTION class=INVALID code=STOP_INSPECTION_UNCONFIRMED
FULL_REHEARSAL_FAILURE kind=PRIMARY stage=REPORT class=INVALID code=REPORT_INVALID
FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=UNKNOWN
FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=CAPACITY code=UNKNOWN
FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=IO code=UNKNOWN
FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=DATABASE code=UNKNOWN
FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=JSON code=UNKNOWN
FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=ASSET code=UNKNOWN
'''


def helper_facts(network='none', status='created'):
    expected = {'id': 'a' * 64, 'name': 'fresh-helper', 'image': 'sha256:' + 'b' * 64,
                'labels': {'knowweave.source-binding.batch': 'fresh'}, 'cmd': ['-c', 'fixture'],
                'entrypoint': ['python3'], 'env': {'PATH': '/usr/bin'}, 'stdin': False,
                'mounts': {('bind', '/fresh/source', '/reviewed', False),
                           ('volume', '/docker/volumes/build', '/target', True),
                           ('volume', '/docker/volumes/source', gate.SOURCE_ROOT, True)},
                'volume_names': {'/target': 'fresh-build', gate.SOURCE_ROOT: 'fresh-source'},
                'network': network, 'network_id': 'c' * 64}
    facts = {'Id': expected['id'], 'Name': '/' + expected['name'], 'Image': expected['image'],
             'Config': {'Image': expected['image'], 'User': '0:0', 'WorkingDir': '/reviewed',
                        'Labels': expected['labels'], 'Cmd': expected['cmd'], 'Entrypoint': ['python3'],
                        'Env': ['PATH=/usr/bin'], 'OpenStdin': False},
             'HostConfig': {'Privileged': False, 'CapAdd': None, 'CapDrop': ['ALL'],
                            'SecurityOpt': ['no-new-privileges'], 'NanoCpus': 4 * 10**9,
                            'Memory': 8 * 1024**3, 'MemorySwap': 8 * 1024**3, 'PidsLimit': 512,
                            'ReadonlyRootfs': True, 'Tmpfs': {'/tmp': 'rw,nosuid,nodev,size=1g'},
                            'IpcMode': 'private', 'NetworkMode': network, 'CgroupnsMode': 'private',
                            'Mounts': [{'Target': '/target', 'VolumeOptions': {'NoCopy': True}},
                                       {'Target': gate.SOURCE_ROOT, 'VolumeOptions': {'NoCopy': True}}]},
             'Mounts': [{'Type': kind, 'Source': source, 'Destination': destination, 'RW': rw,
                         **({'Name': expected['volume_names'][destination]} if kind == 'volume' else {})}
                        for kind, source, destination, rw in sorted(expected['mounts'])],
             'State': {'Status': status, 'Running': status == 'running',
                       'StartedAt': ZERO_TIME if status == 'created' else STARTED_TIME,
                       'FinishedAt': FINISHED_TIME if status == 'exited' else ZERO_TIME, 'ExitCode': 0},
             'NetworkSettings': {'Ports': {}, 'Networks': {} if network == 'none' else
                                 {network: {'NetworkID': '' if status == 'created' else 'c' * 64}}}}
    return facts, expected


def case_network_facts():
    expected = {'id': 'c' * 64, 'name': 'fresh_test', 'subnet': '172.25.40.0/24',
                'project': 'fresh-project', 'pg_id': 'd' * 64}
    facts = {'Id': expected['id'], 'Name': expected['name'], 'Internal': True, 'Driver': 'bridge',
             'Labels': {'com.docker.compose.project': expected['project']},
             'IPAM': {'Config': [{'Subnet': expected['subnet'], 'Gateway': '172.25.40.1'}]},
             'Containers': {expected['pg_id']: {'Name': 'fresh-pg'}}}
    return facts, expected


class BindingDriverTests(unittest.TestCase):
    def test_hash_checked_helper_is_executed_from_same_verified_bytes(self):
        self.assertEqual(helper.ENTRY, gate.HELPER_ENTRY)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'helper.py'
            marker = Path(directory) / 'executed'
            path.write_text(f"open({str(marker)!r}, 'w').write('untrusted')")
            with self.assertRaisesRegex(gate.GateError, 'IMMUTABLE_HELPER_DIGEST'):
                gate.load_helper(path)
            self.assertFalse(marker.exists())

    def test_package_pins_both_drivers_and_full_source_inventory(self):
        manifest, files = gate.verify_package(helper, *package(), 'red')
        self.assertEqual(manifest['snapshot_kind'], 'working-tree-red')
        self.assertEqual(gate.digest(files[gate.HELPER_ENTRY]), gate.HELPER_SHA)
        for extra in ({gate.HELPER_ENTRY: b'unreviewed'}, {'../escape': b'x'}, {'private/key': b'x'}, {'target/a': b'x'}):
            with self.subTest(extra=list(extra)), self.assertRaises(gate.GateError):
                gate.verify_package(helper, *package(extra), 'red')
        values = package()
        with self.assertRaises(gate.GateError):
            gate.verify_package(helper, *values[:3], 'f' * 64, 'red')
        with self.assertRaises(gate.GateError):
            gate.verify_package(helper, *values, 'green')
        with self.assertRaises(gate.GateError):
            gate.verify_package(helper, *package(patch_manifest=lambda m: m['files'][0].update(bytes=999)), 'red')

    def test_public_extraction_preserves_bytes_and_detects_unexpected_changes(self):
        manifest, files = gate.verify_package(helper, *package(), 'red')
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'source'
            gate.extract_public_source(helper, source, files)
            # Windows chmod does not implement Unix ownership/permission semantics.
            first = helper.source_digest(source, manifest, enforce_mode=False)
            if os.name != 'nt':
                self.assertEqual(gate.source_digest(helper, source, manifest), first)
                (source / 'Cargo.lock').chmod(0o400)
                with self.assertRaisesRegex(gate.GateError, 'PUBLIC_SOURCE_MODE'):
                    gate.source_digest(helper, source, manifest)
            (source / 'Cargo.lock').chmod(0o600)
            (source / 'Cargo.lock').write_bytes(b'changed')
            with self.assertRaisesRegex(gate.GateError, 'SOURCE_CHANGED'):
                helper.source_digest(source, manifest, enforce_mode=False)
            for path in source.rglob('*'):
                if path.is_file():
                    path.chmod(0o600)

    def test_issuer_matches_actual_sql_identity_and_exact_pin(self):
        original = issuer_row()
        self.assertEqual(gate.parse_issuer(helper, gate.canonical(original), original['sql']), original)
        mutations = [('issuer_euid', 1000), ('classification', 'PRODUCTION')]
        for key, value in mutations:
            changed = copy.deepcopy(original)
            changed[key] = value
            with self.subTest(key=key), self.assertRaises(gate.GateError):
                gate.parse_issuer(helper, gate.canonical(changed), original['sql'])
        for key, value in (('control_path', '/wrong/control'), ('control_dev', 0), ('control_ino', True),
                           ('database_oid', 16385), ('system_identifier', '999'), ('format_version', True),
                           ('capability', 'unknown'), ('binding_id', str(uuid.uuid1())), ('extra', 'field')):
            changed = copy.deepcopy(original)
            changed['binding'][key] = value
            changed['binding_sha256'] = gate.digest(gate.canonical(changed['binding']))
            with self.subTest(key=key), self.assertRaises(gate.GateError):
                gate.parse_issuer(helper, gate.canonical(changed), original['sql'])
        with self.assertRaises(gate.GateError):
            gate.parse_issuer(helper, b' ' + gate.canonical(original), original['sql'])
        with self.assertRaises(gate.GateError):
            gate.parse_issuer(helper, b'x' * 16385, original['sql'])

    def test_exact_red_proves_acl_mutation_and_not_infrastructure_failure(self):
        name = gate.TESTS[0]
        red = f'''running 1 test
test {name} ... FAILED
failures:
---- {name} stdout ----
NO_DUMP SYNTHETIC: original_backend_absent=true lock_free=true recovery_ok=true connect_before=true connect_after=false
thread panicked: {gate.ACL_ASSERTION}
failures:
    {name}
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 12 filtered out; finished in 0.20s
'''
        self.assertEqual(gate.parse_test(101, red, b'', name, 'red')['failed'], 1)
        for code, stdout, stderr in ((0, red, b''), (101, red.replace(gate.ACL_ASSERTION, 'DSN missing'), b''),
                                     (101, red.replace('lock_free=true', 'lock_free=false'), b''),
                                     (101, red.replace('connect_after=false', 'connect_after=true'), b''),
                                     (101, red, b'unexpected'), (101, red + 'test extra ... ok\n', b'')):
            with self.subTest(code=code, stderr=stderr), self.assertRaises(gate.GateError):
                gate.parse_test(code, stdout, stderr, name, 'red')

    def test_green_requires_one_actual_test_no_ignored_and_no_stderr(self):
        for name in gate.TESTS:
            output = f'running 1 test\ntest {name} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out; finished in 0.20s\n'
            self.assertEqual(gate.parse_test(0, output, b'', name, 'green')['passed'], 1)
            for code, stdout, stderr in ((101, output, b''), (0, output.replace('0 ignored', '1 ignored'), b''),
                                         (0, output.replace('running 1 test', 'running 0 tests'), b''),
                                         (0, output + 'warning\n', b''), (0, output, b'warning')):
                with self.assertRaises(gate.GateError):
                    gate.parse_test(code, stdout, stderr, name, 'green')

    def test_helper_exact_root_and_offline_or_case_network_are_required(self):
        for network in ('none', 'fresh_test'):
            facts, expected = helper_facts(network)
            gate.validate_helper(helper, facts, expected)
            changes = [(None, 'Id', 'd' * 64), ('Config', 'User', '1000:1000'),
                       ('Config', 'Env', ['PATH=/usr/bin', gate.PIN_ENV + '=' + 'f' * 64]),
                       ('HostConfig', 'Privileged', True), ('HostConfig', 'CapAdd', ['DAC_OVERRIDE']),
                       ('HostConfig', 'ReadonlyRootfs', False), ('HostConfig', 'NanoCpus', 8 * 10**9),
                       ('HostConfig', 'MemorySwap', -1), ('HostConfig', 'Devices', [{'PathOnHost': '/dev/sda'}]),
                       ('HostConfig', 'PidMode', 'host'), ('HostConfig', 'IpcMode', 'host'),
                       ('HostConfig', 'CgroupnsMode', 'host'), ('HostConfig', 'NetworkMode', 'host'),
                       ('HostConfig', 'Binds', ['/var/run/docker.sock:/var/run/docker.sock:ro'])]
            for section, key, value in changes:
                changed = copy.deepcopy(facts)
                (changed if section is None else changed[section])[key] = value
                with self.subTest(network=network, key=key), self.assertRaises(gate.GateError):
                    gate.validate_helper(helper, changed, expected)
            changed = copy.deepcopy(facts)
            changed['Mounts'][0]['RW'] = True
            with self.assertRaises(gate.GateError):
                gate.validate_helper(helper, changed, expected)
            changed = copy.deepcopy(facts)
            changed['HostConfig']['Mounts'][0]['VolumeOptions']['NoCopy'] = False
            with self.assertRaises(gate.GateError):
                gate.validate_helper(helper, changed, expected)
            if network != 'none':
                changed = copy.deepcopy(facts)
                changed['NetworkSettings']['Networks'][network]['NetworkID'] = 'd' * 64
                with self.assertRaises(gate.GateError):
                    gate.validate_helper(helper, changed, expected)

    def test_helper_endpoint_accepts_empty_only_when_verified_never_started(self):
        facts, expected = helper_facts('fresh_test')
        gate.validate_helper(helper, facts, expected)
        facts['NetworkSettings']['Networks']['fresh_test']['NetworkID'] = 'c' * 64
        gate.validate_helper(helper, facts, expected)
        for status in ('running', 'exited'):
            active, active_expected = helper_facts('fresh_test', status)
            gate.validate_helper(helper, active, active_expected)
            active['NetworkSettings']['Networks']['fresh_test']['NetworkID'] = ''
            with self.subTest(status=status), self.assertRaises(gate.GateError):
                gate.validate_helper(helper, active, active_expected)
        for change in ({}, {'Status': 'created', 'Running': False},
                       {'Status': 'created', 'Running': True, 'StartedAt': ZERO_TIME, 'FinishedAt': ZERO_TIME},
                       {'Status': 'created', 'Running': False, 'StartedAt': STARTED_TIME, 'FinishedAt': ZERO_TIME},
                       {'Status': 'running', 'Running': False, 'StartedAt': STARTED_TIME, 'FinishedAt': ZERO_TIME},
                       {'Status': 'exited', 'Running': False, 'StartedAt': ZERO_TIME, 'FinishedAt': FINISHED_TIME},
                       {'Status': 'exited', 'Running': False, 'StartedAt': STARTED_TIME, 'FinishedAt': ZERO_TIME},
                       {'Status': 'dead', 'Running': False, 'StartedAt': STARTED_TIME, 'FinishedAt': FINISHED_TIME}):
            broken, broken_expected = helper_facts('fresh_test')
            broken['State'] = change
            with self.subTest(state=change), self.assertRaises(gate.GateError):
                gate.validate_helper(helper, broken, broken_expected)
        for network_id in (None, 'e' * 64):
            broken, broken_expected = helper_facts('fresh_test')
            broken['NetworkSettings']['Networks']['fresh_test']['NetworkID'] = network_id
            with self.assertRaises(gate.GateError):
                gate.validate_helper(helper, broken, broken_expected)
        broken, broken_expected = helper_facts('fresh_test')
        broken['NetworkSettings']['Networks']['fresh_test'].pop('NetworkID')
        with self.assertRaises(gate.GateError):
            gate.validate_helper(helper, broken, broken_expected)
        for attachments in ({}, {'fresh_test': {'NetworkID': ''}, 'foreign_test': {'NetworkID': 'f' * 64}}):
            broken, broken_expected = helper_facts('fresh_test')
            broken['NetworkSettings']['Networks'] = attachments
            with self.subTest(attachments=attachments), self.assertRaises(gate.GateError):
                gate.validate_helper(helper, broken, broken_expected)

    def test_network_object_identity_subnet_project_and_membership_are_independent(self):
        facts, expected = case_network_facts()
        gate.validate_case_network(facts, expected)
        for field, value in (('Id', 'e' * 64), ('Name', 'old_test'), ('Internal', False),
                             ('Driver', 'host'), ('Labels', {'com.docker.compose.project': 'old-project'}),
                             ('IPAM', {'Config': [{'Subnet': '172.25.41.0/24', 'Gateway': '172.25.41.1'}]}),
                             ('Containers', {}), ('Containers', {**facts['Containers'], 'f' * 64: {}})):
            broken = copy.deepcopy(facts)
            broken[field] = value
            with self.subTest(field=field), self.assertRaises(gate.GateError):
                gate.validate_case_network(broken, expected)
        running = copy.deepcopy(facts)
        running['Containers']['a' * 64] = {'Name': 'fresh-helper'}
        gate.validate_case_network(running, expected, helper_id='a' * 64, running=True)
        with self.assertRaises(gate.GateError):
            gate.validate_case_network(facts, expected, helper_id='a' * 64, running=True)
        with self.assertRaises(gate.GateError):
            gate.validate_case_network(running, expected, helper_id='a' * 64, running=False)

    def test_created_empty_endpoint_starts_then_populates_and_cleans_verified_id(self):
        class LifecycleRunner:
            def __init__(self):
                self.facts, _ = helper_facts('fresh_test')
                self.facts['Mounts'] = []
                self.facts['HostConfig']['Mounts'] = []
                self.network, self.network_identity = case_network_facts()
                self.actions, self.started_facts = [], None

            def docker(self, *args, **kwargs):
                self.actions.append(args)
                if args[0] == 'create':
                    self.facts['Name'] = '/' + args[args.index('--name') + 1]
                    self.facts['Config']['Labels'] = dict(args[i + 1].split('=', 1)
                        for i, arg in enumerate(args) if arg == '--label')
                    self.facts['Config']['Cmd'] = list(args[args.index(self.facts['Image']) + 1:])
                    return self.facts['Id'].encode()
                if args[0] == 'ps':
                    return b''
                if args[0] in ('stop', 'rm'):
                    self_outer.assertEqual(args[-1], self.facts['Id'])
                    return self.facts['Id'].encode()
                raise AssertionError(args)

            def inspect(self, kind, ident):
                self.actions.append(('inspect', kind, ident))
                if kind == 'network':
                    self_outer.assertEqual(ident, self.network_identity['id'])
                    return copy.deepcopy(self.network)
                self_outer.assertEqual((kind, ident), ('container', self.facts['Id']))
                return copy.deepcopy(self.facts)

            def run(self, command, **kwargs):
                self.actions.append(tuple(command))
                self_outer.assertEqual(command, [helper.DOCKER, 'start', '-a', self.facts['Id']])
                self.facts['State'].update(Status='running', Running=True, StartedAt=STARTED_TIME)
                self.facts['NetworkSettings']['Networks']['fresh_test']['NetworkID'] = self.network_identity['id']
                self.network['Containers'][self.facts['Id']] = {'Name': self.facts['Name'][1:]}
                self.started_facts = copy.deepcopy(self.facts)
                self.facts['State'].update(Status='exited', Running=False, FinishedAt=FINISHED_TIME, ExitCode=101)
                self.network['Containers'].pop(self.facts['Id'])
                return 0, b'actual fixture output', b''

        self_outer = self
        runner = LifecycleRunner()
        result = {'batch_id': 'fresh', 'preflight_complete': True, 'cases': []}
        image = {'Id': runner.facts['Image'], 'Config': {'Env': ['PATH=/usr/bin'], 'Labels': {}}}
        outcome = gate.run_helper(helper, runner, result, image, [], {}, ['python3', '-c', 'fixture'],
                                  network='fresh_test', network_id=runner.network_identity['id'],
                                  network_identity=runner.network_identity, allowed=(0, 101))
        self.assertEqual(outcome, (101, b'actual fixture output', b''))
        _, expected = helper_facts('fresh_test', 'running')
        expected.update(name=runner.facts['Name'][1:], labels=runner.facts['Config']['Labels'], mounts=set())
        gate.validate_helper(helper, runner.started_facts, expected)
        self.assertTrue(result['helpers'][0]['removed'])
        self.assertTrue(gate.cleanup_verified(result))
        self.assertLess(next(i for i, a in enumerate(runner.actions) if a[:2] == ('inspect', 'network')),
                        next(i for i, a in enumerate(runner.actions) if a[0] == 'create'))
        self.assertIn(('stop', '--time', '10', 'a' * 64), runner.actions)
        self.assertIn(('rm', 'a' * 64), runner.actions)

    def test_cleanup_does_not_accept_empty_or_unknown_or_running_resources(self):
        self.assertFalse(gate.cleanup_verified({}))
        good = {'preflight_complete': True, 'helpers': [{'removed': True}], 'cases': [{'stopped': True}]}
        self.assertTrue(gate.cleanup_verified(good))
        for change in ({'resource_creation_unknown': True}, {'helpers': [{'removed': False}]},
                       {'cases': [{'stopped': False}]}, {'preflight_complete': False}):
            with self.subTest(change=change):
                self.assertFalse(gate.cleanup_verified({**good, **change}))

    def test_compiler_artifact_requires_unique_learning_backup_test_binary(self):
        artifact = {'reason': 'compiler-artifact', 'manifest_path': '/reviewed/crates/learning-backup/Cargo.toml',
                    'target': {'name': 'learning_backup', 'kind': ['lib']}, 'profile': {'test': True},
                    'executable': '/target/build/debug/deps/learning_backup-ab123'}
        raw = json.dumps(artifact) + '\n' + json.dumps({'reason': 'build-finished', 'success': True})
        self.assertEqual(gate.discover_binary(raw), artifact['executable'])
        for output in (raw + '\n' + raw, json.dumps({**artifact, 'executable': '/reviewed/escape'}),
                       json.dumps({**artifact, 'profile': {'test': False}}), '{}'):
            with self.assertRaises(gate.GateError):
                gate.discover_binary(output)

    def test_pure_regression_counts_are_separate_from_actual_pg_gate(self):
        output = 'running 72 tests\ntest result: ok. 68 passed; 0 failed; 4 ignored; 0 measured; 0 filtered out; finished in 0.12s\n'
        self.assertEqual(gate.parse_regression(output), [{'passed': 68, 'failed': 0, 'ignored': 4, 'filtered': 0}])
        with self.assertRaises(gate.GateError):
            gate.parse_regression('68 tests listed, no execution')
        with self.assertRaises(gate.GateError):
            gate.parse_regression(output + 'test result: FAILED. 0 passed; 1 failed;\n')

    def test_compilation_retains_the_compiler_stream_hash_and_readonly_control(self):
        ident = gate.identity()
        artifact = {'reason': 'compiler-artifact', 'manifest_path': '/reviewed/crates/learning-backup/Cargo.toml',
                    'target': {'name': 'learning_backup', 'kind': ['lib']}, 'profile': {'test': True},
                    'executable': '/target/build/debug/deps/learning_backup-ab123'}
        compiler = (json.dumps(artifact) + '\n' + json.dumps({'reason': 'build-finished', 'success': True}) + '\n').encode()
        copied = gate.canonical({'binary_sha256': 'a' * 64}) + b'\n'
        listing = (gate.TESTS[0] + ': test\n1 test, 0 benchmarks\n').encode()
        record = {'identity': ident, 'issuer': {'binding_sha256': 'b' * 64}}
        mounts = gate.case_mounts(Path('/fresh/source'), 'build-volume', ident)
        with patch.object(gate, 'run_helper', side_effect=[(0, compiler, b''), (0, copied, b''), (0, listing, b'')]) as calls:
            gate.compile_case(helper, None, {}, {'Config': {'Env': []}}, mounts, {},
                              {'base_commit': '0' * 40}, 'c' * 64, record, False, 'red')
        self.assertEqual(record['compiler_stdout_sha256'], gate.digest(compiler))
        self.assertEqual(record['discovery_stdout_sha256'], gate.digest(listing))
        self.assertEqual(record['binary_sha256'], 'a' * 64)
        for call in calls.call_args_list:
            control = next(m for m in call.args[4] if m[2] == gate.SOURCE_ROOT)
            self.assertTrue(control[3], 'compiler and binary-copy helpers cannot mutate control')

    def test_first_green_runs_full_root_library_in_pg18_after_retaining_binary(self):
        ident = gate.identity()
        artifact = {'reason': 'compiler-artifact', 'manifest_path': '/reviewed/crates/learning-backup/Cargo.toml',
                    'target': {'name': 'learning_backup', 'kind': ['lib']}, 'profile': {'test': True},
                    'executable': '/target/build/debug/deps/learning_backup-ab123'}
        compiler = (json.dumps(artifact) + '\n' + json.dumps({'reason': 'build-finished', 'success': True})).encode()
        builder = {'Id': helper.BUILDER, 'Config': {'Env': []}}
        postgres = {'Id': 'sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d',
                    'RepoDigests': ['postgres@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d'],
                    'Config': {'Env': ['PG_MAJOR=18', 'PG_VERSION=18.6-1.pgdg12+2'],
                               'Volumes': {'/var/lib/postgresql': {}}}}
        record = {'identity': ident, 'issuer': {'binding_sha256': 'b' * 64}}
        result, calls = {}, []

        class Runner:
            def inspect(self, kind, name):
                if (kind, name) == ('image', helper.PG_IMAGE):
                    return postgres
                if (kind, name) == ('volume', 'build-volume'):
                    return {'Mountpoint': '/docker/volumes/build-volume/_data'}
                raise AssertionError((kind, name))

        def execute(_helper, _runner, _result, image, mounts, volumes, command, **kwargs):
            calls.append((image, mounts, volumes, command, kwargs))
            if image is builder:
                self.assertNotEqual(command, ['/bin/sh', '-ec', 'cargo test --locked --offline -p learning-backup --lib'],
                                    'builder has no native PG18 client; compile the library without running it')
                if '--message-format=json' in command[-1]:
                    return 0, compiler, b''
                if command[0] == 'python3':
                    return 0, gate.canonical({'binary_sha256': 'a' * 64}), b''
                if '--list' in command:
                    return 0, ''.join(name + ': test\n' for name in gate.TESTS).encode(), b''
                return 0, b'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s\n', b''
            self.assertIs(image, postgres)
            self.assertIn('binary_sha256', record, 'only execute the retained compiled ELF')
            self.assertEqual(mounts, [('bind', '/docker/volumes/build-volume/_data/retained/' + ident['case_id'],
                                       '/learning-backup-test.elf', True)])
            self.assertEqual(volumes, {})
            self.assertEqual(command[0:2], ['/bin/sh', '-ec'])
            self.assertIn('exec /usr/bin/env -i PATH=/usr/bin:/bin LC_ALL=C HOME=/tmp TMPDIR=/tmp /learning-backup-test.elf --test-threads=1', command[2])
            self.assertNotIn('--ignored', command[2])
            self.assertNotIn('--exact', command[2])
            self.assertEqual(kwargs, {'library_runtime': True, 'timeout': 7200})
            return 0, LIBRARY_OUTPUT, LIBRARY_STDERR

        with patch.object(gate, 'run_helper', side_effect=execute):
            gate.compile_case(helper, Runner(), result, builder,
                              gate.case_mounts(Path('/fresh/source'), 'build-volume', ident), {'/target': 'build-volume'},
                              {'base_commit': '0' * 40}, 'c' * 64, record, True, 'green')
        self.assertEqual(sum(image is postgres for image, *_ in calls), 1)
        self.assertEqual(result['static_checks'][-1]['counts'],
                         [{'passed': 348, 'failed': 0, 'ignored': 80, 'filtered': 0}])

    def test_root_library_requires_all_four_actual_bodies_and_unfiltered_success(self):
        counts = gate.parse_root_library(0, LIBRARY_OUTPUT, LIBRARY_STDERR)
        self.assertEqual(counts, [{'passed': 348, 'failed': 0, 'ignored': 80, 'filtered': 0}])
        for code, output, stderr in ((101, LIBRARY_OUTPUT, LIBRARY_STDERR), (0, LIBRARY_OUTPUT, b'warning'),
                                     (0, LIBRARY_OUTPUT.replace(b'0 filtered out', b'1 filtered out'), LIBRARY_STDERR),
                                     (0, LIBRARY_OUTPUT.replace(b' ... ok', b' ... ignored', 1), LIBRARY_STDERR),
                                     (0, LIBRARY_OUTPUT.replace(b' ... ok', b' ... FAILED', 1), LIBRARY_STDERR),
                                     (0, LIBRARY_OUTPUT + LIBRARY_OUTPUT, LIBRARY_STDERR),
                                     (0, LIBRARY_OUTPUT.splitlines()[-1] + b'\n', LIBRARY_STDERR)):
            with self.subTest(code=code, output=output, stderr=stderr), self.assertRaises(gate.GateError):
                gate.parse_root_library(code, output, stderr)

    def test_root_library_admits_only_exact_serial_negative_test_diagnostics(self):
        try:
            counts = gate.parse_root_library(0, LIBRARY_OUTPUT, LIBRARY_STDERR)
        except gate.GateError as error:
            self.fail('successful full lib emits nine intentional serial diagnostic-test lines: ' + str(error))
        self.assertEqual(counts[0]['passed'], 348)
        for stderr in (b'', LIBRARY_STDERR + b'warning\n', LIBRARY_STDERR + LIBRARY_STDERR,
                       LIBRARY_STDERR.replace(b'code=REPORT_INVALID', b'code=UNKNOWN'),
                       b'\n'.join(LIBRARY_STDERR.splitlines()[1:]) + b'\n',
                       b'\n'.join(reversed(LIBRARY_STDERR.splitlines())) + b'\n'):
            with self.subTest(stderr=stderr), self.assertRaises(gate.GateError):
                gate.parse_root_library(0, LIBRARY_OUTPUT, stderr)

    def test_pg18_library_helper_has_only_readonly_elf_and_private_tmpfs(self):
        image = {'Id': 'sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d',
                 'RepoDigests': ['postgres@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d'],
                 'Config': {'Env': ['PG_MAJOR=18', 'PG_VERSION=18.6-1.pgdg12+2'], 'Labels': None,
                            'Volumes': {'/var/lib/postgresql': {}}}}
        mounts = [('bind', '/docker/volumes/build-volume/_data/retained/' + str(uuid.uuid4()),
                   '/learning-backup-test.elf', True)]
        command = ['/bin/sh', '-ec', gate.ROOT_LIBRARY_RUN, 'binding-library', 'a' * 64]

        class Runner:
            def __init__(self, change=None):
                self.facts, _ = helper_facts()
                self.facts['Image'] = self.facts['Config']['Image'] = image['Id']
                self.facts['Config'].update(WorkingDir='/tmp', Env=image['Config']['Env'],
                                           Entrypoint=['/bin/sh'], Cmd=command[1:],
                                           Volumes={'/var/lib/postgresql': {}})
                self.facts['HostConfig']['Tmpfs'] = {'/tmp': 'rw,nosuid,nodev,size=1g',
                                                    '/var/lib/postgresql': 'rw,noexec,nosuid,nodev,size=4m,mode=0700'}
                self.facts['HostConfig']['Mounts'] = [{'Type': 'bind', 'Source': mounts[0][1],
                                                       'Target': '/learning-backup-test.elf', 'ReadOnly': True}]
                self.facts['Mounts'] = [{'Type': 'bind', 'Source': mounts[0][1],
                                         'Destination': '/learning-backup-test.elf', 'RW': False}]
                self.change, self.actions = change, []

            def docker(self, *args):
                self.actions.append(args)
                if args[0] == 'create':
                    self.facts['Name'] = '/' + args[args.index('--name') + 1]
                    self.facts['Config']['Labels'] = {'knowweave.source-binding.batch': 'fresh'}
                    if self.change:
                        self.change(self.facts)
                    return self.facts['Id'].encode()
                if args[0] == 'ps':
                    return b''
                if args[0] in ('stop', 'rm'):
                    return self.facts['Id'].encode()
                raise AssertionError(args)

            def inspect(self, kind, name):
                return copy.deepcopy(self.facts)

            def run(self, args, **kwargs):
                self.actions.append(tuple(args))
                self.facts['State'].update(Status='exited', StartedAt=STARTED_TIME, FinishedAt=FINISHED_TIME)
                return 0, LIBRARY_OUTPUT, LIBRARY_STDERR

        runner = Runner()
        result = {'batch_id': 'fresh'}
        try:
            outcome = gate.run_helper(helper, runner, result, image, mounts, {}, command, library_runtime=True)
        except gate.GateError as error:
            self.fail('genuine cached official RepoDigest must admit the fixed PG18 runtime: ' + str(error))
        self.assertEqual(outcome, (0, LIBRARY_OUTPUT, LIBRARY_STDERR))
        create = runner.actions[0]
        for flag in ('--network=none', '--read-only', '--cap-drop=ALL', '--security-opt=no-new-privileges',
                     '--user=0:0', '--workdir=/tmp', '--entrypoint=/bin/sh', '--cpus=4', '--memory=8g',
                     '--memory-swap=8g', '--pids-limit=512', '--tmpfs=/tmp:rw,nosuid,nodev,size=1g',
                     '--tmpfs=/var/lib/postgresql:rw,noexec,nosuid,nodev,size=4m,mode=0700'):
            self.assertIn(flag, create)
        self.assertEqual(create.count('--mount'), 1)
        self.assertEqual(create[create.index('--mount') + 1],
                         'type=bind,src=' + mounts[0][1] + ',dst=/learning-backup-test.elf,readonly')
        self.assertNotIn('--env', create)
        self.assertEqual(create[create.index(image['Id']) + 1:], tuple(command[1:]))
        self.assertTrue(result['helpers'][0]['removed'])
        changes = [lambda f: f['Config'].update(Volumes={}),
                   lambda f: f['HostConfig']['Tmpfs'].pop('/var/lib/postgresql'),
                   lambda f: f['HostConfig'].update(NetworkMode='host'),
                   lambda f: f['Config'].update(Entrypoint=['docker-entrypoint.sh']),
                   lambda f: f['Config'].update(Env=image['Config']['Env'] + ['PGPASSWORD=unsafe']),
                   lambda f: f['Mounts'][0].update(RW=True),
                   lambda f: f['Mounts'].append({'Type': 'volume', 'Source': '/docker/anonymous',
                                               'Destination': '/var/lib/postgresql', 'RW': True})]
        for change in changes:
            rejected = Runner(change)
            with self.subTest(change=change), self.assertRaises(gate.GateError):
                gate.run_helper(helper, rejected, {'batch_id': 'fresh'}, image, mounts, {}, command, library_runtime=True)
            self.assertFalse(any(a[0] == helper.DOCKER for a in rejected.actions), 'reject before start')
        invalid = [(dict(image, RepoDigests=[]), mounts, command, {}),
                   (dict(image, RepoDigests=['untrusted/postgres@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d']), mounts, command, {}),
                   (dict(image, RepoDigests=['postgres@sha256:' + 'a' * 64]), mounts, command, {}),
                   (dict(image, Id=helper.BUILDER), mounts, command, {}),
                   (dict(image, Config={**image['Config'], 'Volumes': {'/unexpected': {}}}), mounts, command, {}),
                   (dict(image, Config={**image['Config'], 'Env': ['PG_MAJOR=18', 'PG_VERSION=18.7']}), mounts, command, {}),
                   (image, mounts + [('bind', '/source', '/reviewed', True)], command, {}),
                   (image, mounts, command + ['--ignored'], {}),
                   (image, mounts, command, {'env': {'PGPASSWORD': 'unsafe'}}),
                   (image, mounts, command, {'network': 'host'})]
        for bad_image, bad_mounts, bad_command, kwargs in invalid:
            rejected = Runner()
            with self.subTest(kwargs=kwargs, image=bad_image), self.assertRaises(gate.GateError):
                gate.run_helper(helper, rejected, {'batch_id': 'fresh'}, bad_image, bad_mounts, {}, bad_command,
                                library_runtime=True, **kwargs)
            self.assertEqual(rejected.actions, [], 'invalid runtime profile must reject before create')

    def test_audit_and_runtime_mounts_keep_the_retained_binary_readonly(self):
        ident = gate.identity()
        mounts = gate.case_mounts(Path('/fresh/source'), 'build-volume', ident,
                                  write_build=False, write_control=True)
        self.assertTrue(next(m for m in mounts if m[2] == '/target')[3])
        self.assertFalse(next(m for m in mounts if m[2] == gate.SOURCE_ROOT)[3])
        self.assertTrue(next(m for m in mounts if m[2] == '/reviewed')[3])

    def test_exclusive_batch_creation_never_mutates_an_existing_batch(self):
        with tempfile.TemporaryDirectory() as directory:
            stage = Path(directory)
            batch_id = str(uuid.uuid4())
            old = stage / ('batch-' + batch_id)
            old.mkdir()
            marker = old / 'old-evidence'
            marker.write_bytes(b'keep')
            with self.assertRaises(FileExistsError):
                gate.create_batch(helper, stage, batch_id)
            self.assertEqual(list(old.iterdir()), [marker])
            fresh = gate.create_batch(helper, stage, str(uuid.uuid4()))
            self.assertEqual((fresh / 'evidence/result.pending').read_bytes(), b'PENDING')

    def test_finalization_preserves_pending_until_durable_result_and_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            batch = gate.create_batch(helper, Path(directory), str(uuid.uuid4()))
            with patch.object(gate, 'sync_directory', side_effect=OSError('unavailable')):
                with self.assertRaises(OSError):
                    gate.finalize_result(helper, batch, {'cleanup_verified': True})
            self.assertTrue((batch / 'evidence/result.pending').exists())
        with tempfile.TemporaryDirectory() as directory:
            batch = gate.create_batch(helper, Path(directory), str(uuid.uuid4()))
            with patch.object(gate, 'sync_directory'):
                gate.finalize_result(helper, batch, {'cleanup_verified': False})
            self.assertTrue((batch / 'evidence/result.pending').exists())
            self.assertEqual(json.loads((batch / 'evidence/result.json').read_bytes()), {'cleanup_verified': False})
        with tempfile.TemporaryDirectory() as directory:
            batch = gate.create_batch(helper, Path(directory), str(uuid.uuid4()))
            with patch.object(gate, 'sync_directory'):
                gate.finalize_result(helper, batch, {'cleanup_verified': True})
            self.assertFalse((batch / 'evidence/result.pending').exists())

    def test_build_output_requires_a_successful_build_finished_marker(self):
        artifact = {'reason': 'compiler-artifact', 'manifest_path': '/reviewed/crates/learning-backup/Cargo.toml',
                    'target': {'name': 'learning_backup', 'kind': ['lib']}, 'profile': {'test': True},
                    'executable': '/target/build/debug/deps/learning_backup-ab123'}
        for end in ('', '\n' + json.dumps({'reason': 'build-finished', 'success': False})):
            with self.assertRaises(gate.GateError):
                gate.discover_binary(json.dumps(artifact) + end)

    def test_timed_out_helper_is_stopped_and_removed_only_by_verified_id(self):
        class FakeRunner:
            def __init__(self):
                self.actions, self.removed = [], False

            def docker(self, *args, **kwargs):
                self.actions.append(args)
                if args[0] == 'create':
                    return ('a' * 64).encode()
                if args[0] == 'rm':
                    self.removed = True
                return b'' if args[0] == 'ps' else ('a' * 64).encode()

            def inspect(self, kind, ident):
                self.actions.append(('inspect', kind, ident))
                return {'Id': ident, 'State': {'Status': 'created', 'Running': False,
                                             'StartedAt': ZERO_TIME, 'FinishedAt': ZERO_TIME, 'ExitCode': 0}}

            def run(self, command, **kwargs):
                self.actions.append(tuple(command))
                raise gate.GateError('PROCESS_TIMEOUT')

        runner = FakeRunner()
        result = {'batch_id': str(uuid.uuid4()), 'preflight_complete': True, 'cases': []}
        image = {'Id': 'sha256:' + 'b' * 64, 'Config': {'Env': [], 'Labels': {}}}
        verified = []
        def validate(_helper, facts, expected):
            self.assertEqual(facts['Id'], expected['id'])
            verified.append(expected['id'])
        with patch.object(gate, 'validate_helper', side_effect=validate):
            with self.assertRaisesRegex(gate.GateError, 'PROCESS_TIMEOUT'):
                gate.run_helper(helper, runner, result, image, [], {}, ['python3', '-c', 'pass'])
        self.assertEqual(verified, ['a' * 64] * 3)
        self.assertIn(('stop', '--time', '10', 'a' * 64), runner.actions)
        self.assertIn(('rm', 'a' * 64), runner.actions)
        self.assertTrue(result['helpers'][0]['removed'])
        self.assertTrue(gate.cleanup_verified(result))
        create = runner.actions[0]
        for flag in ('--read-only', '--cap-drop=ALL', '--security-opt=no-new-privileges',
                     '--cpus=4', '--memory=8g', '--memory-swap=8g', '--network=none', '--user=0:0'):
            self.assertIn(flag, create)

    def test_ambiguous_helper_creation_blocks_cleanup_and_never_targets_a_name(self):
        class FailedCreate:
            def docker(self, *args):
                if args[0] == 'create':
                    raise gate.GateError('PROCESS_TIMEOUT')
                raise AssertionError('unknown resources must never be stopped by name')
        result = {'batch_id': str(uuid.uuid4()), 'preflight_complete': True, 'cases': []}
        image = {'Id': 'sha256:' + 'b' * 64, 'Config': {'Env': [], 'Labels': {}}}
        with self.assertRaisesRegex(gate.GateError, 'PROCESS_TIMEOUT'):
            gate.run_helper(helper, FailedCreate(), result, image, [], {}, ['python3', '-c', 'pass'])
        self.assertFalse(gate.cleanup_verified(result))
        self.assertTrue(result['resource_creation_unknown'])

    @unittest.skipIf(os.name == 'nt', 'Private owner/mode/link predicates require POSIX')
    def test_private_staged_helper_rejects_writable_modes_and_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'helper.py'
            path.write_bytes((HERE / Path(gate.HELPER_ENTRY).name).read_bytes())
            path.chmod(0o400)
            self.assertEqual(gate.load_helper(path, private=True).ENTRY, gate.HELPER_ENTRY)
            path.chmod(0o600)
            with self.assertRaisesRegex(gate.GateError, 'HELPER_PRIVATE_MODE'):
                gate.load_helper(path, private=True)
            path.chmod(0o400)
            link = path.with_name('link.py')
            link.symlink_to(path)
            with self.assertRaises(OSError):
                gate.load_helper(link, private=True)

    @unittest.skipIf(os.name == 'nt', 'Actual directory fsync requires POSIX')
    def test_actual_private_result_finalization_is_durable_and_canonical(self):
        with tempfile.TemporaryDirectory() as directory:
            batch = gate.create_batch(helper, Path(directory), str(uuid.uuid4()))
            result = {'status': 'FAILED', 'cleanup_verified': True, 'classification': 'NO_DUMP_SYNTHETIC'}
            gate.finalize_result(helper, batch, result)
            path = batch / 'evidence/result.json'
            self.assertEqual(path.read_bytes(), gate.canonical(result))
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            self.assertEqual(path.stat().st_nlink, 1)
            self.assertFalse((batch / 'evidence/result.pending').exists())

    @unittest.skipIf(os.name == 'nt', 'Pinned bounded POSIX Runner is verified on Linux')
    def test_bounded_process_logs_timeout_and_secret_stdin_are_private(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            root.chmod(0o700)
            runner = helper.Runner(root)
            secret = b'SENSITIVE_PRIVATE_STDIN_73'
            code, out, err = runner.run([sys.executable, '-c', 'import sys; data=sys.stdin.buffer.read();print(len(data))'],
                                         stdin=secret, env={'PATH': '/usr/bin:/bin'}, timeout=5)
            self.assertEqual((code, out.strip(), err), (0, str(len(secret)).encode(), b''))
            self.assertFalse(any(secret in p.read_bytes() for p in root.iterdir()))
            self.assertTrue(all(stat.S_IMODE(p.stat().st_mode) == 0o600 for p in root.iterdir()))
            with self.assertRaisesRegex(gate.GateError, 'PROCESS_TIMEOUT'):
                runner.run([sys.executable, '-c', 'import time;time.sleep(10)'], timeout=0.2)
            with patch.object(helper, 'LIMIT', 128), self.assertRaisesRegex(gate.GateError, 'PROCESS_LOG_BUDGET'):
                runner.run([sys.executable, '-c', 'print("x"*4096)'], timeout=5)

    def test_case_identities_are_distinct_and_private_subnets_fail_closed(self):
        first, second = gate.identity(), gate.identity()
        self.assertNotEqual(first['source_volume'], second['source_volume'])
        self.assertNotEqual(first['database'], first['other_database'])
        for requested, occupied in ((['172.25.40.0/24'] * 2, []), (['172.25.40.1/24'], []),
                                    (['8.8.8.0/24'], []), (['172.25.40.0/24'], ['172.25.0.0/16'])):
            with self.assertRaises(gate.GateError):
                helper.admit_subnets(requested, occupied)


if __name__ == '__main__':
    unittest.main()
