"""Fresh ordinary-hans RED/GREEN admission evidence; no backup/restore/root trust."""
import argparse
import hashlib
import io
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import selectors
import signal
import stat
import subprocess
import sys
import time
import uuid
import zipfile

BASE = Path('/home/hans/knowweave-c4-source-admission')
ENTRY = 'scripts/p0c4_source_admission_gate_acceptance.py'
MANIFEST = '_knowweave_source_manifest.json'
PG_IMAGE = 'postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d'
BUILDER = 'sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b'
HEX64 = re.compile(r'[0-9a-f]{64}\Z')
TESTS = tuple('source::admission_tests::' + n for n in (
    'busy_source_recovery_preserves_acl_and_journal',
    'same_database_attempts_share_admission_and_other_database_is_independent',
    'single_connection_catalog_and_drain_share_admitted_backend',
    'dropping_admission_closes_backend_without_reopening_gate',
    'release_and_compensation_keep_admission_until_owner_closes'))
HOST_ENV = {'PATH': '/usr/bin:/bin', 'HOME': '/home/hans', 'LC_ALL': 'C', 'DOCKER_HOST': 'unix:///var/run/docker.sock'}
DOCKER = '/usr/bin/docker'
LIMIT = 2 * 1024 ** 2


class GateError(RuntimeError):
    pass


def require(condition, reason):
    if not condition:
        raise GateError(reason)


def fail_result(result, reason):
    result.setdefault('primary_reason', result.get('reason', reason))
    result['status'], result['reason'] = 'FAILED', reason


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode('utf-8')


def unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, 'JSON_DUPLICATE_KEY')
        result[key] = value
    return result


def verify_package(raw, archive_sha, manifest_sha, runner_sha, mode):
    require(all(isinstance(v, str) and HEX64.fullmatch(v) for v in (archive_sha, manifest_sha, runner_sha)), 'INPUT_PINS')
    require(len(raw) <= 32 * 1024 ** 2 and digest(raw) == archive_sha, 'ARCHIVE_DIGEST')
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        infos = archive.infolist()
        names = [i.filename for i in infos]
        require(len(names) <= 2049 and len(set(names)) == len(names) and sum(i.file_size for i in infos) <= 32 * 1024 ** 2, 'ARCHIVE_BUDGET_OR_DUPLICATE')
        for item in infos:
            parts = item.filename.split('/')
            blocked = {'.git', '.codex', '.agents', '.aws', '.superpowers', 'target', 'node_modules', '__pycache__', 'evidence', 'secrets', 'private'}
            require(not item.is_dir() and not item.flag_bits & 1 and '\\' not in item.filename and ':' not in item.filename and all(p and p not in ('.', '..') for p in parts), 'ZIP_PATH')
            require(not any(p in blocked or p == '.env' or p.startswith('.env.') or p.endswith(('.pem', '.key', '.pyc')) for p in parts), 'ZIP_PRIVATE_PATH')
            require((item.external_attr >> 16) & 0o170000 == stat.S_IFREG, 'ZIP_SPECIAL_FILE')
        require(MANIFEST in names, 'MANIFEST_MISSING')
        manifest_raw = archive.read(MANIFEST)
        require(digest(manifest_raw) == manifest_sha, 'MANIFEST_DIGEST')
        manifest = json.loads(manifest_raw, object_pairs_hook=unique_pairs)
        require(type(manifest) is dict and set(manifest) == {'schema', 'base_commit', 'snapshot_kind', 'files'} and type(manifest['schema']) is int and manifest['schema'] == 1, 'MANIFEST_SCHEMA')
        require(isinstance(manifest['base_commit'], str) and re.fullmatch('[0-9a-f]{40}', manifest['base_commit']) and manifest['snapshot_kind'] == 'working-tree-' + mode and canonical(manifest) == manifest_raw, 'MANIFEST_CANONICAL_IDENTITY')
        records = manifest['files']
        require(type(records) is list and len(records) <= 2048, 'MANIFEST_RECORDS')
        for row in records:
            require(type(row) is dict and set(row) == {'path', 'bytes', 'sha256'} and type(row['path']) is str and type(row['bytes']) is int and row['bytes'] >= 0 and type(row['sha256']) is str and HEX64.fullmatch(row['sha256']), 'MANIFEST_RECORD')
        require([r['path'] for r in records] == sorted(set(names) - {MANIFEST}), 'MANIFEST_INVENTORY')
        required = {'Cargo.toml', 'Cargo.lock', 'crates/learning-backup/src/source.rs', 'crates/learning-backup/src/source/admission_tests.rs', ENTRY, 'scripts/test_p0c4_source_admission_gate_acceptance.py'}
        require(required <= set(names), 'SOURCE_REQUIRED')
        files = {r['path']: archive.read(r['path']) for r in records}
        require(all(len(files[r['path']]) == r['bytes'] and digest(files[r['path']]) == r['sha256'] for r in records), 'SOURCE_FILE_DIGEST')
        require(digest(files[ENTRY]) == runner_sha, 'PACKAGED_RUNNER_DIGEST')
        return manifest, files


