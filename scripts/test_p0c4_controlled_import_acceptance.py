from types import SimpleNamespace
import hashlib
import contextlib
import copy
import importlib.util
import io
import json
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
import zipfile
import p0c4_controlled_import_acceptance as runner
import p0c4_import_fixture as fixture


class Backend:
    def __init__(self, fail=False, stop_fail=False):
        self.fail, self.stop_fail = fail, stop_fail
        self.stopped = False
        self.published = None

    def execute(self):
        if self.fail:
            raise RuntimeError("password=private SELECT sensitive; backend secret")
        return {"contract": True, "artifacts": {"dump_sha256": "a" * 64, "sql_sha256": "b" * 64}}

    def stop(self):
        if self.stop_fail:
            raise RuntimeError("private stop failure")
        self.stopped = True
        return {"source_stopped": True, "target_stopped": True, "volumes_retained": True}

    def verify_source(self):
        return True

    def evidence_identity(self):
        return {'package': {}, 'batches': {}, 'resources': {}}

    def publish(self, result):
        if not self.stopped and result["status"].startswith("CLIENT_CONTRACT"):
            raise AssertionError("success published before stop")
        self.published = result.copy()
        return result


class LocalFiles:
    """Real file I/O; Linux root/permission checks belong to the live gate."""
    def _private_write(self, path, payload, mode=0o600):
        with path.open('xb') as stream:
            stream.write(payload)

    def _private_read(self, path, limit):
        return path.read_bytes()[:limit + 1]

    def _json_bytes(self, value):
        return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()

    def _sync_dir(self, path):
        pass  # Windows cannot verify Linux directory fsync.


