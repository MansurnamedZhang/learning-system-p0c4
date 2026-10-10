"""Root fixture issuer, separate from product startup. No caller expected pin.

The root broker supplies already accepted case/package facts and retained root
descriptors. Runtime opens only the independently installed fixed namespace.
"""
from dataclasses import dataclass
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import uuid

REGISTRY_PATH = '/var/lib/knowweave-c4/registry'
DIRECTORIES = ('generations', 'groups', 'protection', 'roots', 'staging')
MAX_SCAN = 536870912
MAX_ENTRIES = 100000
HEX64 = re.compile(r'[0-9a-f]{64}\Z')
HEX40 = re.compile(r'[0-9a-f]{40}\Z')
_ROOT_CONSTRUCTOR = object()

class RegistryError(RuntimeError): pass
def require(ok, token):
    if not ok: raise RegistryError(token)
def canonical(value): return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode('utf-8')
def digest(raw): return hashlib.sha256(raw).hexdigest()
def pairs(items):
    out = {}
    for key, value in items:
        require(key not in out, 'REGISTRY_DUPLICATE_KEY'); out[key] = value
    return out
def parse_record(raw):
    try: value = json.loads(raw, object_pairs_hook=pairs, parse_float=lambda _: (_ for _ in ()).throw(RegistryError('REGISTRY_INTEGER_REQUIRED')),parse_constant=lambda _: (_ for _ in ()).throw(RegistryError('REGISTRY_JSON_CONSTANT')))
    except (ValueError, UnicodeError) as error: raise RegistryError('REGISTRY_JSON') from error
    require(type(value) is dict and canonical(value) == raw, 'REGISTRY_CANONICAL')
    return value
def v4(value):
    try: ident = uuid.UUID(value)
    except (ValueError, TypeError, AttributeError): raise RegistryError('REGISTRY_UUID')
    require(ident.version == 4 and ident.variant == uuid.RFC_4122 and str(ident) == value, 'REGISTRY_UUID')
    return value
def path_text(value):
    require(type(value) is str and value.startswith('/') and value != '/' and '\0' not in value and all(part not in ('', '.', '..') for part in value[1:].split('/')), 'REGISTRY_PATH')
    return value
def integer(value, low=0, high=2**64-1): return type(value) is int and low <= value <= high
def directory(fd, private=True):
    meta = os.fstat(fd)
    require(stat.S_ISDIR(meta.st_mode) and meta.st_uid == 0 and (stat.S_IMODE(meta.st_mode) == 0o700 if private else not meta.st_mode & 0o022), 'REGISTRY_PRIVATE_DIRECTORY')
    return dict(dev=meta.st_dev, ino=meta.st_ino)
def open_directory(path):
    path_text(str(path)); fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        directory(fd, False)
        for part in str(path)[1:].split('/'):
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
            os.close(fd); fd = child; directory(fd, False)
        directory(fd); return fd
    except BaseException: os.close(fd); raise
def child_directory(parent, name):
    require('/' not in name and name not in ('', '.', '..'), 'REGISTRY_ENTRY_NAME')
    fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=parent)
    try: directory(fd); return fd
    except BaseException: os.close(fd); raise
def names(fd, maximum):
    out = []
    # /proc/self/fd refers to this held descriptor, not its substituted pathname.
    with os.scandir('/proc/self/fd/' + str(fd)) as scan:
        for entry in scan:
            require(len(out) < maximum, 'REGISTRY_ENTRY_CAP'); out.append(entry.name)
    return sorted(out)
class Budget:
    def __init__(self): self.bytes = 0; self.entries = 0
    def read(self, size):
        require(integer(size) and self.bytes + size <= MAX_SCAN, 'REGISTRY_SCAN_BYTES'); self.bytes += size
    def listed(self, count):
        require(self.entries + count <= MAX_ENTRIES, 'REGISTRY_ENTRY_CAP'); self.entries += count
def read_file(fd, name, cap, budget):
    file = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=fd)
    try:
        m = os.fstat(file)
        require(stat.S_ISREG(m.st_mode) and m.st_uid == 0 and stat.S_IMODE(m.st_mode) == 0o600 and m.st_nlink == 1 and 0 < m.st_size <= cap, 'REGISTRY_PRIVATE_FILE')
        budget.read(m.st_size)
        with os.fdopen(os.dup(file), 'rb') as stream: raw = stream.read(m.st_size + 1)
        require(len(raw) == m.st_size, 'REGISTRY_READ_CHANGED'); return raw
    finally: os.close(file)
def new_file(fd, name, raw):
    file = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=fd)
    try:
        with os.fdopen(os.dup(file), 'wb') as stream: stream.write(raw); stream.flush(); os.fsync(stream.fileno())
        os.fsync(fd)
    finally: os.close(file)
def rename_no_replace(source_fd, source, target_fd, target):
    libc = ctypes.CDLL(None, use_errno=True)
    call = libc.renameat2; call.argtypes = [ctypes.c_int,ctypes.c_char_p,ctypes.c_int,ctypes.c_char_p,ctypes.c_uint]
    if call(source_fd, os.fsencode(source), target_fd, os.fsencode(target), 1) != 0: raise OSError(ctypes.get_errno(), 'registry publication')
    os.fsync(source_fd); os.fsync(target_fd)