def mkdir_new(path):
    path.mkdir(mode=0o700)
    if os.name != 'nt':
        os.chmod(path, 0o700)


def write_new(path, raw, mode=0o600):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, 'O_NOFOLLOW', 0), 0o600)
    with os.fdopen(fd, 'wb') as handle:
        handle.write(raw)
        if os.name != 'nt':
            os.fchmod(handle.fileno(), mode)
        handle.flush()
        os.fsync(handle.fileno())


def owned_file(path, modes, max_bytes=32 * 1024 ** 2):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, 'rb') as handle:
        meta = os.fstat(handle.fileno())
        require(stat.S_ISREG(meta.st_mode) and meta.st_nlink == 1 and meta.st_uid == os.geteuid() and stat.S_IMODE(meta.st_mode) in modes and meta.st_size <= max_bytes, 'PRIVATE_INPUT_IDENTITY')
        raw = handle.read(max_bytes + 1)
        require(len(raw) <= max_bytes, 'INPUT_SIZE')
        return raw


def extract_source(source, files):
    mkdir_new(source)
    for name, raw in files.items():
        path = source / name
        for parent in reversed(path.parents):
            if parent == source or source in parent.parents:
                if not parent.exists():
                    mkdir_new(parent)
        write_new(path, raw, 0o400)


def source_digest(source, manifest, enforce_mode=True):
    observed = {}
    root = source.lstat()
    require(stat.S_ISDIR(root.st_mode) and (not enforce_mode or (root.st_uid == os.geteuid() and stat.S_IMODE(root.st_mode) == 0o700)), 'SOURCE_ROOT')
    for path in source.rglob('*'):
        meta = path.lstat()
        require(stat.S_ISREG(meta.st_mode) or stat.S_ISDIR(meta.st_mode), 'SOURCE_SPECIAL_FILE')
        if enforce_mode:
            require(meta.st_uid == os.geteuid() and stat.S_IMODE(meta.st_mode) == (0o700 if path.is_dir() else 0o400), 'SOURCE_MODE')
        if stat.S_ISREG(meta.st_mode):
            require(meta.st_nlink == 1, 'SOURCE_HARDLINK')
            raw = path.read_bytes()
            observed[path.relative_to(source).as_posix()] = {'bytes': len(raw), 'sha256': digest(raw)}
    require(observed == {r['path']: {'bytes': r['bytes'], 'sha256': r['sha256']} for r in manifest['files']}, 'SOURCE_CHANGED')
    return digest(canonical(observed))


def admit_subnets(requested, occupied):
    try:
        chosen = [ipaddress.ip_network(v, strict=True) for v in requested]
        private = [ipaddress.ip_network(v) for v in ('10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16')]
        require(all(str(n) == v and n.version == 4 and n.prefixlen == 24 and any(n.subnet_of(p) for p in private) for n, v in zip(chosen, requested)), 'SUBNET_CANONICAL_PRIVATE')
        for index, network in enumerate(chosen):
            require(not any(network.overlaps(n) for n in chosen[:index]), 'SUBNET_REPEAT')
            for raw in occupied:
                other = ipaddress.ip_network(raw, strict=False)
                require(other.version != 4 or not network.overlaps(other), 'SUBNET_OCCUPIED')
        return [str(n) for n in chosen]
    except ValueError:
        raise GateError('SUBNET_INVALID') from None


def parse_test(code, output, name, mode):
    lines = [s.strip() for s in output.splitlines() if s.strip()]
    tests = [s for s in lines if s.startswith('test ') and not s.startswith('test result:')]
    summaries = [s for s in lines if s.startswith('test result:')]
    green = mode == 'green'
    require(code == (0 if green else 101) and lines.count('running 1 test') == 1 and tests == [f'test {name} ... ' + ('ok' if green else 'FAILED')], 'EXACT_TEST_EXIT')
    prefix = 'ok. 1 passed; 0 failed' if green else 'FAILED. 0 passed; 1 failed'
    require(len(summaries) == 1 and re.fullmatch(r'test result: ' + re.escape(prefix) + r'; 0 ignored; 0 measured; \d+ filtered out; finished in [0-9.]+s', summaries[0]), 'EXACT_TEST_COUNTS')
    require((green and len(lines) == 3) or (not green and name == TESTS[0] and 'busy source recovery must preserve runtime CONNECT ACL' in output and f'---- {name} stdout ----' in lines and lines.count(name) == 1), 'ACL_RED_EVIDENCE')
    return {'test': name, 'exit_code': code, 'passed': int(green), 'failed': int(not green), 'ignored': 0, 'stdout_sha256': digest(output.encode())}


def env_dict(values):
    result = {}
    for raw in values:
        key, separator, value = raw.partition('=')
        require(separator and key not in result, 'ENV_SHAPE')
        result[key] = value
    return result


