"""Private current-source, fixed-success orchestration; no restore authority.

Plans bind one reviewed installation. Live creation authority stays with the
shared native owner fd/flock. This module has no general plan or command CLI.
"""
import copy
import hashlib
import io
import ipaddress
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import time
import uuid
import zipfile

_TOKEN=object()
_RUNTIME_ADMITTED=object()
_BUILDER='sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b'
_POSTGRES='postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d'
_PLAN_KEYS={'format_version','capability','case','batch_id','source_case_id','target_case_id','cache_scope',
    'source_subnet','target_subnet','source_commit','public_archive_sha256','public_manifest_sha256',
    'legacy_archive_sha256','legacy_manifest_sha256','builder','postgres','budget_profile'}
_MAX_CACHE_BYTES=16*1024**3
_MAX_CACHE_ENTRIES=100000
_INSTALL_KEYS={'format_version','capability','source_commit','public_archive_sha256','public_manifest_sha256',
    'legacy_archive_sha256','legacy_manifest_sha256','adapter_sha256','docker_sha256','compose_sha256','builder','postgres','budget_profile'}

class ControlledRejected(RuntimeError):pass
def _require(ok,code):
    if not ok:raise ControlledRejected(code)
def _canonical(value):return json.dumps(value,sort_keys=True,ensure_ascii=False,separators=(',',':')).encode()
def _digest(raw):return hashlib.sha256(raw).hexdigest()
def _pairs(rows):
    value={}
    for key,item in rows:_require(key not in value,'CONTROLLED_DUPLICATE_JSON');value[key]=item
    return value
def _v4(value):
    try:return type(value) is str and str(uuid.UUID(value))==value and uuid.UUID(value).version==4
    except (ValueError,TypeError,AttributeError):return False

def _validate_plan(value):
    _require(type(value) is dict and set(value)==_PLAN_KEYS,'CONTROLLED_PLAN_FIELDS')
    _require(type(value['format_version']) is int and value['format_version']==1 and value['capability']=='c4_current11_first_success_plan_v1' and type(value['case']) is str and value['case']=='success' and value['budget_profile']=='controlled_fixture_two_role_first_success_v1','CONTROLLED_FIRST_SUCCESS_ONLY')
    keys=('batch_id','source_case_id','target_case_id','cache_scope')
    _require(all(_v4(value[k]) for k in keys) and len({value[k] for k in keys})==4,'CONTROLLED_FRESH_IDENTITIES')
    _require(value['builder']==_BUILDER and value['postgres']==_POSTGRES,'CONTROLLED_FIXED_IMAGES')
    _require(type(value['source_commit']) is str and re.fullmatch('[0-9a-f]{40}',value['source_commit']) is not None,'CONTROLLED_COMMIT')
    for key in ('public_archive_sha256','public_manifest_sha256','legacy_archive_sha256','legacy_manifest_sha256'):
        _require(type(value[key]) is str and re.fullmatch('[0-9a-f]{64}',value[key]) is not None,'CONTROLLED_PACKAGE_PIN')
    try:
        networks=[ipaddress.IPv4Network(value[k],strict=True) for k in ('source_subnet','target_subnet')]
        _require(all(type(value[k]) is str and str(n)==value[k] and n.prefixlen==24 and any(n.subnet_of(ipaddress.IPv4Network(p)) for p in ('10.0.0.0/8','172.16.0.0/12','192.168.0.0/16')) for n,k in zip(networks,('source_subnet','target_subnet'))) and not networks[0].overlaps(networks[1]),'CONTROLLED_SUBNETS')
    except (ValueError,TypeError):raise ControlledRejected('CONTROLLED_SUBNETS') from None
    return value

def _private_bytes(path,cap,mode):
    _require(sys.platform=='linux' and os.geteuid()==0,'CONTROLLED_NATIVE_ROOT_ONLY')
    path=Path(path)
    _require(path.is_absolute() and path==path.resolve(strict=True),'CONTROLLED_PRIVATE_PATH')
    for ancestor in path.parents:
        meta=os.lstat(ancestor)
        _require(stat.S_ISDIR(meta.st_mode) and meta.st_uid==0 and not meta.st_mode&0o022,'CONTROLLED_PRIVATE_ANCESTOR')
    fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC|os.O_NONBLOCK)
    try:
        meta=os.fstat(fd)
        _require(stat.S_ISREG(meta.st_mode) and meta.st_uid==0 and meta.st_nlink==1 and stat.S_IMODE(meta.st_mode)==mode and 0<meta.st_size<=cap,'CONTROLLED_PRIVATE_FILE')
        raw=bytearray()
        while len(raw)<=cap:
            chunk=os.read(fd,min(65536,cap+1-len(raw)))
            if not chunk:break
            raw.extend(chunk)
        after=os.fstat(fd)
        signature=lambda m:(m.st_dev,m.st_ino,m.st_size,m.st_mtime_ns,m.st_ctime_ns)
        _require(len(raw)==meta.st_size and signature(meta)==signature(after),'CONTROLLED_PRIVATE_CHANGED')
        return bytes(raw),dict(dev=meta.st_dev,ino=meta.st_ino,sha256=_digest(raw))
    finally:os.close(fd)

def _read_plan(path):
    raw,pin=_private_bytes(path,65536,0o600)
    try:value=json.loads(raw,object_pairs_hook=_pairs)
    except (ValueError,UnicodeError):raise ControlledRejected('CONTROLLED_PLAN_JSON') from None
    _require(raw==_canonical(value),'CONTROLLED_PLAN_CANONICAL')
    return _validate_plan(value),pin

def _read_install(suite):
    raw,pin=_private_bytes(Path(suite)/'current11/install.json',16384,0o400)
    value=json.loads(raw,object_pairs_hook=_pairs)
    _require(type(value) is dict and set(value)==_INSTALL_KEYS and raw==_canonical(value) and type(value['format_version']) is int and value['format_version']==1 and value['capability']=='c4_current11_install_v1','CONTROLLED_INSTALL_RECORD')
    _require(type(value['source_commit']) is str and re.fullmatch('[0-9a-f]{40}',value['source_commit']) is not None and value['builder']==_BUILDER and value['postgres']==_POSTGRES and value['budget_profile']=='controlled_fixture_two_role_first_success_v1','CONTROLLED_INSTALL_FIXED_SOURCE')
    _require(all(type(value[k]) is str and re.fullmatch('[0-9a-f]{64}',value[k]) is not None for k in _INSTALL_KEYS if k.endswith('_sha256')),'CONTROLLED_INSTALL_PINS')
    return value,pin

def _choose_subnets(occupied):
    _require(type(occupied) is list and all(type(value) is str for value in occupied),'CONTROLLED_OBSERVED_SUBNETS')
    try:existing=[ipaddress.ip_network(value,strict=False) for value in occupied if value!='default']
    except (ValueError,TypeError):raise ControlledRejected('CONTROLLED_OBSERVED_SUBNETS') from None
    candidates=[str(value) for value in ipaddress.ip_network('172.29.0.0/16').subnets(new_prefix=24) if not any(other.version==4 and value.overlaps(other) for other in existing)]
    _require(len(candidates)>=2,'CONTROLLED_FRESH_SUBNET_CAPACITY')
    return candidates[:2]

