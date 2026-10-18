"""Private fixed same-ID clone lane; observations never grant restore authority.

The only supported authentication is the reviewed fixture's exact loopback trust.
There is no caller command, SQL, environment, path, image or budget interface.
"""
import ipaddress
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import time
import uuid
from contextlib import contextmanager

_TOKEN=object()
_CAPABILITY='c4_current11_same_id_clone_plan_v1'
_PROFILE='controlled_fixture_same_id_clone_v1'
_PGDATA='/var/lib/postgresql/18/docker'
_HEX64=re.compile('[0-9a-f]{64}')
_SAME_ID_SQL="SELECT current_database() || '|' || (pg_catalog.pg_control_system()).system_identifier::text || '|' || d.oid::text || '|' || CASE WHEN pg_is_in_recovery() THEN 't' ELSE 'f' END FROM pg_catalog.pg_database d WHERE d.datname=current_database();"

class CloneRejected(RuntimeError):pass

def _require(ok,code):
    if not ok:raise CloneRejected(code)

def _canonical(value):return json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()
def _digest(raw):return hashlib.sha256(raw).hexdigest()

def _v4(value):
    try:return type(value) is str and str(uuid.UUID(value))==value and uuid.UUID(value).version==4
    except (ValueError,TypeError,AttributeError):return False

def _validate_plan(value):
    from p0c4_completion import controlled_fixture as fixture
    _require(type(value) is dict and set(value)==fixture._PLAN_KEYS|{'clone_case_id','clone_subnet'},'CLONE_PLAN_FIELDS')
    _require(value['capability']==_CAPABILITY and value['budget_profile']==_PROFILE and value['case']=='wrong-endpoint','CLONE_PLAN_PROFILE')
    ids=[value[k] for k in ('batch_id','source_case_id','target_case_id','clone_case_id','cache_scope')]
    _require(all(_v4(v) for v in ids) and len(set(ids))==5,'CLONE_PLAN_IDENTITIES')
    # Reuse the unchanged two-role validation, never broaden its accepted keys.
    base={k:value[k] for k in fixture._PLAN_KEYS}
    base.update(capability='c4_current11_first_success_plan_v1',case='success',budget_profile='controlled_fixture_two_role_first_success_v1')
    try:fixture._validate_plan(base)
    except fixture.ControlledRejected:raise CloneRejected('CLONE_PLAN_BINDING') from None
    try:
        nets=[ipaddress.IPv4Network(value[k],strict=True) for k in ('source_subnet','target_subnet','clone_subnet')]
        _require(all(type(value[k]) is str and str(n)==value[k] and n.prefixlen==24 and any(n.subnet_of(ipaddress.IPv4Network(p)) for p in ('10.0.0.0/8','172.16.0.0/12','192.168.0.0/16')) for n,k in zip(nets,('source_subnet','target_subnet','clone_subnet'))) and all(not a.overlaps(b) for i,a in enumerate(nets) for b in nets[i+1:]),'CLONE_PLAN_NETWORKS')
    except (ValueError,TypeError):raise CloneRejected('CLONE_PLAN_NETWORKS') from None
    return value

def _choose_subnets(occupied):
    _require(type(occupied) is list and all(type(v) is str for v in occupied),'CLONE_NETWORK_OBSERVATIONS')
    try:existing=[ipaddress.ip_network(v,strict=False) for v in occupied if v!='default']
    except (ValueError,TypeError):raise CloneRejected('CLONE_NETWORK_OBSERVATIONS') from None
    choices=[str(n) for pool in ('172.29.0.0/16','10.252.0.0/16') for n in ipaddress.ip_network(pool).subnets(new_prefix=24) if not any(v.version==4 and n.overlaps(v) for v in existing)]
    _require(len(choices)>=3,'CLONE_NETWORK_CAPACITY');return choices[:3]

def _clone_identity(identity):
    from p0c4_completion import controlled_fixture as fixture
    _require(type(identity) is dict and set(identity)=={'project','network','volume','database','image'},'CLONE_IDENTITY_FIELDS')
    project=identity['project']
    _require(type(project) is str and project.startswith('learning-system-p0c4-restore-'),'CLONE_IDENTITY_PROJECT')
    suffix=project.removeprefix('learning-system-p0c4-restore-')
    _require(_v4(suffix) and identity['network']==project+'_test' and identity['volume']==project+'_pg' and identity['database']=='learning_restore_c4_'+suffix and identity['image']==fixture._POSTGRES,'CLONE_IDENTITY_BINDING')
    return suffix

def _pg_helper_contract(kind,identity,target_id):
    suffix=_clone_identity(identity)
    _require(type(target_id) is str and _HEX64.fullmatch(target_id) and kind in ('setup','copy','verify'),'CLONE_HELPER_FIXED_KIND')
    programs={
        'setup':f'test ! -e {_PGDATA}; mkdir -p {_PGDATA}; chmod 0700 {_PGDATA}; chown 999:999 /var/lib/postgresql/18 {_PGDATA}',
        'copy':f'test -d {_PGDATA} && test -z "$(ls -A {_PGDATA})"; exec pg_basebackup -D {_PGDATA} -F plain -X stream --no-password -h 127.0.0.1 -p 5432 -U postgres',
        'verify':f'pg_verifybackup {_PGDATA}; test ! -e {_PGDATA}/standby.signal; test ! -e {_PGDATA}/recovery.signal; test "$(cat {_PGDATA}/PG_VERSION)" = 18',
    }
    return dict(kind=kind,name='knowweave-controlled-clone-'+suffix+'-'+kind,project=identity['project'],
                image=identity['image'],user='0:0' if kind=='setup' else '999:999',caps=['CHOWN'] if kind=='setup' else [],
                network='container:'+target_id if kind=='copy' else 'none',
                mounts=[('volume',identity['volume'],'/var/lib/postgresql',kind=='verify')],
                command=['/bin/sh','-ec',programs[kind]])

def _trust_replication(rows):
    import p0c4_restore_pin_acceptance as reference
    try:method=reference._effective_replication_hba(rows)
    except (ValueError,TypeError,KeyError):raise CloneRejected('CLONE_REPLICATION_RULE') from None
    _require(method=='trust','CLONE_TRUST_ONLY')
    return 'EXACT_LOOPBACK_TRUST'

def _same_id_line(raw,expected):
    _require(type(expected) is dict and set(expected)=={'database','pg_system_identifier','database_oid'},'CLONE_SQL_IDENTITY_FIELDS')
    _require(type(expected['database']) is str and re.fullmatch(r'learning_restore_c4_[0-9a-f-]{36}',expected['database']) and
             type(expected['pg_system_identifier']) is str and re.fullmatch('[1-9][0-9]{0,19}',expected['pg_system_identifier']) and
             type(expected['database_oid']) is int and 0<expected['database_oid']<2**32,'CLONE_SQL_IDENTITY')
    line=(expected['database']+'|'+expected['pg_system_identifier']+'|'+str(expected['database_oid'])+'|f\n').encode('ascii')
    _require(type(raw) is bytes and raw==line,'CLONE_SAME_ID_STANDALONE')
    return dict(expected)