def validate_pg_processes(output):
    rows = [line.split() for line in output.splitlines() if line.strip()]
    require(len(rows) > 1 and rows[0] == ['UID', 'PID', 'COMMAND'], 'PG_PROCESS_HEADER')
    require(all(len(row) == 3 and row[0] == '999' and re.fullmatch(r'[1-9][0-9]*', row[1]) and row[2] == 'postgres' for row in rows[1:]), 'PG_PROCESS_UID_PID')


def validate_container(facts, expected):
    c, h = facts['Config'], facts['HostConfig']
    require(facts['Id'] == expected['id'] and HEX64.fullmatch(facts['Id']) and facts['Name'] == '/' + expected['name'] and facts['Image'] == expected['image'] and c['Image'] == expected.get('image_ref', expected['image']), 'CONTAINER_IDENTITY')
    require(c['Labels'] == expected['labels'] and c['Cmd'] == expected['cmd'] and c['Entrypoint'] == expected['entrypoint'] and env_dict(c['Env']) == expected['env'], 'CONTAINER_CONFIG')
    builder = expected['builder']
    require(h['Privileged'] is False and not h.get('CapAdd') and h.get('CapDrop') in ((['ALL'], ['CAP_ALL']) if builder else (None, [])) and h['SecurityOpt'] in (['no-new-privileges'], ['no-new-privileges:true']), 'CONTAINER_SECURITY')
    require(h['NanoCpus'] == (4 if builder else 2) * 10 ** 9 and h['Memory'] == (8 if builder else 4) * 1024 ** 3 and h['MemorySwap'] == h['Memory'], 'CONTAINER_LIMITS')
    require(h.get('PidMode', '') == '' and h['IpcMode'] == 'private' and not any(h.get(k) for k in ('Devices', 'DeviceRequests', 'VolumesFrom', 'Links', 'ExtraHosts', 'AutoRemove', 'PortBindings')), 'CONTAINER_HOST_ACCESS')
    binds = h.get('Binds')
    require(binds is None or type(binds) is list, 'CONTAINER_BINDS_SHAPE')
    if binds:
        require(not builder and all(type(b) is str for b in binds), 'CONTAINER_BINDS_FORBIDDEN')
        parsed = [b.split(':') for b in binds]
        require(all(len(b) == 3 and b[2] == 'ro' for b in parsed), 'PG_BINDS_MODE')
        declared = {(source, dest) for kind, source, dest, rw in expected['mounts'] if kind == 'bind' and rw is False}
        require({(b[0], b[1]) for b in parsed} == declared and len(binds) == len(declared), 'PG_BINDS_IDENTITY')
    mounts = {(m['Type'], m['Source'], m['Destination'], m['RW']) for m in facts['Mounts']}
    require(mounts == expected['mounts'] and len(facts['Mounts']) == len(mounts), 'CONTAINER_MOUNTS')
    require(h['NetworkMode'] == expected['network'] and not any((facts['NetworkSettings'].get('Ports') or {}).values()) and not h.get('PublishAllPorts'), 'CONTAINER_NETWORK')
    if builder:
        require(c['User'] == expected['user'] and c['WorkingDir'] == '/reviewed' and h['ReadonlyRootfs'] is True and h['PidsLimit'] == 512 and h['Tmpfs'] == {'/tmp': 'rw,nosuid,nodev,size=1g'} and set(facts['NetworkSettings']['Networks']) <= {'none'}, 'BUILDER_ISOLATION')


class Runner:
    """Bounded private logs; no commands, environment, DSNs or errors on console."""
    def __init__(self, logs):
        self.logs, self.counter = logs, 0

    def run(self, command, *, env=None, timeout=60, stdin=None, allowed=(0,)):
        self.counter += 1
        prefix = self.logs / f'{self.counter:04d}'
        handles = [open_new(prefix.with_suffix('.' + name)) for name in ('stdout', 'stderr')]
        p = subprocess.Popen(command, stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env or HOST_ENV, start_new_session=True, close_fds=True)
        chunks, counts, reason = [bytearray(), bytearray()], [0, 0], None
        selector = selectors.DefaultSelector()
        try:
            if stdin is not None:
                p.stdin.write(stdin)
                p.stdin.close()
            for index, stream in enumerate((p.stdout, p.stderr)):
                selector.register(stream, selectors.EVENT_READ, index)
            deadline = time.monotonic() + timeout
            while selector.get_map():
                if time.monotonic() >= deadline:
                    reason = 'PROCESS_TIMEOUT'
                    break
                for key, _ in selector.select(0.1):
                    raw = os.read(key.fileobj.fileno(), 65536)
                    if not raw:
                        selector.unregister(key.fileobj)
                        continue
                    index = key.data
                    counts[index] += len(raw)
                    if counts[index] > LIMIT:
                        reason = 'PROCESS_LOG_BUDGET'
                        break
                    chunks[index].extend(raw)
                    handles[index].write(raw)
                if reason:
                    break
            if reason:
                os.killpg(p.pid, signal.SIGKILL)
            try:
                code = p.wait(timeout=max(0.1, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                os.killpg(p.pid, signal.SIGKILL)
                code, reason = p.wait(), 'PROCESS_TIMEOUT'
        finally:
            if p.poll() is None:
                os.killpg(p.pid, signal.SIGKILL)
                p.wait()
            selector.close()
            for stream in (p.stdout, p.stderr):
                stream.close()
            for handle in handles:
                handle.close()
        write_new(prefix.with_suffix('.process.json'), canonical({'exit_code': code, 'reason': reason, 'stdout_bytes': counts[0], 'stderr_bytes': counts[1]}))
        require(reason is None and code in allowed, reason or 'PROCESS_EXIT')
        return code, bytes(chunks[0]), bytes(chunks[1])

    def docker(self, *args, **kwargs):
        return self.run([DOCKER, *args], **kwargs)[1]

    def inspect(self, kind, identity):
        rows = json.loads(self.docker(kind, 'inspect', identity), object_pairs_hook=unique_pairs)
        require(type(rows) is list and len(rows) == 1, 'INSPECT_COUNT')
        return rows[0]


def open_new(path):
    return os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600), 'wb')


