#!/usr/bin/env python3
"""Render a D bundle and install only its inactive files after separate approval."""

import argparse
import base64
import hashlib
import ipaddress
import json
import os
import re
import stat
import struct
import sys
from contextlib import contextmanager
from pathlib import Path


USER = 'kw-b1-transport'
CONFIG = '/etc/knowweave-c4-b1'
LIBEXEC = '/usr/local/libexec'
GATE = 'knowweave-c4-b1-ssh-gate'
LAUNCHER = 'knowweave-c4-b1-receiver-launcher'
RECEIVER = 'knowweave-c4-b1-receiver'
BINARIES = {GATE: 0o755, LAUNCHER: 0o500, RECEIVER: 0o500}
CONFIG_FILES = {'sshd_config': 0o600, 'authorized_keys': 0o644, 'sudoers.pending': 0o440,
                'receiver-launch.lock': 0o600}
DESTINATIONS = {**{name: (LIBEXEC, mode) for name, mode in BINARIES.items()},
                **{name: (CONFIG, mode) for name, mode in CONFIG_FILES.items()}}
PORT = 22229
HOST_KEY = '/etc/ssh/ssh_host_ed25519_key'
MAX_ELF = 64 * 1024 * 1024
MAX_JSON = 16384
REQUEST_FIELDS = {'schema', 'bind_ipv4', 'source_ipv4', 'uid', 'gid', 'transport_public_key',
                  'host_public_key_sha256', 'parent_identities'}


def need(condition, code):
    if not condition:
        raise ValueError(code)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return (json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True) + '\n').encode()


def parse(raw):
    def unique(pairs):
        result = dict(pairs)
        need(len(result) == len(pairs), 'duplicate JSON member')
        return result

    return json.loads(raw, object_pairs_hook=unique)


def is_digest(value):
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value) is not None


def ipv4(value):
    need(isinstance(value, str), 'IPv4 string required')
    address = ipaddress.IPv4Address(value)
    need(str(address) == value and not (address.is_unspecified or address.is_loopback or address.is_multicast
                                       or address.is_link_local or address.is_reserved), 'specific IPv4 required')
    return str(address)


def public_key_blob(text):
    need(isinstance(text, str) and re.fullmatch(r'ssh-ed25519 [A-Za-z0-9+/]{68}', text) is not None,
         'expected one Ed25519 public key without options or comment')
    raw = base64.b64decode(text.split(' ')[1], validate=True)
    need(len(raw) == 51 and raw.startswith(b'\0\0\0\x0bssh-ed25519\0\0\0 '), 'invalid key wire body')
    return raw


def validate_request(value):
    need(isinstance(value, dict) and value.keys() == REQUEST_FIELDS, 'request field mismatch')
    need(type(value['schema']) is int and value['schema'] == 1, 'unsupported request version')
    bind, source = ipv4(value['bind_ipv4']), ipv4(value['source_ipv4'])
    need(bind != source, 'listen and peer addresses must differ')
    need(all(type(value[key]) is int and 1000 <= value[key] <= 60000 for key in ('uid', 'gid')), 'account range')
    need(is_digest(value['host_public_key_sha256']), 'host key public pin missing')
    parents = value['parent_identities']
    need(isinstance(parents, dict) and parents.keys() == {'/etc', '/usr/local'}, 'missing exact parents')
    for identity in parents.values():
        need(isinstance(identity, list) and len(identity) == 2 and
             all(type(item) is int and item > 0 for item in identity), 'invalid parent identity')
    need(digest(public_key_blob(value['transport_public_key'])) != value['host_public_key_sha256'],
         'host and transport public keys must differ')
    return bind, source


