"""Closed test-lane filesystem owner. No Docker, SQL or caller file service."""
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import uuid

_TOKEN=object()
_CONTROLLED_RECIPE=object()
_CAP=1024*1024
_NEXT={'new':{'setup':'setup'},'setup':{'pin':'pin'},'pin':{'files':'files'},
       'files':{'created':'created'},'created':{'check':'created','birth':'birth'},
       'birth':{'release':'released'},'failed':{'release':'released'}}

class FsRejected(RuntimeError):pass
def _require(ok):
    if not ok:raise FsRejected('FS_POLICY')
def _canonical(value):return json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()
def _pairs(pairs):
    value={}
    for key,item in pairs:_require(key not in value);value[key]=item
    return value
def _v4(value):
    try:return type(value) is str and str(uuid.UUID(value))==value and uuid.UUID(value).version==4
    except (ValueError,TypeError,AttributeError):return False
def _path(value):return type(value) is str and value.startswith('/') and '\\' not in value and '\0' not in value and all(v not in ('','.','..') for v in value[1:].split('/'))

_RESPONSE_KEYS={'setup':{'root','lock_held'},'pin':{'precreation_sha256'},'files':{'compose_sha256'},'created':{'recorded'},'check':{'checked'},'birth':{'birth_sha256','issuance_sha256','birth_path','destination_root','control_root','asset_root'},'failure':{'recorded'},'release':{'released'}}
def _response(op,value):
    _require(type(value) is dict and set(value)==_RESPONSE_KEYS[op])
    for key in ('precreation_sha256','compose_sha256','birth_sha256','issuance_sha256'):
        if key in value:_require(type(value[key]) is str and re.fullmatch('[0-9a-f]{64}',value[key]) is not None)
    for key in ('recorded','checked','released','lock_held'):
        if key in value:_require(value[key] is True)
    if op=='setup':
        meta=value['root']
        _require(type(meta) is dict and set(meta)=={'path','dev','ino','uid','gid','mode'} and _path(meta['path']))
        _require(all(type(meta[k]) is int and 0<meta[k]<2**64 for k in ('dev','ino')) and meta['uid']==meta['gid']==0 and meta['mode']==0o700)
    if op=='birth':
        _require(all(_path(value[k]) for k in ('birth_path','destination_root','control_root','asset_root')))

class _Protocol:
    __slots__=('_nonce','_backend','_seq','_state','_failed')
    def __init__(self,token,nonce,backend):
        _require(token is _TOKEN and _v4(nonce))
        self._nonce,self._backend,self._seq,self._state,self._failed=nonce,backend,0,'new',False
    def handle(self,value):
        phase=self._state
        try:
            _require(type(value) is dict and set(value)=={'session','seq','op','value'} and value['session']==self._nonce and type(value['seq']) is int and value['seq']==self._seq and type(value['value']) is dict)
            op=value['op'];_require(type(op) is str)
            if op=='failure':
                _require(self._state not in ('new','released') and not self._failed)
                next_state='failed'
            else:
                _require(not self._failed or op=='release')
                _require(op in _NEXT.get(self._state,{}));next_state=_NEXT[self._state][op]
            phase=op
            result=self._backend.perform(op,value['value'])
            _response(op,result)
            _require(len(_canonical(result))<=_CAP-256)
            self._state=next_state;self._seq+=1
            return dict(session=self._nonce,seq=value['seq'],op=op,ok=True,value=result)
        except BaseException as error:
            self._failed=True
            # Never export exception text, filenames, SQL or secret values.
            code='FS_UNREACHABLE' if isinstance(error,OSError) else 'FS_POLICY'
            return dict(session=self._nonce,seq=self._seq,op=phase,ok=False,code=code)

def _identity(path):
    meta=os.lstat(path)
    _require(stat.S_ISDIR(meta.st_mode) and meta.st_dev>0 and meta.st_ino>0)
    return dict(path=str(path),dev=meta.st_dev,ino=meta.st_ino,uid=meta.st_uid,gid=meta.st_gid,mode=stat.S_IMODE(meta.st_mode))

