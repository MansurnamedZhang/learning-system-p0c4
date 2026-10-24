"""Private fixed full-rehearsal lane. No normal source capability is widened.

A profile is a root-measured topology/build observation, never source-capture,
CompleteBackup, target-write or recovery authority. Rust accepts it only in test
builds with an exact compile-time byte pin and still performs both live guards.
"""
import hashlib
import copy
import json
import re
import uuid

PROFILE_PATH = '/var/lib/knowweave-full-rehearsal/profile.json'
PROFILE_CAP = 16384
from types import MappingProxyType

RECOVERY_CASE_TESTS = MappingProxyType({
    'recovery-leases': 'full_restore::live_tests::recovery_invalidates_unexpired_and_expired_source_tokens',
    'recovery-retry-limit': 'full_restore::live_tests::recovery_preserves_attempt_count_and_three_attempt_ceiling',
    'recovery-effects': 'full_restore::live_tests::recovery_unknown_snapshot_effect_is_unclaimable_and_blocks_usable',
    'recovery-derived': 'full_restore::live_tests::recovery_missing_derived_export_and_digest_mismatch_stay_isolated',
    'recovery-receipt-failure': 'full_restore::live_tests::recovery_receipt_sync_failure_never_returns_recovered_instance',
})
ATTENTION_CASE_TESTS = MappingProxyType({
    'attention-full-chain': 'restore_preflight::target_binding::controlled_import::full::attention_tests::attention_full_chain_preserves_authoritative_inventory_and_originals',
    'attention-missing-asset': 'restore_preflight::target_binding::controlled_import::full::attention_tests::attention_missing_asset',
    'attention-corrupt-dump': 'restore_preflight::target_binding::controlled_import::full::attention_tests::attention_corrupt_dump',
    'attention-interrupted-transfer': 'restore_preflight::target_binding::controlled_import::full::attention_tests::attention_interrupted_transfer',
    'attention-unknown-effect': 'restore_preflight::target_binding::controlled_import::full::attention_tests::attention_unknown_effect',
    'attention-root-replacement': 'restore_preflight::target_binding::controlled_import::full::attention_tests::attention_root_replacement',
})
ATTENTION_PROOF_CLASSIFICATION = 'FULL_RESTORE_REHEARSAL_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE'
ATTENTION_REJECTION_CLASSIFICATION = 'EXPECTED_REJECTION_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE'

RECOVERY_PROOF_CLASSIFICATION = 'FULL_RECOVERY_FOUNDATION_CASE_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE'
_SNAPSHOT_TEST_PREFIX='restore_preflight::target_binding::controlled_import::full::snapshot_effects_tests::recovery_snapshot_'
SNAPSHOT_CASE_TESTS=MappingProxyType({
    'snapshot-p1':_SNAPSHOT_TEST_PREFIX+'p1_revoke_unexpired',
    'snapshot-p2':_SNAPSHOT_TEST_PREFIX+'p2_revoke_full_history',
    'snapshot-p3-live':_SNAPSHOT_TEST_PREFIX+'p3_ceiling_unexpired',
    'snapshot-p3-expired':_SNAPSHOT_TEST_PREFIX+'p3_ceiling_expired',
    'snapshot-p4':_SNAPSHOT_TEST_PREFIX+'p4_real_none_after_third_expiry',
    'snapshot-p5-one':_SNAPSHOT_TEST_PREFIX+'p5_transient_one',
    'snapshot-p5-two':_SNAPSHOT_TEST_PREFIX+'p5_transient_two',
    'snapshot-p5-terminal':_SNAPSHOT_TEST_PREFIX+'p5_terminal_unchanged',
    'snapshot-p6':_SNAPSHOT_TEST_PREFIX+'p6_committed_copy_held_delivery',
    'snapshot-queued-zero':_SNAPSHOT_TEST_PREFIX+'queued_zero_no_effect',
    'snapshot-history-gap':_SNAPSHOT_TEST_PREFIX+'direct_claim_history_gap_never_positive',
})
# The five fixed fixtures contain at most seventeen jobs; this is not a new
# general recovery metadata allowance or an excuse to truncate a larger report.
RECOVERY_REPORT_CAP = 1024 * 1024

CASE_TESTS = MappingProxyType({
    **RECOVERY_CASE_TESTS,
    **ATTENTION_CASE_TESTS,
    **SNAPSHOT_CASE_TESTS,
    'full-import': 'full_restore::live_tests::full_import_preserves_one_writer_transaction_and_exact_endpoint',
    'full-precommit-eof': 'full_restore::live_tests::full_import_eof_before_commit_leaves_zero_objects',
    'full-cancel': 'full_restore::live_tests::full_import_cancel_retains_guards_until_isolation',
    'full-commit-unknown': 'full_restore::live_tests::full_import_commit_unknown_is_unusable_and_no_replay',
    'full-wrong-endpoint': 'full_restore::live_tests::full_import_wrong_endpoint_writes_nothing',
    'full-bad-input': 'full_restore::live_tests::full_import_bad_dump_dirty_target_or_bad_role_writes_nothing',
    'full-bad-role': 'full_restore::live_tests::full_import_bad_role_writes_nothing',
    'full-dirty-target': 'full_restore::live_tests::full_import_dirty_target_writes_nothing',
    'full-asset-corrupt': 'full_restore::live_tests::full_assets_corruption_blocks_recovery_receipt',
})
_KEYS = {'format_version','capability','source_case_id','target_case_id','source_container_id','target_container_id','source_network_id','target_network_id','daemon_id','application_build_sha256','target_birth_sha256','source_bind_root','initdb','volumes','docker_client','docker_socket'}
_TOKEN = object()

# Closed recipe for the just-created private target only. stat does not follow
# the final symlink; /var/run is the sole allowed, fixed alias. Once its parents
# are root-owned and non-writable, the named-volume mount cannot be replaced by
# the PG user. Mutation uses the held cwd, never a caller-supplied pathname.
_TARGET_SOCKET_RECIPE = r'''
export PATH=/usr/sbin:/usr/bin:/sbin:/bin LC_ALL=C
set -f
for parent in / /var /run; do
 parent_meta=$(stat -c '%d %i %u %g %a %f' -- "$parent")
 set -- $parent_meta
 [ "$#" = 6 ] && [ "$3" = 0 ] || exit 1
 [ "$((0x$6 & 0170000))" = 16384 ] && [ "$((0x$6 & 0022))" = 0 ] || exit 1
done
[ "$(readlink -- /var/run)" = /run ]
before=$(stat -c '%d %i %u %g %a %f' -- /var/run/postgresql)
set -- $before
[ "$#" = 6 ] && [ "$1" -gt 0 ] && [ "$2" -gt 0 ] || exit 1
[ "$((0x$6 & 0170000))" = 16384 ] && [ "$3" = 999 ] || exit 1
case "$4:$5" in 0:1775|999:1775|999:3775) ;; *) exit 1;; esac
device=$1;inode=$2
cd -P /var/run/postgresql
[ "$(stat -c '%d %i %u %g %a %f' -- .)" = "$before" ]
chown -h -- 999:999 .
chmod -- 3775 .
after=$(stat -c '%d %i %u %g %a %f' -- .)
set -- $after
[ "$#" = 6 ] && [ "$1" = "$device" ] && [ "$2" = "$inode" ] || exit 1
[ "$3:$4:$5" = 999:999:3775 ] && [ "$((0x$6 & 0170000))" = 16384 ] || exit 1
[ "$(stat -c '%d %i %u %g %a %f' -- /var/run/postgresql)" = "$after" ]
printf '%s\n%s\n' "$before" "$after"
'''


class FullImportError(RuntimeError):
    pass


def _require(ok, token):
    if not ok:
        raise FullImportError(token)


def _canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        _require(key not in result, 'FULL_PROFILE_DUPLICATE_KEY')
        result[key] = value
    return result


def _hash(value):
    return type(value) is str and re.fullmatch('[0-9a-f]{64}', value) is not None


def _uuid(value):
    try:
        parsed = uuid.UUID(value)
        return type(value) is str and str(parsed) == value and parsed.version == 4
    except (ValueError, TypeError, AttributeError):
        return False


def _path(value):
    return type(value) is str and value.startswith('/') and '\\' not in value and '\0' not in value and all(part not in ('', '.', '..') for part in value[1:].split('/'))


def _identity(value, extra=()):
    _require(type(value) is dict and set(value) == {'path','dev','ino','uid','gid','mode'} | set(extra) and _path(value['path']), 'FULL_PROFILE_IDENTITY_FIELDS')
    _require(all(type(value[k]) is int and 0 < value[k] < 2**64 for k in ('dev','ino')) and all(type(value[k]) is int and 0 <= value[k] < 2**32 for k in ('uid','gid')) and type(value['mode']) is int and 0 <= value['mode'] <= 0o7777, 'FULL_PROFILE_IDENTITY_NUMBERS')


def _validate_profile(value):
    _require(type(value) is dict and set(value) == _KEYS, 'FULL_PROFILE_FIELDS')
    _require(type(value['format_version']) is int and (value['format_version'],value['capability']) in ((1,'full_rehearsal_profile_v1'),(2,'full_rehearsal_snapshot_profile_v2')), 'FULL_PROFILE_VERSION')
    _require(_uuid(value['source_case_id']) and _uuid(value['target_case_id']) and value['source_case_id'] != value['target_case_id'], 'FULL_PROFILE_UUIDS')
    ids = [value[k] for k in ('source_container_id','target_container_id','source_network_id','target_network_id')]
    _require(all(_hash(v) for v in ids) and len(set(ids)) == 4, 'FULL_PROFILE_RESOURCE_IDS')
    _require(type(value['daemon_id']) is str and re.fullmatch('[A-Za-z0-9_.:-]{1,128}', value['daemon_id']) is not None and _hash(value['application_build_sha256']) and _hash(value['target_birth_sha256']), 'FULL_PROFILE_BUILD_DAEMON')
    _identity(value['source_bind_root'])
    _require(value['source_bind_root']['path'].endswith('/'+value['source_case_id']) and value['source_bind_root']['uid'] == 0 and value['source_bind_root']['mode'] == 0o700, 'FULL_PROFILE_SOURCE_ROOT')
    _identity(value['initdb'], ('sha256',))
    _require(value['initdb']['path'] == value['source_bind_root']['path']+'/full-profile/initdb.sh' and value['initdb']['uid'] == 0 and value['initdb']['mode'] == 0o444 and _hash(value['initdb']['sha256']), 'FULL_PROFILE_INITDB')
    source = 'kwc4c-'+value['source_case_id'].replace('-', '')
    target = 'learning-system-p0c4-restore-'+value['target_case_id']
    names = dict(source_pg=source+'-pg',source_data=source+'-source',source_build=source+'-build',source_registry=source+'-registry',target_pg=target+'_pg',target_control=target+'_control',target_socket=target+'_socket')
    if value['format_version']==2:names.update(snapshot_effects=source+'-snapshot-effects',snapshot_export=source+'-snapshot-export')
    _require(type(value['volumes']) is dict and set(value['volumes']) == set(names), 'FULL_PROFILE_VOLUMES')
    paths = set();identities=set()
    for kind, name in names.items():
        volume = value['volumes'][kind]
        _identity(volume, ('name',))
        _require(volume['name'] == name and volume['path'].endswith('/volumes/'+name+'/_data') and volume['path'] not in paths, 'FULL_PROFILE_VOLUME_BINDING')
        _require((volume['dev'],volume['ino']) not in identities,'FULL_PROFILE_VOLUME_ALIAS')
        paths.add(volume['path']);identities.add((volume['dev'],volume['ino']))
        if kind in ('snapshot_effects','snapshot_export'):_require(volume['uid']==volume['gid']==0 and volume['mode']==0o700,'SNAPSHOT_ROOT_OWNER_MODE')
    socket = value['volumes']['target_socket']
    _require(socket['uid'] == 999 and socket['mode'] == 0o3775, 'FULL_PROFILE_SOCKET_OWNER_MODE')
    control = value['volumes']['target_control']
    _require(control['uid'] == 0 and control['mode'] == 0o700, 'FULL_PROFILE_TARGET_CONTROL')
    tool = value['docker_client']
    _identity(tool, ('sha256',))
    _require(tool['path'] == '/usr/bin/docker' and tool['uid'] == 0 and tool['mode'] in (0o555, 0o755) and _hash(tool['sha256']), 'FULL_PROFILE_DOCKER_TOOL')
    socket = value['docker_socket']
    _identity(socket)
    _require(socket['path'] == '/var/run/docker.sock' and socket['uid'] == 0 and socket['mode'] in (0o600, 0o660), 'FULL_PROFILE_DOCKER_SOCKET')
    return value