def _validate_pg_helper(facts,contract,image,cid):
    _require(type(facts) is dict and type(cid) is str and _HEX64.fullmatch(cid) and facts.get('Id')==cid and facts.get('Name')=='/'+contract['name'] and facts.get('Image')==image['Id'],'CLONE_HELPER_ID')
    c=facts.get('Config',{});h=facts.get('HostConfig',{});s=facts.get('State',{})
    labels={**(image['Config'].get('Labels') or {}),'knowweave.controlled.clone':contract['project'],'knowweave.controlled.clone.helper':contract['kind']}
    _require(c.get('Image')==image['Id'] and c.get('User')==contract['user'] and c.get('Entrypoint')==contract['command'][:1] and c.get('Cmd')==contract['command'][1:] and c.get('Env')==image['Config']['Env'] and c.get('Labels')==labels and c.get('WorkingDir')=='/' and c.get('OpenStdin') is False and c.get('Tty') is False,'CLONE_HELPER_CONFIG')
    _require(not any(v.startswith(('PGPASSFILE=','PGPASSWORD=','POSTGRES_PASSWORD=')) for v in c['Env']),'CLONE_HELPER_NO_PASSFILE')
    cap=h.get('CapAdd') or []
    _require(h.get('NetworkMode')==contract['network'] and h.get('CapDrop') in (['ALL'],['CAP_ALL']) and cap in (contract['caps'],['CAP_'+v for v in contract['caps']]) and h.get('SecurityOpt')==['no-new-privileges'] and h.get('Privileged') is False and h.get('PidMode','')=='' and h.get('ReadonlyRootfs') is True and not h.get('PortBindings') and not h.get('Binds') and not h.get('Devices') and not h.get('DeviceRequests') and h.get('RestartPolicy')=={'Name':'no','MaximumRetryCount':0} and h.get('Memory')==h.get('MemorySwap')==1024**3 and h.get('NanoCpus')==2*10**9 and h.get('PidsLimit')==128 and h.get('Tmpfs')=={'/tmp':'rw,nosuid,nodev,size=16m'},'CLONE_HELPER_PRIVILEGES')
    mounts=facts.get('Mounts',[]);kind,name,dest,readonly=contract['mounts'][0]
    _require(len(mounts)==1 and mounts[0].get('Type')==kind and mounts[0].get('Name')==name and mounts[0].get('Destination')==dest and mounts[0].get('RW') is (not readonly) and type(mounts[0].get('Source')) is str and mounts[0]['Source'].endswith('/volumes/'+name+'/_data'),'CLONE_HELPER_MOUNT')
    _require(not facts.get('ExecIDs') and facts.get('RestartCount')==0 and s.get('Dead') is False and s.get('OOMKilled') is False and s.get('Error')=='' and type(s.get('Running')) is bool and type(s.get('Pid')) is int,'CLONE_HELPER_STATE')
    n=facts.get('NetworkSettings',{})
    _require(not any((n.get('Ports') or {}).values()) and set(n.get('Networks') or {}) <= ({'none'} if contract['network']=='none' else set()),'CLONE_HELPER_NETWORK')
    return facts

def _compose_document(identity,subnet):
    _clone_identity(identity)
    try:
        network=ipaddress.IPv4Network(subnet,strict=True)
        _require(type(subnet) is str and network.prefixlen==24 and str(network)==subnet and any(network.subnet_of(ipaddress.IPv4Network(p)) for p in ('10.0.0.0/8','172.16.0.0/12','192.168.0.0/16')),'CLONE_COMPOSE_SUBNET')
    except (ValueError,TypeError):raise CloneRejected('CLONE_COMPOSE_SUBNET') from None
    return {'name':identity['project'],'services':{'pg':{'image':identity['image'],'pull_policy':'never','user':'999:999',
        'command':['postgres'],'restart':'no','cap_drop':['ALL'],'security_opt':['no-new-privileges'],
        'mem_limit':'1g','memswap_limit':'1g','cpus':2,'pids_limit':128,
        'environment':{'PGDATA':_PGDATA},'volumes':[{'type':'volume','source':identity['volume'],'target':'/var/lib/postgresql','volume':{'nocopy':True}}],
        'networks':['test']}},'networks':{'test':{'name':identity['network'],'internal':True,'ipam':{'config':[{'subnet':subnet}]}}},
        'volumes':{identity['volume']:{'name':identity['volume']}}}

class _RootProtocol:
    """Only original-root setup, one immutable observation and release."""
    def __init__(self,token,nonce,backend):
        _require(token is _TOKEN and _v4(nonce),'CLONE_ROOT_MINT')
        self.nonce,self.backend,self.seq,self.state,self.failed=nonce,backend,0,'new',False
    def handle(self,value):
        op='invalid'
        try:
            _require(not self.failed and type(value) is dict and set(value)=={'session','seq','op','value'} and value['session']==self.nonce and type(value['seq']) is int and value['seq']==self.seq and type(value['value']) is dict,'CLONE_ROOT_FRAME')
            op=value['op'];_require(op in {'new':('setup',),'setup':('birth','release'),'birth':('release',)}.get(self.state,()),'CLONE_ROOT_PHASE')
            result=self.backend.perform(op,value['value'])
            _require(type(result) is dict and len(_canonical(result))<=16384,'CLONE_ROOT_RESULT')
            self.state=op;self.seq+=1
            return dict(session=self.nonce,seq=value['seq'],op=op,ok=True,value=result)
        except BaseException:
            self.failed=True
            return dict(session=self.nonce,seq=self.seq,op=op,ok=False,code='CLONE_ROOT_POLICY')

