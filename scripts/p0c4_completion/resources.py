"""Acceptance-only host route observer. No caller routes, namespace or pins.

The root capsule issuer must mount the host's independently observed nsfs
network handle read-only at HOST_NETNS and set network=host. PG and source
helpers remain on their isolated or none namespaces. No automatic fallback.
"""
import hashlib
import ipaddress
import os
from pathlib import Path
import socket
import struct
import time
import uuid
if __package__.startswith('scripts.'):
    from .. import p0c4_storage_registry as registry
    from .. import p0c4_source_lifecycle_gate_acceptance as driver
    from .. import p0c4_source_admission_gate_acceptance as admission
else:
    import p0c4_storage_registry as registry
    import p0c4_source_lifecycle_gate_acceptance as driver
    import p0c4_source_admission_gate_acceptance as admission
from .plan import CompletionError, require as envelope_require
import sys
import stat
import copy

HOST_NETNS='/run/knowweave-c4/host-netns'
MAX_BYTES=1024*1024
MAX_DATAGRAM=65536
MAX_ROUTES=4096
class ResourceError(RuntimeError): pass
def require(ok,token):
    if not ok:raise ResourceError(token)

def require_host_namespace():
    fds=[]
    try:
        try:
            reference=os.open(HOST_NETNS,os.O_RDONLY|getattr(os,'O_NOFOLLOW',0)|getattr(os,'O_CLOEXEC',0));fds.append(reference)
            own=os.open('/proc/self/ns/net',os.O_RDONLY|getattr(os,'O_CLOEXEC',0));fds.append(own)
            reference_meta,own_meta=os.fstat(reference),os.fstat(own)
        except OSError as error:raise ResourceError('HOST_NETNS_REFERENCE_UNAVAILABLE') from error
        require(reference_meta.st_uid==0 and (reference_meta.st_dev,reference_meta.st_ino)==(own_meta.st_dev,own_meta.st_ino),'HOST_NETNS_MISMATCH')
        return dict(dev=reference_meta.st_dev,ino=reference_meta.st_ino)
    finally:
        for fd in fds:os.close(fd)

class RouteParser:
    def __init__(self,sequence,port):self.sequence,self.port=sequence,port;self.bytes=0;self.messages=0;self.routes=set();self.done=False
    def feed(self,raw):
        require(type(raw) is bytes and 0<len(raw)<=MAX_DATAGRAM and not self.done,'NETLINK_DATAGRAM')
        require(self.bytes+len(raw)<=MAX_BYTES,'NETLINK_BYTE_CAP');self.bytes+=len(raw);offset=0
        while offset<len(raw):
            require(len(raw)-offset>=16,'NETLINK_TRUNCATED_HEADER')
            size,kind,flags,sequence,port=struct.unpack_from('=IHHII',raw,offset)
            require(size>=16 and offset+size<=len(raw) and sequence==self.sequence and port in (0,self.port) and not flags&0x10 and not flags&~6,'NETLINK_MESSAGE_HEADER')
            payload=raw[offset+16:offset+size];aligned=(size+3)&~3
            require(offset+aligned<=len(raw) and raw[offset+size:offset+aligned]==b'\0'*(aligned-size),'NETLINK_MESSAGE_PADDING')
            offset+=aligned;self.messages+=1;require(self.messages<=MAX_ROUTES+1,'NETLINK_ROUTE_CAP')
            if kind==3:
                require(len(payload) in (0,4) and (not payload or struct.unpack('=i',payload)[0]==0) and offset==len(raw),'NETLINK_DONE')
                self.done=True;continue
            require(kind==24,'NETLINK_ERROR_OR_UNKNOWN')
            require(len(payload)>=12,'NETLINK_ROUTE_HEADER')
            family,prefix,source_prefix,tos,table,protocol,scope,route_type,route_flags=struct.unpack_from('=BBBBBBBBI',payload)
            require(family==2 and prefix<=32 and source_prefix<=32,'NETLINK_IPV4_ROUTE')
            destination=None;attribute_offset=12;seen=set()
            while attribute_offset<len(payload):
                require(len(payload)-attribute_offset>=4,'NETLINK_ATTRIBUTE_HEADER')
                attr_size,attr_type=struct.unpack_from('=HH',payload,attribute_offset)
                require(attr_size>=4 and attribute_offset+attr_size<=len(payload) and attr_type not in seen,'NETLINK_ATTRIBUTE_SIZE_OR_DUPLICATE');seen.add(attr_type)
                value=payload[attribute_offset+4:attribute_offset+attr_size]
                if attr_type==1:
                    require(len(value)==4,'NETLINK_DESTINATION');destination=ipaddress.IPv4Address(value)
                padded=(attr_size+3)&~3;require(attribute_offset+padded<=len(payload),'NETLINK_ATTRIBUTE_PADDING');attribute_offset+=padded
            if prefix==0:
                require(destination in (None,ipaddress.IPv4Address('0.0.0.0')),'NETLINK_DEFAULT_ROUTE');continue
            require(destination is not None,'NETLINK_DESTINATION_MISSING')
            try:network=ipaddress.IPv4Network((destination,prefix),strict=True)
            except ValueError as error:raise ResourceError('NETLINK_NONCANONICAL_ROUTE') from error
            self.routes.add(str(network))
    def finish(self):
        require(self.done,'NETLINK_INCOMPLETE_DUMP');return [dict(dst=route) for route in sorted(self.routes)]