def render(request, launcher_hash):
    bind, source = validate_request(request)
    need(is_digest(launcher_hash), 'launcher digest missing')
    sshd = f'''Port {PORT}
AddressFamily inet
ListenAddress {bind}
PidFile /run/knowweave-c4-b1-sshd.pid
HostKey {HOST_KEY}
HostKeyAlgorithms ssh-ed25519
AllowUsers {USER}@{source}
AuthorizedKeysFile {CONFIG}/authorized_keys
PubkeyAcceptedAlgorithms ssh-ed25519
AuthenticationMethods publickey
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
HostbasedAuthentication no
GSSAPIAuthentication no
PermitRootLogin no
PermitEmptyPasswords no
UsePAM yes
StrictModes yes
ForceCommand knowweave-c4-b1-fixed
DisableForwarding yes
PermitTTY no
PermitTunnel no
PermitUserRC no
PermitUserEnvironment no
PrintMotd no
PrintLastLog no
LoginGraceTime 20
MaxAuthTries 1
MaxSessions 1
MaxStartups 4
LogLevel VERBOSE
'''
    rule = f'''Cmnd_Alias KW_C4_B1_LAUNCH = sha256:{launcher_hash} {LIBEXEC}/{LAUNCHER} ""
Defaults!KW_C4_B1_LAUNCH env_reset,fdexec=always,!setenv,!use_pty,!log_input,!log_output,!log_stdin,!log_stdout,!log_stderr,!log_ttyin,!log_ttyout
{USER} ALL=(root:root) NOPASSWD:NOSETENV:NOLOG_INPUT:NOLOG_OUTPUT: KW_C4_B1_LAUNCH
'''
    return {'sshd_config': sshd.encode('ascii'), 'sudoers.pending': rule.encode('ascii'),
            'authorized_keys': f'restrict,from="{source}/32" {request["transport_public_key"]}\n'.encode('ascii'),
            'receiver-launch.lock': b''}


def read_bounded(stream, cap):
    raw = stream.read(cap + 1)
    need(len(raw) <= cap, 'input size limit')
    return raw


def check_elf(raw):
    need(len(raw) >= 64 and raw[:7] == b'\x7fELF\x02\x01\x01', 'expected Linux ELF64 input')
    kind, machine = struct.unpack_from('<HH', raw, 16)
    need(kind in {2, 3} and machine == 62, 'expected amd64 executable')


def make_manifest(request, contents):
    return {'schema': 1, 'scope': 'H4_INACTIVE_D_FILES_ONLY', 'request': request, 'files': {
        name: {'destination': parent + '/' + name, 'owner': [0, 0], 'mode': f'{mode:04o}',
               'bytes': len(contents[name]), 'sha256': digest(contents[name])}
        for name, (parent, mode) in DESTINATIONS.items()}}


def prepare(request_path, binaries, output):
    with request_path.open('rb') as stream:
        request = parse(read_bounded(stream, MAX_JSON))
    validate_request(request)
    contents = {}
    for name in BINARIES:
        with (binaries / name).open('rb') as stream:
            contents[name] = read_bounded(stream, MAX_ELF)
        check_elf(contents[name])
    contents.update(render(request, digest(contents[LAUNCHER])))
    manifest = canonical(make_manifest(request, contents))
    output.mkdir(mode=0o700)
    for name, raw in {**contents, 'manifest.json': manifest}.items():
        with (output / name).open('xb') as stream:
            stream.write(raw)
    return {'status': 'H4_BUNDLE_PREPARED_NOT_INSTALLED', 'manifest_sha256': digest(manifest)}


def stamp(item):
    return (item.st_dev, item.st_ino, item.st_mode, item.st_uid, item.st_gid,
            item.st_nlink, item.st_size, item.st_mtime_ns, item.st_ctime_ns)


@contextmanager
def trusted_dir(path):
    need(path.is_absolute() and '..' not in path.parts, 'absolute trusted directory required')
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW
    fd = os.open('/', flags)
    try:
        for name in ('.', *path.parts[1:]):
            next_fd = os.open(name, flags, dir_fd=fd)
            os.close(fd)
            fd = next_fd
            item = os.fstat(fd)
            need(item.st_uid == item.st_gid == 0 and item.st_mode & 0o022 == 0, 'untrusted directory')
        yield fd
    finally:
        os.close(fd)


def read_leaf(parent, name, cap, mode=None):
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK
    with os.fdopen(os.open(name, flags, dir_fd=parent), 'rb') as stream:
        before = os.fstat(stream.fileno())
        need(stat.S_ISREG(before.st_mode) and before.st_nlink == 1 and before.st_uid == before.st_gid == 0 and
             before.st_mode & 0o022 == 0 and (mode is None or stat.S_IMODE(before.st_mode) == mode) and
             before.st_size <= cap, 'untrusted file')
        raw = read_bounded(stream, cap)
        after = os.fstat(stream.fileno())
        named = os.stat(name, dir_fd=parent, follow_symlinks=False)
        need(len(raw) == before.st_size and stamp(before) == stamp(after) == stamp(named), 'file changed')
        return raw


