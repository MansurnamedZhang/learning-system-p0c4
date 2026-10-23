"""Portable full-owner framing tests; no Docker, PG or native authority credit."""
import copy
from contextlib import ExitStack, contextmanager
import hashlib
import json
import os
from pathlib import Path
import queue
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import p0c4_source_lifecycle_gate_acceptance as driver
import p0c4_restore_target as provisioner
from p0c4_completion import full_import, full_target_fs as fs

SESSION = '11111111-1111-4111-8111-111111111111'
TARGET = '22222222-2222-4222-8222-222222222222'
sha = lambda raw: hashlib.sha256(raw).hexdigest()


def observations():
    identity = provisioner.identity_for(TARGET)
    cid, nid, image = 'a' * 64, 'b' * 64, 'sha256:' + 'c' * 64
    mount = '/var/lib/docker/volumes/' + identity['volume'] + '/_data'
    before = dict(daemon_id='one-daemon', containers=[], networks=[], volumes=[], routes=[])
    after = dict(daemon_id=before['daemon_id'], routes=[],
                 containers=[dict(Id=cid, Name='/' + identity['project'] + '-pg-1', Image=image,
                                  Config=dict(Image=identity['image'], Labels={
                                      'com.docker.compose.project': identity['project'],
                                      'com.docker.compose.service': 'pg'}),
                                  State=dict(Running=True, Health=dict(Status='healthy')),
                                  HostConfig=dict(NetworkMode=identity['network'], PortBindings={}),
                                  NetworkSettings=dict(Networks={identity['network']: dict(NetworkID=nid)}, Ports={}),
                                  Mounts=[dict(Destination='/var/lib/postgresql', Type='volume',
                                               Name=identity['volume'], Source=mount, RW=True)])],
                 networks=[dict(Id=nid, Name=identity['network'], Internal=True,
                                Labels={'com.docker.compose.project': identity['project']},
                                IPAM=dict(Config=[dict(Subnet='172.30.241.0/24')]))],
                 volumes=[dict(Name=identity['volume'], Mountpoint=mount,
                               Labels={'com.docker.compose.project': identity['project']})],
                 images=[dict(Id=image, RepoDigests=[identity['image']])])
    ids = provisioner.verify_created(identity, '172.30.241.0/24', before, after)
    return before, after, ids


class LocalTransport:
    """Only transport and native FS prerequisites are substituted, not parsing."""
    def __init__(self, protocol):
        self.protocol, self.sent = protocol, []
        self._full_profile = fs._HelperProfile(fs._TOKEN, 'owner', [], ['python3', 'fixed'], True)
        self.child = SimpleNamespace(p=SimpleNamespace(poll=lambda: None), events=queue.Queue(),
                                     check=lambda: None, send=self.send)

    def send(self, raw):
        self.sent.append(raw)
        reply = self.protocol.handle(json.loads(raw, object_pairs_hook=driver.unique_pairs))
        self.child.events.put((0, driver.canonical(reply) + b'\n'))

    def persistent(self, logs, timeout):
        return self.child


@contextmanager
def held_owner(target_id=TARGET, subnet='172.30.241.0/24', controlled=False):
    # These portable substitutions do not test root ownership, flock or no-follow.
    # The real files/created backend, canonical framing, protocol and verifier run.
    with TemporaryDirectory(prefix='kwc4-cached-before-') as directory, ExitStack() as stack:
        root = Path(directory); (root / 'targets').mkdir()
        native = fs._new_controlled_owner() if controlled else fs._NativeOwner()
        native.config = dict(root=str(root), target_case_id=target_id, subnet=subnet,
                             case_root=str(root / 'case'))
        if controlled:
            native.config['role'] = 'target'
        held = object(); native.lock = held; native.root_fd = 123
        rechecks = []
        def recheck():
            if native.lock is not held or native.root_fd != 123:
                raise fs.FsRejected('FS_POLICY')
            rechecks.append(held)
        def private_write(path, raw):
            with path.open('xb') as output:
                output.write(raw)
        stack.enter_context(patch.object(native, '_recheck', side_effect=recheck))
        stack.enter_context(patch.object(provisioner, '_sync_directory', return_value=None))
        stack.enter_context(patch.object(provisioner, '_private_write', side_effect=private_write))
        stack.enter_context(patch.object(fs.os, 'fchown', return_value=None, create=True))
        for key in ('O_NOFOLLOW', 'O_CLOEXEC'):
            if not hasattr(fs.os, key):
                stack.enter_context(patch.object(fs.os, key, 0, create=True))
        protocol = fs._Protocol(fs._TOKEN, SESSION, native)
        protocol._seq = 2; protocol._state = 'pin'
        transport = LocalTransport(protocol)
        broker = driver.BrokerClient(transport, None, driver.CaseBudget())
        owner = fs._OwnerClient(fs._TOKEN, transport, broker)
        owner._nonce = SESSION; owner._seq = 2; owner._state = 'pin'
        known = []
        context = SimpleNamespace(_fs_owner=owner, _target_owner=object(), _target_created_ids=None,
                                  _runner=SimpleNamespace(ownership=SimpleNamespace(
                                      known=lambda held_owner, cid: known.append((held_owner, cid)))))
        yield SimpleNamespace(native=native, protocol=protocol, transport=transport, broker=broker,
                              owner=owner, context=context, known=known, rechecks=rechecks, held=held)