class _CloneRootOwner:
    def __init__(self):
        _require(sys.platform=='linux' and os.geteuid()==0,'CLONE_ROOT_NATIVE')
        self.source=Path(__file__).resolve().parents[2]
        match=re.fullmatch(r'(/var/lib/docker/volumes/kwc4c-suite-[0-9a-f]{32}/_data)/controlled-import/batches/([0-9a-f-]{36})/source',str(self.source))
        _require(match is not None,'CLONE_ROOT_SOURCE')
        sys.path.insert(0,str(self.source/'scripts'))
        from p0c4_completion import controlled_fixture as f
        self.suite=Path(match[1]);self.batch=self.source.parent
        self.plan,self.pin=f._read_plan(self.suite/'current11/plan.json');_validate_plan(self.plan)
        _require(self.plan['batch_id']==match[2],'CLONE_ROOT_BATCH')
        self.record,self.install_pin=f._read_install(self.suite)
        _require(all(self.plan[k]==self.record[k] for k in ('source_commit','public_archive_sha256','public_manifest_sha256','legacy_archive_sha256','legacy_manifest_sha256','builder','postgres','budget_profile')),'CLONE_ROOT_INSTALL')
        import p0c4_source_admission_gate_acceptance as admission
        raw,_=f._private_bytes(self.suite/'incoming/source-public.zip',32*1024**2,0o400)
        self.manifest,files=admission.verify_package(raw,self.plan['public_archive_sha256'],self.plan['public_manifest_sha256'],_digest(Path(admission.__file__).read_bytes()),'green')
        _require(self.manifest['base_commit']==self.plan['source_commit'] and files['scripts/p0c4_completion/controlled_clone.py']==Path(__file__).read_bytes(),'CLONE_ROOT_SOURCE_BINDING')
        self.source_hash=admission.source_digest(self.source,self.manifest)
        self.root=self.batch/'clone-control';self.fd=None;self.lock=None;self.identity=None;self.released=False
    def recheck(self):
        from p0c4_completion import controlled_fixture as f
        import p0c4_source_admission_gate_acceptance as admission
        plan,pin=f._read_plan(self.suite/'current11/plan.json')
        _require(plan==self.plan and pin==self.pin and f._read_install(self.suite)==(self.record,self.install_pin) and admission.source_digest(self.source,self.manifest)==self.source_hash,'CLONE_ROOT_SOURCE_CHANGED')
        if self.fd is not None:
            m=os.fstat(self.fd);named=os.lstat(self.root)
            _require((m.st_dev,m.st_ino,m.st_uid,stat.S_IMODE(m.st_mode))==self.identity==(named.st_dev,named.st_ino,named.st_uid,stat.S_IMODE(named.st_mode)),'CLONE_ROOT_REPLACED')
    def perform(self,op,value):
        self.recheck()
        from p0c4_completion import controlled_fixture as f
        import p0c4_restore_target as provisioner
        if op=='setup':
            _require(value=={} and self.fd is None and not os.path.lexists(self.root),'CLONE_ROOT_NEW_ONLY')
            self.root.mkdir(mode=0o700);provisioner._sync_directory(self.root.parent);provisioner._trusted_root(self.root)
            self.lock=provisioner._locked_root(self.root);self.lock.__enter__()
            self.fd=os.open(self.root,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
            m=os.fstat(self.fd);self.identity=(m.st_dev,m.st_ino,m.st_uid,stat.S_IMODE(m.st_mode))
            document=_compose_document(provisioner.identity_for(self.plan['clone_case_id']),self.plan['clone_subnet'])
            raw=_canonical(document);f._require(len(raw)<=16384,'CLONE_COMPOSE_CAP')
            path=self.root/'compose.json';fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,0o400)
            try:_require(os.write(fd,raw)==len(raw),'CLONE_COMPOSE_WRITE');os.fsync(fd)
            finally:os.close(fd)
            provisioner._sync_directory(self.root);self.recheck()
            return dict(root=dict(path=str(self.root),dev=m.st_dev,ino=m.st_ino,uid=m.st_uid,gid=m.st_gid,mode=stat.S_IMODE(m.st_mode)),compose_sha256=_digest(raw),lock_held=True)
        _require(self.fd is not None,'CLONE_ROOT_NOT_HELD')
        if op=='birth':
            _validate_birth(value,self.plan)
            _require(value['root']==dict(path=str(self.root),dev=self.identity[0],ino=self.identity[1],uid=0,gid=0,mode=0o700),'CLONE_BIRTH_ROOT')
            raw=_canonical(value);fd=os.open(self.root/'birth.json',os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,0o400)
            try:_require(os.write(fd,raw)==len(raw),'CLONE_BIRTH_WRITE');os.fsync(fd)
            finally:os.close(fd)
            provisioner._sync_directory(self.root);self.recheck();return dict(birth_sha256=_digest(raw))
        _require(op=='release' and value=={},'CLONE_ROOT_RELEASE')
        os.close(self.fd);self.fd=None;self.lock.__exit__(None,None,None);self.lock=None;self.released=True
        return dict(released=True)
    def disconnected(self):
        if self.fd is not None and not self.released:
            import signal
            while True:
                try:signal.pause()
                except KeyboardInterrupt:continue

def _root_main():
    os.umask(0o077);owner=None;protocol=None
    try:
        owner=_CloneRootOwner()
        from p0c4_completion.controlled_fixture import _pairs
        for raw in iter(lambda:sys.stdin.buffer.readline(32769),b''):
            _require(len(raw)<=32768 and raw.endswith(b'\n'),'CLONE_ROOT_FRAME_CAP')
            value=json.loads(raw,object_pairs_hook=_pairs)
            if protocol is None:protocol=_RootProtocol(_TOKEN,value.get('session'),owner)
            reply=protocol.handle(value);print(_canonical(reply).decode(),flush=True)
            if not reply['ok']:return 1
            if protocol.state=='release':return 0
        return 1
    except BaseException:return 1
    finally:
        if owner is not None:owner.disconnected()

class _RootClient:
    def __init__(self,container,broker):
        self.container,self.broker=container,broker;self.nonce=str(uuid.uuid4());self.seq=0;self.state='new';self.failed=False
    def call(self,op,value):
        _require(not self.failed and self.state!='release','CLONE_ROOT_CLIENT_USED')
        try:
            row=self.broker.call(dict(session=self.nonce,seq=self.seq,op=op,value=value))
            _require(type(row) is dict and set(row)=={'session','seq','op','ok','value'} and row['session']==self.nonce and row['seq']==self.seq and row['op']==op and row['ok'] is True and type(row['value']) is dict,'CLONE_ROOT_REPLY')
            self.seq+=1;self.state=op;return row['value']
        except BaseException:self.failed=True;raise
    def held(self):_require(not self.failed and self.state in ('setup','birth') and not self.container.closed,'CLONE_ROOT_OWNER_REQUIRED')
    def release(self):
        self.held();_require(self.call('release',{})==dict(released=True),'CLONE_ROOT_RELEASE_REPLY');self.container.close()

