"""Independent fresh Linux/PG18 source-binding evidence. NO_DUMP; SYNTHETIC journal phase digests."""
import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import stat
import sys
import time
import types
import uuid

BASE = Path('/home/hans/knowweave-c4-source-binding')
ENTRY = 'scripts/p0c4_source_binding_gate_acceptance.py'
HELPER_ENTRY = 'scripts/p0c4_source_admission_gate_acceptance.py'
HELPER_SHA = 'e407684f101d467cc0d7271ff678489a2cc894fa97f507e9d8ebada06f8a73ba'
PIN_ENV = 'KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256'
SOURCE_ROOT = '/var/lib/knowweave-source'
CONTROL = SOURCE_ROOT + '/control'
MANIFEST = '_knowweave_source_manifest.json'
HEX64 = re.compile(r'[0-9a-f]{64}\Z')
TESTS = tuple('source::binding_tests::' + name for name in (
    'alternate_root_after_session_loss_preserves_acl_and_journal',
    'original_bound_root_keeps_unfinished_journal_authoritative',
    'matching_bound_recovery_recloses_without_finishing',
    'another_database_is_rejected_before_source_mutation'))
ACL_ASSERTION = 'alternate root recovery must preserve runtime CONNECT ACL after original backend loss'
REGRESSIONS = tuple('source::admission_tests::' + name for name in (
    'same_database_attempts_share_admission_and_other_database_is_independent',
    'single_connection_catalog_and_drain_share_admitted_backend'))


class GateError(RuntimeError):
    pass


def require(condition, reason):
    if not condition:
        raise GateError(reason)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode()


def load_helper(path, private=False):
    """Execute only the bytes whose pinned hash was checked, never re-open/import."""
    flags = os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0)
    with os.fdopen(os.open(path, flags), 'rb') as handle:
        meta = os.fstat(handle.fileno())
        require(stat.S_ISREG(meta.st_mode) and meta.st_nlink == 1 and meta.st_size <= 128 * 1024, 'HELPER_FILE_IDENTITY')
        if private:
            require(meta.st_uid == os.geteuid() and stat.S_IMODE(meta.st_mode) in (0o400, 0o500), 'HELPER_PRIVATE_MODE')
        raw = handle.read(128 * 1024 + 1)
    require(digest(raw) == HELPER_SHA, 'IMMUTABLE_HELPER_DIGEST')
    module = types.ModuleType('p0c4_pinned_admission_helper')
    module.__file__ = str(path)
    exec(compile(raw, str(path), 'exec'), module.__dict__)
    module.GateError = GateError
    return module


def verify_package(helper, raw, archive_sha, manifest_sha, runner_sha, mode):
    # The immutable helper admits the full original public-source schema first.
    manifest, files = helper.verify_package(raw, archive_sha, manifest_sha, HELPER_SHA, mode)
    require(ENTRY in files and 'scripts/test_p0c4_source_binding_gate_acceptance.py' in files and
            'crates/learning-backup/src/source/binding_tests.rs' in files, 'BINDING_SOURCE_REQUIRED')
    require(digest(files[HELPER_ENTRY]) == HELPER_SHA, 'PACKAGED_HELPER_DIGEST')
    require(isinstance(runner_sha, str) and HEX64.fullmatch(runner_sha) and digest(files[ENTRY]) == runner_sha,
            'PACKAGED_BINDING_RUNNER_DIGEST')
    return manifest, files


def extract_public_source(helper, source, files):
    helper.extract_source(source, files)
    for path in source.rglob('*'):
        path.chmod(0o755 if path.is_dir() else 0o444)
    source.chmod(0o755)


def source_digest(helper, source, manifest):
    for path in (source, *source.rglob('*')):
        meta = path.lstat()
        require(meta.st_uid == os.geteuid() and stat.S_IMODE(meta.st_mode) ==
                (0o755 if stat.S_ISDIR(meta.st_mode) else 0o444), 'PUBLIC_SOURCE_MODE')
    return helper.source_digest(source, manifest, enforce_mode=False)


def parse_test(code, output, stderr, name, mode):
    require(stderr == b'', 'EXACT_TEST_STDERR')
    lines = [line.strip() for line in output.splitlines() if line.strip()]
    tests = [line for line in lines if line.startswith('test ') and not line.startswith('test result:')]
    summaries = [line for line in lines if line.startswith('test result:')]
    green = mode == 'green'
    require(code == (0 if green else 101) and lines.count('running 1 test') == 1 and
            tests == [f'test {name} ... ' + ('ok' if green else 'FAILED')], 'EXACT_TEST_EXIT')
    prefix = 'ok. 1 passed; 0 failed' if green else 'FAILED. 0 passed; 1 failed'
    require(len(summaries) == 1 and re.fullmatch(r'test result: ' + re.escape(prefix) +
            r'; 0 ignored; 0 measured; \d+ filtered out; finished in [0-9.]+s', summaries[0]), 'EXACT_TEST_COUNTS')
    if green:
        require(len(lines) == 3, 'EXACT_TEST_OUTPUT')
    else:
        require(name == TESTS[0] and ACL_ASSERTION in output and
                f'---- {name} stdout ----' in lines and lines.count(name) == 1 and
                'NO_DUMP SYNTHETIC: original_backend_absent=true lock_free=true recovery_ok=true connect_before=true connect_after=false'
                in output, 'ACL_RED_EVIDENCE')
    return {'test': name, 'exit_code': code, 'passed': int(green), 'failed': int(not green), 'ignored': 0,
            'stdout_sha256': digest(output.encode()), 'classification': 'NO_DUMP_SYNTHETIC'}


def parse_regression(output):
    """Capture actual libtest counts independently from live exact binding gates."""
    matches = re.findall(r'^test result: ok\. (\d+) passed; 0 failed; (\d+) ignored; 0 measured; (\d+) filtered out; finished in [0-9.]+s$', output, re.M)
    require(bool(matches), 'REGRESSION_COUNTS_MISSING')
    require(not re.search(r'^test result: (?!ok\.)', output, re.M), 'REGRESSION_FAILED')
    return [{'passed': int(p), 'failed': 0, 'ignored': int(i), 'filtered': int(f)} for p, i, f in matches]