def _crash_seam(_name): pass

@dataclass(frozen=True, init=False)
class FreshProvisioningContext:
    batch_id: str
    case_id: str
    source_package_sha256: str
    application_commit: str
    application_build_sha256: str
    source_binding_bytes: bytes
    source_database_facts: dict
    source_roots: dict
    registry_root_handle: int
    def __init__(self, token, **fields):
        require(token is _ROOT_CONSTRUCTOR, 'ROOT_HELPER_CONSTRUCTOR_ONLY')
        require(set(fields) == set(self.__annotations__), 'REGISTRY_CONTEXT_FIELDS')
        for key, value in fields.items(): object.__setattr__(self, key, value)

def _root_helper_context(*, accepted_case, source_binding_bytes, source_database_facts, source_roots, registry_root_handle):
    """Only called after the existing broker's ownership/package checks.

    Root identities are fstats of handles, never JSON inode assertions.
    """
    require(os.geteuid() == 0, 'REGISTRY_ISSUER_ROOT')
    require(set(accepted_case) == {'batch_id','case_id','source_package_sha256','application_commit','application_build_sha256'}, 'REGISTRY_ACCEPTED_CASE')
    for key in ('batch_id','case_id'): v4(accepted_case[key])
    require(accepted_case['batch_id'] != accepted_case['case_id'] and HEX64.fullmatch(accepted_case['source_package_sha256']) and HEX64.fullmatch(accepted_case['application_build_sha256']) and HEX40.fullmatch(accepted_case['application_commit']), 'REGISTRY_PACKAGE_FACTS')
    require(set(source_database_facts) == {'database','database_oid','system_identifier'}, 'REGISTRY_SQL_FACTS')
    require(re.fullmatch(r'learning_backup_c4_task3_[0-9a-f-]{36}', source_database_facts['database']) and integer(source_database_facts['database_oid'],1,2**32-1) and re.fullmatch(r'[1-9][0-9]*', source_database_facts['system_identifier']) and int(source_database_facts['system_identifier']) < 2**64, 'REGISTRY_SQL_FACTS')
    require(set(source_roots) == {'control','assets','staging','local_pins'}, 'REGISTRY_ROOT_FACTS')
    held = {}
    for key, row in source_roots.items():
        require(type(row) is tuple and len(row) == 2 and type(row[0]) is int, 'REGISTRY_HELD_ROOT')
        fd, path = row; path_text(path); held[key] = directory(fd)
        reopened = open_directory(path)
        try: require(directory(reopened) == held[key], 'REGISTRY_ORIGINAL_ROOT')
        finally: os.close(reopened)
    require(len({(v['dev'],v['ino']) for v in held.values()}) == 4, 'REGISTRY_ROOT_ALIAS')
    binding = parse_record(source_binding_bytes)
    require(set(binding) == {'format_version','capability','binding_id','control_path','control_dev','control_ino','database','database_oid','system_identifier'} and binding['format_version'] == 1 and binding['capability'] == 'source_control_binding_v1', 'REGISTRY_INDEPENDENT_BINDING')
    v4(binding['binding_id'])
    require(binding['control_path'] == source_roots['control'][1] and binding['control_dev'] == held['control']['dev'] and binding['control_ino'] == held['control']['ino'] and all(binding[k] == v for k,v in source_database_facts.items()), 'REGISTRY_BINDING_FACTS')
    directory(registry_root_handle)
    reopened = open_directory(REGISTRY_PATH)
    try: require(directory(reopened) == directory(registry_root_handle), 'REGISTRY_INSTALLED_MOUNT')
    finally: os.close(reopened)
    return FreshProvisioningContext(_ROOT_CONSTRUCTOR, **accepted_case, source_binding_bytes=source_binding_bytes, source_database_facts=dict(source_database_facts), source_roots=dict(source_roots), registry_root_handle=registry_root_handle)

@dataclass(frozen=True)
class RegistryProvisioningReceipt:
    record: dict
    def __post_init__(self): validate_provisioning_receipt(self.record)
    def canonical_bytes(self): return canonical(self.record)
@dataclass(frozen=True)
class RegistryObservation:
    authority: dict
    generation: dict
    roots: dict
    groups: dict
    hashes: dict
    scan_bytes: int