def _parse_pinned(raw, pin):
    _require(type(raw) is bytes and 0 < len(raw) <= PROFILE_CAP and _hash(pin) and hashlib.sha256(raw).hexdigest() == pin, 'FULL_PROFILE_PIN')
    try:
        value = json.loads(raw, object_pairs_hook=_pairs)
    except (ValueError, UnicodeError) as error:
        raise FullImportError('FULL_PROFILE_JSON') from error
    _require(_canonical(value) == raw, 'FULL_PROFILE_CANONICAL')
    return _validate_profile(value)


def _mounts(value, side):
    _validate_profile(value)
    v = value['volumes'];root = value['source_bind_root']['path']
    target_root = v['target_control']['path']+'/targets/'+value['target_case_id']
    if side == 'source':
        rows = [('volume',v[k]['path'],dest,rw) for k,dest,rw in (
            ('source_pg','/var/lib/postgresql',True),('source_data','/var/lib/knowweave-source',True),('source_build','/target',False),('source_registry','/var/lib/knowweave-c4/registry',True),('target_control',v['target_control']['path'],True),('target_socket','/var/run/knowweave-target',False))]
        rows += [('bind',root+'/initdb.sh','/docker-entrypoint-initdb.d/10-lifecycle.sh',False),('bind','/usr/bin/docker','/usr/bin/docker',False),('bind','/var/run/docker.sock','/var/run/docker.sock',False),('bind',v['target_pg']['path'],v['target_pg']['path'],False),('bind',root+'/full-profile','/var/lib/knowweave-full-rehearsal',False),('bind',value['initdb']['path'],value['initdb']['path'],False)]
        rows += [('bind',root+'/secrets/'+role+'_password','/run/secrets/'+role+'_password',False) for role in ('postgres','admin')]
        if value['format_version']==2:
            rows += [('bind',v['source_data']['path']+'/'+name,destination,rw) for name,destination,rw in (
                ('assets','/var/lib/knowweave-source/assets',False),
                ('snapshot-sockets','/run/kwc4/sfx',True))]
            rows += [('volume',v[key]['path'],destination,True) for key,destination in (
                ('snapshot_effects','/var/lib/knowweave-c4/snapshot-effects-v2'),
                ('snapshot_export','/var/lib/knowweave-c4/snapshot-export-v2'))]
    elif side == 'target':
        # Dedicated initdb path is derived, never selected in profile content.
        rows = [('volume',v['target_pg']['path'],'/var/lib/postgresql',True),('volume',v['target_socket']['path'],'/var/run/postgresql',True),('bind',value['initdb']['path'],'/docker-entrypoint-initdb.d/10-restore.sh',False)]
        rows += [('bind',target_root+'/secrets/'+role+'_password','/run/secrets/'+role+'_password',False) for role in ('postgres','admin')]
    else:
        raise FullImportError('FULL_PROFILE_SIDE')
    return sorted(rows)


def _require_mounts(actual, profile, side):
    expected = _mounts(profile, side)
    _require(type(actual) is list and len(actual) == len(expected) and sorted(actual) == expected, 'FULL_PROFILE_LIVE_MOUNTS')