class _PgHelper:
    """Exact pinned postgres helper integrated with original cleanup reservations."""
    def __init__(self,lane,kind):
        lane.held()
        _require(type(lane) is _CloneLane and lane.state=='created' and 0<=len(lane.helper_proofs)<3 and kind==('setup','copy','verify')[len(lane.helper_proofs)] and lane.creation['state']=='known' and lane.creation['id']==lane.ids['container_id'],'CLONE_HELPER_ONE_FIXED_SEQUENCE')
        self.lane=lane;self.runner=lane.context._runner;self.contract=_pg_helper_contract(kind,lane.identity,lane.target_id)
        self.image=lane.image;self.closed=False;self.child=None;self._full_profile=None
        c=self.contract;labels={**(self.image['Config'].get('Labels') or {}),'knowweave.controlled.clone':c['project'],'knowweave.controlled.clone.helper':kind}
        self.expected=dict(id=None,name=c['name'],stdin=False);self.record=dict(id=None,name=c['name'],image=self.image['Id'],removed=False)
        registry=self.runner.ownership;self.owner=registry.acquire('helper',c['name'],self.expected,None,self.record);self.owner['object']=self
        lane.context._result['helpers'].append(self.record);lane.context._result['resource_creation_unknown']=True
        args=['create','--name',c['name'],'--pull=never','--network='+c['network'],'--read-only','--cap-drop=ALL','--security-opt=no-new-privileges','--cpus=2','--memory=1g','--memory-swap=1g','--pids-limit=128','--tmpfs=/tmp:rw,nosuid,nodev,size=16m','--user='+c['user'],'--workdir=/','--entrypoint=/bin/sh']
        for cap in c['caps']:args+=['--cap-add='+cap]
        for key,value in labels.items():args+=['--label',key+'='+value]
        for _,name,dest,readonly in c['mounts']:args+=['--mount','type=volume,src='+name+',dst='+dest+',volume-nocopy'+(',readonly' if readonly else '')]
        import p0c4_source_lifecycle_gate_acceptance as driver
        cid=driver.creation_call(self.runner,self.owner,lambda:self.runner.docker(*args,self.image['Id'],*c['command'][1:],timeout=lane.remaining(30))).decode().strip()
        _require(_HEX64.fullmatch(cid),'CLONE_HELPER_CREATED_ID');self.record['id']=self.expected['id']=cid
        registry.known(self.owner,cid);facts=self.inspect();_require(facts['State']['Status']=='created' and facts['State']['Running'] is False,'CLONE_HELPER_NOT_FRESH')
        lane.context._result['resource_creation_unknown']=False
    def validate(self,facts):
        _validate_pg_helper(facts,self.contract,self.image,self.expected['id'])
        _require(facts['Mounts'][0]['Source']==self.lane.ids['volume_mountpoint'],'CLONE_HELPER_EXACT_VOLUME_ROOT')
        return facts
    def inspect(self,runner=None):return self.validate((runner or self.runner).inspect('container',self.record['id']))
    def run(self):
        self.lane._owner_live();code,out,err=self.runner.run(['/usr/bin/docker','start','-a',self.record['id']],timeout=self.lane.remaining(120))
        self.lane._owner_live()
        facts=self.inspect();_require(code==0 and not err and facts['State']['Status']=='exited' and facts['State']['Running'] is False and facts['State']['Pid']==0 and facts['State']['ExitCode']==0,'CLONE_HELPER_EXIT')
        self.record.update(exit_code=code,stdout_sha256=_digest(out),stderr_sha256=_digest(err),final_inspection_sha256=_digest(_canonical(facts)))
        self.close();return dict(kind=self.contract['kind'],container_id=self.record['id'],stdout_sha256=_digest(out),stderr_sha256=_digest(err),exit_code=0)
    def close(self,phase='close'):
        if self.closed:return
        import p0c4_source_lifecycle_gate_acceptance as driver
        _require(phase in ('close','fallback') and not self.owner.get(phase+'_attempted'),'CLONE_HELPER_CLOSE_REPLAY');self.owner[phase+'_attempted']=True
        clean=driver.CleanupRunner(self.runner,self.owner,phase)
        try:
            facts=self.inspect(clean)
            if facts['State']['Running']:clean.docker('stop','--timeout','5',self.record['id'],timeout=10)
            facts=self.inspect(clean);_require(facts['State']['Running'] is False and facts['State']['Pid']==0,'CLONE_HELPER_STOP_UNKNOWN')
            _require(clean.docker('rm',self.record['id'])==(self.record['id']+'\n').encode(),'CLONE_HELPER_REMOVE')
            _require(self.record['id'] not in clean.docker('ps','-aq','--no-trunc').decode().split(),'CLONE_HELPER_REMOVE_UNKNOWN')
            self.record['removed']=True;self.closed=True;self.runner.ownership.release(self.owner,'absent')
        except BaseException:self.owner['close_failed']=True;raise