def validate_provisioning_receipt(row):
    keys={'format_version','capability','batch_id','case_id','source_package_sha256','application_commit','application_build_sha256','source_binding_sha256','deployment_id','source_group_id','authority_sha256','generation','generation_sha256','registry_dev','registry_ino','roots','issuer_euid','state'}
    require(type(row) is dict and set(row)==keys and type(row['format_version']) is int and row['format_version']==1 and row['capability']=='independent_registry_provisioning_v1' and type(row['generation']) is int and row['generation']==1 and type(row['issuer_euid']) is int and row['issuer_euid']==0 and row['state']=='READBACK_DURABLE','REGISTRY_RECEIPT_SCHEMA')
    for key in ('batch_id','case_id','deployment_id','source_group_id'):v4(row[key])
    for key in ('source_package_sha256','application_build_sha256','source_binding_sha256','authority_sha256','generation_sha256'):
        require(type(row[key]) is str and HEX64.fullmatch(row[key]),'REGISTRY_RECEIPT_DIGEST')
    require(HEX40.fullmatch(row['application_commit']) and integer(row['registry_dev'],1) and integer(row['registry_ino'],1),'REGISTRY_RECEIPT_IDENTITY')
    roots=row['roots'];require(type(roots) is list and len(roots)==4 and [r['id'] for r in roots]==sorted(set(r['id'] for r in roots)),'REGISTRY_RECEIPT_ROSTER')
    require({r['kind'] for r in roots}=={'source_assets','source_staging','source_control','local_pins'},'REGISTRY_RECEIPT_KINDS')
    paths=set();identities=set()
    for r in roots:
        require(set(r)=={'id','kind','path','dev','ino','record_sha256'} and integer(r['dev'],1) and integer(r['ino'],1) and HEX64.fullmatch(r['record_sha256']),'REGISTRY_RECEIPT_ROOT')
        v4(r['id']);path_text(r['path']);require(r['path'] not in paths and (r['dev'],r['ino']) not in identities,'REGISTRY_RECEIPT_ALIAS');paths.add(r['path']);identities.add((r['dev'],r['ino']))
    require(len(canonical(row))<=16384,'REGISTRY_RECEIPT_CAP');return row

def provision_registry(context: FreshProvisioningContext) -> RegistryProvisioningReceipt:
    require(type(context) is FreshProvisioningContext and os.geteuid() == 0, 'REGISTRY_ROOT_CONTEXT')
    root = context.registry_root_handle; root_identity = directory(root)
    require(names(root, 1) == [], 'REGISTRY_ALREADY_INITIALIZED')
    deployment = str(uuid.uuid4()); group_id = str(uuid.uuid4()); dirs = {}; fds = {}
    try:
        for name in DIRECTORIES:
            os.mkdir(name, 0o700, dir_fd=root); fds[name] = child_directory(root,name); dirs[name] = directory(fds[name]); os.fsync(root)
        lock = os.open('registry.lock', os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=root)
        try: lm = os.fstat(lock); os.fsync(lock); os.fsync(root)
        finally: os.close(lock)
        kinds = {'assets':'source_assets','control':'source_control','local_pins':'local_pins','staging':'source_staging'}
        roots = []; root_map = {}; receipt_roots = []
        for key in sorted(context.source_roots):
            fd,path = context.source_roots[key]; identity = directory(fd); ident = str(uuid.uuid4()); root_map[key] = ident
            record = dict(format_version=1,capability='backup_root_v1',deployment_id=deployment,enrollment_id=ident,group_id=group_id,kind=kinds[key],path=path,**identity,uid=0,mode=448,enrolled_generation=1)
            raw = canonical(record); new_file(fds['roots'],ident+'.json',raw)
            roots.append(dict(id=ident,sha256=digest(raw))); receipt_roots.append(dict(id=ident,kind=kinds[key],path=path,**identity,record_sha256=digest(raw)))
        group = dict(format_version=1,capability='backup_source_group_v1',deployment_id=deployment,group_id=group_id,**context.source_database_facts,source_binding_sha256=digest(context.source_binding_bytes),application_commit=context.application_commit,application_build_sha256=context.application_build_sha256,roots=root_map)
        raw = canonical(group); new_file(fds['groups'],group_id+'.json',raw)
        os.mkdir(group_id,0o700,dir_fd=fds['protection']); os.fsync(fds['protection'])
        generation = dict(format_version=1,capability='backup_registry_generation_v1',deployment_id=deployment,generation=1,previous_generation_sha256=None,roots=sorted(roots,key=lambda r:r['id']),groups=[dict(id=group_id,sha256=digest(raw))])
        generation_raw = canonical(generation); _crash_seam('before_generation_commit'); new_file(fds['generations'],'00000000000000000001.json',generation_raw); _crash_seam('after_generation_commit')
        authority = dict(format_version=1,capability='backup_registry_v1',deployment_id=deployment,registry_path=REGISTRY_PATH,registry_dev=root_identity['dev'],registry_ino=root_identity['ino'],lock_dev=lm.st_dev,lock_ino=lm.st_ino,directories=dirs,initial_generation=1,initial_generation_sha256=digest(generation_raw))
        tx = str(uuid.uuid4()); os.mkdir(tx,0o700,dir_fd=fds['staging']); temporary = child_directory(fds['staging'],tx)
        try:
            authority_raw = canonical(authority); new_file(temporary,'authority.json',authority_raw); os.fsync(temporary); os.fsync(fds['staging']); _crash_seam('before_initial_authority')
            rename_no_replace(temporary,'authority.json',root,'authority.json'); _crash_seam('after_initial_authority')
            os.rmdir(tx,dir_fd=fds['staging']); os.fsync(fds['staging'])
        finally: os.close(temporary)
        observation = validate_installed_registry()
        require(observation.authority == authority and observation.generation == generation and len(observation.roots) == 4, 'REGISTRY_DURABLE_READBACK')
        receipt = dict(format_version=1,capability='independent_registry_provisioning_v1',batch_id=context.batch_id,case_id=context.case_id,source_package_sha256=context.source_package_sha256,application_commit=context.application_commit,application_build_sha256=context.application_build_sha256,source_binding_sha256=digest(context.source_binding_bytes),deployment_id=deployment,source_group_id=group_id,authority_sha256=digest(authority_raw),generation=1,generation_sha256=digest(generation_raw),registry_dev=root_identity['dev'],registry_ino=root_identity['ino'],roots=sorted(receipt_roots,key=lambda r:r['id']),issuer_euid=0,state='READBACK_DURABLE')
        require(len(canonical(receipt)) <= 16384, 'REGISTRY_RECEIPT_CAP'); return RegistryProvisioningReceipt(receipt)
    finally:
        for fd in fds.values(): os.close(fd)