class FullRehearsalContext:
    """Only the fixed full driver constructs this process-local orchestration.

    It carries measured resource ownership, not a deserialized capture permit.
    SourceLocalPin/FreshLocalCapture never cross this Python boundary.
    """
    __slots__ = ('_batch', '_binding', '_case', '_case_name', '_expected_initdb', '_h', '_ident', '_initdb', '_plan', '_profile', '_profile_root', '_profile_sha', '_record', '_result', '_runner', '_target_identity', '_target_root', '_target_status', '_target_volumes', '_images', '_source', '_fs_owner', '_target_owner', '_compose_sha', '_phase', '_last_daemon', '_compose_bytes', '_target_created_ids', '_broker_observations','_snapshot_source_mountpoint','_snapshot_volumes')

    def _full_c(self):
        return self._plan.get('schema_profile')=='snapshot_c0_d1_v1'

    def _snapshot_effects(self):
        return self._plan.get('format_version')==3 and self._plan.get('snapshot_profile')=='root_snapshot_broker_v2' and self._case_name in (*SNAPSHOT_CASE_TESTS,*ATTENTION_CASE_TESTS)

    def _source_volume_created(self,volume):
        _require(self._snapshot_effects() and self._snapshot_source_mountpoint is None,'SNAPSHOT_ROOT_PREPARATION_ONCE')
        name='kwc4c-'+self._plan['case_id'].replace('-','')+'-source'
        _require(type(volume)is dict and volume['Name']==name and volume['Driver']=='local' and not volume.get('Options') and _path(volume['Mountpoint']) and volume['Mountpoint'].endswith('/volumes/'+name+'/_data'),'SNAPSHOT_SOURCE_VOLUME')
        self._snapshot_source_mountpoint=volume['Mountpoint']

    def _snapshot_volumes_created(self,effects,export):
        _require(self._snapshot_effects() and self._snapshot_volumes is None,'SNAPSHOT_VOLUME_PREPARATION_ONCE')
        saved={}
        for key,suffix,row in [('snapshot_effects','snapshot-effects',effects),('snapshot_export','snapshot-export',export)]:
            name='kwc4c-'+self._plan['case_id'].replace('-','')+'-'+suffix
            _require(type(row)is dict and row['Name']==name and row['Driver']=='local' and not row.get('Options') and _path(row['Mountpoint']) and row['Mountpoint'].endswith('/volumes/'+name+'/_data'),'SNAPSHOT_FRESH_VOLUME')
            saved[key]=copy.deepcopy(row)
        _require(saved['snapshot_effects']['Mountpoint']!=saved['snapshot_export']['Mountpoint'],'SNAPSHOT_DISTINCT_VOLUMES')
        self._snapshot_volumes=saved

    def _selected_test(self):
        return CASE_TESTS[self._case_name]

    def observation(self):
        return dict(case=self._case_name,test=self._selected_test())

    def __init__(self, token, plan):
        _require(token is _TOKEN, 'FULL_CONTEXT_READER_REQUIRED')
        import copy
        self._plan = copy.deepcopy(plan)
        self._case_name = plan['case_name']
        self._profile = None
        self._target_status = None
        self._target_volumes = {}
        self._runner = None
        self._broker_observations=[]
        self._snapshot_source_mountpoint=None
        self._snapshot_volumes=None
        self._fs_owner=None;self._target_owner=None;self._compose_sha=None;self._phase='FS_OWNER_START';self._last_daemon=None;self._compose_bytes=None;self._target_created_ids=None

    def _admit_call(self, ident, subnet, name, archive_sha):
        _require(ident['case_id'] == self._plan['case_id'] and subnet == self._plan['subnet'] and name == self._selected_test() and archive_sha == self._plan['source']['application_build_sha256'], 'FULL_CONTEXT_CALL_BINDING')

    def _prepare(self, h, binding, runner, batch, source, case, ident, images, result, record, files):
        import os
        from pathlib import Path
        import p0c4_restore_target as target
        import p0c4_restore_target_pin_prepare as pin_prepare
        import p0c4_source_lifecycle_gate_acceptance as driver
        self._h, self._binding, self._runner, self._record, self._case, self._ident = h, binding, runner, record, case, copy.deepcopy(ident)
        self._batch,self._result=batch,result
        self._images,self._source=images,source
        self._target_identity = target.identity_for(self._plan['target']['case_id'])
        self._profile_root = case/'full-profile'
        h.mkdir_new(self._profile_root)
        self._initdb = self._profile_root/'initdb.sh'
        _require('deploy/p0c4_full_restore_initdb.sh' in files, 'FULL_INITDB_PACKAGE_MISSING')
        self._expected_initdb=files['deploy/p0c4_full_restore_initdb.sh']
        h.write_new(self._initdb, self._expected_initdb, 0o444)
        names = dict(control=self._target_identity['project']+'_control', socket=self._target_identity['project']+'_socket')
        driver.admit_resources(list(names.values()), runner.docker('volume','ls','--format','{{.Name}}').decode().splitlines())
        # The budget wrapper owns all IO; only its existing ledger lives below.
        ledger=driver.runner_base(runner).ownership
        for kind, name in names.items():
            labels={'knowweave.full-rehearsal.batch':self._plan['batch_id'],'knowweave.full-rehearsal.target':self._plan['target']['case_id'],'knowweave.full-rehearsal.kind':kind}
            row = dict(name=name, labels=labels)
            owner=ledger.acquire('volume',name,dict(name=name,labels=labels),None,row)
            value=driver.creation_call(runner,owner,lambda:binding.create_volume(h,runner,name,labels))
            ledger.known(owner,name);ledger.release(owner,'retained')
            _require(value['Driver']=='local' and not value.get('Options') and value['Labels']==labels, 'FULL_NEW_VOLUME')
            self._target_volumes[kind]=value
        self._target_root=Path(self._target_volumes['control']['Mountpoint'])
        from . import full_target_fs
        self._phase='FS_OWNER_START'
        self._fs_owner=full_target_fs._start_owner(self,source)
        ledger._full_pair=self
        self._phase='PRECREATION_OBSERVE'
        pin_prepare.prepare(self._target_root,self._plan['target']['case_id'],self._plan['target']['subnet'],self._initdb,_full_profile=self)
        self._target_status=target._provision(self._target_root,self._plan['target']['case_id'],self._plan['target']['subnet'],self._initdb,_full_profile=self)
        self._target_volumes['pg']=runner.inspect('volume',self._target_status['volume_name'])
        self._fs_owner.close();self._fs_owner=None
        record['full_target']=copy.deepcopy(dict(identity=self._target_identity,status=self._target_status,volumes=self._target_volumes,stopped=False,execs_absent=False,network_empty=False))
        negative={'full-bad-role':'ALTER ROLE learning_runtime CONNECTION LIMIT 0;','full-dirty-target':'CREATE TABLE public.full_restore_dirty_probe(id integer);'}.get(self._plan['case_name'])
        if negative is not None:
            raw=runner.docker('exec','--user','postgres',self._target_status['container_id'],'psql','-XqAt','-v','ON_ERROR_STOP=1','--dbname',self._target_identity['database'],'-c',negative)
            _require(raw==b'','FULL_NEGATIVE_SETUP_OUTPUT')
            record['full_target']['negative_setup']=self._plan['case_name']


    def _docker(self,*args):
        import p0c4_source_lifecycle_gate_acceptance as driver
        if args and args[0]=='compose':
            path=str(self._target_root/'targets'/self._plan['target']['case_id']/'compose.json')
            _require(args[:3]==('compose','-f',path) and args[3:] in (('config','-q'),('up','-d','--wait','--no-build','--no-deps','pg')) and self._compose_bytes is not None,'FULL_COMPOSE_COMMAND')
            _require(hashlib.sha256(self._compose_bytes).hexdigest()==self._compose_sha,'FULL_COMPOSE_BYTES')
            call=lambda:self._runner.docker('compose','-f','-',*args[3:],stdin=self._compose_bytes,timeout=90)
            if args[3]=='up':
                _require(self._target_owner is not None,'FULL_CREATE_OWNER')
                return driver.creation_call(self._runner,self._target_owner,call).decode()
            return call().decode()
        return self._runner.docker(*args,timeout=15).decode()

    def _inspect(self,kind,ids):
        # Only exact owned resources use full inspect; foreign inventory is a
        # separate fixed projection below. This path remains bounded per call.
        return [self._runner.inspect(kind,identity) for identity in ids]

    def _inventory(self,kind,ids):
        import time
        import p0c4_controlled_import_acceptance as bounded
        import p0c4_maintenance_gate_acceptance as metadata
        import p0c4_source_lifecycle_gate_acceptance as driver
        _require(len(ids)==len(set(ids)),'FULL_INVENTORY_DUPLICATE')
        batches=bounded._inspection_batches(kind,ids)
        driver.log_capacity(driver.runner_base(self._runner).logs,3*len(batches))
        deadline=time.monotonic()+bounded.INSPECT_OPERATION_SECONDS
        result=[];total=0
        for batch in batches:
            left=deadline-time.monotonic();_require(left>0,'FULL_INVENTORY_DEADLINE')
            command=['inspect'] if kind=='container' else [kind,'inspect']
            raw=self._runner.docker(*command,'--format',metadata._inventory_format(kind),*batch,timeout=min(left,5))
            total+=len(raw);_require(total<=bounded.INSPECT_TOTAL_STDOUT_LIMIT and time.monotonic()<deadline,'FULL_INVENTORY_BUDGET')
            rows=metadata.parse_inventory(kind,raw)
            observed=[r['Name'] if kind=='volume' else r['Id'] for r in rows]
            _require(len(observed)==len(batch) and set(observed)==set(batch),'FULL_INVENTORY_COVERAGE')
            result.extend(rows)
        return result

    def _snapshot(self):
        from . import resources
        daemon=self._docker('info','--format','{{.ID}}').strip();self._last_daemon=daemon
        containers=self._docker('ps','-aq','--no-trunc').split()
        networks=self._docker('network','ls','-q','--no-trunc').split()
        volumes=self._docker('volume','ls','--format','{{.Name}}').split()
        rows={kind:self._inventory(kind,ids) for kind,ids in [('container',containers),('network',networks),('volume',volumes)]}
        if self._target_owner is not None:
            expected={'container':'/'+self._target_identity['project']+'-pg-1','network':self._target_identity['network'],'volume':self._target_identity['volume']}
            for kind,values in rows.items():
                for index,row in enumerate(values):
                    labels=row.get('Config',{}).get('Labels',{}) if kind=='container' else row.get('Labels',{})
                    if row['Name']==expected[kind] and labels.get('com.docker.compose.project')==self._target_identity['project']:
                        values[index]=self._runner.inspect(kind,row['Name'] if kind=='volume' else row['Id'])
        routes=resources.route_observer(self._batch,self._result)
        return dict(daemon_id=daemon,containers=rows['container'],networks=rows['network'],volumes=rows['volume'],routes=[row['dst'] for row in routes])

    def _owned_pg_names(self):
        return (self._ident['pg_name'],self._target_identity['project']+'-pg-1')

    def _failure_observation(self):
        if hasattr(self,'_record') and 'preparation_failure' in self._record:return copy.deepcopy(self._record['preparation_failure'])
        allowed={'FS_OWNER_START','PRECREATION_OBSERVE','PRECREATION_PUBLISH','TARGET_FILES','TARGET_CREATE','PG_UID_CHECK','PG_ROOT_MEASURE','BIRTH_PUBLISH','PROFILE_MEASURE','COMPLETE'}
        return dict(phase=self._phase if self._phase in allowed else 'FS_OWNER_START',code=(self._fs_owner._failure if self._fs_owner is not None and self._fs_owner._failure in ('FS_POLICY','FS_UNREACHABLE','HELPER_PROTOCOL') else 'PREPARATION_REJECTED'))

    def _prepare_pin(self,root,batch_id,subnet,initdb):
        self._validate_target(root,batch_id,subnet,initdb)
        self._fs_owner.held();before=self._snapshot();self._phase='PRECREATION_PUBLISH'
        return self._fs_owner.call('pin',dict(before=before))['precreation_sha256']

    def _held_files(self):
        from contextlib import contextmanager
        @contextmanager
        def held():
            self._fs_owner.held()
            yield
            if self._fs_owner is not None:self._fs_owner.held()
        return held()

    def _create_target_files(self,before):
        self._phase='TARGET_FILES'
        self._compose_sha=self._fs_owner.call('files',dict(before=before))['compose_sha256']

    def _verify_compose_bytes(self,raw):
        _require(hashlib.sha256(raw).hexdigest()==self._compose_sha,'FULL_COMPOSE_BYTES')
        self._compose_bytes=bytes(raw)

    def _before_target_create(self,before):
        import p0c4_source_lifecycle_gate_acceptance as driver
        self._phase='TARGET_CREATE';ledger=driver.runner_base(self._runner).ownership
        _require(self._target_owner is None,'FULL_TARGET_ALREADY_ATTEMPTED')
        name=self._target_identity['project']+'-pg-1'
        labels={'com.docker.compose.project':self._target_identity['project'],'com.docker.compose.service':'pg'}
        expected=dict(name=name,labels=labels)
        record=self._record.setdefault('target_creation',dict(identity=copy.deepcopy(self._target_identity)))
        self._target_owner=ledger.acquire('pg',name,expected,dict(name=self._target_identity['network']),record)

    def _target_created(self,before,after,ids):
        import p0c4_source_lifecycle_gate_acceptance as driver
        self._target_created_ids=copy.deepcopy(ids)
        driver.runner_base(self._runner).ownership.known(self._target_owner,ids['container_id'])
        self._fs_owner.call('created',dict(before_sha256=hashlib.sha256(driver.canonical(before)).hexdigest(),after=after,ids=ids))

    def _verify_target_files(self,status):
        _require(self._fs_owner.call('check',dict(status=status))==dict(checked=True),'FULL_CREATED_FILES')

    def _measure_birth_pg(self,ids):
        from . import full_target_fs
        self._phase='PG_ROOT_MEASURE'
        row=self._runner.inspect('volume',ids['volume_name'])
        _require(row['Name']==self._target_identity['volume'] and row['Mountpoint']==ids['volume_mountpoint'],'FULL_PG_VOLUME_BINDING')
        self._target_volumes['pg']=row
        facts=full_target_fs._measure(self,'birth',dict(target_pg=row))['target_pg']['identity']
        _require(facts['mode']&0o022==0 and facts['dev']>0 and facts['ino']>0,'FULL_PG_VOLUME_METADATA')
        return facts['dev'],facts['ino']

    def _publish_target_birth(self,status,ids,image_id,started,volume_id,facts):
        self._phase='BIRTH_PUBLISH'
        value=dict(status=status,ids=ids,image_id=image_id,started_at=started,volume_id=list(volume_id),facts=facts,daemon_id=self._last_daemon)
        if self._full_c():
            value['full_c']=dict(source_role_recipe=_fixed_full_c_fixture_roles(),broker_observations=copy.deepcopy(self._broker_observations))
        return self._fs_owner.call('birth',value)

    def _target_failure(self,isolated):
        if self._fs_owner is not None and self._fs_owner._failure is None and self._fs_owner._state not in ('failure','birth'):
            self._fs_owner.call('failure',dict(code='PREPARATION_REJECTED' if isolated else 'ISOLATION_UNCONFIRMED'))

    def _postgres_uid(self):
        # Fixed approved recipe; measured on the exact newly owned target in
        # _configure before any role/birth acceptance, without a temporary PG.
        return 999,999

    def _validate_target(self, root, batch_id, subnet, initdb):
        _require(str(root)==str(self._target_root) and batch_id==self._plan['target']['case_id'] and subnet==self._plan['target']['subnet'] and str(initdb)==str(self._initdb), 'FULL_TARGET_PRIVATE_PROFILE')
        raw, identity = _measured_regular(self._initdb, 65536)
        _require(raw == self._expected_initdb, 'FULL_INITDB_STABLE')
        _require(identity['uid']==0 and identity['mode']==0o444, 'FULL_INITDB_PRIVATE')

    def _extend_target_document(self, document):
        from . import full_target_fs
        _require(self._target_volumes['socket']['Name']==self._target_identity['project']+'_socket','FULL_SOCKET_NAME')
        full_target_fs._extend_document(document,self._target_identity,self._target_root/'targets'/self._plan['target']['case_id'])
        if self._full_c():
            document['services']['pg']['environment']['C4_FULL_SCHEMA_PROFILE']='snapshot_c0_d1_v1'

    def _extra_target_mounts(self):
        return {'/var/run/postgresql':('volume',self._target_volumes['socket']['Mountpoint'],True)}

    def _validate_extra_target_mounts(self, facts):
        rows=[m for m in facts['Mounts'] if m['Destination']=='/var/run/postgresql']
        _require(len(rows)==1 and rows[0]['Type']=='volume' and rows[0]['Name']==self._target_volumes['socket']['Name'] and rows[0]['Source']==self._target_volumes['socket']['Mountpoint'] and rows[0]['RW'] is True, 'FULL_TARGET_SOCKET_MOUNT')
        declared=[m for m in facts['HostConfig']['Mounts'] if m.get('Target')=='/var/run/postgresql']
        _require(len(declared)==1 and declared[0].get('VolumeOptions')=={'NoCopy':True}, 'FULL_TARGET_SOCKET_NOCOPY')

    def _configure(self, identity, container_id):
        import p0c4_source_lifecycle_gate_acceptance as driver
        _require(identity==self._target_identity and _hash(container_id) and self._target_owner is not None and self._target_owner['state']=='known' and self._target_owner['id']==container_id and self._target_created_ids['container_id']==container_id, 'FULL_TARGET_CONFIGURE_ID')
        name=identity['project']+'-pg-1';ledger=driver.runner_base(self._runner).ownership
        _require(ledger.owners.get(name) is self._target_owner and self._target_owner['intent']['kind']=='pg' and self._target_owner['intent']['batch_id']==self._plan['batch_id'] and self._target_owner['intent']['case_id']==self._plan['case_id'] and self._target_status is None and self._fs_owner is not None and self._fs_owner._state=='created','FULL_TARGET_CONFIGURE_OWNER')
        self._fs_owner.held()
        self._phase='PG_UID_CHECK'
        output=self._runner.docker('exec','--user','0:0',container_id,'sh','-c','id -u postgres; id -g postgres')
        _require(output==b'999\n999\n','FULL_PG_IMAGE_UID')
        self._record['target_pg_identity']=dict(container_id=container_id,uid=999,gid=999)
        socket=self._target_volumes['socket'];root=self._target_root/'targets'/self._plan['target']['case_id']
        expected={'/var/run/postgresql':('volume',socket['Mountpoint'],True),'/var/lib/postgresql':('volume',self._target_created_ids['volume_mountpoint'],True),'/docker-entrypoint-initdb.d/10-restore.sh':('bind',str(self._initdb),False)}
        expected.update({'/run/secrets/'+role+'_password':('bind',str(root/'secrets'/(role+'_password')),False) for role in ('postgres','admin')})
        def current_mount():
            facts=self._runner.inspect('container',container_id)
            labels=facts['Config'].get('Labels') or {}
            _require(facts['Id']==container_id and facts['Name']=='/'+name and facts['Image']==self._plan['images']['postgres'] and facts['Config']['Image']==identity['image'] and labels.get('com.docker.compose.project')==identity['project'] and labels.get('com.docker.compose.service')=='pg' and facts['State']['Running'] is True and facts['State'].get('Health',{}).get('Status')=='healthy','FULL_TARGET_SOCKET_CONTAINER')
            mounts=facts['Mounts']
            _require(len(mounts)==5 and {m['Destination'] for m in mounts}==set(expected) and all((m['Type'],m['Source'],m['RW'])==expected[m['Destination']] for m in mounts),'FULL_TARGET_SOCKET_TOPOLOGY')
            _require(next(m for m in mounts if m['Destination']=='/var/lib/postgresql').get('Name')==identity['volume'],'FULL_TARGET_SOCKET_PG_MOUNT')
            self._validate_extra_target_mounts(facts)
            declared=next(m for m in facts['HostConfig']['Mounts'] if m.get('Target')=='/var/run/postgresql')
            _require(declared.get('Type')=='volume' and declared.get('Source')==socket['Name'] and declared.get('ReadOnly',False) is False and declared['VolumeOptions']['NoCopy'] is True,'FULL_TARGET_SOCKET_DECLARATION')
            observed=self._runner.inspect('volume',socket['Name'])
            _require(socket['Name']==identity['project']+'_socket' and all(observed.get(k)==socket.get(k) for k in ('Name','Mountpoint','Driver','Options','Labels')) and observed['Driver']=='local' and not observed.get('Options'),'FULL_TARGET_SOCKET_VOLUME')
            started=facts['State'].get('StartedAt')
            _require(type(started) is str and bool(started),'FULL_TARGET_SOCKET_EPOCH')
            return started
        self._record['target_socket_stage']='MOUNT_CHECK'
        started=current_mount()
        self._record['target_socket_stage']='DIRECTORY_CONFIGURE'
        code,output,error=self._runner.run([driver.DOCKER,'exec','--user','0:0',container_id,'sh','-ceu',_TARGET_SOCKET_RECIPE],timeout=15)
        _require(code==0 and not error and 0<len(output)<=256,'FULL_TARGET_SOCKET_EXEC')
        rows=output.splitlines()
        _require(len(rows)==2 and all(re.fullmatch(rb'[1-9][0-9]{0,19} [1-9][0-9]{0,19} [0-9]{1,10} [0-9]{1,10} [0-7]{3,4} [0-9a-f]{4}',row) for row in rows),'FULL_TARGET_SOCKET_RECEIPT')
        before,after=([int(v,8 if i==4 else 16 if i==5 else 10) for i,v in enumerate(row.split())] for row in rows)
        _require(before[:2]==after[:2] and all(0<n<2**64 for n in after[:2]) and before[2]==999 and (before[3],before[4]) in ((0,0o1775),(999,0o1775),(999,0o3775)) and after[2:5]==[999,999,0o3775] and all(row[5]==0o40000|row[4] for row in (before,after)),'FULL_TARGET_SOCKET_READBACK')
        self._record['target_socket_stage']='MOUNT_RECHECK'
        _require(current_mount()==started,'FULL_TARGET_SOCKET_RESTARTED')
        self._record['target_socket_configuration']=dict(container_id=container_id,volume_name=socket['Name'],before=dict(zip(('dev','ino','uid','gid','mode'),before[:5])),after=dict(zip(('dev','ino','uid','gid','mode'),after[:5])))
        self._record['target_socket_stage']='VERIFIED'
        # The dedicated, exact package initdb already creates this fixed safe
        # fixture recipe. This is not evidence of an imported source recipe.

    def _probe(self, identity, container_id):
        from . import roles
        _require(identity==self._target_identity and _hash(container_id), 'FULL_TARGET_PROBE_ID')
        raw=self._runner.docker('exec','--user','postgres',container_id,'psql','-XqAt','-v','ON_ERROR_STOP=1','--dbname',identity['database'],'-c',roles._target_probe_sql(self._full_c()))
        observed=json.loads(raw,object_pairs_hook=_pairs)
        expected=_fixed_full_c_fixture_roles() if self._full_c()else _fixed_fixture_roles()
        actual=observed['recipe'] if self._full_c()else roles._validate_recipe(json.dumps(observed['recipe']).encode())
        _require(actual==roles._target_projection(expected) and observed['database_acl']=='{learning_admin=CTc/learning_admin}' and observed['database_owner']=='learning_admin' and observed['schema_owner']=='pg_database_owner' and observed['runtime_create'] is False and observed['runtime_control_system'] is False and observed['admin_control_system'] is True,'FULL_TARGET_ROLE_FACTS')
        if self._full_c():
            raw=self._runner.docker('exec','--user','postgres',container_id,'psql','-XqAt','-v','ON_ERROR_STOP=1','--dbname',identity['database'],'-c',roles._ROOT_BROKER_PROBE)
            broker=roles._validate_broker_birth(json.loads(raw,object_pairs_hook=_pairs))
            _require(not self._broker_observations or broker==self._broker_observations[0],'FULL_C_ROOT_BROKER_CHANGED')
            self._broker_observations.append(broker)
            self._record['target_full_c_broker_observations']=copy.deepcopy(self._broker_observations)

    def _extend_source_document(self, document, binds, volume_spec):
        extras=[dict(type='bind',source='/usr/bin/docker',target='/usr/bin/docker',read_only=True),dict(type='bind',source='/var/run/docker.sock',target='/var/run/docker.sock',read_only=True),dict(type='bind',source=self._target_volumes['pg']['Mountpoint'],target=self._target_volumes['pg']['Mountpoint'],read_only=True),dict(type='bind',source=str(self._profile_root),target='/var/lib/knowweave-full-rehearsal',read_only=True),dict(type='bind',source=str(self._initdb),target=str(self._initdb),read_only=True)]
        binds.extend(extras);document['services']['pg']['volumes'].extend(extras)
        if self._snapshot_effects():
            _require(self._snapshot_source_mountpoint is not None and self._snapshot_volumes is not None,'SNAPSHOT_SOURCE_VOLUME_BEFORE_COMPOSE')
            snapshot=[dict(type='bind',source=self._snapshot_source_mountpoint+'/'+name,target=destination,read_only=readonly) for name,destination,readonly in (
                ('assets','/var/lib/knowweave-source/assets',True),
                ('snapshot-sockets','/run/kwc4/sfx',False))]
            binds.extend(snapshot);document['services']['pg']['volumes'].extend(snapshot)
            for key,destination in [('snapshot_effects','/var/lib/knowweave-c4/snapshot-effects-v2'),('snapshot_export','/var/lib/knowweave-c4/snapshot-export-v2')]:
                document['volumes'][key]={'name':self._snapshot_volumes[key]['Name'],'external':True}
                document['services']['pg']['volumes'].append(dict(type='volume',source=key,target=destination,read_only=False,volume={'nocopy':True}))
        for key,destination,readonly in [('control',str(self._target_root),False),('socket','/var/run/knowweave-target',True)]:
            alias='full_'+key;document['volumes'][alias]={'name':self._target_volumes[key]['Name'],'external':True}
            document['services']['pg']['volumes'].append(dict(type='volume',source=alias,target=destination,read_only=readonly,volume={'nocopy':True}))

    def _source_extra_mounts(self):
        value={('volume',str(self._target_root),str(self._target_root),True),('volume',self._target_volumes['socket']['Mountpoint'],'/var/run/knowweave-target',False)}
        if self._snapshot_effects():value.update(('volume',self._snapshot_volumes[key]['Mountpoint'],destination,True) for key,destination in [('snapshot_effects','/var/lib/knowweave-c4/snapshot-effects-v2'),('snapshot_export','/var/lib/knowweave-c4/snapshot-export-v2')])
        return value

    def _container_adapter(self, legacy):
        return _FullContainerAdapter(legacy,self)

    def _finalize_profile(self, record, expected):
        import os
        from . import full_target_fs
        self._phase='PROFILE_MEASURE'
        mapping={'source_pg':'volume','source_data':'source_volume','source_build':'build_volume','source_registry':'registry_volume'}
        actual={kind:record['volumes'][key] for kind,key in mapping.items()}
        if self._snapshot_effects():actual.update({key:self._snapshot_volumes[key] for key in ('snapshot_effects','snapshot_export')})
        actual.update({kind:self._target_volumes[key] for kind,key in [('target_pg','pg'),('target_control','control'),('target_socket','socket')]})
        record['full_profile_stage']='PG_MEASURE'
        measured=dict(full_target_fs._measure(self,'pg',{key:actual[key] for key in ('source_pg','target_pg')}))
        record['full_profile_stage']='ROOTS_MEASURE'
        measured.update(full_target_fs._measure(self,'roots_snapshot' if self._snapshot_effects()else 'roots',{key:value for key,value in actual.items() if key not in ('source_pg','target_pg')}))
        volumes={key:dict(name=actual[key]['Name'],**value['identity']) for key,value in measured.items()}
        record['full_profile_stage']='DOCKER_METADATA'
        docker, docker_meta = _measured_regular('/usr/bin/docker',256*1024*1024)
        record['full_profile_stage']='INITDB_METADATA'
        initdb, initdb_meta = _measured_regular(self._initdb,65536)
        record['full_profile_stage']='DAEMON_MEASURE'
        daemon=self._runner.docker('info','--format','{{.ID}}').decode().strip()
        record['full_profile_stage']='SOURCE_ROOT_METADATA'
        source_root=_measured_identity(self._case)
        record['full_profile_stage']='DOCKER_SOCKET_METADATA'
        docker_socket=_measured_identity('/var/run/docker.sock',socket=True)
        value=dict(format_version=2 if self._snapshot_effects()else 1,capability='full_rehearsal_snapshot_profile_v2' if self._snapshot_effects()else 'full_rehearsal_profile_v1',source_case_id=self._ident['case_id'],target_case_id=self._plan['target']['case_id'],source_container_id=record['container_id'],target_container_id=self._target_status['container_id'],source_network_id=record['network_id'],target_network_id=self._target_status['network_id'],daemon_id=daemon,application_build_sha256=self._plan['source']['application_build_sha256'],target_birth_sha256=self._target_status['birth_sha256'],source_bind_root=source_root,initdb=dict(sha256=hashlib.sha256(initdb).hexdigest(),**initdb_meta),volumes=volumes,docker_client=dict(sha256=hashlib.sha256(docker).hexdigest(),**docker_meta),docker_socket=docker_socket)
        record['full_profile_stage']='PROFILE_VALIDATION'
        self._profile=_validate_profile(value)
        record['full_profile_stage']='PROFILE_PUBLISH'
        self._h.write_new(self._profile_root/'profile.json',_canonical(value),0o444)
        self._profile_sha=hashlib.sha256(_canonical(value)).hexdigest()
        record['full_rehearsal_profile']=dict(path=str(self._profile_root/'profile.json'),sha256=self._profile_sha,source_mount_count=18 if self._snapshot_effects()else 14,target_mount_count=5)
        record['full_profile_stage']='SOURCE_MOUNTS'
        self._source_mount_projection(self._runner.inspect('container',record['container_id'])['Mounts'])
        record['full_profile_stage']='VERIFIED'
        if self._case_name in ATTENTION_CASE_TESTS:
            from . import attention
            attention.preseed_after_issuer(self,record)
        self._phase='COMPLETE'

    def _compile_pins(self, record):
        _require(record['test']==self._selected_test() and record['container_id']==self._profile['source_container_id'],'FULL_COMPILE_PROFILE_SOURCE')
        pins=dict(KNOWWEAVE_C4_FULL_PROFILE_SHA256=self._profile_sha,KNOWWEAVE_C4_TARGET_BIRTH_SHA256=self._profile['target_birth_sha256'])
        if self._snapshot_effects():pins['KNOWWEAVE_C4_DOCKER_CLI_SHA256']=self._profile['docker_client']['sha256']
        return pins

    def _consumer_environment(self, ident):
        value=dict(KNOWWEAVE_C4_FULL_TARGET_ROOT=str(self._target_root/'targets'/self._plan['target']['case_id']))
        if self._snapshot_effects():value.update(TEST_C4_LIFECYCLE_BROKER_DSN_FILE='/var/lib/knowweave-source/broker-dsn',TEST_C4_LIFECYCLE_RUNTIME_DSN_FILE='/var/lib/knowweave-source/runtime-dsn')
        return value

    def _profile_observation(self):
        _require(self._profile is not None,'FULL_PROFILE_NOT_ISSUED')
        raw, _ = _measured_regular(self._profile_root/'profile.json',PROFILE_CAP)
        return _parse_pinned(raw,self._profile_sha)

    def _source_mount_projection(self, actual):
        value=self._profile_observation()
        _require_mounts([(m.get('Type'),m.get('Source'),m.get('Destination'),m.get('RW')) for m in actual],value,'source')
        names={v['path']:v['name'] for v in value['volumes'].values()}
        rows=[]
        for m in actual:
            if m['Type']=='volume':_require(m.get('Name')==names[m['Source']],'FULL_SOURCE_VOLUME_NAME')
            rows.append(dict(destination=m['Destination'],source=m['Source'],kind=m['Type'],rw=m['RW'],name=m.get('Name','')))
        return sorted(rows,key=lambda row:row['destination'])

    def _pin_management_children(self, rows, consumers, pins):
        if not consumers:return
        for row in rows:
            if row['exe'] in ('/usr/bin/docker','/usr/lib/postgresql/18/bin/pg_restore'):
                expected=({'DOCKER_HOST'},{'DOCKER_HOST','PATH'}) if row['exe']=='/usr/bin/docker' else ({'LC_ALL'},)
                if self._snapshot_effects()and row['exe']=='/usr/bin/docker':expected=(*expected,{'LC_ALL','DOCKER_HOST'})
                _require(row['uid']==0 and row['ppid']==consumers[0]['pid'] and set(row['env_keys']) in expected,'FULL_MANAGEMENT_CHILD')
                pins[row['pid']]=dict(starttime=row['starttime'],exe=row['exe'],env_keys=row['env_keys'])

    def _verify_body(self, record):
        _require(record['test']==self._selected_test() and record['outcome']['passed']==1 and record['outcome']['failed']==record['outcome']['ignored']==0,'FULL_BODY_COUNTS')
        if self._case_name in ATTENTION_CASE_TESTS:
            self._verify_attention_body(record)
            self._probe_target_stopped(record)
            return
        import p0c4_source_lifecycle_gate_acceptance as driver
        if self._case_name=='snapshot-history-gap':
            code,raw,err=driver.pg_exec(self._runner,record['container_id'],['cat','/var/lib/knowweave-source/proofs/full-snapshot-history-gap-result.json'])
            _require(code==0 and not err and 0<len(raw)<=16384,'SNAPSHOT_GAP_BODY')
            _validate_history_gap_proof(json.loads(raw,object_pairs_hook=_pairs),self._target_status)
            path=self._case/'full-snapshot-history-gap-result.json';self._h.write_new(path,raw)
            record['full_snapshot_history_gap_proof']=dict(path=str(path),sha256=hashlib.sha256(raw).hexdigest())
            self._probe_target_stopped(record);return
        code,raw,err=driver.pg_exec(self._runner,record['container_id'],['cat','/var/lib/knowweave-source/proofs/full-import-result.json'])
        _require(code==0 and not err and 0<len(raw)<=16384,'FULL_BODY_OBSERVATION')
        proof=json.loads(raw,object_pairs_hook=_pairs)
        _validate_body_proof(proof,_import_proof_plan(self._plan),self._target_status)
        self._h.write_new(self._case/'full-import-result.json',raw)
        record['full_import_proof']=dict(path=str(self._case/'full-import-result.json'),sha256=hashlib.sha256(raw).hexdigest())
        if self._case_name in RECOVERY_CASE_TESTS:
            self._verify_recovery_body(record,proof)
        if self._snapshot_effects():self._verify_snapshot_body(record,proof)
        self._probe_target_stopped(record)

    def _verify_attention_body(self,record):
        from . import attention
        _require(attention.WIRE_CONTRACT is not None,'ATTENTION_PROOF_CONTRACT_NOT_FROZEN')
        import p0c4_source_lifecycle_gate_acceptance as driver
        try:
            bundle=attention.read_fixed_bundle(driver,self._runner,record['container_id'],self._case_name)
            attention.validate_bundle(bundle,self._plan,self._target_status,record.get('snapshot_capture_byte_audit'))
        except attention.AttentionError as error:raise FullImportError(str(error))from error
        for name,cap,key in attention.fixed_proof_files(self._case_name):
            raw=bundle[name];path=self._case/name;self._h.write_new(path,raw)
            record[key]=dict(path=str(path),sha256=hashlib.sha256(raw).hexdigest())

    def _verify_snapshot_body(self,record,imported):
        import p0c4_source_lifecycle_gate_acceptance as driver
        code,raw,err=driver.pg_exec(self._runner,record['container_id'],['cat','/var/lib/knowweave-source/proofs/full-snapshot-effect-result.json'])
        _require(code==0 and not err and 0<len(raw)<=16384,'SNAPSHOT_BODY_OBSERVATION')
        proof=json.loads(raw,object_pairs_hook=_pairs)
        _validate_snapshot_proof(proof,self._plan,self._target_status,imported)
        code,report,err=driver.pg_exec(self._runner,record['container_id'],['cat','/var/lib/knowweave-source/proofs/full-snapshot-effect-normalization.json'])
        _require(code==0 and not err and 0<len(report)<=RECOVERY_REPORT_CAP and len(report)==proof['normalization_report_bytes'] and hashlib.sha256(report).hexdigest()==proof['normalization_report_sha256'],'SNAPSHOT_NORMALIZATION_BODY_PIN')
        for name,data,key in [('full-snapshot-effect-result.json',raw,'full_snapshot_effect_proof'),('full-snapshot-effect-normalization.json',report,'full_snapshot_effect_normalization')]:
            self._h.write_new(self._case/name,data);record[key]=dict(path=str(self._case/name),sha256=hashlib.sha256(data).hexdigest())

    def _verify_recovery_body(self, record, imported):
        import p0c4_source_lifecycle_gate_acceptance as driver
        code,raw,err=driver.pg_exec(self._runner,record['container_id'],['cat','/var/lib/knowweave-source/proofs/full-recovery-result.json'])
        _require(code==0 and not err and 0<len(raw)<=16384,'RECOVERY_BODY_OBSERVATION')
        proof=json.loads(raw,object_pairs_hook=_pairs)
        _validate_recovery_proof(proof,self._plan,self._target_status,imported)
        code,report,err=driver.pg_exec(self._runner,record['container_id'],['cat','/var/lib/knowweave-source/proofs/full-recovery-normalization.json'])
        _require(code==0 and not err and 0<len(report)<=RECOVERY_REPORT_CAP and len(report)==proof['normalization_report_bytes'] and hashlib.sha256(report).hexdigest()==proof['normalization_report_sha256'],'RECOVERY_NORMALIZATION_BODY_PIN')
        _require(type(json.loads(report,object_pairs_hook=_pairs)) is dict,'RECOVERY_NORMALIZATION_REPORT')
        self._h.write_new(self._case/'full-recovery-normalization.json',report)
        self._h.write_new(self._case/'full-recovery-result.json',raw)
        record['full_recovery_proof']=dict(path=str(self._case/'full-recovery-result.json'),sha256=hashlib.sha256(raw).hexdigest())
        record['full_recovery_normalization']=dict(path=str(self._case/'full-recovery-normalization.json'),sha256=hashlib.sha256(report).hexdigest())

    def _probe_target_stopped(self, record):
        status=self._target_status
        _require(status is not None,'FULL_TARGET_ISSUANCE_MISSING')
        current=self._runner.inspect('container',status['container_id']);network=self._runner.inspect('network',status['network_id'])
        _require(current['Id']==status['container_id'] and current['Config']['Labels'].get('com.docker.compose.project')==self._target_identity['project'] and current['State']['Running'] is False and current['State']['Pid']==0 and not current.get('ExecIDs') and not network.get('Containers'),'FULL_TARGET_NOT_STOPPED')
        record.setdefault('full_target',copy.deepcopy(dict(identity=self._target_identity,status=self._target_status,volumes=self._target_volumes))).update(stopped=True,execs_absent=True,network_empty=True)

    def _cleanup_target(self, record):
        import p0c4_source_lifecycle_gate_acceptance as driver
        isolated=self._target_owner is None
        try:
            if self._target_owner is not None:
                ledger=driver.runner_base(self._runner).ownership;owner=self._target_owner
                if owner['state']=='planned':ledger.release(owner,'absent');isolated=True
                else:
                    clean=driver.CleanupRunner(self._runner,owner,'pg')
                    unknown=owner['id'] is None
                    current=ledger.reconcile(owner) if unknown else clean.inspect('container',owner['id'])
                    # Unknown creations retain both pre-reserved reconciliation
                    # calls. Known IDs use the existing PG cleanup reservation.
                    network=ledger.reconcile(owner,'network',True) if unknown else clean.inspect('network',self._target_created_ids['network_id'])
                    if network is not None:
                        _require(_hash(network['Id']) and network['Name']==self._target_identity['network'] and network.get('Labels',{}).get('com.docker.compose.project')==self._target_identity['project'],'FULL_TARGET_NETWORK_ID')
                    if current is not None:
                        cid=current['Id']
                        _require(_hash(cid) and current['Name']=='/'+self._target_identity['project']+'-pg-1' and current['Config']['Labels'].get('com.docker.compose.project')==self._target_identity['project'] and current['Config']['Labels'].get('com.docker.compose.service')=='pg' and (unknown or cid==owner['id']),'FULL_TARGET_STOP_ID')
                        if current['State']['Running']:clean.docker('stop','--timeout','10',cid,timeout=15)
                        stopped=clean.inspect('container',cid)
                        _require(stopped['Id']==cid and stopped['State']['Running'] is False and stopped['State']['Pid']==0 and not stopped.get('ExecIDs'),'FULL_TARGET_STOP_UNKNOWN')
                        _require(network is not None,'FULL_TARGET_NETWORK_UNKNOWN')
                    if network is not None:
                        after=clean.inspect('network',network['Id'])
                        _require(after['Id']==network['Id'] and after['Name']==network['Name'] and not after.get('Containers'),'FULL_TARGET_NETWORK_NOT_EMPTY')
                    ledger.release(owner,'absent' if current is None else 'stopped');isolated=True
                    if self._target_status is not None:
                        record.setdefault('full_target',copy.deepcopy(dict(identity=self._target_identity,status=self._target_status,volumes=self._target_volumes))).update(stopped=True,execs_absent=True,network_empty=True)
        finally:
            if not isolated:
                record['preparation_cleanup']='ISOLATION_UNCONFIRMED'
                if self._target_owner is not None:self._target_owner['state']='unknown'
            if self._fs_owner is not None:
                try:self._target_failure(isolated)
                finally:
                    if not self._retain_fs_owner(self._fs_owner._container):
                        self._fs_owner.close(isolated=isolated);self._fs_owner=None

    def _retain_fs_owner(self,helper):
        owner=self._fs_owner
        if owner is None or owner._container is not helper or owner._closed or owner._state in ('birth','release'):return False
        if self._target_owner is None or self._target_owner['state'] in ('planned','absent','stopped'):return False
        # A received failure acknowledgement rechecked the original held root
        # and persisted its marker. Lost protocol/process state is UNKNOWN.
        acknowledged=owner._failure is None and owner._state=='failure'
        observed=dict(state='FAILED_UNUSABLE',reason='ISOLATION_UNCONFIRMED',lock_state='HELD' if acknowledged else 'UNKNOWN',failure_marker_confirmed=acknowledged,no_replay=True)
        helper.record['owner_retained']=copy.deepcopy(observed)
        self._record['owner_retained']=observed
        return True

    def _retire_fs_owner_transport(self,helper):
        import p0c4_source_lifecycle_gate_acceptance as driver
        _require(self._retain_fs_owner(helper),'FULL_OWNER_RETENTION_BINDING')
        owner=self._fs_owner
        owner._failure='HELPER_PROTOCOL';owner._broker.broken=True
        self._record['owner_retained']['lock_state']='UNKNOWN'
        helper.record['owner_retained']['lock_state']='UNKNOWN'
        child=owner._broker.child
        if not child.retired and not child.retire_attempted and (not child.finished or child.reason is not None):
            child.retire(timeout=driver.TRANSPORT_TEARDOWN)