def _mint_plan_from_install():
    """One fixed read-only daemon preflight followed by an O_EXCL plan.

    This private Ops hook consumes the sealed installed source closure. Its
    metadata is not creation authority; the native owner is minted later.
    """
    source=Path(__file__).resolve().parents[2]
    match=re.fullmatch(r'(/var/lib/docker/volumes/kwc4c-suite-[0-9a-f]{32}/_data)/current11/bootstrap',str(source))
    _require(match is not None,'CONTROLLED_FIXED_BOOTSTRAP')
    suite=Path(match[1]);record,_=_read_install(suite)
    _require(not os.path.lexists(suite/'current11/plan.json') and not os.path.lexists(suite/'current11/plan-logs'),'CONTROLLED_PLAN_REPLAY')
    import p0c4_source_admission_gate_acceptance as admission
    from p0c4_completion import resources
    raw,_=_private_bytes(suite/'incoming/source-public.zip',32*1024**2,0o400)
    manifest,files=admission.verify_package(raw,record['public_archive_sha256'],record['public_manifest_sha256'],_digest(Path(admission.__file__).read_bytes()),'green')
    _require(manifest['base_commit']==record['source_commit'] and files['scripts/p0c4_completion/controlled_fixture.py']==Path(__file__).read_bytes(),'CONTROLLED_PLAN_SOURCE')
    admission.source_digest(source,manifest)
    tools=_verify_tools(suite)
    _require(tools=={k:record[k] for k in ('docker_sha256','compose_sha256')},'CONTROLLED_INSTALL_TOOL_BINDING')
    logs=suite/'current11/plan-logs';logs.mkdir(mode=0o700)
    runner=admission.Runner(logs)
    resources.require_host_namespace();_measure_outer(runner,suite)
    volume=runner.inspect('volume',suite.parent.name)
    _require(volume.get('Name')==suite.parent.name and volume.get('Mountpoint')==str(suite) and volume.get('Driver')=='local' and volume.get('Scope')=='local' and not volume.get('Options') and volume.get('Labels')=={'knowweave.c4.ops':'completion-prep'},'CONTROLLED_PLAN_ACTUAL_SUITE')
    names=runner.docker('network','ls','-q','--no-trunc',timeout=30).decode().split()
    _require(len(names)<=4096 and len(names)==len(set(names)) and all(re.fullmatch('[0-9a-f]{64}',name) for name in names),'CONTROLLED_PLAN_NETWORK_INVENTORY')
    occupied=[];deadline=time.monotonic()+30
    import p0c4_controlled_import_acceptance as bounded
    for batch in bounded._inspection_batches('network',names):
        _require(time.monotonic()<deadline,'CONTROLLED_PLAN_NETWORK_DEADLINE')
        rows=json.loads(runner.docker('network','inspect',*batch,timeout=max(0.001,deadline-time.monotonic())),object_pairs_hook=_pairs)
        _require(type(rows) is list and len(rows)==len(batch) and {r.get('Id') for r in rows}==set(batch),'CONTROLLED_PLAN_NETWORK_COVERAGE')
        for row in rows:occupied.extend(admission.occupied_subnets(row.get('IPAM',{}).get('Config')))
    routes,route_raw,route_record=resources.sample_host_routes()
    admission.write_new(suite/'current11/plan-routes.bin',route_raw,0o400)
    admission.write_new(suite/'current11/plan-routes.json',_canonical(route_record),0o400)
    occupied.extend(row['dst'] for row in routes)
    source_subnet,target_subnet=_choose_subnets(occupied)
    ids=[str(uuid.uuid4()) for _ in range(4)]
    plan=dict(format_version=1,capability='c4_current11_first_success_plan_v1',case='success',batch_id=ids[0],source_case_id=ids[1],target_case_id=ids[2],cache_scope=ids[3],source_subnet=source_subnet,target_subnet=target_subnet,**{k:record[k] for k in ('source_commit','public_archive_sha256','public_manifest_sha256','legacy_archive_sha256','legacy_manifest_sha256','builder','postgres','budget_profile')})
    _validate_plan(plan);_require(admission.source_digest(source,manifest) is not None,'CONTROLLED_PLAN_SOURCE_AFTER')
    path=suite/'current11/plan.json';fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,0o600)
    try:
        data=_canonical(plan);_require(os.write(fd,data)==len(data),'CONTROLLED_PLAN_WRITE');os.fsync(fd)
    finally:os.close(fd)
    parent=os.open(path.parent,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
    try:os.fsync(parent)
    finally:os.close(parent)
    observed,pin=_read_plan(path);_require(observed==plan,'CONTROLLED_PLAN_READBACK')
    return dict(scope='FRESH_FIRST_SUCCESS_PLAN_NOT_CREATION_AUTHORITY',plan_sha256=pin['sha256'],plan=observed)

def _verify_tools(suite):
    raw,_=_private_bytes(Path(suite)/'current11/tool-pins.json',4096,0o400)
    tools=json.loads(raw,object_pairs_hook=_pairs)
    _require(raw==_canonical(tools) and type(tools) is dict and set(tools)=={'docker_sha256','compose_sha256'},'CONTROLLED_TOOL_RECORD')
    for key,path in (('docker_sha256','/usr/bin/docker'),('compose_sha256','/usr/libexec/docker/cli-plugins/docker-compose')):
        _require(type(tools[key]) is str and re.fullmatch('[0-9a-f]{64}',tools[key]) is not None,'CONTROLLED_TOOL_PIN')
        fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC)
        try:
            meta=os.fstat(fd);_require(stat.S_ISREG(meta.st_mode) and meta.st_uid==0 and not meta.st_mode&0o022 and meta.st_nlink==1 and 0<meta.st_size<=64*1024**2,'CONTROLLED_TOOL_FILE')
            signature=lambda m:(m.st_dev,m.st_ino,m.st_size,m.st_mtime_ns,m.st_ctime_ns,m.st_nlink,m.st_uid,m.st_mode)
            value=hashlib.sha256();size=0
            while size<=64*1024**2:
                part=os.read(fd,65536)
                if not part:break
                size+=len(part);value.update(part)
            _require(size==meta.st_size and signature(meta)==signature(os.fstat(fd))==signature(os.lstat(path)) and value.hexdigest()==tools[key],'CONTROLLED_TOOL_CHANGED')
        finally:os.close(fd)
    return tools

def _outer_projection(facts,suite):
    host=facts.get('HostConfig',{});config=facts.get('Config',{})
    _require(type(facts.get('Id')) is str and re.fullmatch('[0-9a-f]{64}',facts['Id']) is not None and facts.get('Image')==config.get('Image')==_BUILDER and config.get('User')=='0:0' and facts.get('State',{}).get('Running') is True,'CONTROLLED_OUTER_IDENTITY')
    _require(host.get('NetworkMode')=='host' and host.get('ReadonlyRootfs') is True and host.get('Privileged') is False and host.get('PidMode','')=='' and host.get('CapAdd') in (None,[]) and host.get('CapDrop') in (['ALL'],['CAP_ALL']) and 'no-new-privileges' in host.get('SecurityOpt',[]),'CONTROLLED_OUTER_PRIVILEGES')
    expected={(str(suite),'volume',str(suite),True),*[(p,'bind',p,False) for p in ('/usr/bin/docker','/usr/libexec/docker/cli-plugins/docker-compose','/var/run/docker.sock')],('/run/knowweave-c4/host-netns','bind','/proc/1/ns/net',False)}
    mounts=facts.get('Mounts',[])
    _require(len(mounts)==5 and {(m.get('Destination'),m.get('Type'),m.get('Source'),m.get('RW')) for m in mounts}==expected and next(m for m in mounts if m['Type']=='volume').get('Name')==Path(suite).parent.name,'CONTROLLED_OUTER_EXACT_MOUNTS')
    return dict(container_id=facts['Id'],image_id=_BUILDER,mounts=[dict(type=m['Type'],source=m['Source'],destination=m['Destination'],writable=m['RW']) for m in mounts],host_network=True,readonly_rootfs=True)

def _measure_outer(runner,suite):
    # Docker's private UTS hostname is the default short immutable CID. This
    # fixed recipe deliberately provides no caller hostname/CID selector.
    fd=os.open('/etc/hostname',os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC)
    try:
        meta=os.fstat(fd);raw=os.read(fd,65)
        _require(stat.S_ISREG(meta.st_mode) and meta.st_uid==0 and not meta.st_mode&0o022 and len(raw)==meta.st_size and re.fullmatch(b'[0-9a-f]{12}\n',raw) is not None,'CONTROLLED_OUTER_DEFAULT_HOSTNAME')
    finally:os.close(fd)
    prefix=raw.decode().strip();row=runner.inspect('container',prefix)
    _require(row.get('Id','').startswith(prefix),'CONTROLLED_OUTER_CURRENT_CID')
    return _outer_projection(row,suite)

def _measure_cache(path,held_fd):
    """Observe the original owned cache through no-follow directory fds."""
    path=Path(path)
    _require(sys.platform=='linux' and os.geteuid()==0,'CONTROLLED_NATIVE_ROOT_ONLY')
    identity=lambda m:(m.st_dev,m.st_ino,m.st_uid,stat.S_IMODE(m.st_mode))
    original=os.fstat(held_fd);current=os.lstat(path)
    _require(stat.S_ISDIR(current.st_mode) and identity(current)==identity(original) and original.st_uid==0 and stat.S_IMODE(original.st_mode)==0o700,'CONTROLLED_CACHE_ROOT_CHANGED')
    total=entries=0
    def walk(fd,depth):
        nonlocal total,entries
        _require(depth<=128,'CONTROLLED_CACHE_DEPTH')
        before=os.fstat(fd)
        for name in os.listdir(fd):
            entries+=1;_require(entries<=_MAX_CACHE_ENTRIES,'CONTROLLED_CACHE_ENTRIES')
            meta=os.stat(name,dir_fd=fd,follow_symlinks=False)
            _require(meta.st_uid==0 and not meta.st_mode&0o022,'CONTROLLED_CACHE_OWNER')
            if stat.S_ISDIR(meta.st_mode):
                child=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=fd)
                try:
                    _require(identity(os.fstat(child))==identity(meta),'CONTROLLED_CACHE_CHANGED');walk(child,depth+1)
                finally:os.close(child)
            else:
                _require(stat.S_ISREG(meta.st_mode),'CONTROLLED_CACHE_SPECIAL')
                total+=meta.st_size;_require(total<=_MAX_CACHE_BYTES,'CONTROLLED_CACHE_BYTES')
                check=os.stat(name,dir_fd=fd,follow_symlinks=False)
                _require((meta.st_dev,meta.st_ino,meta.st_size,meta.st_mtime_ns,meta.st_ctime_ns)==(check.st_dev,check.st_ino,check.st_size,check.st_mtime_ns,check.st_ctime_ns),'CONTROLLED_CACHE_CHANGED')
        after=os.fstat(fd)
        _require((before.st_dev,before.st_ino,before.st_mtime_ns,before.st_ctime_ns)==(after.st_dev,after.st_ino,after.st_mtime_ns,after.st_ctime_ns),'CONTROLLED_CACHE_CHANGED')
    walk(held_fd,0)
    _require(identity(os.lstat(path))==identity(original),'CONTROLLED_CACHE_ROOT_CHANGED')
    return dict(dev=original.st_dev,ino=original.st_ino,bytes=total,entries=entries,bytes_cap=_MAX_CACHE_BYTES,entries_cap=_MAX_CACHE_ENTRIES,quota=False)

