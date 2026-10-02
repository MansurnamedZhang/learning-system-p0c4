"""Root-only PG18 contract capture and opt-in controlled fixture import cases.

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
CLONE_HELPER = 'scripts/p0c4_restore_pin_acceptance.py'
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
INSPECT_OPERATION_SECONDS = 30
INSPECT_BATCH_SIZE = 64
INSPECT_MAX_OBJECTS = 4096
INSPECT_TOTAL_STDOUT_LIMIT = 16 * 1024 * 1024

# Closed opt-in import protocol. These are predicates over actual live receipts;
# the mapping itself never grants fixture provenance or target authority.
IMPORT_PREFIX = 'restore_preflight::target_binding::controlled_import::live_tests::'
IMPORT_PASSED = 'CONTROLLED_FIXTURE_IMPORT_PASSED_SINGLE_HOST_QUARANTINED_NOT_FULL_RESTORE'
IMPORT_NEGATIVE = 'CONTROLLED_FIXTURE_IMPORT_EXPECTED_REJECTION_SINGLE_HOST_QUARANTINED_NOT_FULL_RESTORE'
IMPORT_FAILED = 'CONTROLLED_FIXTURE_IMPORT_FAILED_QUARANTINED_NOT_FULL_RESTORE'
IMPORT_UNCONFIRMED = 'CONTROLLED_FIXTURE_IMPORT_UNCONFIRMED_UNUSABLE_NOT_FULL_RESTORE'
POSTGRES_IMAGE = 'postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d'
GATE1_SHA256 = '505ab6179e3be1f1c8afb441fa694db8d5033f27e77a0940908de208bad96508'
IMPORT_CASES = {
    'success': ('live_controlled_import_commits_fixture', None, (
        'READY', 'ATTEMPT_SYNCED', 'PAYLOAD_SENT', 'PRECOMMIT', 'INTENT_SYNCED',
        'COMMIT_ONCE', 'COMMITTED', 'WRITER_GONE', 'CONTENT_ROWS_PK_DIGEST')),
    'ready-eof': ('live_controlled_import_ready_eof', 'Protocol', (
        'READY', 'TRUE_EOF_NO_TRANSACTION_COMMAND', 'WRITER_GONE',
        'ZERO_OBJECTS_BEFORE_STOP', 'ATTEMPT_ABSENT', 'INTENT_ABSENT')),
    'precommit-eof': ('live_controlled_import_precommit_eof', 'Protocol', (
        'READY', 'ATTEMPT_SYNCED', 'PAYLOAD_SENT', 'PRECOMMIT',
        'TRUE_EOF_NO_TRANSACTION_COMMAND', 'WRITER_GONE',
        'ZERO_OBJECTS_BEFORE_STOP', 'INTENT_ABSENT')),
    'precommit-cancel': ('live_controlled_import_cancel_before_commit', 'Cancelled', (
        'READY', 'ATTEMPT_SYNCED', 'PAYLOAD_SENT', 'PRECOMMIT',
        'CANCEL_ACCEPTED_BEFORE_COMMIT', 'GUARDS_RETAINED', 'WRITER_GONE',
        'ZERO_OBJECTS_BEFORE_STOP', 'INTENT_ABSENT')),
    'ready-restart': ('live_controlled_import_restart_before_ddl', 'Identity', (
        'READY', 'RESTART_BEFORE_DDL', 'OLD_GUARD_REJECTED',
        'ATTEMPT_ABSENT', 'INTENT_ABSENT', 'DDL_NOT_SENT')),
    'sql-error': ('live_controlled_import_sql_error', 'Exit', (
        'READY', 'ATTEMPT_SYNCED', 'FIXED_SQL_ERROR_OBSERVED',
        'COMMIT_NOT_SENT', 'INTENT_ABSENT')),
    'copy-truncated': ('live_controlled_import_copy_truncated', 'Exit', (
        'READY', 'ATTEMPT_SYNCED', 'FIXED_COPY_TRUNCATED',
        'NO_BLIND_ROLLBACK', 'COMMIT_NOT_SENT', 'INTENT_ABSENT')),
    'attempt-sync-failure': ('live_controlled_import_attempt_sync_failure', 'Journal', (
        'READY', 'ATTEMPT_SYNC_FAILED', 'ATTEMPT_PARTIAL_RETAINED',
        'DDL_NOT_SENT', 'COMMIT_NOT_SENT', 'INTENT_ABSENT')),
    'commit-intent-sync-failure': ('live_controlled_import_commit_intent_sync_failure', 'Journal', (
        'READY', 'ATTEMPT_SYNCED', 'PAYLOAD_SENT', 'PRECOMMIT',
        'INTENT_SYNC_FAILED', 'INTENT_PARTIAL_RETAINED', 'COMMIT_NOT_SENT')),
    'commit-unknown': ('live_controlled_import_commit_confirmation_lost', 'CommitUnknown', (
        'READY', 'ATTEMPT_SYNCED', 'PAYLOAD_SENT', 'PRECOMMIT', 'INTENT_SYNCED',
        'COMMIT_ONCE', 'CONFIRMATION_LOST', 'OUTCOME_UNKNOWN_NO_RETRY')),
    'wrong-endpoint': ('live_controlled_import_same_id_wrong_endpoint', 'Exit', (
        'CLONE_SAME_SYSTEM_ID_DATABASE_OID', 'WRITER_DOUBLE_LOCK_REJECTED',
        'ZERO_OBJECTS_BOTH_BEFORE_STOP', 'ATTEMPT_ABSENT', 'INTENT_ABSENT',
        'DDL_NOT_SENT', 'BOTH_EXACT_CONTAINERS_STOPPED')),
}
IMPORT_COMMON_CHECKPOINTS = ('FRESH_SOURCE_FIXED_PRODUCER', 'SNAPSHOT_NOFOLLOW_FROZEN',
    'REAL_TOC_VERIFIED', 'FULL_GOLDEN_VERIFIED', 'TARGET_UNUSABLE', 'EXACT_TARGET_STOPPED')


def _import_test_command(binary, case):
    require(type(case) is str and case in IMPORT_CASES, 'Protocol')
    return [str(binary), '--exact', IMPORT_PREFIX + IMPORT_CASES[case][0],
            '--ignored', '--nocapture', '--test-threads=1']


# Diagnostics are evidence about rejection only. Never an Observation or gate.
IMPORT_DIAGNOSTIC_TAG = b'KW_C4_IMPORT_DIAGNOSTIC|'
IMPORT_DIAGNOSTIC_PHASES = {'authority-case-env', 'source-config', 'target-config',
    'artifact-root-env', 'fresh-fixture-capture', 'acquire-target-guard',
    'target-control-connection', 'candidate-admission',
    'live-toc', 'live-clone', 'candidate-report', 'evidence-unwrap', 'harness-join'}
IMPORT_CANDIDATE_PHASES = {'InputFrozen', 'SqlVerified', 'WriterReady', 'AttemptDurable',
    'PayloadSent', 'PrecommitVerified', 'CommitAttempted', 'ImportObserved',
    'Quarantined', 'Cancelled', 'CommitUnknown'}
IMPORT_DIAGNOSTIC_REASONS = CODES | {'JoinPanic', 'JoinCancelled', 'JoinUnknown',
                                    'EvidenceShared', 'EvidencePoisoned'}


def _import_failure_diagnostic(stderr, case):
    """Closed, bounded projection. Malformed events cannot interrupt cleanup."""
    absent = {'diagnostic_status': 'ABSENT', 'diagnostic': None}
    rejected = {'diagnostic_status': 'REJECTED', 'diagnostic': None}
    if type(stderr) is not bytes or len(stderr) > 64 * 1024 or case not in IMPORT_CASES:
        return rejected
    if IMPORT_DIAGNOSTIC_TAG not in stderr:
        return absent
    if stderr.count(IMPORT_DIAGNOSTIC_TAG) != 1:
        return rejected
    lines = [line for line in stderr.split(b'\n') if IMPORT_DIAGNOSTIC_TAG in line]
    if len(lines) != 1 or not lines[0].startswith(IMPORT_DIAGNOSTIC_TAG) or len(lines[0]) > 1024:
        return rejected
    def unique_pairs(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                raise ValueError('duplicate diagnostic key')
            value[key] = item
        return value
    try:
        value = json.loads(lines[0][len(IMPORT_DIAGNOSTIC_TAG):].decode('ascii'),
                           object_pairs_hook=unique_pairs)
        if (type(value) is not dict or set(value) != {'schema', 'case', 'phase', 'reason', 'candidate'}
                or type(value['schema']) is not int or value['schema'] != 1
                or type(value['case']) is not str or value['case'] != case
                or type(value['phase']) is not str or value['phase'] not in IMPORT_DIAGNOSTIC_PHASES
                or type(value['reason']) is not str or value['reason'] not in IMPORT_DIAGNOSTIC_REASONS):
            return rejected
        candidate = value['candidate']
        if candidate is not None:
            if (type(candidate) is not dict or set(candidate) != {'phase', 'failure',
                    'stop_confirmed', 'content_verified', 'commit_attempted'}
                    or type(candidate['phase']) is not str or candidate['phase'] not in IMPORT_CANDIDATE_PHASES
                    or (candidate['failure'] is not None and
                        (type(candidate['failure']) is not str or candidate['failure'] not in CODES))
                    or any(type(candidate[key]) is not bool for key in
                           ('stop_confirmed', 'content_verified', 'commit_attempted'))):
                return rejected
        # No raw stderr, unrecognized keys, error text, or private data survives.
        return {'diagnostic_status': 'PRESENT', 'diagnostic': value}
    except (ValueError, UnicodeError, RecursionError):
        return rejected


def _import_failure_process(process, case):
    expected = IMPORT_PREFIX + IMPORT_CASES[case][0]
    observed = None
    if (type(process.stdout) is bytes and len(process.stdout) <= 64 * 1024
            and re.search(rb'(?m)^test ' + re.escape(expected.encode('ascii')) + rb' \.\.\. ', process.stdout)):
        observed = expected
    return {'exact_test': observed,
            'exact_test_exit_code': process.returncode if type(process.returncode) is int else None,
            **_import_failure_diagnostic(process.stderr, case)}


def _import_observations(output, case, exit_code):
    require(type(case) is str and case in IMPORT_CASES, 'Protocol')
    require(type(exit_code) is int and exit_code == 0 and type(output) is bytes and
            len(output) <= 64 * 1024, 'Protocol')
    try:
        text = output.decode('ascii')
    except UnicodeError:
        raise ImportRejected('Protocol') from None
    name, failure, checkpoints = IMPORT_CASES[case]
    # One exact --nocapture transcript: the sole test emits one redacted receipt.
    # Do not accept a success summary attached to failed/additional test lines.
    require(re.fullmatch(r'\nrunning 1 test\ntest ' +
        re.escape(IMPORT_PREFIX + name) + r' \.\.\. KW_C4_IMPORT\|[^\n]+\nok\n\n'
        r'test result: [^\n]+\n\n?', text), 'Protocol')
    summaries = re.findall(r'^test result: (.+)$', text, re.M)
    require(len(summaries) == 1 and re.fullmatch(
        r'ok\. 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; '
        r'finished in [0-9]+\.[0-9]+s', summaries[0]), 'Protocol')
    require(text.count('KW_C4_IMPORT|') == 1, 'Protocol')
    raw = text.split('KW_C4_IMPORT|', 1)[1].split('\n', 1)[0]
    def unique_pairs(pairs):
        value = {}
        for key, item in pairs:
            require(key not in value, 'Protocol')
            value[key] = item
        return value
    try:
        record = json.loads(raw, object_pairs_hook=unique_pairs,
            parse_constant=lambda value: require(False, 'Protocol'))
    except (ValueError, RecursionError):
        raise ImportRejected('Protocol') from None
    required = {'schema', 'case', 'failure', 'checkpoints', 'commit_attempted',
                'rollback_verified', 'retry_allowed', 'dump_sha256', 'toc_sha256',
                'raw_sql_sha256', 'transformed_sql_sha256', 'content_sha256'}
    require(type(record) is dict and set(record) == required, 'Protocol')
    require(type(record['schema']) is int and record['schema'] == 1 and
            record['case'] == case and record['failure'] == failure, 'Protocol')
    required_points = {*IMPORT_COMMON_CHECKPOINTS, *checkpoints}
    points = record['checkpoints']
    require(type(points) is list and all(type(point) is str for point in points) and
            len(points) == len(required_points) and set(points) == required_points,
            'Protocol')
    require(record['commit_attempted'] is (case in ('success', 'commit-unknown')) and
            record['retry_allowed'] is False, 'Protocol')
    require(record['rollback_verified'] is (case in
        ('ready-eof', 'precommit-eof', 'precommit-cancel')), 'Protocol')
    for key in ('dump_sha256', 'toc_sha256', 'raw_sql_sha256', 'transformed_sql_sha256'):
        require(type(record[key]) is str and HEX_ID.fullmatch(record[key]), 'Protocol')
    require(record['content_sha256'] == (digest(b'1|alpha\n2|beta\n') if case == 'success' else None), 'Protocol')
    return record


def _validate_import_isolation(result):
    require(type(result) is dict and result.get('case') in IMPORT_CASES, 'Protocol')
    isolation = result.get('isolation', {})
    require(type(isolation) is dict and isolation.get('source_stopped') is True and
            isolation.get('target_stopped') is True and
            isolation.get('volumes_retained') is True, 'UnconfirmedIsolation')
    if result['case'] == 'wrong-endpoint':
        require(isolation.get('clone_stopped') is True, 'UnconfirmedIsolation')


def _validate_import_final(result):
    _validate_import_isolation(result)
    require(result.get('source_unchanged') is True, 'Identity')
    require(result.get('pending_absent') is True, 'Journal')


class BoundedCommands:
    """Synchronous adapter for the privately loaded provisioner's commands.

    Only pipe reads run in threads. A timed-out helper stack cannot continue
    issuing Docker actions. Isolation consumes one deadline, never a new
    timeout per resource or snapshot; reserve time to kill/reap the current CLI.
    Docker progress stderr is bounded and discarded, as in the old provisioner.
    """
    def __init__(self):
        self.isolation_deadline = None
        self.unusable = False

    def require_usable(self):
        if self.unusable or fixture._UNSETTLED_CLIENTS:
            self.unusable = True
            raise fixture.ImportRejected('UnconfirmedIsolation')

    def __call__(self, binary, *args):
        return self._run(binary, args)

    def _run(self, binary, args, operation_deadline=None):
        self.require_usable()
        require(binary in ('/usr/bin/docker', '/usr/sbin/ip'), 'Identity')
        now = time.monotonic()
        deadline = now + 30
        if operation_deadline is not None:
            deadline = min(deadline, operation_deadline)
        if self.isolation_deadline is not None:
            deadline = min(deadline, self.isolation_deadline)
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
        except fixture.ImportRejected as error:
            if getattr(error, 'cleanup_owner', None) is not None:
                self.unusable = True
            if self.isolation_deadline is not None:
                raise fixture.ImportRejected('UnconfirmedIsolation') from None
            raise
        except UnicodeError:
            raise fixture.ImportRejected('Protocol') from None

    def inspect(self, kind, ids):
        require(kind in ('container', 'network', 'volume', 'image'), 'Identity')
        require(type(ids) in (list, tuple), 'Identity')
        require(len(ids) <= INSPECT_MAX_OBJECTS, 'InputLimit')
        require(all(type(item) is str and item and len(item) <= 255 and
                    not item.startswith('-') and not any(char.isspace() or char == '\x00'
                    for char in item) for item in ids), 'Identity')
        if not ids:
            return []
        deadline = time.monotonic() + INSPECT_OPERATION_SECONDS
        aggregate = 0
        inspected = []
        for offset in range(0, len(ids), INSPECT_BATCH_SIZE):
            batch = ids[offset:offset + INSPECT_BATCH_SIZE]
            args = ('inspect', *batch) if kind == 'container' else (kind, 'inspect', *batch)
            output = self._run('/usr/bin/docker', args, deadline)
            aggregate += len(output.encode('utf-8'))
            require(aggregate <= INSPECT_TOTAL_STDOUT_LIMIT, 'StdoutLimit')
            try:
                members = json.loads(output, parse_constant=lambda value: require(False, 'Protocol'))
            except (ValueError, RecursionError):
                raise fixture.ImportRejected('Protocol') from None
            require(type(members) is list and len(members) == len(batch) and
                    all(type(item) is dict for item in members), 'Protocol')
            inspected.extend(members)
            effective_deadline = deadline
            if self.isolation_deadline is not None:
                effective_deadline = min(deadline, self.isolation_deadline)
            require(time.monotonic() < effective_deadline, 'UnconfirmedIsolation'
                    if self.isolation_deadline is not None else 'Deadline')
        return inspected


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
    require(fixture is None or not fixture._UNSETTLED_CLIENTS, 'UnconfirmedIsolation')
    installed = Path(__file__).resolve(strict=True)
    _checked_installed(installed, args.runner_sha256, 0o500)
    helper_path = installed.parent / Path(ARCHIVE_HELPER).name
    _checked_installed(helper_path, args.archive_helper_sha256, 0o400)
    fixture_path = installed.parent / Path(FIXTURE).name
    _checked_installed(fixture_path, args.fixture_sha256, 0o400)
    helper = _load(helper_path, 'p0c4_import_archive_helper_private')
    helper.ENTRY, helper.REQUIRED, helper.__file__ = ENTRY, REQUIRED | (
        {CLONE_HELPER} if getattr(args, 'phase', None) == 'import' else set()), str(installed)
    manifest, content = helper.verify_archive(args.archive, args.archive_sha256,
                                              args.manifest_sha256, args.commit)
    files = {item['path']: item['sha256'] for item in manifest['files']}
    require(files[ENTRY] == args.runner_sha256 and
            files[ARCHIVE_HELPER] == args.archive_helper_sha256 and
            files[FIXTURE] == args.fixture_sha256, 'Identity')
    if getattr(args, 'phase', None) == 'import':
        require(files[CLONE_HELPER] == args.clone_helper_sha256, 'Identity')
    fixture = _load(fixture_path, 'p0c4_import_fixture_sealed')
    return helper, manifest, content


_UNSETTLED_BACKENDS = set()


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
            self.provisioner._inspect = self.commands.inspect
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

    def _isolation_deadline(self):
        if self.commands.isolation_deadline is None:
            self.commands.isolation_deadline = time.monotonic() + ISOLATION_SECONDS
        return self.commands.isolation_deadline

    def stop(self):
        stopped = {}
        failure = False
        deadline = self._isolation_deadline()
        for row in self.resources:
            require(time.monotonic() < deadline, 'UnconfirmedIsolation')
            try:
                self.commands.require_usable()
                if row['container_id'] is not None:
                    fact = self.helper.stop_verified_pg(self.provisioner, row['identity'], row['container_id'])
                elif row['role'] != 'clone':
                    fact = self.helper.stop_early_owned_pg(self.provisioner, row['identity'],
                        row['control'] / 'targets' / row['batch_id'], row['subnet'], row['before'], self.initdb)
                else:
                    raise ImportRejected('UnconfirmedIsolation')
                stopped[row['role'] + '_stopped'] = fact.get('confirmed') is True
                require(stopped[row['role'] + '_stopped'], 'UnconfirmedIsolation')
            except BaseException as error:
                failure = True
                if self.commands.unusable or time.monotonic() >= deadline:
                    break  # Never resume an unsettled or expired action stack.
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
        require(not fixture._UNSETTLED_CLIENTS and not self.commands.unusable, 'UnconfirmedIsolation')
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
        if fixture is not None and fixture._UNSETTLED_CLIENTS:
            _UNSETTLED_BACKENDS.add(self)
            raise ImportRejected('UnconfirmedIsolation')
        _UNSETTLED_BACKENDS.discard(self)
        for name, old in self.old_temp.items():
            if old is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = old
        self.lock.__exit__(None, None, None)


class ImportBackend(ContractBackend):
    """New phase only; the privately loaded legacy helpers retain their modes."""
    def __init__(self, args, helper, manifest, content):
        super().__init__(args, helper, manifest, content)
        try:
            self.delegate = _load(self.source / CLONE_HELPER, 'p0c4_import_clone_helpers_private')
            self.clone_record = None
            self.build_evidence = {}
        except BaseException:
            self.release()
            raise

    def _budget(self):
        super()._budget()
        free = os.statvfs(self.batch)
        available = free.f_bavail * free.f_frsize
        memory = re.search(r'^MemAvailable:\s+([0-9]+) kB$',
                           Path('/proc/meminfo').read_text(encoding='ascii'), re.M)
        pg_count = 3 if self.args.case == 'wrong-endpoint' else 2
        disk_budget = (32 + 2 * pg_count + 2) * 1024**3
        memory_budget = (8 + 4 * pg_count + 1) * 1024**3
        require(available >= disk_budget and memory and int(memory[1]) * 1024 >= memory_budget,
                'InputLimit')
        self.capacity = {'available_disk_bytes': available,
            'available_memory_bytes': int(memory[1]) * 1024,
            'declared_build_disk_bytes_each': 16 * 1024**3, 'build_directories': 2,
            'declared_build_memory_bytes': 8 * 1024**3, 'builder_cpus': 4,
            'required_disk_bytes': disk_budget, 'required_memory_bytes': memory_budget,
            'disk_budget_is_estimated_reserve_not_quota': True,
            'declared_pg_disk_bytes_each': 2 * 1024**3,
            'declared_pg_instances': pg_count,
            'builder_image_id': BUILDER_ID, 'postgres_image': POSTGRES_IMAGE}

    def _compile_import_binary(self, source_pin, target_pin, *, placeholder):
        import subprocess
        require(all(type(pin) is str and HEX_ID.fullmatch(pin) for pin in (source_pin, target_pin)), 'Identity')
        require((source_pin == target_pin == '0' * 64) if placeholder else
                source_pin != target_pin and '0' * 64 not in (source_pin, target_pin), 'Identity')
        original = self.delegate._run_bounded
        original_cleanup = self.delegate._cleanup_builder
        injected = []
        observed = {}
        builder_deadline = None
        cleaning = False
        def compile_command(command, **kwargs):
            nonlocal builder_deadline
            if cleaning:
                self.commands.require_usable()
                require(command[0] == '/usr/bin/docker', 'Identity')
                now = time.monotonic()
                if builder_deadline is None or now >= builder_deadline:
                    self.commands.unusable = True
                    raise fixture.ImportRejected('UnconfirmedIsolation')
                reserve = min(0.25, (builder_deadline - now) / 4)
                try:
                    with fixture.OpenClient(command, timeout=kwargs.get('timeout', 60),
                            deadline=builder_deadline - reserve, cleanup_deadline=builder_deadline,
                            stdout_limit=kwargs.get('limit', 16*1024*1024),
                            stderr_limit=kwargs.get('limit', 16*1024*1024),
                            allow_stderr=True, env=kwargs.get('env')) as client:
                        try:
                            output = client.finish()
                        except fixture.ImportRejected as error:
                            if error.code != 'Exit':
                                raise
                            output = bytes(client.buffers[0])
                        result = subprocess.CompletedProcess(command, client.process.returncode,
                            output, bytes(client.buffers[1]))
                    return result
                except fixture.ImportRejected:
                    self.commands.unusable = True
                    raise fixture.ImportRejected('UnconfirmedIsolation') from None
            if command[:2] == ['/usr/bin/docker', 'run']:
                target = 'KNOWWEAVE_C4_TARGET_BIRTH_SHA256=' + target_pin
                require(command.count(target) == 1 and command.count('--entrypoint') == 1 and
                        command.count(BUILDER_ID) == 1 and '--network' in command and
                        command[command.index('--network') + 1] == 'none', 'Identity')
                require(not any('KNOWWEAVE_C4_IMPORT_SOURCE_BIRTH_SHA256=' in word for word in command), 'Identity')
                require(not any(word.split('=', 1)[0] in ('--cpus', '--memory', '--memory-swap') for word in command), 'Identity')
                command = list(command)
                index = command.index('--entrypoint')
                command[index:index] = ['--cpus=4', '--memory=8g', '--memory-swap=8g',
                    '--env', 'KNOWWEAVE_C4_IMPORT_SOURCE_BIRTH_SHA256=' + source_pin]
                require(builder_deadline is None, 'Protocol')
                builder_deadline = time.monotonic() + kwargs.get('timeout', 7200)
                injected.append(True)
                return self._run_import_builder(command, kwargs, observed, deadline=builder_deadline)
            return original(command, **kwargs)
        def cleanup(batch, stage):
            nonlocal cleaning
            self.commands.require_usable()
            if builder_deadline is None or time.monotonic() >= builder_deadline:
                self.commands.unusable = True
                raise fixture.ImportRejected('UnconfirmedIsolation')
            cleaning = True
            try:
                if observed:
                    _, label = self.delegate._builder_identity(batch, stage)
                    ids = self.delegate._builder_container_ids(label)
                    require(ids in ([], [observed['container_id']]), 'Identity')
                original_cleanup(batch, stage, expected_id=observed.get('container_id'))
                require(time.monotonic() < builder_deadline, 'UnconfirmedIsolation')
                if observed:
                    observed['cleanup_confirmed'] = True
            except BaseException:
                self.commands.unusable = True
                raise
            finally:
                cleaning = False
        # Scoped adaptation of this separately loaded verified module instance;
        # no global process environment or old runner mode changes.
        self.delegate._run_bounded = compile_command
        self.delegate._cleanup_builder = cleanup
        try:
            binary, sha = self.delegate._compile_bound_probe(self.source, self.batch,
                'probe-preflight-build' if placeholder else 'probe-live-build', target_pin)
        finally:
            self.delegate._run_bounded = original
            self.delegate._cleanup_builder = original_cleanup
        require(injected == [True] and observed.get('cleanup_confirmed') is True and
                self.delegate._file_digest(binary) == sha, 'Identity')
        return binary, {'binary_sha256': sha, 'source_birth_sha256': source_pin,
                        'target_birth_sha256': target_pin, 'placeholder': placeholder,
                        'builder_observation': observed,
                        'builder_image_id': BUILDER_ID, 'source_sha256': self.source_hash}

    def _builder_projection(self, command, row):
        name = command[command.index('--name') + 1]
        label = command[command.index('--label') + 1].split('=', 1)
        cid = row.get('Id')
        host = row.get('HostConfig', {})
        config = row.get('Config', {})
        require(type(cid) is str and HEX_ID.fullmatch(cid) and row.get('Name') == '/' + name and
            row.get('Image') == BUILDER_ID and config.get('Image') == BUILDER_ID and
            config.get('Labels', {}).get(label[0]) == label[1] and config.get('User') == '0:0' and
            row.get('State', {}).get('Running') is True and host.get('NanoCpus') == 4_000_000_000 and
            host.get('Memory') == 8 * 1024**3 and host.get('MemorySwap') == 8 * 1024**3 and
            host.get('NetworkMode') == 'none' and host.get('Privileged') is False and
            host.get('CapDrop') == ['ALL'] and 'no-new-privileges' in host.get('SecurityOpt', []), 'Identity')
        binds = [mount for mount in row.get('Mounts', []) if mount.get('Type') == 'bind']
        # Derive exact build mount from the fixed inherited command, then verify
        # it remains one of this batch's two allowlisted directories.
        mounts = [command[i+1] for i, word in enumerate(command[:-1]) if word == '--mount']
        require(len(mounts) == 2, 'Identity')
        build_mount = next((value for value in mounts if value.endswith(',dst=/target')), None)
        require(build_mount is not None, 'Identity')
        build = Path(build_mount.removeprefix('type=bind,src=').removesuffix(',dst=/target'))
        require(build in (self.batch / 'probe-preflight-build', self.batch / 'probe-live-build') and
            len(binds) == 2 and all(any(mount.get('Source') == str(source) and
                mount.get('Destination') == destination and mount.get('RW') is writable
                for mount in binds) for source, destination, writable in
                [(self.source, '/reviewed', False), (build, '/target', True)]) and
            not any(mount.get('Type') == 'volume' for mount in row.get('Mounts', [])), 'Identity')
        return {'container_id': cid, 'image_id': BUILDER_ID, 'nano_cpus': host['NanoCpus'],
                'memory_bytes': host['Memory'], 'memory_swap_bytes': host['MemorySwap'],
                'network_none': True, 'source_readonly': True, 'build_directory': build.name,
                'cleanup_confirmed': False}

    def _run_import_builder(self, command, kwargs, observed, *, deadline=None):
        import subprocess
        if deadline is None:
            deadline = time.monotonic() + kwargs.get('timeout', 7200)
        active_deadline = deadline - 5  # reserve settlement inside the same absolute budget
        startup = min(active_deadline, time.monotonic() + 30)
        name = command[command.index('--name') + 1]
        with fixture.OpenClient(command, timeout=7200, deadline=active_deadline,
            cleanup_deadline=deadline, stdout_limit=16*1024*1024, stderr_limit=16*1024*1024,
            allow_stderr=True, env=kwargs['env']) as client:
            while True:
                require(time.monotonic() < startup and client.process.poll() is None, 'Deadline')
                try:
                    raw = self.commands._run('/usr/bin/docker', ('inspect', name), startup)
                except fixture.ImportRejected as error:
                    if error.code != 'Exit':
                        raise
                    time.sleep(0.05)
                    continue
                rows = json.loads(raw)
                require(type(rows) is list and len(rows) == 1 and type(rows[0]) is dict, 'Identity')
                observed.update(self._builder_projection(command, rows[0]))
                break
            # After discovery every observation and inherited cleanup binding
            # uses the captured immutable ID. Never accept a replacement name.
            exact = json.loads(self.commands._run('/usr/bin/docker', ('inspect', observed['container_id']), active_deadline))
            require(type(exact) is list and len(exact) == 1 and
                    self._builder_projection(command, exact[0]) == observed, 'Identity')
            try:
                output = client.finish()
            except fixture.ImportRejected as error:
                if error.code != 'Exit':
                    raise
                output = bytes(client.buffers[0])
            stderr = bytes(client.buffers[1])
            stage = observed['build_directory']
            self.helper._private_write(self.batch / 'evidence' / (stage + '-stdout.private.log'), output)
            self.helper._private_write(self.batch / 'evidence' / (stage + '-stderr.private.log'), stderr)
            return subprocess.CompletedProcess(command, client.process.returncode, output, stderr)

    def _list_import_tests(self, binary, sha):
        require(self.delegate._file_digest(binary) == sha, 'Identity')
        process = self.delegate._run_bounded([str(binary), '--list', '--ignored'],
            cwd=self.source, timeout=60, env={'PATH': '/usr/bin:/bin', 'HOME': str(self.batch),
                                            'TMPDIR': str(self.batch / 'tmp')})
        require(process.returncode == 0 and not process.stderr and
                self.delegate._file_digest(binary) == sha, 'Protocol')
        lines = process.stdout.splitlines()
        expected = [(IMPORT_PREFIX + definition[0] + ': test').encode() for definition in IMPORT_CASES.values()]
        require(all(lines.count(name) == 1 for name in expected) and
                sum(line.startswith(IMPORT_PREFIX.encode()) for line in lines) == 11, 'Protocol')
        return {'exit_code': 0, 'exact_tests_listed': [line.decode().removesuffix(': test') for line in expected],
                'listing_sha256': digest(process.stdout), 'executed': False}

    def _preflight_import_builder(self):
        """Separable root-batch compile/list only; never creates PG resources.

        Controller may call after verified archive/backend preparation and
        read-only _budget(). Placeholder binaries cannot enter execute_import.
        """
        binary, build = self._compile_import_binary('0' * 64, '0' * 64, placeholder=True)
        return {**build, **self._list_import_tests(binary, build['binary_sha256'])}

    def _birth(self, row):
        target = row['control'] / 'targets' / row['batch_id']
        candidate, inspection = self.pin.inspect_candidate(row['control'], row['batch_id'], self.initdb)
        raw = self.helper._private_read(target / 'control' / (row['identity']['database'] + '.birth.json'), limit=4096)
        require(digest(raw) == candidate['birth_sha256'], 'Identity')
        row['birth_sha256'] = digest(raw)
        row['inspection_sha256'] = digest(self.helper._json_bytes(inspection))
        self.helper._private_write(self.batch / 'evidence' / (row['role'] + '-inspection.json'),
                                   self.helper._json_bytes(inspection))
        return target, json.loads(raw)

    def execute_import(self, case):
        self._budget()
        self.build_evidence['preflight'] = self._preflight_import_builder()
        before = self.provisioner.snapshot()
        specs = [('source', self.args.source_batch_id, self.args.source_subnet),
                 ('target', self.args.target_batch_id, self.args.target_subnet)]
        if case == 'wrong-endpoint':
            specs.append(('clone', self.args.clone_batch_id, self.args.clone_subnet))
        import ipaddress
        networks = []
        for _, batch, subnet in specs:
            self.provisioner.admit_fresh(self.provisioner.identity_for(batch), subnet, before)
            network = ipaddress.ip_network(subnet, strict=True)
            require(not any(network.overlaps(other) for other in networks), 'Identity')
            networks.append(network)
        source = self._create('source', self.args.source_batch_id, self.args.source_subnet)
        target = self._create('target', self.args.target_batch_id, self.args.target_subnet)
        source_root, _ = self._birth(source)
        target_root, target_birth = self._birth(target)
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(self.batch), 'LC_ALL': 'C',
            'TMPDIR': str(self.batch / 'tmp'), 'TMP': str(self.batch / 'tmp'), 'TEMP': str(self.batch / 'tmp'),
            'KNOWWEAVE_C4_IMPORT_CASE': case, 'KNOWWEAVE_C4_IMPORT_SOURCE_ROOT': str(source_root),
            'KNOWWEAVE_C4_IMPORT_TARGET_ROOT': str(target_root),
            'KNOWWEAVE_C4_IMPORT_ARTIFACT_ROOT': str(self.batch / 'artifacts')}
        if case == 'wrong-endpoint':
            identity = self.provisioner.identity_for(self.args.clone_batch_id)
            row = {'role': 'clone', 'identity': identity, 'control': self.batch / 'clone-control',
                   'batch_id': self.args.clone_batch_id, 'subnet': self.args.clone_subnet,
                   'container_id': None, 'before': before}
            self.clone_record = row
            self.resources.append(row)
            cloned = self.delegate._prepare_physical_clone(self.provisioner, self.batch, target_root,
                target['container_id'], identity, self.args.clone_subnet, before, target_birth,
                target['identity']['database'])
            row['container_id'] = cloned['container_id']
            row['inspection_sha256'] = digest(self.helper._json_bytes(cloned))
            env.update({'KNOWWEAVE_C4_CLONE_CONTAINER_ID': cloned['container_id'],
                'KNOWWEAVE_C4_CLONE_NETWORK_ID': cloned['network_id'],
                'KNOWWEAVE_C4_CLONE_NETWORK_NAME': identity['network'],
                'KNOWWEAVE_C4_CLONE_PROJECT': identity['project'],
                'KNOWWEAVE_C4_CLONE_VOLUME_NAME': identity['volume'],
                'KNOWWEAVE_C4_CLONE_SUBNET': self.args.clone_subnet})
        # Revalidate both fresh issuer records after clone/setup and immediately
        # before compilation. Each pair uses its own build directory and pins.
        self._birth_recheck(source)
        self._birth_recheck(target)
        binary, build = self._compile_import_binary(source['birth_sha256'], target['birth_sha256'], placeholder=False)
        self.build_evidence['live'] = {**build, **self._list_import_tests(binary, build['binary_sha256'])}
        require(self.verify_source() and self.delegate._file_digest(binary) == build['binary_sha256'], 'Identity')
        process = self.delegate._run_bounded(_import_test_command(binary, case), cwd=self.source,
                                            env=env, timeout=360, limit=64 * 1024)
        self.import_failure_diagnostic = _import_failure_process(process, case)
        self.helper._private_write(self.batch / 'evidence' / 'live-test-stdout.private.log', process.stdout)
        self.helper._private_write(self.batch / 'evidence' / 'live-test-stderr.private.log', process.stderr)
        require(self.delegate._file_digest(binary) == build['binary_sha256'] and not process.stderr, 'Identity')
        observed = _import_observations(process.stdout, case, process.returncode)
        artifacts = {}
        for name, key in [('fixture.dump', 'dump'), ('pg_restore-list.txt', 'toc'), ('decoded.sql', 'source_sql')]:
            raw = self.helper._private_read(self.batch / 'artifacts' / name, limit=64 * 1024)
            require(0 < len(raw) <= 64 * 1024, 'InputLimit')
            artifacts[key] = {'sha256': digest(raw), 'bytes': len(raw),
                              'provenance': 'fresh_root_private_fixed_producer_artifact'}
            if key != 'source_sql':
                require(digest(raw) == observed[key + '_sha256'], 'Identity')
        return {'observation': observed, 'builds': self.build_evidence, 'capacity': self.capacity,
                'artifacts': artifacts,
                'exact_test': IMPORT_PREFIX + IMPORT_CASES[case][0], 'test_exit_code': process.returncode,
                'test_stdout_sha256': digest(process.stdout), 'markers': self._markers(target, observed)}

    def _birth_recheck(self, row):
        candidate, inspection = self.pin.inspect_candidate(row['control'], row['batch_id'], self.initdb)
        require(candidate['birth_sha256'] == row['birth_sha256'], 'Identity')
        require(inspection['live']['container_id'] == row['container_id'], 'Identity')

    def _markers(self, target, observation):
        root = target['control'] / 'targets' / target['batch_id'] / 'control'
        db = target['identity']['database']
        result = {}
        for stage, suffix in [('attempt', '.restore.attempt'), ('intent', '.restore.commit-attempt')]:
            path = root / (db + suffix)
            expected = (observation['case'] not in ('ready-eof', 'ready-restart', 'wrong-endpoint')
                        if stage == 'attempt' else observation['case'] in
                        ('success', 'commit-intent-sync-failure', 'commit-unknown'))
            require(os.path.lexists(path) is expected, 'Journal')
            if not expected:
                result[stage] = {'exists': False}
                continue
            raw = self.helper._private_read(path, limit=4096)
            value = json.loads(raw)
            require(type(value) is dict and value.get('record_type') == 'CONTROLLED_IMPORT_CANDIDATE' and
                    type(value.get('format_version')) is int and value['format_version'] == 1 and
                    value.get('phase') == ('ATTEMPT' if stage == 'attempt' else 'COMMIT_ATTEMPTED') and
                    value.get('database') == db and type(value.get('writer_sha256')) is str and
                    HEX_ID.fullmatch(value['writer_sha256']), 'Journal')
            result[stage] = {'exists': True, 'sha256': digest(raw), 'phase': value.get('phase'),
                             'writer_sha256': value.get('writer_sha256')}
            if stage == 'attempt':
                require(value.get('batch_id') == target['batch_id'] and
                        type(value.get('fixture_version')) is int and value['fixture_version'] == 1 and
                        type(value.get('inspection_sha256')) is str and HEX_ID.fullmatch(value['inspection_sha256']) and
                        value.get('dump_sha256') == observation['dump_sha256'] and
                        value.get('raw_sql_sha256') == observation['raw_sql_sha256'] and
                        value.get('transformed_sql_sha256') == observation['transformed_sql_sha256'] and
                        value.get('birth_sha256') == target['birth_sha256'], 'Journal')
            else:
                require(value.get('attempt_sha256') == result['attempt'].get('sha256') and
                        value.get('writer_sha256') == result['attempt'].get('writer_sha256'), 'Journal')
        return result

    def stop(self):
        self._isolation_deadline()
        discovery_failed = False
        if self.clone_record is not None and self.clone_record['container_id'] is None:
            try:
                self.commands.require_usable()
                self.clone_record['container_id'] = self.delegate._find_owned_clone_pg(
                    self.provisioner, self.clone_record['identity'], self.clone_record['before'])
                require(self.clone_record['container_id'] is not None, 'UnconfirmedIsolation')
            except BaseException:
                discovery_failed = True
        result = super().stop()
        require(not discovery_failed, 'UnconfirmedIsolation')
        return result

    def evidence_identity(self):
        identity = super().evidence_identity()
        identity['package']['clone_helper_sha256'] = self.args.clone_helper_sha256
        identity['gate1'] = {'status': PASSED, 'result_sha256': GATE1_SHA256,
            'provenance': 'controller_reviewed_reference_not_fresh_dump_identity'}
        for row in self.resources:
            identity['resources'][row['role']]['birth_sha256'] = row.get('birth_sha256')
        return identity


def _prepare_backend(args):
    require(sys.platform == 'linux' and os.geteuid() == 0, 'Identity')
    batches = [_canonical_batch(getattr(args, name)) for name in
               ('batch_id', 'source_batch_id', 'target_batch_id')]
    require(len(set(batches)) == 3, 'Identity')
    helper, manifest, content = _archive_contract(args)
    backend = ImportBackend if getattr(args, 'phase', None) == 'import' else ContractBackend
    return backend(args, helper, manifest, content)


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


def run_import_case(args, case: str) -> dict:
    require(getattr(args, 'phase', None) == 'import' and case in IMPORT_CASES, 'Protocol')
    # This is the reviewed gate1 prerequisite, never a fresh-dump capability.
    # All real source authority is re-established by the private Linux issuer.
    require(getattr(args, 'gate1_status', None) == PASSED and
            getattr(args, 'gate1_result_sha256', None) == GATE1_SHA256, 'Identity')
    require(getattr(args, 'case', None) == case and
            getattr(args, 'postgres_image', None) == POSTGRES_IMAGE and
            getattr(args, 'builder_image_id', None) == BUILDER_ID, 'Identity')
    names = ['batch_id', 'source_batch_id', 'target_batch_id']
    if case == 'wrong-endpoint':
        names.append('clone_batch_id')
        require(type(getattr(args, 'clone_subnet', None)) is str, 'Identity')
    else:
        require(getattr(args, 'clone_batch_id', None) is None and
                getattr(args, 'clone_subnet', None) is None, 'Identity')
    batches = [_canonical_batch(getattr(args, name, None)) for name in names]
    require(len(set(batches)) == len(batches), 'Identity')
    require(all(type(getattr(args, name, None)) is str and HEX_ID.fullmatch(getattr(args, name))
        for name in ('archive_sha256', 'manifest_sha256', 'runner_sha256',
                     'archive_helper_sha256', 'fixture_sha256', 'clone_helper_sha256')), 'Identity')
    require(type(getattr(args, 'commit', None)) is str and re.fullmatch(r'[0-9a-f]{40}', args.commit), 'Identity')
    backend = _prepare_backend(args)
    result = {'case': case, 'status': IMPORT_FAILED, 'reason_code': None,
              'not_full_restore': True, 'retry_allowed': False}
    try:
        try:
            result.update(backend.execute_import(case))
            result['status'] = IMPORT_PASSED if case == 'success' else IMPORT_NEGATIVE
        except BaseException as error:
            known = isinstance(error, ImportRejected) or (fixture is not None and isinstance(error, fixture.ImportRejected))
            result['reason_code'] = error.code if known and error.code in CODES else 'Io'
        result['builds'] = backend.build_evidence
        result['capacity'] = getattr(backend, 'capacity', {})
        try:
            result['isolation'] = backend.stop()
            _validate_import_isolation(result)
        except BaseException:
            result['status'], result['reason_code'] = IMPORT_UNCONFIRMED, 'UnconfirmedIsolation'
        try:
            require(backend.verify_source(), 'Identity')
            result['source_unchanged'] = True
        except BaseException:
            if result['status'] != IMPORT_UNCONFIRMED:
                result['status'], result['reason_code'] = IMPORT_FAILED, 'Identity'
        result['identity'] = backend.evidence_identity()
        if result['status'] not in (IMPORT_PASSED, IMPORT_NEGATIVE):
            result.update(getattr(backend, 'import_failure_diagnostic', {
                'exact_test': None, 'exact_test_exit_code': None,
                'diagnostic_status': 'UNKNOWN', 'diagnostic': None}))
        published = backend.publish(result)
        require(not os.path.lexists(backend.batch / 'evidence' / 'result.pending'), 'Journal')
        require(digest(backend.helper._private_read(backend.batch / 'evidence' / 'result.json', limit=256*1024)) ==
                published['result_sha256'], 'Journal')
        published['pending_absent'] = True
        if published['status'] in (IMPORT_PASSED, IMPORT_NEGATIVE):
            _validate_import_final(published)
        return published
    finally:
        backend.release()


def _hold_unusable_owners():
    """Failed CLI lifetime, not a renewed isolation deadline or success path.

    An unserviceable kernel may leave this owner/lock here until external host
    teardown. Normal CLI exit must not abandon the exact child or its pipes.
    """
    while fixture._UNSETTLED_CLIENTS:
        for owner in tuple(fixture._UNSETTLED_CLIENTS):
            try:
                owner.close(observe_only=True)  # No new kill/wait budget, even before original expiry.
            except fixture.ImportRejected:
                pass
            if owner in fixture._UNSETTLED_CLIENTS:
                with owner.condition:
                    owner.condition.wait(0.05)
    for backend in tuple(_UNSETTLED_BACKENDS):
        backend.release()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--phase', choices=['contract', 'import'], required=True)
    parser.add_argument('--archive', type=Path, required=True)
    for name in ('archive-sha256', 'manifest-sha256', 'commit', 'runner-sha256',
                 'archive-helper-sha256', 'fixture-sha256', 'batch-id',
                 'source-batch-id', 'target-batch-id', 'source-subnet', 'target-subnet'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--case', choices=tuple(IMPORT_CASES))
    for name in ('gate1-status', 'gate1-result-sha256', 'postgres-image', 'builder-image-id',
                 'clone-helper-sha256', 'clone-batch-id', 'clone-subnet'):
        parser.add_argument('--' + name)
    args = parser.parse_args(argv)
    try:
        result = run_contract(args) if args.phase == 'contract' else run_import_case(args, args.case)
        print(json.dumps(result, separators=(',', ':')), flush=True)
        return 0 if result['status'] in (PASSED, IMPORT_PASSED, IMPORT_NEGATIVE) else 1
    except BaseException:
        if fixture is not None and fixture._UNSETTLED_CLIENTS:
            try:
                print('CONTROLLED_IMPORT_UNCONFIRMED_UNUSABLE_OWNING_CLEANUP', flush=True)
            except OSError:
                pass  # Closed diagnostic output cannot relinquish cleanup ownership.
            finally:
                _hold_unusable_owners()
            return 1
        print('CONTRACT_ADMISSION_OR_EVIDENCE_REJECTED_NOT_IMPORT' if args.phase == 'contract'
              else 'CONTROLLED_IMPORT_ADMISSION_OR_EVIDENCE_REJECTED_NOT_FULL_RESTORE', flush=True)
        return 1


if __name__ == '__main__':
    sys.dont_write_bytecode = True
    raise SystemExit(main())