class _FullContainerAdapter:
    __slots__=('_legacy','_context')
    def __init__(self, legacy, context):self._legacy,self._context=legacy,context
    def __getattr__(self, name):return getattr(self._legacy,name)
    def _snapshot_socket_view(self,facts,expected):
        context=self._context
        if not (context._snapshot_effects() and expected.get('builder')is False and expected['name']==context._ident['pg_name']):
            return facts
        project='kwc4c-'+uuid.UUID(context._plan['case_id']).hex
        source=context._snapshot_source_mountpoint;destination='/run/kwc4/sfx'
        _require(context._full_c() and context._ident['case_id']==context._plan['case_id'] and context._ident['project']==project and expected['name']==project+'-pg-1' and facts['Id']==expected['id'] and facts['Name']=='/'+expected['name'],'FULL_SNAPSHOT_SOCKET_IDENTITY')
        # These are the fixed Root prepare_volumes helper's resource facts,
        # carried by the original private context. They confer no business
        # effect/FD authority. The control capsule cannot open host volumes.
        roots=context._record.get('snapshot_root_prepare')
        _require(type(roots)is dict and set(roots)=={'source','snapshot_effects','snapshot_export'},'FULL_SNAPSHOT_SOCKET_PREPARATION')
        for row in roots.values():
            _require(type(row)is dict and set(row)=={'dev','ino','uid','gid','mode'} and all(type(row[k])is int and 0<row[k]<2**64 for k in ('dev','ino')) and all(type(row[k])is int and row[k]==0 for k in ('uid','gid')) and type(row['mode'])is int and row['mode']==0o700,'FULL_SNAPSHOT_SOCKET_PREPARATION')
        _require(len({(v['dev'],v['ino'])for v in roots.values()})==3,'FULL_SNAPSHOT_SOCKET_PREPARATION')
        paths=[]
        for key,suffix,record_key in [('source','source','source_volume'),('snapshot_effects','snapshot-effects','snapshot_effects_volume'),('snapshot_export','snapshot-export','snapshot_export_volume')]:
            volume=context._record['volumes'][record_key]
            original=volume if key=='source'else context._snapshot_volumes[key]
            _require(type(volume)is dict and volume==original and volume['Name']==project+'-'+suffix and _path(volume['Mountpoint']) and volume['Mountpoint'].endswith('/volumes/'+volume['Name']+'/_data') and volume['Driver']=='local' and not volume.get('Options') and volume['Labels']=={'com.docker.compose.project':project,'knowweave.source-lifecycle.batch':context._plan['batch_id']},'FULL_SNAPSHOT_SOCKET_VOLUME')
            paths.append(volume['Mountpoint'])
        _require(paths[0]==source and len(set(paths))==3,'FULL_SNAPSHOT_SOCKET_VOLUME')
        socket=source+'/snapshot-sockets';exception=(socket,destination,'rw')
        binds=facts['HostConfig'].get('Binds');mounts=facts['Mounts']
        _require(type(binds)is list and len(binds)==10 and all(type(v)is str for v in binds),'FULL_SNAPSHOT_SOCKET_BINDS')
        parsed=[tuple(v.split(':'))for v in binds]
        declared={(src,dest,'rw'if rw else'ro')for kind,src,dest,rw in expected['mounts']if kind=='bind'}
        _require(len(expected['mounts'])==18 and len(declared)==10 and all(len(v)==3 for v in parsed) and len(set(parsed))==len(parsed) and set(parsed)==declared and {v for v in parsed if v[2]!='ro'}=={exception},'FULL_SNAPSHOT_SOCKET_BINDS')
        _require(type(mounts)is list and len(mounts)==18 and all(type(v.get('RW'))is bool for v in mounts) and {(v['Type'],v['Source'],v['Destination'],v['RW'])for v in mounts}==expected['mounts'],'FULL_SNAPSHOT_SOCKET_MOUNTS')
        rows=[v for v in mounts if v['Destination']==destination]
        _require(len(rows)==1 and rows[0]['Type']=='bind' and rows[0]['Source']==socket and rows[0]['RW']is True and rows[0].get('Mode')=='rw' and rows[0].get('Propagation')=='rslave','FULL_SNAPSHOT_SOCKET_OPTIONS')
        # Only this verified redundant Binds entry is omitted. Actual Mounts,
        # expected and all legacy identity/security checks remain intact.
        view=copy.deepcopy(facts)
        view['HostConfig']['Binds']=[v for v,parts in zip(binds,parsed)if parts!=exception]
        return view
    def validate_container(self, facts, expected):
        self._legacy.validate_container(self._snapshot_socket_view(facts,expected),expected)
        if expected.get('builder') is False and expected['name']==self._context._ident['pg_name']:
            extra=self._context._source_extra_mounts()
            _require(extra <= expected['mounts'] and len(expected['mounts'])==(18 if self._context._snapshot_effects()else 14),'FULL_SOURCE_DECLARED_MOUNTS')
            volumes=[(str(self._context._target_root),self._context._target_volumes['control']['Name']),('/var/run/knowweave-target',self._context._target_volumes['socket']['Name'])]
            if self._context._snapshot_effects():volumes.extend((destination,self._context._snapshot_volumes[key]['Name'])for key,destination in [('snapshot_effects','/var/lib/knowweave-c4/snapshot-effects-v2'),('snapshot_export','/var/lib/knowweave-c4/snapshot-export-v2')])
            for dest,name in volumes:
                rows=[v for v in facts['HostConfig']['Mounts'] if v.get('Target')==dest]
                _require(len(rows)==1 and rows[0].get('Source')==name and rows[0].get('VolumeOptions')=={'NoCopy':True},'FULL_SOURCE_EXTRA_NOCOPY')


