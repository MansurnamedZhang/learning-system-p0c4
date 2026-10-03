"""Behavioral checks for the new admission runner; never executes Docker."""
import copy
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import stat
import tempfile
import unittest
import zipfile

ENTRY = 'scripts/p0c4_source_admission_gate_acceptance.py'
RUNNER = Path(__file__).with_name('p0c4_source_admission_gate_acceptance.py')
spec = importlib.util.spec_from_file_location('admission_driver', RUNNER) if RUNNER.exists() else None
gate = importlib.util.module_from_spec(spec) if spec else None
if gate:
    spec.loader.exec_module(gate)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def package(extra=None, manifest_patch=None):
    files = {p: b'fixture' for p in ('Cargo.toml', 'Cargo.lock', 'crates/learning-backup/src/source.rs',
             'crates/learning-backup/src/source/admission_tests.rs', ENTRY,
             'scripts/test_p0c4_source_admission_gate_acceptance.py')}
    files.update(extra or {})
    manifest = {'schema': 1, 'base_commit': '0' * 40, 'snapshot_kind': 'working-tree-red',
                'files': [{'path': p, 'bytes': len(raw), 'sha256': sha(raw)} for p, raw in sorted(files.items())]}
    if manifest_patch:
        manifest_patch(manifest)
    raw_manifest = json.dumps(manifest, sort_keys=True, separators=(',', ':')).encode()
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, 'w') as archive:
        for path, raw in {**files, '_knowweave_source_manifest.json': raw_manifest}.items():
            info = zipfile.ZipInfo(path)
            info.external_attr = (stat.S_IFREG | 0o400) << 16
            archive.writestr(info, raw)
    raw = stream.getvalue()
    return raw, sha(raw), sha(raw_manifest), sha(files[ENTRY])


class DriverTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(gate, 'New admission driver is missing')

    def test_canonical_candidate_rejects_escape_private_and_wrong_hash(self):
        # Removing ZIP path or content verification must fail this test.
        data = package()
        manifest, files = gate.verify_package(*data, 'red')
        self.assertEqual(manifest['snapshot_kind'], 'working-tree-red')
        self.assertIn('Cargo.lock', files)
        for bad in ('../outside', '/absolute', 'a\\b', '.env', '.git/config', 'target/a', 'evidence/result.json'):
            with self.subTest(path=bad), self.assertRaises(gate.GateError):
                gate.verify_package(*package({bad: b'x'}), 'red')
        with self.assertRaises(gate.GateError):
            gate.verify_package(data[0], '1' * 64, data[2], data[3], 'red')
        with self.assertRaises(gate.GateError):
            gate.verify_package(*package(manifest_patch=lambda m: m['files'][0].update(bytes=999)), 'red')
        with self.assertRaises(gate.GateError):
            gate.verify_package(*data, 'green')

    def test_zip_duplicate_and_symlink_are_rejected(self):
        # Missing duplicate or Unix file-type admission allows ambiguous extraction.
        data = package()
        for path, mode in (('Cargo.lock', stat.S_IFREG), ('link', stat.S_IFLNK)):
            stream = io.BytesIO(data[0])
            with zipfile.ZipFile(stream, 'a') as archive:
                info = zipfile.ZipInfo(path)
                info.external_attr = (mode | 0o400) << 16
                import warnings
                with warnings.catch_warnings():
                    warnings.simplefilter('ignore', UserWarning)
                    archive.writestr(info, b'fixture')
            raw = stream.getvalue()
            with self.assertRaises(gate.GateError):
                gate.verify_package(raw, sha(raw), data[2], data[3], 'red')

    def test_password_install_uses_only_new_container_and_private_stdin(self):
        # A malformed init script could expose host credentials before provisioning.
        # Execute the fixed shell with local stand-ins instead of inspecting its text.
        if __import__('os').name == 'nt':
            self.skipTest('POSIX shell execution is a Linux-only local check')
        import os
        import subprocess
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            psql = root / 'psql'
            psql.write_text('#!/bin/sh\ncat > "$CAPTURE"\n')
            psql.chmod(0o700)
            capture = root / 'sql'
            env = {'PATH': str(root) + ':/usr/bin:/bin', 'CAPTURE': str(capture),
                   'C4_DATABASE': 'learning_backup_c4_task3_54a1dc59-3d6c-44fa-926e-3d4d5cabcdcf',
                   'C4_OTHER_DATABASE': 'learning_backup_c4_task3_06a0882f-5c7a-4fa5-ab86-252ece7732fa'}
            run = subprocess.run(['/bin/sh'], input=gate.INITDB, env=env, capture_output=True)
            self.assertEqual(run.returncode, 0)
            self.assertEqual(run.stdout, b'')
            sql = capture.read_text()
            self.assertIn('CREATE ROLE learning_admin LOGIN NOSUPERUSER', sql)
            self.assertIn('OWNER learning_admin;', sql)
            self.assertNotIn('PASSWORD', sql)

    def test_frozen_source_detects_changed_content_added_file_and_symlink(self):
        # Ignoring unexpected entries or trusting the manifest without bytes fails.
        manifest, files = gate.verify_package(*package(), 'red')
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'source'
            gate.extract_source(source, files)
            first = gate.source_digest(source, manifest, enforce_mode=False)
            self.assertEqual(len(first), 64)
            (source / 'Cargo.lock').chmod(0o600)
            (source / 'Cargo.lock').write_bytes(b'changed')
            with self.assertRaises(gate.GateError):
                gate.source_digest(source, manifest, enforce_mode=False)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'source'
            gate.extract_source(source, files)
            (source / 'new').write_bytes(b'new')
            with self.assertRaises(gate.GateError):
                gate.source_digest(source, manifest, enforce_mode=False)

    def test_new_path_refusal_preserves_existing_bytes(self):
        # Removing exclusive creation would overwrite a retained batch/input.
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'input'
            gate.write_new(path, b'old', 0o400)
            with self.assertRaises(FileExistsError):
                gate.write_new(path, b'new', 0o400)
            self.assertEqual(path.read_bytes(), b'old')
            path.chmod(0o600)
            batch = Path(directory) / 'batch'
            gate.mkdir_new(batch)
            with self.assertRaises(FileExistsError):
                gate.mkdir_new(batch)

    def test_exact_libtest_results_distinguish_acl_red_and_one_green(self):
        # Accepting exit alone, ignored test, unrelated panic or extra execution fails.
        name = 'source::admission_tests::busy_source_recovery_preserves_acl_and_journal'
        green = f'\nrunning 1 test\ntest {name} ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.10s\n'
        red = f'\nrunning 1 test\ntest {name} ... FAILED\n\nfailures:\n---- {name} stdout ----\nthread panicked: busy source recovery must preserve runtime CONNECT ACL\n\nfailures:\n    {name}\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.10s\n'
        self.assertEqual(gate.parse_test(0, green, name, 'green')['passed'], 1)
        self.assertEqual(gate.parse_test(101, red, name, 'red')['failed'], 1)
        for code, output, mode in ((0, red, 'red'), (101, red.replace('must preserve runtime CONNECT ACL', 'unrelated panic'), 'red'),
                                   (0, green.replace('0 ignored', '1 ignored'), 'green'), (0, green + '\ntest extra ... ok\n', 'green')):
            with self.subTest(mode=mode), self.assertRaises(gate.GateError):
                gate.parse_test(code, output, name, mode)

    def test_builder_predicate_requires_owned_bind_uid_and_rejects_wrong_identity(self):
        # Root without DAC override cannot access hans0700/0400; wrong UID must fail before start.
        expected = {'id': 'a' * 64, 'name': 'new-builder', 'image': 'sha256:' + 'b' * 64,
                    'env': {'PATH': '/usr/bin'}, 'labels': {'batch': 'new'}, 'cmd': ['-ec', 'true'],
                    'entrypoint': ['/bin/sh'], 'mounts': {('bind', '/source', '/reviewed', False), ('bind', '/target', '/target', True)},
                    'network': 'none', 'builder': True, 'user': '1000:1000'}
        facts = {'Id': 'a' * 64, 'Name': '/new-builder', 'Image': expected['image'],
                 'Config': {'Image': expected['image'], 'Env': ['PATH=/usr/bin'], 'Labels': {'batch': 'new'},
                            'User': '1000:1000', 'WorkingDir': '/reviewed', 'Cmd': ['-ec', 'true'], 'Entrypoint': ['/bin/sh']},
                 'State': {'Running': False},
                 'HostConfig': {'Privileged': False, 'CapAdd': None, 'CapDrop': ['ALL'], 'SecurityOpt': ['no-new-privileges'],
                                'NanoCpus': 4000000000, 'Memory': 8589934592, 'MemorySwap': 8589934592, 'PidsLimit': 512,
                                'NetworkMode': 'none', 'ReadonlyRootfs': True, 'Tmpfs': {'/tmp': 'rw,nosuid,nodev,size=1g'},
                                'IpcMode': 'private', 'PidMode': '', 'PortBindings': {}, 'AutoRemove': False},
                 'Mounts': [{'Type': 'bind', 'Source': '/source', 'Destination': '/reviewed', 'RW': False},
                            {'Type': 'bind', 'Source': '/target', 'Destination': '/target', 'RW': True}],
                 'NetworkSettings': {'Networks': {}, 'Ports': {}}}
        try:
            gate.validate_container(facts, expected)
        except gate.GateError as error:
            self.fail('Builder matching owner UID:GID must be accepted: ' + str(error))
        for section, key, value in ((None, 'Id', 'c' * 64), ('Config', 'User', '0:0'),
                                     ('Config', 'User', '1001:1000'), ('Config', 'User', '1000:1001'), ('HostConfig', 'NetworkMode', 'bridge'),
                                     ('HostConfig', 'MemorySwap', -1), ('HostConfig', 'Binds', ['/source:/reviewed:ro']),
                                     ('Config', 'Env', ['PATH=/usr/bin', 'PGPASSWORD=secret'])):
            changed = copy.deepcopy(facts)
            (changed if section is None else changed[section])[key] = value
            with self.subTest(key=key), self.assertRaises(gate.GateError):
                gate.validate_container(changed, expected)
        changed = copy.deepcopy(facts)
        changed['Mounts'][0]['RW'] = True
        with self.assertRaises(gate.GateError):
            gate.validate_container(changed, expected)

    def test_pg_compose_readonly_binds_accept_exact_declarations_and_reject_extra_access(self):
        # Docker Compose's legitimate HostConfig.Binds must work; relaxing RO/identity must fail.
        binds = ['/new-case/initdb.sh:/docker-entrypoint-initdb.d/10-admission.sh:ro',
                 '/new-case/secrets/postgres_password:/run/secrets/postgres_password:ro',
                 '/new-case/secrets/admin_password:/run/secrets/admin_password:ro',
                 '/new-case/secrets/runtime_password:/run/secrets/runtime_password:ro']
        mounts = [{'Type': 'bind', 'Source': '/new-case/initdb.sh', 'Destination': '/docker-entrypoint-initdb.d/10-admission.sh', 'RW': False},
                  {'Type': 'bind', 'Source': '/new-case/secrets/postgres_password', 'Destination': '/run/secrets/postgres_password', 'RW': False},
                  {'Type': 'bind', 'Source': '/new-case/secrets/admin_password', 'Destination': '/run/secrets/admin_password', 'RW': False},
                  {'Type': 'bind', 'Source': '/new-case/secrets/runtime_password', 'Destination': '/run/secrets/runtime_password', 'RW': False},
                  {'Type': 'volume', 'Source': '/new-volume', 'Destination': '/var/lib/postgresql', 'RW': True}]
        expected = {'id': 'd' * 64, 'name': 'new-pg', 'image': 'sha256:' + 'e' * 64,
                    'env': {'POSTGRES_PASSWORD_FILE': '/run/secrets/postgres_password'}, 'labels': {'project': 'new'},
                    'cmd': ['postgres', '-c', 'max_prepared_transactions=16'], 'entrypoint': ['docker-entrypoint.sh'],
                    'network': 'new_test', 'builder': False,
                    'mounts': {('bind', '/new-case/initdb.sh', '/docker-entrypoint-initdb.d/10-admission.sh', False),
                               ('bind', '/new-case/secrets/postgres_password', '/run/secrets/postgres_password', False),
                               ('bind', '/new-case/secrets/admin_password', '/run/secrets/admin_password', False),
                               ('bind', '/new-case/secrets/runtime_password', '/run/secrets/runtime_password', False),
                               ('volume', '/new-volume', '/var/lib/postgresql', True)}}
        facts = {'Id': 'd' * 64, 'Name': '/new-pg', 'Image': 'sha256:' + 'e' * 64,
                 'Config': {'Image': 'sha256:' + 'e' * 64, 'Env': ['POSTGRES_PASSWORD_FILE=/run/secrets/postgres_password'],
                            'Labels': {'project': 'new'}, 'Cmd': ['postgres', '-c', 'max_prepared_transactions=16'], 'Entrypoint': ['docker-entrypoint.sh']},
                 'HostConfig': {'Privileged': False, 'CapAdd': None, 'CapDrop': None, 'SecurityOpt': ['no-new-privileges'],
                                'NanoCpus': 2000000000, 'Memory': 4294967296, 'MemorySwap': 4294967296,
                                'IpcMode': 'private', 'PidMode': '', 'NetworkMode': 'new_test', 'PortBindings': {}, 'Binds': binds},
                 'Mounts': mounts, 'NetworkSettings': {'Ports': {}}}
        try:
            gate.validate_container(facts, expected)
        except gate.GateError as error:
            self.fail('Exact declared readonly Compose binds must be accepted: ' + str(error))
        for bad in (binds + ['/foreign:/extra:ro'], binds + [binds[0]], binds[1:],
                    [binds[0].removesuffix(':ro') + ':rw', *binds[1:]],
                    [binds[0].removesuffix(':ro') + ':ro,z', *binds[1:]],
                    [binds[0].replace('/new-case/', '/old-case/'), *binds[1:]]):
            changed = copy.deepcopy(facts)
            changed['HostConfig']['Binds'] = bad
            with self.subTest(binds=bad), self.assertRaises(gate.GateError):
                gate.validate_container(changed, expected)
        changed = copy.deepcopy(facts)
        changed['Mounts'][0]['RW'] = True
        with self.assertRaises(gate.GateError):
            gate.validate_container(changed, expected)

    def test_cleanup_failure_keeps_the_primary_fixed_reason(self):
        # Overwriting the actual predicate failure with cleanup-only status prevents diagnosis.
        self.assertTrue(hasattr(gate, 'fail_result'), 'Failure cause preservation is missing')
        result = {}
        gate.fail_result(result, 'CONTAINER_HOST_ACCESS')
        gate.fail_result(result, 'CLEANUP_UNKNOWN')
        self.assertEqual(result, {'status': 'FAILED', 'reason': 'CLEANUP_UNKNOWN', 'primary_reason': 'CONTAINER_HOST_ACCESS'})
        gate.fail_result(result, 'RESULT_PERSISTENCE_UNKNOWN')
        self.assertEqual(result['primary_reason'], 'CONTAINER_HOST_ACCESS')

    def test_pg_top_requires_pid_column_and_only_postgres_owner_processes(self):
        # Omitting PID makes Docker top fail; accepting malformed rows weakens owner evidence.
        self.assertTrue(hasattr(gate, 'validate_pg_processes'), 'PID-aware process validation is missing')
        gate.validate_pg_processes('  UID     PID COMMAND\n  999   21001 postgres\n  999   21002 postgres\n')
        for bad in ('UID COMMAND\n999 postgres\n', 'UID PID COMMAND\n',
                    'UID PID COMMAND\n0 21001 postgres\n', 'UID PID COMMAND\n999 0 postgres\n',
                    'UID PID COMMAND\n999 -1 postgres\n', 'UID PID COMMAND\n999 abc postgres\n',
                    'UID PID COMMAND\n999 21001 postgres extra\n', 'UID PID COMMAND\n999 21001 sh\n',
                    'UID PID COMMAND\n999 postgres\n', 'UID PID COMMAND\n999 21001 postgres\n0 21002 postgres\n'):
            with self.subTest(table=bad), self.assertRaises(gate.GateError):
                gate.validate_pg_processes(bad)

    def test_subnets_are_canonical_private_distinct_and_not_occupied(self):
        self.assertEqual(gate.admit_subnets(['172.25.20.0/24'], ['172.25.19.0/24']), ['172.25.20.0/24'])
        for requested, occupied in ((['172.25.20.1/24'], []), (['8.8.8.0/24'], []),
                                    (['172.25.20.0/24'] * 2, []), (['172.25.20.0/24'], ['172.25.0.0/16'])):
            with self.assertRaises(gate.GateError):
                gate.admit_subnets(requested, occupied)

    def test_actual_default_network_null_ipam_preserves_occupied_ranges(self):
        # Treating null as iterable breaks default networks; dropping real ranges allows collisions.
        self.assertTrue(hasattr(gate, 'occupied_subnets'), 'IPAM normalization is missing')
        self.assertEqual(gate.occupied_subnets(None), [])
        self.assertEqual(gate.occupied_subnets([{'Subnet': 'default'}, {'Subnet': '10.253.186.0/24'}]), ['10.253.186.0/24'])
        with self.assertRaises(gate.GateError):
            gate.occupied_subnets({'Subnet': '10.253.186.0/24'})


if __name__ == '__main__':
    unittest.main()