def validate_installed_registry() -> RegistryObservation:
    root = open_directory(REGISTRY_PATH)
    try: return _validate_held(root)
    finally: os.close(root)


# Acceptance diagnostics only. These functions never produce a RegistryObservation,
# enroll storage, clear staging, or grant product publication/release authority.
FAULT_AUDIT_TESTS = ('protection_tests::real_registry_pending_staged_fault',
                     'protection_tests::real_registry_catalog_fault')
FAULT_BASELINE_NAME = 'registry-fault-baseline.json'


def _fault_read(fd,name,cap,budget,mode=0o600,empty=False):
    file=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC,dir_fd=fd)
    try:
        before=os.fstat(file)
        require(stat.S_ISREG(before.st_mode) and before.st_uid==0 and stat.S_IMODE(before.st_mode)==mode and before.st_nlink==1 and (0 if empty else 1)<=before.st_size<=cap,'REGISTRY_FAULT_PRIVATE_FILE')
        budget.read(before.st_size)
        with os.fdopen(os.dup(file),'rb') as stream:raw=stream.read(before.st_size+1)
        after=os.fstat(file)
        fields=('st_dev','st_ino','st_uid','st_mode','st_nlink','st_size','st_mtime_ns','st_ctime_ns')
        require(len(raw)==before.st_size and all(getattr(before,k)==getattr(after,k) for k in fields),'REGISTRY_FAULT_READ_CHANGED')
        return raw,dict(dev=before.st_dev,ino=before.st_ino,uid=before.st_uid,mode=stat.S_IMODE(before.st_mode),links=before.st_nlink,size=len(raw),sha256=digest(raw))
    finally:os.close(file)


def _fault_names(fd,cap,budget):
    entries=names(fd,cap);budget.listed(len(entries));return entries


def _fault_immutable(root,receipt,budget):
    require(_fault_names(root,7,budget)==['authority.json','generations','groups','protection','registry.lock','roots','staging'],'REGISTRY_FAULT_LAYOUT')
    files={};directories={'registry':directory(root)};raws={}
    for name,cap in (('authority.json',4096),('registry.lock',0)):
        raws[name],files[name]=_fault_read(root,name,cap,budget,empty=name=='registry.lock')
    expected={'generations':['00000000000000000001.json'],'groups':[receipt['source_group_id']+'.json'],'roots':[r['id']+'.json' for r in receipt['roots']]}
    for name in DIRECTORIES:
        fd=child_directory(root,name)
        try:
            directories[name]=directory(fd)
            if name in expected:
                require(_fault_names(fd,len(expected[name]),budget)==expected[name],'REGISTRY_FAULT_ROSTER')
                for leaf in expected[name]:raws[name+'/'+leaf],files[name+'/'+leaf]=_fault_read(fd,leaf,524288 if name=='generations' else 16384,budget)
            elif name=='protection':
                require(_fault_names(fd,1,budget)==[receipt['source_group_id']],'REGISTRY_FAULT_GROUP')
                group=child_directory(fd,receipt['source_group_id'])
                try:directories['protection/'+receipt['source_group_id']]=directory(group)
                finally:os.close(group)
        finally:os.close(fd)
    for row in receipt['roots']:
        fd=open_directory(row['path'])
        try:
            directories['source/'+row['kind']]=directory(fd)
            require(directory(fd)==dict(dev=row['dev'],ino=row['ino']),'REGISTRY_FAULT_SOURCE_ROOT')
        finally:os.close(fd)
    require(files['authority.json']['sha256']==receipt['authority_sha256'] and files['generations/00000000000000000001.json']['sha256']==receipt['generation_sha256'] and directories['registry']==dict(dev=receipt['registry_dev'],ino=receipt['registry_ino']),'REGISTRY_FAULT_ISSUER')
    require(all(files['roots/'+r['id']+'.json']['sha256']==r['record_sha256'] for r in receipt['roots']),'REGISTRY_FAULT_ROOT_HASH')
    return dict(files=files,directories=directories),parse_record(raws['groups/'+receipt['source_group_id']+'.json'])


def capture_fault_audit_baseline(receipt,test):
    require(test in FAULT_AUDIT_TESTS,'REGISTRY_FAULT_TEST');validate_provisioning_receipt(receipt)
    observed=validate_installed_registry()
    require(observed.generation['generation']==1 and len(observed.groups)==1 and len(observed.roots)==4,'REGISTRY_FAULT_FRESH_BASELINE')
    budget=Budget();budget.read(observed.scan_bytes);root=open_directory(REGISTRY_PATH)
    try:
        snapshot,group=_fault_immutable(root,receipt,budget)
        require(group==observed.groups[receipt['source_group_id']] and all(snapshot['files'][name]['sha256']==sha for name,sha in observed.hashes.items()),'REGISTRY_FAULT_BASELINE_CHANGED')
        return dict(format_version=1,capability='registry_fault_audit_baseline_v1',classification='DIAGNOSTIC_NOT_AUTHORITY',test=test,issuer=receipt,immutable=snapshot)
    finally:os.close(root)