def _fixed_fixture_roles():
    return dict(format_version=1,roles=[dict(name=name,login=name!='learning_auth_lock',inherit=True,superuser=False,createdb=False,createrole=False,bypassrls=False,replication=False,connection_limit=-1) for name in ('learning_admin','learning_auth_lock','learning_runtime')],memberships=[dict(role='learning_auth_lock',member='learning_admin',inherit=False,set=True,admin=False)])


def _fixed_full_c_fixture_roles():
    from . import roles
    value=_fixed_fixture_roles()
    value['format_version']=2
    value['roles'].append(dict(name='learning_snapshot_claim_broker',login=True,inherit=False,superuser=False,createdb=False,createrole=False,bypassrls=False,replication=False,connection_limit=4))
    value['components']=[dict(id='snapshot_c0_d1',version=1,schema_contract_sha256=roles._c0_d1_contract_sha256())]
    roles._validate_recipe(_canonical(value))
    return value


def _measured_identity(path, socket=False):
    import os
    import stat
    meta=os.lstat(path)
    _require(stat.S_ISSOCK(meta.st_mode) if socket else stat.S_ISDIR(meta.st_mode),'FULL_RESOURCE_KIND')
    return dict(path=str(path),dev=meta.st_dev,ino=meta.st_ino,uid=meta.st_uid,gid=meta.st_gid,mode=stat.S_IMODE(meta.st_mode))