class CachedBeforeTests(unittest.TestCase):
    def deliver(self, before, after, ids):
        target_id = ids['volume_name'][len('learning-system-p0c4-restore-'):-len('_pg')]
        # The incident has unrelated networks too; use the exact owned network.
        identity = provisioner.identity_for(target_id)
        subnet = next(row for row in after['networks'] if row['Name'] == identity['network'])['IPAM']['Config'][0]['Subnet']
        with held_owner(target_id, subnet) as held:
            full_import.FullRehearsalContext._create_target_files(held.context, before)
            full_import.FullRehearsalContext._target_created(held.context, before, after, ids)
            request = json.loads(held.transport.sent[-1])
            self.assertEqual(set(request['value']), {'before_sha256', 'after', 'ids'})
            self.assertEqual(request['value']['before_sha256'], sha(driver.canonical(before)))
            self.assertEqual(sha(driver.canonical(request['value']['after'])), sha(driver.canonical(after)))
            self.assertTrue(all(len(raw) <= 1024**2 for raw in held.transport.sent))
            self.assertEqual(held.native._creation_before, (driver.canonical(before), sha(driver.canonical(before))))
            self.assertEqual(held.native.status['container_id'], ids['container_id'])
            self.assertEqual(held.owner._state, 'created')
            self.assertEqual(held.known, [(held.context._target_owner, ids['container_id'])])
            self.assertEqual(held.rechecks, [held.held, held.held])

    def test_full_producer_broker_protocol_and_backend_use_complete_cached_before(self):
        self.deliver(*observations())

    def test_cached_canonical_before_has_no_mutable_input_alias(self):
        before, after, ids = observations(); original = driver.canonical(before)
        with held_owner() as held:
            full_import.FullRehearsalContext._create_target_files(held.context, before)
            before['daemon_id'] = 'changed-after-files'
            held.owner.call('created', dict(before_sha256=sha(original), after=after, ids=ids))
            self.assertEqual(held.native._creation_before, (original, sha(original)))
            self.assertEqual(held.native.status['container_id'], ids['container_id'])

    def test_producer_changed_before_cannot_rebind_the_files_snapshot(self):
        before, after, ids = observations(); original = driver.canonical(before)
        with held_owner() as held:
            full_import.FullRehearsalContext._create_target_files(held.context, before)
            before['routes'].append('10.99.0.0/24')
            with self.assertRaises(fs.FsRejected):
                full_import.FullRehearsalContext._target_created(held.context, before, after, ids)
            self.assertEqual(held.native._creation_before, (original, sha(original)))
            self.assertIsNone(held.native.status)

    def test_controlled_owner_keeps_complete_before_shape_and_no_cache(self):
        for digest_shape in (False, True):
            with self.subTest(digest_shape=digest_shape), held_owner(controlled=True) as held:
                before, after, ids = observations()
                held.owner.call('files', dict(before=before))
                self.assertIsNone(held.native._creation_before)
                value = dict(before=before, after=after, ids=ids)
                if digest_shape:
                    value.pop('before'); value['before_sha256'] = sha(driver.canonical(before))
                    with self.assertRaises(fs.FsRejected): held.owner.call('created', value)
                    self.assertIsNone(held.native.status)
                else:
                    held.owner.call('created', value)
                    self.assertEqual(held.native.status['container_id'], ids['container_id'])

    def test_wrong_digest_legacy_shape_after_tamper_and_id_tamper_reject(self):
        for mutation in ('digest', 'legacy', 'after', 'ids', 'cache', 'held'):
            with self.subTest(mutation=mutation), held_owner() as held:
                before, after, ids = observations()
                full_import.FullRehearsalContext._create_target_files(held.context, before)
                value = dict(before_sha256=sha(driver.canonical(before)), after=copy.deepcopy(after), ids=dict(ids))
                if mutation == 'digest': value['before_sha256'] = '0' * 64
                elif mutation == 'legacy': value = dict(before=before, after=after, ids=ids)
                elif mutation == 'after': value['after']['daemon_id'] = 'different'
                elif mutation == 'ids': value['ids']['container_id'] = 'f' * 64
                elif mutation == 'cache': held.native._creation_before = (b'{}', value['before_sha256'])
                else: held.native.lock = None
                with self.assertRaises(fs.FsRejected): held.owner.call('created', value)
                self.assertIsNone(held.native.status)
                self.assertTrue(held.protocol._failed)

    def test_missing_files_wrong_session_and_reordered_sequence_reject(self):
        for mutation in ('missing-files', 'session', 'seq', 'repeat-files'):
            with self.subTest(mutation=mutation), held_owner() as held:
                before, after, ids = observations()
                if mutation != 'missing-files':
                    full_import.FullRehearsalContext._create_target_files(held.context, before)
                request = dict(session=SESSION, seq=3, op='created', value=dict(
                    before_sha256=sha(driver.canonical(before)), after=after, ids=ids))
                if mutation == 'session': request['session'] = TARGET
                elif mutation == 'seq': request['seq'] = 4
                elif mutation == 'repeat-files': request.update(op='files', value=dict(before=before))
                reply = held.protocol.handle(request)
                self.assertFalse(reply['ok'])
                self.assertIsNone(held.native.status)
                self.assertTrue(held.protocol._failed)


@unittest.skipUnless(os.environ.get('P0C4_FULL_CREATED_FRAME_PATH'), 'private original frame is opt-in')
class ReconstructedNormalFrameTests(unittest.TestCase):
    def test_actual_over_limit_frame_uses_same_complete_observations_under_original_cap(self):
        raw = Path(os.environ['P0C4_FULL_CREATED_FRAME_PATH']).read_bytes()
        self.assertEqual(len(raw), 1053020)
        self.assertEqual(sha(raw), '10e433a5e9d4a77a875c76f1a144d9ef79289d32131b85c2db02e2a298883323')
        request = json.loads(raw)
        self.assertEqual(driver.canonical(request), raw)
        CachedBeforeTests.deliver(self, **request['value'])


if __name__ == '__main__':
    unittest.main()