def _fault_index(raw,digests):
    # Legacy AssetIndexV1 is declared-field order, not sorted registry JSON.
    try:value=json.loads(raw,object_pairs_hook=pairs,parse_float=lambda _:require(False,'REGISTRY_FAULT_INDEX_INTEGER'),parse_constant=lambda _:require(False,'REGISTRY_FAULT_INDEX_INTEGER'))
    except (ValueError,UnicodeError) as error:raise RegistryError('REGISTRY_FAULT_INDEX_JSON') from error
    require(type(value) is dict and set(value)=={'format_version','assets'} and type(value['format_version']) is int and value['format_version']==1 and type(value['assets']) is list and len(value['assets'])<=100000,'REGISTRY_FAULT_INDEX')
    rows=[];seen=set();last=None
    for row in value['assets']:
        require(type(row) is dict and set(row)=={'space_id','id','sha256','byte_size','storage_key'},'REGISTRY_FAULT_INDEX_ROW')
        # Existing asset UUIDs need canonical syntax, not enrollment UUIDv4.
        for key in ('space_id','id'):require(str(uuid.UUID(row[key]))==row[key],'REGISTRY_FAULT_ASSET_UUID')
        key=(row['space_id'],row['id']);sha=row['sha256']
        require(key not in seen and (last is None or last<key) and type(sha) is str and HEX64.fullmatch(sha) and integer(row['byte_size'],0,2**63-1) and row['storage_key']=='sha256/'+sha[:2]+'/'+sha,'REGISTRY_FAULT_INDEX_ROW')
        require(sha not in digests or digests[sha]==row['byte_size'],'REGISTRY_FAULT_INDEX_DIGEST')
        seen.add(key);last=key;digests[sha]=row['byte_size'];rows.append({k:row[k] for k in ('space_id','id','sha256','byte_size','storage_key')})
        require(len(digests)<=MAX_ENTRIES,'REGISTRY_FAULT_INDEX_UNIQUE_CAP')
    require(json.dumps(dict(format_version=1,assets=rows),ensure_ascii=False,separators=(',',':')).encode()==raw,'REGISTRY_FAULT_INDEX_CANONICAL')
    return len(rows)