class _CloneLane:
    def __init__(self,context):
        from p0c4_completion.controlled_fixture import ControlledFixtureContext
        _require(type(context) is ControlledFixtureContext,'CLONE_CONTEXT_MINT_ONLY')
        context._alive();_validate_plan(context._plan)
        self.context=context;self.plan=context._plan;self.state='new';self.deadline=None;self.owner=None;self.creation=None;self.row=None;self.birth=None;self.isolated=False;self.target_owner=None
    def __reduce_ex__(self,protocol):raise TypeError('clone custody is process-local')
    def remaining(self,maximum=300):
        _require(self.deadline is not None and time.monotonic()<self.deadline,'CLONE_DEADLINE')
        return min(maximum,self.deadline-time.monotonic())
    def held(self):
        self.context._alive();_require(self.context._clone is self and self.state!='unknown','CLONE_ORIGINAL_LANE')
        target=self.context._sides.get('target');_require(target is not None and target['owner'] is not None,'CLONE_TARGET_ORIGINAL_OWNER')
        _require(target['owner'] is self.target_owner,'CLONE_TARGET_OWNER_REPLACED')
        target['owner'].held()
        _require(target['creation']['state']=='known' and target['creation']['id']==target['ids']['container_id'] and self.context._runner.ownership.owners.get(target['creation']['intent']['name']) is target['creation'],'CLONE_TARGET_CREATION')
        if self.owner is not None:self.owner.held()

    def take_target(self,side):
        from p0c4_completion import full_target_fs as fs
        _require(self.target_owner is None and side is self.context._sides['target'] and type(side['owner']) is fs._OwnerClient and side['owner']._state=='birth','CLONE_ORIGINAL_BIRTH_OWNER')
        self.target_owner=side['owner'];self.target_id=side['ids']['container_id']
        self.target_owner_started=self.target_owner._container.inspect()['State']['StartedAt']
        self.target_pg=self.context._runner.inspect('container',self.target_id)
        self.target_started_at=self.target_pg['State']['StartedAt']
        self.held()

    def _owner_live(self):
        self.held()
        pairs=[(self.target_owner._container,self.target_owner._broker,self.context._sides['target']['owner_root'],self.target_owner_started)]
        if self.owner is not None:pairs.append((self.owner.container,self.owner.broker,self.root,self.owner_started))
        for helper,broker,root,started in pairs:
            broker.child.check();_require(not broker.broken and broker.child.p.poll() is None,'CLONE_OWNER_TRANSPORT_LOST')
            facts=helper.inspect(self.context._runner);s=facts['State']
            _require(s['Running'] is True and s['StartedAt']==started and facts.get('RestartCount')==0,'CLONE_OWNER_RESTARTED')
            meta=os.lstat(root['path'])
            _require(stat.S_ISDIR(meta.st_mode) and (meta.st_dev,meta.st_ino,meta.st_uid,meta.st_gid,stat.S_IMODE(meta.st_mode))==(root['dev'],root['ino'],root['uid'],root['gid'],root['mode']),'CLONE_OWNER_ROOT_CHANGED')

    def _run(self,args,*,stdin=None,maximum=30,allowed=(0,)):
        self._owner_live()
        pg=self.context._runner.inspect('container',self.target_id)
        _require(pg['Id']==self.target_id and pg['State']['Running'] is True and pg['State']['StartedAt']==self.target_started_at and pg['RestartCount']==0 and pg['Image']==self.target_pg['Image'],'CLONE_TARGET_LIVE_CHANGED')
        code,out,err=self.context._runner.run(['/usr/bin/docker',*args],stdin=stdin,timeout=self.remaining(maximum),allowed=allowed)
        compose_create=args==['compose','-f','-','-p',self.identity['project'],'create','--no-build','--pull','never','--no-recreate','pg']
        # Compose's ordinary creation progress is stderr. Its exact process is
        # retained, then all three reserved objects are independently verified.
        _require(code in allowed and (not err or compose_create),'CLONE_FIXED_PROCESS');return code,out

    def prepare(self):
        self.held();_require(self.state=='new','CLONE_PREPARE_REPLAY');self.deadline=time.monotonic()+300;self.state='preparing'
        original=self.context._runner;self.context._runner=_deadline_runner(self,original)
        try:
            answer=self._prepare();self.remaining(0.001)
            # The clone-phase send budget is over. The ORIGINAL owner transport
            # keeps its independently bounded lifetime for compile/body/settlement.
            self.owner.broker.budget.refresh_deadline=None
            self.owner.container.runner=original
            return answer
        except BaseException:
            self.state='unknown';self.context._native_broken=True
            raise
        finally:self.context._runner=original

    def _prepare(self):
        from p0c4_completion import controlled_fixture as f,full_target_fs as fs
        import p0c4_source_lifecycle_gate_acceptance as driver
        c=self.context;b=c._backend;target=c._sides['target'];self.identity=b.provisioner.identity_for(self.plan['clone_case_id'])
        _require(f._verify_tools(c._suite)==c._tools,'CLONE_TOOL_PINS')
        self.before=c._snapshot();b.provisioner.admit_fresh(self.identity,self.plan['clone_subnet'],self.before)
        # Capacity is an admission floor, never a claimed kernel disk quota.
        measured=target['record']['pg_root_measurement']['roots']['target_pg']['bytes']
        disk=os.statvfs(b.batch);_require(type(measured) is int and 0<=measured<2**63 and disk.f_bavail*disk.f_frsize>=3*1024**3+measured,'CLONE_PHYSICAL_CAPACITY')
        driver.log_capacity(c._runner.logs,132)
        self.image=c._runner.inspect('image',self.identity['image'])
        _require(self.image['Id']==self.target_pg['Image'],'CLONE_PG_IMAGE')
        _require(any(v.endswith('@'+self.identity['image'].split('@')[1]) for v in self.image.get('RepoDigests',[])),'CLONE_PG_DIGEST')
        # Exact fixed query from the reviewed physical clone reference; no HBA writes.
        _,raw=self._run(['exec','--user','postgres',self.target_id,'psql','-XAt','-v','ON_ERROR_STOP=1','--dbname','postgres','-c',_REPLICATION_SQL])
        _trust_replication(json.loads(raw,object_pairs_hook=f._pairs))
        _require(not os.path.lexists(Path('/run')/('knowweave-c4-clone-'+self.plan['batch_id'])/'replication.pgpass'),'CLONE_PASSFILE_NOT_CREATED')
        mounts,command=c._helper_contract('controlled-clone-owner')
        profile=fs._HelperProfile(fs._TOKEN,'controlled-clone-owner',mounts,command,True,_controlled=c)
        helper=driver.HelperContainer(c._h,c._binding,c._runner,c._result,c._images[f._BUILDER],mounts,command,interactive=True,_full_profile=profile)
        broker=driver.BrokerClient(helper,b.batch/'evidence/logs',driver.CaseBudget())
        broker.budget.refresh_deadline=self.deadline
        owner=_RootClient(helper,broker);self.owner=owner;setup=owner.call('setup',{})
        _require(set(setup)=={'root','compose_sha256','lock_held'} and setup['lock_held'] is True,'CLONE_ROOT_SETUP')
        self.root=setup['root'];self.owner_started=helper.inspect()['State']['StartedAt']
        _require(self.root['path']==str(b.batch/'clone-control') and self.root['uid']==self.root['gid']==0 and self.root['mode']==0o700,'CLONE_ROOT_SETUP_BINDING')
        raw,pin=f._private_bytes(b.batch/'clone-control/compose.json',16384,0o400)
        _require(raw==_canonical(_compose_document(self.identity,self.plan['clone_subnet'])) and pin['sha256']==setup['compose_sha256'],'CLONE_COMPOSE_BINDING');self.compose=raw
        self.row=dict(role='clone',identity=self.identity,control=b.batch/'clone-control',batch_id=self.plan['clone_case_id'],subnet=self.plan['clone_subnet'],container_id=None,before=self.before)
        b.resources.append(self.row);b.clone_record=self.row
        prefix=['compose','-f','-','-p',self.identity['project']]
        self._run(prefix+['config','-q'],stdin=raw)
        self._owner_live();before=c._snapshot();b.provisioner.admit_fresh(self.identity,self.plan['clone_subnet'],before)
        expected=dict(name=self.identity['project']+'-pg-1',labels={'com.docker.compose.project':self.identity['project'],'com.docker.compose.service':'pg'},
                      clone_resources=dict(network=self.identity['network'],volume=self.identity['volume'],subnet=self.plan['clone_subnet'],compose_sha256=_digest(raw)))
        self.creation=c._runner.ownership.acquire('pg',expected['name'],expected,dict(name=self.identity['network']),self.row)
        # The root-held durable intent precedes Compose's first network/volume/PG send.
        b.helper._private_write(b.batch/'clone-control/create.intent.json',_canonical(self.creation['intent']))
        c._result['resource_creation_unknown']=True
        driver.creation_call(c._runner,self.creation,lambda:self._run(prefix+['create','--no-build','--pull','never','--no-recreate','pg'],stdin=raw))
        _require(self.creation['state']=='sent','CLONE_CREATION_NOT_RESERVED')
        after=c._snapshot();self.ids=_created_clone_resources(self.identity,self.plan['clone_subnet'],before,after,self.image)
        c._runner.ownership.known(self.creation,self.ids['container_id']);self.row['container_id']=self.ids['container_id'];c._result['resource_creation_unknown']=False
        self.state='created';self.helper_proofs=[]
        for kind in ('setup','copy','verify'):
            self._owner_live();self._check_created();self.helper_proofs.append(_PgHelper(self,kind).run());self._check_created()
            if kind=='copy':
                _require(not os.path.lexists(Path('/run')/('knowweave-c4-clone-'+self.plan['batch_id'])/'replication.pgpass'),'CLONE_PASSFILE_NOT_CREATED')
        self.measurement=self._measure_pg()
        self._owner_live();self._check_created();_,started=self._run(['start',self.ids['container_id']])
        _require(started==(self.ids['container_id']+'\n').encode(),'CLONE_START_EXACT_ID')
        ready=False
        for _ in range(30):
            code,_=self._run(['exec','--user','999:999',self.ids['container_id'],'pg_isready','-h','127.0.0.1','-U','postgres','-d',target['identity']['database']],allowed=(0,1,2))
            if code==0:ready=True;break
            time.sleep(min(1,self.remaining(1)))
        _require(ready,'CLONE_START_NOT_READY')
        target_path=target['root']/'targets'/target['case_id']/'control'/(target['identity']['database']+'.birth.json')
        target_raw,_=f._private_bytes(target_path,16384,0o600);target_birth=json.loads(target_raw,object_pairs_hook=f._pairs)
        _require(_digest(target_raw)==target['status']['birth_sha256'],'CLONE_TARGET_BIRTH_CHANGED')
        self.sql_identity=dict(database=target_birth['database_name'],pg_system_identifier=target_birth['pg_system_identifier'],database_oid=target_birth['database_oid'])
        for cid in (self.target_id,self.ids['container_id']):
            _,observed=self._run(['exec','--user','postgres',cid,'psql','-XAt','-v','ON_ERROR_STOP=1','--dbname',target['identity']['database'],'-c',_SAME_ID_SQL])
            _same_id_line(observed,self.sql_identity)
        live=c._runner.inspect('container',self.ids['container_id']);self.started_at=live['State']['StartedAt']
        _require(c._runner.inspect('container',self.target_id)['State']['StartedAt']==self.target_started_at,'CLONE_TARGET_RESTARTED')
        birth=dict(format_version=1,capability='c4_controlled_clone_birth_v1',plan_sha256=c._install.plan_pin['sha256'],
                   clone_case_id=self.plan['clone_case_id'],source_birth_sha256=c._sides['source']['status']['birth_sha256'],target_birth_sha256=target['status']['birth_sha256'],
                   root=self.root,ids=self.ids,image_id=self.image['Id'],started_at=self.started_at,target_started_at=self.target_started_at,restart_count=0,
                   sql_identity=self.sql_identity,compose_sha256=_digest(self.compose),pg_measurement=self.measurement,
                   helpers=self.helper_proofs,passfile_state='NOT_CREATED',replication_auth='EXACT_LOOPBACK_TRUST',no_standby=True)
        _validate_birth(birth,self.plan);self.birth=birth;self.recheck()
        reply=self.owner.call('birth',birth);_require(reply==dict(birth_sha256=_digest(_canonical(birth))),'CLONE_BIRTH_PUBLICATION')
        self.birth_sha256=reply['birth_sha256'];self.row['inspection_sha256']=self.birth_sha256;self.row['birth_sha256']=self.birth_sha256
        self.state='born';self.remaining(0.001)
        _require(f._verify_tools(c._suite)==c._tools,'CLONE_TOOL_PINS_AFTER')
        return dict(**self.ids,same_system_identifier=True,same_database_oid=True,passfile_state='NOT_CREATED',backup_verified=True,no_standby=True,clone_birth_sha256=self.birth_sha256)

    def _check_created(self):
        facts=self.context._runner.inspect('container',self.ids['container_id'])
        _validate_clone_pg(facts,self.identity,self.image,self.ids['network_id'],self.ids['volume_mountpoint'],False)
        _require(facts['State']['Status']=='created' and facts['State']['Pid']==0 and facts['RestartCount']==0 and not facts.get('ExecIDs'),'CLONE_NOT_STARTED_BEFORE_VERIFY')

    def _measure_pg(self):
        from p0c4_completion import controlled_fixture as f,full_target_fs as fs
        import p0c4_source_lifecycle_gate_acceptance as driver
        c=self.context;mounts=[('volume',self.identity['volume'],self.ids['volume_mountpoint'],True)]
        command=['python3','-I','-u','-c',_measurement_program()]
        profile=fs._HelperProfile(fs._TOKEN,'pg',mounts,command,True)
        helper=driver.HelperContainer(c._h,c._binding,c._runner,c._result,c._images[f._BUILDER],mounts,command,interactive=True,_full_profile=profile)
        request=dict(phase='birth',source_case_id=self.plan['target_case_id'],target_case_id=self.plan['clone_case_id'],roots=dict(target_pg=dict(name=self.identity['volume'],path=self.ids['volume_mountpoint'])))
        code,out,err=helper.run(stdin=_canonical(request),timeout=self.remaining(30))
        _require(code==0 and not err and 0<len(out)<=1024**2,'CLONE_PG_MEASUREMENT')
        value=json.loads(out,object_pairs_hook=f._pairs)
        _require(set(value)=={'phase','source_case_id','target_case_id','roots'} and all(value[k]==request[k] for k in ('phase','source_case_id','target_case_id')) and set(value['roots'])=={'target_pg'},'CLONE_MEASUREMENT_BINDING')
        return value['roots']['target_pg']

    def recheck(self):
        self._owner_live();_require(self.birth is not None,'CLONE_BIRTH_REQUIRED')
        _validate_birth(self.birth,self.plan)
        _check_live_birth(self.birth,self.plan,self.context._runner)
        # Original source/target births are revalidated by the existing backend too.
        target=self.context._runner.inspect('container',self.target_id)
        _require(target['State']['Running'] is True and target['State']['StartedAt']==self.target_started_at and target['RestartCount']==0,'CLONE_TARGET_RESTARTED')

    def retain(self,helper):
        retained=(not self.isolated or self.state=='unknown') and (self.target_owner is not None and helper is self.target_owner._container or self.owner is not None and helper is self.owner.container)
        if retained:helper.record['owner_retained']=dict(state='UNCONFIRMED_UNUSABLE',lock_state='HELD_OR_UNKNOWN',no_replay=True)
        return retained

    def confirm_isolated(self):
        c=self.context;original=c._runner;deadline=c._backend.commands.isolation_deadline
        budget=_StopDeadline(_TOKEN,deadline);budget.remaining()
        wrapped=_deadline_runner(budget,original);c._runner=wrapped
        persistent=[]
        if self.target_owner is not None:persistent.append((self.target_owner._container,self.target_owner._broker))
        if self.owner is not None:persistent.append((self.owner.container,self.owner.broker))
        previous=[(helper,helper.runner,broker,broker.budget.refresh_deadline) for helper,broker in persistent]
        for helper,broker in persistent:
            helper.runner=wrapped;broker.budget.refresh_deadline=min(broker.budget.refresh_deadline or deadline,deadline)
        try:
            self._confirm_isolated();budget.remaining()
        except BaseException:
            self.state='unknown';c._native_broken=True;raise
        finally:
            c._runner=original
            if self.state!='unknown':
                for helper,runner,broker,refresh in previous:helper.runner=runner;broker.budget.refresh_deadline=refresh

    def _confirm_isolated(self):
        self._owner_live();_require(self.birth is not None and self.creation['state']=='known','CLONE_ISOLATION_AUTHORITY')
        from p0c4_completion import controlled_fixture as f
        raw,_=f._private_bytes(self.context._backend.batch/'clone-control/birth.json',32768,0o400)
        _require(_digest(raw)==self.birth_sha256 and raw==_canonical(self.birth),'CLONE_FINAL_BIRTH_CHANGED')
        for role,cid,volume,started in [('target',self.target_id,self.context._sides['target']['identity']['volume'],self.target_started_at),('clone',self.ids['container_id'],self.identity['volume'],self.started_at)]:
            facts=self.context._inspect('container',[cid])[0];s=facts['State']
            _require(facts['Id']==cid and s['Running'] is False and s['Pid']==0 and not facts.get('ExecIDs') and s['OOMKilled'] is False and s['Dead'] is False and s['Error']=='' and s['StartedAt']==started and facts['RestartCount']==0 and type(s['ExitCode']) is int and s['Status']=='exited','CLONE_EXACT_STOP_REQUIRED')
            _require(self.context._inspect('volume',[volume])[0]['Name']==volume,'CLONE_VOLUME_RETAINED')
            network=self.context._sides['target']['ids']['network_id'] if role=='target' else self.ids['network_id']
            _require(not self.context._inspect('network',[network])[0].get('Containers'),'CLONE_NETWORK_NOT_EMPTY')
            self.row[role+'_final_stop_sha256']=_digest(_canonical(facts))
        self.isolated=True;self.state='isolated';self.owner.release();self.target_owner.close();self.context._sides['target']['owner']=None