def load_bundle(directory, expected):
    need(is_digest(expected), 'exact approved manifest SHA-256 required')
    with trusted_dir(directory) as fd:
        need(stat.S_IMODE(os.fstat(fd).st_mode) == 0o700, 'bundle directory must be private')
        need(set(os.listdir(fd)) == set(DESTINATIONS) | {'manifest.json'}, 'unexpected bundle file')
        raw = read_leaf(fd, 'manifest.json', MAX_JSON)
        need(digest(raw) == expected, 'manifest digest mismatch')
        manifest = parse(raw)
        request = manifest['request']
        validate_request(request)
        contents = {name: read_leaf(fd, name, MAX_ELF if name in BINARIES else MAX_JSON) for name in DESTINATIONS}
    for name in BINARIES:
        check_elf(contents[name])
    need(all(contents[name] == value for name, value in render(request, digest(contents[LAUNCHER])).items()),
         'configuration differs from fixed renderer')
    need(canonical(make_manifest(request, contents)) == raw, 'manifest does not bind exact bundle')
    return request, contents


def require_absent(parent, name):
    try:
        os.stat(name, dir_fd=parent, follow_symlinks=False)
    except FileNotFoundError:
        return
    raise ValueError('destination already exists')


def install_leaf(parent, name, mode, raw):
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW
    with os.fdopen(os.open(name, flags, 0o600, dir_fd=parent), 'wb') as stream:
        stream.write(raw)
        stream.flush()
        os.fchmod(stream.fileno(), mode)
        os.fsync(stream.fileno())
    os.fsync(parent)
    need(read_leaf(parent, name, len(raw), mode) == raw, 'readback mismatch')


def verify_host_key_metadata(expected):
    key = Path(HOST_KEY)
    with trusted_dir(key.parent) as fd:
        metadata = os.stat(key.name, dir_fd=fd, follow_symlinks=False)
        need(stat.S_ISREG(metadata.st_mode) and stat.S_IMODE(metadata.st_mode) == 0o600 and
             metadata.st_uid == metadata.st_gid == 0 and metadata.st_nlink == 1, 'untrusted host key metadata')
        fields = read_leaf(fd, key.name + '.pub', 1024).decode('ascii').split(maxsplit=2)
        need(len(fields) >= 2 and digest(public_key_blob(' '.join(fields[:2]))) == expected, 'host key public pin drift')


def stage(directory, expected):
    need(sys.platform == 'linux' and os.getuid() == os.geteuid() == os.getgid() == os.getegid() == 0,
         'staging requires Linux root in the approved environment')
    request, contents = load_bundle(directory, expected)
    verify_host_key_metadata(request['host_public_key_sha256'])
    with trusted_dir(Path('/etc')) as etc, trusted_dir(Path('/usr/local')) as local:
        for name, fd in (('/etc', etc), ('/usr/local', local)):
            item = os.fstat(fd)
            need([item.st_dev, item.st_ino] == request['parent_identities'][name], 'approved parent identity drift')
        require_absent(etc, Path(CONFIG).name)
        require_absent(local, Path(LIBEXEC).name)
        os.umask(0o022)
        os.mkdir(Path(CONFIG).name, 0o755, dir_fd=etc)
        os.fsync(etc)
        os.mkdir(Path(LIBEXEC).name, 0o755, dir_fd=local)
        os.fsync(local)
        for parent, name, files in ((etc, Path(CONFIG).name, CONFIG_FILES),
                                    (local, Path(LIBEXEC).name, BINARIES)):
            fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=parent)
            try:
                item = os.fstat(fd)
                need(item.st_uid == item.st_gid == 0 and stat.S_IMODE(item.st_mode) == 0o755 and
                     not os.listdir(fd), 'new directory identity or contents')
                for leaf, mode in files.items():
                    install_leaf(fd, leaf, mode, contents[leaf])
            finally:
                os.close(fd)
    return {'status': 'H4_INACTIVE_FILES_STAGED_READBACK_ONLY', 'manifest_sha256': expected}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='action', required=True)
    render_command = commands.add_parser('prepare')
    render_command.add_argument('--request', type=Path, required=True)
    render_command.add_argument('--binaries', type=Path, required=True)
    render_command.add_argument('--output', type=Path, required=True)
    stage_command = commands.add_parser('stage-inactive')
    stage_command.add_argument('--bundle', type=Path, required=True)
    stage_command.add_argument('--expected-manifest-sha256', required=True)
    args = parser.parse_args()
    try:
        if args.action == 'prepare':
            result = prepare(args.request, args.binaries, args.output)
        else:
            result = stage(args.bundle, args.expected_manifest_sha256)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({'status': 'H4_HOLD_NO_AUTOMATIC_REPLAY', 'failure_type': type(error).__name__}))
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
