"""Tooling-only tests. Filesystem integration requires a fresh user namespace."""

import base64
import copy
import os
import stat
import struct
import tempfile
import unittest
from pathlib import Path

import install


def public_key(byte):
    raw = b'\0\0\0\x0bssh-ed25519\0\0\0 ' + bytes([byte]) * 32
    return 'ssh-ed25519 ' + base64.b64encode(raw).decode(), install.digest(raw)


def request():
    return {'schema': 1, 'bind_ipv4': '192.0.2.9', 'source_ipv4': '198.51.100.7', 'uid': 2401, 'gid': 2401,
            'host_public_key_sha256': public_key(1)[1], 'transport_public_key': public_key(2)[0],
            'parent_identities': {'/etc': [1, 2], '/usr/local': [1, 3]}}


def header_fixture():
    raw = bytearray(64)
    raw[:7] = b'\x7fELF\x02\x01\x01'
    struct.pack_into('<HH', raw, 16, 2, 62)
    return bytes(raw)


class Render(unittest.TestCase):
    def test_exact_sudo_and_ssh_allowlists(self):
        content = install.render(request(), 'a' * 64)
        self.assertEqual(set(content), set(install.CONFIG_FILES))
        config = content['sshd_config'].decode().splitlines()
        self.assertIn('ListenAddress 192.0.2.9', config)
        self.assertIn('AllowUsers kw-b1-transport@198.51.100.7', config)
        for line in ('AuthenticationMethods publickey', 'DisableForwarding yes', 'PermitRootLogin no',
                     'PermitTTY no', 'PermitUserEnvironment no', 'ForceCommand knowweave-c4-b1-fixed'):
            self.assertIn(line, config)
        self.assertTrue(all(not line.startswith(('Include ', 'Subsystem ', 'AcceptEnv ')) for line in config))
        self.assertTrue(content['authorized_keys'].startswith(b'restrict,from="198.51.100.7/32" ssh-ed25519 '))
        sudo = content['sudoers.pending'].decode().splitlines()
        self.assertEqual(sudo[0], 'Cmnd_Alias KW_C4_B1_LAUNCH = sha256:' + 'a' * 64 +
                         ' /usr/local/libexec/knowweave-c4-b1-receiver-launcher ""')
        self.assertIn('fdexec=always,!setenv,!use_pty,!log_input,!log_output', sudo[1])
        self.assertEqual(sudo[2], 'kw-b1-transport ALL=(root:root) NOPASSWD:NOSETENV:NOLOG_INPUT:NOLOG_OUTPUT: KW_C4_B1_LAUNCH')

    def test_request_rejects_injection_and_missing_values(self):
        for key, value in (('uid', True), ('gid', 0), ('bind_ipv4', '0.0.0.0'), ('source_ipv4', '198.51.100.7/24'),
                           ('bind_ipv4', '192.0.2.9\nPermitRootLogin yes'), ('host_public_key_sha256', None),
                           ('transport_public_key', 'command="sh" ' + public_key(2)[0]),
                           ('transport_public_key', public_key(2)[0] + '\n' + public_key(3)[0]),
                           ('transport_public_key', public_key(1)[0]), ('parent_identities', {'/etc': [1, 2]})):
            with self.subTest(key=key, value=value):
                changed = {**request(), key: value}
                with self.assertRaises(ValueError):
                    install.render(changed, 'a' * 64)
        with self.assertRaises(ValueError):
            install.parse(b'{"uid":2401,"uid":0}')

    def test_reproducible_exact_bundle_and_no_overwrite(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'inputs').mkdir()
            for name in install.BINARIES:
                (root / 'inputs' / name).write_bytes(header_fixture())
            (root / 'request').write_bytes(install.canonical(request()))
            first = install.prepare(root / 'request', root / 'inputs', root / 'one')
            self.assertEqual(first, install.prepare(root / 'request', root / 'inputs', root / 'two'))
            with self.assertRaises(FileExistsError):
                install.prepare(root / 'request', root / 'inputs', root / 'one')
            (root / 'inputs' / install.GATE).write_bytes(b'shell script')
            with self.assertRaises(ValueError):
                install.prepare(root / 'request', root / 'inputs', root / 'wrong')


class IsolatedStage(unittest.TestCase):
    @unittest.skipUnless(os.name == 'posix' and os.getuid() == 0 and hasattr(os, 'fork'), 'requires user namespace')
    def test_chroot_stage_and_readback(self):
        mapping = Path('/proc/self/uid_map').read_text().split()
        self.assertEqual(len(mapping), 3)
        self.assertEqual(mapping[0], '0')
        self.assertNotEqual(mapping[1], '0')  # refuse real root
        self.assertEqual(mapping[2], '1')
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for name in ('etc/ssh', 'usr/local', 'bundle'):
                (root / name).mkdir(parents=True)
            (root / 'bundle').chmod(0o700)
            key = root / install.HOST_KEY.lstrip('/')
            key.write_bytes(b'unread fixture sentinel')
            key.chmod(0o600)
            Path(str(key) + '.pub').write_text(public_key(1)[0] + ' fixture\n')
            spec = copy.deepcopy(request())
            for name in spec['parent_identities']:
                item = (root / name.lstrip('/')).stat()
                spec['parent_identities'][name] = [item.st_dev, item.st_ino]
            contents = {name: header_fixture() for name in install.BINARIES}
            contents.update(install.render(spec, install.digest(contents[install.LAUNCHER])))
            manifest = install.canonical(install.make_manifest(spec, contents))
            for name, raw in {**contents, 'manifest.json': manifest}.items():
                (root / 'bundle' / name).write_bytes(raw)
            child = os.fork()
            if child == 0:
                try:
                    os.chroot(root)
                    os.chdir('/')
                    self._exercise_stage(manifest, contents)
                except BaseException:
                    import traceback
                    traceback.print_exc()
                    os._exit(1)
                os._exit(0)
            _, status = os.waitpid(child, 0)
            self.assertEqual(os.waitstatus_to_exitcode(status), 0)

    def _exercise_stage(self, manifest, contents):
        bundle = Path('/bundle')
        expected = install.digest(manifest)
        config = Path(install.CONFIG)
        libexec = Path(install.LIBEXEC)
        with self.assertRaises(ValueError):
            install.stage(bundle, 'a' * 64)
        self.assertFalse(config.exists() or libexec.exists())
        bad = bundle / 'sudoers.pending'
        bad.write_bytes(b'injected')
        with self.assertRaises(ValueError):
            install.stage(bundle, expected)
        self.assertFalse(config.exists() or libexec.exists())
        bad.write_bytes(contents[bad.name])
        libexec.symlink_to('/etc')
        with self.assertRaises(ValueError):
            install.stage(bundle, expected)
        self.assertFalse(config.exists())
        libexec.unlink()
        result = install.stage(bundle, expected)
        self.assertEqual(result['status'], 'H4_INACTIVE_FILES_STAGED_READBACK_ONLY')
        for name, (parent, mode) in install.DESTINATIONS.items():
            path = Path(parent) / name
            self.assertEqual(path.read_bytes(), contents[name])
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), mode)
        self.assertFalse(Path('/etc/sudoers.d/knowweave-c4-b1').exists())
        with self.assertRaises(ValueError):
            install.stage(bundle, expected)


if __name__ == '__main__':
    unittest.main()