def audit_incomplete_publication(baseline):
    require(type(baseline) is dict and set(baseline)=={'format_version','capability','classification','test','issuer','immutable'} and type(baseline['format_version']) is int and baseline['format_version']==1 and baseline['capability']=='registry_fault_audit_baseline_v1' and baseline['classification']=='DIAGNOSTIC_NOT_AUTHORITY' and baseline['test'] in FAULT_AUDIT_TESTS,'REGISTRY_FAULT_BASELINE')
    receipt=validate_provisioning_receipt(baseline['issuer']);budget=Budget();root=open_directory(REGISTRY_PATH)
    held=[];files={};directories={};asset_digests={}
    def child(fd,name,path):
        sub=child_directory(fd,name);held.append(sub);directories[path]=directory(sub);return sub
    def read(fd,name,path,cap=16384,mode=0o600):
        raw,files[path]=_fault_read(fd,name,cap,budget,mode);return raw
    def pending(raw,backup):
        expected=dict(format_version=1,capability='source_protection_v1',deployment_id=receipt['deployment_id'],group_id=receipt['source_group_id'],backup_id=v4(backup),registry_generation=1,registry_generation_sha256=receipt['generation_sha256'],source_binding_sha256=receipt['source_binding_sha256'],application_commit=receipt['application_commit'],application_build_sha256=receipt['application_build_sha256'],state='capture_pending',previous_sha256=None,roots=group['roots'])
        require(raw==canonical(expected),'REGISTRY_FAULT_PENDING');return expected
    def catalog(raw,prior,index):
        row=dict(parse_record(prior),state='catalog_durable',previous_sha256=digest(prior),asset_index=dict(path='asset-index.json',size=len(index),sha256=digest(index)),logical_asset_count=_fault_index(index,asset_digests));row.pop('roots')
        require(raw==canonical(row),'REGISTRY_FAULT_CATALOG')
    try:
        snapshot,group=_fault_immutable(root,receipt,budget)
        require(canonical(snapshot)==canonical(baseline['immutable']),'REGISTRY_FAULT_IMMUTABLE_CHANGED')
        # Charge the strict validator's pre-staging authority read and listing
        # to this same operation budget; it must still fail at the fixed seam.
        budget.read(snapshot['files']['authority.json']['size']);budget.listed(7)
        try:_validate_held(root)
        except RegistryError as error:require(str(error)=='REGISTRY_INCOMPLETE_PUBLICATION','REGISTRY_FAULT_WRONG_REFUSAL')
        else:raise RegistryError('REGISTRY_FAULT_EXPECTED_REFUSAL')
        staging=child(root,'staging','staging');tokens=_fault_names(staging,1,budget);require(len(tokens)==1,'REGISTRY_FAULT_STAGING');tx=v4(tokens[0]);staged=child(staging,tx,'staging/'+tx)
        is_pending=baseline['test']==FAULT_AUDIT_TESTS[0];leaf='00-pending.json' if is_pending else '10-catalog.json'
        require(_fault_names(staged,1,budget)==[leaf],'REGISTRY_FAULT_STAGING');staged_raw=read(staged,leaf,'staging/'+tx+'/'+leaf);staged_row=parse_record(staged_raw);backup=v4(staged_row.get('backup_id'))
        protection=child(root,'protection','protection');groupfd=child(protection,receipt['source_group_id'],'protection/'+receipt['source_group_id']);attempts=_fault_names(groupfd,2,budget)
        retained=[ident for ident in attempts if ident!=backup]
        require(len(retained)==1 and attempts==sorted(retained+([] if is_pending else [backup])),'REGISTRY_FAULT_RETAINED')
        if is_pending:pending(staged_raw,backup)
        for ident in attempts:
            v4(ident);prefix='protection/'+receipt['source_group_id']+'/'+ident;fd=child(groupfd,ident,prefix)
            expected=['00-pending.json','asset-index.json'] if ident==backup else ['00-pending.json','10-catalog.json','20-retained.json','asset-index.json']
            require(_fault_names(fd,4,budget)==expected,'REGISTRY_FAULT_ATTEMPT_LAYOUT')
            p=read(fd,'00-pending.json',prefix+'/00-pending.json');pending(p,ident)
            index=read(fd,'asset-index.json',prefix+'/asset-index.json',64*1024**2)
            c=staged_raw if ident==backup else read(fd,'10-catalog.json',prefix+'/10-catalog.json');catalog(c,p,index)
            if ident!=backup:
                raw=read(fd,'20-retained.json',prefix+'/20-retained.json');row=parse_record(raw)
                pins=next(r for r in receipt['roots'] if r['kind']=='local_pins');pinroot=open_directory(pins['path']);held.append(pinroot)
                sealed=os.open(ident+'.sealed',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=pinroot);held.append(sealed);meta=os.fstat(sealed)
                require(stat.S_ISDIR(meta.st_mode) and meta.st_uid==0 and stat.S_IMODE(meta.st_mode)==0o500,'REGISTRY_FAULT_SEALED_DIRECTORY')
                sealed_identity=dict(dev=meta.st_dev,ino=meta.st_ino);directories['sealed/'+ident]=sealed_identity
                manifest=read(sealed,'manifest.json','sealed/'+ident+'/manifest.json',64*1024**2,0o400)
                expected=dict(parse_record(p),state='retained',previous_sha256=digest(c),asset_index_sha256=digest(index),manifest_sha256=digest(manifest),sealed_name=ident+'.sealed',sealed_dev=sealed_identity['dev'],sealed_ino=sealed_identity['ino']);expected.pop('roots')
                require(raw==canonical(expected),'REGISTRY_FAULT_RETAINED')
        control=next(r for r in receipt['roots'] if r['kind']=='source_control');controlfd=open_directory(control['path']);held.append(controlfd)
        binding=read(controlfd,'source-binding.json','source/source-binding.json',4096)
        require(digest(binding)==receipt['source_binding_sha256'],'REGISTRY_FAULT_BINDING')
        control_names=_fault_names(controlfd,4096,budget)
        pinfd=open_directory(next(r['path'] for r in receipt['roots'] if r['kind']=='local_pins'));held.append(pinfd)
        require(not any(name.startswith(backup+'.') for name in _fault_names(pinfd,4096,budget)),'REGISTRY_FAULT_NEW_PIN')
        if is_pending:require(not any(name.startswith(backup+'.') for name in control_names),'REGISTRY_FAULT_NEW_CONTROL')
        phases=('intent','closed','drained','dump_and_index_durable','pins_durable','release_ready','released')
        for ident in retained+([] if is_pending else [backup]):
            prefix='source/'+ident+'.control';fd=child(controlfd,ident+'.control',prefix);sequence=phases[:3] if ident==backup else phases
            require(_fault_names(fd,7,budget)==sorted(phase.replace('_','-')+'.json' for phase in sequence),'REGISTRY_FAULT_JOURNAL')
            previous=None
            for n,phase in enumerate(sequence):
                leaf=phase.replace('_','-')+'.json';raw=read(fd,leaf,prefix+'/'+leaf,4096)
                row=json.loads(raw,object_pairs_hook=pairs)
                keys=('backup_id','phase','dump_and_index_sha256','pins_sha256')
                require(type(row) is dict and set(row)==set(keys) and row['backup_id']==ident and row['phase']==phase,'REGISTRY_FAULT_JOURNAL')
                for key,needed in (('dump_and_index_sha256',n>=3),('pins_sha256',n>=4)):
                    require((type(row[key]) is str and HEX64.fullmatch(row[key])) if needed else row[key] is None,'REGISTRY_FAULT_JOURNAL')
                    if previous and previous[key] is not None:require(row[key]==previous[key],'REGISTRY_FAULT_JOURNAL')
                require(json.dumps({k:row[k] for k in keys},ensure_ascii=False,separators=(',',':')).encode()==raw,'REGISTRY_FAULT_JOURNAL');previous=row
        return dict(format_version=1,capability='registry_expected_unusable_v1',classification='DIAGNOSTIC_NOT_AUTHORITY',state='EXPECTED_INCOMPLETE_PUBLICATION',reason='REGISTRY_INCOMPLETE_PUBLICATION',test=baseline['test'],case_id=receipt['case_id'],issuer_sha256=digest(canonical(receipt)),baseline_sha256=digest(canonical(baseline)),backup_id=backup,files=files,directories=directories,scan_bytes=budget.bytes)
    finally:
        for fd in reversed(held):os.close(fd)
        os.close(root)