def discover_binary(output):
    artifacts = []
    finished = []
    for line in output.splitlines():
        row = json.loads(line)
        if row.get('reason') == 'build-finished':
            finished.append(row.get('success'))
        if (row.get('reason') == 'compiler-artifact' and
                row.get('manifest_path') == '/reviewed/crates/learning-backup/Cargo.toml' and
                row.get('target', {}).get('name') == 'learning_backup' and
                row['target'].get('kind') == ['lib'] and row.get('profile', {}).get('test') is True):
            artifacts.append(row.get('executable'))
    require(finished == [True] and len(artifacts) == 1 and isinstance(artifacts[0], str) and
            re.fullmatch('/target/build/debug/deps/learning_backup-[0-9a-f]+', artifacts[0]), 'BUILD_ARTIFACT')
    return artifacts[0]


def identity():
    case, other = str(uuid.uuid4()), str(uuid.uuid4())
    project = 'knowweave-source-binding-' + uuid.UUID(case).hex
    return {'case_id': case, 'project': project, 'pg_name': project + '-pg-1', 'network': project + '_test',
            'volume': project + '_pg', 'source_volume': project + '_source', 'registry_volume': project + '_registry',
            'database': 'learning_backup_c4_task3_' + case, 'other_database': 'learning_backup_c4_task3_' + other}


def validate_helper(helper, facts, expected):
    """Exact root helper predicate, including joins of one case's internal network."""
    config, host = facts['Config'], facts['HostConfig']
    require(facts['Id'] == expected['id'] and HEX64.fullmatch(facts['Id']) and
            facts['Name'] == '/' + expected['name'] and facts['Image'] == expected['image'] and
            config['Image'] == expected['image'], 'HELPER_IDENTITY')
    require(config['User'] == '0:0' and config['WorkingDir'] == '/reviewed' and
            config['Labels'] == expected['labels'] and config['Cmd'] == expected['cmd'] and
            config['Entrypoint'] == expected['entrypoint'] and
            helper.env_dict(config['Env']) == expected['env'] and config['OpenStdin'] == expected['stdin'], 'HELPER_CONFIG')
    require(host['Privileged'] is False and not host.get('CapAdd') and host.get('CapDrop') in
            (['ALL'], ['CAP_ALL']) and host['SecurityOpt'] in
            (['no-new-privileges'], ['no-new-privileges:true']), 'HELPER_SECURITY')
    require(host['NanoCpus'] == 4 * 10**9 and host['Memory'] == 8 * 1024**3 and
            host['MemorySwap'] == host['Memory'] and host['PidsLimit'] == 512 and
            host['ReadonlyRootfs'] is True and host['Tmpfs'] == {'/tmp': 'rw,nosuid,nodev,size=1g'}, 'HELPER_BUDGET')
    require(host.get('PidMode', '') == '' and host['IpcMode'] == 'private' and not any(host.get(key)
            for key in ('Binds', 'Devices', 'DeviceRequests', 'VolumesFrom', 'Links', 'ExtraHosts',
                        'AutoRemove', 'PortBindings', 'PublishAllPorts', 'UTSMode', 'CgroupnsMode')
            if key != 'CgroupnsMode') and host.get('CgroupnsMode', 'private') == 'private', 'HELPER_HOST_ACCESS')
    require({(m['Type'], m['Source'], m['Destination'], m['RW']) for m in facts['Mounts']} == expected['mounts']
            and len(facts['Mounts']) == len(expected['mounts']), 'HELPER_MOUNTS')
    for mount in facts['Mounts']:
        if mount['Type'] == 'volume':
            require(mount['Name'] == expected['volume_names'][mount['Destination']], 'HELPER_VOLUME_NAME')
            declared = [m for m in host.get('Mounts', []) if m.get('Target') == mount['Destination']]
            require(len(declared) == 1 and declared[0].get('VolumeOptions', {}).get('NoCopy') is True, 'HELPER_VOLUME_NOCOPY')
    require(host['NetworkMode'] == expected['network'] and not any((facts['NetworkSettings'].get('Ports') or {}).values()), 'HELPER_NETWORK')
    state = facts.get('State') or {}
    status, running = state.get('Status'), state.get('Running')
    zero_time = '0001-01-01T00:00:00Z'
    started, finished = state.get('StartedAt'), state.get('FinishedAt')
    timestamp = r'\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z'
    require(status in ('created', 'running', 'exited') and type(running) is bool and
            running == (status == 'running') and isinstance(started, str) and isinstance(finished, str) and
            re.fullmatch(timestamp, started) and re.fullmatch(timestamp, finished), 'HELPER_STATE')
    never_started = status == 'created' and started == finished == zero_time
    require(never_started or (status in ('running', 'exited') and started != zero_time and
            (finished == zero_time if running else finished != zero_time)), 'HELPER_STATE')
    attached = facts['NetworkSettings']['Networks']
    if expected['network'] == 'none':
        require(set(attached) <= {'none'}, 'HELPER_OFFLINE')
    else:
        require(isinstance(expected['network_id'], str) and HEX64.fullmatch(expected['network_id']) and
                set(attached) == {expected['network']} and
                attached[expected['network']].get('NetworkID') in
                (('', expected['network_id']) if never_started else (expected['network_id'],)), 'HELPER_ATTACHMENT')