def sample_host_routes():
    namespace=require_host_namespace();deadline=time.monotonic()+5;sequence=int.from_bytes(uuid.uuid4().bytes[:4],'little') or 1
    chunks=[]
    try:
        with socket.socket(socket.AF_NETLINK,socket.SOCK_RAW,socket.NETLINK_ROUTE) as observer:
            observer.bind((0,0));port=observer.getsockname()[0];parser=RouteParser(sequence,port)
            payload=struct.pack('=BBBBBBBBI',2,0,0,0,0,0,0,0,0)
            request=struct.pack('=IHHII',16+len(payload),26,0x301,sequence,port)+payload
            observer.settimeout(max(0.001,deadline-time.monotonic()));require(observer.sendto(request,(0,0))==len(request),'NETLINK_REQUEST')
            while not parser.done:
                require(time.monotonic()<deadline,'NETLINK_DEADLINE');observer.settimeout(deadline-time.monotonic())
                raw,ancillary,flags,peer=observer.recvmsg(MAX_DATAGRAM)
                require(peer==(0,0) and not ancillary and flags==0,'NETLINK_PEER_OR_TRUNCATION')
                parser.feed(raw);chunks.append(raw)
            routes=parser.finish()
    except OSError as error:raise ResourceError('NETLINK_UNAVAILABLE') from error
    require(require_host_namespace()==namespace,'HOST_NETNS_CHANGED')
    raw=b''.join(chunks);require(len(raw)<=MAX_BYTES,'NETLINK_BYTE_CAP')
    return routes,raw,dict(format_version=1,capability='c4_host_routes_v1',namespace=namespace,route_count=len(routes),scan_bytes=len(raw),raw_sha256=hashlib.sha256(raw).hexdigest(),routes=routes)

def route_observer(batch,result):
    """A fixed internal hook, never an input deserializer or CLI option."""
    ordinal=len(result.setdefault('host_route_observations',[]));require(ordinal<10,'HOST_ROUTE_OBSERVATION_CAP')
    routes,raw,record=sample_host_routes();evidence=batch/'evidence'
    root=registry.open_directory(str(evidence))
    try:
        binary=f'host-routes-{ordinal:02d}.bin';receipt=f'host-routes-{ordinal:02d}.json'
        registry.new_file(root,binary,raw);record['raw_file']=dict(path='evidence/'+binary,size=len(raw),sha256=record['raw_sha256'])
        value=registry.canonical(record);require(len(value)<=512*1024,'HOST_ROUTE_RECEIPT_CAP');registry.new_file(root,receipt,value)
        require(registry.read_file(root,binary,MAX_BYTES,registry.Budget())==raw and registry.read_file(root,receipt,512*1024,registry.Budget())==value,'HOST_ROUTE_READBACK')
    finally:os.close(root)
    result['host_route_observations'].append(dict(path='evidence/'+receipt,size=len(value),sha256=registry.digest(value)))
    return routes

def actor_preflight(require_docker=True):
    envelope_require(sys.platform=='linux' and os.geteuid()==0,'COMPLETION_ROOT_ACTOR')
    if not require_docker:return
    envelope_require(driver.DOCKER=='/usr/bin/docker' and admission.DOCKER==driver.DOCKER and driver.HOST_ENV.get('DOCKER_HOST')=='unix:///var/run/docker.sock','COMPLETION_DOCKER_ENDPOINT')
    m=os.stat('/usr/bin/docker',follow_symlinks=True);envelope_require(stat.S_ISREG(m.st_mode) and m.st_uid==0 and not m.st_mode&0o022,'COMPLETION_DOCKER_IDENTITY')
    m=os.stat('/var/run/docker.sock',follow_symlinks=True);envelope_require(stat.S_ISSOCK(m.st_mode) and m.st_uid==0 and stat.S_IMODE(m.st_mode) in (0o600,0o660),'COMPLETION_DOCKER_SOCKET')