def _configuration(value):
    _require(type(value) is dict and set(value)=={'batch_id','source_case_id','target_case_id','subnet','root','suite_root','source_root','case_root'})
    _require(all(_v4(value[k]) for k in ('batch_id','source_case_id','target_case_id')) and len({value[k] for k in ('batch_id','source_case_id','target_case_id')})==3)
    _require(all(_path(value[k]) for k in ('root','suite_root','source_root','case_root')))
    name='learning-system-p0c4-restore-'+value['target_case_id']+'_control'
    _require(value['root'].endswith('/volumes/'+name+'/_data'))
    _require(re.fullmatch(r'/[A-Za-z0-9_./-]+/volumes/kwc4c-suite-[0-9a-f]{32}/_data',value['suite_root']) is not None)
    _require(value['source_root'].startswith(value['suite_root']+'/') and value['case_root'].startswith(value['suite_root']+'/') and value['case_root'].endswith('/batch-'+value['batch_id']+'/'+value['source_case_id']))
    return value

def _controlled_configuration(value):
    """Closed two-role paths are derived from the single native Suite."""
    _require(type(value) is dict and set(value)=={'batch_id','source_case_id','target_case_id','subnet','root','suite_root','source_root','case_root','role'})
    _require(all(_v4(value[k]) for k in ('batch_id','source_case_id','target_case_id')) and len({value[k] for k in ('batch_id','source_case_id','target_case_id')})==3)
    _require(value['role'] in ('source','target') and all(_path(value[k]) for k in ('root','suite_root','source_root','case_root')))
    _require(re.fullmatch(r'/[A-Za-z0-9_./-]+/volumes/kwc4c-suite-[0-9a-f]{32}/_data',value['suite_root']) is not None)
    case=value['suite_root']+'/controlled-import/batches/'+value['batch_id']
    _require(value['case_root']==case and value['source_root']==case+'/source' and value['root']==case+'/'+value['role']+'-control')
    import ipaddress
    try:
        network=ipaddress.IPv4Network(value['subnet'],strict=True)
        _require(str(network)==value['subnet'] and network.prefixlen==24 and any(network.subnet_of(ipaddress.IPv4Network(n)) for n in ('10.0.0.0/8','172.16.0.0/12','192.168.0.0/16')))
    except (ValueError,TypeError):raise FsRejected('FS_POLICY') from None
    return value

def _extend_secret_document(document,target):
    pg=document['services']['pg'];keys=('postgres_password','admin_password')
    _require(pg.pop('secrets')==list(keys) and document.pop('secrets')=={key:{'file':str(target/'secrets'/key)} for key in keys})
    pg['volumes'].extend(dict(type='bind',source=str(target/'secrets'/key),target='/run/secrets/'+key,read_only=True,bind={'create_host_path':False}) for key in keys)

def _extend_document(document,identity,target):
    """Fixed full-only model: the manager never opens target secret files."""
    _extend_secret_document(document,target)
    pg=document['services']['pg']
    pg['container_name']=identity['project']+'-pg-1'
    document['volumes']['full_socket']={'name':identity['project']+'_socket','external':True}
    pg['volumes'].append(dict(type='volume',source='full_socket',target='/var/run/postgresql',volume={'nocopy':True}))