def _measured_regular(path, cap):
    import os
    import stat
    fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC)
    try:
        meta=os.fstat(fd)
        _require(stat.S_ISREG(meta.st_mode) and meta.st_nlink==1 and meta.st_uid==0 and meta.st_mode&0o022==0 and 0<meta.st_size<=cap,'FULL_TOOL_FILE')
        raw=bytearray()
        while len(raw)<=cap:
            chunk=os.read(fd,min(65536,cap+1-len(raw)))
            if not chunk:break
            raw.extend(chunk)
        after=os.fstat(fd)
        _require(len(raw)==meta.st_size and (after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns)==(meta.st_dev,meta.st_ino,meta.st_size,meta.st_mtime_ns,meta.st_ctime_ns),'FULL_TOOL_CHANGED')
        return bytes(raw),dict(path=str(path),dev=meta.st_dev,ino=meta.st_ino,uid=meta.st_uid,gid=meta.st_gid,mode=stat.S_IMODE(meta.st_mode))
    finally:os.close(fd)

def validate_plan(value, case):
    from . import plan as base
    import ipaddress
    snapshot=case in SNAPSHOT_CASE_TESTS or case in ATTENTION_CASE_TESTS
    full_c=type(value)is dict and value.get('format_version') in (2,3)
    expected=base.PLAN_KEYS|{'target','schema_profile'} if full_c else base.PLAN_KEYS|{'target'}
    if snapshot:expected=expected|{'snapshot_profile'}
    capability='c4_full_snapshot_import_plan_v2' if snapshot else 'c4_full_c_import_plan_v1' if full_c else 'c4_full_import_plan_v1'
    _require(type(value)is dict and set(value)==expected and case in CASE_TESTS and value['case_name']==case and value['capability']==capability and value['budget_profile']=='full_import_task5_v1','FULL_PLAN_FIELDS')
    if snapshot:_require(type(value['format_version'])is int and value['format_version']==3 and value['snapshot_profile']=='root_snapshot_broker_v2','SNAPSHOT_PLAN_PROFILE')
    else:_require(value['format_version'] in (1,2),'FULL_PLAN_VERSION')
    if full_c:
        _require(type(value['format_version'])is int and value['schema_profile']=='snapshot_c0_d1_v1','FULL_C_PLAN_PROFILE')
    target=value['target']
    _require(type(target) is dict and set(target)=={'case_id','subnet'},'FULL_PLAN_TARGET')
    original={k:v for k,v in value.items() if k not in ('target','schema_profile','snapshot_profile')}
    if full_c:original['format_version']=1
    original.update(case_name='registry-reopen',capability='c4_completion_case_plan_v1',budget_profile='registry_task1_v1')
    base.validate_plan(original,'registry-reopen')
    peer=dict(original,**target);base.validate_plan(peer,'registry-reopen')
    _require(target['case_id'] not in {value['case_id'],value['batch_id'],value['cache_scope']} and not ipaddress.IPv4Network(target['subnet']).overlaps(ipaddress.IPv4Network(value['subnet'])),'FULL_PLAN_DISTINCT_RESOURCES')
    return value


def read_context(path, case):
    from . import evidence
    from pathlib import Path
    raw=evidence.private_read(path,65536)
    value=json.loads(raw,object_pairs_hook=_pairs)
    _require(raw==_canonical(value),'FULL_PLAN_CANONICAL')
    validate_plan(value,case)
    _require(Path(path).parent==Path(value['evidence_root']),'FULL_PLAN_PRIVATE_ROOT')
    return FullRehearsalContext(_TOKEN,value)