def occupied_subnets(configs):
    require(configs is None or type(configs) is list, 'IPAM_SHAPE')
    require(all(type(c) is dict and ('Subnet' not in c or isinstance(c['Subnet'], str)) for c in configs or []), 'IPAM_SHAPE')
    return [c['Subnet'] for c in configs or [] if c.get('Subnet') not in (None, '', 'default')]


def fresh_resources(runner, identities, subnets, builder_name, *, route_observer=None):
    occupied = []
    if route_observer is None:
        ip_tool = next((p for p in ('/usr/sbin/ip', '/usr/bin/ip') if Path(p).is_file()), None)
        require(ip_tool is not None, 'IP_TOOL_MISSING')
        routes = json.loads(runner.run([ip_tool, '-json', 'route', 'show', 'table', 'all'])[1])
    else:
        routes = route_observer()
        require(type(routes) is list and len(routes)<=4096 and all(type(row) is dict and set(row)=={'dst'} and type(row['dst']) is str for row in routes),'NATIVE_ROUTE_OBSERVATION')
    occupied.extend(r['dst'] for r in routes if r.get('dst') not in (None, 'default'))
    for kind, key in (('container', 'pg_name'), ('network', 'network'), ('volume', 'volume')):
        args = ('ps', '-a', '--format', '{{.Names}}\t{{.Label "com.docker.compose.project"}}') if kind == 'container' else (kind, 'ls', '--format', '{{.Name}}\t{{.Label "com.docker.compose.project"}}')
        for line in runner.docker(*args).decode().splitlines():
            name, _, project = line.partition('\t')
            require(name != builder_name and all(name != i[key] and project != i['project'] and not name.startswith(i['project'] + '-') for i in identities), 'RESOURCE_ALREADY_EXISTS')
            if kind == 'network':
                configs = json.loads(runner.docker('network', 'inspect', '--format', '{{json .IPAM.Config}}', name))
                occupied.extend(occupied_subnets(configs))
    return admit_subnets(subnets, occupied)