def validate_case_network(facts, expected, *, helper_id=None, running=False):
    """Check the independent network object, never an uninitialized endpoint ID."""
    require(HEX64.fullmatch(expected['id']) and HEX64.fullmatch(expected['pg_id']) and
            facts.get('Id') == expected['id'] and facts.get('Name') == expected['name'] and
            facts.get('Internal') is True and facts.get('Driver') == 'bridge' and
            (facts.get('Labels') or {}).get('com.docker.compose.project') == expected['project'], 'HELPER_CASE_NETWORK')
    require((facts.get('IPAM') or {}).get('Config') == [{'Subnet': expected['subnet'],
            'Gateway': str(ipaddress.ip_network(expected['subnet']).network_address + 1)}], 'HELPER_CASE_SUBNET')
    members = facts.get('Containers') or {}
    require(type(members) is dict and type(running) is bool and
            (helper_id is None or (isinstance(helper_id, str) and HEX64.fullmatch(helper_id))) and
            (not running or helper_id is not None) and
            set(members) == ({expected['pg_id'], helper_id} if running else {expected['pg_id']}), 'HELPER_CASE_MEMBERSHIP')


def cleanup_verified(result):
    # Empty observations do not establish cleanup after an ambiguous create.
    return (result.get('preflight_complete') is True and result.get('resource_creation_unknown') is not True and
            all(h.get('removed') is True for h in result.get('helpers', [])) and
            all(c.get('stopped') is True for c in result.get('cases', [])))


def create_batch(helper, stage, batch_id):
    # Return the path only after exclusive creation; failed replays own nothing.
    batch = stage / ('batch-' + batch_id)
    helper.mkdir_new(batch)
    helper.mkdir_new(batch / 'evidence')
    helper.write_new(batch / 'evidence/result.pending', b'PENDING')
    helper.mkdir_new(batch / 'evidence/logs')
    return batch


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def finalize_result(helper, batch, result):
    evidence = batch / 'evidence'
    temporary = evidence / 'result.finalizing'
    helper.write_new(temporary, canonical(result))
    os.link(temporary, evidence / 'result.json', follow_symlinks=False)
    temporary.unlink()
    sync_directory(evidence)
    if result['cleanup_verified']:
        (evidence / 'result.pending').unlink()
        sync_directory(evidence)


def create_volume(helper, runner, name, labels):
    require(name not in runner.docker('volume', 'ls', '--format', '{{.Name}}').decode().splitlines(), 'VOLUME_ALREADY_EXISTS')
    args = ['volume', 'create', '--driver=local']
    for key, value in labels.items():
        args += ['--label', key + '=' + value]
    require(runner.docker(*args, name).decode().strip() == name, 'VOLUME_CREATE')
    facts = runner.inspect('volume', name)
    require(facts['Name'] == name and facts['Driver'] == 'local' and not facts.get('Options') and facts['Labels'] == labels,
            'VOLUME_IDENTITY')
    return facts


def run_helper(helper, runner, result, image, mounts, volume_names, command, *, env=None,
               network='none', network_id=None, network_identity=None, stdin=None, timeout=180, allowed=(0,)):
    name = 'knowweave-binding-helper-' + uuid.uuid4().hex
    labels = {**(image['Config'].get('Labels') or {}), 'knowweave.source-binding.batch': result['batch_id']}
    supplied = env or {}
    require(not any(key in supplied for key in ('PGPASSWORD', 'DATABASE_URL', 'ADMIN_DSN')), 'HELPER_SECRET_ENV')
    if network != 'none':
        require(type(network_identity) is dict and network_identity.get('id') == network_id and
                network_identity.get('name') == network, 'HELPER_CASE_IDENTITY_REQUIRED')
        validate_case_network(runner.inspect('network', network_id), network_identity)
    args = ['create', '--name', name, '--pull=never', '--network=' + network, '--read-only', '--cap-drop=ALL',
            '--security-opt=no-new-privileges', '--cpus=4', '--memory=8g', '--memory-swap=8g', '--pids-limit=512',
            '--tmpfs=/tmp:rw,nosuid,nodev,size=1g', '--user=0:0', '--workdir=/reviewed', '--entrypoint=' + command[0]]
    if stdin is not None:
        args += ['--interactive']
    for key, value in labels.items():
        args += ['--label', key + '=' + value]
    expected_mounts = set()
    for kind, source, destination, readonly in mounts:
        suffix = ',readonly' if readonly else ''
        if kind == 'volume':
            suffix += ',volume-nocopy'
            mount_source = runner.inspect('volume', source)['Mountpoint']
        else:
            mount_source = source
        args += ['--mount', f'type={kind},src={source},dst={destination}' + suffix]
        expected_mounts.add((kind, mount_source, destination, not readonly))
    for key, value in supplied.items():
        args += ['--env', key + '=' + value]
    result['resource_creation_unknown'] = True
    container_id = runner.docker(*args, image['Id'], *command[1:]).decode().strip()
    require(HEX64.fullmatch(container_id), 'HELPER_CREATED_ID')
    expected = {'id': container_id, 'name': name, 'image': image['Id'], 'labels': labels, 'cmd': command[1:],
                'entrypoint': [command[0]], 'env': {**helper.env_dict(image['Config']['Env']), **supplied},
                'mounts': expected_mounts, 'volume_names': volume_names, 'network': network,
                'network_id': network_id, 'stdin': stdin is not None}
    record = {'id': container_id, 'name': name, 'removed': False}
    result.setdefault('helpers', []).append(record)
    result['resource_creation_unknown'] = False
    def inspect_verified():
        facts = runner.inspect('container', container_id)
        validate_helper(helper, facts, expected)
        if network != 'none':
            validate_case_network(runner.inspect('network', network_id), network_identity,
                                  helper_id=container_id, running=facts['State']['Running'])
        return facts
    try:
        require(inspect_verified()['State']['Status'] == 'created', 'HELPER_NOT_FRESH_CREATED')
        args = ['start', '-a'] + (['-i'] if stdin is not None else []) + [container_id]
        _, stdout, stderr = runner.run([helper.DOCKER, *args], stdin=stdin, timeout=timeout, allowed=tuple(set((0, *allowed))))
        facts = inspect_verified()
        require(facts['State']['Status'] == 'exited' and facts['State']['Running'] is False and not facts['State'].get('OOMKilled') and
                facts['State']['ExitCode'] in allowed, 'HELPER_PROCESS_EXIT')
        record['exit_code'] = facts['State']['ExitCode']
        return facts['State']['ExitCode'], stdout, stderr
    finally:
        inspect_verified()
        runner.docker('stop', '--time', '10', container_id)
        facts = inspect_verified()
        require(facts['State']['Running'] is False, 'HELPER_STOP_UNKNOWN')
        runner.docker('rm', container_id)
        require(container_id not in runner.docker('ps', '-aq', '--no-trunc').decode().split(), 'HELPER_REMOVAL_UNKNOWN')
        record['removed'] = True


