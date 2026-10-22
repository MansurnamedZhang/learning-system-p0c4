"""Fixed snapshot rehearsal worker transport; never an effect classifier.

Only the host controller has Docker authority. The metadata broker validates
separate request/ready files and never receives the two export volumes.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import time
import uuid

PROFILE='root_snapshot_broker_v2'
ROOT='/var/lib/knowweave-source'
SOCKET_ROOT='/run/kwc4/sfx'
CLIENT_ROOT='/run/kwc4-client'
WORKER='/usr/local/bin/learning-worker'
DISPOSITIONS=('NO_WORK','COMPLETED','NOT_ACQUIRED','REVOKED_BEFORE_WRITE')
REQUEST_KEYS={'format_version','operation','backup_id','database','compose_project','request_id','epoch_id','binding_id','work_socket_path','lease_ms'}
RESULT_KEYS={'op','request_id','client_exit','disposition','client_stdout_sha256','client_stderr_sha256','client_container_id','worker_binary_sha256'}
HEX=re.compile(r'[0-9a-f]{64}\Z')
MAX_SIGNALS=4

def _driver():
    if __package__.startswith('scripts.'):
        from .. import p0c4_source_lifecycle_gate_acceptance as driver
    else:
        import p0c4_source_lifecycle_gate_acceptance as driver
    return driver
class SnapshotDriverError(_driver().GateError): pass
def require(ok,token):
    if not ok:raise SnapshotDriverError(token)
def canonical(row):return json.dumps(row,sort_keys=True,separators=(',',':')).encode()
def digest(raw):return hashlib.sha256(raw).hexdigest()
def _pairs(pairs):
    result={}
    for key,value in pairs:
        require(key not in result,'SNAPSHOT_DUPLICATE_KEY');result[key]=value
    return result
def _v4(value):
    require(type(value)is str,'SNAPSHOT_UUID')
    try:parsed=uuid.UUID(value)
    except (ValueError,TypeError,AttributeError):raise SnapshotDriverError('SNAPSHOT_UUID') from None
    require(parsed.version==4 and str(parsed)==value,'SNAPSHOT_UUID')
    return value
def validate_request(raw,filename,ident):
    require(type(raw)is bytes and 0<len(raw)<=4096,'SNAPSHOT_REQUEST_BOUND')
    row=json.loads(raw,object_pairs_hook=_pairs)
    require(type(row)is dict and set(row)==REQUEST_KEYS,'SNAPSHOT_REQUEST_FIELDS')
    require(type(row['format_version'])is int and row['format_version']==2 and row['operation']=='snapshot_broker_signal','SNAPSHOT_REQUEST_VERSION')
    for key in ('backup_id','request_id','epoch_id','binding_id'):_v4(row[key])
    require(filename=='snapshot-request-'+row['request_id']+'.json' and row['database']==ident['database'] and row['compose_project']==ident['project'],'SNAPSHOT_REQUEST_IDENTITY')
    require(type(row['lease_ms'])is int and row['lease_ms'] in (30000,180000),'SNAPSHOT_REQUEST_LEASE')
    expected=SOCKET_ROOT+'/'+row['binding_id']+'/'+row['epoch_id']+'/work.sock'
    require(row['work_socket_path']==expected and len(expected.encode())<108,'SNAPSHOT_SOCKET_FIXED_PATH')
    return row
def scheduling_output(disposition):
    require(disposition in DISPOSITIONS,'SNAPSHOT_DISPOSITION')
    return ('SNAPSHOT_BROKER_SIGNAL_OK disposition='+disposition+'\n').encode()
def validate_client_exit(code,out,err):
    require(type(code)is int and code==0 and err==b'','SNAPSHOT_WORKER_NATURAL_EXIT')
    matches=[value for value in DISPOSITIONS if out==scheduling_output(value)]
    require(len(matches)==1,'SNAPSHOT_WORKER_TYPED_STDOUT')
    return matches[0]

class BrokerProtocol:
    """Root metadata only. No SQL, export handle, Docker call or capability."""
    def __init__(self,root,ident):self.root,self.ident=Path(root),dict(ident);self.requests={};self.completed=set();self.partial={}
    def pin_credentials(self):
        d=_driver();require(not hasattr(self,'credential_pins'),'SNAPSHOT_CREDENTIAL_PIN_ONCE');pins={}
        for name,role in [('broker-dsn','learning_snapshot_claim_broker'),('runtime-dsn','learning_runtime')]:
            raw,m=d.private_read(self.root/name,4096)
            pins[name]=dict(role=role,dev=m.st_dev,ino=m.st_ino,bytes=len(raw),sha256=digest(raw),uid=m.st_uid,gid=m.st_gid,mode=stat.S_IMODE(m.st_mode),nlink=m.st_nlink)
        self.credential_pins=pins
        raw=canonical(dict(format_version=2,capability='snapshot_fixture_credential_pins_v2',files=pins))
        d.new_private(self.root/'proofs/snapshot-credential-pins.json',raw);self.credential_proof=raw
        self.audit_credentials()
    def audit_credentials(self):
        d=_driver()
        for name,pin in self.credential_pins.items():
            raw,m=d.private_read(self.root/name,4096)
            require(dict(role=pin['role'],dev=m.st_dev,ino=m.st_ino,bytes=len(raw),sha256=digest(raw),uid=m.st_uid,gid=m.st_gid,mode=stat.S_IMODE(m.st_mode),nlink=m.st_nlink)==pin,'SNAPSHOT_CREDENTIAL_CHANGED')
        raw,_=d.private_read(self.root/'proofs/snapshot-credential-pins.json',4096)
        require(raw==self.credential_proof,'SNAPSHOT_CREDENTIAL_PROOF_CHANGED')
    def poll(self):
        d=_driver();found=[]
        for path in sorted((self.root/'proofs').glob('snapshot-request-*.json')):
            if path.name in self.completed:continue
            if any(value['name']==path.name for value in self.requests.values()):continue
            raw,meta=d.private_read(path,4096,allow_empty=True)
            if d.incomplete_publication(raw):
                first=self.partial.setdefault(path.name,time.monotonic())
                require(time.monotonic()-first<2,'SNAPSHOT_REQUEST_PARTIAL_TIMEOUT');continue
            raw2,meta2=d.private_read(path,4096,allow_empty=True)
            if raw!=raw2 or (meta.st_dev,meta.st_ino,meta.st_mtime_ns,meta.st_ctime_ns)!=(meta2.st_dev,meta2.st_ino,meta2.st_mtime_ns,meta2.st_ctime_ns):continue
            self.partial.pop(path.name,None)
            row=validate_request(raw,path.name,self.ident)
            require(len(self.completed)+len(self.requests)<MAX_SIGNALS and not self.requests,'SNAPSHOT_REQUEST_SINGLE_FLIGHT')
            relative=Path('snapshot-sockets')/row['binding_id']/row['epoch_id']/'work.sock'
            socket=self.root/relative;socket_meta=socket.lstat()
            require(stat.S_ISSOCK(socket_meta.st_mode) and socket_meta.st_uid==0 and socket_meta.st_gid==65532 and stat.S_IMODE(socket_meta.st_mode)==0o660 and socket_meta.st_nlink==1,'SNAPSHOT_SOCKET_LEAF')
            for parent in (self.root/'snapshot-sockets',socket.parent.parent,socket.parent):
                m=parent.lstat();require(stat.S_ISDIR(m.st_mode) and m.st_uid==m.st_gid==0 and stat.S_IMODE(m.st_mode)==0o700,'SNAPSHOT_SOCKET_PRIVATE_PARENTS')
            clients=self.root/'snapshot-sockets'/'clients'
            if not clients.exists():clients.mkdir(mode=0o700);d.sync_dir(clients.parent)
            m=clients.lstat();require(stat.S_ISDIR(m.st_mode) and m.st_uid==m.st_gid==0 and stat.S_IMODE(m.st_mode)==0o700,'SNAPSHOT_GUEST_PARENT')
            request_parent=clients/row['request_id'];request_parent.mkdir(mode=0o700);d.sync_dir(clients)
            self.requests[row['request_id']]=dict(name=path.name,request=row,sha256=digest(raw),socket_identity=(socket_meta.st_dev,socket_meta.st_ino))
            found.append(dict(request=row,request_sha256=digest(raw),socket_dev=socket_meta.st_dev,socket_ino=socket_meta.st_ino))
        return found
    def result(self,message):
        d=_driver();require(type(message)is dict and set(message)==RESULT_KEYS,'SNAPSHOT_RESULT_FIELDS')
        require(message['op']=='snapshot-worker-result' and message['request_id'] in self.requests,'SNAPSHOT_RESULT_REQUEST')
        saved=self.requests[message['request_id']];row=saved['request'];disposition=message['disposition']
        require(message['client_exit']==0 and type(message['client_exit'])is int and disposition in DISPOSITIONS and all(type(message[key])is str and HEX.fullmatch(message[key]) for key in ('client_container_id','worker_binary_sha256')) and message['client_stdout_sha256']==digest(scheduling_output(disposition)) and message['client_stderr_sha256']==digest(b''),'SNAPSHOT_RESULT_NATURAL_TYPED_EXIT')
        socket=self.root/'snapshot-sockets'/row['binding_id']/row['epoch_id']/'work.sock';m=socket.lstat()
        require(stat.S_ISSOCK(m.st_mode) and (m.st_dev,m.st_ino)==saved['socket_identity'],'SNAPSHOT_SOCKET_REPLACED')
        ack=dict(format_version=2,operation='snapshot_broker_signal',backup_id=row['backup_id'],request_id=row['request_id'],epoch_id=row['epoch_id'],binding_id=row['binding_id'],disposition=disposition,client_exit=0)
        d.new_private(self.root/'proofs'/('snapshot-ready-'+row['request_id']+'.json'),canonical(ack))
        d.new_private(self.root/'proofs'/('snapshot-worker-'+row['request_id']+'.json'),canonical(message))
        self.completed.add(saved['name']);del self.requests[row['request_id']]
        return ack

PREPARE_ROOTS=r'''
import json,os,stat
from pathlib import Path
def require(v):
 if not v:raise RuntimeError('snapshot fresh root refused')
def sync(p):
 fd=os.open(p,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
 try:os.fsync(fd)
 finally:os.close(fd)
out={}
for key,path in [('source','/var/lib/knowweave-source'),('snapshot_effects','/fresh-effects'),('snapshot_export','/fresh-export')]:
 p=Path(path);m=p.lstat();require(stat.S_ISDIR(m.st_mode) and m.st_uid==m.st_gid==0 and m.st_nlink==2 and not list(p.iterdir()));p.chmod(0o700);sync(p)
 m=p.lstat();out[key]={'dev':m.st_dev,'ino':m.st_ino,'uid':m.st_uid,'gid':m.st_gid,'mode':stat.S_IMODE(m.st_mode)}
 if key=='source':
  for name in ('assets','snapshot-sockets'):(p/name).mkdir(mode=0o700);sync(p/name)
  sync(p)
require(len({(v['dev'],v['ino']) for v in out.values()})==3)
print(json.dumps(out,sort_keys=True,separators=(',',':')))
'''
def prepare_volumes(h,b,runner,result,image,source,ident,record,context):
    d=_driver();require(context._snapshot_effects(),'SNAPSHOT_FIXED_PROFILE')
    values=[]
    for key,suffix in [('snapshot_effects_volume','snapshot-effects'),('snapshot_export_volume','snapshot-export')]:
        result['resource_creation_unknown']=True
        name='kwc4c-'+ident['case_id'].replace('-','')+'-'+suffix
        labels={'com.docker.compose.project':ident['project'],'knowweave.source-lifecycle.batch':result['batch_id']}
        ledger=d.runner_base(runner).ownership;pending=dict(name=name,labels=labels)
        owner=ledger.acquire('volume',name,dict(name=name,labels=labels),None,pending)
        value=d.creation_call(runner,owner,lambda:b.create_volume(h,runner,name,labels))
        require(value['Name']==name and value['Driver']=='local' and not value.get('Options') and value['Labels']==labels and value['Mountpoint'].endswith('/volumes/'+name+'/_data'),'SNAPSHOT_FRESH_NAMED_VOLUME')
        ledger.known(owner,name);ledger.release(owner,'retained');record['volumes'][key]=value;values.append(value);result['resource_creation_unknown']=False
    context._source_volume_created(record['volumes']['source_volume']);context._snapshot_volumes_created(*values)
    mount=[('volume',ident['source_volume'],ROOT,False),('volume',values[0]['Name'],'/fresh-effects',False),('volume',values[1]['Name'],'/fresh-export',False)]
    _,out,err=d.run_helper(h,b,runner,result,image,mount,['python3','-c',PREPARE_ROOTS])
    require(err==b'','SNAPSHOT_ROOT_PREPARATION_STDERR');facts=json.loads(out,object_pairs_hook=_pairs)
    require(set(facts)=={'source','snapshot_effects','snapshot_export'} and all(type(row)is dict and set(row)=={'dev','ino','uid','gid','mode'} and row['uid']==row['gid']==0 and row['mode']==0o700 for row in facts.values()),'SNAPSHOT_ROOT_PREPARATION_FACTS')
    record['snapshot_root_prepare']=facts

def compile_worker(h,b,runner,result,image,source,ident,env,record):
    d=_driver();command=['cargo','build','--locked','--offline','-p','learning-worker','--bin','learning-worker','--message-format=json']
    code,out,err=d.run_helper(h,b,runner,result,image,d.mounts(source,ident),command,env=env)
    rows=[json.loads(line,object_pairs_hook=_pairs) for line in out.decode().splitlines()]
    matches=[row for row in rows if row.get('reason')=='compiler-artifact' and row.get('manifest_path')=='/reviewed/crates/learning-worker/Cargo.toml' and row.get('target',{}).get('name')=='learning-worker' and row['target'].get('kind')==['bin'] and row.get('executable')=='/target/build/debug/learning-worker']
    require(code==0 and len(matches)==1 and [r.get('success') for r in rows if r.get('reason')=='build-finished']==[True],'SNAPSHOT_REAL_WORKER_BUILD')
    artifact=matches[0]['executable'];profile=d.validate_compiler_profile(out.decode(),artifact,False)
    intermediate,receipt=d.materialize_artifact(h,b,runner,result,image,source,ident,artifact,'worker')
    retained='/target/retained/'+ident['case_id']+'-worker'
    _,copied,stderr=d.run_helper(h,b,runner,result,image,d.mounts(source,ident),['python3','-c',b.COPY_BINARY,intermediate,retained])
    require(stderr==b'' and json.loads(copied,object_pairs_hook=_pairs)=={'binary_sha256':receipt['sha256']},'SNAPSHOT_WORKER_COPY')
    _,mode_out,mode_err=d.run_helper(h,b,runner,result,image,d.mounts(source,ident),['python3','-c','import os,sys;os.chmod(sys.argv[1],0o555)',retained])
    require(mode_out==mode_err==b'','SNAPSHOT_WORKER_EXECUTABLE_MODE')
    record['snapshot_worker_binary']=dict(binary=retained,sha256=receipt['sha256'],artifact=artifact,profile=profile,materialization=receipt,compiler_stdout_sha256=digest(out),compiler_stderr_sha256=digest(err),exit_code=code)
    record['snapshot_worker_binary_before']=audit_worker(h,b,runner,result,image,source,ident,record)

AUDIT_WORKER=r'''
import hashlib,json,os,stat,sys
path=sys.argv[1];fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC)
try:
 m=os.fstat(fd)
 if not(stat.S_ISREG(m.st_mode) and m.st_uid==m.st_gid==0 and stat.S_IMODE(m.st_mode)==0o555 and m.st_nlink==1 and 0<m.st_size<=512*1024*1024):raise RuntimeError('snapshot worker binary metadata')
 h=hashlib.sha256();n=0
 while True:
  chunk=os.read(fd,65536)
  if not chunk:break
  n+=len(chunk)
  if n>m.st_size:raise RuntimeError('snapshot worker binary grew')
  h.update(chunk)
 a=os.fstat(fd);keys=('st_dev','st_ino','st_size','st_mtime_ns','st_ctime_ns')
 if n!=m.st_size or any(getattr(m,k)!=getattr(a,k)for k in keys):raise RuntimeError('snapshot worker binary changed')
 print(json.dumps(dict(sha256=h.hexdigest(),dev=m.st_dev,ino=m.st_ino,bytes=n,uid=m.st_uid,gid=m.st_gid,mode=stat.S_IMODE(m.st_mode),nlink=m.st_nlink),sort_keys=True,separators=(',',':')))
finally:os.close(fd)
'''
def audit_worker(h,b,runner,result,image,source,ident,record):
    d=_driver();saved=record['snapshot_worker_binary']
    _,out,err=d.run_helper(h,b,runner,result,image,d.mounts(source,ident,False),['python3','-c',AUDIT_WORKER,saved['binary']])
    row=json.loads(out,object_pairs_hook=_pairs)
    require(err==b'' and type(row)is dict and set(row)=={'sha256','dev','ino','bytes','uid','gid','mode','nlink'} and row['sha256']==saved['sha256'] and row['bytes']==saved['materialization']['bytes'] and row['uid']==row['gid']==0 and row['mode']==0o555 and row['nlink']==1,'SNAPSHOT_WORKER_RETAINED_IDENTITY')
    return row

def audit_credentials(runner,record):
    d=_driver();_,raw,err=d.pg_exec(runner,record['container_id'],['cat',ROOT+'/proofs/snapshot-credential-pins.json'])
    require(err==b'' and 0<len(raw)<=4096,'SNAPSHOT_CREDENTIAL_PROOF_READ')
    value=json.loads(raw,object_pairs_hook=_pairs)
    require(type(value)is dict and set(value)=={'format_version','capability','files'} and value['format_version']==2 and value['capability']=='snapshot_fixture_credential_pins_v2' and set(value['files'])=={'broker-dsn','runtime-dsn'},'SNAPSHOT_CREDENTIAL_PROOF')
    for name,role in [('broker-dsn','learning_snapshot_claim_broker'),('runtime-dsn','learning_runtime')]:
        pin=value['files'][name]
        require(set(pin)=={'role','dev','ino','bytes','sha256','uid','gid','mode','nlink'} and pin['role']==role and pin['uid']==pin['gid']==0 and pin['mode']==0o600 and pin['nlink']==1 and 0<pin['bytes']<=4096 and HEX.fullmatch(pin['sha256']),'SNAPSHOT_CREDENTIAL_METADATA')
    return value

def login_password(admin_secret,role):
    require(type(admin_secret)is str and HEX.fullmatch(admin_secret) and role in ('learning_runtime','learning_snapshot_claim_broker'),'SNAPSHOT_LOGIN_DOMAIN')
    return digest(('knowweave-task6-native-scram-v1\x00'+role+'\x00'+admin_secret).encode())

def capture_postcheck(runner,container,record,inventory):
    """Independent text-only fixture byte audit; never a finding issuer."""
    d=_driver();require(d.sql(runner,container,'SELECT count(*) FROM public.asset;',record['identity']['database'],'learning_admin')=='0','SNAPSHOT_TEXT_ONLY_ASSETS')
    require(not any(path.startswith('assets/')and not row.get('directory')for path,row in inventory.items()),'SNAPSHOT_ORIGINAL_ASSETS_MUST_BE_EMPTY')
    indexed=[path for path in inventory if re.fullmatch(r'pins/[0-9a-f-]{36}\.sealed/asset-index\.json',path)]
    require(len(indexed)==1,'SNAPSHOT_CAPTURE_INDEX')
    path=indexed[0];prefix=path.removesuffix('asset-index.json')
    _,index_raw,err=d.pg_exec(runner,container,['cat',ROOT+'/'+path]);require(err==b'','SNAPSHOT_INDEX_READ')
    checked=d.validate_index(index_raw,[],fixture_counts=False)
    manifest_raw=bytes.fromhex(inventory[prefix+'manifest.json']['raw_hex']);manifest=json.loads(manifest_raw,object_pairs_hook=_pairs)
    require(list(manifest)==['capability','format_version','backup_id','source','logical_asset_count','unique_asset_bytes','files'] and json.dumps(manifest,ensure_ascii=False,separators=(',',':')).encode()==manifest_raw,'SNAPSHOT_MANIFEST_CANONICAL')
    require(manifest['capability']=='backup_full_v1' and type(manifest['format_version'])is int and manifest['format_version']==1 and manifest['backup_id']==prefix.split('/')[1].removesuffix('.sealed') and manifest['logical_asset_count']==manifest['unique_asset_bytes']==0,'SNAPSHOT_ZERO_ASSET_MANIFEST')
    source=manifest['source'];migrations=record['migrations']
    require(list(source)==['application_build_sha256','application_commit','postgres_major','migration_version','migrations','migration_fingerprint'] and source['application_commit']==d.BASE_COMMIT and source['application_build_sha256']==record['archive_sha256'] and source['postgres_major']==18 and source['migrations']==migrations and source['migration_version']==migrations[-1]['version'] and source['migration_fingerprint']==digest(json.dumps(migrations,separators=(',',':')).encode()),'SNAPSHOT_MANIFEST_SOURCE')
    require([row['path']for row in manifest['files']]==['asset-index.json','database.dump','roles.json'],'SNAPSHOT_SEALED_FILES')
    for row in manifest['files']:
        entry=inventory.get(prefix+row['path'])
        require(set(row)=={'path','size','sha256'} and entry is not None and entry.get('bytes')==row['size'] and entry.get('sha256')==row['sha256'],'SNAPSHOT_SEALED_HASH')
    require({path for path in inventory if path.startswith(prefix)}=={prefix+name for name in ('asset-index.json','database.dump','roles.json','manifest.json')},'SNAPSHOT_SEALED_EXACT_TREE')
    code,toc,err=d.pg_exec(runner,container,['/usr/lib/postgresql/18/bin/pg_restore','--list',ROOT+'/'+prefix+'database.dump'])
    toc_receipt=d.validate_toc(code,toc,err,record['identity']['database']);d.new_private(Path(record['evidence_directory'])/'capture.toc',toc)
    checked.update(original_policy='snapshot_text_only_empty_assets',original_files=0,unique_asset_bytes=0,full_retained_hashes=True,toc_sha256=digest(toc),toc_only_no_restore=True,toc=toc_receipt)
    return checked

def prepare_source_container(runner,record,admin_secret):
    """Explicit fresh snapshot fixture administration, after fixed role birth."""
    d=_driver();statement=''
    for role in ('learning_runtime','learning_snapshot_claim_broker'):
        statement+="\nALTER ROLE "+role+" PASSWORD '"+login_password(admin_secret,role)+"';"
    d.sql(runner,record['container_id'],statement)
    # Only these two fixed writable-layer parents in this fresh PG container
    # are adjusted; mounted roots, including /run/kwc4/sfx, remain unchanged.
    recipe='set -eu; for p in /var/lib/knowweave-c4 /run/kwc4; do test -d "$p"; test ! -L "$p"; test "$(stat -c "%u %g" "$p")" = "0 0"; chmod 700 "$p"; test "$(stat -c "%u %g %a" "$p")" = "0 0 700"; done; printf "SNAPSHOT_PRIVATE_CONTAINER_PARENT_READY\\n"'
    code,out,err=d.pg_exec(runner,record['container_id'],['/bin/sh','-ec',recipe])
    require(code==0 and out==b'SNAPSHOT_PRIVATE_CONTAINER_PARENT_READY\n' and err==b'','SNAPSHOT_CONTAINER_PARENT')
    record['snapshot_source_login_setup']=dict(scram_configured=True,container_private_parent=True,classification='FRESH_TEST_FIXTURE_ADMINISTRATION_NOT_CAPTURE')

class Worker:
    def __init__(self,h,b,runner,result,image,record,row,request_sha256):
        d=_driver();self.h,self.b,self.runner,self.result=h,b,runner,result;self.closed=False;self.child=None
        ident=record['identity'];require(HEX.fullmatch(record['container_id']) and row['compose_project']==ident['project'],'SNAPSHOT_WORKER_SOURCE')
        source=record['volumes']['source_volume']['Mountpoint'];guest=ROOT+'/snapshot-sockets/clients/'+row['request_id']+'/'+row['epoch_id']
        script='set -eu; umask 027; mkdir -m 750 -- "$1"; : > "$1/work.sock"; chmod 600 "$1/work.sock"; stat -c "%u %g %a" "$1"'
        args=[d.DOCKER,'exec','--user','0:65532',record['container_id'],'/usr/bin/env','-i','PATH=/usr/bin:/bin','LC_ALL=C','/bin/sh','-ec',script,'snapshot-guest',guest]
        out,guest_rpc=runner.observed(lambda:runner.run(args,timeout=5));require(out==b'0 65532 750\n','SNAPSHOT_GUEST_ACTUAL_MODE')
        guest_host=source+'/snapshot-sockets/clients/'+row['request_id']+'/'+row['epoch_id'];guest_target=CLIENT_ROOT+'/'+row['epoch_id']
        socket=source+'/snapshot-sockets/'+row['binding_id']+'/'+row['epoch_id']+'/work.sock'
        binary=record['volumes']['build_volume']['Mountpoint']+'/retained/'+ident['case_id']+'-worker'
        command=['--snapshot-broker-client','--socket',guest_target+'/work.sock','--epoch',row['epoch_id'],'--request-id',row['request_id'],'--lease-ms',str(row['lease_ms']),'--start-on-stdin']
        name='knowweave-snapshot-worker-'+uuid.uuid4().hex;labels={**(image['Config'].get('Labels')or{}),'knowweave.source-lifecycle.batch':result['batch_id']}
        env=h.env_dict(image['Config']['Env']);require(not any(re.search(r'(PASSWORD|SECRET|DATABASE_URL|DSN|TOKEN)',key) for key in env),'SNAPSHOT_WORKER_IMAGE_SECRET_ENV')
        mounts=[(binary,WORKER),(guest_host,guest_target),(socket,guest_target+'/work.sock')]
        create=['create','--name',name,'--pull=never','--network=none','--read-only','--cap-drop=ALL','--security-opt=no-new-privileges','--cpus=4','--memory=8g','--memory-swap=8g','--pids-limit=512','--tmpfs=/tmp:rw,nosuid,nodev,size=1g','--user=65532:65532','--pid=container:'+record['container_id'],'--interactive','--workdir=/reviewed','--entrypoint='+WORKER]
        for key,value in labels.items():create+=['--label',key+'='+value]
        for src,dest in mounts:create+=['--mount','type=bind,src='+src+',dst='+dest+',readonly']
        self.expected=dict(id=None,name=name,image=image['Id'],labels=labels,cmd=command,entrypoint=[WORKER],env=env,mounts={('bind',src,dest,False)for src,dest in mounts},volume_names={},network='none',network_id=None,stdin=True)
        self.pid_mode='container:'+record['container_id'];self.record=dict(id=None,name=name,image=image['Id'],removed=False,request_id=row['request_id'],request_sha256=request_sha256,epoch_id=row['epoch_id'],binding_id=row['binding_id'],worker_binary_sha256=record['snapshot_worker_binary']['sha256'],guest_directory_rpc=guest_rpc)
        self.owner=d.runner_base(runner).ownership.acquire('helper',name,self.expected,None,self.record);self.owner['object']=self;result['helpers'].append(self.record)
        result['resource_creation_unknown']=True
        ident_created=d.creation_call(runner,self.owner,lambda:runner.docker(*create,image['Id'],*command)).decode().strip();require(HEX.fullmatch(ident_created),'SNAPSHOT_WORKER_CREATED_ID')
        self.record['id']=self.expected['id']=ident_created;d.runner_base(runner).ownership.known(self.owner,ident_created)
        require(self.inspect()['State']['Status']=='created','SNAPSHOT_WORKER_FRESH_CREATED')
        result['resource_creation_unknown']=False
        self.child=d.MonitoredChild([d.DOCKER,'start','-a','-i',ident_created],runner.logs,timeout=90,interactive=True)
        self.owner['child']=self.child
        deadline=time.monotonic()+5
        while True:
            raw,receipt=runner.observed(lambda:runner.run([d.DOCKER,'container','inspect',ident_created]))
            rows=json.loads(raw,object_pairs_hook=_pairs);require(type(rows)is list and len(rows)==1,'SNAPSHOT_WORKER_LIVE_INSPECT_COUNT')
            facts=self.validate(rows[0])
            if facts['State']['Status']=='running':
                self.record['actual_live_inspect']=dict(receipt=receipt,sha256=digest(raw),request_sha256=request_sha256,classification='ACTUAL_UID65532_RO_LEAF_CLIENT_NOT_BOOTSTRAP_AUTHORITY');break
            require(facts['State']['Status']=='created' and time.monotonic()<deadline,'SNAPSHOT_WORKER_LIVE_INSPECT_REQUIRED')
            self.child.check();time.sleep(0.01)
        self.child.send(b'START\n');self.child.close_input()
    def inspect(self,runner=None):
        facts=(runner or self.runner).inspect('container',self.record['id'])
        return self.validate(facts)
    def validate(self,facts):
        require(facts['Config']['User']=='65532:65532' and facts['HostConfig']['PidMode']==self.pid_mode,'SNAPSHOT_WORKER_ACTUAL_UID_PID')
        adjusted=copy.deepcopy(facts);adjusted['Config']['User']='0:0';adjusted['HostConfig']['PidMode']=''
        self.b.validate_helper(self.h,adjusted,self.expected)
        return facts
    def finish(self):
        require(self.child.p.poll()is not None,'SNAPSHOT_WORKER_NOT_NATURALLY_EXITED')
        code,out,err=self.child.finish(terminate_on_error=False);facts=self.inspect()
        require(facts['State']['Status']=='exited' and facts['State']['ExitCode']==code and not facts['State'].get('OOMKilled'),'SNAPSHOT_WORKER_DOCKER_EXIT')
        disposition=validate_client_exit(code,out,err);result=dict(client_exit=code,disposition=disposition,client_stdout_sha256=digest(out),client_stderr_sha256=digest(err),client_container_id=self.record['id'])
        self.close();return result
    def close(self,phase='close'):
        if self.closed:return
        d=_driver();require(phase in ('close','fallback') and not self.owner.get(phase+'_attempted'),'SNAPSHOT_WORKER_CLEANUP_ONCE');self.owner[phase+'_attempted']=True
        clean=d.CleanupRunner(self.runner,self.owner,phase);error=None
        try:
            self.inspect(clean)
            try:clean.docker('stop','--timeout','5',self.record['id'],timeout=10)
            except BaseException as caught:error=caught
            facts=self.inspect(clean);require(facts['State']['Running']is False and facts['State']['Pid']==0,'SNAPSHOT_WORKER_STOP_UNKNOWN')
            if self.child and not self.child.finished and not self.child.retire_attempted:
                try:self.child.retire(timeout=d.TRANSPORT_TEARDOWN)
                except BaseException as caught:error=error or caught
            clean.docker('rm',self.record['id']);require(self.record['id']not in clean.docker('ps','-aq','--no-trunc').decode().split(),'SNAPSHOT_WORKER_REMOVE_UNKNOWN')
            self.record['removed']=True;self.closed=True;d.runner_base(self.runner).ownership.release(self.owner,'absent')
            if error is not None:raise error
        except BaseException:self.owner['close_failed']=True;raise

class Controller:
    def __init__(self,h,b,runner,result,image,record,broker):self.args=(h,b,runner,result,image,record);self.broker=broker;self.pending=None;self.results=[]
    def service(self):
        record=self.args[-1]
        if self.pending is not None:
            worker,row=self.pending;worker.child.check()
            if worker.child.p.poll()is None:return
            result=worker.finish();result.update(op='snapshot-worker-result',request_id=row['request_id'],worker_binary_sha256=record['snapshot_worker_binary']['sha256'])
            ack=self.broker.call(result);require(ack['request_id']==row['request_id']and ack['disposition']==result['disposition'],'SNAPSHOT_WORKER_ACK')
            self.results.append(result);record['snapshot_worker_results']=list(self.results);self.pending=None
        found=self.broker.call(dict(op='snapshot-poll'));require(type(found)is list and len(found)<=1,'SNAPSHOT_POLL_SINGLE')
        if found:
            row=found[0]['request'];validate_request(canonical(row),'snapshot-request-'+row['request_id']+'.json',record['identity'])
            require(type(found[0]['request_sha256'])is str and HEX.fullmatch(found[0]['request_sha256']),'SNAPSHOT_REQUEST_HASH')
            self.pending=(Worker(*self.args,row,found[0]['request_sha256']),row)
    def close(self):
        if self.pending is not None:self.pending[0].close();self.pending=None
    def completed(self):require(self.pending is None,'SNAPSHOT_WORKER_ACTIVE_AT_CONSUMER_EXIT')