def build(runner, batch, source, manifest, archive_sha, result, image):
    target = batch / 'target'
    mkdir_new(target)
    name = 'knowweave-source-admission-builder-' + batch.name.removeprefix('batch-')
    supplied = {'CARGO_TARGET_DIR': '/target/build', 'CARGO_BUILD_JOBS': '4', 'CARGO_NET_OFFLINE': 'true', 'RUSTUP_AUTO_INSTALL': '0', 'KNOWWEAVE_SOURCE_COMMIT': manifest['base_commit'], 'KNOWWEAVE_BUILD_ID_SHA256': archive_sha}
    require('KNOWWEAVE_C4_VERIFIER_KEY_SHA256' not in env_dict(image['Config']['Env']), 'UNEXPECTED_VERIFIER_ENV')
    commands = ['cargo fmt --all -- --check', 'cargo clippy --locked --offline -p learning-backup --all-targets -- -D warnings']
    if manifest['snapshot_kind'] == 'working-tree-green':
        commands += ['cargo test --locked --offline -p learning-backup --lib', 'cargo test --locked --offline -p learning-backup --test maintenance_contract --test maintenance_journal']
    commands += ['cargo test --locked --offline -p learning-backup --lib --no-run --message-format=json']
    shell = '; '.join(commands)
    user = f'{os.geteuid()}:{os.getegid()}'
    args = ['create', '--name', name, '--pull=never', '--network=none', '--read-only', '--cap-drop=ALL', '--security-opt=no-new-privileges', '--cpus=4', '--memory=8g', '--memory-swap=8g', '--pids-limit=512', '--tmpfs=/tmp:rw,nosuid,nodev,size=1g', '--user=' + user, '--workdir=/reviewed', '--entrypoint=/bin/sh', '--label=knowweave.source-admission.batch=' + batch.name]
    for path, destination, ro in ((source, '/reviewed', True), (target, '/target', False)):
        args += ['--mount', f'type=bind,src={path},dst={destination}' + (',readonly' if ro else '')]
    for key, value in supplied.items():
        args += ['--env', key + '=' + value]
    container_id = runner.docker(*args, BUILDER, '-ec', shell).decode().strip()
    require(HEX64.fullmatch(container_id), 'BUILDER_ID')
    expected = {'id': container_id, 'name': name, 'image': BUILDER, 'env': {**env_dict(image['Config']['Env']), **supplied}, 'labels': {**(image['Config'].get('Labels') or {}), 'knowweave.source-admission.batch': batch.name}, 'cmd': ['-ec', shell], 'entrypoint': ['/bin/sh'], 'mounts': {('bind', str(source), '/reviewed', False), ('bind', str(target), '/target', True)}, 'network': 'none', 'builder': True, 'user': user}
    result['builder'] = {'id': container_id, 'removed': False}
    try:
        validate_container(runner.inspect('container', container_id), expected)
        runner.docker('start', container_id)
        build_exit = runner.docker('wait', container_id, timeout=7200).strip()
        require(re.fullmatch(b'[0-9]+', build_exit), 'BUILDER_EXIT_UNKNOWN')
        result['builder']['exit_code'] = int(build_exit)
        if build_exit != b'0':
            runner.docker('logs', container_id)
        require(build_exit == b'0', 'BUILD_FAILED')
        output = runner.docker('logs', container_id).decode()
        artifacts = []
        for line in output.splitlines():
            if not line.startswith('{'):
                continue
            row = json.loads(line)
            if row.get('reason') == 'compiler-artifact' and row.get('manifest_path') == '/reviewed/crates/learning-backup/Cargo.toml' and row.get('target', {}).get('name') == 'learning_backup' and row['target'].get('kind') == ['lib'] and row.get('profile', {}).get('test') is True:
                artifacts.append(row.get('executable'))
        require(len(artifacts) == 1 and isinstance(artifacts[0], str) and re.fullmatch('/target/build/debug/deps/learning_backup-[0-9a-f]+', artifacts[0]), 'BUILD_ARTIFACT')
        binary = target / artifacts[0].removeprefix('/target/')
        for parent in (target, target / 'build', target / 'build/debug', binary.parent):
            parent_meta = parent.lstat()
            require(stat.S_ISDIR(parent_meta.st_mode) and parent_meta.st_uid in (0, os.geteuid()) and not parent_meta.st_mode & 0o022, 'ARTIFACT_PARENT_IDENTITY')
        meta = binary.lstat()
        require(stat.S_ISREG(meta.st_mode) and meta.st_nlink == 1 and not meta.st_mode & 0o022 and os.access(binary, os.X_OK | os.R_OK), 'BINARY_IDENTITY')
        result['binary_sha256'] = digest(binary.read_bytes())
        listing = runner.run([str(binary), '--list', '--ignored'], env={k: HOST_ENV[k] for k in ('PATH', 'HOME', 'LC_ALL')})[1].decode()
        names = [s.removesuffix(': test') for s in listing.splitlines() if s.endswith(': test')]
        expected_names = TESTS[:1] if manifest['snapshot_kind'] == 'working-tree-red' else TESTS
        require(all(names.count(n) == 1 for n in expected_names), 'IGNORED_TEST_DISCOVERY')
        result['discovered'] = list(expected_names)
        return binary
    finally:
        validate_container(runner.inspect('container', container_id), expected)
        runner.docker('stop', '--time', '10', container_id)
        runner.docker('rm', container_id)
        require(container_id not in runner.docker('ps', '-aq', '--no-trunc').decode().split(), 'BUILDER_REMOVAL_UNKNOWN')
        result['builder']['removed'] = True


# No host password is available during init; all host authentication is SCRAM.
# Root exec in this NEW fixture reads owner0400 binds after real TCP readiness.
INITDB = b'''#!/bin/sh
set -eu
psql -v ON_ERROR_STOP=1 --username postgres --dbname postgres <<SQL
CREATE ROLE learning_admin LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE learning_runtime LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE learning_auth_lock NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
GRANT learning_auth_lock TO learning_admin WITH SET TRUE;
CREATE DATABASE "$C4_DATABASE" OWNER learning_admin;
CREATE DATABASE "$C4_OTHER_DATABASE" OWNER learning_admin;
REVOKE CONNECT ON DATABASE "$C4_DATABASE" FROM PUBLIC;
REVOKE CONNECT ON DATABASE "$C4_OTHER_DATABASE" FROM PUBLIC;
GRANT CONNECT ON DATABASE "$C4_DATABASE" TO learning_runtime, learning_admin;
GRANT CONNECT ON DATABASE "$C4_OTHER_DATABASE" TO learning_runtime, learning_admin;
SQL
'''
SET_PASSWORDS = '''a=$(cat /run/secrets/admin_password); r=$(cat /run/secrets/runtime_password)
psql -v ON_ERROR_STOP=1 --username postgres --dbname postgres <<SQL
ALTER ROLE learning_admin PASSWORD '$a';
ALTER ROLE learning_runtime PASSWORD '$r';
SQL
'''