# This code runs independently inside a root, cap-drop-ALL, network-none helper.
# SQL identity was observed through learning_admin, in the new PG fixture.
ISSUER = r'''
import hashlib,json,os,pathlib,re,stat,sys,uuid
def check(ok):
    if not ok: raise RuntimeError('ISSUER_IDENTITY')
def canon(v): return json.dumps(v,sort_keys=True,separators=(',',':')).encode()
def new(p,raw):
    fd=os.open(p,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    with os.fdopen(fd,'wb') as f: f.write(raw);f.flush();os.fsync(f.fileno())
payload=json.loads(sys.stdin.buffer.read(16385));check(len(canon(payload))<=16384)
sys.path.insert(0,'/reviewed/scripts')
from p0c4_storage_registry import REGISTRY_PATH,_root_helper_context,provision_registry
root=pathlib.Path('/var/lib/knowweave-source');check(os.geteuid()==0)
for p in (pathlib.Path('/'),pathlib.Path('/var'),pathlib.Path('/var/lib'),root):
    s=p.lstat();check(stat.S_ISDIR(s.st_mode) and s.st_uid==0 and s.st_mode&0o022==0)
check(list(root.iterdir())==[])
os.chmod(root,0o700);control=root/'control';control.mkdir(mode=0o700)
fd=os.open(control,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
s=os.fstat(fd);check(s.st_uid==0 and stat.S_IMODE(s.st_mode)==0o700 and s.st_dev>0 and s.st_ino>0)
db=payload['sql']['database'];oid=payload['sql']['database_oid'];sid=payload['sql']['system_identifier']
check(re.fullmatch(r'learning_backup_c4_task3_[0-9a-f-]{36}',db) and str(uuid.UUID(db[25:]))==db[25:])
check(type(oid)==int and 0<oid<=4294967295 and re.fullmatch('[1-9][0-9]*',sid) and int(sid)<=18446744073709551615)
binding={'format_version':1,'capability':'source_control_binding_v1','binding_id':str(uuid.uuid4()),'control_path':str(control),
         'control_dev':s.st_dev,'control_ino':s.st_ino,'database':db,'database_oid':oid,'system_identifier':sid}
raw=canon(binding);check(len(raw)<=4096);new(control/'source-binding.json',raw)
for name in ('pins','assets','asset-staging','proofs'): (root/name).mkdir(mode=0o700)
registry_root=pathlib.Path(REGISTRY_PATH);registry_root.chmod(0o700)
registry_fd=os.open(registry_root,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
held={key:(os.open(root/name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW),str(root/name)) for key,name in (('control','control'),('local_pins','pins'),('assets','assets'),('staging','asset-staging'))}
try:
    context=_root_helper_context(accepted_case=payload['registry_case'],source_binding_bytes=raw,source_database_facts=payload['sql'],source_roots=held,registry_root_handle=registry_fd)
    receipt=provision_registry(context).record;new(root/'proofs/registry-provisioning.json',canon(receipt))
finally:
    os.close(registry_fd)
    for desc,_ in held.values():os.close(desc)
new(root/'admin.dsn',payload['admin_dsn'].encode());new(root/'other-admin.dsn',payload['other_admin_dsn'].encode())
reg=root/'regression';reg.mkdir(mode=0o700)
os.fsync(fd);os.close(fd)
print(canon({'binding':binding,'binding_sha256':hashlib.sha256(raw).hexdigest(),'sql':payload['sql'],'issuer_euid':os.geteuid(),
             'classification':'NO_DUMP_SYNTHETIC','registry_provisioning':receipt}).decode())
'''

AUDIT = r'''
import hashlib,json,os,pathlib,stat,sys
root=pathlib.Path('/var/lib/knowweave-source/control');path=root/'source-binding.json'
fd=os.open(root,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW);s=os.fstat(fd)
f=os.open('source-binding.json',os.O_RDONLY|os.O_NOFOLLOW,dir_fd=fd);m=os.fstat(f)
assert s.st_uid==0 and stat.S_IMODE(s.st_mode)==0o700 and m.st_uid==0 and stat.S_IMODE(m.st_mode)==0o600 and m.st_nlink==1 and stat.S_ISREG(m.st_mode) and m.st_size<=4096
raw=os.read(f,4097);os.close(f);os.close(fd)
binary=pathlib.Path(sys.argv[1]);b=binary.lstat()
assert stat.S_ISREG(b.st_mode) and b.st_uid==0 and b.st_nlink==1 and stat.S_IMODE(b.st_mode)==0o500
print(json.dumps({'binding_sha256':hashlib.sha256(raw).hexdigest(),'control_dev':s.st_dev,'control_ino':s.st_ino,
                  'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest()},sort_keys=True,separators=(',',':')))
'''

COPY_BINARY = r'''
import hashlib,json,os,pathlib,stat,sys
src=pathlib.Path(sys.argv[1]);dst=pathlib.Path(sys.argv[2]);s=src.lstat()
assert stat.S_ISREG(s.st_mode) and s.st_uid==0 and s.st_nlink==1 and s.st_mode&0o022==0
dst.parent.mkdir(mode=0o700,exist_ok=True)
fd=os.open(dst,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o500)
with os.fdopen(fd,'wb') as f: f.write(src.read_bytes());f.flush();os.fsync(f.fileno())
print(json.dumps({'binary_sha256':hashlib.sha256(dst.read_bytes()).hexdigest()},sort_keys=True,separators=(',',':')))
'''