_REPLICATION_SQL="""SELECT CASE WHEN current_setting('server_version_num')::int >= 180000
AND current_setting('server_version_num')::int < 190000
AND current_setting('wal_level') IN ('replica','logical') AND current_setting('max_wal_senders')::int >= 2
AND EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname='postgres' AND rolsuper AND rolreplication)
AND pg_catalog.pg_conf_load_time() >= (pg_catalog.pg_stat_file(current_setting('hba_file'))).modification
AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_hba_file_rules WHERE error IS NOT NULL OR file_name IS NULL OR file_name <> current_setting('hba_file'))
THEN (SELECT COALESCE(json_agg(row_to_json(r) ORDER BY r.rule_number),'[]'::json)::text FROM pg_catalog.pg_hba_file_rules r
WHERE r.type IN ('host','hostssl','hostnossl') AND r.database @> ARRAY['replication']::text[]) ELSE 'REJECT' END;"""

def _created_clone_resources(identity,subnet,before,after,image):
    import p0c4_restore_pin_acceptance as reference
    _require(after['daemon_id']==before['daemon_id'],'CLONE_DAEMON_CHANGED')
    project=identity['project'];selected={}
    for plural in ('containers','networks','volumes'):
        rows=[r for r in after[plural] if ((r.get('Config',{}).get('Labels') if plural=='containers' else r.get('Labels')) or {}).get('com.docker.compose.project')==project]
        _require(len(rows)==1,'CLONE_PROJECT_RESOURCE_COUNT');selected[plural]=rows[0]
    pg,net,vol=[selected[k] for k in ('containers','networks','volumes')]
    _require(pg['Id'] not in {r['Id'] for r in before['containers']} and net['Id'] not in {r['Id'] for r in before['networks']} and vol['Name'] not in {r['Name'] for r in before['volumes']},'CLONE_NEW_RESOURCES_ONLY')
    _require(net['Name']==identity['network'] and net['Internal'] is True and len(net['IPAM']['Config'])==1 and net['IPAM']['Config'][0]['Subnet']==subnet and _HEX64.fullmatch(net['Id']) and vol['Name']==identity['volume'] and vol['Driver']=='local' and vol.get('Scope')=='local' and not vol.get('Options'),'CLONE_RESOURCE_TOPOLOGY')
    _validate_clone_pg(pg,identity,image,net['Id'],vol['Mountpoint'],False)
    _require(pg['State']['Status']=='created' and pg['State']['Pid']==0 and pg['RestartCount']==0 and not pg.get('ExecIDs'),'CLONE_CREATED_BEFORE_COPY')
    return dict(container_id=pg['Id'],network_id=net['Id'],volume_name=vol['Name'],volume_mountpoint=vol['Mountpoint'],daemon_id=after['daemon_id'])