def identity():
    case_id, other_id = str(uuid.uuid4()), str(uuid.uuid4())
    project = 'knowweave-source-admission-' + uuid.UUID(case_id).hex
    return {'case_id': case_id, 'project': project, 'pg_name': project + '-pg-1', 'network': project + '_test', 'volume': project + '_pg', 'database': 'learning_backup_c4_task3_' + case_id, 'other_database': 'learning_backup_c4_task3_' + other_id}


def pg_facts(runner, expected, ident, subnet, record):
    facts = runner.inspect('container', expected['id'])
    validate_container(facts, expected)
    network, volume = runner.inspect('network', ident['network']), runner.inspect('volume', ident['volume'])
    require(network['Id'] == record['network_id'] and HEX64.fullmatch(network['Id']) and network['Name'] == ident['network'] and network['Internal'] is True and network['Driver'] == 'bridge' and network['IPAM']['Config'] == [{'Subnet': subnet, 'Gateway': str(ipaddress.ip_network(subnet).network_address + 1)}], 'PG_NETWORK_IDENTITY')
    require(volume['Name'] == ident['volume'] and volume['Mountpoint'] == record['volume_mountpoint'] and volume['Driver'] == 'local' and not volume.get('Options'), 'PG_VOLUME_IDENTITY')
    require(all((item.get('Labels') or {}).get('com.docker.compose.project') == ident['project'] for item in (network, volume)), 'PG_PROJECT')
    running = facts['State']['Running']
    require(set(network.get('Containers') or {}) == ({expected['id']} if running else set()), 'PG_FOREIGN_ATTACHMENT')
    attached = facts['NetworkSettings']['Networks']
    require(set(attached) == {ident['network']} and attached[ident['network']]['NetworkID'] in ('', network['Id']), 'PG_ATTACHMENT')
    data_mount = next(m for m in facts['Mounts'] if m['Type'] == 'volume')
    volume_options = [m for m in facts['HostConfig']['Mounts'] if m.get('Target') == '/var/lib/postgresql']
    require(data_mount['Name'] == ident['volume'] and len(volume_options) == 1 and volume_options[0].get('VolumeOptions', {}).get('NoCopy') is True, 'PG_VOLUME_NOCOPY')
    return facts, attached[ident['network']]['IPAddress']