def parse_issuer(helper, raw, sql):
    require(len(raw) <= 16384, 'ISSUER_OUTPUT_BUDGET')
    row = json.loads(raw, object_pairs_hook=helper.unique_pairs)
    require(raw in (canonical(row), canonical(row) + b'\n') and set(row) == {'binding', 'binding_sha256', 'sql', 'issuer_euid', 'classification','registry_provisioning'}
            and row['issuer_euid'] == 0 and row['sql'] == sql and row['classification'] == 'NO_DUMP_SYNTHETIC', 'ISSUER_OBSERVATION')
    binding = row['binding']
    require(set(binding) == {'format_version', 'capability', 'binding_id', 'control_path', 'control_dev', 'control_ino',
                            'database', 'database_oid', 'system_identifier'}, 'ISSUER_BINDING_FIELDS')
    require(type(binding['format_version']) is int and binding['format_version'] == 1 and
            binding['capability'] == 'source_control_binding_v1' and binding['control_path'] == CONTROL and
            all(type(binding[k]) is int and 0 < binding[k] <= 2**64 - 1 for k in ('control_dev', 'control_ino')) and
            all(binding[k] == sql[k] for k in ('database', 'database_oid', 'system_identifier')), 'ISSUER_BINDING_IDENTITY')
    ident = uuid.UUID(binding['binding_id'])
    require(ident.version == 4 and str(ident) == binding['binding_id'] and
            digest(canonical(binding)) == row['binding_sha256'], 'ISSUER_CANONICAL_PIN')
    try: from p0c4_storage_registry import validate_provisioning_receipt
    except ModuleNotFoundError: from scripts.p0c4_storage_registry import validate_provisioning_receipt
    validate_provisioning_receipt(row['registry_provisioning'])
    require(row['registry_provisioning']['source_binding_sha256']==row['binding_sha256'],'ISSUER_REGISTRY_BINDING')
    return row


def case_mounts(source, build_volume, ident, *, write_build=True, write_control=False):
    return [('bind', str(source), '/reviewed', True), ('volume', build_volume, '/target', not write_build),
            ('volume', ident['source_volume'], SOURCE_ROOT, not write_control), ('volume', ident['registry_volume'], '/var/lib/knowweave-c4/registry', not write_control)]


def compile_case(helper, runner, result, image, mounts, volumes, manifest, archive_sha, record, first, mode):
    mounts = [(kind, source, dest, True if dest == SOURCE_ROOT else readonly)
              for kind, source, dest, readonly in mounts]
    require(PIN_ENV not in helper.env_dict(image['Config']['Env']) and
            'KNOWWEAVE_C4_VERIFIER_KEY_SHA256' not in helper.env_dict(image['Config']['Env']), 'UNEXPECTED_IMAGE_PIN')
    supplied = {'CARGO_TARGET_DIR': '/target/build', 'CARGO_BUILD_JOBS': '4', 'CARGO_NET_OFFLINE': 'true',
                'RUSTUP_AUTO_INSTALL': '0', 'KNOWWEAVE_SOURCE_COMMIT': manifest['base_commit'],
                'KNOWWEAVE_BUILD_ID_SHA256': archive_sha, PIN_ENV: record['issuer']['binding_sha256']}
    if first:
        checks = ['cargo fmt --all -- --check', 'cargo clippy --locked --offline -p learning-backup --all-targets -- -D warnings']
        if mode == 'green':
            checks += ['cargo test --locked --offline -p learning-backup --lib',
                       'cargo test --locked --offline -p learning-backup --test maintenance_contract --test maintenance_journal']
        for command in checks:
            code, stdout, stderr = run_helper(helper, runner, result, image, mounts, volumes,
                                               ['/bin/sh', '-ec', command], env=supplied, timeout=7200)
            check = {'command': command, 'exit_code': code, 'stdout_sha256': digest(stdout),
                     'stderr_sha256': digest(stderr), 'classification': 'PURE_STATIC_NOT_LIVE_PG'}
            if command.startswith('cargo test'):
                check['counts'] = parse_regression(stdout.decode())
            result.setdefault('static_checks', []).append(check)
    _, output, _ = run_helper(helper, runner, result, image, mounts, volumes,
                             ['/bin/sh', '-ec', 'cargo test --locked --offline -p learning-backup --lib --no-run --message-format=json'],
                             env=supplied, timeout=7200)
    compiler_sha256 = digest(output)
    artifact = discover_binary(output.decode())
    retained = '/target/retained/' + record['identity']['case_id']
    _, output, stderr = run_helper(helper, runner, result, image, mounts, volumes,
                                  ['python3', '-c', COPY_BINARY, artifact, retained])
    require(stderr == b'', 'BINARY_COPY_STDERR')
    row = json.loads(output, object_pairs_hook=helper.unique_pairs)
    require(set(row) == {'binary_sha256'} and HEX64.fullmatch(row['binary_sha256']), 'BINARY_COPY_IDENTITY')
    record.update(binary=retained, binary_sha256=row['binary_sha256'], compiler_artifact=artifact,
                  compiler_stdout_sha256=compiler_sha256)
    _, listing, stderr = run_helper(helper, runner, result, image, mounts, volumes, [retained, '--list', '--ignored'])
    require(stderr == b'', 'DISCOVERY_STDERR')
    names = [line.removesuffix(': test') for line in listing.decode().splitlines() if line.endswith(': test')]
    expected = TESTS[:1] if mode == 'red' else TESTS
    require(all(names.count(name) == 1 for name in expected), 'IGNORED_TEST_DISCOVERY')
    record['discovered'] = list(expected)
    record['discovery_stdout_sha256'] = digest(listing)
    return retained


def audit_case(helper, runner, result, image, source, build_volume, record):
    ident = record['identity']
    mounts = case_mounts(source, build_volume, ident, write_build=False)
    volumes = {'/target': build_volume, SOURCE_ROOT: ident['source_volume'], '/var/lib/knowweave-c4/registry': ident['registry_volume']}
    _, output, stderr = run_helper(helper, runner, result, image, mounts, volumes,
                                  ['python3', '-c', AUDIT, record['binary']])
    require(stderr == b'', 'AUDIT_STDERR')
    row = json.loads(output, object_pairs_hook=helper.unique_pairs)
    binding = record['issuer']['binding']
    require(row == {'binding_sha256': record['issuer']['binding_sha256'], 'control_dev': binding['control_dev'],
                    'control_ino': binding['control_ino'], 'binary_sha256': record['binary_sha256']}, 'PIN_BINARY_OR_INODE_CHANGED')
    return row