def _measurement_program():
    from p0c4_completion.full_target_fs import MEASURE_PROGRAM
    extension='''
original_measure=measure
def measure(path):
 result=original_measure(path);fd=open_root(path)
 try:
  for name in ('18','docker'):
   child=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=fd)
   os.close(fd);fd=child
  m=os.fstat(fd);check(m.st_uid==999 and m.st_gid==999 and stat.S_IMODE(m.st_mode)==0o700)
  result['pgdata_identity']=identity(path+'/18/docker',m)
  return result
 finally:os.close(fd)
'''
    return MEASURE_PROGRAM.replace('def main():',extension+'\ndef main():',1)

def _validate_birth(value,plan):
    _validate_plan(plan)
    keys={'format_version','capability','plan_sha256','clone_case_id','source_birth_sha256','target_birth_sha256','root','ids','image_id','started_at','target_started_at','restart_count','sql_identity','compose_sha256','pg_measurement','helpers','passfile_state','replication_auth','no_standby'}
    _require(type(value) is dict and set(value)==keys and type(value['format_version']) is int and value['format_version']==1 and value['capability']=='c4_controlled_clone_birth_v1' and value['clone_case_id']==plan['clone_case_id'],'CLONE_BIRTH_FIELDS')
    _require(value['plan_sha256']==_digest(_canonical(plan)) and all(type(value[k]) is str and _HEX64.fullmatch(value[k]) and value[k]!='0'*64 for k in ('source_birth_sha256','target_birth_sha256','compose_sha256')) and value['source_birth_sha256']!=value['target_birth_sha256'],'CLONE_BIRTH_PINS')
    _require(value['passfile_state']=='NOT_CREATED' and value['replication_auth']=='EXACT_LOOPBACK_TRUST' and value['no_standby'] is True and type(value['restart_count']) is int and value['restart_count']==0,'CLONE_BIRTH_COPY_FACTS')
    ids=value['ids'];_require(type(ids) is dict and set(ids)=={'container_id','network_id','volume_name','volume_mountpoint','daemon_id'} and all(type(ids[k]) is str and _HEX64.fullmatch(ids[k]) for k in ('container_id','network_id')),'CLONE_BIRTH_RESOURCES')
    project='learning-system-p0c4-restore-'+plan['clone_case_id'];_require(ids['volume_name']==project+'_pg' and type(ids['volume_mountpoint']) is str and ids['volume_mountpoint'].endswith('/volumes/'+project+'_pg/_data') and type(ids['daemon_id']) is str and ids['daemon_id'],'CLONE_BIRTH_VOLUME')
    _require(type(value['image_id']) is str and re.fullmatch('sha256:[0-9a-f]{64}',value['image_id']) and all(type(value[k]) is str and value[k] not in ('','0001-01-01T00:00:00Z') for k in ('started_at','target_started_at')),'CLONE_BIRTH_LIVE_PIN')
    root=value['root'];_require(type(root) is dict and set(root)=={'path','dev','ino','uid','gid','mode'} and type(root['path']) is str and root['path'].endswith('/controlled-import/batches/'+plan['batch_id']+'/clone-control') and root['uid']==root['gid']==0 and root['mode']==0o700 and all(type(root[k]) is int and 0<root[k]<2**64 for k in ('dev','ino')),'CLONE_BIRTH_CONTROL_ROOT')
    sql=value['sql_identity'];_require(sql['database']=='learning_restore_c4_'+plan['target_case_id'],'CLONE_BIRTH_DATABASE')
    _same_id_line((sql['database']+'|'+sql['pg_system_identifier']+'|'+str(sql['database_oid'])+'|f\n').encode(),sql)
    measurement=value['pg_measurement'];_require(type(measurement) is dict and set(measurement)=={'identity','tree_sha256','entries','bytes','pgdata_identity'} and type(measurement['tree_sha256']) is str and _HEX64.fullmatch(measurement['tree_sha256']) and type(measurement['entries']) is int and 0<measurement['entries']<=100010 and type(measurement['bytes']) is int and 0<measurement['bytes']<2**63,'CLONE_BIRTH_PG_MEASUREMENT')
    for key,uid,mode,path in [('identity',0,0o755,ids['volume_mountpoint']),('pgdata_identity',999,0o700,ids['volume_mountpoint']+'/18/docker')]:
        row=measurement[key];_require(type(row) is dict and set(row)=={'path','dev','ino','uid','gid','mode'} and row['path']==path and row['uid']==row['gid']==uid and row['mode']==mode and all(type(row[k]) is int and 0<row[k]<2**64 for k in ('dev','ino')),'CLONE_BIRTH_PG_INODE')
    proofs=value['helpers'];_require(type(proofs) is list and len(proofs)==3,'CLONE_BIRTH_HELPERS')
    for proof,kind in zip(proofs,('setup','copy','verify')):
        _require(type(proof) is dict and set(proof)=={'kind','container_id','stdout_sha256','stderr_sha256','exit_code'} and proof['kind']==kind and type(proof['exit_code']) is int and proof['exit_code']==0 and all(type(proof[k]) is str and _HEX64.fullmatch(proof[k]) for k in ('container_id','stdout_sha256','stderr_sha256')) and proof['stderr_sha256']==_digest(b''),'CLONE_BIRTH_HELPER_PROOF')
    _require(len({r['container_id'] for r in proofs}|{ids['container_id']})==4,'CLONE_BIRTH_DISTINCT_HELPERS')
    return value