def validate_fault_audit_observation(value,receipt,test):
    # This validates evidence links only; it never opens or authorizes storage.
    parse_record(canonical(value))
    keys={'format_version','capability','classification','state','reason','test','case_id','issuer_sha256','baseline_sha256','backup_id','files','directories','scan_bytes'}
    require(type(value) is dict and set(value)==keys and type(value['format_version']) is int and value['format_version']==1 and value['capability']=='registry_expected_unusable_v1' and value['classification']=='DIAGNOSTIC_NOT_AUTHORITY' and value['state']=='EXPECTED_INCOMPLETE_PUBLICATION' and value['reason']=='REGISTRY_INCOMPLETE_PUBLICATION','REGISTRY_FAULT_OBSERVATION')
    require(test in FAULT_AUDIT_TESTS and value['test']==test and value['case_id']==receipt['case_id'] and value['issuer_sha256']==digest(canonical(receipt)) and type(value['baseline_sha256']) is str and HEX64.fullmatch(value['baseline_sha256']),'REGISTRY_FAULT_OBSERVATION_LINK')
    v4(value['backup_id']);require(integer(value['scan_bytes'],1,MAX_SCAN),'REGISTRY_FAULT_OBSERVATION_BUDGET')
    require(type(value['files']) is dict and 1<=len(value['files'])<=64 and type(value['directories']) is dict and 1<=len(value['directories'])<=16,'REGISTRY_FAULT_OBSERVATION_ENTRIES')
    for path,row in value['files'].items():
        require(type(path) is str and all(p not in ('','.','..') for p in path.split('/')) and type(row) is dict and set(row)=={'dev','ino','uid','mode','links','size','sha256'} and integer(row['dev'],1) and integer(row['ino'],1) and type(row['uid']) is int and row['uid']==0 and type(row['mode']) is int and row['mode'] in (0o400,0o600) and type(row['links']) is int and row['links']==1 and integer(row['size'],1,64*1024**2) and type(row['sha256']) is str and HEX64.fullmatch(row['sha256']),'REGISTRY_FAULT_OBSERVATION_FILE')
    require(sum(row['size'] for row in value['files'].values())<=value['scan_bytes'],'REGISTRY_FAULT_OBSERVATION_BUDGET')
    for path,row in value['directories'].items():
        require(type(path) is str and all(p not in ('','.','..') for p in path.split('/')) and type(row) is dict and set(row)=={'dev','ino'} and integer(row['dev'],1) and integer(row['ino'],1),'REGISTRY_FAULT_OBSERVATION_DIRECTORY')
    return value