class ContractTests(unittest.TestCase):
    def _inspection_peer(self, container_ids, payload_size=65000, delay=0,
                         inspect_reply=None):
        """A real child emits Docker-shaped output for the private provisioner."""
        program = """import json, sys, time
ids = json.loads(sys.argv[1])
size = int(sys.argv[2])
delay = float(sys.argv[3])
override = sys.argv[4]
binary, *args = sys.argv[5:]
if binary == '/usr/sbin/ip':
    output = '[]'
elif args[:2] == ['info', '--format']:
    output = 'daemon-id\\n'
elif args[:2] == ['ps', '-aq']:
    output = '\\n'.join(ids) + '\\n'
elif args[:3] in (['network', 'ls', '-q'], ['volume', 'ls', '-q']):
    output = ''
elif args and (args[0] == 'inspect' or args[:2] in
        (['container', 'inspect'], ['network', 'inspect'],
         ['volume', 'inspect'], ['image', 'inspect'])):
    selected = args[1:] if args[0] == 'inspect' else args[2:]
    output = override or json.dumps([{'Id': name,
        'Config': {'Env': ['CANARY=' + 'x' * size, 'LAST=ok'], 'Labels': {'owner': name}},
        'Mounts': [{'Name': 'v-' + name, 'Destination': '/data'}],
        'State': {'Running': True}, 'Extra': {'nested': [1, {'token': name}]}}
        for name in selected], separators=(',', ':'))
else:
    raise SystemExit(2)
time.sleep(delay if args and args[0] == 'inspect' else 0)
sys.stdout.write(output)
sys.stdout.flush()
"""
        launched, processes = [], []
        actual = fixture.subprocess.Popen
        def peer(command, **kwargs):
            launched.append(command)
            process = actual([sys.executable, '-u', '-c', program,
                json.dumps(container_ids), str(payload_size), str(delay),
                inspect_reply or '', *command], **kwargs)
            processes.append(process)
            return process
        return peer, launched, processes

    def test_full_host_snapshot_batches_complete_inspect_metadata(self):
        ids = [f'host-{number:03d}' for number in range(65)]
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, 'fixture', fixture):
            backend = self._isolated_backend(directory)
            peer, launched, processes = self._inspection_peer(ids)
            try:
                with patch.object(fixture.subprocess, 'Popen', side_effect=peer):
                    snapshot = backend.provisioner.snapshot()
                self.assertEqual([item['Id'] for item in snapshot['containers']], ids)
                self.assertEqual(len(snapshot['containers']), 65)
                last = snapshot['containers'][-1]
                self.assertEqual(last['Config']['Env'], ['CANARY=' + 'x' * 65000, 'LAST=ok'])
                self.assertEqual(last['Config']['Labels'], {'owner': ids[-1]})
                self.assertEqual(last['Mounts'], [{'Name': 'v-' + ids[-1],
                    'Destination': '/data'}])
                self.assertEqual(last['State'], {'Running': True})
                self.assertEqual(last['Extra'], {'nested': [1, {'token': ids[-1]}]})
                self.assertEqual(snapshot['networks'], [])
                self.assertEqual(snapshot['volumes'], [])
                self.assertEqual(snapshot['routes'], [])
                inspect = [command for command in launched if command[1] == 'inspect']
                self.assertEqual([len(command) - 2 for command in inspect], [64, 1])
                self.assertTrue(all(process.poll() is not None for process in processes))
            finally:
                backend.release()

    def test_inspect_rejects_oversized_single_reply_and_aggregate(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, 'fixture', fixture):
            backend = self._isolated_backend(directory)
            try:
                for ids, size, expected_commands in [(['one'], 4 * 1024 * 1024 + 1, 1),
                                                      ([f'host-{n:03d}' for n in range(270)],
                                                       65000, 5)]:
                    with self.subTest(count=len(ids)):
                        peer, launched, processes = self._inspection_peer(ids, size)
                        with patch.object(fixture.subprocess, 'Popen', side_effect=peer):
                            with self.assertRaises((runner.ImportRejected,
                                                    fixture.ImportRejected)) as failure:
                                backend.provisioner._inspect('container', ids)
                        self.assertEqual(failure.exception.code, 'StdoutLimit')
                        self.assertEqual(len(launched), expected_commands)
                        self.assertTrue(all(process.poll() is not None for process in processes))
            finally:
                backend.release()

    def test_inspect_accepts_exact_object_count_limit_in_input_order(self):
        ids = [f'host-{number:04d}' for number in range(4096)]
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, 'fixture', fixture):
            backend = self._isolated_backend(directory)
            peer, launched, processes = self._inspection_peer([], 0)
            try:
                with patch.object(fixture.subprocess, 'Popen', side_effect=peer):
                    rows = backend.provisioner._inspect('container', ids)
                self.assertEqual([row['Id'] for row in rows], ids)
                self.assertEqual(len(launched), 64)
                self.assertTrue(all(len(command) - 2 == 64 for command in launched))
                self.assertTrue(all(process.poll() is not None for process in processes))
            finally:
                backend.release()

    def test_inspect_rejects_invalid_kind_count_and_json_shape(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, 'fixture', fixture):
            backend = self._isolated_backend(directory)
            try:
                peer, launched, _ = self._inspection_peer(['one'], 0)
                with patch.object(fixture.subprocess, 'Popen', side_effect=peer):
                    with self.assertRaises(runner.ImportRejected) as failure:
                        backend.provisioner._inspect('exec', ['one'])
                    self.assertEqual(failure.exception.code, 'Identity')
                    with self.assertRaises(runner.ImportRejected) as failure:
                        backend.provisioner._inspect('container', ['one'] * 4097)
                    self.assertEqual(failure.exception.code, 'InputLimit')
                self.assertEqual(launched, [])
                for reply in ('{bad', '{}', '[]', '[1]', '[{"Id":NaN}]'):
                    with self.subTest(reply=reply):
                        peer, launched, processes = self._inspection_peer(
                            ['one'], 0, inspect_reply=reply)
                        with patch.object(fixture.subprocess, 'Popen', side_effect=peer):
                            with self.assertRaises((runner.ImportRejected,
                                                    fixture.ImportRejected)) as failure:
                                backend.provisioner._inspect('container', ['one'])
                        self.assertEqual(failure.exception.code, 'Protocol')
                        self.assertEqual(len(launched), 1)
                        self.assertTrue(all(process.poll() is not None for process in processes))
            finally:
                backend.release()

    def test_inspect_kind_argv_is_closed_and_shared_by_issuer_and_pin(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, 'fixture', fixture):
            backend = self._isolated_backend(directory)
            peer, launched, processes = self._inspection_peer([], 0)
            try:
                self.assertIs(backend.issuer.target_provisioner, backend.provisioner)
                self.assertIs(backend.pin.target_provisioner, backend.provisioner)
                with patch.object(fixture.subprocess, 'Popen', side_effect=peer):
                    for kind in ('container', 'network', 'volume', 'image'):
                        with self.subTest(kind=kind):
                            rows = backend.pin.target_provisioner._inspect(kind, ['owned'])
                            self.assertEqual(rows[0]['Id'], 'owned')
                            self.assertEqual(rows[0]['Extra'],
                                             {'nested': [1, {'token': 'owned'}]})
                self.assertEqual([command[1:-1] for command in launched], [
                    ['inspect'], ['network', 'inspect'],
                    ['volume', 'inspect'], ['image', 'inspect']])
                self.assertTrue(all(process.poll() is not None for process in processes))
            finally:
                backend.release()

    def test_inspect_uses_one_operation_deadline_and_shorter_isolation_deadline(self):
        ids = [f'host-{n:03d}' for n in range(129)]
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, 'fixture', fixture):
            backend = self._isolated_backend(directory)
            try:
                peer, launched, processes = self._inspection_peer(ids, 0, delay=0.35)
                started = time.monotonic()
                with patch.object(runner, 'INSPECT_OPERATION_SECONDS', 0.6), \
                     patch.object(fixture.subprocess, 'Popen', side_effect=peer):
                    with self.assertRaises((runner.ImportRejected,
                                            fixture.ImportRejected)) as failure:
                        backend.provisioner._inspect('container', ids)
                self.assertEqual(failure.exception.code, 'Deadline')
                self.assertLess(time.monotonic() - started, 1.0)
                self.assertLess(len(launched), 3)
                self.assertTrue(all(process.poll() is not None for process in processes))
                actions = launched.copy()
                time.sleep(0.05)
                self.assertEqual(launched, actions)

                backend.commands.isolation_deadline = time.monotonic() + 0.2
                peer, launched, processes = self._inspection_peer(ids, 0, delay=0.4)
                with patch.object(fixture.subprocess, 'Popen', side_effect=peer):
                    with self.assertRaises((runner.ImportRejected,
                                            fixture.ImportRejected)) as failure:
                        backend.provisioner._inspect('container', ids)
                self.assertEqual(failure.exception.code, 'UnconfirmedIsolation')
                self.assertEqual(len(launched), 1)
                self.assertTrue(all(process.poll() is not None for process in processes))
                actions = launched.copy()
                time.sleep(0.05)
                self.assertEqual(launched, actions)
            finally:
                backend.release()

    def _isolated_backend(self, directory):
        root = Path(directory)
        base = root / 'base'
        base.mkdir()
        repository = Path(runner.__file__).parent.parent
        helper = runner._load(repository / runner.ARCHIVE_HELPER, 'test_private_archive_helper')
        helper._private_dir = lambda path: path.mkdir()
        helper._require_private_dir = lambda path: None
        helper._acceptance_lock = lambda path: contextlib.nullcontext()
        def extract(content, manifest, destination):
            destination.mkdir()
            for name in runner.REQUIRED:
                path = destination / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes((repository / name).read_bytes())
            return '1' * 64
        helper.extract_verified = extract
        helper._private_write = LocalFiles()._private_write
        helper._sync_dir = lambda path: None
        args = SimpleNamespace(batch_id='2b8a1252-54d5-48aa-b176-a9586a86bea1',
            source_batch_id='2b8a1252-54d5-48aa-b176-a9586a86bea2',
            target_batch_id='2b8a1252-54d5-48aa-b176-a9586a86bea3',
            archive_sha256='a' * 64, manifest_sha256='b' * 64, commit='c' * 40,
            runner_sha256='d' * 64, archive_helper_sha256='e' * 64, fixture_sha256='f' * 64)
        with patch.object(runner, 'BASE', base):
            backend = runner.ContractBackend(args, helper, None, None)
        for role, batch, cid in [('source', args.source_batch_id, 'a' * 64),
                                 ('target', args.target_batch_id, 'b' * 64)]:
            backend.resources.append({'role': role, 'batch_id': batch,
                'container_id': cid, 'inspection_sha256': '2' * 64,
                'identity': backend.provisioner.identity_for(batch), 'subnet': '172.30.240.0/28'})
        backend.execute = lambda: {'not_golden': True}
        backend.verify_source = lambda: True
        return backend

    def _stop_peer(self, backend, delay):
        live = {'containers': [], 'volumes': []}
        for row in backend.resources:
            identity = row['identity']
            labels = {'com.docker.compose.project': identity['project'], 'com.docker.compose.service': 'pg'}
            live['containers'].append({'Id': row['container_id'], 'Config': {'Labels': labels,
                'Image': identity['image']}, 'Mounts': [{'Name': identity['volume'],
                'Destination': '/var/lib/postgresql', 'Type': 'volume'}], 'State': {'Running': True}})
            live['volumes'].append({'Name': identity['volume'], 'Labels': labels})
        def snapshot():
            backend.provisioner._command('/usr/bin/docker', 'observe')
            return copy.deepcopy(live)
        backend.provisioner.snapshot = snapshot
        launched, processes = [], []
        actual = fixture.subprocess.Popen
        def peer(command, **kwargs):
            launched.append(command)
            if command[1] == 'stop':
                for container in live['containers']:
                    if container['Id'] == command[-1]:
                        container['State']['Running'] = False
            process = actual([sys.executable, '-u', '-c',
                f'import time; time.sleep({delay if command[1] == "observe" else 0})'], **kwargs)
            processes.append(process)
            return process
        return peer, launched, processes

    def test_reused_stalled_observation_is_reaped_and_cannot_publish_success(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, 'fixture', fixture):
            backend = self._isolated_backend(directory)
            peer, launched, processes = self._stop_peer(backend, 0.45)
            started = time.monotonic()
            with patch.object(runner, 'ISOLATION_SECONDS', 0.2, create=True), \
                 patch.object(fixture.subprocess, 'Popen', side_effect=peer), \
                 patch.object(runner, '_prepare_backend', return_value=backend):
                result = runner.run_contract(SimpleNamespace(phase='contract'))
            elapsed = time.monotonic() - started
            self.assertEqual(result['status'], runner.UNCONFIRMED)
            self.assertEqual(result['reason_code'], 'UnconfirmedIsolation')
            self.assertLess(elapsed, 0.5)
            self.assertTrue(all(process.poll() is not None for process in processes))
            self.assertFalse(any(command[1] == 'stop' for command in launched))
            actions = launched.copy()
            time.sleep(0.05)
            self.assertEqual(launched, actions, 'late action after isolation returned')
            persisted = json.loads((backend.batch / 'evidence' / 'result.json').read_bytes())
            self.assertEqual(persisted['status'], runner.UNCONFIRMED)

    def test_both_owned_resources_share_one_absolute_isolation_budget(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, 'fixture', fixture):
            backend = self._isolated_backend(directory)
            peer, launched, processes = self._stop_peer(backend, 0.12)
            started = time.monotonic()
            with patch.object(runner, 'ISOLATION_SECONDS', 0.5, create=True), \
                 patch.object(fixture.subprocess, 'Popen', side_effect=peer), \
                 patch.object(runner, '_prepare_backend', return_value=backend):
                result = runner.run_contract(SimpleNamespace(phase='contract'))
            self.assertEqual(result['status'], runner.UNCONFIRMED)
            self.assertLess(time.monotonic() - started, 0.8)
            self.assertTrue(all(process.poll() is not None for process in processes))
            self.assertTrue(any(process.returncode != 0 for process in processes),
                'deadline must interrupt a CLI, not reject after unbounded completion')

    def test_observation_command_output_is_bounded_before_exit(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, 'fixture', fixture):
            backend = self._isolated_backend(directory)
            actual = fixture.subprocess.Popen
            def peer(command, **kwargs):
                return actual([sys.executable, '-u', '-c',
                    "import sys; sys.stdout.buffer.write(b'x'*(4*1024*1024+1)); sys.stdout.flush()"], **kwargs)
            try:
                with patch.object(fixture.subprocess, 'Popen', side_effect=peer):
                    with self.assertRaisesRegex(fixture.ImportRejected, 'StdoutLimit'):
                        backend.provisioner._command('/usr/bin/docker', 'observe')
            finally:
                backend.release()

    def test_observation_caps_and_clean_environment_preserve_docker_progress(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, 'fixture', fixture):
            backend = self._isolated_backend(directory)
            actual = fixture.subprocess.Popen
            program = """import json, os, sys
sys.stderr.buffer.write(b'p'*16384); sys.stderr.flush()
print(json.dumps({key: os.environ.get(key) for key in ['LC_ALL','DOCKER_HOST','PGHOST','HOME']})+'|'+('x'*12000))
"""
            def peer(command, **kwargs):
                return actual([sys.executable, '-u', '-c', program], **kwargs)
            try:
                with patch.object(fixture.subprocess, 'Popen', side_effect=peer), \
                     patch.dict(runner.os.environ,
                        {'PGHOST': 'untrusted', 'HOME': 'untrusted'}):
                    output = backend.provisioner._command('/usr/bin/docker', 'observe')
                environment, payload = output.split('|', 1)
                self.assertEqual(json.loads(environment), {'LC_ALL': 'C',
                    'DOCKER_HOST': 'unix:///var/run/docker.sock', 'PGHOST': None, 'HOME': None})
                self.assertEqual(payload.rstrip('\r\n'), 'x' * 12000)
            finally:
                backend.release()

    def test_observation_stderr_cap_does_not_expose_raw_progress(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, 'fixture', fixture):
            backend = self._isolated_backend(directory)
            actual = fixture.subprocess.Popen
            def peer(command, **kwargs):
                return actual([sys.executable, '-u', '-c',
                    "import sys; sys.stderr.buffer.write(b'private' * 10000); sys.stderr.flush()"], **kwargs)
            try:
                with patch.object(fixture.subprocess, 'Popen', side_effect=peer):
                    with self.assertRaises(fixture.ImportRejected) as failure:
                        backend.provisioner._command('/usr/bin/docker', 'observe')
                self.assertEqual(str(failure.exception), 'StderrLimit')
            finally:
                backend.release()

    def test_bootstrap_does_not_execute_unverified_adjacent_fixture(self):
        # Removing deferred loading would execute this unapproved file even
        # before argument parsing/admission, observable through its marker.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'entry.py').write_bytes(Path(runner.__file__).read_bytes())
            marker = root / 'unverified-executed'
            (root / 'p0c4_import_fixture.py').write_text(
                f"from pathlib import Path\nPath({str(marker)!r}).write_text('executed')\n"
                "class ImportRejected(RuntimeError): pass\ndef require(*args): pass\n")
            result = subprocess.run([sys.executable, '-B', str(root / 'entry.py'), '--help'],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
            self.assertEqual(result.returncode, 0)
            self.assertFalse(marker.exists(), 'unverified fixture executed before hash gate')

    def test_extracted_control_helpers_load_without_unsealed_adjacent_imports(self):
        # A missing binding previously let pin import birth from the caller's
        # sys.path, or crash when the deployed tools directory held only three
        # sealed scripts. Exercise that installation shape in a fresh process.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tools, bundle = root / 'tools', root / 'bundle'
            tools.mkdir()
            bundle.mkdir()
            for name in (runner.ENTRY, runner.ARCHIVE_HELPER, runner.FIXTURE):
                (tools / Path(name).name).write_bytes((Path(runner.__file__).parent.parent / name).read_bytes())
            for name in (runner.PROVISIONER, runner.ISSUER, runner.PIN, runner.PREPARE):
                path = bundle / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes((Path(runner.__file__).parent.parent / name).read_bytes())
            (bundle / runner.INITDB).parent.mkdir(parents=True)
            (bundle / runner.INITDB).write_bytes(b'#!/bin/sh\n')
            (root / 'base').mkdir()
            program = """import contextlib, importlib.util, pathlib, shutil, sys
from types import SimpleNamespace
sys.path.insert(0, sys.argv[1])
import p0c4_controlled_import_acceptance as runner
import p0c4_restore_birth_acceptance as helper
root, bundle = pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3])
runner.BASE = root
helper._require_private_dir = lambda path: None
helper._private_dir = lambda path: path.mkdir()
helper._acceptance_lock = lambda path: contextlib.nullcontext()
helper.extract_verified = lambda content, manifest, destination: (shutil.copytree(bundle, destination), 'sealed')[1]
helper._private_write = lambda path, payload, mode: path.write_bytes(payload)
sentinel = object()
sys.modules['p0c4_restore_target_birth'] = sentinel
backend = runner.ContractBackend(SimpleNamespace(batch_id='2b8a1252-54d5-48aa-b176-a9586a86bea3'), helper, None, None)
assert backend.pin.issuer is backend.issuer
assert backend.pin.acceptance is helper
assert sys.modules['p0c4_restore_target_birth'] is sentinel
backend.release()
print('CONTROL_HELPERS_LOADED')
"""
            result = subprocess.run([sys.executable, '-B', '-c', program, str(tools),
                str(root / 'base'), str(bundle)], stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, timeout=5, cwd=root)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            self.assertEqual(result.stdout.splitlines(), [b'CONTROL_HELPERS_LOADED'])

    def test_contract_rejects_replayed_names(self):
        args = SimpleNamespace(phase="contract", source_batch_id="2b8a1252-54d5-48aa-b176-a9586a86bea3",
                               target_batch_id="2b8a1252-54d5-48aa-b176-a9586a86bea3")
        with self.assertRaises(runner.ImportRejected):
            runner.run_contract(args)

    def test_contract_redacts_failures_and_publishes_after_stop(self):
        for fail in (False, True):
            backend = Backend(fail)
            with patch.object(runner, "_prepare_backend", return_value=backend):
                result = runner.run_contract(SimpleNamespace(phase="contract"))
            self.assertTrue(backend.stopped)
            self.assertEqual(result["status"], "CONTRACT_FAILED_QUARANTINED_NOT_IMPORT" if fail else
                             "CLIENT_CONTRACT_AND_FIXTURE_CAPTURE_PASSED_NOT_IMPORT")
            self.assertEqual(result["reason_code"], "Io" if fail else None)
            self.assertNotIn("password", str(result))
            self.assertNotIn("SELECT", str(result))
            self.assertEqual(backend.published, result)

    def test_stop_failure_cannot_publish_success(self):
        backend = Backend(stop_fail=True)
        with patch.object(runner, "_prepare_backend", return_value=backend):
            result = runner.run_contract(SimpleNamespace(phase="contract"))
        self.assertEqual(result["status"], "CONTRACT_FAILED_UNCONFIRMED_ISOLATION_NOT_IMPORT")
        self.assertEqual(result["reason_code"], "UnconfirmedIsolation")

    def test_rejects_other_phase_before_any_resource_action(self):
        with self.assertRaises(runner.ImportRejected):
            runner.run_contract(SimpleNamespace(phase="import"))

    def test_changed_source_cannot_publish_success(self):
        backend = Backend()
        backend.verify_source = lambda: False
        with patch.object(runner, "_prepare_backend", return_value=backend):
            result = runner.run_contract(SimpleNamespace(phase='contract'))
        self.assertEqual(result['status'], 'CONTRACT_FAILED_QUARANTINED_NOT_IMPORT')
        self.assertEqual(result['reason_code'], 'Identity')

    def test_import_rejection_publishes_only_fixed_code(self):
        backend = Backend()
        def reject():
            raise runner.ImportRejected('Stderr')
        backend.execute = reject
        with patch.object(runner, '_prepare_backend', return_value=backend):
            result = runner.run_contract(SimpleNamespace(phase='contract'))
        self.assertEqual(result['reason_code'], 'Stderr')
        self.assertEqual(result['status'], 'CONTRACT_FAILED_QUARANTINED_NOT_IMPORT')

    def test_actual_artifacts_are_separate_and_target_readback_is_required(self):
        with tempfile.TemporaryDirectory() as directory:
            backend = runner.ContractBackend.__new__(runner.ContractBackend)
            backend.batch = Path(directory)
            (backend.batch / 'artifacts').mkdir()
            backend.helper = LocalFiles()
            backend.args = SimpleNamespace(source_batch_id='2b8a1252-54d5-48aa-b176-a9586a86bea3',
                target_batch_id='2b8a1252-54d5-48aa-b176-a9586a86bea4',
                source_subnet='172.30.240.0/28', target_subnet='172.30.240.16/28')
            backend._budget = lambda: None
            source = {'role': 'source', 'identity': {'database': 'learning_restore_c4_' + backend.args.source_batch_id},
                'subnet': backend.args.source_subnet, 'before': {}, 'container_id': 'a' * 64}
            target = {'role': 'target', 'identity': {'database': 'learning_restore_c4_' + backend.args.target_batch_id},
                'subnet': backend.args.target_subnet, 'before': {}, 'container_id': 'b' * 64}
            for row in (source, target):
                row['identity']['image'] = 'sealed-image'
            backend._create = lambda role, *args: source if role == 'source' else target
            backend._client_contract = lambda row: {'ready_stdin_open': True}
            backend.provisioner = SimpleNamespace(snapshot=lambda: {},
                identity_for=lambda batch: source['identity'] if batch == backend.args.source_batch_id else target['identity'],
                admit_fresh=lambda *args: None, _inspect=lambda *args: [],
                verify_created=lambda identity, *args: {'container_id': source['container_id'] if identity is source['identity'] else target['container_id']})
            readbacks = []
            backend.issuer = SimpleNamespace(probe_pg_facts=lambda identity, cid: readbacks.append(cid))
            captured = {'dump': b'PGDMPcontrolled-fixture', 'sql': b'private synthetic SQL\n',
                'source_eof_insert_rolled_back': True, 'not_golden': True}
            with patch.object(runner, 'fixture', fixture), patch.object(fixture, 'capture_fixture', return_value=captured.copy()):
                result = backend.execute()
            self.assertEqual(readbacks, ['b' * 64])
            self.assertEqual((backend.batch / 'artifacts' / 'fixture.dump').read_bytes(), b'PGDMPcontrolled-fixture')
            self.assertEqual((backend.batch / 'artifacts' / 'decoded.sql').read_bytes(), b'private synthetic SQL\n')
            self.assertEqual(result['artifacts']['dump_sha256'], hashlib.sha256(b'PGDMPcontrolled-fixture').hexdigest())
            self.assertNotIn('private synthetic SQL', json.dumps(result))
            self.assertTrue(result['target_baseline_unchanged'])
            backend.issuer.probe_pg_facts = lambda *args: (_ for _ in ()).throw(ValueError('dirty baseline'))
            # A fresh artifact directory is necessary; existing entries refuse.
            for path in (backend.batch / 'artifacts').iterdir():
                path.unlink()
            with patch.object(runner, 'fixture', fixture), patch.object(fixture, 'capture_fixture', return_value=captured.copy()):
                with self.assertRaisesRegex(ValueError, 'dirty baseline'):
                    backend.execute()

    def test_artifact_readback_mismatch_cannot_pass(self):
        # Existing test exercises real write/readback; corrupt only the read
        # boundary to prove a post-write mutation rejects before publication.
        with patch.object(LocalFiles, '_private_read', return_value=b'changed'):
            with self.assertRaisesRegex(runner.ImportRejected, 'Fixture'):
                self.test_actual_artifacts_are_separate_and_target_readback_is_required()

    def test_real_publication_refuses_existing_final_and_removes_pending(self):
        with tempfile.TemporaryDirectory() as directory:
            backend = runner.ContractBackend.__new__(runner.ContractBackend)
            backend.batch = Path(directory)
            backend.helper = LocalFiles()
            evidence = backend.batch / 'evidence'
            evidence.mkdir()
            result = {'status': runner.FAILED, 'reason_code': 'Fixture', 'not_import': True}
            published = backend.publish(result)
            self.assertEqual(json.loads((evidence / 'result.json').read_bytes()), result)
            self.assertFalse((evidence / 'result.pending').exists())
            self.assertEqual(published['result_sha256'], hashlib.sha256((evidence / 'result.json').read_bytes()).hexdigest())
            with self.assertRaisesRegex(runner.ImportRejected, 'Journal'):
                backend.publish({'status': runner.PASSED})
            self.assertEqual(json.loads((evidence / 'result.json').read_bytes()), result)

    def test_final_file_persists_verified_package_and_exact_resource_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            backend = runner.ContractBackend.__new__(runner.ContractBackend)
            backend.batch = Path(directory)
            (backend.batch / 'evidence').mkdir()
            backend.helper = LocalFiles()
            backend.args = SimpleNamespace(batch_id='2b8a1252-54d5-48aa-b176-a9586a86bea1',
                source_batch_id='2b8a1252-54d5-48aa-b176-a9586a86bea2',
                target_batch_id='2b8a1252-54d5-48aa-b176-a9586a86bea3',
                archive_sha256='a' * 64, manifest_sha256='b' * 64, commit='c' * 40,
                runner_sha256='d' * 64, archive_helper_sha256='e' * 64, fixture_sha256='f' * 64)
            backend.source_hash = '1' * 64
            backend.resources = [dict(role=role, container_id=cid * 64, inspection_sha256=sha * 64,
                batch_id=batch, subnet=subnet, identity={'database': 'learning_restore_c4_' + batch,
                    'project': 'project-' + role, 'volume': 'volume-' + role})
                for role, cid, sha, batch, subnet in [
                    ('source', '2', '3', backend.args.source_batch_id, '172.30.240.0/28'),
                    ('target', '4', '5', backend.args.target_batch_id, '172.30.240.16/28')]]
            backend.execute = lambda: {'not_golden': True}
            backend.stop = lambda: {'source_stopped': True, 'target_stopped': True, 'volumes_retained': True}
            backend.verify_source = lambda: True
            backend.release = lambda: None
            with patch.object(runner, '_prepare_backend', return_value=backend):
                runner.run_contract(SimpleNamespace(phase='contract'))
            persisted = json.loads((backend.batch / 'evidence' / 'result.json').read_bytes())
            expected = {'package': {'commit': 'c' * 40, 'archive_sha256': 'a' * 64,
                    'manifest_sha256': 'b' * 64, 'runner_sha256': 'd' * 64,
                    'archive_helper_sha256': 'e' * 64, 'fixture_sha256': 'f' * 64,
                    'source_sha256': '1' * 64},
                'batches': {'control': '2b8a1252-54d5-48aa-b176-a9586a86bea1',
                    'source': '2b8a1252-54d5-48aa-b176-a9586a86bea2',
                    'target': '2b8a1252-54d5-48aa-b176-a9586a86bea3'},
                'resources': {'source': {'container_id': '2' * 64, 'inspection_sha256': '3' * 64,
                        'database': 'learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea2',
                        'project': 'project-source', 'volume': 'volume-source', 'subnet': '172.30.240.0/28'},
                    'target': {'container_id': '4' * 64, 'inspection_sha256': '5' * 64,
                        'database': 'learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3',
                        'project': 'project-target', 'volume': 'volume-target', 'subnet': '172.30.240.16/28'}}}
            self.assertEqual(persisted.get('identity'), expected)

    def test_archive_checks_actual_zip_members_and_independent_helper_seals(self):
        import p0c4_restore_birth_acceptance as old
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            installed = root / Path(runner.ENTRY).name
            members = {name: (Path(runner.__file__).parent.parent / name).read_bytes() for name in runner.REQUIRED}
            for name in (runner.ENTRY, runner.ARCHIVE_HELPER, runner.FIXTURE):
                (root / Path(name).name).write_bytes(members[name])
            def archive(omitted=None):
                included = {key: value for key, value in members.items() if key != omitted}
                manifest = {'format_version': 1, 'commit': 'd' * 40, 'files': [
                    {'path': name, 'sha256': hashlib.sha256(payload).hexdigest(), 'size': len(payload)}
                    for name, payload in sorted(included.items())]}
                manifest_bytes = json.dumps(manifest, sort_keys=True, separators=(',', ':')).encode()
                content = io.BytesIO()
                with zipfile.ZipFile(content, 'w') as sealed:
                    for name, payload in {**included, 'SOURCE_MANIFEST.json': manifest_bytes}.items():
                        info = zipfile.ZipInfo(name)
                        info.external_attr = (stat.S_IFREG | 0o400) << 16
                        sealed.writestr(info, payload)
                destination = root / 'incoming' / 'source.zip'
                destination.parent.mkdir(exist_ok=True)
                destination.write_bytes(content.getvalue())
                return SimpleNamespace(archive=destination, archive_sha256=hashlib.sha256(content.getvalue()).hexdigest(),
                    manifest_sha256=hashlib.sha256(manifest_bytes).hexdigest(), commit='d' * 40,
                    runner_sha256=hashlib.sha256(members[runner.ENTRY]).hexdigest(),
                    archive_helper_sha256=hashlib.sha256(members[runner.ARCHIVE_HELPER]).hexdigest(),
                    fixture_sha256=hashlib.sha256(members[runner.FIXTURE]).hexdigest())
            actual_load = runner._load
            def load(path, name):
                module = actual_load(path, name)
                if name == 'p0c4_import_archive_helper_private':
                    module.BASE = root
                    module._require_private_dir = lambda path: None
                    module._trusted_path = lambda *args, **kwargs: None
                    # Linux ownership/modes and no-follow flags are external;
                    # ZIP bytes, manifest hashes and member inventory stay real.
                    module.os = SimpleNamespace(lstat=lambda path: SimpleNamespace(
                        st_mode=stat.S_IFREG | (0o500 if Path(path) == installed else 0o400), st_nlink=1),
                        open=old.os.open, fdopen=old.os.fdopen, O_RDONLY=old.os.O_RDONLY,
                        O_NOFOLLOW=0, O_CLOEXEC=0)
                return module
            def check(path, sha, mode):
                runner.require(hashlib.sha256(path.read_bytes()).hexdigest() == sha, 'Identity')
            original_entry, original_inventory = old.ENTRY, old.REQUIRED.copy()
            with patch.object(runner, '__file__', str(installed)), patch.object(runner, '_load', side_effect=load), \
                 patch.object(runner, '_checked_installed', side_effect=check), patch.object(runner, 'fixture', None):
                helper, manifest, content = runner._archive_contract(archive())
                self.assertEqual(helper.ENTRY, 'scripts/p0c4_controlled_import_acceptance.py')
                self.assertEqual(manifest['commit'], 'd' * 40)
                for missing in (runner.ARCHIVE_HELPER, runner.FIXTURE, runner.PIN, runner.PREPARE):
                    with self.subTest(missing=missing), self.assertRaisesRegex(ValueError, 'inventory'):
                        runner._archive_contract(archive(missing))
                args = archive()
                args.fixture_sha256 = 'f' * 64
                with self.assertRaisesRegex(runner.ImportRejected, 'Identity'):
                    runner._archive_contract(args)
            self.assertEqual(old.ENTRY, original_entry)
            self.assertEqual(old.REQUIRED, original_inventory)


class ImportPhaseTests(unittest.TestCase):
    """Local runner decisions only; no Docker, PostgreSQL, or live evidence."""

    def entry(self, name):
        function = getattr(runner, name, None)
        self.assertTrue(callable(function), f"missing closed import predicate: {name}")
        return function

    def receipt(self, case):
        """Synthetic libtest text exercises parser only, never live acceptance."""
        _, failure, points = runner.IMPORT_CASES[case]
        return {'schema': 1, 'case': case, 'failure': failure,
            'checkpoints': [*runner.IMPORT_COMMON_CHECKPOINTS, *points],
            'commit_attempted': case in ('success', 'commit-unknown'),
            'rollback_verified': case in ('ready-eof', 'precommit-eof', 'precommit-cancel'),
            'retry_allowed': False, 'dump_sha256': 'a' * 64, 'toc_sha256': 'b' * 64,
            'raw_sql_sha256': 'c' * 64, 'transformed_sql_sha256': 'd' * 64,
            'content_sha256': hashlib.sha256(b'1|alpha\n2|beta\n').hexdigest() if case == 'success' else None}

    def output(self, case, record):
        name = runner.IMPORT_PREFIX + runner.IMPORT_CASES[case][0]
        return (f'\nrunning 1 test\ntest {name} ... '
                f'KW_C4_IMPORT|{json.dumps(record, separators=(",", ":"))}\nok\n\n'
                'test result: ok. 1 passed; 0 failed; 0 ignored; '
                '0 measured; 123 filtered out; finished in 0.01s\n').encode()

    def test_import_receipt_rejects_each_missing_or_fabricated_checkpoint(self):
        parse = self.entry('_import_observations')
        for case in runner.IMPORT_CASES:
            record = self.receipt(case)
            self.assertEqual(parse(self.output(case, record), case, 0), record)
            for point in list(record['checkpoints']):
                missing = copy.deepcopy(record)
                missing['checkpoints'].remove(point)
                with self.subTest(case=case, point=point), self.assertRaises(runner.ImportRejected):
                    parse(self.output(case, missing), case, 0)
            for extra in ('MODEL_PASSED', record['checkpoints'][0]):
                surplus = copy.deepcopy(record)
                surplus['checkpoints'].append(extra)
                with self.subTest(case=case, extra=extra), self.assertRaises(runner.ImportRejected):
                    parse(self.output(case, surplus), case, 0)

    def test_import_receipt_rejects_non_exact_process_and_count_evidence(self):
        parse = self.entry('_import_observations')
        record = self.receipt('success')
        valid = self.output('success', record)
        mutations = [valid.replace(b'1 passed', b'0 passed'),
                     valid.replace(b'0 ignored', b'1 ignored'),
                     valid.replace(b'0 failed', b'1 failed'),
                     valid.replace(b'\nok\n', b'\nFAILED\n'),
                     valid.replace(b'running 1 test', b'running 2 tests'),
                     valid.replace(b'\n\ntest result:', b'\ntest unrelated ... ok\n\ntest result:'),
                     valid.replace(b'live_controlled_import_commits_fixture', b'other_test'),
                     valid + valid,
                     valid.replace(b'"schema":1', b'"schema":1,"schema":1'),
                     valid.replace(b'"schema":1', b'"schema":true')]
        for output in mutations:
            with self.subTest(output=output[:80]), self.assertRaises(runner.ImportRejected):
                parse(output, 'success', 0)
        for code in (1, -9, True, None):
            with self.subTest(code=code), self.assertRaises(runner.ImportRejected):
                parse(valid, 'success', code)

    def test_commit_unknown_complete_receipt_rejects_retry_and_rollback_claims(self):
        parse = self.entry('_import_observations')
        record = self.receipt('commit-unknown')
        for key, invalid in [('rollback_verified', True), ('retry_allowed', True),
                             ('commit_attempted', False), ('failure', None)]:
            changed = {**record, key: invalid}
            with self.subTest(key=key), self.assertRaises(runner.ImportRejected):
                parse(self.output('commit-unknown', changed), 'commit-unknown', 0)

    def test_success_requires_actual_independent_content_digest(self):
        record = self.receipt('success')
        for invalid in (None, 'a' * 64, True):
            with self.subTest(invalid=invalid), self.assertRaises(runner.ImportRejected):
                runner._import_observations(self.output('success', {**record, 'content_sha256': invalid}), 'success', 0)

    def test_compile_only_placeholder_injects_separate_source_pin_and_lists_all_eleven(self):
        backend = object.__new__(runner.ImportBackend)
        backend.source = Path('/private/source')
        backend.batch = Path('/private/batch')
        backend.source_hash = 'c' * 64
        commands, build_names, cleanup_expectations = [], [], []
        binary = Path('/private/batch/test-binary')
        def bounded(command, **kwargs):
            commands.append(command)
            if command[:2] == ['/usr/bin/docker', 'run']:
                return SimpleNamespace(returncode=0)
            return SimpleNamespace(returncode=0, stderr=b'', stdout=b'\n'.join(
                (runner.IMPORT_PREFIX + value[0] + ': test').encode() for value in runner.IMPORT_CASES.values()))
        def compile_probe(source, batch, name, target_pin):
            build_names.append(name)
            backend.delegate._run_bounded(['/usr/bin/docker', 'run', '--network', 'none',
                '--env', 'KNOWWEAVE_C4_TARGET_BIRTH_SHA256=' + target_pin,
                '--entrypoint', '/bin/sh', runner.BUILDER_ID])
            backend.delegate._cleanup_builder(batch, 'preflight' if name == 'probe-preflight-build' else 'live')
            return binary, 'b' * 64
        backend.delegate = SimpleNamespace(_run_bounded=bounded, _compile_bound_probe=compile_probe,
                                          _cleanup_builder=lambda *args, **kwargs: cleanup_expectations.append(kwargs),
                                          _builder_identity=lambda *args: ('builder', 'label=value'),
                                          _builder_container_ids=lambda *args: [],
                                          _file_digest=lambda path: 'b' * 64)
        def inspected(command, kwargs, observed):
            observed.update(container_id='e' * 64, cleanup_confirmed=False)
            return bounded(command, **kwargs)
        backend._run_import_builder = inspected
        result = backend._preflight_import_builder()
        self.assertTrue(result['placeholder'])
        self.assertFalse(result['executed'])
        self.assertEqual(len(result['exact_tests_listed']), 11)
        self.assertEqual(build_names, ['probe-preflight-build'])
        self.assertEqual(commands[0].count('KNOWWEAVE_C4_IMPORT_SOURCE_BIRTH_SHA256=' + '0' * 64), 1)
        for option in ('--cpus=4', '--memory=8g', '--memory-swap=8g'):
            self.assertEqual(commands[0].count(option), 1)
        self.assertTrue(result['builder_observation']['cleanup_confirmed'])
        self.assertEqual(cleanup_expectations, [{'expected_id': 'e' * 64}])
        self.assertEqual(commands[1], [str(binary), '--list', '--ignored'])
        self.assertIs(backend.delegate._run_bounded, bounded)
        backend._compile_import_binary('a' * 64, 'd' * 64, placeholder=False)
        self.assertEqual(build_names[-1], 'probe-live-build')
        self.assertIn('KNOWWEAVE_C4_IMPORT_SOURCE_BIRTH_SHA256=' + 'a' * 64, commands[-1])
        for source_pin, target_pin in [('0' * 64, '0' * 64), ('a' * 64, 'a' * 64), ('bad', 'a' * 64)]:
            with self.subTest(source_pin=source_pin), self.assertRaises(runner.ImportRejected):
                backend._compile_import_binary(source_pin, target_pin, placeholder=False)

    def test_import_cleanup_never_removes_replacement_after_admission_inventory(self):
        # Exercise the real cleanup recipe through the import adapter. The
        # label inventory changes AFTER its initial admission check.
        delegate = runner._load(Path(__file__).with_name('p0c4_restore_pin_acceptance.py'),
                                'task6_builder_cleanup_model')
        batch = Path('/private/2b8a1252-54d5-48aa-b176-a9586a86bea3')
        captured, replacement = 'a' * 64, 'b' * 64
        for first_inventory in ([captured], []):
            backend = object.__new__(runner.ImportBackend)
            backend.source, backend.batch, backend.source_hash = Path('/private/source'), batch, 'c' * 64
            backend.delegate = delegate
            observed_reference = []
            def inspected(command, kwargs, observed):
                observed.update(container_id=captured, cleanup_confirmed=False)
                observed_reference.append(observed)
                return SimpleNamespace(returncode=0)
            backend._run_import_builder = inspected
            def compile_probe(source, root, build_name, target_pin):
                delegate._run_bounded(['/usr/bin/docker', 'run', '--network', 'none',
                    '--env', 'KNOWWEAVE_C4_TARGET_BIRTH_SHA256=' + target_pin,
                    '--entrypoint', '/bin/sh', runner.BUILDER_ID])
                delegate._cleanup_builder(root, 'preflight')
                return root / 'binary', 'd' * 64
            name, _ = delegate._builder_identity(batch, 'preflight')
            facts = [{'Id': replacement, 'Name': '/' + name, 'Image': runner.BUILDER_ID,
                      'Config': {'Labels': {'com.knowweave.bound-probe.batch': batch.name}}}]
            with self.subTest(first_inventory=first_inventory), \
                 patch.object(delegate, '_compile_bound_probe', side_effect=compile_probe), \
                 patch.object(delegate, '_file_digest', return_value='d' * 64), \
                 patch.object(delegate, '_builder_container_ids', side_effect=[first_inventory, [replacement], []]), \
                 patch.object(delegate, '_probe_docker', side_effect=[
                     SimpleNamespace(returncode=0, stdout=json.dumps(facts).encode()),
                     SimpleNamespace(returncode=0, stdout=b'')]) as docker:
                failure = None
                try:
                    backend._compile_import_binary('0' * 64, '0' * 64, placeholder=True)
                except ValueError as error:
                    failure = error
                destructive = [call.args[0] for call in docker.call_args_list
                               if call.args[0][1] in ('rm', 'stop', 'kill')]
                self.assertEqual(destructive, [], 'replacement B must never be removed or stopped')
                self.assertIsNotNone(failure)
                self.assertFalse(observed_reference[0]['cleanup_confirmed'])

    def test_listing_missing_one_case_never_authorizes_resource_creation(self):
        backend = object.__new__(runner.ImportBackend)
        backend.source, backend.batch = Path('/source'), Path('/batch')
        for missing in runner.IMPORT_CASES:
            listing = b'\n'.join((runner.IMPORT_PREFIX + value[0] + ': test').encode()
                for key, value in runner.IMPORT_CASES.items() if key != missing)
            backend.delegate = SimpleNamespace(_file_digest=lambda path: 'b' * 64,
                _run_bounded=lambda *args, **kwargs: SimpleNamespace(returncode=0, stderr=b'', stdout=listing))
            with self.subTest(missing=missing), self.assertRaises(runner.ImportRejected):
                backend._list_import_tests(Path('/binary'), 'b' * 64)

    def builder(self):
        backend = object.__new__(runner.ImportBackend)
        backend.source, backend.batch = Path('/source'), Path('/batch')
        build = backend.batch / 'probe-preflight-build'
        command = ['/usr/bin/docker', 'run', '--name', 'builder', '--label', 'owner=case',
                   '--mount', f'type=bind,src={backend.source},dst=/reviewed,readonly',
                   '--mount', f'type=bind,src={build},dst=/target']
        row = {'Id': 'a' * 64, 'Name': '/builder', 'Image': runner.BUILDER_ID,
            'State': {'Running': True}, 'Config': {'Image': runner.BUILDER_ID,
                'Labels': {'owner': 'case'}, 'User': '0:0', 'Env': ['PRIVATE=do-not-emit']},
            'HostConfig': {'NanoCpus': 4_000_000_000, 'Memory': 8 * 1024**3,
                'MemorySwap': 8 * 1024**3, 'NetworkMode': 'none', 'Privileged': False,
                'CapDrop': ['ALL'], 'SecurityOpt': ['no-new-privileges']},
            'Mounts': [{'Type': 'bind', 'Source': str(backend.source), 'Destination': '/reviewed', 'RW': False},
                       {'Type': 'bind', 'Source': str(build), 'Destination': '/target', 'RW': True}]}
        return backend, command, row

    def test_builder_receipt_requires_actual_limits_identity_and_readonly_source(self):
        backend, command, row = self.builder()
        projection = backend._builder_projection(command, row)
        self.assertEqual(projection['container_id'], row['Id'])
        self.assertNotIn('PRIVATE', json.dumps(projection))
        for parent, key, invalid in [('HostConfig', 'Memory', 0), ('HostConfig', 'MemorySwap', -1),
                ('HostConfig', 'NanoCpus', 1), ('HostConfig', 'Privileged', True),
                ('HostConfig', 'NetworkMode', 'bridge'), ('State', 'Running', False),
                ('Config', 'Image', 'other'), (None, 'Id', 'short')]:
            changed = copy.deepcopy(row)
            (changed[parent] if parent else changed)[key] = invalid
            with self.subTest(key=key), self.assertRaises(runner.ImportRejected):
                backend._builder_projection(command, changed)
        changed = copy.deepcopy(row)
        changed['Mounts'][0]['RW'] = True
        with self.assertRaises(runner.ImportRejected):
            backend._builder_projection(command, changed)

    def test_builder_observation_failure_reaps_owned_real_child_without_name_substitution(self):
        backend, command, row = self.builder()
        original = fixture.subprocess.Popen
        processes, inspected = [], []
        def child(_command, **kwargs):
            process = original([sys.executable, '-u', '-c', 'import time; time.sleep(30)'], **kwargs)
            processes.append(process)
            return process
        replacement = {**row, 'Id': 'b' * 64}
        replies = iter([row, replacement])
        def inspect(binary, args, deadline):
            inspected.append(args[-1])
            return json.dumps([next(replies)])
        backend.commands = SimpleNamespace(_run=inspect)
        with patch.object(runner, 'fixture', fixture), patch.object(fixture.subprocess, 'Popen', side_effect=child):
            with self.assertRaises(runner.ImportRejected):
                backend._run_import_builder(command, {'timeout': 30, 'env': {'LC_ALL': 'C'}}, {})
        self.assertEqual(inspected, ['builder', 'a' * 64])
        self.assertEqual(len(processes), 1)
        self.assertIsNotNone(processes[0].poll())
        self.assertTrue(all(pipe.closed for pipe in (processes[0].stdin, processes[0].stdout, processes[0].stderr)))

    def test_marker_records_require_actual_case_stage_and_bound_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            batch = '2b8a1252-54d5-48aa-b176-a9586a86bea3'
            database = 'learning_restore_c4_' + batch
            control = root / 'targets' / batch / 'control'
            control.mkdir(parents=True)
            target = {'control': root, 'batch_id': batch, 'birth_sha256': 'e' * 64,
                      'identity': {'database': database}}
            backend = object.__new__(runner.ImportBackend)
            backend.helper = SimpleNamespace(_private_read=lambda path, **kwargs: path.read_bytes())
            observation = self.receipt('success')
            attempt = {'format_version': 1, 'record_type': 'CONTROLLED_IMPORT_CANDIDATE',
                'phase': 'ATTEMPT', 'batch_id': batch, 'database': database, 'fixture_version': 1,
                'birth_sha256': 'e' * 64, 'inspection_sha256': 'f' * 64,
                'dump_sha256': 'a' * 64, 'raw_sql_sha256': 'c' * 64,
                'transformed_sql_sha256': 'd' * 64, 'writer_sha256': '1' * 64}
            attempt_path = control / (database + '.restore.attempt')
            intent_path = control / (database + '.restore.commit-attempt')
            def save(record):
                raw = json.dumps(record).encode()
                attempt_path.write_bytes(raw)
                intent = {'format_version': 1, 'record_type': 'CONTROLLED_IMPORT_CANDIDATE',
                    'phase': 'COMMIT_ATTEMPTED', 'database': database,
                    'attempt_sha256': hashlib.sha256(raw).hexdigest(), 'writer_sha256': record['writer_sha256']}
                intent_path.write_text(json.dumps(intent))
            save(attempt)
            self.assertTrue(backend._markers(target, observation)['intent']['exists'])
            for key, value in [('phase', 'COMMITTED'), ('batch_id', '11111111-1111-4111-8111-111111111111'),
                               ('writer_sha256', 'caller-token'), ('format_version', True)]:
                save({**attempt, key: value})
                with self.subTest(key=key), self.assertRaises(runner.ImportRejected):
                    backend._markers(target, observation)
            save(attempt)
            with self.assertRaises(runner.ImportRejected):
                backend._markers(target, self.receipt('ready-eof'))
            intent_path.unlink()
            with self.assertRaises(runner.ImportRejected):
                backend._markers(target, observation)

    def test_missing_gate1_attestation_prevents_creation(self):
        run = self.entry('run_import_case')
        with patch.object(runner, '_prepare_backend') as prepare:
            with self.assertRaisesRegex(runner.ImportRejected, 'Identity'):
                run(SimpleNamespace(phase='import'), 'success')
        prepare.assert_not_called()

    def test_selects_one_exact_live_test(self):
        command = self.entry('_import_test_command')
        expected = ('restore_preflight::target_binding::controlled_import::'
                    'live_tests::live_controlled_import_commits_fixture')
        self.assertEqual(command(Path('/private/test-binary'), 'success'),
            [str(Path('/private/test-binary')), '--exact', expected, '--ignored',
             '--nocapture', '--test-threads=1'])
        for case in ('', 'all', 'success --include-ignored', 'SUCCESS'):
            with self.subTest(case=case), self.assertRaises(runner.ImportRejected):
                command(Path('/private/test-binary'), case)

    def test_negative_is_not_success_without_expected_evidence(self):
        observations = self.entry('_import_observations')
        name = ('restore_preflight::target_binding::controlled_import::'
                'live_tests::live_controlled_import_ready_eof')
        output = (f'\nrunning 1 test\ntest {name} ... ok\n\n'
                  'test result: ok. 1 passed; 0 failed; 0 ignored; '
                  '0 measured; 123 filtered out; finished in 0.01s\n').encode()
        for code in (0, 1):
            with self.subTest(code=code), self.assertRaises(runner.ImportRejected):
                observations(output, 'ready-eof', code)

    def test_final_result_requires_stop_and_no_pending(self):
        validate = self.entry('_validate_import_final')
        result = {'case': 'success', 'status':
            'CONTROLLED_FIXTURE_IMPORT_PASSED_SINGLE_HOST_QUARANTINED_NOT_FULL_RESTORE',
            'isolation': {'source_stopped': True, 'target_stopped': True,
                          'volumes_retained': True},
            'source_unchanged': True, 'pending_absent': True}
        validate(result)
        for path in (('isolation', 'source_stopped'), ('isolation', 'target_stopped'),
                     ('isolation', 'volumes_retained'), ('source_unchanged',),
                     ('pending_absent',)):
            changed = copy.deepcopy(result)
            parent = changed
            for key in path[:-1]:
                parent = parent[key]
            parent[path[-1]] = False
            with self.subTest(path=path), self.assertRaises(runner.ImportRejected):
                validate(changed)
        clone = copy.deepcopy(result)
        clone['case'] = 'wrong-endpoint'
        with self.assertRaises(runner.ImportRejected):
            validate(clone)

    def test_commit_unknown_is_never_reported_as_rollback(self):
        observations = self.entry('_import_observations')
        name = ('restore_preflight::target_binding::controlled_import::'
                'live_tests::live_controlled_import_commit_confirmation_lost')
        receipt = {'case': 'commit-unknown', 'failure': 'CommitUnknown',
                   'rollback_verified': True, 'retry_allowed': False,
                   'commit_attempted': True}
        output = (f'\nrunning 1 test\ntest {name} ... '
                  f'KW_C4_IMPORT|{json.dumps(receipt)}\nok\n\n'
                  'test result: ok. 1 passed; 0 failed; 0 ignored; '
                  '0 measured; 123 filtered out; finished in 0.01s\n').encode()
        with self.assertRaisesRegex(runner.ImportRejected, 'Protocol'):
            observations(output, 'commit-unknown', 0)


if __name__ == "__main__":
    unittest.main()