def _check_live_birth(birth,plan,runner):
    import p0c4_restore_target as target
    import p0c4_restore_pin_acceptance as reference
    identity=target.identity_for(plan['clone_case_id']);ids=birth['ids']
    pg=runner.inspect('container',ids['container_id']);net=runner.inspect('network',ids['network_id']);vol=runner.inspect('volume',ids['volume_name'])
    image=runner.inspect('image',identity['image']);_require(image['Id']==birth['image_id'],'CLONE_LIVE_IMAGE')
    _validate_clone_pg(pg,identity,image,ids['network_id'],ids['volume_mountpoint'],True)
    _require(pg['State']['StartedAt']==birth['started_at'] and pg['RestartCount']==0 and not pg.get('ExecIDs') and pg['State']['OOMKilled'] is False and pg['State']['Dead'] is False and pg['State']['Error']=='','CLONE_LIVE_STARTED_AT')
    _require(net['Name']==identity['network'] and net['Internal'] is True and len(net['IPAM']['Config'])==1 and net['IPAM']['Config'][0]['Subnet']==plan['clone_subnet'] and set(net.get('Containers') or {})=={ids['container_id']} and vol['Name']==identity['volume'] and vol['Mountpoint']==ids['volume_mountpoint'] and vol['Driver']=='local' and not vol.get('Options'),'CLONE_LIVE_TOPOLOGY')
    return pg

def _validate_clone_pg(pg,identity,image,network_id,mountpoint,running):
    import p0c4_restore_pin_acceptance as reference
    reference._verify_clone_pg(pg,identity,None,network_id,mountpoint,running=running,image_id=image['Id'],uid=999,gid=999)
    h=pg['HostConfig'];config=pg['Config']
    _require(h.get('CapAdd') in (None,[]) and h.get('PidMode','')=='' and not h.get('Devices') and not h.get('DeviceRequests') and h.get('RestartPolicy')=={'Name':'no','MaximumRetryCount':0} and h.get('Memory')==h.get('MemorySwap')==1024**3 and h.get('NanoCpus')==2*10**9 and h.get('PidsLimit')==128,'CLONE_PG_EXACT_PRIVILEGES')
    def env(values):
        _require(type(values) is list and all(type(v) is str and '=' in v for v in values),'CLONE_PG_ENV')
        pairs=[v.split('=',1) for v in values];_require(len({p[0] for p in pairs})==len(pairs),'CLONE_PG_ENV_DUPLICATE');return dict(pairs)
    expected=env(image['Config']['Env']);expected['PGDATA']=_PGDATA
    _require(config.get('Entrypoint')==image['Config'].get('Entrypoint') and env(config['Env'])==expected,'CLONE_PG_FIXED_IMAGE_ENV')
    return pg

@contextmanager
def _native_route(plan,batch):
    from p0c4_completion import controlled_fixture as f
    import p0c4_controlled_import_acceptance as runner
    _validate_plan(plan);raw,pin=f._private_bytes(batch/'clone-control/birth.json',32768,0o400);birth=json.loads(raw,object_pairs_hook=f._pairs)
    _require(raw==_canonical(birth),'CLONE_NATIVE_CANONICAL');_validate_birth(birth,plan)
    dispatch_raw,_=f._private_bytes(batch/'body.dispatched',16384,0o600);dispatch=json.loads(dispatch_raw,object_pairs_hook=f._pairs)
    _require(dispatch.get('case')=='wrong-endpoint' and dispatch.get('clone_birth_sha256')==pin['sha256'],'CLONE_NATIVE_DISPATCH')
    class NativeInspect:
        def __init__(self):self.commands=runner.BoundedCommands()
        def inspect(self,kind,identity):
            rows=self.commands.inspect(kind,[identity]);_require(len(rows)==1,'CLONE_NATIVE_INSPECT');return rows[0]
    _check_live_birth(birth,plan,NativeInspect())
    for role in ('source','target'):
        cid=plan[role+'_case_id'];database='learning_restore_c4_'+cid
        raw,_=f._private_bytes(batch/(role+'-control')/'targets'/cid/'control'/(database+'.birth.json'),16384,0o600)
        _require(_digest(raw)==birth[role+'_birth_sha256'],'CLONE_NATIVE_FRESH_BIRTH')
    project='learning-system-p0c4-restore-'+plan['clone_case_id']
    held=[]
    try:
        for key in ('identity','pgdata_identity'):
            expected=birth['pg_measurement'][key];path=Path(expected['path'])
            _require(path==path.resolve(strict=True),'CLONE_NATIVE_PG_PATH')
            fd=_open_native_inode(path,expected);held.append((fd,path,expected))
        yield {'KNOWWEAVE_C4_CLONE_CONTAINER_ID':birth['ids']['container_id'],'KNOWWEAVE_C4_CLONE_NETWORK_ID':birth['ids']['network_id'],
               'KNOWWEAVE_C4_CLONE_NETWORK_NAME':project+'_test','KNOWWEAVE_C4_CLONE_PROJECT':project,'KNOWWEAVE_C4_CLONE_VOLUME_NAME':project+'_pg','KNOWWEAVE_C4_CLONE_SUBNET':plan['clone_subnet']}
        final,after=f._private_bytes(batch/'clone-control/birth.json',32768,0o400)
        _require(final==_canonical(birth) and after==pin,'CLONE_NATIVE_BIRTH_CHANGED')
        for fd,path,expected in held:_check_held_inode(fd,path,expected)
    finally:
        for fd,_,_ in held:os.close(fd)

def _check_held_inode(fd,path,expected):
    for meta in (os.fstat(fd),os.lstat(path)):
        _require(stat.S_ISDIR(meta.st_mode) and (meta.st_dev,meta.st_ino,meta.st_uid,meta.st_gid,stat.S_IMODE(meta.st_mode))==tuple(expected[k] for k in ('dev','ino','uid','gid','mode')),'CLONE_NATIVE_PG_INODE_CHANGED')

def _deadline_runner(lane,runner):
    import p0c4_source_lifecycle_gate_acceptance as driver
    class CloneDeadlineRunner(driver.BudgetRunner):
        @property
        def ownership(self):return self.runner.ownership
        @property
        def logs(self):return self.runner.logs
        @property
        def counter(self):return self.runner.counter
    return CloneDeadlineRunner(runner,lane)

class _StopDeadline:
    def __init__(self,token,deadline):
        _require(token is _TOKEN and type(deadline) in (int,float) and 0<deadline<float('inf'),'CLONE_EXISTING_STOP_DEADLINE')
        self.deadline=deadline
    def remaining(self):
        left=self.deadline-time.monotonic();_require(left>0,'CLONE_STOP_DEADLINE');return left

def _open_native_inode(path,expected):
    # O_PATH binds identity without reading UID999/0700 PGDATA. Native body's
    # root UID has ALL capabilities dropped and must not gain a DAC bypass.
    _require(sys.platform=='linux','CLONE_NATIVE_INODE_PLATFORM')
    fd=os.open(path,os.O_PATH|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
    try:_check_held_inode(fd,path,expected);return fd
    except BaseException:os.close(fd);raise

if __name__=='__main__':
    sys.dont_write_bytecode=True
    if sys.argv[1:]==['--fixed-clone-owner']:raise SystemExit(_root_main())
    raise SystemExit(2)