def _validate_held(root):
    budget = Budget(); layout = names(root,7); budget.listed(len(layout))
    require(layout == ['authority.json','generations','groups','protection','registry.lock','roots','staging'], 'REGISTRY_LAYOUT')
    authority_raw = read_file(root,'authority.json',4096,budget); a = parse_record(authority_raw)
    require(set(a) == {'format_version','capability','deployment_id','registry_path','registry_dev','registry_ino','lock_dev','lock_ino','directories','initial_generation','initial_generation_sha256'} and type(a['format_version']) is int and a['format_version'] == 1 and a['capability'] == 'backup_registry_v1' and a['registry_path'] == REGISTRY_PATH and type(a['initial_generation']) is int and a['initial_generation'] == 1 and HEX64.fullmatch(a['initial_generation_sha256']), 'REGISTRY_AUTHORITY')
    v4(a['deployment_id']); require(directory(root) == dict(dev=a['registry_dev'],ino=a['registry_ino']), 'REGISTRY_ORIGINAL_ROOT')
    lock = os.open('registry.lock',os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,dir_fd=root)
    try:
        lm = os.fstat(lock); require(stat.S_ISREG(lm.st_mode) and lm.st_uid == 0 and stat.S_IMODE(lm.st_mode) == 0o600 and lm.st_nlink == 1 and lm.st_size == 0 and (lm.st_dev,lm.st_ino) == (a['lock_dev'],a['lock_ino']), 'REGISTRY_LOCK_IDENTITY')
    finally: os.close(lock)
    require(set(a['directories']) == set(DIRECTORIES), 'REGISTRY_DIRECTORIES'); fds = {}
    try:
        for name in DIRECTORIES:
            fds[name] = child_directory(root,name); require(directory(fds[name]) == a['directories'][name] and set(a['directories'][name]) == {'dev','ino'}, 'REGISTRY_METADATA_IDENTITY')
        require(names(fds['staging'],1) == [], 'REGISTRY_INCOMPLETE_PUBLICATION')
        roots = {}; groups = {}; hashes = {'authority.json':digest(authority_raw)}
        for kind,cap,collection in (('roots',1024,roots),('groups',256,groups)):
            entries = names(fds[kind],cap); budget.listed(len(entries))
            for name in entries:
                require(name.endswith('.json'), 'REGISTRY_RECORD_NAME'); ident = v4(name[:-5]); raw = read_file(fds[kind],name,16384,budget); row = parse_record(raw)
                require(row.get('deployment_id') == a['deployment_id'] and row.get('format_version') == 1 and type(row['format_version']) is int, 'REGISTRY_RECORD_DEPLOYMENT')
                collection[ident] = row; hashes[kind+'/'+name] = digest(raw)
        entries = names(fds['generations'],128); budget.listed(len(entries)); require(entries, 'REGISTRY_NO_GENERATION')
        previous = None; previous_raw = None
        for n,name in enumerate(entries,1):
            require(name == f'{n:020}.json','REGISTRY_GENERATION_GAP'); raw = read_file(fds['generations'],name,524288,budget); g = parse_record(raw)
            require(set(g) == {'format_version','capability','deployment_id','generation','previous_generation_sha256','roots','groups'} and type(g['generation']) is int and g['generation'] == n and g['format_version'] == 1 and type(g['format_version']) is int and g['capability'] == 'backup_registry_generation_v1' and g['deployment_id'] == a['deployment_id'] and g['previous_generation_sha256'] == (digest(previous_raw) if previous else None), 'REGISTRY_GENERATION_CHAIN')
            for kind,cap in (('roots',1024),('groups',256)):
                roster = g[kind]; require(type(roster) is list and len(roster)<=cap and roster == sorted(roster,key=lambda r:r['id']) and len({r['id'] for r in roster}) == len(roster), 'REGISTRY_SORTED_ROSTER')
                for r in roster:
                    require(set(r) == {'id','sha256'} and HEX64.fullmatch(r['sha256']), 'REGISTRY_ROSTER_REFERENCE'); v4(r['id'])
                if previous: require(all(r in roster for r in previous[kind]), 'REGISTRY_ROLLBACK')
            if n == 1: require(digest(raw) == a['initial_generation_sha256'], 'REGISTRY_INITIAL_COMMIT')
            previous,previous_raw = g,raw; hashes['generations/'+name] = digest(raw)
        for kind,collection in (('roots',roots),('groups',groups)):
            require(previous[kind] == [dict(id=i,sha256=hashes[kind+'/'+i+'.json']) for i in sorted(collection)], 'REGISTRY_LOOSE_OR_MISSING')
        paths=set(); identities=set(); used=set()
        for ident,r in roots.items():
            require(set(r) == {'format_version','capability','deployment_id','enrollment_id','group_id','kind','path','dev','ino','uid','mode','enrolled_generation'} and r['capability']=='backup_root_v1' and r['enrollment_id']==ident and type(r['uid']) is int and r['uid']==0 and type(r['mode']) is int and r['mode']==448 and integer(r['dev'],1) and integer(r['ino'],1) and integer(r['enrolled_generation'],1,previous['generation']), 'REGISTRY_ROOT_RECORD')
            v4(r['group_id']); path_text(r['path']); require(r['path'] not in paths and (r['dev'],r['ino']) not in identities,'REGISTRY_ROOT_ALIAS'); paths.add(r['path']); identities.add((r['dev'],r['ino']))
        for ident,g in groups.items():
            if g.get('capability') == 'backup_source_group_v1':
                require(set(g) == {'format_version','capability','deployment_id','group_id','database','database_oid','system_identifier','source_binding_sha256','application_commit','application_build_sha256','roots'} and re.fullmatch(r'learning_backup_c4_task3_[0-9a-f-]{36}',g['database']) and integer(g['database_oid'],1,2**32-1) and re.fullmatch('[1-9][0-9]*',g['system_identifier']) and int(g['system_identifier'])<2**64 and HEX40.fullmatch(g['application_commit']) and HEX64.fullmatch(g['application_build_sha256']) and HEX64.fullmatch(g['source_binding_sha256']), 'REGISTRY_SOURCE_GROUP')
                kinds={'assets':'source_assets','staging':'source_staging','control':'source_control','local_pins':'local_pins'}
            else:
                require(set(g)=={'format_version','capability','deployment_id','group_id','destination_host_id','destination_storage_id','roots'} and g['capability']=='backup_destination_group_v1' and type(g['destination_host_id']) is str and g['destination_host_id'] and type(g['destination_storage_id']) is str and g['destination_storage_id'],'REGISTRY_DESTINATION_GROUP')
                kinds={'destination':'backup_destination','evidence':'destination_evidence'}
            require(g['group_id']==ident and set(g['roots'])==set(kinds),'REGISTRY_GROUP_ROOTS')
            for key,kind in kinds.items():
                root_id=g['roots'][key]; require(root_id in roots and root_id not in used and roots[root_id]['group_id']==ident and roots[root_id]['kind']==kind,'REGISTRY_GROUP_ROOT_LINK'); used.add(root_id)
        require(used==set(roots),'REGISTRY_ORPHAN_ROOT')
        require(names(fds['protection'],256)==sorted(i for i,g in groups.items() if g['capability']=='backup_source_group_v1'),'REGISTRY_PROTECTION_GROUP_ROSTER')
        # Only metadata is globally observed. Relevant physical roots are opened
        # by the selected product admission, never every historical root here.
        return RegistryObservation(a,previous,roots,groups,hashes,budget.bytes)
    finally:
        for fd in fds.values(): os.close(fd)