class _NativeOwner:
    """Only constructed by the fixed owner entry in its measured namespace."""
    def __init__(self,*,_recipe=None):
        _require(_recipe is None or _recipe is _CONTROLLED_RECIPE)
        self._recipe=_recipe
        self.lock=None;self.root_fd=None;self.config=None;self.status=None
        self._files_ready=False;self._birth_complete=False
    def perform(self,op,value):
        if op=='setup':return self._setup(value)
        self._recheck()
        c=self.config;root=Path(c['root'])
        import p0c4_restore_target as provisioner
        import p0c4_restore_target_pin_prepare as pin
        import p0c4_restore_target_birth as birth
        case_id=c[c['role']+'_case_id'] if self._recipe is _CONTROLLED_RECIPE else c['target_case_id']
        identity=provisioner.identity_for(case_id)
        target=root/'targets'/case_id
        initdb=Path(c['case_root'])/'initdb.sh' if self._recipe is _CONTROLLED_RECIPE else Path(c['case_root'])/'full-profile'/'initdb.sh'
        if op=='pin':
            _require(set(value)=={'before'})
            return dict(precreation_sha256=pin._prepare_locked(root,case_id,c['subnet'],initdb,value['before']))
        if op=='files':
            _require(set(value)=={'before'})
            provisioner.admit_fresh(identity,c['subnet'],value['before'])
            _require(not target.exists())
            target.mkdir(mode=0o700);provisioner._sync_directory(target.parent)
            secrets=target/'secrets';secrets.mkdir(mode=0o700)
            import secrets as entropy
            for name in ('postgres_password','admin_password'):
                path=secrets/name
                fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,0o600)
                with os.fdopen(fd,'wb') as file:
                    file.write((entropy.token_hex(32)+'\n').encode());file.flush()
                    os.fchown(file.fileno(),999,999);os.fsync(file.fileno())
                provisioner._sync_directory(secrets)
            document=provisioner.compose_document(identity,c['subnet'],target,initdb)
            if self._recipe is _CONTROLLED_RECIPE:_extend_secret_document(document,target)
            else:_extend_document(document,identity,target)
            raw=_canonical(document);provisioner._private_write(target/'compose.json',raw)
            self._files_ready=True
            return dict(compose_sha256=hashlib.sha256(raw).hexdigest())
        if op=='created':
            _require(set(value)=={'before','after','ids'})
            ids=provisioner.verify_created(identity,c['subnet'],value['before'],value['after'])
            _require(ids==value['ids'])
            self.status=dict(state='CREATED_QUARANTINED',batch_id=case_id,project=identity['project'],database=identity['database'],network=identity['network'],subnet=c['subnet'],**ids)
            provisioner._private_write(target/'state.json',json.dumps(self.status,sort_keys=True).encode())
            return dict(recorded=True)
        if op=='check':
            _require(set(value)=={'status'} and value['status']==self.status)
            birth._verify_creation_state(root,target,identity,self.status)
            _require(all(not os.path.lexists(target/name) for name in ('control','assets','destination')))
            return dict(checked=True)
        if op=='birth':
            _require(set(value)=={'status','ids','image_id','started_at','volume_id','facts','daemon_id'} and value['status']==self.status)
            _require(all(self.status[k]==v for k,v in value['ids'].items()))
            birth.validate_pg_facts(value['facts'])
            result=birth._publish_birth_files(root,target,identity,self.status,value['ids'],value['image_id'],value['started_at'],value['volume_id'],value['facts'],value['daemon_id'])
            self._recheck();self._birth_complete=True;return result
        if op=='failure':
            _require(set(value)=={'code'} and value['code'] in ('PREPARATION_REJECTED','ISOLATION_UNCONFIRMED'))
            if target.exists() and not (target/'failure.json').exists():
                provisioner._private_write(target/'failure.json',_canonical(dict(state='FAILED_UNUSABLE',code=value['code'])))
            return dict(recorded=True)
        if op=='release':
            _require(not value);self.close();return dict(released=True)
        raise FsRejected('FS_POLICY')
    def _setup(self,value):
        _require(sys.platform=='linux' and os.geteuid()==0 and self.config is None)
        c=_controlled_configuration(value) if self._recipe is _CONTROLLED_RECIPE else _configuration(value)
        _require(Path(c['source_root']).resolve()==Path(__file__).resolve().parents[2])
        sys.path.insert(0,str(Path(c['source_root'])/'scripts'))
        import p0c4_restore_target as provisioner
        import p0c4_restore_target_pin_prepare as pin
        root=Path(c['root']);m=os.lstat(root)
        _require(stat.S_ISDIR(m.st_mode) and m.st_uid==0 and not list(root.iterdir()))
        initdb=Path(c['case_root'])/'initdb.sh' if self._recipe is _CONTROLLED_RECIPE else Path(c['case_root'])/'full-profile'/'initdb.sh'
        reviewed=Path(c['source_root'])/'deploy'/('p0c4_restore_initdb.sh' if self._recipe is _CONTROLLED_RECIPE else 'p0c4_full_restore_initdb.sh')
        import p0c4_restore_target_pin as pin_reader
        if self._recipe is _CONTROLLED_RECIPE:
            provisioner._trusted_root(root)
            _require(pin_reader._initdb_digest(initdb)==pin_reader._initdb_digest(reviewed))
        os.chmod(root,0o700);(root/'targets').mkdir(mode=0o700)
        provisioner._sync_directory(root)
        provisioner._trusted_initdb(initdb)
        if self._recipe is None:_require(pin_reader._initdb_digest(initdb)==pin_reader._initdb_digest(reviewed))
        self.lock=provisioner._locked_root(root);self.lock.__enter__()
        pin._check_setup(root,c[c['role']+'_case_id'] if self._recipe is _CONTROLLED_RECIPE else c['target_case_id'])
        self.root_fd=os.open(root,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
        self.config=c;self.root_identity=_identity(root)
        return dict(root=self.root_identity,lock_held=True)
    def _recheck(self):
        _require(self.config is not None and self.root_fd is not None and _identity(self.config['root'])==self.root_identity)
        m=os.fstat(self.root_fd);_require((m.st_dev,m.st_ino)==(self.root_identity['dev'],self.root_identity['ino']))
    def close(self):
        if self.root_fd is not None:os.close(self.root_fd);self.root_fd=None
        if self.lock is not None:self.lock.__exit__(None,None,None);self.lock=None
    def _disconnect_requires_hold(self):
        return self.root_fd is not None and self._files_ready and not self._birth_complete
    def disconnected(self):
        if self._disconnect_requires_hold():
            # Protocol loss must not voluntarily unlock a possibly live target.
            # This fixed worker accepts no further input or publication. Only
            # subsequent exact isolation/termination can end this retained state.
            try:self.perform('failure',dict(code='ISOLATION_UNCONFIRMED'))
            except BaseException:pass
            import signal
            while True:
                try:signal.pause()
                except KeyboardInterrupt:continue
        self.close()

def _new_controlled_owner():
    return _NativeOwner(_recipe=_CONTROLLED_RECIPE)

def _owner_main(*,_recipe=None):
    _require(_recipe is None or _recipe is _CONTROLLED_RECIPE)
    os.umask(0o077);backend=_new_controlled_owner() if _recipe is _CONTROLLED_RECIPE else _NativeOwner();protocol=None
    try:
        for raw in iter(lambda:sys.stdin.buffer.readline(_CAP+1),b''):
            _require(len(raw)<=_CAP and raw.endswith(b'\n'))
            value=json.loads(raw,object_pairs_hook=_pairs)
            if protocol is None:protocol=_Protocol(_TOKEN,value.get('session'),backend)
            reply=protocol.handle(value)
            print(_canonical(reply).decode(),flush=True)
            if not reply['ok']:return 1
            if protocol._state=='released':return 0
        return 1
    except BaseException:
        return 1
    finally:backend.disconnected()

# A fixed stdlib-only program lets UID999 inspect its PG directories without
# gaining access to Suite0700/root0600 source files or any DAC capability.
MEASURE_PROGRAM=r'''
import hashlib,json,os,stat,sys,uuid
def check(ok):
 if not ok:raise RuntimeError('MEASUREMENT_REJECTED')
def pairs(rows):
 value={}
 for k,v in rows:check(k not in value);value[k]=v
 return value
def canonical(value):return json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()
def validate(value):
 check(type(value) is dict and set(value)=={'phase','source_case_id','target_case_id','roots'})
 for k in ('source_case_id','target_case_id'):
  x=value[k];check(type(x) is str and str(uuid.UUID(x))==x and uuid.UUID(x).version==4)
 check(value['source_case_id']!=value['target_case_id'])
 source='kwc4c-'+value['source_case_id'].replace('-','');target='learning-system-p0c4-restore-'+value['target_case_id']
 names=dict(source_pg=source+'-pg',source_data=source+'-source',source_build=source+'-build',source_registry=source+'-registry',target_pg=target+'_pg',target_control=target+'_control',target_socket=target+'_socket')
 groups={'birth':{'target_pg'},'pg':{'source_pg','target_pg'},'roots':{'source_data','source_build','source_registry','target_control','target_socket'}}
 check(value['phase'] in groups and type(value['roots']) is dict and set(value['roots'])==groups[value['phase']])
 for kind,row in value['roots'].items():
  check(type(row) is dict and set(row)=={'name','path'} and row['name']==names[kind])
  p=row['path'];check(type(p) is str and p.startswith('/') and '\\' not in p and '\0' not in p and all(x not in ('','.','..') for x in p[1:].split('/')) and p.endswith('/volumes/'+row['name']+'/_data'))
 return value
def identity(path,m):return dict(path=path,dev=m.st_dev,ino=m.st_ino,uid=m.st_uid,gid=m.st_gid,mode=stat.S_IMODE(m.st_mode))
def open_root(path):
 fd=os.open('/',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
 try:
  components=path[1:].split('/')
  for index,name in enumerate(components):
   parent=os.fstat(fd);check(stat.S_ISDIR(parent.st_mode) and parent.st_uid==0 and not parent.st_mode&0o022)
   entry=os.stat(name,dir_fd=fd,follow_symlinks=False);check(stat.S_ISDIR(entry.st_mode))
   child=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=fd)
   held=os.fstat(child)
   if (held.st_dev,held.st_ino)!=(entry.st_dev,entry.st_ino):os.close(child);check(False)
   os.close(fd);fd=child
  return fd
 except BaseException:os.close(fd);raise
def measure(path):
 root=open_root(path);m=os.fstat(root)
 check(stat.S_ISDIR(m.st_mode));digest=hashlib.sha256();count=0;size=0
 pending=[('',os.dup(root))];opened=[]
 try:
  while pending:
   prefix,fd=pending.pop();opened.append(fd)
   for name in sorted(os.listdir(fd)):
    check(name not in ('.','..') and '/' not in name and '\\' not in name)
    entry=os.stat(name,dir_fd=fd,follow_symlinks=False);relative=prefix+'/'+name
    check(stat.S_ISDIR(entry.st_mode) or stat.S_ISREG(entry.st_mode) or stat.S_ISSOCK(entry.st_mode))
    count+=1;check(count<=100010)
    digest.update(canonical([relative,entry.st_dev,entry.st_ino,entry.st_uid,entry.st_gid,entry.st_mode,entry.st_size])+b'\n')
    size+=entry.st_size if stat.S_ISREG(entry.st_mode) else 0
    if stat.S_ISDIR(entry.st_mode):
     child=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=fd);held=os.fstat(child)
     if (held.st_dev,held.st_ino)!=(entry.st_dev,entry.st_ino):os.close(child);check(False)
     pending.append((relative,child))
   os.close(fd);opened.remove(fd)
  after=os.stat(path,follow_symlinks=False);check((after.st_dev,after.st_ino,after.st_uid,after.st_gid,after.st_mode)==(m.st_dev,m.st_ino,m.st_uid,m.st_gid,m.st_mode))
  return dict(identity=identity(path,m),tree_sha256=digest.hexdigest(),entries=count,bytes=size)
 finally:
  for fd in opened+[fd for _,fd in pending]:os.close(fd)
  os.close(root)
def main():
 try:
  check(sys.platform=='linux');raw=sys.stdin.buffer.read(16385);check(0<len(raw)<=16384)
  value=validate(json.loads(raw,object_pairs_hook=pairs));check(os.geteuid()==(999 if value['phase'] in ('birth','pg') else 0) and os.getegid()==(999 if value['phase'] in ('birth','pg') else 0))
  result={key:measure(row['path']) for key,row in value['roots'].items()}
  print(canonical(dict(phase=value['phase'],source_case_id=value['source_case_id'],target_case_id=value['target_case_id'],roots=result)).decode(),flush=True)
  return 0
 except BaseException:return 1
if __name__=='__main__':raise SystemExit(main())
'''

class _HelperProfile:
    __slots__=('_kind','_mounts','_command','_interactive','_controlled')
    def __init__(self,token,kind,mounts,command,interactive,*,_controlled=None):
        _require(token is _TOKEN and kind in ('owner','pg','roots','controlled-owner','controlled-native'))
        if kind.startswith('controlled-'):
            from p0c4_completion.controlled_fixture import ControlledFixtureContext
            _require(type(_controlled) is ControlledFixtureContext)
            _controlled._validate_helper_contract(kind,mounts,command,interactive)
        else:_require(_controlled is None)
        self._controlled=_controlled
        self._kind,self._mounts,self._command,self._interactive=kind,tuple(mounts),tuple(command),interactive
    def _request(self,mounts,command,interactive,fs,env):
        _require(tuple(mounts)==self._mounts and tuple(command)==self._command and interactive==self._interactive and not fs and not env)
        if self._controlled is not None:
            self._controlled._validate_helper_contract(self._kind,mounts,command,interactive)
        else:
            _require(all(row[0]=='volume' for row in mounts))
            _require(sum(not row[3] for row in mounts)==(1 if self._kind=='owner' else 0))
    def _args(self,args):
        value=list(args)
        if self._kind in ('owner','controlled-owner'):value.append('--cap-add=CHOWN')
        if self._kind=='controlled-native':value[value.index('--network=none')]='--network=host'
        if self._kind=='pg':value[value.index('--user=0:0')]='--user=999:999'
        return value
    def _checked_default_copy(self,facts):
        import copy
        host=facts['HostConfig'];config=facts['Config']
        _require(config['User']==('999:999' if self._kind=='pg' else '0:0'))
        _require(host.get('CapAdd') in (['CHOWN'],['CAP_CHOWN']) if self._kind in ('owner','controlled-owner') else host.get('CapAdd') in (None,[]))
        _require(host.get('CapDrop') in (['ALL'],['CAP_ALL']) and not host['Privileged'] and host['NetworkMode']==('host' if self._kind=='controlled-native' else 'none') and host.get('PidMode','')=='')
        adjusted=copy.deepcopy(facts);adjusted['Config']['User']='0:0';adjusted['HostConfig']['CapAdd']=None
        if self._kind=='controlled-native':
            _require(set(facts['NetworkSettings']['Networks'])<= {'host'} and not any((facts['NetworkSettings'].get('Ports') or {}).values()))
            adjusted['HostConfig']['NetworkMode']='none';adjusted['NetworkSettings']['Networks']={}
        return adjusted

class _OwnerClient:
    __slots__=('_container','_broker','_nonce','_seq','_state','_closed','_failure')
    def __init__(self,token,container,broker):
        _require(token is _TOKEN);self._container,self._broker=container,broker
        self._nonce=str(uuid.uuid4());self._seq=0;self._state='new';self._closed=False;self._failure=None
    def call(self,op,value):
        _require(not self._closed and self._failure is None)
        try:
            reply=self._broker.call(dict(session=self._nonce,seq=self._seq,op=op,value=value))
            if type(reply) is dict and reply.get('ok') is False:self._failure=reply.get('code') if reply.get('code') in ('FS_POLICY','FS_UNREACHABLE') else 'HELPER_PROTOCOL'
            _require(type(reply) is dict and reply.get('session')==self._nonce and reply.get('seq')==self._seq and reply.get('op')==op and reply.get('ok') is True and set(reply)=={'session','seq','op','ok','value'})
            _response(op,reply['value'])
        except BaseException:
            self._failure=self._failure or 'HELPER_PROTOCOL';raise
        self._seq+=1;self._state=op;return reply['value']
    def held(self):_require(not self._closed and self._failure is None and self._state not in ('new','release'))
    def close(self,isolated=False):
        if self._closed:return
        try:
            if self._failure is None and (self._state=='birth' or self._state=='failure' and isolated):self.call('release',{})
        finally:self._container.close();self._closed=True

def _suite(context):
    text=str(context._case)
    match=re.match(r'(.*/volumes/(kwc4c-suite-[0-9a-f]{32})/_data)/',text)
    _require(match is not None)
    root,name=match.groups();facts=context._runner.inspect('volume',name)
    _require(facts['Name']==name and facts['Mountpoint']==root and facts['Driver']=='local' and not facts.get('Options'))
    return name,root

def _start_owner(context,source):
    import p0c4_source_lifecycle_gate_acceptance as driver
    name,suite=_suite(context);control=context._target_volumes['control']
    mounts=[('volume',control['Name'],control['Mountpoint'],False),('volume',name,suite,True)]
    command=['python3','-u',str(source/'scripts'/'p0c4_completion'/'full_target_fs.py'),'--fixed-owner']
    spec=_HelperProfile(_TOKEN,'owner',mounts,command,True)
    container=driver.HelperContainer(context._h,context._binding,context._runner,context._result,context._images[context._h.BUILDER],mounts,command,interactive=True,_full_profile=spec)
    broker=driver.BrokerClient(container,context._batch/'evidence/logs',context._runner.budget)
    owner=_OwnerClient(_TOKEN,container,broker)
    # Preserve this exact failed channel for fixed primary diagnostics and the
    # lifecycle's reserved isolation path; never retry setup on a new helper.
    context._fs_owner=owner
    owner.call('setup',dict(batch_id=context._plan['batch_id'],source_case_id=context._plan['case_id'],target_case_id=context._plan['target']['case_id'],subnet=context._plan['target']['subnet'],root=control['Mountpoint'],suite_root=suite,source_root=str(source),case_root=str(context._case)))
    return owner

def _measure(context,phase,volumes):
    import p0c4_source_lifecycle_gate_acceptance as driver
    roots={key:dict(name=row['Name'],path=row['Mountpoint']) for key,row in volumes.items()}
    request=dict(phase=phase,source_case_id=context._plan['case_id'],target_case_id=context._plan['target']['case_id'],roots=roots)
    mounts=[('volume',row['name'],row['path'],True) for row in roots.values()]
    command=['python3','-I','-u','-c',MEASURE_PROGRAM]
    spec=_HelperProfile(_TOKEN,'pg' if phase in ('birth','pg') else 'roots',mounts,command,True)
    helper=driver.HelperContainer(context._h,context._binding,context._runner,context._result,context._images[context._h.BUILDER],mounts,command,interactive=True,_full_profile=spec)
    code,out,err=helper.run(stdin=_canonical(request),timeout=30)
    _require(code==0 and not err and 0<len(out)<=_CAP)
    result=json.loads(out,object_pairs_hook=_pairs)
    _require(set(result)=={'phase','source_case_id','target_case_id','roots'} and all(result[k]==request[k] for k in ('phase','source_case_id','target_case_id')) and set(result['roots'])==set(roots))
    for key,value in result['roots'].items():
        _require(type(value) is dict and set(value)=={'identity','tree_sha256','entries','bytes'} and type(value['tree_sha256']) is str and re.fullmatch('[0-9a-f]{64}',value['tree_sha256']) is not None)
        meta=value['identity']
        _require(type(meta) is dict and set(meta)=={'path','dev','ino','uid','gid','mode'} and meta['path']==roots[key]['path'])
        _require(all(type(meta[k]) is int and 0<meta[k]<2**64 for k in ('dev','ino')) and all(type(meta[k]) is int and 0<=meta[k]<2**32 for k in ('uid','gid')) and type(meta['mode']) is int and 0<=meta['mode']<=0o7777)
        _require(type(value['entries']) is int and 0<=value['entries']<=100010 and type(value['bytes']) is int and 0<=value['bytes']<2**64)
    context._record.setdefault('target_measurements',[]).append(result)
    return result['roots']

if __name__=='__main__':
    if sys.argv[1:]==['--fixed-owner']:raise SystemExit(_owner_main())
    if sys.argv[1:]==['--fixed-controlled-owner']:raise SystemExit(_owner_main(_recipe=_CONTROLLED_RECIPE))
    raise SystemExit(2)