def run_case(runner, batch, ident, subnet, name, binary, image, result, mode):
    case = batch / ident['case_id']
    mkdir_new(case)
    mkdir_new(case / 'control')
    mkdir_new(case / 'secrets')
    passwords = {n: secrets.token_hex(32) for n in ('postgres', 'admin', 'runtime')}
    for role, password in passwords.items():
        write_new(case / 'secrets' / (role + '_password'), password.encode(), 0o400)
    write_new(case / 'initdb.sh', INITDB, 0o444)
    pg_env = {'POSTGRES_USER': 'postgres', 'POSTGRES_DB': 'postgres', 'POSTGRES_PASSWORD_FILE': '/run/secrets/postgres_password', 'POSTGRES_INITDB_ARGS': '--auth-host=scram-sha-256', 'C4_DATABASE': ident['database'], 'C4_OTHER_DATABASE': ident['other_database']}
    binds = [{'type': 'bind', 'source': str(case / 'initdb.sh'), 'target': '/docker-entrypoint-initdb.d/10-admission.sh', 'read_only': True}]
    binds += [{'type': 'bind', 'source': str(case / 'secrets' / (r + '_password')), 'target': '/run/secrets/' + r + '_password', 'read_only': True} for r in passwords]
    document = {'name': ident['project'], 'services': {'pg': {'image': PG_IMAGE, 'pull_policy': 'never', 'container_name': ident['pg_name'], 'command': ['postgres', '-c', 'max_prepared_transactions=16'], 'cpus': 2, 'mem_limit': '4g', 'memswap_limit': '4g', 'security_opt': ['no-new-privileges'], 'environment': pg_env, 'volumes': [{'type': 'volume', 'source': 'pg', 'target': '/var/lib/postgresql', 'volume': {'nocopy': True}}, *binds], 'networks': ['test']}}, 'networks': {'test': {'name': ident['network'], 'internal': True, 'ipam': {'config': [{'subnet': subnet}]}}}, 'volumes': {'pg': {'name': ident['volume']}}}
    write_new(case / 'compose.json', canonical(document))
    record = {'identity': ident, 'subnet': subnet, 'test': name, 'stopped': False, 'stage': 'creating', 'initdb_sha256': digest(INITDB)}
    result['cases'].append(record)
    fresh_resources(runner, [ident], [subnet], '')
    compose = [DOCKER, 'compose', '--project-name', ident['project'], '--project-directory', str(case), '-f', str(case / 'compose.json')]
    expected = None
    try:
        runner.run([*compose, 'create', '--no-build', '--pull', 'never'])
        facts = runner.inspect('container', ident['pg_name'])
        record['container_id'] = facts['Id']
        network, volume = runner.inspect('network', ident['network']), runner.inspect('volume', ident['volume'])
        record.update(network_id=network['Id'], volume_mountpoint=volume['Mountpoint'], volume_name=volume['Name'])
        mounts = {('bind', b['source'], b['target'], False) for b in binds} | {('volume', volume['Mountpoint'], '/var/lib/postgresql', True)}
        expected = {'id': facts['Id'], 'name': ident['pg_name'], 'image': image['Id'], 'image_ref': PG_IMAGE, 'env': {**env_dict(image['Config']['Env']), **pg_env}, 'labels': facts['Config']['Labels'], 'cmd': document['services']['pg']['command'], 'entrypoint': image['Config']['Entrypoint'], 'mounts': mounts, 'network': ident['network'], 'builder': False}
        require(expected['labels'].get('com.docker.compose.project') == ident['project'] and expected['labels'].get('com.docker.compose.service') == 'pg' and expected['labels'].get('com.docker.compose.project.working_dir') == str(case) and expected['labels'].get('com.docker.compose.project.config_files') == str(case / 'compose.json'), 'PG_COMPOSE_LABELS')
        pg_facts(runner, expected, ident, subnet, record)
        runner.docker('start', expected['id'])
        deadline = time.monotonic() + 90
        while True:
            code = runner.run([DOCKER, 'exec', expected['id'], 'pg_isready', '-h', '127.0.0.1', '-p', '5432', '-U', 'postgres', '-d', 'postgres'], allowed=(0, 1, 2, 3))[0]
            if code == 0:
                break
            require(time.monotonic() < deadline, 'PG_TCP_NOT_READY')
            time.sleep(0.5)
        pg_facts(runner, expected, ident, subnet, record)
        verify_sql = b"SELECT current_setting('server_version_num')::int / 10000, current_setting('max_prepared_transactions');\nSELECT count(*) > 0 AND bool_and(auth_method = 'scram-sha-256') FROM pg_hba_file_rules WHERE type LIKE 'host%';\nSELECT count(*) = 2 AND bool_and(rolpassword IS NULL) FROM pg_authid WHERE rolname IN ('learning_admin','learning_runtime');\n"
        observation = runner.run([DOCKER, 'exec', '-i', expected['id'], 'psql', '-At', '-v', 'ON_ERROR_STOP=1', '-U', 'postgres', '-d', 'postgres'], stdin=verify_sql)[1]
        require(observation.strip() == b'18|16\nt\nt', 'PG_VERSION_HBA_NULL_PASSWORD')
        runner.docker('exec', '--user', '0:0', expected['id'], '/bin/sh', '-ec', SET_PASSWORDS)
        validate_pg_processes(runner.docker('top', expected['id'], '-eo', 'uid,pid,comm').decode())
        facts, ip = pg_facts(runner, expected, ident, subnet, record)
        require(facts['State']['Running'] is True and ipaddress.ip_address(ip) in ipaddress.ip_network(subnet) and ip not in (str(ipaddress.ip_network(subnet).network_address), str(ipaddress.ip_network(subnet).broadcast_address)), 'PG_IP')
        task_env = {k: HOST_ENV[k] for k in ('PATH', 'HOME', 'LC_ALL')}
        for other, db in ((False, ident['database']), (True, ident['other_database'])):
            path = case / ('other-admin.dsn' if other else 'admin.dsn')
            write_new(path, f'postgresql://learning_admin:{passwords["admin"]}@{ip}:5432/{db}?sslmode=disable'.encode())
            task_env['TEST_C4_ADMISSION_' + ('OTHER_' if other else '') + 'DATABASE_NAME'] = db
            task_env['TEST_C4_ADMISSION_' + ('OTHER_' if other else '') + 'ADMIN_DSN_FILE'] = str(path)
        task_env['TEST_C4_ADMISSION_CONTROL_ROOT'] = str(case / 'control')
        require(digest(binary.read_bytes()) == result['binary_sha256'], 'BINARY_CHANGED')
        record['stage'] = 'executing'
        code, stdout, _ = runner.run([str(binary), '--ignored', '--exact', name, '--test-threads=1'], env=task_env, timeout=180, allowed=(0, 101))
        record['outcome'] = parse_test(code, stdout.decode(), name, mode)
        require(digest(binary.read_bytes()) == result['binary_sha256'], 'BINARY_CHANGED')
        record['stage'] = 'completed'
    finally:
        if expected is not None:
            pg_facts(runner, expected, ident, subnet, record)
            runner.docker('stop', '--time', '10', expected['id'])
            facts, _ = pg_facts(runner, expected, ident, subnet, record)
            require(facts['State']['Running'] is False, 'PG_STOP_UNKNOWN')
            record['stopped'] = True
            runner.docker('logs', expected['id'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('archive', 'archive-sha256', 'manifest-sha256', 'runner-sha256', 'batch-id'):
        parser.add_argument('--' + key, required=True)
    parser.add_argument('--mode', choices=('red', 'green'), required=True)
    parser.add_argument('--subnet', action='append', required=True)
    args = parser.parse_args()
    result, batch = {'status': 'FAILED', 'cleanup_verified': False, 'cases': []}, None
    try:
        require(sys.platform == 'linux' and os.geteuid() != 0 and Path.home() == Path('/home/hans'), 'ORDINARY_HANS_LINUX_REQUIRED')
        os.umask(0o077)
        runner_path, archive = Path(__file__).absolute(), Path(args.archive)
        stage = runner_path.parent
        require(stage.parent == BASE and archive.is_absolute() and archive.parent == stage and runner_path.name == Path(ENTRY).name, 'FIXED_STAGE_REQUIRED')
        for path in (Path('/home/hans'), *reversed(BASE.parents[:-2]), BASE, stage):
            meta = path.lstat()
            require(stat.S_ISDIR(meta.st_mode) and meta.st_uid == os.geteuid() and not meta.st_mode & 0o022, 'PARENT_IDENTITY')
        require(stage.lstat().st_uid == os.geteuid() and stat.S_IMODE(stage.lstat().st_mode) == 0o700, 'STAGE_PRIVATE')
        require(digest(owned_file(runner_path, (0o400, 0o500))) == args.runner_sha256, 'RUNNER_DIGEST')
        manifest, files = verify_package(owned_file(archive, (0o400,)), args.archive_sha256, args.manifest_sha256, args.runner_sha256, args.mode)
        parsed = uuid.UUID(args.batch_id)
        require(parsed.version == 4 and str(parsed) == args.batch_id and len(args.subnet) == (1 if args.mode == 'red' else 5), 'BATCH_OR_SUBNET_COUNT')
        subnets = admit_subnets(args.subnet, [])
        new_batch = stage / ('batch-' + args.batch_id)
        mkdir_new(new_batch)
        batch = new_batch
        mkdir_new(batch / 'evidence')
        write_new(batch / 'evidence/result.pending', b'PENDING')
        mkdir_new(batch / 'evidence/logs')
        result.update(stage='extracting', batch_id=args.batch_id, mode=args.mode, base_commit=manifest['base_commit'], snapshot_kind=manifest['snapshot_kind'], archive_sha256=args.archive_sha256, manifest_sha256=args.manifest_sha256, runner_sha256=args.runner_sha256, no_full_backup_restore=True, production_root_trust=False)
        source = batch / 'source'
        extract_source(source, files)
        result['source_sha256_before'] = source_digest(source, manifest)
        runner = Runner(batch / 'evidence/logs')
        result['stage'] = 'preflight'
        identities = [identity() for _ in subnets]
        fresh_resources(runner, identities, subnets, 'knowweave-source-admission-builder-' + args.batch_id)
        images = {ref: runner.inspect('image', ref) for ref in (PG_IMAGE, BUILDER)}
        require(images[BUILDER]['Id'] == BUILDER and any(ref.split('@')[-1] == PG_IMAGE.split('@')[-1] for ref in images[PG_IMAGE].get('RepoDigests', [])), 'CACHED_IMAGE_PIN')
        result['stage'] = 'offline-build'
        binary = build(runner, batch, source, manifest, args.archive_sha256, result, images[BUILDER])
        for ident, subnet, name in zip(identities, subnets, TESTS):
            result['stage'] = name
            run_case(runner, batch, ident, subnet, name, binary, images[PG_IMAGE], result, args.mode)
            require(source_digest(source, manifest) == result['source_sha256_before'], 'SOURCE_CHANGED')
        result['source_sha256_after'] = source_digest(source, manifest)
        result['status'] = 'RED_EVIDENCE' if args.mode == 'red' else 'FIVE_ADMISSION_GATES_PASSED_NOT_FULL_BACKUP_NOT_RESTORE'
    except BaseException as error:
        fail_result(result, str(error) if isinstance(error, GateError) else 'UNEXPECTED_ERROR')
    finally:
        try:
            if batch is not None and (batch / 'evidence/result.pending').is_file():
                result['cleanup_verified'] = bool(result.get('builder', {}).get('removed')) and all(c.get('stopped') for c in result['cases'])
                if not result['cleanup_verified']:
                    fail_result(result, 'CLEANUP_UNKNOWN')
                temporary = batch / 'evidence/result.finalizing'
                write_new(temporary, canonical(result))
                os.link(temporary, batch / 'evidence/result.json', follow_symlinks=False)
                temporary.unlink()
                if result['cleanup_verified']:
                    (batch / 'evidence/result.pending').unlink()
                fd = os.open(batch / 'evidence', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
                try:
                    os.fsync(fd)
                finally:
                    os.close(fd)
        except BaseException:
            fail_result(result, 'RESULT_PERSISTENCE_UNKNOWN')
        print(canonical(result).decode())
    return 0 if result['status'] != 'FAILED' else 1


if __name__ == '__main__':
    sys.exit(main())
