"""Root-only PG18 client contract and synthetic artifact capture, never import.

Deployment requires this runner plus its fixture/archive helpers installed as
individually reviewed root-private files. All other helpers load from the sealed
archive. No captured SQL is automatically a golden or restore authorization.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import stat
import sys
import time
import uuid

# Do not execute an adjacent helper until its individual seal and archive
# member have been verified. Even --help must remain a standard-library path.
fixture = None
HEX_ID = re.compile(r'[0-9a-f]{64}\Z')


class ImportRejected(RuntimeError):
    def __init__(self, code):
        self.code = code
        super().__init__(code)


def require(value, code):
    if not value:
        raise ImportRejected(code)

BASE = Path('/var/lib/knowweave-c4')
ENTRY = 'scripts/p0c4_controlled_import_acceptance.py'
ARCHIVE_HELPER = 'scripts/p0c4_restore_birth_acceptance.py'
FIXTURE = 'scripts/p0c4_import_fixture.py'
PROVISIONER = 'scripts/p0c4_restore_target.py'
ISSUER = 'scripts/p0c4_restore_target_birth.py'
PIN = 'scripts/p0c4_restore_target_pin.py'
PREPARE = 'scripts/p0c4_restore_target_pin_prepare.py'
INITDB = 'deploy/p0c4_restore_initdb.sh'
REQUIRED = {ENTRY, ARCHIVE_HELPER, FIXTURE, PROVISIONER, ISSUER, PIN, PREPARE, INITDB}
BUILDER_ID = 'sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b'
PASSED = 'CLIENT_CONTRACT_AND_FIXTURE_CAPTURE_PASSED_NOT_IMPORT'
FAILED = 'CONTRACT_FAILED_QUARANTINED_NOT_IMPORT'
UNCONFIRMED = 'CONTRACT_FAILED_UNCONFIRMED_ISOLATION_NOT_IMPORT'
CODES = {'Identity', 'Session', 'Version', 'Protocol', 'Fixture', 'InputLimit',
         'Deadline', 'StdoutLimit', 'StderrLimit', 'Stderr', 'Exit', 'Io', 'Journal',
         'Cancelled', 'CommitUnknown', 'UnconfirmedIsolation'}
ISOLATION_SECONDS = 15
OBSERVATION_STDOUT_LIMIT = 4 * 1024 * 1024
OBSERVATION_STDERR_LIMIT = 64 * 1024


class BoundedCommands:
    """Synchronous adapter for the privately loaded provisioner's commands.

    Only pipe reads run in threads. A timed-out helper stack cannot continue
    issuing Docker actions. Isolation consumes one deadline, never a new
    timeout per resource or snapshot; reserve time to kill/reap the current CLI.
    Docker progress stderr is bounded and discarded, as in the old provisioner.
    """
    def __init__(self):
        self.isolation_deadline = None

    def __call__(self, binary, *args):
        require(binary in ('/usr/bin/docker', '/usr/sbin/ip'), 'Identity')
        now = time.monotonic()
        deadline = self.isolation_deadline if self.isolation_deadline is not None else now + 30
        if now >= deadline:
            raise fixture.ImportRejected('UnconfirmedIsolation' if self.isolation_deadline is not None else 'Deadline')
        reserve = min(0.25, (deadline - now) / 4)
        try:
            with fixture.OpenClient([binary, *args], timeout=30, deadline=deadline - reserve,
                    cleanup_deadline=deadline, stdout_limit=OBSERVATION_STDOUT_LIMIT,
                    stderr_limit=OBSERVATION_STDERR_LIMIT, allow_stderr=True,
                    env={'PATH': '/usr/sbin:/usr/bin:/bin', 'LC_ALL': 'C',
                         'DOCKER_HOST': 'unix:///var/run/docker.sock'}) as client:
                output = client.finish()
            return output.decode('utf-8')
        except fixture.ImportRejected:
            if self.isolation_deadline is not None:
                raise fixture.ImportRejected('UnconfirmedIsolation') from None
            raise
        except UnicodeError:
            raise fixture.ImportRejected('Protocol') from None


def digest(data):
    return hashlib.sha256(data).hexdigest()


def _load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    previous = sys.dont_write_bytecode
    try:
        sys.dont_write_bytecode = True
        spec.loader.exec_module(module)
    finally:
        sys.dont_write_bytecode = previous
    return module


def _canonical_batch(value):
    require(type(value) is str, 'Identity')
    try:
        parsed = uuid.UUID(value)
    except ValueError:
        raise ImportRejected('Identity') from None
    require(parsed.version == 4 and str(parsed) == value, 'Identity')
    return value


def _checked_installed(path, sha256, mode):
    require(type(sha256) is str and HEX_ID.fullmatch(sha256), 'Identity')
    require(path.is_absolute() and path == path.resolve(strict=True), 'Identity')
    for ancestor in path.parents:
        meta = os.lstat(ancestor)
        require(stat.S_ISDIR(meta.st_mode) and meta.st_uid == 0 and not meta.st_mode & 0o022, 'Identity')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(fd, 'rb') as stream:
        meta = os.fstat(stream.fileno())
        require(stat.S_ISREG(meta.st_mode) and meta.st_uid == 0 and
                stat.S_IMODE(meta.st_mode) == mode and meta.st_nlink == 1, 'Identity')
        content = stream.read(1024 * 1024 + 1)
    require(len(content) <= 1024 * 1024 and digest(content) == sha256, 'Identity')


def _archive_contract(args):
    """Reuse a separate archive-helper instance, bound to THIS runner's bytes.

    The installed helper is independently hash-checked before execution and
    checked again against the fully verified archive's required file manifest.
    This does not mutate any existing runner module or its entrypoint.
    """
    global fixture
    installed = Path(__file__).resolve(strict=True)
    _checked_installed(installed, args.runner_sha256, 0o500)
    helper_path = installed.parent / Path(ARCHIVE_HELPER).name
    _checked_installed(helper_path, args.archive_helper_sha256, 0o400)
    fixture_path = installed.parent / Path(FIXTURE).name
    _checked_installed(fixture_path, args.fixture_sha256, 0o400)
    helper = _load(helper_path, 'p0c4_import_archive_helper_private')
    helper.ENTRY, helper.REQUIRED, helper.__file__ = ENTRY, REQUIRED, str(installed)
    manifest, content = helper.verify_archive(args.archive, args.archive_sha256,
                                              args.manifest_sha256, args.commit)
    files = {item['path']: item['sha256'] for item in manifest['files']}
    require(files[ENTRY] == args.runner_sha256 and
            files[ARCHIVE_HELPER] == args.archive_helper_sha256 and
            files[FIXTURE] == args.fixture_sha256, 'Identity')
    fixture = _load(fixture_path, 'p0c4_import_fixture_sealed')
    return helper, manifest, content


class ContractBackend:
    def __init__(self, args, helper, manifest, content):
        self.args, self.helper, self.manifest = args, helper, manifest
        root = BASE / 'controlled-import'
        helper._require_private_dir(BASE)
        if not os.path.lexists(root):
            helper._private_dir(root)
        helper._require_private_dir(root)
        self.lock = helper._acceptance_lock(root)
        self.lock.__enter__()
        try:
            batches = root / 'batches'
            if not os.path.lexists(batches):
                helper._private_dir(batches)
            helper._require_private_dir(batches)
            self.batch = batches / args.batch_id
            helper._private_dir(self.batch)  # O_EXCL-style: any prior batch refuses.
            for name in ('evidence', 'artifacts', 'tmp', 'source-control', 'target-control'):
                helper._private_dir(self.batch / name)
            self.source = self.batch / 'source'
            self.source_hash = helper.extract_verified(content, manifest, self.source)
            self.provisioner, self.issuer = helper._load_reviewed(self.source)
            self.commands = BoundedCommands()
            self.provisioner._command = self.commands
            names = ('p0c4_restore_target', 'p0c4_restore_target_birth',
                     'p0c4_restore_birth_acceptance', 'p0c4_restore_target_pin')
            previous = {name: sys.modules.get(name) for name in names}
            try:
                sys.modules['p0c4_restore_target'] = self.provisioner
                sys.modules['p0c4_restore_target_birth'] = self.issuer
                sys.modules['p0c4_restore_birth_acceptance'] = helper
                self.pin = _load(self.source / PIN, 'p0c4_restore_target_pin')
                sys.modules['p0c4_restore_target_pin'] = self.pin
                self.prepare = _load(self.source / PREPARE, 'p0c4_import_prepare_reviewed')
            finally:
                for name, old in previous.items():
                    if old is None:
                        sys.modules.pop(name, None)
                    else:
                        sys.modules[name] = old
            self.initdb = self.batch / 'initdb.sh'
            helper._private_write(self.initdb, (self.source / INITDB).read_bytes(), 0o444)
            self.resources = []
            self.old_temp = {name: os.environ.get(name) for name in ('TMP', 'TEMP', 'TMPDIR')}
            for name in self.old_temp:
                os.environ[name] = str(self.batch / 'tmp')
        except BaseException:
            self.lock.__exit__(None, None, None)
            raise

    def _budget(self):
        disk = os.statvfs(self.batch)
        require(disk.f_bavail * disk.f_frsize >= 2 * 1024**3, 'InputLimit')
        memory = Path('/proc/meminfo').read_text(encoding='ascii')
        match = re.search(r'^MemAvailable:\s+([0-9]+) kB$', memory, re.MULTILINE)
        require(match and int(match[1]) >= 1024**2, 'InputLimit')
        image = fixture.run_fixed(['/usr/bin/docker', 'image', 'inspect', '--format', '{{.Id}}', BUILDER_ID])
        require(image == (BUILDER_ID + '\n').encode(), 'Identity')

    def _create(self, role, batch_id, subnet):
        identity = self.provisioner.identity_for(batch_id)
        control = self.batch / (role + '-control')
        self.helper._private_dir(control / 'targets')
        record = {'role': role, 'identity': identity, 'control': control,
                  'batch_id': batch_id, 'subnet': subnet, 'container_id': None,
                  'before': self.provisioner.snapshot()}
        self.resources.append(record)
        self.prepare.prepare(control, batch_id, subnet, self.initdb)
        state = self.provisioner.provision(control, batch_id, subnet, self.initdb,
                                           _birth_issuer=self.issuer.issue_birth)
        record['container_id'] = state['container_id']
        _, inspection = self.pin.inspect_candidate(control, batch_id, self.initdb)
        record['inspection_sha256'] = digest(self.helper._json_bytes(inspection))
        return record

    def _client_contract(self, record):
        cid, db = record['container_id'], record['identity']['database']
        fixture.validate_identity(cid, db)
        # A char-device parent cannot contain a passfile; never suppress stderr.
        require(fixture.run_fixed(fixture.client_prefix(cid) + ['/usr/bin/test', '-c', '/dev/null']) == b'', 'Identity')
        for client in ('psql', 'pg_restore', 'pg_dump'):
            output = fixture.run_fixed(fixture.client_prefix(cid) + [f'/usr/lib/postgresql/18/bin/{client}', '--version'])
            require(re.fullmatch((client + r' \(PostgreSQL\) 18\.6 \(Debian 18\.6-[1-9][0-9]{0,5}\.pgdg12\+[1-9][0-9]{0,5}\)\n').encode(), output), 'Version')
        # Fixed privileged local observation records actual local HBA methods.
        privileged = fixture.writer_command(cid, db)
        privileged[privileged.index('--username=learning_admin')] = '--username=postgres'
        privileged[privileged.index('--dbname=' + db)] = '--dbname=postgres'
        hba = fixture.run_fixed(privileged, b"SELECT COALESCE(json_agg(json_build_object('type',type,'method',auth_method,'error',error) ORDER BY rule_number),'[]'::json) FROM pg_catalog.pg_hba_file_rules WHERE type='local';\n", timeout=45, stdout_limit=fixture.MAX_WRITER)
        try:
            local = json.loads(hba)
        except (ValueError, UnicodeError):
            raise ImportRejected('Protocol') from None
        require(type(local) is list and local and all(type(row) is dict and
            set(row) == {'type', 'method', 'error'} and row == {'type': 'local', 'method': 'trust', 'error': None}
            for row in local), 'Identity')
        command = fixture.writer_command(cid, db)
        facts = fixture.run_handshake(command, secrets.token_hex(16))
        # Target EOF with no DDL is a separate protocol fact, not DDL rollback.
        nonce = secrets.token_hex(16)
        with fixture.OpenClient(command) as process:
            process.send(b'BEGIN READ ONLY; ' + fixture.TIMEOUTS.replace(b'\n', b' ') + fixture.receipt_sql('READY', nonce))
            require(fixture.parse_writer_line(process.line(), nonce)[0] == 'READY', 'Protocol')
            process.still_open()
            require(process.finish() == b'', 'Protocol')
        return {**facts, 'no_ddl_eof_exit': True, 'dev_null_character_device': True,
                'disabled_passfile_stderr_empty': True, 'local_hba': local}

    def execute(self):
        self._budget()
        before = self.provisioner.snapshot()
        source_identity = self.provisioner.identity_for(self.args.source_batch_id)
        target_identity = self.provisioner.identity_for(self.args.target_batch_id)
        # Admit the whole pair before creating anything.
        self.provisioner.admit_fresh(source_identity, self.args.source_subnet, before)
        self.provisioner.admit_fresh(target_identity, self.args.target_subnet, before)
        import ipaddress
        require(not ipaddress.ip_network(self.args.source_subnet).overlaps(
            ipaddress.ip_network(self.args.target_subnet)), 'Identity')
        source = self._create('source', self.args.source_batch_id, self.args.source_subnet)
        target = self._create('target', self.args.target_batch_id, self.args.target_subnet)
        clients = {row['role']: self._client_contract(row) for row in (source, target)}
        captured = fixture.capture_fixture(source['container_id'], source_identity['database'])
        artifacts = {}
        for key, name in [('dump', 'fixture.dump'), ('sql', 'decoded.sql')]:
            payload = captured.pop(key)
            path = self.batch / 'artifacts' / name
            self.helper._private_write(path, payload, 0o600)
            # Freeze and verify the same no-follow handle bytes after publication.
            frozen = self.helper._private_read(path, limit=fixture.MAX_INPUT)
            require(frozen == payload and (key != 'dump' or frozen.startswith(b'PGDMP')), 'Fixture')
            artifacts[key + '_sha256'] = digest(frozen)
            artifacts[key + '_size'] = len(frozen)
        # Independent target catalog must remain the fresh baseline.
        self.issuer.probe_pg_facts(target_identity, target['container_id'])
        for row in (source, target):
            # Source now has fixture DDL: use exact Docker reinspection, not clean-catalog inspection.
            live = self.provisioner.snapshot()
            live['images'] = self.provisioner._inspect('image', [row['identity']['image']])
            ids = self.provisioner.verify_created(row['identity'], row['subnet'], row['before'], live)
            require(ids['container_id'] == row['container_id'], 'Identity')
        return {'clients': clients, 'fixture': captured, 'artifacts': artifacts,
                'target_baseline_unchanged': True, 'not_golden': True}

    def stop(self):
        stopped = {}
        failure = False
        deadline = time.monotonic() + ISOLATION_SECONDS
        self.commands.isolation_deadline = deadline
        for row in self.resources:
            require(time.monotonic() < deadline, 'UnconfirmedIsolation')
            try:
                if row['container_id'] is not None:
                    fact = self.helper.stop_verified_pg(self.provisioner, row['identity'], row['container_id'])
                else:
                    fact = self.helper.stop_early_owned_pg(self.provisioner, row['identity'],
                        row['control'] / 'targets' / row['batch_id'], row['subnet'], row['before'], self.initdb)
                stopped[row['role'] + '_stopped'] = fact.get('confirmed') is True
                require(stopped[row['role'] + '_stopped'], 'UnconfirmedIsolation')
            except BaseException as error:
                failure = True
                if isinstance(error, fixture.ImportRejected) and error.code == 'UnconfirmedIsolation':
                    break  # Current CLI is reaped; never resume a timed-out action stack.
        require(not failure and time.monotonic() < deadline, 'UnconfirmedIsolation')
        return {**stopped, 'volumes_retained': True}

    def verify_source(self):
        return self.helper.source_digest(self.source, self.manifest) == self.source_hash

    def evidence_identity(self):
        # Values are established by archive validation, canonical batch
        # admission and verified creation/inspection, never raw command output.
        package = {name: getattr(self.args, name) for name in (
            'commit', 'archive_sha256', 'manifest_sha256', 'runner_sha256',
            'archive_helper_sha256', 'fixture_sha256')}
        package['source_sha256'] = self.source_hash
        batches = {'control': self.args.batch_id, 'source': self.args.source_batch_id,
                   'target': self.args.target_batch_id}
        resources = {row['role']: {
            'container_id': row['container_id'],
            'inspection_sha256': row.get('inspection_sha256'),
            'database': row['identity']['database'], 'project': row['identity']['project'],
            'volume': row['identity']['volume'], 'subnet': row['subnet']}
            for row in self.resources}
        return {'package': package, 'batches': batches, 'resources': resources}

    def publish(self, result):
        evidence = self.batch / 'evidence'
        pending = evidence / 'result.pending'
        final = evidence / 'result.json'
        require(not os.path.lexists(pending) and not os.path.lexists(final), 'Journal')
        payload = self.helper._json_bytes(result)
        self.helper._private_write(pending, payload)
        # Lock guarantees no cooperating publisher. Hardlink avoids replacement
        # if an unexpected existing final appears; both names remain on failure.
        os.link(pending, final, follow_symlinks=False)
        self.helper._sync_dir(evidence)
        os.unlink(pending)
        self.helper._sync_dir(evidence)
        require(not os.path.lexists(pending), 'Journal')
        return {**result, 'result_sha256': digest(payload), 'evidence': str(final)}

    def release(self):
        for name, old in self.old_temp.items():
            if old is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = old
        self.lock.__exit__(None, None, None)


def _prepare_backend(args):
    require(sys.platform == 'linux' and os.geteuid() == 0, 'Identity')
    batches = [_canonical_batch(getattr(args, name)) for name in
               ('batch_id', 'source_batch_id', 'target_batch_id')]
    require(len(set(batches)) == 3, 'Identity')
    helper, manifest, content = _archive_contract(args)
    return ContractBackend(args, helper, manifest, content)


def run_contract(args) -> dict:
    require(getattr(args, 'phase', None) == 'contract', 'Protocol')
    source_id, target_id = getattr(args, 'source_batch_id', None), getattr(args, 'target_batch_id', None)
    if source_id is not None or target_id is not None:
        require(type(source_id) is str and type(target_id) is str and source_id != target_id, 'Identity')
        _canonical_batch(source_id)
        _canonical_batch(target_id)
    backend = _prepare_backend(args)
    result = {'status': FAILED, 'reason_code': None, 'not_import': True, 'not_golden': True}
    try:
        try:
            result.update(backend.execute())
            result['status'] = PASSED
        except BaseException as error:
            known = isinstance(error, ImportRejected) or (
                fixture is not None and isinstance(error, fixture.ImportRejected))
            result['reason_code'] = error.code if known and error.code in CODES else 'Io'
        try:
            result['isolation'] = backend.stop()
        except BaseException:
            result['status'], result['reason_code'] = UNCONFIRMED, 'UnconfirmedIsolation'
        try:
            require(backend.verify_source(), 'Identity')
            result['source_unchanged'] = True
        except BaseException:
            if result['status'] != UNCONFIRMED:
                result['status'], result['reason_code'] = FAILED, 'Identity'
        result['identity'] = backend.evidence_identity()
        return backend.publish(result)
    finally:
        if hasattr(backend, 'release'):
            backend.release()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--phase', choices=['contract'], required=True)
    parser.add_argument('--archive', type=Path, required=True)
    for name in ('archive-sha256', 'manifest-sha256', 'commit', 'runner-sha256',
                 'archive-helper-sha256', 'fixture-sha256', 'batch-id',
                 'source-batch-id', 'target-batch-id', 'source-subnet', 'target-subnet'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args(argv)
    try:
        result = run_contract(args)
        print(json.dumps(result, separators=(',', ':')), flush=True)
        return 0 if result['status'] == PASSED else 1
    except BaseException:
        print('CONTRACT_ADMISSION_OR_EVIDENCE_REJECTED_NOT_IMPORT', flush=True)
        return 1


if __name__ == '__main__':
    sys.dont_write_bytecode = True
    raise SystemExit(main())