class SourceClone:
    """Fixed negative-fixture peer; never target birth or source authority.

    A fresh PG volume is temporarily mounted at the one fixed copy destination
    in the original. pg_basebackup and pg_verifybackup run as postgres there;
    the independent clone has its own single network and ordinary seven mounts.
    Every create/stop remains in the current controller's ownership ledger.
    """
    COPY_ROOT='/var/lib/knowweave-source-clone'
    COPY_DATA=COPY_ROOT+'/18/docker'

    def __init__(self,h,b,runner,batch,plan,result):
        self.h,self.b,self.runner,self.result=h,b,runner,result
        self.ledger=driver.runner_base(runner).ownership
        self.ident=driver.completion_case_identity(plan['case_id']);self.subnet=plan['subnet']
        self.path=batch/('clone-'+plan['case_id']);h.mkdir_new(self.path)
        self.record=dict(identity=self.ident,subnet=self.subnet,stopped=False,execs_absent=False,retained_network_empty=False,physical_backup_verified=False)
        require(not result.get('source_clones'),'SOURCE_CLONE_SINGLE_PEER')
        result['source_clones']=[self.record];self.owner=None;self.expected=None;self.document=None
        labels={'com.docker.compose.project':self.ident['project'],'knowweave.source-lifecycle.batch':result['batch_id']}
        volume=dict(name=self.ident['volume'],labels=labels)
        owner=self.ledger.acquire('volume',self.ident['volume'],volume,None,volume)
        result['resource_creation_unknown']=True
        self.volume=driver.creation_call(runner,owner,lambda:b.create_volume(h,runner,self.ident['volume'],labels))
        self.ledger.known(owner,self.ident['volume']);self.ledger.release(owner,'retained')
        self.record['volume']=self.volume;result['resource_creation_unknown']=False

    def start(self,primary_id,document,expected):
        r=self.runner;h=self.h
        require(type(primary_id) is str and driver.HEX64.fullmatch(primary_id),'SOURCE_CLONE_ORIGIN')
        self.record['original_container_id']=primary_id
        # Only the new admitted clone volume is writable at this fixed path.
        script='test -d /var/lib/knowweave-source-clone; test -z "$(ls -A /var/lib/knowweave-source-clone)"; mkdir -p /var/lib/knowweave-source-clone/18/docker; chmod 0700 /var/lib/knowweave-source-clone/18/docker; chown 999:999 /var/lib/knowweave-source-clone/18 /var/lib/knowweave-source-clone/18/docker'
        driver.pg_exec(r,primary_id,['/bin/sh','-ec',script])
        prefix=[driver.DOCKER,'exec','--user','999:999',primary_id,'/usr/bin/env','-i','LC_ALL=C']
        backup=[*prefix,'/usr/lib/postgresql/18/bin/pg_basebackup','--pgdata='+self.COPY_DATA,'--format=plain','--wal-method=stream','--no-password','--host=/var/run/postgresql','--port=5432','--username=postgres']
        code,out,err=r.run(backup,timeout=900)
        require(code==0 and out==b'' and err==b'','SOURCE_CLONE_PHYSICAL_COPY')
        self.record['copy']=dict(exit_code=code,stdout_sha256=driver.digest(out),stderr_sha256=driver.digest(err),origin_container_id=primary_id)
        code,out,err=r.run([*prefix,'/usr/lib/postgresql/18/bin/pg_verifybackup',self.COPY_DATA],timeout=900)
        require(code==0 and out==b'backup successfully verified\n' and err==b'','SOURCE_CLONE_VERIFY_BACKUP')
        driver.pg_exec(r,primary_id,['/bin/sh','-ec','test ! -e /var/lib/knowweave-source-clone/18/docker/standby.signal; test ! -e /var/lib/knowweave-source-clone/18/docker/recovery.signal'])
        self.record['verification']=dict(exit_code=code,stdout_sha256=driver.digest(out),stderr_sha256=driver.digest(err))
        self.record['physical_backup_verified']=True
        clone=copy.deepcopy(document);clone['name']=self.ident['project'];pg=clone['services']['pg']
        pg['container_name']=self.ident['pg_name'];pg['volumes']=[m for m in pg['volumes'] if m['target']!=self.COPY_ROOT]
        clone['volumes'].pop('clonepg',None);clone['volumes']['pg']['name']=self.ident['volume']
        clone['networks']['test'].update(name=self.ident['network'],ipam={'config':[{'subnet':self.subnet}]})
        h.write_new(self.path/'compose.json',driver.canonical(clone));self.document=clone
        e=copy.deepcopy(expected);e.update(id=None,name=self.ident['pg_name'],network=self.ident['network'])
        e['labels']={'com.docker.compose.project':self.ident['project'],'com.docker.compose.service':'pg','com.docker.compose.project.working_dir':str(self.path),'com.docker.compose.project.config_files':str(self.path/'compose.json')}
        e['mounts']={(kind,(self.volume['Mountpoint'] if target=='/var/lib/postgresql' else source),target,rw) for kind,source,target,rw in e['mounts'] if target!=self.COPY_ROOT}
        net=dict(name=self.ident['network'],project=self.ident['project'],subnet=self.subnet)
        self.owner=self.ledger.acquire('pg',self.ident['pg_name'],e,net,self.record)
        self.result['resource_creation_unknown']=True
        command=[driver.DOCKER,'compose','--project-name',self.ident['project'],'--project-directory',str(self.path),'-f',str(self.path/'compose.json'),'create','--no-build','--pull','never']
        driver.creation_call(r,self.owner,lambda:r.run(command))
        pg,network=r.inspect('container',self.ident['pg_name']),r.inspect('network',self.ident['network'])
        self.expected=driver.admit_created_pg(h,self.owner,pg,network,self.record);self.ledger.known(self.owner,pg['Id'])
        self.result['resource_creation_unknown']=False;r.docker('start',pg['Id']);deadline=time.monotonic()+90
        while True:
            code,_,_=driver.pg_exec(r,pg['Id'],['/usr/lib/postgresql/18/bin/pg_isready','--host=/var/run/postgresql','--port=5432','--username=postgres'],allowed=(0,1,2,3))
            if code==0:break
            require(time.monotonic()<deadline,'SOURCE_CLONE_READY');time.sleep(.2)
        self.facts()
        return self.record

    def facts(self):
        pg=self.runner.inspect('container',self.record['container_id']);network=self.runner.inspect('network',self.record['network_id'])
        self.h.validate_container(pg,self.expected)
        require(pg['State']['Running'] is True and not pg['State'].get('OOMKilled') and network['Internal'] is True and set(network.get('Containers',{}))=={pg['Id']},'SOURCE_CLONE_LIVE_IDENTITY')
        pin=dict(container_id=pg['Id'],image_id=pg['Image'],started_at=pg['State']['StartedAt'],restart_count=pg['RestartCount'])
        if 'running_pin' in self.record:require(self.record['running_pin']==pin,'SOURCE_CLONE_RESTARTED')
        else:self.record['running_pin']=pin
        return pg,network

    def stop(self):
        if self.owner is None:return
        ledger=self.ledger;ledger.enter()
        if self.owner['state']=='planned':
            ledger.release(self.owner,'absent');self.record.update(stopped=True,execs_absent=True,retained_network_empty=True);return
        if self.owner['state']=='sent':
            facts=ledger.reconcile(self.owner)
            if facts is None:
                network=ledger.reconcile(self.owner,'network',True)
                if network is not None:
                    require(network.get('Name')==self.ident['network'] and network.get('Labels',{}).get('com.docker.compose.project')==self.ident['project'] and network.get('Internal') is True and not network.get('Containers'),'SOURCE_CLONE_ABSENT_NETWORK_UNKNOWN')
                ledger.release(self.owner,'absent');self.record.update(stopped=True,execs_absent=True,retained_network_empty=True);return
            network=ledger.reconcile(self.owner,'network',True)
            require(network is not None,'SOURCE_CLONE_NETWORK_UNKNOWN')
            self.expected=driver.admit_created_pg(self.h,self.owner,facts,network,self.record);ledger.known(self.owner,facts['Id'])
        require(self.owner['state']=='known' and self.expected is not None,'SOURCE_CLONE_IDENTITY_UNKNOWN')
        clean=driver.CleanupRunner(self.runner,self.owner,'pg')
        self.h.validate_container(clean.inspect('container',self.record['container_id']),self.expected)
        stop_error=None
        try:clean.docker('stop','--timeout','10',self.record['container_id'],timeout=15)
        except BaseException as error:stop_error=error
        facts=clean.inspect('container',self.record['container_id']);network=clean.inspect('network',self.record['network_id'])
        self.h.validate_container(facts,self.expected)
        require(facts['State']['Running'] is False and facts['State']['Pid']==0 and not facts.get('ExecIDs') and not network.get('Containers'),'SOURCE_CLONE_STOP_UNCONFIRMED')
        self.record.update(stopped=True,execs_absent=True,retained_network_empty=True);ledger.release(self.owner,'stopped')
        if stop_error is not None:raise stop_error