def validate_result(plan, result):
    validate_plan(plan,plan['case_name'])
    _require(len(result['cases'])==1,'FULL_RESULT_CASE_COUNT')
    row=result['cases'][0]
    _require(row['test']==CASE_TESTS[plan['case_name']] and row['outcome']['passed']==1 and row['outcome']['failed']==row['outcome']['ignored']==0 and row['outcome']['exit_code']==0,'FULL_RESULT_BODY')
    if plan['case_name'] in ATTENTION_CASE_TESTS:return _validate_attention_result(plan,row)
    target=row.get('full_target')
    _require(type(target) is dict and target.get('stopped') is True and target.get('execs_absent') is True and target.get('network_empty') is True and target['identity']['database']=='learning_restore_c4_'+plan['target']['case_id'],'FULL_RESULT_TARGET')
    if plan['case_name']=='snapshot-history-gap':return _validate_history_gap_result(plan,row,target)
    from pathlib import Path
    from . import evidence
    proof_ref=row.get('full_import_proof')
    proof_path=Path(plan['evidence_root'])/'driver'/('batch-'+plan['batch_id'])/plan['case_id']/'full-import-result.json'
    _require(type(proof_ref) is dict and set(proof_ref)=={'path','sha256'} and proof_ref['path']==str(proof_path),'FULL_RESULT_PROOF_PATH')
    proof_raw=evidence.private_read(proof_path,16384)
    _require(hashlib.sha256(proof_raw).hexdigest()==proof_ref['sha256'],'FULL_RESULT_PROOF_DIGEST')
    proof=json.loads(proof_raw,object_pairs_hook=_pairs);_validate_body_proof(proof,_import_proof_plan(plan),target['status'])
    inventory=json.loads(evidence.private_read(proof_path.parent/'inventory.json',32*1024**2),object_pairs_hook=_pairs)
    manifest=inventory.get('pins/'+proof['backup_id']+'.sealed/manifest.json',{})
    _require(manifest.get('sha256')==proof['manifest_sha256'],'FULL_RESULT_CAPTURE_MANIFEST')
    if proof['negative_evidence'] is not None:
        negative=inventory.get('proofs/full-negative-inventory.json',{})
        _require(negative.get('sha256')==proof['negative_evidence']['sha256'] and negative.get('bytes')==proof['negative_evidence']['bytes'],'FULL_RESULT_NEGATIVE_INVENTORY')
    if plan['case_name'] in RECOVERY_CASE_TESTS:
        recovery_path=proof_path.parent/'full-recovery-result.json'
        recovery_ref=row.get('full_recovery_proof')
        _require(type(recovery_ref) is dict and set(recovery_ref)=={'path','sha256'} and recovery_ref['path']==str(recovery_path),'RECOVERY_RESULT_PROOF_PATH')
        recovery_raw=evidence.private_read(recovery_path,16384)
        _require(hashlib.sha256(recovery_raw).hexdigest()==recovery_ref['sha256'],'RECOVERY_RESULT_PROOF_DIGEST')
        recovery=json.loads(recovery_raw,object_pairs_hook=_pairs)
        _validate_recovery_proof(recovery,plan,target['status'],proof)
        report_path=proof_path.parent/'full-recovery-normalization.json'
        report_ref=row.get('full_recovery_normalization')
        _require(type(report_ref) is dict and set(report_ref)=={'path','sha256'} and report_ref['path']==str(report_path) and report_ref['sha256']==recovery['normalization_report_sha256'],'RECOVERY_RESULT_NORMALIZATION_PATH')
        report=evidence.private_read(report_path,RECOVERY_REPORT_CAP)
        _require(len(report)==recovery['normalization_report_bytes'] and hashlib.sha256(report).hexdigest()==report_ref['sha256'] and type(json.loads(report,object_pairs_hook=_pairs)) is dict,'RECOVERY_RESULT_NORMALIZATION_BYTES')
        for name,raw in [('full-recovery-result.json',recovery_raw),('full-recovery-normalization.json',report)]:
            entry=inventory.get('proofs/'+name,{})
            _require(entry.get('bytes')==len(raw) and entry.get('sha256')==hashlib.sha256(raw).hexdigest(),'RECOVERY_RESULT_NATIVE_INVENTORY')
    return _validate_result_profile(plan,row,target,proof_path,proof,inventory)


def _validate_result_profile(plan,row,target,proof_path,proof=None,inventory=None):
    observation=row.get('full_rehearsal_profile')
    _require(type(observation) is dict and observation['source_mount_count']==(18 if plan['case_name']in (*SNAPSHOT_CASE_TESTS,*ATTENTION_CASE_TESTS) else 14) and observation['target_mount_count']==5,'FULL_RESULT_PROFILE')
    from . import evidence
    from pathlib import Path
    from . import roles
    expected_path=Path(plan['evidence_root'])/'driver'/('batch-'+plan['batch_id'])/plan['case_id']/'full-profile/profile.json'
    _require(observation['path']==str(expected_path),'FULL_RESULT_PROFILE_PATH')
    raw,_=roles._private_read(expected_path,PROFILE_CAP)
    profile=_parse_pinned(raw,observation['sha256'])
    _require(profile['source_case_id']==plan['case_id'] and profile['target_case_id']==plan['target']['case_id'] and profile['application_build_sha256']==plan['source']['application_build_sha256'] and profile['target_container_id']==target['status']['container_id'] and profile['target_birth_sha256']==target['status']['birth_sha256'],'FULL_RESULT_PROFILE_BINDING')
    _require(profile['format_version']==(2 if plan['case_name']in (*SNAPSHOT_CASE_TESTS,*ATTENTION_CASE_TESTS) else 1),'FULL_RESULT_SELECTED_PROFILE')
    if plan['case_name']in SNAPSHOT_CASE_TESTS:
        paths=[('full-snapshot-effect-result.json','full_snapshot_effect_proof'),('full-snapshot-effect-normalization.json','full_snapshot_effect_normalization')]
        returned=[]
        for name,key in paths:
            path=proof_path.parent/name;ref=row.get(key)
            _require(type(ref)is dict and set(ref)=={'path','sha256'} and ref['path']==str(path),'SNAPSHOT_RESULT_PATH')
            data=evidence.private_read(path,RECOVERY_REPORT_CAP)
            entry=inventory.get('proofs/'+name,{})
            _require(hashlib.sha256(data).hexdigest()==ref['sha256'] and entry.get('bytes')==len(data) and entry.get('sha256')==ref['sha256'],'SNAPSHOT_RESULT_NATIVE_INVENTORY')
            returned.append(data)
        snapshot=json.loads(returned[0],object_pairs_hook=_pairs)
        _validate_snapshot_proof(snapshot,plan,target['status'],proof)
        _require(len(returned[1])==snapshot['normalization_report_bytes'] and hashlib.sha256(returned[1]).hexdigest()==snapshot['normalization_report_sha256'],'SNAPSHOT_RESULT_NORMALIZATION_PIN')
    return profile


def _validate_attention_result(plan,row):
    from . import attention,evidence
    from pathlib import Path
    _require(attention.WIRE_CONTRACT is not None,'ATTENTION_PROOF_CONTRACT_NOT_FROZEN')
    target=row.get('full_target')
    _require(type(target)is dict and target.get('stopped')is True and target.get('execs_absent')is True and target.get('network_empty')is True and target['identity']['database']=='learning_restore_c4_'+plan['target']['case_id'],'ATTENTION_RESULT_TARGET')
    directory=Path(plan['evidence_root'])/'driver'/('batch-'+plan['batch_id'])/plan['case_id']
    source_inventory=json.loads(evidence.private_read(directory/'inventory.json',32*1024**2),object_pairs_hook=_pairs);bundle={}
    for name,cap,key in attention.fixed_proof_files(plan['case_name']):
        path=directory/name;ref=row.get(key)
        _require(type(ref)is dict and set(ref)=={'path','sha256'}and ref['path']==str(path),'ATTENTION_RESULT_FIXED_PATH')
        raw=evidence.private_read(path,cap);original=source_inventory.get('proofs/'+name,{})
        _require(hashlib.sha256(raw).hexdigest()==ref['sha256']and type(original.get('bytes'))is int and original['bytes']==len(raw)and original.get('sha256')==ref['sha256'],'ATTENTION_RESULT_ORIGINAL_HASH')
        bundle[name]=raw
    try:
        attention.validate_worker_results(row)
        attention.validate_bundle(bundle,plan,target['status'],row.get('snapshot_capture_byte_audit'))
    except attention.AttentionError as error:raise FullImportError(str(error))from error
    return _validate_result_profile(plan,row,target,directory/'full-attention-result.json')


def _import_proof_plan(plan):
    # A recovery fixture still executes the unmodified fresh Success importer.
    # Its separate import proof never impersonates a recovery validation proof.
    return dict(plan,case_name='full-import') if plan['case_name'] in RECOVERY_CASE_TESTS or plan['case_name']in SNAPSHOT_CASE_TESTS else plan


def _validate_history_gap_proof(proof,target):
    keys={'format_version','classification','case','backup_id','job_id','original_attempts','actual_attempts','original_claim_revoked','new_token_observed','failure_code','opaque_capture_returned','destination_copy_entered','target_import_entered','normalization_entered','target_admitted_clean','target_container_id','stop_required','usable','task6_complete'}
    _require(type(proof)is dict and set(proof)==keys and type(proof['format_version'])is int and proof['format_version']==2 and proof['classification']=='FULL_SNAPSHOT_HISTORY_GAP_REFUSED_NO_CAPTURE_NOT_COMPLETE' and proof['case']=='snapshot-history-gap','SNAPSHOT_GAP_FIELDS')
    _require(_uuid(proof['backup_id'])and _uuid(proof['job_id'])and type(proof['original_attempts'])is int and proof['original_attempts']==1 and type(proof['actual_attempts'])is int and proof['actual_attempts']==2 and proof['failure_code']=='SNAPSHOT_EFFECTS_V2_CONTRACT' and proof['target_container_id']==target['container_id'],'SNAPSHOT_GAP_ACTUAL_HISTORY')
    _require(all(proof[k]is True for k in ('original_claim_revoked','new_token_observed','target_admitted_clean','stop_required')) and all(proof[k]is False for k in ('opaque_capture_returned','destination_copy_entered','target_import_entered','normalization_entered','usable','task6_complete')),'SNAPSHOT_GAP_NO_AUTHORITY')


def _validate_history_gap_result(plan,row,target):
    from pathlib import Path
    from . import evidence,roles
    base=Path(plan['evidence_root'])/'driver'/('batch-'+plan['batch_id'])/plan['case_id']
    path=base/'full-snapshot-history-gap-result.json';ref=row.get('full_snapshot_history_gap_proof')
    _require(type(ref)is dict and set(ref)=={'path','sha256'} and ref['path']==str(path),'SNAPSHOT_GAP_RESULT_PATH')
    raw=evidence.private_read(path,16384);_require(hashlib.sha256(raw).hexdigest()==ref['sha256'],'SNAPSHOT_GAP_RESULT_PIN')
    _validate_history_gap_proof(json.loads(raw,object_pairs_hook=_pairs),target['status'])
    inventory=json.loads(evidence.private_read(base/'inventory.json',32*1024**2),object_pairs_hook=_pairs)
    entry=inventory.get('proofs/full-snapshot-history-gap-result.json',{})
    _require(entry.get('bytes')==len(raw)and entry.get('sha256')==ref['sha256'],'SNAPSHOT_GAP_NATIVE_INVENTORY')
    observation=row.get('full_rehearsal_profile');path=base/'full-profile/profile.json'
    _require(type(observation)is dict and observation['path']==str(path)and observation['source_mount_count']==18 and observation['target_mount_count']==5,'SNAPSHOT_GAP_PROFILE')
    raw,_=roles._private_read(path,PROFILE_CAP)
    profile=_parse_pinned(raw,observation['sha256'])
    _require(profile['format_version']==2 and profile['source_case_id']==plan['case_id']and profile['target_case_id']==plan['target']['case_id']and profile['application_build_sha256']==plan['source']['application_build_sha256']and profile['target_container_id']==target['status']['container_id']and profile['target_birth_sha256']==target['status']['birth_sha256'],'SNAPSHOT_GAP_PROFILE_BINDING')
    return profile