def run_case(helper, runner, batch, source, build_volume, ident, subnet, name, images, result, manifest, archive_sha, mode, first):
    case = batch / ident['case_id']
    helper.mkdir_new(case)
    helper.mkdir_new(case / 'secrets')
    passwords = {role: secrets.token_hex(32) for role in ('postgres', 'admin', 'runtime')}
    for role, password in passwords.items():
        helper.write_new(case / 'secrets' / (role + '_password'), password.encode(), 0o400)
    helper.write_new(case / 'initdb.sh', helper.INITDB, 0o444)
    pg_env = {'POSTGRES_USER': 'postgres', 'POSTGRES_DB': 'postgres', 'POSTGRES_PASSWORD_FILE': '/run/secrets/postgres_password',
              'POSTGRES_INITDB_ARGS': '--auth-host=scram-sha-256', 'C4_DATABASE': ident['database'], 'C4_OTHER_DATABASE': ident['other_database']}
    binds = [{'type': 'bind', 'source': str(case / 'initdb.sh'), 'target': '/docker-entrypoint-initdb.d/10-admission.sh', 'read_only': True}]
    binds += [{'type': 'bind', 'source': str(case / 'secrets' / (r + '_password')), 'target': '/run/secrets/' + r + '_password', 'read_only': True} for r in passwords]
    document = {'name': ident['project'], 'services': {'pg': {'image': helper.PG_IMAGE, 'pull_policy': 'never',
                'container_name': ident['pg_name'], 'command': ['postgres', '-c', 'max_prepared_transactions=16'],
                'cpus': 2, 'mem_limit': '4g', 'memswap_limit': '4g', 'security_opt': ['no-new-privileges'],
                'environment': pg_env, 'volumes': [{'type': 'volume', 'source': 'pg', 'target': '/var/lib/postgresql', 'volume': {'nocopy': True}}, *binds],
                'networks': ['test']}}, 'networks': {'test': {'name': ident['network'], 'internal': True, 'ipam': {'config': [{'subnet': subnet}]}}},
                'volumes': {'pg': {'name': ident['volume']}}}
    helper.write_new(case / 'compose.json', canonical(document))
    record = {'identity': ident, 'subnet': subnet, 'test': name, 'stopped': False, 'stage': 'creating',
              'initdb_sha256': digest(helper.INITDB), 'classification': 'NO_DUMP_SYNTHETIC'}
    result['cases'].append(record)
    helper.fresh_resources(runner, [ident], [subnet], '')
    compose = [helper.DOCKER, 'compose', '--project-name', ident['project'], '--project-directory', str(case), '-f', str(case / 'compose.json')]
    expected = None
    try:
        result['resource_creation_unknown'] = True
        runner.run([*compose, 'create', '--no-build', '--pull', 'never'])
        facts = runner.inspect('container', ident['pg_name'])
        network, volume = runner.inspect('network', ident['network']), runner.inspect('volume', ident['volume'])
        record.update(container_id=facts['Id'], network_id=network['Id'], volume_mountpoint=volume['Mountpoint'], volume_name=volume['Name'])
        expected = {'id': facts['Id'], 'name': ident['pg_name'], 'image': images[helper.PG_IMAGE]['Id'], 'image_ref': helper.PG_IMAGE,
                    'env': {**helper.env_dict(images[helper.PG_IMAGE]['Config']['Env']), **pg_env}, 'labels': facts['Config']['Labels'],
                    'cmd': document['services']['pg']['command'], 'entrypoint': images[helper.PG_IMAGE]['Config']['Entrypoint'],
                    'mounts': {('bind', b['source'], b['target'], False) for b in binds} | {('volume', volume['Mountpoint'], '/var/lib/postgresql', True)},
                    'network': ident['network'], 'builder': False}
        require(expected['labels'].get('com.docker.compose.project') == ident['project'] and
                expected['labels'].get('com.docker.compose.service') == 'pg' and
                expected['labels'].get('com.docker.compose.project.working_dir') == str(case) and
                expected['labels'].get('com.docker.compose.project.config_files') == str(case / 'compose.json'), 'PG_COMPOSE_LABELS')
        helper.pg_facts(runner, expected, ident, subnet, record)
        result['resource_creation_unknown'] = False
        source_volume = create_volume(helper, runner, ident['source_volume'], {'com.docker.compose.project': ident['project'], 'knowweave.source-binding.batch': result['batch_id']})
        record['source_volume_mountpoint'] = source_volume['Mountpoint']
        record['registry_volume'] = create_volume(helper, runner, ident['registry_volume'], {'com.docker.compose.project': ident['project'], 'knowweave.source-binding.batch': result['batch_id']})
        runner.docker('start', expected['id'])
        deadline = time.monotonic() + 90
        while True:
            code = runner.run([helper.DOCKER, 'exec', expected['id'], 'pg_isready', '-h', '127.0.0.1', '-p', '5432', '-U', 'postgres', '-d', 'postgres'], allowed=(0, 1, 2, 3))[0]
            if code == 0:
                break
            require(time.monotonic() < deadline, 'PG_TCP_NOT_READY')
            time.sleep(0.5)
        sql = b"SELECT current_setting('server_version_num')::int / 10000, current_setting('max_prepared_transactions');\nSELECT count(*) > 0 AND bool_and(auth_method = 'scram-sha-256') FROM pg_hba_file_rules WHERE type LIKE 'host%';\nSELECT count(*) = 2 AND bool_and(rolpassword IS NULL) FROM pg_authid WHERE rolname IN ('learning_admin','learning_runtime');\n"
        observed = runner.run([helper.DOCKER, 'exec', '-i', expected['id'], 'psql', '-At', '-v', 'ON_ERROR_STOP=1', '-U', 'postgres', '-d', 'postgres'], stdin=sql)[1]
        require(observed.strip() == b'18|16\nt\nt', 'PG_VERSION_HBA_NULL_PASSWORD')
        runner.docker('exec', '--user', '0:0', expected['id'], '/bin/sh', '-ec', helper.SET_PASSWORDS)
        for db in (ident['database'], ident['other_database']):
            runner.run([helper.DOCKER, 'exec', '-i', expected['id'], 'psql', '-At', '-v', 'ON_ERROR_STOP=1', '-U', 'postgres', '-d', db],
                       stdin=b'GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO learning_admin;\n')
        helper.validate_pg_processes(runner.docker('top', expected['id'], '-eo', 'uid,pid,comm').decode())
        _, ip = helper.pg_facts(runner, expected, ident, subnet, record)
        require(ipaddress.ip_address(ip) in ipaddress.ip_network(subnet) and ip not in
                (str(ipaddress.ip_network(subnet).network_address), str(ipaddress.ip_network(subnet).broadcast_address)), 'PG_IP')
        observation_sql = b"SELECT current_database(), oid::text, (pg_catalog.pg_control_system()).system_identifier::text, current_user, pg_get_userbyid(datdba) FROM pg_catalog.pg_database WHERE datname=current_database();\n"
        sql_rows = []
        for db in (ident['database'], ident['other_database']):
            output = runner.run([helper.DOCKER, 'exec', '-i', expected['id'], 'psql', '-At', '-v', 'ON_ERROR_STOP=1', '-U', 'learning_admin', '-d', db], stdin=observation_sql)[1].decode().strip()
            parts = output.split('|')
            require(len(parts) == 5 and parts[0] == db and parts[3:] == ['learning_admin', 'learning_admin'] and
                    re.fullmatch('[1-9][0-9]*', parts[1]) and 0 < int(parts[1]) <= 2**32 - 1 and
                    re.fullmatch('[1-9][0-9]*', parts[2]) and int(parts[2]) <= 2**64 - 1, 'PG_SQL_IDENTITY')
            sql_rows.append({'database': parts[0], 'database_oid': int(parts[1]), 'system_identifier': parts[2]})
        require(sql_rows[0]['database_oid'] != sql_rows[1]['database_oid'] and
                sql_rows[0]['system_identifier'] == sql_rows[1]['system_identifier'], 'PG_OTHER_IDENTITY')
        record['sql_observations'] = sql_rows
        mounts = case_mounts(source, build_volume, ident, write_control=True)
        volumes = {'/target': build_volume, SOURCE_ROOT: ident['source_volume'], '/var/lib/knowweave-c4/registry': ident['registry_volume']}
        payload = {'registry_case':dict(batch_id=result['batch_id'],case_id=ident['case_id'],source_package_sha256=archive_sha,application_commit=manifest['base_commit'],application_build_sha256=archive_sha),'sql': sql_rows[0], 'admin_dsn': f'postgresql://learning_admin:{passwords["admin"]}@{ip}:5432/{ident["database"]}?sslmode=disable',
                   'other_admin_dsn': f'postgresql://learning_admin:{passwords["admin"]}@{ip}:5432/{ident["other_database"]}?sslmode=disable'}
        _, issued, stderr = run_helper(helper, runner, result, images[helper.BUILDER], mounts, volumes,
                                      ['python3', '-c', ISSUER], stdin=canonical(payload))
        require(stderr == b'', 'ISSUER_STDERR')
        record['issuer'] = parse_issuer(helper, issued, sql_rows[0])
        helper.write_new(case / 'issuer.json', canonical(record['issuer']))
        record['stage'] = 'building-with-independent-pin'
        binary = compile_case(helper, runner, result, images[helper.BUILDER], mounts, volumes, manifest, archive_sha, record, first, mode)
        runtime_mounts = case_mounts(source, build_volume, ident, write_build=False, write_control=True)
        record['audit_before'] = audit_case(helper, runner, result, images[helper.BUILDER], source, build_volume, record)
        task_env = {'TEST_C4_BINDING_DATABASE_NAME': ident['database'], 'TEST_C4_BINDING_OTHER_DATABASE_NAME': ident['other_database'],
                    'TEST_C4_BINDING_ADMIN_DSN_FILE': SOURCE_ROOT + '/admin.dsn', 'TEST_C4_BINDING_OTHER_ADMIN_DSN_FILE': SOURCE_ROOT + '/other-admin.dsn',
                    'TEST_C4_BINDING_CONTROL_ROOT': CONTROL}
        record['stage'] = 'executing'
        require(PIN_ENV not in task_env, 'RUNTIME_EXPECTED_PIN_FORBIDDEN')
        network_identity = {'id': record['network_id'], 'name': ident['network'], 'subnet': subnet,
                            'project': ident['project'], 'pg_id': expected['id']}
        code, stdout, stderr = run_helper(helper, runner, result, images[helper.BUILDER], runtime_mounts, volumes,
                                         [binary, '--ignored', '--exact', name, '--test-threads=1'], env=task_env,
                                         network=ident['network'], network_id=record['network_id'],
                                         network_identity=network_identity, allowed=(0, 101))
        record['outcome'] = parse_test(code, stdout.decode(), stderr, name, mode)
        record['audit_after'] = audit_case(helper, runner, result, images[helper.BUILDER], source, build_volume, record)
        if first and mode == 'green':
            regression_env = {key.replace('BINDING', 'ADMISSION'): value for key, value in task_env.items()}
            regression_env['TEST_C4_ADMISSION_CONTROL_ROOT'] = SOURCE_ROOT + '/regression'
            for regression in REGRESSIONS:
                code, stdout, stderr = run_helper(helper, runner, result, images[helper.BUILDER], runtime_mounts, volumes,
                                                 [binary, '--ignored', '--exact', regression, '--test-threads=1'], env=regression_env,
                                                 network=ident['network'], network_id=record['network_id'],
                                                 network_identity=network_identity)
                result.setdefault('live_admission_regressions', []).append(parse_test(code, stdout.decode(), stderr, regression, 'green'))
        record['stage'] = 'completed'
    finally:
        if expected is not None:
            helper.pg_facts(runner, expected, ident, subnet, record)
            runner.docker('stop', '--time', '10', expected['id'])
            facts, _ = helper.pg_facts(runner, expected, ident, subnet, record)
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
    result, batch, helper = {'status': 'FAILED', 'cleanup_verified': False, 'cases': [], 'helpers': []}, None, None
    try:
        require(sys.platform == 'linux' and os.geteuid() != 0 and Path.home() == Path('/home/hans'), 'ORDINARY_HANS_LINUX_REQUIRED')
        os.umask(0o077)
        path, archive = Path(__file__).absolute(), Path(args.archive)
        stage = path.parent
        require(stage.parent == BASE and archive.is_absolute() and archive.parent == stage and path.name == Path(ENTRY).name, 'FIXED_STAGE_REQUIRED')
        stage_id = uuid.UUID(stage.name)
        batch_id = uuid.UUID(args.batch_id)
        require(stage_id.version == batch_id.version == 4 and str(stage_id) == stage.name and str(batch_id) == args.batch_id and
                len(args.subnet) == (1 if args.mode == 'red' else 4), 'BATCH_OR_SUBNET_COUNT')
        for parent in (Path('/home/hans'), BASE, stage):
            meta = parent.lstat()
            require(stat.S_ISDIR(meta.st_mode) and meta.st_uid == os.geteuid() and not meta.st_mode & 0o022, 'PARENT_IDENTITY')
        require(stat.S_IMODE(stage.lstat().st_mode) == 0o700, 'STAGE_PRIVATE')
        helper = load_helper(stage / Path(HELPER_ENTRY).name, private=True)
        require(digest(helper.owned_file(path, (0o400, 0o500))) == args.runner_sha256, 'RUNNER_DIGEST')
        manifest, files = verify_package(helper, helper.owned_file(archive, (0o400,)), args.archive_sha256,
                                         args.manifest_sha256, args.runner_sha256, args.mode)
        subnets = helper.admit_subnets(args.subnet, [])
        batch = create_batch(helper, stage, args.batch_id)
        result.update(batch_id=args.batch_id, mode=args.mode, base_commit=manifest['base_commit'], snapshot_kind=manifest['snapshot_kind'],
                      archive_sha256=args.archive_sha256, manifest_sha256=args.manifest_sha256, runner_sha256=args.runner_sha256,
                      helper_sha256=HELPER_SHA, classification='NO_DUMP_SYNTHETIC', no_full_backup_restore=True,
                      production_root_trust=False, root_trust_scope='fresh_root_in_container_namespace')
        source = batch / 'source'
        extract_public_source(helper, source, files)
        result['source_sha256_before'] = source_digest(helper, source, manifest)
        runner = helper.Runner(batch / 'evidence/logs')
        identities = [identity() for _ in subnets]
        helper.fresh_resources(runner, identities, subnets, '')
        images = {ref: runner.inspect('image', ref) for ref in (helper.PG_IMAGE, helper.BUILDER)}
        require(images[helper.BUILDER]['Id'] == helper.BUILDER and any(ref.split('@')[-1] == helper.PG_IMAGE.split('@')[-1]
                for ref in images[helper.PG_IMAGE].get('RepoDigests', [])), 'CACHED_IMAGE_PIN')
        result['image_ids'] = {ref: image['Id'] for ref, image in images.items()}
        require(os.statvfs(batch).f_bavail * os.statvfs(batch).f_frsize >= 32 * 1024**3, 'FREE_SPACE_BUDGET')
        require(not any(PIN_ENV in helper.env_dict(image['Config']['Env']) for image in images.values()), 'UNEXPECTED_IMAGE_PIN')
        result['preflight_complete'] = True
        build_volume = 'knowweave-binding-build-' + batch_id.hex
        result['build_volume'] = create_volume(helper, runner, build_volume, {'knowweave.source-binding.batch': args.batch_id})['Name']
        for index, (ident, subnet, name) in enumerate(zip(identities, subnets, TESTS)):
            result['stage'] = name
            run_case(helper, runner, batch, source, build_volume, ident, subnet, name, images, result, manifest,
                     args.archive_sha256, args.mode, index == 0)
            require(source_digest(helper, source, manifest) == result['source_sha256_before'], 'SOURCE_CHANGED')
        # Every retained case is re-audited after the last pin recompilation.
        for record in result['cases']:
            record['audit_final'] = audit_case(helper, runner, result, images[helper.BUILDER], source, build_volume, record)
        result['source_sha256_after'] = source_digest(helper, source, manifest)
        require(all(source_digest(helper, source, manifest) == result['source_sha256_before'] for _ in (0,)), 'SOURCE_CHANGED')
        result['status'] = 'RED_EVIDENCE' if args.mode == 'red' else 'FOUR_BINDING_GATES_PASSED_NOT_FULL_BACKUP_NOT_RESTORE'
    except BaseException as error:
        reason = str(error) if isinstance(error, GateError) else 'UNEXPECTED_ERROR'
        result.setdefault('primary_reason', reason)
        result.update(status='FAILED', reason=reason)
    finally:
        if batch is not None:
            try:
                result['cleanup_verified'] = cleanup_verified(result)
                if not result['cleanup_verified']:
                    result.setdefault('primary_reason', result.get('reason', 'CLEANUP_UNKNOWN'))
                    result.update(status='FAILED', reason='CLEANUP_UNKNOWN')
                finalize_result(helper, batch, result)
            except BaseException:
                result.setdefault('primary_reason', result.get('reason', 'RESULT_PERSISTENCE_UNKNOWN'))
                result.update(status='FAILED', reason='RESULT_PERSISTENCE_UNKNOWN')
        print(canonical(result).decode())
    return 0 if result['status'] != 'FAILED' else 1


if __name__ == '__main__':
    sys.exit(main())