def _materialize_compiler_output(output,cache,build,source_pin,target_pin):
    """Validate compiler selection, then share the real streaming O_EXCL copy.

    The returned data receipt is not execution credit or a pin constructor.
    Actual compiler/container/current-input evidence is checked by the caller.
    """
    _require(type(output) is bytes and 0<len(output)<=16*1024**2,'CONTROLLED_COMPILER_OUTPUT')
    _require(all(type(p) is str and re.fullmatch('[0-9a-f]{64}',p) for p in (source_pin,target_pin)) and ((source_pin==target_pin=='0'*64) or (source_pin!=target_pin and '0'*64 not in (source_pin,target_pin))),'CONTROLLED_COMPILE_PINS')
    import p0c4_source_lifecycle_gate_acceptance as driver
    try:rows=[json.loads(line,object_pairs_hook=_pairs) for line in output.splitlines()]
    except (ValueError,UnicodeError):raise ControlledRejected('CONTROLLED_COMPILER_JSON') from None
    binaries=[r.get('executable') for r in rows if r.get('reason')=='compiler-artifact' and r.get('manifest_path')=='/reviewed/crates/learning-backup/Cargo.toml' and r.get('target',{}).get('name')=='learning_backup' and r.get('target',{}).get('kind')==['lib'] and r.get('profile',{}).get('test') is True]
    finished=[r for r in rows if r.get('reason')=='build-finished']
    _require(len(binaries)==1 and type(binaries[0]) is str and re.fullmatch('/cache/debug/deps/learning_backup-[0-9a-f]+',binaries[0]) is not None and len(finished)==1 and finished[0].get('success') is True,'CONTROLLED_EXACT_COMPILER_ARTIFACT')
    try:profile=driver.validate_compiler_profile(output.decode(),binaries[0],True)
    except driver.GateError:raise ControlledRejected('CONTROLLED_EXACT_COMPILER_PROFILE') from None
    cache,build=Path(cache),Path(build)
    directory=build/'materialized'
    if not os.path.lexists(directory):
        directory.mkdir(mode=0o700)
        fd=os.open(build,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
        try:os.fsync(fd)
        finally:os.close(fd)
    copied=driver.materialize_build_data(cache/'debug/deps'/Path(binaries[0]).name,directory/'learning-backup-tests')
    copied['source_path']=binaries[0];copied['destination_path']='/target/materialized/learning-backup-tests'
    return dict(format_version=1,capability='controlled_fixture_materialized_compiler_data_v1',
        binary_sha256=copied['sha256'],compiler_stdout_sha256=_digest(output),source_birth_sha256=source_pin,
        target_birth_sha256=target_pin,materialization=copied,profile=profile,executed=False)

def _read_materialized(build,output,source_pin,target_pin,cache=None):
    _require(type(output) is bytes and 0<len(output)<=16*1024**2,'CONTROLLED_COMPILER_OUTPUT')
    lines=output.splitlines(keepends=True);tags=[n for n,line in enumerate(lines) if line.startswith(b'CURRENT11_MATERIALIZED\t')]
    _require(tags==[len(lines)-1] and lines[-1].endswith(b'\n'),'CONTROLLED_MATERIALIZATION_TRANSCRIPT')
    compiler=b''.join(lines[:-1]);tag=lines[-1].removeprefix(b'CURRENT11_MATERIALIZED\t').removesuffix(b'\n')
    raw,_=_private_bytes(Path(build)/'materialized.json',16384,0o400)
    _require(tag==raw,'CONTROLLED_MATERIALIZATION_RECEIPT')
    value=json.loads(raw,object_pairs_hook=_pairs)
    keys={'format_version','capability','binary_sha256','compiler_stdout_sha256','source_birth_sha256','target_birth_sha256','materialization','profile','executed'}
    _require(type(value) is dict and set(value)==keys and raw==_canonical(value) and type(value['format_version']) is int and value['format_version']==1 and value['capability']=='controlled_fixture_materialized_compiler_data_v1' and value['executed'] is False and value['source_birth_sha256']==source_pin and value['target_birth_sha256']==target_pin and value['compiler_stdout_sha256']==_digest(compiler),'CONTROLLED_MATERIALIZATION_BINDING')
    import p0c4_source_lifecycle_gate_acceptance as driver
    rows=[json.loads(line,object_pairs_hook=_pairs) for line in compiler.splitlines()]
    _require(all(type(r) is dict for r in rows),'CONTROLLED_COMPILER_JSON')
    artifacts=[r for r in rows if r.get('reason')=='compiler-artifact' and r.get('manifest_path')=='/reviewed/crates/learning-backup/Cargo.toml' and r.get('target',{}).get('name')=='learning_backup' and r.get('target',{}).get('kind')==['lib'] and r.get('profile',{}).get('test') is True]
    _require(len(artifacts)==1 and re.fullmatch('/cache/debug/deps/learning_backup-[0-9a-f]+',artifacts[0].get('executable','')) is not None and [r.get('success') for r in rows if r.get('reason')=='build-finished']==[True],'CONTROLLED_EXACT_COMPILER_ARTIFACT')
    try:profile=driver.validate_compiler_profile(compiler.decode(),artifacts[0]['executable'],True)
    except driver.GateError:raise ControlledRejected('CONTROLLED_EXACT_COMPILER_PROFILE') from None
    _require(profile==value['profile'],'CONTROLLED_MATERIALIZATION_PROFILE')
    binary=Path(build)/'materialized'/'learning-backup-tests';data,pin=_private_bytes(binary,driver.BUILD_DATA_LIMIT,0o500)
    copied=value['materialization'];meta=os.lstat(binary)
    expected=dict(dev=meta.st_dev,ino=meta.st_ino,uid=meta.st_uid,mode=stat.S_IMODE(meta.st_mode),nlink=meta.st_nlink,bytes=meta.st_size)
    _require(type(copied) is dict and set(copied)=={'source_path','source','destination_path','destination','bytes','sha256'} and copied['destination_path']=='/target/materialized/learning-backup-tests' and copied['source_path']==artifacts[0]['executable'] and copied['destination']==expected and copied['bytes']==len(data) and copied['sha256']==value['binary_sha256']==pin['sha256'] and (copied['source']['dev'],copied['source']['ino'])!=(meta.st_dev,meta.st_ino),'CONTROLLED_MATERIALIZATION_COPY')
    if cache is not None:
        source=Path(cache)/'debug/deps'/Path(artifacts[0]['executable']).name;source_meta=os.lstat(source)
        _require(stat.S_ISREG(source_meta.st_mode) and copied['source']==dict(dev=source_meta.st_dev,ino=source_meta.st_ino,uid=source_meta.st_uid,mode=stat.S_IMODE(source_meta.st_mode),nlink=source_meta.st_nlink,bytes=source_meta.st_size),'CONTROLLED_MATERIALIZATION_CACHE_SOURCE')
    return binary,value

def _build_materialize_main():
    # The fixed builder command owns these three paths and all compile env.
    # No arbitrary source/destination/expected hash argument is accepted.
    sys.path.insert(0,'/reviewed/scripts')
    output,_=_private_bytes(Path('/target/compiler.jsonl'),16*1024**2,0o600)
    result=_materialize_compiler_output(output,Path('/cache'),Path('/target'),
        os.environ.get('KNOWWEAVE_C4_IMPORT_SOURCE_BIRTH_SHA256'),os.environ.get('KNOWWEAVE_C4_TARGET_BIRTH_SHA256'))
    path=Path('/target/materialized.json')
    fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,0o400)
    try:
        raw=_canonical(result);_require(os.write(fd,raw)==len(raw),'CONTROLLED_MATERIALIZATION_RECEIPT');os.fsync(fd)
    finally:os.close(fd)
    parent=os.open('/target',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:os.fsync(parent)
    finally:os.close(parent)
    sys.stdout.buffer.write(output)
    print('CURRENT11_MATERIALIZED\t'+_canonical(result).decode(),flush=True)
    return 0

class _VerifiedInstall:
    __slots__=('backend','plan','plan_pin','suite','source','suite_fd','source_fd','suite_id','source_id','files','record','record_pin','adapter_pin')
    def __init__(self,token,backend,plan,pin,suite,files,record,record_pin,adapter_pin):
        _require(token is _TOKEN,'CONTROLLED_INSTALL_READER_ONLY')
        self.backend,self.plan,self.plan_pin,self.suite,self.source=backend,copy.deepcopy(plan),pin,suite,backend.source
        self.files=files;self.suite_fd=self.source_fd=None
        self.record,self.record_pin,self.adapter_pin=record,record_pin,adapter_pin
        try:
            self.suite_fd=os.open(suite,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
            self.source_fd=os.open(self.source,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
            self.suite_id=self._id(os.fstat(self.suite_fd));self.source_id=self._id(os.fstat(self.source_fd))
            _require(self.suite_id[2:]==(0,0o700) and self.source_id[2:]==(0,0o700),'CONTROLLED_INSTALL_ROOTS')
        except BaseException:self.close();raise
    @staticmethod
    def _id(meta):return meta.st_dev,meta.st_ino,meta.st_uid,stat.S_IMODE(meta.st_mode)
    def recheck(self):
        _require(self.suite_fd is not None and self.source_fd is not None,'CONTROLLED_INSTALL_NOT_HELD')
        for path,fd,expected in ((self.suite,self.suite_fd,self.suite_id),(self.source,self.source_fd,self.source_id)):
            observed=os.lstat(path)
            _require(stat.S_ISDIR(observed.st_mode) and self._id(observed)==self._id(os.fstat(fd))==expected,'CONTROLLED_INSTALL_ROOT_CHANGED')
        plan,pin=_read_plan(self.suite/'current11'/'plan.json')
        _require(plan==self.plan and pin==self.plan_pin and self.backend.verify_source(),'CONTROLLED_INSTALL_SOURCE_CHANGED')
        record,record_pin=_read_install(self.suite)
        _,adapter_pin=_private_bytes(self.suite/'current11/adapter.py',128*1024,0o500)
        _require(record==self.record and record_pin==self.record_pin and adapter_pin==self.adapter_pin,'CONTROLLED_INSTALL_RECORD_CHANGED')
    def close(self):
        for key in ('suite_fd','source_fd'):
            fd=getattr(self,key,None)
            if fd is not None:os.close(fd);setattr(self,key,None)
    def __reduce_ex__(self,protocol):raise TypeError('controlled installation is process-local')

class ControlledFixtureContext:
    """Exact private orchestration type; never deserialized from observations."""
    __slots__=('_install','_backend','_plan','_suite','_runner','_h','_binding','_images','_result',
        '_sides','_active','_cache','_cache_fd','_cache_id','_tools','_closed','_native_broken','_body_used','_native_operation','_native_helpers','_budget','_compiled','_runtime_admission','_compiling','_observation_deadline')
    def __init__(self,token,install=None):
        _require(token is _TOKEN and type(install) is _VerifiedInstall,'CONTROLLED_CONTEXT_MINT_ONLY')
        install.recheck()
        self._install,self._backend,self._plan,self._suite=install,install.backend,install.plan,install.suite
        self._runner=None;self._cache=None;self._cache_fd=None;self._cache_id=None;self._tools=None
        self._sides={};self._active=None;self._closed=False;self._native_broken=False;self._body_used=False
        self._native_operation=None;self._native_helpers=[];self._budget=None
        self._compiled={}
        self._runtime_admission=None
        self._compiling=None
        self._observation_deadline=None
    def __reduce_ex__(self,protocol):raise TypeError('controlled context is process-local')
    def _alive(self):
        _require(not self._closed and not self._native_broken,'CONTROLLED_CONTEXT_UNUSABLE')
        self._install.recheck()
    def _owned_pg_names(self):
        self._alive()
        return tuple(self._backend.provisioner.identity_for(self._plan[role+'_case_id'])['project']+'-pg-1' for role in ('source','target'))

    def _runtime(self):
        self._alive();_require(self._runner is None,'CONTROLLED_RUNTIME_REUSE')
        import p0c4_source_admission_gate_acceptance as admission
        import p0c4_source_binding_gate_acceptance as binding
        import p0c4_source_lifecycle_gate_acceptance as driver
        from p0c4_completion import full_target_fs,resources
        for module in (admission,binding,driver,full_target_fs,resources):
            name='scripts/'+Path(module.__file__).resolve().relative_to(self._backend.source/'scripts').as_posix()
            _require(name in self._install.files and Path(module.__file__).read_bytes()==self._install.files[name],'CONTROLLED_RUNTIME_MODULE_SEAL')
        self._h,self._binding=admission,binding
        self._result=dict(batch_id=self._plan['batch_id'],cases=[],helpers=[],resource_creation_unknown=False)
        logs=self._backend.batch/'evidence'/'logs';self._backend.helper._private_dir(logs)
        self._runner=driver.owned_log_runner(admission,self._backend.batch,self._result,0)
        self._runner.ownership.case_id=self._plan['source_case_id'];self._runner.ownership._controlled_pair=self
        suite_name=self._suite.parent.name
        suite=self._runner.inspect('volume',suite_name)
        _require(suite['Name']==suite_name and suite['Mountpoint']==str(self._suite) and suite['Driver']=='local' and suite.get('Scope')=='local' and not suite.get('Options') and suite.get('Labels')=={'knowweave.c4.ops':'completion-prep'},'CONTROLLED_ACTUAL_SUITE')
        resources.require_host_namespace()
        self._tools=_verify_tools(self._suite)
        self._result['outer_capsule_observation']=_measure_outer(self._runner,self._suite)
        image=self._runner.inspect('image',_BUILDER)
        _require(image['Id']==_BUILDER,'CONTROLLED_BUILDER_IMAGE');self._images={_BUILDER:image}
        self._backend.provisioner._command=self._legacy_command
        self._backend.provisioner._inspect=self._inspect
        self._backend.provisioner.snapshot=self._snapshot
        self._runtime_admission=_RUNTIME_ADMITTED

    def _prepare_cache(self):
        self._alive();_require(self._runtime_admission is _RUNTIME_ADMITTED,'CONTROLLED_CACHE_RUNTIME_ADMISSION')
        if self._cache is not None:return _measure_cache(self._cache,self._cache_fd)
        parent=self._suite/'current11'/'cache'
        current_fd=os.open('current11',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=self._install.suite_fd)
        try:
            meta=os.fstat(current_fd);_require(meta.st_uid==0 and stat.S_IMODE(meta.st_mode)==0o700,'CONTROLLED_CACHE_PARENT')
            os.mkdir('cache',mode=0o700,dir_fd=current_fd)  # no old/unknown cache admission
            os.fsync(current_fd)
            parent_fd=os.open('cache',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=current_fd)
            try:
                scope=self._plan['cache_scope'];os.mkdir(scope,mode=0o700,dir_fd=parent_fd);os.fsync(parent_fd)
                fd=os.open(scope,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=parent_fd)
                self._cache,self._cache_fd=parent/scope,fd;self._cache_id=(os.fstat(fd).st_dev,os.fstat(fd).st_ino)
                os.fsync(fd)
                _require((os.lstat(self._suite/'current11').st_dev,os.lstat(self._suite/'current11').st_ino)==(meta.st_dev,meta.st_ino),'CONTROLLED_CACHE_PARENT_CHANGED')
            finally:os.close(parent_fd)
        finally:os.close(current_fd)
        cache=self._cache
        return _measure_cache(cache,fd)

    def _begin_compile(self,source_pin,target_pin,placeholder):
        self._alive();_require(type(placeholder) is bool and self._compiling is None,'CONTROLLED_COMPILE_SEQUENCE')
        _require(all(type(p) is str and re.fullmatch('[0-9a-f]{64}',p) for p in (source_pin,target_pin)),'CONTROLLED_COMPILE_PINS')
        if placeholder:
            _require(source_pin==target_pin=='0'*64 and not self._sides and not self._compiled,'CONTROLLED_PLACEHOLDER_ORDER')
        else:
            _require(set(self._sides)=={'source','target'} and 'preflight' in self._compiled and all(self._sides[r]['status'] is not None for r in ('source','target')),'CONTROLLED_LIVE_BIRTH_REQUIRED')
            _require(source_pin==self._sides['source']['status']['birth_sha256'] and target_pin==self._sides['target']['status']['birth_sha256'] and source_pin!=target_pin and '0'*64 not in (source_pin,target_pin),'CONTROLLED_ACTUAL_DISTINCT_BIRTH_PINS')
        _require(self._runtime_admission is _RUNTIME_ADMITTED,'CONTROLLED_COMPILE_RUNTIME_ADMISSION')
        stage='preflight' if placeholder else 'live';_require(stage not in self._compiled,'CONTROLLED_COMPILE_REPLAY')
        before=self._prepare_cache()
        self._compiling=dict(stage=stage,source_pin=source_pin,target_pin=target_pin,cache_before=before,command=None)

    def _adapt_builder_command(self,command,source,batch,build_name,target_pin):
        self._alive();state=self._compiling
        _require(state is not None and source==self._backend.source and batch==self._backend.batch and build_name=='probe-'+state['stage']+'-build' and target_pin==state['target_pin'],'CONTROLLED_BUILDER_BINDING')
        name,label=self._backend.delegate._builder_identity(batch,state['stage'])
        expected=['/usr/bin/docker','run','--rm','--pull','never','--name',name,'--label',label,
            '--network','none','--cap-drop','ALL','--security-opt','no-new-privileges','--user','0:0',
            '--workdir','/reviewed','--tmpfs','/tmp:rw,nosuid,nodev,size=1g',
            '--mount',f'type=bind,src={source},dst=/reviewed,readonly',
            '--mount',f'type=bind,src={batch/build_name},dst=/target',
            '--env','CARGO_TARGET_DIR=/target','--env','CARGO_NET_OFFLINE=true',
            '--env','RUSTUP_AUTO_INSTALL=0','--env','CARGO_TERM_COLOR=never',
            '--env','KNOWWEAVE_C4_TARGET_BIRTH_SHA256='+target_pin,
            '--entrypoint','/bin/sh',_BUILDER,'-ec',
            'cargo test --locked --offline -p learning-backup --lib --no-run --message-format=json']
        command=list(command);_require(state['command'] is None and command==expected,'CONTROLLED_BUILDER_COMMAND')
        command[command.index('CARGO_TARGET_DIR=/target')]='CARGO_TARGET_DIR=/cache'
        import p0c4_source_lifecycle_gate_acceptance as driver
        index=command.index('--entrypoint');env=['CARGO_INCREMENTAL=0',*[k+'='+v for k,v in sorted(driver.BUILD_PROFILE_ENV.items())]]
        command[index:index]=['--mount',f'type=bind,src={self._cache},dst=/cache',*[word for item in env for word in ('--env',item)]]
        command[-1]='umask 077; cargo test --locked --offline -p learning-backup --lib --no-run --message-format=json > /target/compiler.jsonl; python3 -u /reviewed/scripts/p0c4_completion/controlled_fixture.py --fixed-build-materialize'
        state['command']=list(command)
        return command

    def _finish_compile(self,build,output):
        self._alive();state=self._compiling
        _require(state is not None and state['command'] is not None and build==self._backend.batch/('probe-'+state['stage']+'-build'),'CONTROLLED_COMPILE_SEQUENCE')
        binary,value=_read_materialized(build,output,state['source_pin'],state['target_pin'],self._cache)
        value['cache_before']=state['cache_before'];value['cache_after']=_measure_cache(self._cache,self._cache_fd)
        # The disk receipt remains the exact materializer receipt; this richer
        # process-local observation is never deserialized into authority.
        self._compiled[state['stage']]=value;self._compiling=None
        return binary,value['binary_sha256']

    def _fail_compile(self):
        self._compiling=None;self._native_broken=True

    def _builder_projection(self,command,row):
        self._alive();state=self._compiling
        _require(state is not None and state['command'] is not None,'CONTROLLED_COMPILE_SEQUENCE')
        expected=list(state['command']);index=expected.index('--entrypoint')
        expected[index:index]=['--cpus=4','--memory=8g','--memory-swap=8g','--env','KNOWWEAVE_C4_IMPORT_SOURCE_BIRTH_SHA256='+state['source_pin']]
        _require(command==expected,'CONTROLLED_EXACT_BUILDER_COMMAND')
        mounts=row.get('Mounts',[]);cache=[m for m in mounts if m.get('Destination')=='/cache']
        _require(len(mounts)==3 and len(cache)==1 and cache[0].get('Type')=='bind' and cache[0].get('Source')==str(self._cache) and cache[0].get('RW') is True and not any(m.get('Type')=='volume' for m in mounts),'CONTROLLED_EXACT_CACHE_MOUNT')
        config=row.get('Config',{});image=self._images[_BUILDER].get('Config',{})
        env=[command[i+1] for i,w in enumerate(command[:-1]) if w=='--env']
        base={item.split('=',1)[0]:item for item in image.get('Env',[])}
        base.update({item.split('=',1)[0]:item for item in env})
        _require(len(config.get('Env',[]))==len(base) and set(config.get('Env',[]))==set(base.values()) and config.get('Entrypoint')==['/bin/sh'] and config.get('Cmd')==command[-2:] and config.get('WorkingDir')=='/reviewed','CONTROLLED_EXACT_BUILDER_CONFIG')
        _require((os.lstat(self._cache).st_dev,os.lstat(self._cache).st_ino)==self._cache_id==(os.fstat(self._cache_fd).st_dev,os.fstat(self._cache_fd).st_ino),'CONTROLLED_CACHE_ROOT_CHANGED')
        projected=copy.deepcopy(row);projected['Mounts']=[m for m in mounts if m is not cache[0]]
        reduced=list(command);index=reduced.index('type=bind,src='+str(self._cache)+',dst=/cache');del reduced[index-1:index+1]
        return reduced,projected

    def _legacy_command(self,binary,*args):
        self._alive();_require(binary=='/usr/bin/docker','CONTROLLED_FIXED_DOCKER')
        return self._docker(*args)

    def _timeout(self,maximum):
        self._backend.commands.require_usable()
        now=time.monotonic();deadline=now+maximum
        for bound in (self._backend.commands.isolation_deadline,self._observation_deadline):
            if bound is not None:deadline=min(deadline,bound)
        _require(deadline>now,'CONTROLLED_OBSERVATION_DEADLINE')
        return max(0.001,deadline-now-min(0.25,(deadline-now)/4))

    def _docker(self,*args):
        self._alive();_require(self._runner is not None,'CONTROLLED_RUNTIME_REQUIRED')
        if args and args[0]=='compose':
            side=self._current();path=str(side['root']/'targets'/side['case_id']/'compose.json')
            _require(args[:3]==('compose','-f',path) and args[3:] in (('config','-q'),('up','-d','--wait','--no-build','--no-deps','pg')) and side['compose_bytes'] is not None,'CONTROLLED_COMPOSE_COMMAND')
            _require(_digest(side['compose_bytes'])==side['compose_sha'],'CONTROLLED_COMPOSE_BYTES')
            call=lambda:self._runner.docker('compose','-f','-',*args[3:],stdin=side['compose_bytes'],timeout=self._timeout(30))
            if args[3]=='up':
                import p0c4_source_lifecycle_gate_acceptance as driver
                _require(side['creation'] is not None,'CONTROLLED_CREATION_NOT_HELD')
                return driver.creation_call(self._runner,side['creation'],call).decode()
            return call().decode()
        return self._runner.docker(*args,timeout=self._timeout(30)).decode()

    def _inspect(self,kind,ids):
        self._alive()
        result=[]
        for value in ids:
            args=('inspect',value) if kind=='container' else (kind,'inspect',value)
            rows=json.loads(self._docker(*args),object_pairs_hook=_pairs)
            _require(type(rows) is list and len(rows)==1,'CONTROLLED_INSPECT_COUNT');result.extend(rows)
        return result

    def _snapshot(self):
        previous=self._observation_deadline
        self._observation_deadline=min(previous,time.monotonic()+30) if previous is not None else time.monotonic()+30
        try:return self._snapshot_inner()
        finally:self._observation_deadline=previous

    def _snapshot_inner(self):
        self._alive()
        from p0c4_completion import resources
        import p0c4_maintenance_gate_acceptance as metadata
        import p0c4_controlled_import_acceptance as bounded
        daemon=self._docker('info','--format','{{.ID}}').strip()
        ids={'container':self._docker('ps','-aq','--no-trunc').split(),
             'network':self._docker('network','ls','-q','--no-trunc').split(),
             'volume':self._docker('volume','ls','--format','{{.Name}}').split()}
        rows={};total=0
        for kind,values in ids.items():
            collected=[]
            for batch in bounded._inspection_batches(kind,values):
                command=['inspect'] if kind=='container' else [kind,'inspect']
                raw=self._runner.docker(*command,'--format',metadata._inventory_format(kind),*batch,timeout=self._timeout(5))
                total+=len(raw);_require(total<=bounded.INSPECT_TOTAL_STDOUT_LIMIT,'CONTROLLED_INVENTORY_CAP')
                part=metadata.parse_inventory(kind,raw)
                observed=[r['Name'] if kind=='volume' else r['Id'] for r in part]
                _require(len(observed)==len(batch) and set(observed)==set(batch),'CONTROLLED_INVENTORY_COVERAGE')
                collected.extend(part)
            for index,row in enumerate(collected):
                for side in self._sides.values():
                    expected={'container':'/'+side['identity']['project']+'-pg-1','network':side['identity']['network'],'volume':side['identity']['volume']}
                    if row['Name']==expected[kind]:
                        collected[index]=self._inspect(kind,[row['Name'] if kind=='volume' else row['Id']])[0]
            rows[kind]=collected
        routes,raw,record=resources.sample_host_routes()
        observations=self._result.setdefault('host_route_observations',[])
        _require(len(observations)<64,'CONTROLLED_ROUTE_OBSERVATION_CAP')
        stem='controlled-host-routes-'+str(len(observations)).zfill(2)
        self._backend.helper._private_write(self._backend.batch/'evidence'/(stem+'.bin'),raw)
        self._backend.helper._private_write(self._backend.batch/'evidence'/(stem+'.json'),_canonical(record))
        observations.append(dict(record=stem+'.json',raw=stem+'.bin',sha256=_digest(raw),bytes=len(raw)))
        _require(self._timeout(0.01)>0,'CONTROLLED_OBSERVATION_DEADLINE')
        return dict(daemon_id=daemon,**{k+'s':rows[k] for k in rows},routes=[r['dst'] for r in routes])

    def _current(self):
        self._alive();_require(self._active in self._sides,'CONTROLLED_ACTIVE_SIDE_REQUIRED')
        return self._sides[self._active]

    def _create(self,role,batch_id,subnet):
        self._alive();_require(role in ('source','target') and role not in self._sides and batch_id==self._plan[role+'_case_id'] and subnet==self._plan[role+'_subnet'],'CONTROLLED_FIXED_SIDE')
        if self._runner is None:self._runtime()
        from p0c4_completion import full_target_fs as fs
        import p0c4_source_lifecycle_gate_acceptance as driver
        backend=self._backend;identity=backend.provisioner.identity_for(batch_id);root=backend.batch/(role+'-control')
        side=dict(role=role,case_id=batch_id,subnet=subnet,root=root,identity=identity,owner=None,creation=None,status=None,ids=None,compose_sha=None,compose_bytes=None,pg=None)
        self._sides[role]=side;self._active=role
        row=dict(role=role,identity=identity,control=root,batch_id=batch_id,subnet=subnet,container_id=None,before=self._snapshot())
        backend.resources.append(row);side['record']=row
        mounts,command=self._helper_contract('controlled-owner')
        spec=fs._HelperProfile(fs._TOKEN,'controlled-owner',mounts,command,True,_controlled=self)
        helper=driver.HelperContainer(self._h,self._binding,self._runner,self._result,self._images[_BUILDER],mounts,command,interactive=True,_full_profile=spec)
        self._budget=driver.CaseBudget()
        broker=driver.BrokerClient(helper,backend.batch/'evidence'/'logs',self._budget)
        side['owner']=fs._OwnerClient(fs._TOKEN,helper,broker)
        config=dict(batch_id=self._plan['batch_id'],source_case_id=self._plan['source_case_id'],target_case_id=self._plan['target_case_id'],subnet=subnet,root=str(root),suite_root=str(self._suite),source_root=str(backend.source),case_root=str(backend.batch),role=role)
        try:
            side['owner'].call('setup',config)
            side['owner'].call('pin',dict(before=self._snapshot()))
            side['status']=backend.provisioner._provision(root,batch_id,subnet,backend.initdb,_controlled_profile=self)
            row['container_id']=side['status']['container_id']
            side['owner'].close();side['owner']=None
            _,inspection=self._peer_inspect(role)
            row['inspection_sha256']=_digest(backend.helper._json_bytes(inspection))
            return row
        except BaseException:
            self._native_broken=True
            raise
        finally:self._active=None

    def _validate_target(self,root,batch_id,subnet,initdb):
        side=self._current()
        _require(root==side['root'] and batch_id==side['case_id'] and subnet==side['subnet'] and initdb==self._backend.initdb and side['owner'] is not None,'CONTROLLED_TARGET_BINDING')
        side['owner'].held()
        current,_=_private_bytes(initdb,65536,0o444)
        _require(current==self._install.files['deploy/p0c4_restore_initdb.sh'],'CONTROLLED_OLD_INITDB_ONLY')

    def _held_files(self):
        from contextlib import contextmanager
        @contextmanager
        def held():
            owner=self._current()['owner'];owner.held();yield;owner.held()
        return held()

    def _create_target_files(self,before):
        side=self._current();side['compose_sha']=side['owner'].call('files',dict(before=before))['compose_sha256']

    def _extend_target_document(self,document):
        from p0c4_completion import full_target_fs
        side=self._current();full_target_fs._extend_secret_document(document,side['root']/'targets'/side['case_id'])

    def _verify_compose_bytes(self,raw):
        side=self._current();_require(_digest(raw)==side['compose_sha'],'CONTROLLED_COMPOSE_BYTES');side['compose_bytes']=bytes(raw)

    def _before_target_create(self,before):
        side=self._current();side['owner'].held()
        _require(side['creation'] is None,'CONTROLLED_CREATE_REPLAY')
        name=side['identity']['project']+'-pg-1'
        expected=dict(name=name,labels={'com.docker.compose.project':side['identity']['project'],'com.docker.compose.service':'pg'})
        side['creation']=self._runner.ownership.acquire('pg',name,expected,dict(name=side['identity']['network']),side['record'])

    def _target_created(self,before,after,ids):
        side=self._current();_require(side['creation'] is not None and side['creation']['state']=='sent','CONTROLLED_OWNER_BEFORE_SEND')
        self._runner.ownership.known(side['creation'],ids['container_id']);side['ids']=copy.deepcopy(ids)
        side['record']['container_id']=ids['container_id'];side['owner'].call('created',dict(before=before,after=after,ids=ids))

    def _require_current_container(self,identity,container_id):
        side=self._current();owner=side['creation'];side['owner'].held()
        _require(identity==side['identity'] and owner is not None and owner['state']=='known' and owner['id']==container_id and side['ids']['container_id']==container_id and self._runner.ownership.owners.get(owner['intent']['name']) is owner,'CONTROLLED_EXACT_CREATED_CONTAINER')

    def _postgres_uid(self):return 999,999
    def _configure(self,identity,container_id):self._require_current_container(identity,container_id)
    def _probe(self,identity,container_id):
        self._require_current_container(identity,container_id)
        self._backend.provisioner.probe_initdb(identity,container_id)
    def _verify_target_files(self,status):
        _require(self._current()['owner'].call('check',dict(status=status))==dict(checked=True),'CONTROLLED_CREATION_FILES')

    def _measure_birth_pg(self,ids):
        from p0c4_completion import full_target_fs as fs
        import p0c4_source_lifecycle_gate_acceptance as driver
        side=self._current();self._require_current_container(side['identity'],ids['container_id'])
        row=self._runner.inspect('volume',ids['volume_name'])
        _require(row['Name']==side['identity']['volume'] and row['Mountpoint']==ids['volume_mountpoint'] and row['Driver']=='local' and row.get('Scope')=='local' and not row.get('Options') and (row.get('Labels') or {}).get('com.docker.compose.project')==side['identity']['project'],'CONTROLLED_OWNED_PG_VOLUME')
        side['pg']=row
        other=self._plan[('target' if side['role']=='source' else 'source')+'_case_id']
        request=dict(phase='birth',source_case_id=other,target_case_id=side['case_id'],roots=dict(target_pg=dict(name=row['Name'],path=row['Mountpoint'])))
        mounts=[('volume',row['Name'],row['Mountpoint'],True)];command=['python3','-I','-u','-c',fs.MEASURE_PROGRAM]
        spec=fs._HelperProfile(fs._TOKEN,'pg',mounts,command,True)
        helper=driver.HelperContainer(self._h,self._binding,self._runner,self._result,self._images[_BUILDER],mounts,command,interactive=True,_full_profile=spec)
        code,out,err=helper.run(stdin=_canonical(request),timeout=30)
        _require(code==0 and not err and 0<len(out)<=1024**2,'CONTROLLED_PG_NATIVE_MEASUREMENT')
        value=json.loads(out,object_pairs_hook=_pairs)
        _require(set(value)=={'phase','source_case_id','target_case_id','roots'} and all(value[k]==request[k] for k in ('phase','source_case_id','target_case_id')) and set(value['roots'])=={'target_pg'},'CONTROLLED_PG_MEASUREMENT_BINDING')
        meta=value['roots']['target_pg']['identity']
        measured=value['roots']['target_pg']
        _require(type(measured) is dict and set(measured)=={'identity','tree_sha256','entries','bytes'} and type(measured['tree_sha256']) is str and re.fullmatch('[0-9a-f]{64}',measured['tree_sha256']) is not None and type(measured['entries']) is int and 0<=measured['entries']<=100010 and type(measured['bytes']) is int and 0<=measured['bytes']<2**64,'CONTROLLED_PG_MEASUREMENT_SHAPE')
        _require(set(meta)=={'path','dev','ino','uid','gid','mode'} and meta['path']==row['Mountpoint'] and all(type(meta[k]) is int and 0<meta[k]<2**64 for k in ('dev','ino')) and meta['uid']==meta['gid']==999 and type(meta['mode']) is int and not meta['mode']&0o022,'CONTROLLED_PG_REAL_INODE')
        side['record']['pg_root_measurement']=value
        return meta['dev'],meta['ino']

    def _publish_target_birth(self,status,ids,image_id,started_at,volume_id,facts):
        side=self._current();self._require_current_container(side['identity'],ids['container_id'])
        return side['owner'].call('birth',dict(status=status,ids=ids,image_id=image_id,started_at=started_at,volume_id=list(volume_id),facts=facts,daemon_id=self._snapshot()['daemon_id']))

    def _helper_contract(self,kind):
        self._alive()
        suite=[('volume',self._suite.parent.name,str(self._suite),False)]
        if kind=='controlled-owner':
            self._current()
            return suite,['python3','-B','-u',str(self._backend.source/'scripts/p0c4_completion/full_target_fs.py'),'--fixed-controlled-owner']
        _require(kind=='controlled-native' and self._native_operation in ('inspect-source','inspect-target','body'),'CONTROLLED_FIXED_NATIVE_OPERATION')
        volumes=[]
        for role in ('source','target'):
            side=self._sides.get(role)
            if side is None:
                _require(self._native_operation=='inspect-source' and role=='target','CONTROLLED_BOTH_BIRTHS_REQUIRED')
                continue
            _require(side['status'] is not None and side['pg'] is not None and side['creation']['state']=='known','CONTROLLED_NATIVE_CREATED_OWNER')
            volumes.append(('volume',side['pg']['Name'],side['pg']['Mountpoint'],True))
        fixed=[('bind',p,p,True) for p in ('/usr/bin/docker','/usr/libexec/docker/cli-plugins/docker-compose','/var/run/docker.sock')]+[('bind','/proc/1/ns/net','/run/knowweave-c4/host-netns',True)]
        return suite+volumes+fixed,['python3','-B','-u',str(self._backend.source/'scripts/p0c4_completion/controlled_fixture.py'),'--fixed-'+self._native_operation]

    def _validate_helper_contract(self,kind,mounts,command,interactive):
        expected,argv=self._helper_contract(kind)
        _require(tuple(mounts)==tuple(expected) and tuple(command)==tuple(argv) and interactive is (kind=='controlled-owner'),'CONTROLLED_EXACT_HELPER_CONTRACT')

    def _retain_fs_owner(self,helper):
        for side in self._sides.values():
            owner=side['owner'];created=side['creation']
            if owner is not None and owner._container is helper and owner._state not in ('birth','release') and (self._native_broken or created is not None and created['state'] not in ('planned','absent','stopped')):
                helper.record['owner_retained']=dict(state='UNCONFIRMED_UNUSABLE',lock_state='UNKNOWN',no_replay=True)
                return True
        return self._native_broken and helper in self._native_helpers and not helper.closed

    def _retire_fs_owner_transport(self,helper):
        _require(self._retain_fs_owner(helper),'CONTROLLED_UNKNOWN_OWNER_BINDING')
        # Detach/reap only this actual CLI transport; leave the exact native
        # Docker helper and its held lock recorded. No stop/rm/retry here.
        for side in self._sides.values():
            owner=side['owner']
            if owner is not None and owner._container is helper:
                owner._failure='HELPER_PROTOCOL';owner._broker.broken=True
                child=owner._broker.child
                if not child.retired and not child.retire_attempted:child.retire(timeout=5)
                return

    def _native(self,operation):
        self._alive();_require(self._runner is not None and self._native_operation is None,'CONTROLLED_NATIVE_SEQUENCE')
        from p0c4_completion import full_target_fs as fs
        import p0c4_source_lifecycle_gate_acceptance as driver
        self._native_operation=operation
        try:
            mounts,command=self._helper_contract('controlled-native')
            spec=fs._HelperProfile(fs._TOKEN,'controlled-native',mounts,command,False,_controlled=self)
            helper=driver.HelperContainer(self._h,self._binding,self._runner,self._result,self._images[_BUILDER],mounts,command,_full_profile=spec)
            self._native_helpers.append(helper)
            code,out,err=helper.run(timeout=360 if operation=='body' else 30,allowed=(0,101) if operation=='body' else (0,))
            self._install.recheck()
            return subprocess.CompletedProcess(command,code,out,err)
        except BaseException:
            self._native_broken=True
            raise
        finally:self._native_operation=None

    def _peer_inspect(self,role):
        self._alive();_require(role in ('source','target') and role in self._sides and self._sides[role]['status'] is not None,'CONTROLLED_BORN_SIDE_REQUIRED')
        process=self._native('inspect-'+role)
        _require(process.returncode==0 and not process.stderr and 0<len(process.stdout)<=1024**2,'CONTROLLED_PEER_INSPECTION_PROCESS')
        value=json.loads(process.stdout,object_pairs_hook=_pairs)
        _require(_canonical(value)==process.stdout and type(value) is dict and set(value)=={'candidate','inspection'},'CONTROLLED_PEER_INSPECTION_SHAPE')
        side=self._sides[role]
        _require(value['candidate']['birth_sha256']==side['status']['birth_sha256'] and value['inspection']['live']['container_id']==side['status']['container_id'],'CONTROLLED_PEER_CURRENT_BIRTH')
        return value['candidate'],value['inspection']

    def _execute_body(self):
        self._alive()
        _require(not self._body_used and set(self._sides)=={'source','target'} and 'live' in self._compiled,'CONTROLLED_LIVE_COMPILE_REQUIRED')
        build=self._compiled['live']
        _require(build['source_birth_sha256']==self._sides['source']['status']['birth_sha256'] and build['target_birth_sha256']==self._sides['target']['status']['birth_sha256'] and build['source_birth_sha256']!=build['target_birth_sha256'] and '0'*64 not in (build['source_birth_sha256'],build['target_birth_sha256']),'CONTROLLED_ACTUAL_DISTINCT_BIRTH_PINS')
        self._backend.helper._private_write(self._backend.batch/'body.dispatched',_canonical(dict(scope='EXACT_SUCCESS_BODY_DISPATCHED_NOT_ACCEPTANCE',binary_sha256=build['binary_sha256'])))
        self._body_used=True
        return self._native('body')

    def observation(self):
        compiled={stage:{k:value[k] for k in ('binary_sha256','compiler_stdout_sha256','source_birth_sha256','target_birth_sha256','materialization','profile','cache_before','cache_after','executed')} for stage,value in self._compiled.items()}
        return dict(scope='CONTROLLED_FIXTURE_ORCHESTRATION_NOT_AUTHORITY',plan_sha256=self._install.plan_pin['sha256'],body_dispatched=self._body_used,unusable=self._native_broken,compilations=compiled,resources=self._result if self._runner is not None else None)

    def release(self):
        if self._closed:return
        if self._runner is not None:
            import p0c4_source_lifecycle_gate_acceptance as driver
            errors=driver.cleanup_known_helpers(self._runner,self._result)
            self._result['helper_settlement_errors']=errors
            owners=self._runner.ownership.owners.values()
            if self._result.get('resource_creation_unknown') is True or any(o['intent']['kind']=='helper' and (o['state']!='absent' or o['record'].get('removed') is not True) for o in owners) or any(not child.finished and not child.retired for child in self._runner.ownership.children):
                self._native_broken=True
        if self._native_broken:return
        if self._cache_fd is not None:os.close(self._cache_fd);self._cache_fd=None
        self._install.close();self._closed=True

def _native_snapshot():
    """The unchanged legacy inspector plus the held host-netns route reader."""
    import p0c4_restore_target as target
    from p0c4_completion import resources
    containers=target._docker('ps','-aq','--no-trunc').split()
    networks=target._docker('network','ls','-q','--no-trunc').split()
    volumes=target._docker('volume','ls','--format','{{.Name}}').split()
    return dict(daemon_id=target._docker('info','--format','{{.ID}}').strip(),
        containers=target._inspect('container',containers),networks=target._inspect('network',networks),
        volumes=target._inspect('volume',volumes),routes=[r['dst'] for r in resources.sample_host_routes()[0]])

def _native_main(operation):
    """Only three fixed native commands; no input files/commands/SQL in argv."""
    _require(operation in ('inspect-source','inspect-target','body'),'CONTROLLED_NATIVE_OPERATION')
    source=Path(__file__).resolve().parents[2]
    match=re.fullmatch(r'(/var/lib/docker/volumes/kwc4c-suite-[0-9a-f]{32}/_data)/controlled-import/batches/([0-9a-f-]{36})/source',str(source))
    _require(match is not None,'CONTROLLED_NATIVE_SOURCE_PATH')
    suite=Path(match[1]);batch=source.parent;plan,_=_read_plan(suite/'current11'/'plan.json')
    _require(plan['batch_id']==match[2],'CONTROLLED_NATIVE_BATCH')
    sys.path.insert(0,str(source/'scripts'))
    import p0c4_controlled_import_acceptance as runner
    import p0c4_import_fixture as clients
    import p0c4_restore_target as target
    import p0c4_restore_target_pin as pin
    import p0c4_restore_pin_acceptance as delegate
    import p0c4_source_admission_gate_acceptance as admission
    from p0c4_completion import resources
    resources.require_host_namespace()
    record,_=_read_install(suite);_,adapter_pin=_private_bytes(suite/'current11/adapter.py',128*1024,0o500)
    _require(adapter_pin['sha256']==record['adapter_sha256'] and all(plan[k]==record[k] for k in ('source_commit','public_archive_sha256','public_manifest_sha256','legacy_archive_sha256','legacy_manifest_sha256','builder','postgres','budget_profile')),'CONTROLLED_NATIVE_INSTALL_BINDING')
    _require(_verify_tools(suite)=={k:record[k] for k in ('docker_sha256','compose_sha256')},'CONTROLLED_NATIVE_INSTALL_TOOLS')
    raw,_=_private_bytes(suite/'incoming/source-public.zip',32*1024**2,0o400)
    manifest,files=admission.verify_package(raw,plan['public_archive_sha256'],plan['public_manifest_sha256'],_digest(Path(admission.__file__).read_bytes()),'green')
    _require(manifest['base_commit']==plan['source_commit'],'CONTROLLED_NATIVE_COMMIT')
    source_hash=admission.source_digest(source,manifest)
    for module in (runner,clients,target,pin,delegate,admission,resources):
        path=Path(module.__file__).resolve()
        name=path.relative_to(source).as_posix()
        _require(name in files and path.read_bytes()==files[name],'CONTROLLED_NATIVE_MODULE_GRAPH')
    _require(files['scripts/p0c4_completion/controlled_fixture.py']==Path(__file__).read_bytes(),'CONTROLLED_NATIVE_SELF_SEAL')
    runner.fixture=clients;commands=runner.BoundedCommands()
    target._command=commands;target._inspect=commands.inspect;target.snapshot=_native_snapshot
    def inspect(role):return pin.inspect_candidate(batch/(role+'-control'),plan[role+'_case_id'],batch/'initdb.sh')
    if operation.startswith('inspect-'):
        candidate,inspection=inspect(operation.removeprefix('inspect-'))
        _require(admission.source_digest(source,manifest)==source_hash,'CONTROLLED_NATIVE_SOURCE_CHANGED')
        sys.stdout.buffer.write(_canonical(dict(candidate=candidate,inspection=inspection)))
        return 0
    source_candidate,_=inspect('source');target_candidate,_=inspect('target')
    build_root=batch/'probe-live-build';receipt,_=_private_bytes(build_root/'materialized.json',16384,0o400)
    compiler,_=_private_bytes(build_root/'compiler.jsonl',16*1024**2,0o600)
    _require(source_candidate['birth_sha256']!=target_candidate['birth_sha256'] and '0'*64 not in (source_candidate['birth_sha256'],target_candidate['birth_sha256']),'CONTROLLED_NATIVE_COMPILE_BINDING')
    binary,build=_read_materialized(build_root,compiler+b'CURRENT11_MATERIALIZED\t'+receipt+b'\n',source_candidate['birth_sha256'],target_candidate['birth_sha256'],suite/'current11/cache'/plan['cache_scope'])
    env={'PATH':'/usr/bin:/bin','HOME':str(batch),'LC_ALL':'C','TMPDIR':str(batch/'tmp'),'TMP':str(batch/'tmp'),'TEMP':str(batch/'tmp'),
         'KNOWWEAVE_C4_IMPORT_CASE':'success','KNOWWEAVE_C4_IMPORT_SOURCE_ROOT':str(batch/'source-control'/'targets'/plan['source_case_id']),
         'KNOWWEAVE_C4_IMPORT_TARGET_ROOT':str(batch/'target-control'/'targets'/plan['target_case_id']),
         'KNOWWEAVE_C4_IMPORT_ARTIFACT_ROOT':str(batch/'artifacts')}
    process=delegate._run_bounded(runner._import_test_command(binary,'success'),cwd=source,env=env,timeout=360,limit=64*1024)
    _require(_digest(_private_bytes(binary,128*1024**2,0o500)[0])==build['binary_sha256'],'CONTROLLED_NATIVE_BINARY_CHANGED')
    _require(admission.source_digest(source,manifest)==source_hash,'CONTROLLED_NATIVE_SOURCE_CHANGED')
    sys.stdout.buffer.write(process.stdout);sys.stderr.buffer.write(process.stderr)
    return process.returncode

def _mint_from_backend(backend):
    """Called only by the fixed sealed Ops success entry after legacy admission."""
    import p0c4_controlled_import_acceptance as runner
    _require(type(backend) is runner.ImportBackend and getattr(backend,'_controlled_context',None) is None,'CONTROLLED_EXACT_BACKEND')
    _require(sys.platform=='linux' and os.geteuid()==0 and not backend.resources and backend.verify_source(),'CONTROLLED_FRESH_BACKEND')
    match=re.fullmatch(r'(/var/lib/docker/volumes/(kwc4c-suite-[0-9a-f]{32})/_data)/controlled-import/batches/([0-9a-f-]{36})',str(backend.batch))
    _require(match is not None and _v4(match[3]),'CONTROLLED_SUITE_DERIVATION')
    suite=Path(match[1]);value,pin=_read_plan(suite/'current11'/'plan.json')
    record,record_pin=_read_install(suite)
    _,adapter_pin=_private_bytes(suite/'current11/adapter.py',128*1024,0o500)
    _require(adapter_pin['sha256']==record['adapter_sha256'] and all(value[k]==record[k] for k in ('source_commit','public_archive_sha256','public_manifest_sha256','legacy_archive_sha256','legacy_manifest_sha256','builder','postgres','budget_profile')),'CONTROLLED_PLAN_INSTALL_BINDING')
    args=backend.args
    _require(value['batch_id']==match[3]==args.batch_id and value['source_case_id']==args.source_batch_id and value['target_case_id']==args.target_batch_id and value['source_subnet']==args.source_subnet and value['target_subnet']==args.target_subnet and args.case==value['case']=='success' and args.phase=='import','CONTROLLED_BACKEND_PLAN')
    _require(value['source_commit']==args.commit==backend.manifest['commit'] and value['legacy_archive_sha256']==args.archive_sha256 and value['legacy_manifest_sha256']==args.manifest_sha256,'CONTROLLED_LEGACY_PACKAGE')
    _require(args.archive==suite/'incoming'/'source-legacy.zip','CONTROLLED_FIXED_ARCHIVE_PATH')
    checked,_=backend.helper.verify_archive(args.archive,args.archive_sha256,args.manifest_sha256,args.commit)
    _require(checked==backend.manifest,'CONTROLLED_LEGACY_READBACK')
    raw,_=_private_bytes(suite/'incoming'/'source-public.zip',32*1024**2,0o400)
    import p0c4_source_admission_gate_acceptance as admission
    required='scripts/p0c4_source_admission_gate_acceptance.py'
    _require(required in {r['path'] for r in backend.manifest['files']},'CONTROLLED_PUBLIC_HELPER')
    actual=(backend.source/required).read_bytes()
    _require(Path(admission.__file__).read_bytes()==actual,'CONTROLLED_IMPORTED_PUBLIC_HELPER')
    public,files=admission.verify_package(raw,value['public_archive_sha256'],value['public_manifest_sha256'],_digest(actual),'green')
    _require(public['base_commit']==value['source_commit'] and
        [(r['path'],r['bytes'],r['sha256']) for r in public['files']]==[(r['path'],r['size'],r['sha256']) for r in backend.manifest['files']], 'CONTROLLED_DIALECT_MEMBER_EQUALITY')
    modules=((sys.modules[__name__],'scripts/p0c4_completion/controlled_fixture.py'),
        (runner,'scripts/p0c4_controlled_import_acceptance.py'),(backend.helper,'scripts/p0c4_restore_birth_acceptance.py'),
        (backend.provisioner,'scripts/p0c4_restore_target.py'),(backend.issuer,'scripts/p0c4_restore_target_birth.py'),
        (backend.pin,'scripts/p0c4_restore_target_pin.py'),(backend.prepare,'scripts/p0c4_restore_target_pin_prepare.py'),
        (backend.delegate,'scripts/p0c4_restore_pin_acceptance.py'))
    for module,path in modules:
        origin=getattr(getattr(module,'__spec__',None),'origin',None) or module.__file__
        _require(path in files and Path(origin).read_bytes()==files[path],'CONTROLLED_SEALED_MODULE_GRAPH')
    install=_VerifiedInstall(_TOKEN,backend,value,pin,suite,files,record,record_pin,adapter_pin)
    try:
        context=ControlledFixtureContext(_TOKEN,install)
        _require(not os.path.lexists(backend.batch/'context.minted'),'CONTROLLED_CONTEXT_ALREADY_MINTED')
        backend.helper._private_write(backend.batch/'context.minted',_canonical(dict(scope='CONTEXT_MINTED_NOT_AUTHORITY',plan_sha256=pin['sha256'],batch_id=value['batch_id'])))
        backend._controlled_context=context
        return context
    except BaseException:install.close();raise

if __name__=='__main__':
    sys.dont_write_bytecode=True
    choices={'--fixed-inspect-source':'inspect-source','--fixed-inspect-target':'inspect-target','--fixed-body':'body'}
    if sys.argv[1:]==['--fixed-build-materialize']:raise SystemExit(_build_materialize_main())
    if len(sys.argv)!=2 or sys.argv[1] not in choices:raise SystemExit(2)
    try:raise SystemExit(_native_main(choices[sys.argv[1]]))
    except ControlledRejected:raise SystemExit(1)