def _validate_snapshot_proof(proof,plan,target,imported):
    keys={'format_version','classification','case','backup_id','manifest_sha256','target_database','target_container_id','job_id','attempts','old_token_rejections','positive_snapshot_findings','snapshot_holds','normalization_report_sha256','normalization_report_bytes','normalization_commit_known','post_normalization_covered_tables','post_normalization_rows_sha256','logical_assets','unique_original_bytes','snapshot_delivery','source_effect_handles_consumed_by_copy','complete_backup','usable','task6_complete','stop_required'}
    _require(type(proof)is dict and set(proof)==keys and type(proof['format_version'])is int and proof['format_version']==2 and proof['classification']=='FULL_SNAPSHOT_EFFECT_NATIVE_CASE_PASSED_EXACT_STOP_PENDING_NOT_COMPLETE','SNAPSHOT_PROOF_FIELDS')
    case=plan['case_name'];_require(case in SNAPSHOT_CASE_TESTS and proof['case']==case,'SNAPSHOT_FIXED_CASE')
    _validate_body_proof(imported,_import_proof_plan(plan),target)
    _require(all(proof[k]==imported[k] for k in ('backup_id','manifest_sha256','target_database','target_container_id')) and _uuid(proof['job_id']),'SNAPSHOT_IMPORT_IDENTITY_JOIN')
    attempts={'snapshot-p1':1,'snapshot-p2':2,'snapshot-p3-live':3,'snapshot-p3-expired':3,'snapshot-p4':3,'snapshot-p5-one':1,'snapshot-p5-two':2,'snapshot-p5-terminal':1,'snapshot-p6':1,'snapshot-queued-zero':0}[case]
    _require(all(type(proof[k])is int for k in ('attempts','old_token_rejections','positive_snapshot_findings','snapshot_holds','logical_assets','unique_original_bytes')) and (proof['attempts'],proof['old_token_rejections'],proof['positive_snapshot_findings'],proof['snapshot_holds'],proof['logical_assets'],proof['unique_original_bytes'])==(attempts,4*attempts,1,0,0,0),'SNAPSHOT_FIXED_COVERAGE')
    _require(proof['normalization_commit_known']is True and proof['source_effect_handles_consumed_by_copy']is True and proof['stop_required']is True and all(proof[k]is False for k in ('complete_backup','usable','task6_complete')),'SNAPSHOT_BOUNDARY')
    _require(_hash(proof['normalization_report_sha256']) and type(proof['normalization_report_bytes'])is int and 0<proof['normalization_report_bytes']<=RECOVERY_REPORT_CAP,'SNAPSHOT_REPORT_PIN')
    _require(type(proof['post_normalization_covered_tables'])is int and proof['post_normalization_covered_tables']==64 and _hash(proof['post_normalization_rows_sha256']),'SNAPSHOT_FINAL_ALL_TABLES')
    d=proof['snapshot_delivery']
    _require(type(d)is dict and set(d)=={'count','deliveries','execution','recovery_copy_receipt_sha256','source_access','runtime_reconnected','republished'} and type(d['count'])is int and d['count']==int(case=='snapshot-p6') and type(d['deliveries'])is list and len(d['deliveries'])==d['count'] and d['execution']=='REAL_SNAPSHOT_DELIVERY_CORE_ON_HELD_MANAGEMENT_TRANSACTION_RECOVERY_CONTROL_COPY' and _hash(d['recovery_copy_receipt_sha256']) and all(d[k]is False for k in ('source_access','runtime_reconnected','republished')),'SNAPSHOT_ACTUAL_DELIVERY')
    if case=='snapshot-p6':
        one=d['deliveries'][0];_require(type(one)is dict and set(one)=={'job_id','stage_token','manifest_sha256','files'} and one['job_id']==proof['job_id'] and _uuid(one['stage_token']) and _hash(one['manifest_sha256']),'SNAPSHOT_DELIVERY_IDENTITY')
        files=one['files'];_require(type(files)is list and len(files)==4 and len({r['path']for r in files})==4,'SNAPSHOT_NONEMPTY_FOUR_FILES')
        _require(all(type(r)is dict and set(r)=={'path','bytes','sha256'} and type(r['path'])is str and r['path'] and '..'not in r['path'].split('/') and not r['path'].startswith('/') and type(r['bytes'])is int and r['bytes']>0 and _hash(r['sha256']) for r in files),'SNAPSHOT_FILE_BYTES')


def _validate_recovery_proof(proof,plan,target,imported):
    keys={'format_version','classification','case','backup_id','manifest_sha256','target_database','target_container_id','stop_required','snapshot_authority','external_effect_bytes_verified','normalization_report_sha256','normalization_report_bytes','normalization_verified','attempt_counts_preserved','running_leases_cleared','source_job_count','old_token_rejections','snapshot_holds','terminal_receipts_unchanged','validation_state','fault_observed','derived_validation','usable','task6_complete'}
    _require(type(proof) is dict and set(proof)==keys and type(proof['format_version']) is int and proof['format_version']==1 and proof['classification']==RECOVERY_PROOF_CLASSIFICATION,'RECOVERY_BODY_PROOF_FIELDS')
    case=plan['case_name']
    _require(case in RECOVERY_CASE_TESTS and proof['case']==case,'RECOVERY_BODY_FIXED_CASE')
    _validate_body_proof(imported,_import_proof_plan(plan),target)
    _require(all(proof[k]==imported[k] for k in ('backup_id','manifest_sha256','target_database','target_container_id')),'RECOVERY_IMPORT_IDENTITY_JOIN')
    _require(proof['stop_required'] is True and proof['snapshot_authority']=='UNAVAILABLE_ALL_SNAPSHOTS_UNKNOWN' and proof['external_effect_bytes_verified'] is False and proof['usable'] is False and proof['task6_complete'] is False,'RECOVERY_FOUNDATION_NO_POSITIVE_AUTHORITY')
    _require(all(proof[k] is True for k in ('normalization_verified','attempt_counts_preserved','running_leases_cleared','terminal_receipts_unchanged')),'RECOVERY_REQUIRED_ASSERTIONS')
    expected={
        'recovery-leases':(4,16,2,'HELD_UNKNOWN_SNAPSHOT_EFFECT',False),
        'recovery-retry-limit':(17,48,6,'HELD_UNKNOWN_SNAPSHOT_EFFECT',False),
        'recovery-effects':(7,8,7,'HELD_UNKNOWN_SNAPSHOT_EFFECT',False),
        'recovery-derived':(2,4,0,'REJECTED_DERIVED_DIGEST',True),
        'recovery-receipt-failure':(2,4,0,'REJECTED_RECEIPT_SYNC',True),
    }[case]
    _require(all(type(proof[k]) is int for k in ('source_job_count','old_token_rejections','snapshot_holds')) and tuple(proof[k] for k in ('source_job_count','old_token_rejections','snapshot_holds','validation_state','fault_observed'))==expected and type(proof['fault_observed']) is bool,'RECOVERY_FIXED_NATIVE_COVERAGE')
    derived=proof['derived_validation']
    if case in ('recovery-derived','recovery-receipt-failure'):
        counts={'result_v1_count':1,'block_asset_use_count':1,'reading_view_revisions':2,'reading_current_revisions':1,'reading_historical_revisions':1,'owner_projections':6,'nonexistent_actor_denials':6,'granted_peer_denials':6,'views_without_granted_peer':0}
        _require(type(derived) is dict and set(derived)==set(counts)|{'reading_report_sha256'} and all(type(derived[k]) is int and derived[k]==v for k,v in counts.items()) and _hash(derived['reading_report_sha256']),'RECOVERY_NONEMPTY_APPLICATION_BARRIERS')
    else:
        _require(derived is None,'RECOVERY_HELD_APPLICATION_BARRIERS_NOT_REACHED')
    _require(_hash(proof['normalization_report_sha256']) and type(proof['normalization_report_bytes']) is int and 0<proof['normalization_report_bytes']<=RECOVERY_REPORT_CAP,'RECOVERY_NORMALIZATION_REPORT_PIN')


def _validate_body_proof(proof,plan,target):
    keys={'format_version','classification','case','backup_id','manifest_sha256','target_database','target_container_id','target_stopped','commit_attempted','rollback_verified','import_catalog_verified','originals_verified','logical_assets','unique_asset_bytes','wrong_endpoint_proof','negative_evidence'}
    _require(type(proof) is dict and set(proof)==keys and type(proof['format_version']) is int and proof['format_version']==1 and proof['classification']=='FULL_IMPORT_CASE_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE','FULL_BODY_PROOF_FIELDS')
    case=plan['case_name']
    _require(case not in RECOVERY_CASE_TESTS,'RECOVERY_REQUIRES_SEPARATE_FOUNDATION_PROOF')
    _require(proof['case']==case and _uuid(proof['backup_id']) and _hash(proof['manifest_sha256']) and proof['target_database']=='learning_restore_c4_'+plan['target']['case_id'] and proof['target_container_id']==target['container_id'] and proof['target_stopped'] is True,'FULL_BODY_PROOF_IDENTITY')
    for field,want in [('commit_attempted',case in ('full-import','full-commit-unknown','full-asset-corrupt')),('rollback_verified',case in ('full-precommit-eof','full-cancel')),('import_catalog_verified',case in ('full-import','full-asset-corrupt')),('originals_verified',case=='full-import')]:
        _require(type(proof[field]) is bool and proof[field] is want,'FULL_BODY_PROOF_OUTCOME')
    snapshot=plan.get('format_version')==3 and plan.get('snapshot_profile')=='root_snapshot_broker_v2' and plan.get('schema_profile')=='snapshot_c0_d1_v1' and case=='full-import'
    _require(type(proof['logical_assets']) is int and proof['logical_assets']==(0 if snapshot else 4) and type(proof['unique_asset_bytes']) is int and proof['unique_asset_bytes']==(0 if snapshot else 96),'FULL_BODY_SOURCE_FIXTURE_INVENTORY')
    _validate_negative_observation(case,proof['negative_evidence'])
    _require(proof['wrong_endpoint_proof']==('different_database_route_only_not_same_id_clone' if case=='full-wrong-endpoint' else None),'FULL_BODY_ENDPOINT_SCOPE')


def _validate_negative_observation(case,negative):
    stages={'full-bad-input':'dump_freeze','full-bad-role':'roles','full-dirty-target':'clean_target','full-wrong-endpoint':'writer_route','full-asset-corrupt':'original_set'}
    if case not in stages:
        _require(negative is None,'FULL_BODY_UNEXPECTED_NEGATIVE')
        return
    _require(type(negative) is dict and set(negative)=={'stage','sha256','bytes','unchanged'} and negative['stage']==stages[case] and _hash(negative['sha256']) and type(negative['bytes']) is int and 0<negative['bytes']<=64*1024**2 and negative['unchanged'] is True,'FULL_BODY_NEGATIVE_BOUNDARY')