def service_source_clone_observer(peer,broker,record,anchors,isolation):
    """One fixed read-only query in each exact PG; requests contain numbers only."""
    requests=broker.call(dict(op='source-clone-poll'))
    require(type(requests) is list and len(requests)<=1,'SOURCE_CLONE_OBSERVER_COUNT')
    if not requests:return
    require('source_clone_observation' not in record,'SOURCE_CLONE_OBSERVER_REUSED')
    request=requests[0];keys=request['challenge_keys'];pid=request['backend_pid'];oid=request['database_oid']
    require(type(keys) is list and len(keys)==2 and all(type(k) is int and -(2**63)<=k<2**63 for k in keys) and keys[0]!=keys[1] and type(pid) is int and 0<pid<2**31 and type(oid) is int and 0<oid<2**32,'SOURCE_CLONE_OBSERVER_NUMBERS')
    pairs=[((key & ((1<<64)-1))>>32,key & 0xffffffff) for key in keys]
    sql="BEGIN READ ONLY;SET LOCAL statement_timeout='5000ms';SELECT current_user::text||'|'||current_database()||'|'||d.oid::text||'|'||pcs.system_identifier::text FROM pg_database d CROSS JOIN pg_control_system() pcs WHERE d.datname=current_database();SELECT pid,database::bigint,classid::bigint,objid::bigint,objsubid,mode,granted FROM pg_locks WHERE locktype='advisory' AND (classid::bigint,objid::bigint) IN ((%d,%d),(%d,%d));ROLLBACK;"%(*pairs[0],*pairs[1])
    database=record['identity']['database'];require(database=='learning_backup_c4_task3_'+record['identity']['case_id'],'SOURCE_CLONE_DATABASE')
    args=['/usr/lib/postgresql/18/bin/psql','-XqAt','--no-password','--host=/var/run/postgresql','--port=5432','--username=learning_admin','--dbname='+database,'-v','ON_ERROR_STOP=1','-c',sql]
    primary=record['container_id'];clone=peer.record['container_id'];require(primary!=clone,'SOURCE_CLONE_DISTINCT_ENDPOINT')
    peer.facts()
    primary_raw=driver.pg_exec(peer.runner,primary,args)[1];clone_raw=driver.pg_exec(peer.runner,clone,args)[1]
    require(len(primary_raw)<=8192 and len(clone_raw)<=8192,'SOURCE_CLONE_OBSERVER_BYTES')
    first='learning_admin|'+database+'|'+str(oid)+'|'+request['system_identifier']
    primary_lines=primary_raw.decode('ascii').splitlines()
    expected={f'{pid}|{oid}|{high}|{low}|1|ExclusiveLock|t' for high,low in pairs}
    require(len(primary_lines)==3 and primary_lines[0]==first and set(primary_lines[1:])==expected and primary_raw.endswith(b'\n'),'SOURCE_CLONE_ORIGINAL_LOCKS')
    require(clone_raw==(first+'\n').encode(),'SOURCE_CLONE_EQUAL_IDS_MISSING_LOCKS')
    original_ns=driver.parse_namespace_sampler(peer.runner.run(driver.namespace_command(primary))[1])
    clone_ns=driver.parse_namespace_sampler(peer.runner.run(driver.namespace_command(clone))[1])
    require(original_ns['pid_ns']!=clone_ns['pid_ns'],'SOURCE_CLONE_DISTINCT_NAMESPACE')
    peer.facts()
    response=dict(nonce=request['nonce'],backup_id=request['backup_id'],source_backend_pid=pid,source_database_oid=oid,source_system_identifier=request['system_identifier'],
        original_container_id=primary,clone_container_id=clone,clone_project=peer.ident['project'],clone_socket_output=clone_raw.decode('ascii'),physical_backup_verified=peer.record['physical_backup_verified'])
    record['source_clone_observation']=dict(request=request,response=response,original_namespace=original_ns,clone_namespace=clone_ns,
        original_socket_output=primary_raw.decode('ascii'),clone_socket_output=clone_raw.decode('ascii'),original_output_sha256=driver.digest(primary_raw),clone_output_sha256=driver.digest(clone_raw))
    broker.call(dict(op='source-clone-observed',response=response))
