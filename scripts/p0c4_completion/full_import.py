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

CASE_TESTS = MappingProxyType({
    'recovery-leases': 'full_restore::live_tests::recovery_invalidates_unexpired_and_expired_source_tokens',
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
    _require(type(value['format_version']) is int and value['format_version'] == 1 and value['capability'] == 'full_rehearsal_profile_v1', 'FULL_PROFILE_VERSION')
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
    _require(type(value['volumes']) is dict and set(value['volumes']) == set(names), 'FULL_PROFILE_VOLUMES')
    paths = set();identities=set()
    for kind, name in names.items():
        volume = value['volumes'][kind]
        _identity(volume, ('name',))
        _require(volume['name'] == name and volume['path'].endswith('/volumes/'+name+'/_data') and volume['path'] not in paths, 'FULL_PROFILE_VOLUME_BINDING')
        _require((volume['dev'],volume['ino']) not in identities,'FULL_PROFILE_VOLUME_ALIAS')
        paths.add(volume['path']);identities.add((volume['dev'],volume['ino']))
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
    __slots__ = ('_batch', '_binding', '_case', '_case_name', '_expected_initdb', '_h', '_ident', '_initdb', '_plan', '_profile', '_profile_root', '_profile_sha', '_record', '_result', '_runner', '_target_identity', '_target_root', '_target_status', '_target_volumes', '_images', '_source', '_fs_owner', '_target_owner', '_compose_sha', '_phase', '_last_daemon', '_compose_bytes', '_target_created_ids')

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
        self._fs_owner.call('created',dict(before=before,after=after,ids=ids))

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
        return self._fs_owner.call('birth',dict(status=status,ids=ids,image_id=image_id,started_at=started,volume_id=list(volume_id),facts=facts,daemon_id=self._last_daemon))

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
        raw=self._runner.docker('exec','--user','postgres',container_id,'psql','-XqAt','-v','ON_ERROR_STOP=1','--dbname',identity['database'],'-c',roles._ROLE_PROBE)
        observed=json.loads(raw,object_pairs_hook=_pairs)
        expected=_fixed_fixture_roles()
        _require(roles._validate_recipe(json.dumps(observed['recipe']).encode())==expected and observed['database_acl']=='{learning_admin=CTc/learning_admin}' and observed['database_owner']=='learning_admin' and observed['schema_owner']=='pg_database_owner' and observed['runtime_create'] is False and observed['runtime_control_system'] is False and observed['admin_control_system'] is True,'FULL_TARGET_ROLE_FACTS')

    def _extend_source_document(self, document, binds, volume_spec):
        extras=[dict(type='bind',source='/usr/bin/docker',target='/usr/bin/docker',read_only=True),dict(type='bind',source='/var/run/docker.sock',target='/var/run/docker.sock',read_only=True),dict(type='bind',source=self._target_volumes['pg']['Mountpoint'],target=self._target_volumes['pg']['Mountpoint'],read_only=True),dict(type='bind',source=str(self._profile_root),target='/var/lib/knowweave-full-rehearsal',read_only=True),dict(type='bind',source=str(self._initdb),target=str(self._initdb),read_only=True)]
        binds.extend(extras);document['services']['pg']['volumes'].extend(extras)
        for key,destination,readonly in [('control',str(self._target_root),False),('socket','/var/run/knowweave-target',True)]:
            alias='full_'+key;document['volumes'][alias]={'name':self._target_volumes[key]['Name'],'external':True}
            document['services']['pg']['volumes'].append(dict(type='volume',source=alias,target=destination,read_only=readonly,volume={'nocopy':True}))

    def _source_extra_mounts(self):
        return {('volume',str(self._target_root),str(self._target_root),True),('volume',self._target_volumes['socket']['Mountpoint'],'/var/run/knowweave-target',False)}

    def _container_adapter(self, legacy):
        return _FullContainerAdapter(legacy,self)

    def _finalize_profile(self, record, expected):
        import os
        from . import full_target_fs
        self._phase='PROFILE_MEASURE'
        mapping={'source_pg':'volume','source_data':'source_volume','source_build':'build_volume','source_registry':'registry_volume'}
        actual={kind:record['volumes'][key] for kind,key in mapping.items()}
        actual.update({kind:self._target_volumes[key] for kind,key in [('target_pg','pg'),('target_control','control'),('target_socket','socket')]})
        record['full_profile_stage']='PG_MEASURE'
        measured=dict(full_target_fs._measure(self,'pg',{key:actual[key] for key in ('source_pg','target_pg')}))
        record['full_profile_stage']='ROOTS_MEASURE'
        measured.update(full_target_fs._measure(self,'roots',{key:value for key,value in actual.items() if key not in ('source_pg','target_pg')}))
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
        value=dict(format_version=1,capability='full_rehearsal_profile_v1',source_case_id=self._ident['case_id'],target_case_id=self._plan['target']['case_id'],source_container_id=record['container_id'],target_container_id=self._target_status['container_id'],source_network_id=record['network_id'],target_network_id=self._target_status['network_id'],daemon_id=daemon,application_build_sha256=self._plan['source']['application_build_sha256'],target_birth_sha256=self._target_status['birth_sha256'],source_bind_root=source_root,initdb=dict(sha256=hashlib.sha256(initdb).hexdigest(),**initdb_meta),volumes=volumes,docker_client=dict(sha256=hashlib.sha256(docker).hexdigest(),**docker_meta),docker_socket=docker_socket)
        record['full_profile_stage']='PROFILE_VALIDATION'
        self._profile=_validate_profile(value)
        record['full_profile_stage']='PROFILE_PUBLISH'
        self._h.write_new(self._profile_root/'profile.json',_canonical(value),0o444)
        self._profile_sha=hashlib.sha256(_canonical(value)).hexdigest()
        record['full_rehearsal_profile']=dict(path=str(self._profile_root/'profile.json'),sha256=self._profile_sha,source_mount_count=14,target_mount_count=5)
        record['full_profile_stage']='SOURCE_MOUNTS'
        self._source_mount_projection(self._runner.inspect('container',record['container_id'])['Mounts'])
        record['full_profile_stage']='VERIFIED'
        self._phase='COMPLETE'

    def _compile_pins(self, record):
        _require(record['test']==self._selected_test() and record['container_id']==self._profile['source_container_id'],'FULL_COMPILE_PROFILE_SOURCE')
        return dict(KNOWWEAVE_C4_FULL_PROFILE_SHA256=self._profile_sha,KNOWWEAVE_C4_TARGET_BIRTH_SHA256=self._profile['target_birth_sha256'])

    def _consumer_environment(self, ident):
        return dict(KNOWWEAVE_C4_FULL_TARGET_ROOT=str(self._target_root/'targets'/self._plan['target']['case_id']))

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
                _require(row['uid']==0 and row['ppid']==consumers[0]['pid'] and set(row['env_keys']) in expected,'FULL_MANAGEMENT_CHILD')
                pins[row['pid']]=dict(starttime=row['starttime'],exe=row['exe'],env_keys=row['env_keys'])

    def _verify_body(self, record):
        _require(record['test']==self._selected_test() and record['outcome']['passed']==1 and record['outcome']['failed']==record['outcome']['ignored']==0,'FULL_BODY_COUNTS')
        import p0c4_source_lifecycle_gate_acceptance as driver
        code,raw,err=driver.pg_exec(self._runner,record['container_id'],['cat','/var/lib/knowweave-source/proofs/full-import-result.json'])
        _require(code==0 and not err and 0<len(raw)<=16384,'FULL_BODY_OBSERVATION')
        proof=json.loads(raw,object_pairs_hook=_pairs)
        _validate_body_proof(proof,self._plan,self._target_status)
        self._h.write_new(self._case/'full-import-result.json',raw)
        record['full_import_proof']=dict(path=str(self._case/'full-import-result.json'),sha256=hashlib.sha256(raw).hexdigest())
        self._probe_target_stopped(record)

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
    def validate_container(self, facts, expected):
        self._legacy.validate_container(facts,expected)
        if expected.get('builder') is False and expected['name']==self._context._ident['pg_name']:
            extra=self._context._source_extra_mounts()
            _require(extra <= expected['mounts'] and len(expected['mounts'])==14,'FULL_SOURCE_DECLARED_MOUNTS')
            for dest,name in [(str(self._context._target_root),self._context._target_volumes['control']['Name']),('/var/run/knowweave-target',self._context._target_volumes['socket']['Name'])]:
                rows=[v for v in facts['HostConfig']['Mounts'] if v.get('Target')==dest]
                _require(len(rows)==1 and rows[0].get('Source')==name and rows[0].get('VolumeOptions')=={'NoCopy':True},'FULL_SOURCE_EXTRA_NOCOPY')


def _fixed_fixture_roles():
    return dict(format_version=1,roles=[dict(name=name,login=name!='learning_auth_lock',inherit=True,superuser=False,createdb=False,createrole=False,bypassrls=False,replication=False,connection_limit=-1) for name in ('learning_admin','learning_auth_lock','learning_runtime')],memberships=[dict(role='learning_auth_lock',member='learning_admin',inherit=False,set=True,admin=False)])


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
    _require(type(value) is dict and set(value)==base.PLAN_KEYS|{'target'} and case in CASE_TESTS and value['case_name']==case and value['capability']=='c4_full_import_plan_v1' and value['budget_profile']=='full_import_task5_v1','FULL_PLAN_FIELDS')
    target=value['target']
    _require(type(target) is dict and set(target)=={'case_id','subnet'},'FULL_PLAN_TARGET')
    original={k:v for k,v in value.items() if k!='target'}
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
    target=row.get('full_target')
    _require(type(target) is dict and target.get('stopped') is True and target.get('execs_absent') is True and target.get('network_empty') is True and target['identity']['database']=='learning_restore_c4_'+plan['target']['case_id'],'FULL_RESULT_TARGET')
    from pathlib import Path
    from . import evidence
    proof_ref=row.get('full_import_proof')
    proof_path=Path(plan['evidence_root'])/'driver'/('batch-'+plan['batch_id'])/plan['case_id']/'full-import-result.json'
    _require(type(proof_ref) is dict and set(proof_ref)=={'path','sha256'} and proof_ref['path']==str(proof_path),'FULL_RESULT_PROOF_PATH')
    proof_raw=evidence.private_read(proof_path,16384)
    _require(hashlib.sha256(proof_raw).hexdigest()==proof_ref['sha256'],'FULL_RESULT_PROOF_DIGEST')
    proof=json.loads(proof_raw,object_pairs_hook=_pairs);_validate_body_proof(proof,plan,target['status'])
    inventory=json.loads(evidence.private_read(proof_path.parent/'inventory.json',32*1024**2),object_pairs_hook=_pairs)
    manifest=inventory.get('pins/'+proof['backup_id']+'.sealed/manifest.json',{})
    _require(manifest.get('sha256')==proof['manifest_sha256'],'FULL_RESULT_CAPTURE_MANIFEST')
    if proof['negative_evidence'] is not None:
        negative=inventory.get('proofs/full-negative-inventory.json',{})
        _require(negative.get('sha256')==proof['negative_evidence']['sha256'] and negative.get('bytes')==proof['negative_evidence']['bytes'],'FULL_RESULT_NEGATIVE_INVENTORY')
    observation=row.get('full_rehearsal_profile')
    _require(type(observation) is dict and observation['source_mount_count']==14 and observation['target_mount_count']==5,'FULL_RESULT_PROFILE')
    from . import evidence
    from pathlib import Path
    from . import roles
    expected_path=Path(plan['evidence_root'])/'driver'/('batch-'+plan['batch_id'])/plan['case_id']/'full-profile/profile.json'
    _require(observation['path']==str(expected_path),'FULL_RESULT_PROFILE_PATH')
    raw,_=roles._private_read(expected_path,PROFILE_CAP)
    profile=_parse_pinned(raw,observation['sha256'])
    _require(profile['source_case_id']==plan['case_id'] and profile['target_case_id']==plan['target']['case_id'] and profile['application_build_sha256']==plan['source']['application_build_sha256'] and profile['target_container_id']==target['status']['container_id'] and profile['target_birth_sha256']==target['status']['birth_sha256'],'FULL_RESULT_PROFILE_BINDING')
    return profile


def _validate_body_proof(proof,plan,target):
    keys={'format_version','classification','case','backup_id','manifest_sha256','target_database','target_container_id','target_stopped','commit_attempted','rollback_verified','import_catalog_verified','originals_verified','logical_assets','unique_asset_bytes','wrong_endpoint_proof','negative_evidence'}
    _require(type(proof) is dict and set(proof)==keys and type(proof['format_version']) is int and proof['format_version']==1 and proof['classification']=='FULL_IMPORT_CASE_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE','FULL_BODY_PROOF_FIELDS')
    case=plan['case_name']
    _require(proof['case']==case and _uuid(proof['backup_id']) and _hash(proof['manifest_sha256']) and proof['target_database']=='learning_restore_c4_'+plan['target']['case_id'] and proof['target_container_id']==target['container_id'] and proof['target_stopped'] is True,'FULL_BODY_PROOF_IDENTITY')
    for field,want in [('commit_attempted',case in ('full-import','full-commit-unknown','full-asset-corrupt')),('rollback_verified',case in ('full-precommit-eof','full-cancel')),('import_catalog_verified',case in ('full-import','full-asset-corrupt')),('originals_verified',case=='full-import')]:
        _require(type(proof[field]) is bool and proof[field] is want,'FULL_BODY_PROOF_OUTCOME')
    _require(type(proof['logical_assets']) is int and proof['logical_assets']==4 and type(proof['unique_asset_bytes']) is int and proof['unique_asset_bytes']==96,'FULL_BODY_SOURCE_FIXTURE_INVENTORY')
    _validate_negative_observation(case,proof['negative_evidence'])
    _require(proof['wrong_endpoint_proof']==('different_database_route_only_not_same_id_clone' if case=='full-wrong-endpoint' else None),'FULL_BODY_ENDPOINT_SCOPE')


def _validate_negative_observation(case,negative):
    stages={'full-bad-input':'dump_freeze','full-bad-role':'roles','full-dirty-target':'clean_target','full-wrong-endpoint':'writer_route','full-asset-corrupt':'original_set'}
    if case not in stages:
        _require(negative is None,'FULL_BODY_UNEXPECTED_NEGATIVE')
        return
    _require(type(negative) is dict and set(negative)=={'stage','sha256','bytes','unchanged'} and negative['stage']==stages[case] and _hash(negative['sha256']) and type(negative['bytes']) is int and 0<negative['bytes']<=64*1024**2 and negative['unchanged'] is True,'FULL_BODY_NEGATIVE_BOUNDARY')
