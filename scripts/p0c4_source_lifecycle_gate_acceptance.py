"""Fresh ordinary-hans Linux/PG18 lifecycle gates; no replay, restore or production claim.

Stage this exact runner and the three pinned public helpers beside the reviewed ZIP
in /home/hans/knowweave-c4-source-lifecycle/<new-v4-uuid> (0700). See --help.
Docker authority stays in this ordinary-user process. The network-none root broker
only operates on this batch's new named volume and receives observed facts on stdin.
"""
import argparse
import copy
import functools
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import secrets
import signal
import stat
import subprocess
import sys
import threading
import time
import types
import uuid

BASE = Path('/home/hans/knowweave-c4-source-lifecycle')
BASE_COMMIT = 'e43bb2cef380adda3bf9486005e58a175237d247'
ENTRY = 'scripts/p0c4_source_lifecycle_gate_acceptance.py'
TEST_ENTRY = 'scripts/test_p0c4_source_lifecycle_gate_acceptance.py'
ADMISSION_ENTRY = 'scripts/p0c4_source_admission_gate_acceptance.py'
BINDING_ENTRY = 'scripts/p0c4_source_binding_gate_acceptance.py'
ISOLATION_ENTRY = 'scripts/p0c4_source_isolation.py'
PUBLIC_PINS = {
    'scripts/p0c4_source_admission_gate_acceptance.py': 'e407684f101d467cc0d7271ff678489a2cc894fa97f507e9d8ebada06f8a73ba',
    'scripts/p0c4_source_binding_gate_acceptance.py': '48b0c1713fceb662b62b8abc9994a40c1ae12be4613176678d3426ae05a8d50b',
    'scripts/p0c4_source_isolation.py': '38395d4037eacad39c593e0b6c879fded115ffd37d103babd7bfe98c8a5d2904',
}
PRODUCT_PINS = {
    'crates/learning-assets/src/backup_fs.rs': ('b53e2b7ecd169d6f0d76f98b77f69155b5bea55fc36f321f9ab9e71a6518ac74', 11002),
    'crates/learning-assets/src/secure_dir.rs': ('00c616cb1915c28935a1909027dad274ee72893d13b7d1cb99f185d1da7b1bba', 20378),
    'crates/learning-assets/src/fs.rs': ('c04199a2169ba65f29093ea3a223dfb403b819fdc74b26f4441ed9e9b1ca9723', 33994),
    'crates/learning-backup/src/lib.rs': ('46e4d229c17de8e18a3d530797189c90e468278da7649be0cbd0cee1d3850886', 14296),
    'crates/learning-backup/src/maintenance.rs': ('e0f5b25a113f2cd3bf104093160cd77c80996345033fad94e8e1335f528347d5', 18412),
    'crates/learning-backup/src/sealed.rs': ('37d8a0b379ab02e8a56c415647abb5e9168181fc882a200ca335d682534a84d8', 24632),
    'crates/learning-backup/src/source.rs': ('8e182734a8a4301b9d0124845df1d45ab7b074b71e1dbc25bbaf59b7008a450b', 57879),
    'crates/learning-backup/src/source/lifecycle.rs': ('0f02c523b5aa1f9c6e40a7ae3bab4f3c3595bd94846dc0fbea40c640212eb70d', 27432),
    'crates/learning-backup/src/source/lifecycle_tests.rs': ('ab333c9a2c097683e3c30963d3212247abcede5444f5bb6b68ab3215ba490043', 65016),
    'crates/learning-backup/src/registry.rs': ('504c28f87f3d6c29cc04fd22a84adb1da0f6d18b8caf5ae90036774630c1be88', 45366),
    'crates/learning-backup/src/protection.rs': ('6580acf33ca1574049bc8b541faa27c414d1f885f5884c8db8da5e5848ade832', 49676),
    'scripts/p0c4_storage_registry.py': ('371ba6089a99bb7cb8c45ddcb158c43f8c801579e480ed2877765ff385a2d179', 39198),
}
MANIFEST = '_knowweave_source_manifest.json'
ROOT = '/var/lib/knowweave-source'
PIN_ENV = 'KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256'
DOCKER = '/usr/bin/docker'
HOST_ENV = {'PATH':'/usr/bin:/bin','HOME':'/home/hans','LC_ALL':'C','DOCKER_HOST':'unix:///var/run/docker.sock'}
HEX64 = re.compile(r'[0-9a-f]{64}\Z')
REGISTRY_TESTS = tuple('protection_tests::real_registry_'+name for name in ('reopen','root_replacement','corrupt_stale','concurrent_capture','protection_release','pending_staged_fault','pending_visible_fault','catalog_fault','retained_release_fault','abandon_ready_fault','abandon_terminal_fault'))
REGISTRY_INCOMPLETE_TESTS = ('protection_tests::real_registry_pending_staged_fault','protection_tests::real_registry_catalog_fault')
SOURCE_CLONE_TEST='source::endpoint_tests::source_same_id_physical_clone_rejected_before_journal_or_acl'
SOURCE_BOUND_DUMP_TEST='source::endpoint_tests::source_dump_uses_exact_container_socket_and_admitted_backend'
FULL_RETAINED_ORIGINAL_TESTS=(
    'full_restore::live_tests::recovery_invalidates_unexpired_and_expired_source_tokens',
    'full_restore::live_tests::full_import_preserves_one_writer_transaction_and_exact_endpoint',
    'full_restore::live_tests::full_import_eof_before_commit_leaves_zero_objects',
    'full_restore::live_tests::full_import_cancel_retains_guards_until_isolation',
    'full_restore::live_tests::full_import_commit_unknown_is_unusable_and_no_replay',
    'full_restore::live_tests::full_import_wrong_endpoint_writes_nothing',
    'full_restore::live_tests::full_import_bad_dump_dirty_target_or_bad_role_writes_nothing',
    'full_restore::live_tests::full_import_bad_role_writes_nothing',
    'full_restore::live_tests::full_import_dirty_target_writes_nothing',
    'full_restore::live_tests::full_assets_corruption_blocks_recovery_receipt')
SOURCE_FAULT_TESTS=('source::endpoint_tests::source_restart_or_lock_loss_blocks_release','source::endpoint_tests::source_dump_timeout_cancel_and_overflow_reap_child_keep_gate_closed','source::endpoint_tests::source_dump_failure_keeps_registry_protection')
SOURCE_ENDPOINT_TESTS=(SOURCE_CLONE_TEST,SOURCE_BOUND_DUMP_TEST,*SOURCE_FAULT_TESTS)
DESTINATION_TESTS=('destination::tests::linux_cases::real_destination_negative','destination::tests::linux_cases::real_transfer_interrupted')
PG_TESTS = tuple('source::lifecycle_tests::'+name for name in (
    'real_capture_all_ready_and_retained_pin', 'finish_real_pin_after_sealed_rename',
    'finish_real_pin_from_pins_durable', 'finish_real_release_ready_before_and_after_grant',
    'abandon_early_and_late_attempts', 'abandon_crash_retry_and_terminal_ambiguity',
    'lifecycle_release_failure_compensates_same_session', 'lifecycle_admission_and_held_roots'))
FS_TESTS = (
    ('learning-assets','backup_fs::publication_contract_tests::bounded_listing_and_only_own_empty_transaction_cleanup'),
    ('learning-assets','backup_fs::publication_contract_tests::cross_parent_atomic_file_directory_and_held_parent_publication'),
    ('learning-backup','source::lifecycle_tests::journal_future_publications_are_atomic_and_legacy_bad_finals_stay_bad'),
    ('learning-backup','source::lifecycle_tests::strict_scan_joins_journal_sidecar_and_retained_staging_without_inference'),
    ('learning-backup','source::lifecycle_tests::abandonment_terminal_scan_resyncs_child_and_rechecks_before_clearing_attempt'))
LIMIT = 2*1024**2
MAX_REQUESTS = 16
LEGACY_TEST = 'source::binding_tests::matching_bound_recovery_recloses_without_finishing'
PHASE_LIMITS={'SETUP':7200,'LIVE':1200,'CLEANUP':180}
TRANSPORT_TEARDOWN=5
BUILD_PROFILE_ENV={f'CARGO_PROFILE_{profile}_{key}':value for profile in ('DEV','TEST')
                   for key,value in (('DEBUG','0'),('DEBUG_ASSERTIONS','true'),('OPT_LEVEL','0'),
                                     ('OVERFLOW_CHECKS','true'),('STRIP','none'))}
BUILD_DATA_LIMIT=128*1024**2


class GateError(RuntimeError):
    pass


def require(ok, token):
    if not ok:
        raise GateError(token)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value,sort_keys=True,ensure_ascii=False,separators=(',',':')).encode()


def unique_pairs(pairs):
    row={}
    for key,value in pairs:
        require(key not in row,'REQUEST_DUPLICATE_KEY')
        row[key]=value
    return row


def v4(value, token='REQUEST_UUID'):
    try:
        parsed=uuid.UUID(value)
        require(type(value) is str and parsed.version==4 and str(parsed)==value,token)
    except (ValueError,TypeError,AttributeError):
        raise GateError(token) from None
    return value


def load_public_helpers(directory, private=False):
    modules=[]
    for entry,pin in PUBLIC_PINS.items():
        path=directory/Path(entry).name
        fd=os.open(path,os.O_RDONLY|getattr(os,'O_NOFOLLOW',0))
        with os.fdopen(fd,'rb') as handle:
            meta=os.fstat(handle.fileno())
            require(stat.S_ISREG(meta.st_mode) and meta.st_nlink==1 and meta.st_size<=128*1024,'HELPER_FILE_IDENTITY')
            if private:
                require(meta.st_uid==os.geteuid() and stat.S_IMODE(meta.st_mode) in (0o400,0o500),'HELPER_PRIVATE_MODE')
            raw=handle.read(128*1024+1)
        require(digest(raw)==pin,'IMMUTABLE_HELPER_DIGEST')
        module=types.ModuleType('pinned_'+path.stem)
        module.__file__=str(path)
        exec(compile(raw,str(path),'exec'),module.__dict__)
        if hasattr(module,'GateError'):
            module.GateError=GateError
        modules.append(module)
    return tuple(modules)


def verify_package(helper, raw, archive_sha, manifest_sha, runner_sha):
    manifest,files=helper.verify_package(raw,archive_sha,manifest_sha,PUBLIC_PINS[ADMISSION_ENTRY],'green')
    require(manifest['base_commit']==BASE_COMMIT,'BASE_COMMIT')
    for entry,(pin,size) in PRODUCT_PINS.items():
        require(entry in files and len(files[entry])==size and digest(files[entry])==pin,'PRODUCT_PIN')
    require(TEST_ENTRY in files and ENTRY in files and HEX64.fullmatch(runner_sha) and digest(files[ENTRY])==runner_sha,'PACKAGED_RUNNER_DIGEST')
    require(all(entry in files and digest(files[entry])==pin for entry,pin in PUBLIC_PINS.items()),'PACKAGED_HELPER_DIGEST')
    require('crates/learning-backup/examples/c4_task3_migrate.rs' in files,'GENUINE_MIGRATOR_REQUIRED')
    return manifest,files


def parse_request(raw,filename,ident):
    require(0<len(raw)<=4096,'REQUEST_BUDGET')
    match=re.fullmatch(r'driver-request-([0-9a-f-]{36})\.json',filename)
    require(match is not None,'REQUEST_FILENAME')
    nonce=v4(match[1])
    try:
        row=json.loads(raw,object_pairs_hook=unique_pairs)
    except (ValueError,UnicodeDecodeError):
        raise GateError('REQUEST_JSON') from None
    require(type(row) is dict and set(row)=={'backup_id','compose_project','database','format_version','operation','request_id'},'REQUEST_FIELDS')
    require(canonical(row)==raw and type(row['format_version']) is int and row['format_version']==1 and
            row['operation']=='refresh_isolation','REQUEST_CANONICAL')
    require(row['request_id']==nonce and row['compose_project']==ident['project'] and row['database']==ident['database'],'REQUEST_IDENTITY')
    v4(row['backup_id'])
    return row


def incomplete_publication(raw):
    if not raw:
        return True
    if not raw.startswith(b'{'):
        return False
    try:
        json.loads(raw)
        return False
    except (ValueError,UnicodeDecodeError):
        return not raw.rstrip().endswith(b'}')


class GenerationLedger:
    def __init__(self,ident):
        self.ident,self.seen,self.active=ident,set(),None

    def admit(self,raw,name):
        row=parse_request(raw,name,self.ident)
        require(row['request_id'] not in self.seen,'REQUEST_REPLAY')
        require(len(self.seen)<MAX_REQUESTS,'REQUEST_COUNT')
        self.seen.add(row['request_id'])
        self.active=row
        return row

    def require_active(self,nonce):
        require(self.active is not None and self.active['request_id']==nonce,'STALE_GENERATION')


def validate_denial(code,stderr,database):
    require(type(code) is int and code in (1,2) and isinstance(stderr,bytes) and len(stderr)<=8192 and
            ('permission denied for database "'+database+'"').encode() in stderr and
            b'FATAL:' in stderr and b'CONNECT privilege' in stderr and
            not any(word in stderr for word in (b'authentication failed',b'No such file',b'timeout')),'RUNTIME_DENIAL')


def parse_test(code,output,stderr,name):
    require(stderr==b'','EXACT_TEST_STDERR')
    lines=[line.strip() for line in output.splitlines() if line.strip()]
    require(code==0 and len(lines)==3 and lines[:2]==['running 1 test',f'test {name} ... ok'],'EXACT_TEST_EXIT')
    require(re.fullmatch(r'test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; \d+ filtered out; finished in [0-9.]+s',lines[2]) is not None,'EXACT_TEST_COUNTS')
    return dict(test=name,exit_code=0,passed=1,failed=0,ignored=0,stdout_sha256=digest(output.encode()))


def validate_toc(code,raw,stderr,database):
    """Validate real bounded pg_restore --list output, never SQL/import output."""
    require(code==0 and stderr==b'' and 0<len(raw)<=LIMIT,'PG_RESTORE_TOC_EXIT_BUDGET')
    try:lines=raw.decode('utf-8').splitlines()
    except UnicodeDecodeError:raise GateError('PG_RESTORE_TOC_ENCODING') from None
    require(sum(bool(re.fullmatch(r'; Archive created at .+',line)) for line in lines)==1 and
            lines.count('; Selected TOC Entries:')==1,'PG_RESTORE_TOC_HEADER')
    def field(name):
        candidates=[line for line in lines if re.match(r';\s*'+re.escape(name)+r'\s*:',line)]
        require(len(candidates)==1,'PG_RESTORE_TOC_FIELD')
        match=re.fullmatch(r';\s+'+re.escape(name)+r': (.+)',candidates[0])
        require(match is not None,'PG_RESTORE_TOC_FIELD');return match[1]
    require(field('dbname')==database and field('Format')=='CUSTOM' and
            all(re.fullmatch(r'18\.[0-9]+(?: .*)?',field(name)) for name in ('Dumped from database version','Dumped by pg_dump version')),'PG_RESTORE_TOC_IDENTITY')
    count=field('TOC Entries');require(re.fullmatch(r'[1-9][0-9]*',count) is not None,'PG_RESTORE_TOC_COUNT')
    start=lines.index('; Selected TOC Entries:')+1;entries=[]
    require(all(not line or line.startswith(';') for line in lines[:start]),'PG_RESTORE_TOC_PLAIN_SQL')
    for line in lines[start:]:
        if not line or line.startswith(';'):continue
        require(re.fullmatch(r'[1-9][0-9]*; [0-9]+ [0-9]+ [A-Z][A-Z0-9 _]* .+',line) is not None,'PG_RESTORE_TOC_ENTRY')
        ident=int(line.split(';',1)[0]);require(ident not in entries,'PG_RESTORE_TOC_ENTRY_DUPLICATE');entries.append(ident)
    require(entries and len(entries)<=int(count),'PG_RESTORE_TOC_EMPTY')
    return dict(database=database,format='CUSTOM',entries=len(entries),declared_entries=int(count),toc_sha256=digest(raw),toc_only_no_restore=True)


class CaseBudget:
    """Finite aggregate phases; an active/stuck RPC cannot renew its deadline."""
    def __init__(self,clock=time.monotonic,total_deadline=None):
        self.clock,self.phase,self.pending=clock,'SETUP',False
        self.total_deadline=total_deadline
        self.deadline=min(clock()+PHASE_LIMITS[self.phase],total_deadline-180 if total_deadline is not None else float('inf'));self.history=[];self.rpc_deadline=None;self.refresh_deadline=None

    def check(self):
        require(self.clock()<self.deadline,'CASE_'+self.phase+'_TIMEOUT')
        if self.refresh_deadline is not None:require(self.clock()<self.refresh_deadline,'REFRESH_HOST_BUDGET')

    def remaining(self):
        self.check();return min(self.deadline,self.refresh_deadline or self.deadline)-self.clock()

    def transition(self,phase):
        require(not self.pending,'CASE_PENDING_RPC')
        require((self.phase,phase) in (('SETUP','LIVE'),('SETUP','CLEANUP'),('LIVE','CLEANUP')),'CASE_PHASE_TRANSITION')
        if phase!='CLEANUP':self.check()
        self.history.append(dict(phase=self.phase,deadline_monotonic=self.deadline))
        self.phase=phase;self.deadline=min(self.clock()+PHASE_LIMITS[phase],(self.total_deadline if phase=='CLEANUP' else self.total_deadline-180) if self.total_deadline is not None else float('inf'))

    def begin_rpc(self):
        self.check();require(not self.pending,'CASE_PENDING_RPC')
        self.pending=True;self.rpc_deadline=min(self.deadline,self.refresh_deadline or self.deadline,self.clock()+5)

    def check_rpc(self):
        require(self.pending and self.clock()<self.rpc_deadline,'BROKER_TIMEOUT');self.check()

    def end_rpc(self):
        self.pending=False;self.rpc_deadline=None


class BudgetRunner:
    def __init__(self,runner,budget):
        self.runner,self.budget=runner,budget;self.refresh_deadline=None
    def run(self,command,**kwargs):
        kwargs['timeout']=min(kwargs.get('timeout',5),self.budget.remaining())
        if self.refresh_deadline is not None:
            left=self.refresh_deadline-time.monotonic();require(left>0,'REFRESH_HOST_BUDGET')
            kwargs['timeout']=min(kwargs['timeout'],5,left)
        return self.runner.run(command,**kwargs)
    def docker(self,*args,**kwargs):return self.run([DOCKER,*args],**kwargs)[1]
    def inspect(self,kind,identity):
        rows=json.loads(self.docker(kind,'inspect',identity),object_pairs_hook=unique_pairs)
        require(type(rows) is list and len(rows)==1,'INSPECT_COUNT');return rows[0]

    def observed(self,call):return observed_rpc(self.runner,call)


def observed_rpc(runner,call):
    before=runner.counter;started=time.monotonic_ns()
    code,out,err=call()
    finished=time.monotonic_ns();require(runner.counter==before+1,'NAMESPACE_RPC_OCCURRENCE')
    prefix=f'{runner.counter:04d}';raws={}
    for suffix in ('stdout','stderr','process.json'):
        path=runner.logs/(prefix+'.'+suffix)
        require(path.is_file() and not path.is_symlink() and path.stat().st_size<=LIMIT,'NAMESPACE_RPC_FILE')
        raws[suffix]=path.read_bytes()
    process=json.loads(raws['process.json'],object_pairs_hook=unique_pairs)
    require(raws['stdout']==out and raws['stderr']==err and code==0 and err==b'' and
            canonical(process)==raws['process.json'] and set(process)=={'exit_code','reason','stdout_bytes','stderr_bytes'} and
            type(process['exit_code']) is int and process['exit_code']==0 and process['reason'] is None and
            type(process['stdout_bytes']) is int and process['stdout_bytes']==len(out) and
            type(process['stderr_bytes']) is int and process['stderr_bytes']==0,'NAMESPACE_RPC_RECEIPT')
    return out,dict(log_prefix=prefix,stdout_sha256=digest(out),stderr_sha256=digest(err),
                    process_sha256=digest(raws['process.json']),started_monotonic_ns=started,finished_monotonic_ns=finished)


# Case reservations share the actual log directory, not a guessed RPC counter.
LOG_RESERVATIONS={}
LOG_RESERVATION_LOCK=threading.RLock()


def log_capacity(logs,additional=0):
    with LOG_RESERVATION_LOCK:
        state=LOG_RESERVATIONS.get(str(logs),dict(archives=0,ledgers=0,pending=0,audits=0))
        names={p.name for p in logs.iterdir()};count=len(names)
        promised=set().union(*state.get('work',{}).values(),*state.get('child_files',{}).values())
        cleanup=3*sum(len(v) for v in state.get('cleanup',{}).values())
        require(count+len(promised-names)+additional+3*state['archives']+state['ledgers']+state['pending']+cleanup<=8192,'NAMESPACE_LOG_CAPACITY')
        return count


def runner_base(runner):return runner.runner if isinstance(runner,BudgetRunner) else runner


def reserved_call(runner,token,call):
    base=runner_base(runner);require(getattr(base,'next_log_token',None) is None,'CLEANUP_TOKEN_PENDING')
    base.next_log_token=token
    try:return call()
    finally:base.next_log_token=None


def creation_call(runner,owner,call):
    base=runner_base(runner);require(getattr(base,'pending_create',None) is None,'CREATE_INTENT_PENDING');base.pending_create=owner
    try:return call()
    finally:base.pending_create=None


def begin_log_rpc(runner):
    state=LOG_RESERVATIONS[str(runner.logs)];prefix=f'{runner.counter+1:04d}'
    with LOG_RESERVATION_LOCK:
        require(prefix not in state.setdefault('work',{}),'LOG_RPC_PENDING')
        token=getattr(runner,'next_log_token',None);runner.next_log_token=None
        if token is None:log_capacity(runner.logs,3)
        elif token[0]=='archive':
            require(token[1] in state.setdefault('archive_tokens',set()),'ARCHIVE_TOKEN_REUSED')
            state['archive_tokens'].remove(token[1]);state['archives']-=1
        else:
            owner,operation=token;tokens=state.setdefault('cleanup',{}).get(owner,set())
            require(operation in tokens,'CLEANUP_TOKEN_REUSED');tokens.remove(operation)
        state['work'][prefix]={prefix+'.'+suffix for suffix in ('stdout','stderr','process.json')}
        log_capacity(runner.logs)
    return prefix


def end_log_rpc(runner,prefix):
    with LOG_RESERVATION_LOCK:
        state=LOG_RESERVATIONS[str(runner.logs)];state['work'].pop(prefix)
        produced=state.setdefault('producer_identities',{})
        for suffix in ('stdout','stderr','process.json'):
            path=runner.logs/(prefix+'.'+suffix)
            if path.exists():
                meta=path.lstat();produced[path.name]=(meta.st_dev,meta.st_ino)


def absence_error(kind,name):
    forms={'container':'No such container: '+name,'network':'network '+name+' not found','volume':'get '+name+': no such volume'}
    require(kind in forms,'CLEANUP_RECONCILIATION_KIND')
    return ('Error response from daemon: '+forms[kind]+'\n').encode()


class CleanupReservations:
    """Small lifecycle-owned intent table; tokens authorize only existing cleanup operations."""
    def __init__(self,runner,result):
        self.runner,self.result=runner,result;self.owners={};self.children=[];self.case_id=None;self.deadline=None
    def acquire(self,kind,name,expected,network,record):
        active=[o for o in self.owners.values() if o['state'] not in ('absent','stopped','retained')]
        require(not any(o['state'] in ('planned','sent') for o in active),'CREATE_INTENT_PENDING')
        clone_names=getattr(self,'source_clone_pg_names',None)
        full_pair=getattr(self,'_full_pair',None)
        controlled_pair=getattr(self,'_controlled_pair',None)
        if controlled_pair is not None:
            from p0c4_completion.controlled_fixture import ControlledFixtureContext
            require(type(controlled_pair) is ControlledFixtureContext and full_pair is None and clone_names is None,'CONTROLLED_LEDGER_PAIR')
        if full_pair is not None:
            from p0c4_completion.full_import import FullRehearsalContext
            require(type(full_pair) is FullRehearsalContext and clone_names is None,'FULL_LEDGER_PAIR')
        if kind=='pg':
            if full_pair is not None:require(name in full_pair._owned_pg_names(),'FULL_LEDGER_PG_NAME')
            if controlled_pair is not None:require(name in controlled_pair._owned_pg_names(),'CONTROLLED_LEDGER_PG_NAME')
            require(sum(o['intent']['kind']=='pg' for o in active)<(2 if clone_names is not None or full_pair is not None or controlled_pair is not None else 1),'CLEANUP_PG_COUNT')
            if clone_names is not None:require(name in clone_names,'SOURCE_CLONE_PG_NAME')
        if kind=='helper':
            helpers=[o for o in active if o['intent']['kind']=='helper']
            require(len(helpers)<2 and all(o['intent']['expected']['stdin'] and not o.get('close_failed') for o in helpers) and
                    len(self.result['helpers'])<82,'CLEANUP_HELPER_COUNT')
        count={'pg':11,'helper':11,'volume':1}[kind];state=LOG_RESERVATIONS[str(self.runner.logs)]
        with LOG_RESERVATION_LOCK:
            log_capacity(self.runner.logs,3*count)
            operations=({'pg-'+str(i) for i in range(9)}|{'reconcile','network-reconcile'} if kind=='pg' else
                        {phase+'-'+str(i) for phase in ('close','fallback') for i in range(5)}|{'reconcile'} if kind=='helper' else {'reconcile'})
            require(name not in state.setdefault('cleanup',{}),'CLEANUP_OWNER_REUSED');state['cleanup'][name]=operations
        owner=dict(intent=dict(kind=kind,name=name,case_id=self.case_id,batch_id=self.result['batch_id'],expected=copy.deepcopy(expected),network=copy.deepcopy(network)),
                   state='planned',id=None,record=record)
        self.owners[name]=owner;return owner
    def known(self,owner,identity):
        require(HEX64.fullmatch(identity) if owner['intent']['kind']!='volume' else identity==owner['intent']['name'],'CLEANUP_CREATED_ID')
        owner.update(state='known',id=identity)
        tokens=LOG_RESERVATIONS[str(self.runner.logs)]['cleanup'][owner['intent']['name']]
        tokens.discard('reconcile');tokens.discard('network-reconcile')
    def release(self,owner,state):
        owner['state']=state
        with LOG_RESERVATION_LOCK:LOG_RESERVATIONS[str(self.runner.logs)]['cleanup'][owner['intent']['name']].clear()
    def enter(self,budget=None):
        if self.deadline is None:self.deadline=budget.deadline if budget is not None else time.monotonic()+180
    def left(self):
        require(self.deadline is not None and time.monotonic()<self.deadline,'CASE_CLEANUP_TIMEOUT');return self.deadline-time.monotonic()
    def reconcile(self,owner,kind='container',network=False):
        name=owner['intent']['network']['name'] if network else owner['intent']['name']
        token=(owner['intent']['name'],'network-reconcile' if network else 'reconcile')
        before=self.runner.counter;started=time.monotonic_ns()
        answer=reserved_call(self.runner,token,lambda:self.runner.run([DOCKER,kind,'inspect',name],timeout=min(5,self.left()),allowed=(0,1)))
        finished=time.monotonic_ns();code,out,err=answer
        require(self.runner.counter==before+1,'CLEANUP_RPC_OCCURRENCE')
        prefix=f'{self.runner.counter:04d}';raw=(self.runner.logs/(prefix+'.process.json')).read_bytes();process=json.loads(raw,object_pairs_hook=unique_pairs)
        require(canonical(process)==raw and process==dict(exit_code=code,reason=None,stdout_bytes=len(out),stderr_bytes=len(err)) and
                all(type(process[k]) is int for k in ('exit_code','stdout_bytes','stderr_bytes')) and
                (self.runner.logs/(prefix+'.stdout')).read_bytes()==out and (self.runner.logs/(prefix+'.stderr')).read_bytes()==err,'CLEANUP_RPC_PROCESS')
        rpc=dict(log_prefix=prefix,stdout_sha256=digest(out),stderr_sha256=digest(err),process_sha256=digest(raw),started_monotonic_ns=started,finished_monotonic_ns=finished)
        receipt=owner['record'].setdefault('cleanup_reconciliation',dict(outcome='UNKNOWN',rpcs=[],discovered_id=None));receipt['rpcs'].append(rpc)
        if code==1 and out==b'[]\n' and err==absence_error(kind,name):return None
        require(code==0 and err==b'','CLEANUP_DISCOVERY_UNKNOWN')
        rows=json.loads(out,object_pairs_hook=unique_pairs);require(type(rows) is list and len(rows)==1,'CLEANUP_DISCOVERY_UNKNOWN')
        return rows[0]


class CleanupRunner:
    def __init__(self,runner,owner,phase):
        self.runner,self.owner,self.phase,self.index=runner,owner,phase,0
        schema={'pg':[]} if owner['intent']['kind']=='pg' else {'close':[],'fallback':[]}
        self.receipts=owner['record'].setdefault('cleanup_rpcs',schema)[phase]
    def run(self,command,**kwargs):
        base=runner_base(self.runner);registry=base.ownership
        if registry.deadline is not None:kwargs['timeout']=min(kwargs.get('timeout',5),registry.left())
        token=(self.owner['intent']['name'],self.phase+'-'+str(self.index));self.index+=1
        before=base.counter;started=time.monotonic_ns();error=None;answer=None
        try:answer=reserved_call(self.runner,token,lambda:self.runner.run(command,**kwargs))
        except BaseException as caught:error=caught
        finished=time.monotonic_ns();prefix=f'{base.counter:04d}'
        paths=[base.logs/(prefix+'.'+suffix) for suffix in ('stdout','stderr','process.json')]
        complete=base.counter==before+1 and all(p.is_file() and not p.is_symlink() and p.stat().st_size<=LIMIT for p in paths)
        if complete:
            out,err,process=[p.read_bytes() for p in paths]
            self.receipts.append(dict(log_prefix=prefix,stdout_sha256=digest(out),stderr_sha256=digest(err),process_sha256=digest(process),
                                      started_monotonic_ns=started,finished_monotonic_ns=finished))
        if error is not None:raise error
        require(complete,'CLEANUP_RECEIPT_MISSING');return answer
    def docker(self,*args,**kwargs):return self.run([DOCKER,*args],**kwargs)[1]
    def inspect(self,kind,identity):
        rows=json.loads(self.docker(kind,'inspect',identity),object_pairs_hook=unique_pairs)
        require(type(rows) is list and len(rows)==1,'INSPECT_COUNT');return rows[0]
    def observed(self,call):return observed_rpc(runner_base(self.runner),call)
    @property
    def namespace(self):return self.runner.namespace


NETWORK_ID_LIST=('network','ls','--no-trunc','--format','{{.ID}}\t{{.Name}}\t{{.Label "com.docker.compose.project"}}')
NETWORK_NAME_LIST=('network','ls','--format','{{.Name}}\t{{.Label "com.docker.compose.project"}}')
IPAM_FORMAT='{{json .IPAM.Config}}'


def network_list_rows(raw,ids=True):
    require(type(raw) is bytes and len(raw)<=LIMIT and (not raw or raw.endswith(b'\n')) and b'\r' not in raw,'PREFLIGHT_LIST_FRAMING')
    try:lines=raw.decode('utf-8').split('\n')[:-1] if raw else []
    except UnicodeDecodeError:raise GateError('PREFLIGHT_LIST_ENCODING') from None
    result={};names=set()
    for line in lines:
        fields=line.split('\t');require(len(fields)==(3 if ids else 2),'PREFLIGHT_LIST_FIELDS')
        if ids:
            identity,name,project=fields;require(HEX64.fullmatch(identity),'PREFLIGHT_NETWORK_ID')
        else:name,project=fields;identity=name
        require(name and '\x00' not in line and identity not in result and name not in names,'PREFLIGHT_LIST_DUPLICATE')
        result[identity]=dict(name=name,project=project);names.add(name)
    return result


class ResourcePreflightAdapter:
    """Disposable raw slices from real ordered batched IPAM calls, never a cache across invocations."""
    def __init__(self,helper,runner,ordinal,case_id):
        self.helper,self.runner,self.active=helper,runner,True
        self.rows,self.values,self.consumed={}, {},set()
        self.receipt=dict(ordinal=ordinal,case_id=case_id,batches=[])
        raw,rpc=self.rpc([DOCKER,*NETWORK_ID_LIST]);self.rows=network_list_rows(raw);self.receipt['pre']=rpc

    def rpc(self,command,**kwargs):
        require(self.active,'PREFLIGHT_ADAPTER_REUSE')
        return observed_rpc(self.runner,lambda:self.runner.run(command,**kwargs))

    def run(self,command,**kwargs):
        require(self.active,'PREFLIGHT_ADAPTER_REUSE')
        if command[0] in ('/usr/sbin/ip','/usr/bin/ip'):
            require(command[1:]==['-json','route','show','table','all'] and 'route' not in self.receipt,'PREFLIGHT_ROUTE_SHAPE')
            raw,rpc=self.rpc(command,**kwargs);self.receipt['route']=rpc;return 0,raw,b''
        return self.runner.run(command,**kwargs)

    def docker(self,*args,**kwargs):
        require(self.active,'PREFLIGHT_ADAPTER_REUSE')
        labels='{{.Names}}\t{{.Label "com.docker.compose.project"}}'
        if args==('ps','-a','--format',labels):key='containers'
        elif args==('volume','ls','--format','{{.Name}}\t{{.Label "com.docker.compose.project"}}'):key='volumes'
        elif args==NETWORK_NAME_LIST:key='networks'
        elif args[:2]==('network','inspect'):
            require(len(args)==5 and args[2:4]==('--format',IPAM_FORMAT) and args[4] in self.values and args[4] not in self.consumed and not kwargs,'PREFLIGHT_IPAM_LOOKUP')
            self.consumed.add(args[4]);return self.values[args[4]]
        else:
            require(args[:2]!=('network','ls'),'PREFLIGHT_LIST_COMMAND')
            return self.runner.docker(*args,**kwargs)
        require(key not in self.receipt,'PREFLIGHT_DUPLICATE_CALL')
        raw,rpc=self.rpc([DOCKER,*args],**kwargs);self.receipt[key]=rpc
        if key=='networks':
            actual=network_list_rows(raw,False);expected={r['name']:r for r in self.rows.values()}
            require(actual==expected,'PREFLIGHT_HELPER_LIST_CHANGED')
            ordered=sorted(self.rows);chunks=[ordered[n:n+16] for n in range(0,len(ordered),16)]
            log_capacity(self.runner.logs,3*(len(chunks)+2))
            for chunk in chunks:
                output,rpc=self.rpc([DOCKER,'network','inspect','--format',IPAM_FORMAT,*chunk])
                require(len(output)<=LIMIT and output.endswith(b'\n') and output.count(b'\n')==len(chunk),'PREFLIGHT_IPAM_FRAMING')
                values=output.split(b'\n')[:-1]
                for identity,line in zip(chunk,values):
                    require(line and b'\r' not in line,'PREFLIGHT_IPAM_FRAMING')
                    try:value=json.loads(line,object_pairs_hook=unique_pairs)
                    except (ValueError,UnicodeDecodeError):raise GateError('PREFLIGHT_IPAM_JSON') from None
                    self.helper.occupied_subnets(value)
                    self.values[self.rows[identity]['name']]=line+b'\n'
                self.receipt['batches'].append(rpc)
        return raw

    def finish(self):
        require(self.active and set(self.receipt)=={'ordinal','case_id','pre','route','containers','networks','volumes','batches'} and
                self.consumed=={r['name'] for r in self.rows.values()},'PREFLIGHT_INCOMPLETE')
        raw,rpc=self.rpc([DOCKER,*NETWORK_ID_LIST]);require(network_list_rows(raw)==self.rows,'PREFLIGHT_POST_LIST_CHANGED')
        self.receipt['post']=rpc;return self.receipt

    def close(self):
        self.active=False;self.rows.clear();self.values.clear();self.consumed.clear()


def fresh_resource_preflight(helper,runner,identities,subnets,result,case_id=None,*,route_observer=None):
    receipts=result.setdefault('resource_preflights',[]);ordinal=len(receipts)
    require(ordinal<10 and ((ordinal==0 and case_id is None) or (ordinal>0 and v4(case_id) and len(identities)==1 and identities[0]['case_id']==case_id)),'PREFLIGHT_INVOCATION')
    adapter=ResourcePreflightAdapter(helper,runner,ordinal,case_id)
    try:
        native=None
        if route_observer is not None:
            def native():
                before=len(result.get('host_route_observations',[]));routes=route_observer()
                require(len(result.get('host_route_observations',[]))==before+1 and 'route' not in adapter.receipt,'PREFLIGHT_NATIVE_ROUTE_RECEIPT')
                adapter.receipt['route']=dict(observer='native_host_netlink',evidence=result['host_route_observations'][-1])
                return routes
        answer=helper.fresh_resources(adapter,identities,subnets,'',route_observer=native)
        receipts.append(adapter.finish());return answer
    finally:adapter.close()


JOURNAL_PHASES=('intent','closed','drained','dump_and_index_durable','pins_durable','release_ready','released')


def classify_attempt_documents(attempt,binding,journal,sidecar,*,journal_exists,sidecar_exists=False):
    """Observation only: missing/corrupt bytes never establish an API outcome."""
    answer=dict(journal=dict(status='ABSENT',phase=None,file_sha256={name:digest(raw) for name,raw in journal.items()}),
                sidecar=dict(status='ABSENT',file_sha256={name:digest(raw) for name,raw in sidecar.items()}))
    def ordered(row):return json.dumps(row,ensure_ascii=False,separators=(',',':')).encode()
    try:
        require(journal_exists or not journal,'STATE_JOURNAL')
        if journal_exists:
            names=[phase.replace('_','-')+'.json' for phase in JOURNAL_PHASES]
            require(journal and set(journal)==set(names[:len(journal)]) and len(journal)<=7,'STATE_JOURNAL_SEQUENCE')
            previous=None;last_raw=None
            for number,name in enumerate(names[:len(journal)]):
                raw=journal[name];row=json.loads(raw,object_pairs_hook=unique_pairs)
                keys=('backup_id','phase','dump_and_index_sha256','pins_sha256')
                require(type(row) is dict and set(row)==set(keys) and 0<len(raw)<=4096 and ordered({k:row[k] for k in keys})==raw and
                        row['backup_id']==attempt and row['phase']==JOURNAL_PHASES[number],'STATE_JOURNAL_CANONICAL')
                for key,needed in (('dump_and_index_sha256',number>=3),('pins_sha256',number>=4)):
                    require((isinstance(row[key],str) and HEX64.fullmatch(row[key])) if needed else row[key] is None,'STATE_JOURNAL_PROOF')
                    if previous and previous[key] is not None:require(row[key]==previous[key],'STATE_JOURNAL_TRANSITION')
                previous,last_raw=row,raw
            answer['journal'].update(status='CANONICAL',phase=previous['phase'],last_record_sha256=digest(last_raw))
        if sidecar_exists or sidecar:
            require(journal_exists and answer['journal']['status']=='CANONICAL' and previous['phase']!='released','STATE_SIDECAR_ORPHAN')
            for name,raw in sidecar.items():
                require(name in ('ready.json','abandoned.json') or re.fullmatch(r'\.tmp-[0-9a-f-]{36}',name),'STATE_SIDECAR_ENTRY')
                if name.startswith('.tmp-'):v4(name[5:],'STATE_TEMP_UUID');require(len(raw)<=4096,'STATE_TEMP_BUDGET')
            if 'ready.json' not in sidecar:
                require('abandoned.json' not in sidecar,'STATE_TERMINAL_ORPHAN');answer['sidecar']['status']='UNRESOLVED'
            else:
                ready=dict(format_version=1,capability='source_abandonment_v1',backup_id=attempt,action='abandon',source_binding_sha256=binding,
                           journal_phase=previous['phase'],journal_record_sha256=digest(last_raw),retained_artifacts='keep_all',state='release_ready')
                require(ordered(ready)==sidecar['ready.json'],'STATE_READY_CANONICAL')
                answer['sidecar']['status']='READY'
                if 'abandoned.json' in sidecar:
                    terminal=dict(format_version=1,capability='source_abandonment_v1',backup_id=attempt,action='abandon',source_binding_sha256=binding,
                                  ready_sha256=digest(sidecar['ready.json']),state='abandoned')
                    require(ordered(terminal)==sidecar['abandoned.json'],'STATE_TERMINAL_CANONICAL')
                    answer['sidecar']['status']='ABANDONED'
    except (GateError,ValueError,UnicodeDecodeError,TypeError,KeyError):
        if answer['journal']['status']!='CANONICAL':answer['journal'].update(status='CORRUPT',phase=None)
        else:answer['sidecar']['status']='CORRUPT'
    return answer


def generation_classification(observations,denial):
    state='UNKNOWN'
    if denial is not None:state='ACTUAL_RUNTIME_CONNECT_DENIED'
    elif len(observations)>=2:
        first,last=observations[0],observations[-1];before,after=first['attempt_state'],last['attempt_state']
        if before==after and first['acl']==last['acl']:
            if after['journal']['status']=='CANONICAL' and (after['journal']['phase']=='released' or after['sidecar']['status']=='ABANDONED'):
                if after['sidecar']['status'] in ('ABSENT','ABANDONED'):state='OBSERVED_READ_ONLY_TERMINAL_STATE'
            elif after['journal']['status']=='ABSENT' and after['sidecar']['status']=='ABSENT' and first['acl']=='t':
                pub=after.get('publication') or {}
                if pub.get('scan_complete') is True and pub.get('dump_files')==[] and pub.get('manifest_files')==[]:state='OBSERVED_PREFLIGHT_STATE'
    return dict(state=state,operation_outcome='UNPROVEN',basis='bounded_boundary_observations_only')


def validate_drain_barrier(observation):
    state=observation['attempt_state'];publication=state.get('publication') or {}
    require(observation['acl']=='f' and observation['runtime_state']=='idle in transaction' and
            state['journal']['status']=='CANONICAL' and state['journal']['phase']=='closed' and state['sidecar']['status']=='ABSENT' and
            publication.get('scan_complete') is True and publication.get('dump_files')==[] and publication.get('manifest_files')==[],'DRAIN_PREDUMP_BARRIER')
    return observation


def require_legacy_observation(record,values):
    receipt=record.get('legacy_rendezvous') or {};process=record.get('consumer_process') or {};cohort=record.get('legacy_cohort') or []
    binary=record.get('compiled',{}).get('lib',{}).get('binary')
    require(receipt.get('lock_confirmed') is True and receipt.get('deadline_expired') is False and receipt.get('commit_exit')==0 and receipt.get('backend_absent') is True and
            type(process.get('pid')) is int and process['pid']>0 and type(process.get('starttime')) is int and process['starttime']>0 and
            process.get('exe')==binary and process.get('env_keys')==sorted(values) and
            any(row['pid']==process['pid'] and row['starttime']==process['starttime'] and row['exe']==binary and row['uid']==0 and row['env_keys']==sorted(values) for row in cohort),'LEGACY_ACTUAL_OBSERVATION')
    return process


def attempt_snapshot(control_fd,pin_fd,attempt,binding):
    """Bounded read-only observation from the originally held root handles."""
    def documents(parent,name):
        try:fd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=parent)
        except FileNotFoundError:return {},False,None
        except OSError:return {},True,'STATE_DIRECTORY_IO'
        files={};failure=None
        try:
            meta=os.fstat(fd);require(meta.st_uid==0 and stat.S_IMODE(meta.st_mode)==0o700,'STATE_DIRECTORY_METADATA')
            names=os.listdir(fd);require(len(names)<=32,'STATE_DIRECTORY_BUDGET')
            for leaf in sorted(names):
                child=os.open(leaf,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=fd)
                with os.fdopen(child,'rb') as handle:
                    m=os.fstat(handle.fileno());require(stat.S_ISREG(m.st_mode) and m.st_uid==0 and stat.S_IMODE(m.st_mode)==0o600 and m.st_nlink==1 and m.st_size<=4096,'STATE_FILE_METADATA')
                    raw=handle.read(4097);after=os.fstat(handle.fileno());require(m.st_size==after.st_size and len(raw)<=4096,'STATE_FILE_CHANGED')
                    files[leaf]=raw
        except (OSError,GateError) as error:failure=str(error) if isinstance(error,GateError) else 'STATE_FILE_IO'
        finally:os.close(fd)
        return files,True,failure
    journal,exists,jerror=documents(control_fd,attempt+'.control')
    sidecar,side_exists,serror=documents(control_fd,attempt+'.abandonment')
    state=classify_attempt_documents(attempt,binding,journal,sidecar,journal_exists=exists,sidecar_exists=side_exists)
    if jerror:state['journal'].update(status='CORRUPT',phase=None,error=jerror)
    if serror:state['sidecar'].update(status='CORRUPT',error=serror)
    state['raw_records']={'journal':{k:v.hex() for k,v in journal.items()},'sidecar':{k:v.hex() for k,v in sidecar.items()}}
    publication=dict(dump_files=[],manifest_files=[],scan_complete=True,entries=[])
    def signature(meta):
        return (meta.st_dev,meta.st_ino,meta.st_mode,meta.st_nlink,meta.st_size,meta.st_mtime_ns,meta.st_ctime_ns)
    def scan(parent,name,prefix,required=False):
        try:entry=os.stat(name,dir_fd=parent,follow_symlinks=False)
        except FileNotFoundError:
            require(not required,'STATE_PUBLICATION_DISAPPEARED');return
        require(stat.S_ISDIR(entry.st_mode),'STATE_PUBLICATION_DIRECTORY')
        try:fd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=parent)
        except FileNotFoundError:
            raise GateError('STATE_PUBLICATION_DISAPPEARED') from None
        try:
            before=os.fstat(fd);require(signature(before)==signature(entry),'STATE_PUBLICATION_CHANGED')
            names=sorted(os.listdir(fd));require(len(names)<=128,'STATE_PUBLICATION_BUDGET')
            for leaf in names:
                require(len(publication['entries'])<128,'STATE_PUBLICATION_BUDGET')
                meta=os.stat(leaf,dir_fd=fd,follow_symlinks=False);relative=prefix+'/'+leaf
                require(stat.S_ISREG(meta.st_mode) or stat.S_ISDIR(meta.st_mode),'STATE_PUBLICATION_SPECIAL')
                if stat.S_ISREG(meta.st_mode):require(meta.st_nlink==1,'STATE_PUBLICATION_LINKED')
                publication['entries'].append(dict(path=relative,bytes=meta.st_size,mode=stat.S_IMODE(meta.st_mode),uid=meta.st_uid))
                if stat.S_ISDIR(meta.st_mode):scan(fd,leaf,relative,required=True)
                elif leaf in ('database.dump','manifest.json'):publication['dump_files' if leaf=='database.dump' else 'manifest_files'].append(relative)
                require(signature(meta)==signature(os.stat(leaf,dir_fd=fd,follow_symlinks=False)),'STATE_PUBLICATION_CHANGED')
            require(names==sorted(os.listdir(fd)) and signature(before)==signature(os.fstat(fd)) and
                    signature(entry)==signature(os.stat(name,dir_fd=parent,follow_symlinks=False)),'STATE_PUBLICATION_CHANGED')
        finally:os.close(fd)
    try:
        control_before=os.fstat(control_fd);pin_before=os.fstat(pin_fd);pin_names=sorted(os.listdir(pin_fd))
        require(len(pin_names)<=128,'STATE_PUBLICATION_BUDGET')
        scan(control_fd,attempt+'.source','control/'+attempt+'.source')
        scan(pin_fd,attempt+'.sealed','pins/'+attempt+'.sealed')
        for name in pin_names:
            if name.startswith(attempt+'.staging'):
                suffix=name.removeprefix(attempt+'.staging-')
                require(name.startswith(attempt+'.staging-'),'STATE_STAGING_NAME')
                v4(suffix,'STATE_STAGING_NAME')
                scan(pin_fd,name,'pins/'+name,required=True)
        require(pin_names==sorted(os.listdir(pin_fd)) and signature(pin_before)==signature(os.fstat(pin_fd)) and
                signature(control_before)==signature(os.fstat(control_fd)),'STATE_PUBLICATION_CHANGED')
    except (OSError,GateError):
        publication['scan_complete']=False
    state['publication']=publication
    state['held_roots']={name:dict(dev=os.fstat(fd).st_dev,ino=os.fstat(fd).st_ino) for name,fd in (('control',control_fd),('pins',pin_fd))}
    require(len(canonical(state))<=128*1024,'STATE_OBSERVATION_BUDGET')
    return state


def discover_artifact(output,package,kind):
    targets={'learning-backup':('learning_backup','c4_task3_migrate'),'learning-assets':('learning_assets',None)}
    require(package in targets and kind in ('lib','example'),'BUILD_ARTIFACT_SELECTOR')
    name=targets[package][0 if kind=='lib' else 1]
    finished,artifacts=[],[]
    try:
        for line in output.splitlines():
            row=json.loads(line,object_pairs_hook=unique_pairs)
            if row.get('reason')=='build-finished':
                finished.append(row.get('success'))
            if row.get('reason')=='compiler-artifact' and row.get('manifest_path')==f'/reviewed/crates/{package}/Cargo.toml' and row.get('target',{}).get('name')==name and row['target'].get('kind')==[kind] and row.get('profile',{}).get('test') is (kind=='lib'):
                artifacts.append(row.get('executable'))
    except (ValueError,TypeError,KeyError):
        raise GateError('BUILD_ARTIFACT_JSON') from None
    pattern='/target/build/debug/deps/'+name+'-[0-9a-f]+' if kind=='lib' else '/target/build/debug/examples/c4_task3_migrate'
    require(finished==[True] and len(artifacts)==1 and type(artifacts[0]) is str and re.fullmatch(pattern,artifacts[0]),'BUILD_ARTIFACT')
    return artifacts[0]


def validate_compiler_profile(output,artifact,expected_test):
    try:
        rows=[json.loads(line,object_pairs_hook=unique_pairs) for line in output.splitlines()]
        profiles=[row.get('profile') for row in rows if row.get('reason')=='compiler-artifact' and row.get('executable')==artifact]
        require(len(profiles)==1 and type(profiles[0]) is dict,'BUILD_PROFILE')
        profile=profiles[0]
        require(profile.get('opt_level')=='0' and type(profile.get('debuginfo')) is int and profile['debuginfo']==0 and
                profile.get('debug_assertions') is True and profile.get('overflow_checks') is True and
                profile.get('test') is expected_test,'BUILD_PROFILE')
        return {key:profile[key] for key in ('opt_level','debuginfo','debug_assertions','overflow_checks','test')}
    except (ValueError,TypeError,KeyError):
        raise GateError('BUILD_PROFILE') from None


def materialize_build_data(source,destination):
    """Treat compiler output as data; publish a new private single-link copy only after readback."""
    require(os.name=='posix' and os.geteuid()==0,'MATERIALIZE_ROOT')
    source,destination=Path(source),Path(destination)
    require(source.is_absolute() and destination.is_absolute() and
            not any(part in ('.','..') for path in (source,destination) for part in path.parts),'MATERIALIZE_PATH')
    held=[];chains=[]
    def identity(row):return (row.st_dev,row.st_ino,row.st_uid,row.st_mode)
    def signature(row):return (*identity(row),row.st_nlink,row.st_size,row.st_mtime_ns,row.st_ctime_ns)
    def metadata(row):return dict(dev=row.st_dev,ino=row.st_ino,uid=row.st_uid,mode=stat.S_IMODE(row.st_mode),nlink=row.st_nlink,bytes=row.st_size)
    def parent(path,private=False):
        fd=os.open('/',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW);held.append(fd)
        chain=[]
        require(os.fstat(fd).st_uid==0 and os.fstat(fd).st_mode&0o022==0,'MATERIALIZE_ANCESTOR')
        for part in path.parent.parts[1:]:
            row=os.stat(part,dir_fd=fd,follow_symlinks=False)
            require(stat.S_ISDIR(row.st_mode) and row.st_uid==0 and row.st_mode&0o022==0,'MATERIALIZE_ANCESTOR')
            next_fd=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd);held.append(next_fd)
            require(identity(row)==identity(os.fstat(next_fd)),'MATERIALIZE_ANCESTOR_CHANGED')
            chain.append((fd,part,next_fd,identity(row)));fd=next_fd
        if private:require(stat.S_IMODE(os.fstat(fd).st_mode)==0o700,'MATERIALIZE_PRIVATE_PARENT')
        chains.append(chain)
        return fd
    def stable_parents():
        for chain in chains:
            for fd,name,child,before in chain:
                require(identity(os.fstat(child))==before and identity(os.stat(name,dir_fd=fd,follow_symlinks=False))==before,
                        'MATERIALIZE_ANCESTOR_CHANGED')
    try:
        src_parent=parent(source);dst_parent=parent(destination,True)
        row=os.stat(source.name,dir_fd=src_parent,follow_symlinks=False)
        require(stat.S_ISREG(row.st_mode) and row.st_uid==0 and row.st_mode&0o022==0 and row.st_nlink>=1 and
                0<row.st_size<=BUILD_DATA_LIMIT,'MATERIALIZE_SOURCE')
        src=os.open(source.name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=src_parent);held.append(src)
        require(signature(row)==signature(os.fstat(src)),'MATERIALIZE_SOURCE_CHANGED')
        try:dst=os.open(destination.name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600,dir_fd=dst_parent)
        except FileExistsError:raise GateError('MATERIALIZE_DESTINATION_EXISTS') from None
        held.append(dst);sha=hashlib.sha256();size=0
        while True:
            raw=os.read(src,1024**2)
            if not raw:break
            size+=len(raw);require(size<=BUILD_DATA_LIMIT,'MATERIALIZE_SOURCE_CHANGED');sha.update(raw)
            view=memoryview(raw)
            while view:
                written=os.write(dst,view);require(written>0,'MATERIALIZE_IO');view=view[written:]
        require(size==row.st_size and signature(row)==signature(os.fstat(src)) and
                signature(row)==signature(os.stat(source.name,dir_fd=src_parent,follow_symlinks=False)),'MATERIALIZE_SOURCE_CHANGED')
        stable_parents();os.fchmod(dst,0o500);os.fsync(dst);os.fsync(dst_parent)
        copied=os.fstat(dst)
        require(stat.S_ISREG(copied.st_mode) and copied.st_uid==0 and stat.S_IMODE(copied.st_mode)==0o500 and
                copied.st_nlink==1 and copied.st_size==size and (copied.st_dev,copied.st_ino)!=(row.st_dev,row.st_ino),'MATERIALIZE_DESTINATION')
        reader=os.open(destination.name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=dst_parent);held.append(reader)
        require(signature(copied)==signature(os.fstat(reader)),'MATERIALIZE_DESTINATION_CHANGED')
        read_sha=hashlib.sha256();read_size=0
        while True:
            raw=os.read(reader,1024**2)
            if not raw:break
            read_size+=len(raw);require(read_size<=BUILD_DATA_LIMIT,'MATERIALIZE_DESTINATION_CHANGED');read_sha.update(raw)
        require(read_size==size and read_sha.hexdigest()==sha.hexdigest() and signature(copied)==signature(os.fstat(reader)) and
                signature(copied)==signature(os.stat(destination.name,dir_fd=dst_parent,follow_symlinks=False)),
                'MATERIALIZE_DESTINATION_CHANGED')
        require(signature(row)==signature(os.fstat(src)) and signature(row)==signature(os.stat(source.name,dir_fd=src_parent,follow_symlinks=False)),
                'MATERIALIZE_SOURCE_CHANGED')
        stable_parents()
        return dict(source_path=str(source),source=metadata(row),destination_path=str(destination),destination=metadata(copied),bytes=size,sha256=sha.hexdigest())
    except OSError:
        raise GateError('MATERIALIZE_IO') from None
    finally:
        for fd in reversed(held):os.close(fd)


def materialize_main():
    require(len(sys.argv)==3,'MATERIALIZE_ARGUMENTS')
    source,destination=sys.argv[1:]
    require(re.fullmatch(r'/target/build/debug/(?:deps/(?:learning_backup|learning_assets)-[0-9a-f]+|examples/c4_task3_migrate)',source) and
            re.fullmatch(r'/target/materialized/[0-9a-f-]{36}-(?:lib|example|assets)',destination),'MATERIALIZE_ARGUMENTS')
    v4(Path(destination).name.rsplit('-',1)[0],'MATERIALIZE_ARGUMENTS')
    # /target is the fresh named volume. Hold its trusted directory while creating the private child.
    fd=os.open('/target',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:
        row=os.fstat(fd)
        require(row.st_uid==0 and row.st_mode&0o022==0,'MATERIALIZE_ANCESTOR')
        try:os.mkdir('materialized',0o700,dir_fd=fd);os.fsync(fd)
        except FileExistsError:pass
        receipt=materialize_build_data(Path(source),Path(destination))
        require((os.stat('/target',follow_symlinks=False).st_dev,os.stat('/target',follow_symlinks=False).st_ino)==(row.st_dev,row.st_ino),'MATERIALIZE_ANCESTOR_CHANGED')
        print(canonical(receipt).decode())
    finally:os.close(fd)


def materialize_artifact(h,b,runner,result,image,source,ident,artifact,kind):
    intermediate='/target/materialized/'+ident['case_id']+'-'+kind
    command=['python3','-c','import sys;sys.path.insert(0,"/reviewed/scripts");from p0c4_source_lifecycle_gate_acceptance import materialize_main;materialize_main()',artifact,intermediate]
    _,out,err=run_helper(h,b,runner,result,image,mounts(source,ident),command)
    require(err==b'','MATERIALIZE_STDERR');receipt=json.loads(out,object_pairs_hook=unique_pairs)
    require(type(receipt) is dict and set(receipt)=={'source_path','source','destination_path','destination','bytes','sha256'} and
            receipt['source_path']==artifact and receipt['destination_path']==intermediate and type(receipt['bytes']) is int and
            0<receipt['bytes']<=BUILD_DATA_LIMIT and type(receipt['sha256']) is str and HEX64.fullmatch(receipt['sha256']),'MATERIALIZE_RECEIPT')
    for key in ('source','destination'):
        row=receipt[key]
        require(type(row) is dict and set(row)=={'dev','ino','uid','mode','nlink','bytes'} and all(type(value) is int for value in row.values()) and
                row['dev']>=0 and row['ino']>0 and row['uid']==0 and row['bytes']==receipt['bytes'] and row['nlink']>=1 and row['mode']&0o022==0,'MATERIALIZE_RECEIPT')
    require(receipt['destination']['mode']==0o500 and receipt['destination']['nlink']==1 and
            (receipt['source']['dev'],receipt['source']['ino'])!=(receipt['destination']['dev'],receipt['destination']['ino']),'MATERIALIZE_RECEIPT')
    return intermediate,receipt


def compile_binding(record):
    issuer=record.get('issuer') or {}
    binding=issuer.get('binding') or {}
    observed=record.get('pg_root_before')
    pin=issuer.get('binding_sha256')
    require(type(pin) is str and HEX64.fullmatch(pin) and observed==dict(binding_sha256=pin,control_dev=binding.get('control_dev'),control_ino=binding.get('control_ino')),'ISSUER_BEFORE_COMPILE')
    return pin


def validate_issuer(row,sql_identity):
    require(type(row) is dict and set(row)=={'binding','binding_sha256','sql','issuer_euid','classification','registry_provisioning'} and
            row['issuer_euid']==0 and row['classification']=='INDEPENDENT_PRECOMPILE_BINDING' and row['sql']==sql_identity,'ISSUER_RECEIPT')
    binding=row['binding']
    require(type(binding) is dict and set(binding)=={'format_version','capability','binding_id','control_path','control_dev','control_ino','database','database_oid','system_identifier'} and
            type(binding['format_version']) is int and binding['format_version']==1 and binding['capability']=='source_control_binding_v1' and
            binding['control_path']==ROOT+'/control' and all(type(binding[k]) is int and 0<binding[k]<2**64 for k in ('control_dev','control_ino')) and
            all(binding[k]==sql_identity[k] for k in ('database','database_oid','system_identifier')),'ISSUER_BINDING_SCHEMA')
    v4(binding['binding_id'],'ISSUER_UUID')
    require(row['binding_sha256']==digest(canonical(binding)),'ISSUER_CANONICAL_PIN')
    try: from p0c4_storage_registry import validate_provisioning_receipt, RegistryError
    except ModuleNotFoundError: from scripts.p0c4_storage_registry import validate_provisioning_receipt, RegistryError
    try: receipt=validate_provisioning_receipt(row['registry_provisioning'])
    except (RegistryError,KeyError,TypeError):raise GateError('ISSUER_REGISTRY_RECEIPT')
    require(receipt['source_binding_sha256']==row['binding_sha256'] and any(r['kind']=='source_control' and r['path']==binding['control_path'] and r['dev']==binding['control_dev'] and r['ino']==binding['control_ino'] for r in receipt['roots']),'ISSUER_REGISTRY_BINDING')
    return row


def validate_anchor(row,previous=None):
    require(type(row.get('pid')) is int and row['pid']>0 and type(row.get('starttime')) is int and row['starttime']>0 and
            row.get('uids')==[0,0,0,0] and row.get('flock_exit')==73 and
            row.get('fd_dev')==row.get('lock_dev') and row.get('fd_ino')==row.get('lock_ino') and
            all(type(row.get(k)) is int and row[k]>0 for k in ('fd_dev','fd_ino')) and
            row.get('pid_ns')==row.get('pg_pid_ns') and re.fullmatch(r'pid:\[\d+\]',str(row.get('pid_ns'))),'ANCHOR_IDENTITY')
    if previous:
        require(row==previous,'ANCHOR_REUSED_PID')
    return row


def consumer_env(ident,*,_full_context=None):
    values={'PATH':'/usr/bin:/bin','HOME':'/root','LC_ALL':'C'}
    paths={'CONTROL_ROOT':'control','PIN_ROOT':'pins','ASSETS_ROOT':'assets','ASSET_STAGING_ROOT':'asset-staging',
           'PROOF_ROOT':'proofs','ADMIN_DSN_FILE':'secrets/admin.dsn','PGPASSFILE':'secrets/pgpass'}
    values.update({'TEST_C4_LIFECYCLE_'+key:ROOT+'/'+path for key,path in paths.items()})
    values.update(TEST_C4_LIFECYCLE_DATABASE=ident['database'],TEST_C4_LIFECYCLE_COMPOSE_PROJECT=ident['project'])
    if _full_context is not None:values.update(_full_context._consumer_environment(ident))
    return values


def consumer_command(container,binary,name,env,*,_full_context=None):
    allowed=set(consumer_env({'database':'','project':''},_full_context=_full_context))
    require(set(env)==allowed and not any('postgresql://' in value or '\n' in value for value in env.values()),'CONSUMER_ENV')
    require(HEX64.fullmatch(container) and re.fullmatch(r'/target/retained/[a-z0-9-]+',binary) and (name in (*PG_TESTS,*REGISTRY_TESTS,*SOURCE_ENDPOINT_TESTS,*DESTINATION_TESTS) if _full_context is None else name==_full_context._selected_test()),'CONSUMER_IDENTITY')
    return [DOCKER,'exec','--user','0:0',container,'/usr/bin/env','-i',*[key+'='+value for key,value in env.items()],binary,'--ignored','--exact',name,'--test-threads=1']


def legacy_env(ident):
    return {'PATH':'/usr/bin:/bin','HOME':'/root','LC_ALL':'C','TEST_C4_BINDING_DATABASE_NAME':ident['database'],
            'TEST_C4_BINDING_OTHER_DATABASE_NAME':ident['other_database'],'TEST_C4_BINDING_CONTROL_ROOT':ROOT+'/control',
            'TEST_C4_BINDING_ADMIN_DSN_FILE':ROOT+'/secrets/admin.dsn','TEST_C4_BINDING_OTHER_ADMIN_DSN_FILE':ROOT+'/secrets/other-admin.dsn'}


def equal_audit(before,after):
    require(before==after,'AUDIT_CHANGED')


def cleanup_verified(result):
    return result.get('preflight_complete') is True and result.get('resource_creation_unknown') is not True and result.get('registered_transports_reaped',True) is True and all(h.get('removed') is True for h in result.get('helpers',[])) and all(c.get('stopped') is True and c.get('execs_absent') is True and c.get('transports_reaped') is True and 'clone_cleanup_error' not in c for c in result.get('cases',[])) and all(c.get('stopped') is True and c.get('execs_absent') is True and c.get('retained_network_empty') is True for c in result.get('source_clones',[]))


def admit_resources(planned,existing):
    require(len(planned)==len(set(planned)) and not set(planned)&set(existing),'RESOURCE_ALREADY_EXISTS')


class MonitoredChild:
    """Drain both pipes continuously while caller services fresh nonce requests.

    Killing a Docker CLI is never treated as killing its in-container exec. Caller
    must stop and inspect that exact PG on timeout/transport ambiguity.
    """
    def __init__(self,command,logs,*,timeout=180,limit=LIMIT,env=None,interactive=False,clock=time.monotonic):
        self.log_reservation=LOG_RESERVATIONS.get(str(logs))
        self.prefix=logs/('child-'+uuid.uuid4().hex)
        self.finished=False;self.retired=False;self.retire_attempted=False;self.registry=self.log_reservation.get('ownership') if self.log_reservation else None
        with LOG_RESERVATION_LOCK:
            log_capacity(logs,4)
            if self.registry is not None:
                live=[c for c in self.registry.children if not c.finished and not c.retired]
                require(len(live)<19,'CHILD_COHORT_CAP')
                require(sum(ANCHOR in getattr(c,'command',[]) for c in live)<16 if ANCHOR in command else True,'ANCHOR_COHORT_CAP')
            if self.log_reservation is not None:self.log_reservation.setdefault('child_files',{})[self.prefix.name]={self.prefix.name+'.'+s for s in ('stdout','stderr','process.json','teardown.json')}
        self.log_identities={}
        self.command=command
        self.clock=clock
        self.limit,self.deadline,self.reason=limit,clock()+timeout,None
        self.chunks=[bytearray(),bytearray()]
        self.counts=[0,0]
        self.events=queue.Queue()
        try:
            self.p=subprocess.Popen(command,stdin=subprocess.PIPE if interactive else subprocess.DEVNULL,
                                    stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env or HOST_ENV,
                                    start_new_session=True,close_fds=True)
        except BaseException:
            if self.log_reservation is not None:self.log_reservation['child_files'].pop(self.prefix.name)
            raise
        if self.registry is not None:
            self.registry.children.append(self)
            for owner in self.registry.owners.values():
                if owner['intent']['kind']=='helper' and owner['id'] is not None and owner['id'] in command:
                    owner['child']=self
                    if 'object' in owner:owner['object'].child=self
        self.threads=[]
        for index,stream in enumerate((self.p.stdout,self.p.stderr)):
            thread=threading.Thread(target=self._drain,args=(index,stream),daemon=True)
            thread.start();self.threads.append(thread)

    def _drain(self,index,stream):
        path=self.prefix.with_suffix('.stdout' if index==0 else '.stderr')
        try:
            with open(path,'xb') as handle:
                meta=os.fstat(handle.fileno());self.log_identities[path.name]=(meta.st_dev,meta.st_ino)
                if os.name!='nt':os.chmod(path,0o600)
                while True:
                    raw=os.read(stream.fileno(),65536)
                    if not raw:break
                    self.counts[index]+=len(raw)
                    if self.counts[index]>self.limit:
                        self.reason='PROCESS_LOG_BUDGET';break
                    self.chunks[index].extend(raw);handle.write(raw)
                    self.events.put((index,raw))
                handle.flush();os.fsync(handle.fileno())
        except BaseException:
            self.reason='PROCESS_LOG_IO'

    def check(self):
        if self.reason:raise GateError(self.reason)
        if self.clock()>=self.deadline:
            self.reason='PROCESS_TIMEOUT';raise GateError(self.reason)

    def send(self,raw):
        self.check()
        require(self.p.poll() is None and self.p.stdin is not None,'CHILD_STDIN_CLOSED')
        self.p.stdin.write(raw);self.p.stdin.flush()

    def close_input(self):
        if self.p.stdin and not self.p.stdin.closed:self.p.stdin.close()

    def terminate(self):
        self.close_input()
        if self.p.poll() is None:
            if os.name=='nt':self.p.kill()
            else:os.killpg(self.p.pid,signal.SIGKILL)
        self.p.wait(timeout=5)

    def retire(self,timeout=TRANSPORT_TEARDOWN):
        """Independent short transport teardown; never an in-container proof."""
        require(timeout>0,'TRANSPORT_TEARDOWN_BUDGET')
        require(not self.retire_attempted and (not self.finished or self.reason is not None),'TRANSPORT_ALREADY_RETIRED');self.retire_attempted=True
        if self.registry is not None and self.registry.deadline is not None:timeout=min(timeout,self.registry.left())
        deadline=time.monotonic()+timeout;self.close_input();killed=False
        try:
            self.p.wait(timeout=max(0.01,timeout*0.5))
        except subprocess.TimeoutExpired:
            killed=True
            if os.name=='nt':self.p.kill()
            else:os.killpg(self.p.pid,signal.SIGKILL)
            self.p.wait(timeout=max(0.01,deadline-time.monotonic()))
        for thread in self.threads:thread.join(timeout=max(0,deadline-time.monotonic()))
        require(self.p.poll() is not None and not any(t.is_alive() for t in self.threads),'TRANSPORT_TEARDOWN_UNKNOWN')
        for stream in (self.p.stdout,self.p.stderr):stream.close()
        receipt=dict(reaped=True,exit_code=self.p.returncode,forced_cli_kill=killed,scope='CLI_ONLY_NOT_EXEC_PROOF')
        with open(self.prefix.with_suffix('.teardown.json'),'xb') as handle:
            meta=os.fstat(handle.fileno());self.log_identities[self.prefix.name+'.teardown.json']=(meta.st_dev,meta.st_ino);handle.write(canonical(receipt));handle.flush();os.fsync(handle.fileno())
        self.retired=True
        if self.log_reservation is not None:self.log_reservation['child_files'].pop(self.prefix.name,None)
        return receipt

    def finish(self,terminate_on_error=True):
        require(not self.finished and not self.retired,'TRANSPORT_ALREADY_FINISHED')
        try:
            while self.p.poll() is None:
                self.check();time.sleep(0.02)
            for thread in self.threads:thread.join(timeout=2)
            self.check()
            require(not any(thread.is_alive() for thread in self.threads),'PROCESS_PIPE_DRAIN')
            return self.p.returncode,bytes(self.chunks[0]),bytes(self.chunks[1])
        finally:
            if self.p.poll() is None and terminate_on_error:self.terminate()
            # Live callers leave a failed transport intact until the exact PG
            # has been stopped/inspected, then use independent short retire().
            if self.p.poll() is not None:
                for thread in self.threads:thread.join(timeout=2)
                for stream in (self.p.stdout,self.p.stderr):stream.close()
                if self.p.stdin and not self.p.stdin.closed:self.p.stdin.close()
                receipt=dict(exit_code=self.p.returncode,reason=self.reason,stdout_bytes=self.counts[0],stderr_bytes=self.counts[1])
                with open(self.prefix.with_suffix('.process.json'),'xb') as handle:
                    meta=os.fstat(handle.fileno());self.log_identities[self.prefix.name+'.process.json']=(meta.st_dev,meta.st_ino);handle.write(canonical(receipt));handle.flush();os.fsync(handle.fileno())
                self.finished=not any(thread.is_alive() for thread in self.threads)
                if self.finished and self.log_reservation is not None:self.log_reservation['child_files'][self.prefix.name]={self.prefix.name+'.teardown.json'} if self.reason else set()


def identity():
    case=str(uuid.uuid4());project='learning-system-p0c4-'+uuid.UUID(case).hex
    return dict(case_id=case,project=project,pg_name=project+'-pg-1',network=project+'_test',volume=project+'_pg',
                source_volume=project+'_source',build_volume=project+'_build',database='learning_backup_c4_task3_'+case,
                other_database='learning_backup_c4_task3_'+str(uuid.uuid4()),registry_volume=project+'_registry')


def private_read(path,max_bytes,uid=0,mode=0o600,allow_empty=False):
    fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW)
    with os.fdopen(fd,'rb') as handle:
        meta=os.fstat(handle.fileno())
        require(stat.S_ISREG(meta.st_mode) and meta.st_uid==uid and stat.S_IMODE(meta.st_mode)==mode and meta.st_nlink==1 and
                meta.st_size<=max_bytes and (allow_empty or meta.st_size>0),'PRIVATE_FILE_IDENTITY')
        raw=handle.read(max_bytes+1)
        after=os.fstat(handle.fileno())
        require(meta.st_ino==after.st_ino and meta.st_dev==after.st_dev and meta.st_size==after.st_size and
                len(raw)<=max_bytes,'PRIVATE_FILE_CHANGED')
    require(path.lstat().st_ino==meta.st_ino,'PRIVATE_FILE_REPLACED')
    return raw,meta


def sync_dir(path):
    fd=os.open(path,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:os.fsync(fd)
    finally:os.close(fd)


def new_private(path,raw,mode=0o600):
    fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,mode)
    with os.fdopen(fd,'wb') as handle:
        os.fchmod(handle.fileno(),mode);handle.write(raw);handle.flush();os.fsync(handle.fileno())
    sync_dir(path.parent)


def validate_fault_audit_request(baseline,message,case_id,test_exit_seen):
    require(type(baseline) is dict and test_exit_seen is True and set(message)=={'op','case_id','test','exit_code','stdout','stderr','binaries'} and message['op']=='audit-incomplete-publication','BROKER_FAULT_AUDIT')
    require(message['case_id']==case_id==baseline['issuer']['case_id'] and message['test']==baseline['test'] and message['test'] in REGISTRY_INCOMPLETE_TESTS,'BROKER_FAULT_CASE')
    v4(case_id);require(type(message['exit_code']) is int and type(message['stdout']) is str and len(message['stdout'].encode())<=4096 and message['stderr']=='','BROKER_FAULT_BODY')
    return parse_test(message['exit_code'],message['stdout'],b'',message['test'])


def read_fault_baseline():
    try:from p0c4_storage_registry import FAULT_BASELINE_NAME,parse_record
    except ModuleNotFoundError:from scripts.p0c4_storage_registry import FAULT_BASELINE_NAME,parse_record
    raw,_=private_read(Path(ROOT)/'proofs'/FAULT_BASELINE_NAME,65536)
    return parse_record(raw)


def final_fault_registry_audit(expected):
    try:from p0c4_storage_registry import audit_incomplete_publication
    except ModuleNotFoundError:from scripts.p0c4_storage_registry import audit_incomplete_publication
    observed=audit_incomplete_publication(read_fault_baseline())
    require(canonical(observed)==canonical(expected),'FINAL_REGISTRY_UNUSABLE_CHANGED')
    return observed


def completed_case_audit(broker,record,binaries,code,out,err):
    name=record['test']
    if name in REGISTRY_INCOMPLETE_TESTS:
        require(canonical(record['outcome'])==canonical(parse_test(code,out.decode(),err,name)),'FAULT_BODY_READBACK')
        audited=broker.call(dict(op='audit-incomplete-publication',case_id=record['identity']['case_id'],test=name,exit_code=code,stdout=out.decode(),stderr=err.decode(),binaries=binaries))
        require(set(audited)=={'audit','registry_unusable'},'FAULT_AUDIT_RESULT')
        record['audit_after']=audited['audit'];record['registry_unusable_after']=audited['registry_unusable']
    else:record['audit_after']=broker.call(dict(op='audit',binaries=binaries))


def atomic_private(path,raw):
    temp=path.with_name('.driver-'+uuid.uuid4().hex)
    new_private(temp,raw)
    os.replace(temp,path);sync_dir(path.parent)


# Read-only reviewed Python executes only in the pinned network-none root helper.
BROKER = r'''
import sys
sys.path.insert(0,'/reviewed/scripts')
from p0c4_source_lifecycle_gate_acceptance import broker_main
broker_main()
'''

def source_fault_request_limit(test):
    return 6 if test=='source::endpoint_tests::source_dump_timeout_cancel_and_overflow_reap_child_keep_gate_closed' else 4

def validate_source_fault_request(row,backup,claim):
    require(type(row) is dict and set(row)=={'format_version','backup_id','epoch','nonce','phase','backend_pid','database_oid','challenge_keys','postmaster_start_ticks'},'SOURCE_FAULT_REQUEST_FIELDS')
    require(type(row['format_version']) is int and row['format_version']==1 and row['backup_id']==backup and row['epoch']==claim['epoch'] and row['phase'] in ('pause','resume'),'SOURCE_FAULT_REQUEST_SCOPE')
    for key in ('backup_id','epoch','nonce'):v4(row[key])
    require(type(row['backend_pid']) is int and 0<row['backend_pid']<2**31 and type(row['database_oid']) is int and 0<row['database_oid']<2**32 and type(row['postmaster_start_ticks']) is int and row['postmaster_start_ticks']==claim['native']['postmaster_start_ticks'],'SOURCE_FAULT_REQUEST_IDENTITY')
    keys=row['challenge_keys'];require(type(keys) is list and len(keys)==2 and all(type(k) is int and -(2**63)<=k<2**63 for k in keys) and keys[0]!=keys[1],'SOURCE_FAULT_REQUEST_KEYS')


def broker_main():
    require(os.geteuid()==0,'BROKER_ROOT')
    os.umask(0o077)
    root=Path(ROOT)
    for path in (Path('/'),Path('/var'),Path('/var/lib'),root):
        meta=path.lstat()
        require(stat.S_ISDIR(meta.st_mode) and meta.st_uid==0 and not meta.st_mode&0o022,'BROKER_ANCESTOR')
    ident=None;ledger=None;last=None;partial={};root_pins={};control_fd=None;pin_fd=None;approved_requests={};observation_counts={};fault_baseline=None;fault_test_exit=False;fault_audited=False
    clone_setup=None;clone_observation=None;clone_observation_done=False;clone_refresh_seen=False
    source_fault_test=None;source_fault_seen={};source_fault_pending=None;source_fault_paused=None
    while True:
        line=sys.stdin.buffer.readline(1024**2+1)
        if not line:break
        require(len(line)<=1024**2 and line.endswith(b'\n'),'BROKER_FRAME_BUDGET')
        message=json.loads(line,object_pairs_hook=unique_pairs)
        require(type(message) is dict and message.get('op') in ('init','poll','refresh','denial','observe','audit','audit-incomplete-publication','inventory','stop','source-clone-setup','source-clone-poll','source-clone-observed','source-clone-result','source-fault-enable','source-fault-poll','source-fault-observed'),'BROKER_OPERATION')
        operation=message['op']
        if operation=='init':
            require(set(message)=={'op','identity','sql','admin_password','registry_case'} and ident is None and not list(root.iterdir()),'ISSUER_FRESH_ROOT')
            ident=message['identity'];v4(ident['case_id'])
            require(ident['project'] in ('learning-system-p0c4-'+uuid.UUID(ident['case_id']).hex,'kwc4c-'+uuid.UUID(ident['case_id']).hex) and ident['database']=='learning_backup_c4_task3_'+ident['case_id'],'ISSUER_CASE_IDENTITY')
            password=message['admin_password']
            require(re.fullmatch('[0-9a-f]{64}',password),'ISSUER_SECRET_FORMAT')
            root.chmod(0o700)
            for name in ('control','pins','assets','asset-staging','proofs','secrets','regressions'):
                (root/name).mkdir(mode=0o700);sync_dir(root)
                meta=(root/name).lstat();root_pins[name]=(meta.st_dev,meta.st_ino)
            control_fd=os.open(root/'control',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
            pin_fd=os.open(root/'pins',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
            sql=message['sql'];meta=os.fstat(control_fd)
            require(sql['database']==ident['database'] and type(sql['database_oid']) is int and 0<sql['database_oid']<2**32 and
                    re.fullmatch('[1-9][0-9]*',sql['system_identifier']) and int(sql['system_identifier'])<2**64,'ISSUER_SQL')
            binding=dict(format_version=1,capability='source_control_binding_v1',binding_id=str(uuid.uuid4()),control_path=ROOT+'/control',
                         control_dev=meta.st_dev,control_ino=meta.st_ino,**sql)
            raw=canonical(binding);new_private(root/'control/source-binding.json',raw)
            from p0c4_storage_registry import REGISTRY_PATH, _root_helper_context, provision_registry
            registry_path=Path(REGISTRY_PATH);registry_path.chmod(0o700)
            registry_fd=os.open(registry_path,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
            asset_fd=os.open(root/'assets',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
            stage_fd=os.open(root/'asset-staging',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
            try:
                require(message['registry_case']['case_id']==ident['case_id'],'REGISTRY_CASE_OWNERSHIP')
                context=_root_helper_context(accepted_case=message['registry_case'],source_binding_bytes=raw,source_database_facts=sql,
                    source_roots={'control':(control_fd,str(root/'control')),'assets':(asset_fd,str(root/'assets')),'staging':(stage_fd,str(root/'asset-staging')),'local_pins':(pin_fd,str(root/'pins'))},registry_root_handle=registry_fd)
                registry_receipt=provision_registry(context).record
                new_private(root/'proofs/registry-provisioning.json',canonical(registry_receipt))
            finally:
                for fd in (registry_fd,asset_fd,stage_fd):os.close(fd)
            for db,name in ((ident['database'],'admin.dsn'),(ident['other_database'],'other-admin.dsn')):
                new_private(root/'secrets'/name,f'postgresql://learning_admin:{password}@localhost:5432/{db}?sslmode=disable'.encode())
            new_private(root/'secrets/pgpass',f'localhost:5432:{ident["database"]}:learning_admin:{password}\n'.encode())
            ledger=GenerationLedger(ident)
            answer=dict(binding=binding,binding_sha256=digest(raw),sql=sql,issuer_euid=0,classification='INDEPENDENT_PRECOMPILE_BINDING',registry_provisioning=registry_receipt)
        elif operation=='poll':
            require(set(message)=={'op'} and ident is not None,'BROKER_POLL')
            found=[]
            for path in sorted((root/'proofs').glob('driver-request-*.json')):
                if path.name in partial and partial[path.name].get('accepted'):continue
                raw,meta=private_read(path,4096,allow_empty=True)
                if incomplete_publication(raw):
                    old=partial.setdefault(path.name,dict(first=time.monotonic()))
                    require(time.monotonic()-old['first']<2,'REQUEST_PARTIAL_TIMEOUT');continue
                # Two stable reads: do not acknowledge partially published requests.
                raw2,meta2=private_read(path,4096,allow_empty=True)
                if raw2!=raw or meta2.st_ino!=meta.st_ino:continue
                candidate=json.loads(raw,object_pairs_hook=unique_pairs)
                clone_request=clone_setup is not None and candidate.get('compose_project')==clone_setup['clone_project']
                if clone_request:
                    require(not clone_refresh_seen,'SOURCE_CLONE_REFRESH_REUSED')
                    ledger.ident=dict(ident,project=clone_setup['clone_project'])
                try:row=ledger.admit(raw,path.name)
                finally:ledger.ident=ident
                if clone_request:clone_refresh_seen=True
                partial[path.name]=dict(accepted=True)
                found.append(dict(request=row,request_sha256=digest(raw),lock_new=not (root/'proofs'/('isolation-'+row['backup_id']+'.lock')).exists()))
            require(len(found)<=1,'REQUEST_CONCURRENT')
            if found:
                row=found[0]['request'];backup=row['backup_id']
                if last:
                    archive=root/'proofs/generations'/last['request_id']
                    require(archive.is_dir(),'GENERATION_ARCHIVE')
                    for prefix in ('inspection-','isolation-','runtime-connect-denied-'):
                        path=root/'proofs'/(prefix+last['backup_id']+'.json')
                        if path.exists():
                            raw,_=private_read(path,1024**2)
                            new_private(archive/path.name,raw);path.unlink();sync_dir(path.parent)
                generations=root/'proofs/generations'
                generations.mkdir(mode=0o700,exist_ok=True)
                archive=generations/row['request_id'];archive.mkdir(mode=0o700);sync_dir(generations)
                new_private(archive/'request.json',canonical(row))
                lock=root/'proofs'/('isolation-'+backup+'.lock')
                if found[0]['lock_new']:new_private(lock,b'lock')
                else:private_read(lock,1024)
                last=row
                approved_requests[row['request_id']]=row
            answer=found
        elif operation=='refresh':
            require(set(message)=={'op','nonce','inspection','anchor','observations'} and last is not None,'BROKER_REFRESH')
            ledger.require_active(message['nonce']);anchor=validate_anchor(message['anchor'])
            inspection=message['inspection'];raw=canonical(inspection)
            require(len(raw)<=1024**2 and inspection['network']['project']==last['compose_project'] and inspection['network_internal'] is True and
                    all(inspection[key]==0 for key in ('runtime_running','worker_running','other_admin_processes','postgres_published_ports','manager_runtime_secret_mounts','manager_runtime_env_keys')),'BROKER_INSPECTION')
            inspection_name='inspection-'+last['backup_id']+'.json'
            atomic_private(root/'proofs'/inspection_name,raw)
            proof=dict(backup_id=last['backup_id'],compose_project=last['compose_project'],database=ident['database'],format_version=2,
                       docker_inspection_sha256=digest(raw),driver_pid=anchor['pid'],inspection_file=inspection_name,observed_unix_ms=int(time.time()*1000),
                       **{key:inspection[key] for key in ('runtime_running','worker_running','other_admin_processes','postgres_published_ports','network_internal','manager_runtime_secret_mounts','manager_runtime_env_keys')})
            atomic_private(root/'proofs'/('isolation-'+last['backup_id']+'.json'),canonical(proof))
            archive=root/'proofs/generations'/last['request_id']
            new_private(archive/'host-observations.json',canonical(message['observations']))
            new_private(archive/'anchor.json',canonical(anchor))
            ack=dict(backup_id=last['backup_id'],result='fresh_isolation_ready')
            new_private(root/'proofs'/('driver-ready-'+last['request_id']+'.json'),canonical(ack))
            new_private(archive/'ack.json',canonical(ack))
            answer=dict(nonce=last['request_id'],proof_sha256=digest(canonical(proof)),inspection_sha256=digest(raw),ack_sha256=digest(canonical(ack)))
        elif operation=='source-fault-enable':
            require(set(message)=={'op','test'} and ident is not None and not approved_requests and source_fault_test is None and message['test'] in SOURCE_FAULT_TESTS,'SOURCE_FAULT_ENABLE')
            source_fault_test=message['test'];answer=dict(accepted=True)
        elif operation=='source-fault-poll':
            require(set(message)=={'op'} and source_fault_test is not None,'SOURCE_FAULT_POLL_SCOPE')
            paths=sorted((root/'proofs').glob('source-fault-request-*.json'));require(len(paths)<=source_fault_request_limit(source_fault_test),'SOURCE_FAULT_REQUEST_COUNT')
            answer=[]
            for path in paths:
                if path.name in source_fault_seen:continue
                raw,_=private_read(path,4096,allow_empty=True)
                if incomplete_publication(raw):continue
                raw2,_=private_read(path,4096,allow_empty=True)
                if raw2!=raw:continue
                require(last is not None,'SOURCE_FAULT_NO_EPOCH')
                inspection,_=private_read(root/'proofs'/('inspection-'+last['backup_id']+'.json'),1024**2)
                claim=json.loads(inspection,object_pairs_hook=unique_pairs)['source_endpoint']
                row=json.loads(raw,object_pairs_hook=unique_pairs);validate_source_fault_request(row,last['backup_id'],claim)
                require(canonical(row)==raw and path.name=='source-fault-request-'+row['nonce']+'.json','SOURCE_FAULT_CANONICAL')
                require((row['phase']=='pause' and source_fault_paused is None) or (row['phase']=='resume' and source_fault_paused is not None and all(row[k]==source_fault_paused[k] for k in ('backup_id','epoch','backend_pid','database_oid','challenge_keys','postmaster_start_ticks'))),'SOURCE_FAULT_PHASE')
                if source_fault_pending is not None:require(source_fault_pending==row,'SOURCE_FAULT_CONCURRENT')
                source_fault_pending=row;answer.append(dict(request=row,claim=claim))
            require(len(answer)<=1,'SOURCE_FAULT_CONCURRENT')
        elif operation=='source-fault-observed':
            require(set(message)=={'op','response'} and source_fault_pending is not None,'SOURCE_FAULT_ACK_ORDER')
            row=source_fault_pending;response=message['response']
            require(type(response) is dict and set(response)=={'backup_id','epoch','nonce','phase','postmaster_start_ticks','observed_state'} and all(response[k]==row[k] for k in ('backup_id','epoch','nonce','phase','postmaster_start_ticks')) and (response['observed_state']=='T' if row['phase']=='pause' else response['observed_state'] in ('R','S','D','I')),'SOURCE_FAULT_ACK_IDENTITY')
            new_private(root/'proofs'/('source-fault-ready-'+row['nonce']+'.json'),canonical(response))
            source_fault_seen['source-fault-request-'+row['nonce']+'.json']=row
            source_fault_paused=row if row['phase']=='pause' else None
            source_fault_pending=None;answer=dict(accepted=True)
        elif operation=='source-clone-setup':
            require(set(message)=={'op','setup'} and ident is not None and clone_setup is None and not approved_requests,'SOURCE_CLONE_SETUP_ORDER')
            setup=message['setup']
            require(type(setup) is dict and set(setup)=={'physical_backup_verified','original_container_id','clone_container_id','clone_project','database'} and setup['physical_backup_verified'] is True and setup['database']==ident['database'] and
                    all(type(setup[k]) is str and HEX64.fullmatch(setup[k]) for k in ('original_container_id','clone_container_id')) and setup['original_container_id']!=setup['clone_container_id'] and
                    re.fullmatch(r'kwc4c-[0-9a-f]{32}',setup['clone_project']) and setup['clone_project']!=ident['project'],'SOURCE_CLONE_SETUP_IDENTITY')
            clone_setup=setup;new_private(root/'proofs/source-clone-setup.json',canonical(setup));answer=dict(accepted=True)
        elif operation=='source-clone-poll':
            require(set(message)=={'op'} and clone_setup is not None,'SOURCE_CLONE_POLL_SCOPE')
            paths=sorted((root/'proofs').glob('source-endpoint-observe-*.json'));require(len(paths)<=1,'SOURCE_CLONE_ONE_REQUEST')
            answer=[]
            if paths and not clone_observation_done:
                raw,_=private_read(paths[0],4096,allow_empty=True)
                if not incomplete_publication(raw):
                    row=json.loads(raw,object_pairs_hook=unique_pairs)
                    require(canonical(row)==raw and set(row)=={'format_version','backup_id','nonce','backend_pid','database_oid','system_identifier','challenge_keys'} and type(row['format_version']) is int and row['format_version']==1 and last is not None and row['backup_id']==last['backup_id'] and last['compose_project']==clone_setup['clone_project'],'SOURCE_CLONE_OBSERVE_SCOPE')
                    v4(row['nonce']);require(paths[0].name=='source-endpoint-observe-'+row['nonce']+'.json' and type(row['backend_pid']) is int and 0<row['backend_pid']<2**31 and type(row['database_oid']) is int and 0<row['database_oid']<2**32 and type(row['system_identifier']) is str and re.fullmatch('[1-9][0-9]*',row['system_identifier']),'SOURCE_CLONE_OBSERVE_IDENTITY')
                    keys=row['challenge_keys'];require(type(keys) is list and len(keys)==2 and all(type(k) is int and -(2**63)<=k<2**63 for k in keys) and keys[0]!=keys[1],'SOURCE_CLONE_OBSERVE_KEYS')
                    if clone_observation is not None:require(clone_observation==row,'SOURCE_CLONE_OBSERVE_CHANGED')
                    clone_observation=row;answer=[row]
        elif operation=='source-clone-observed':
            require(set(message)=={'op','response'} and clone_observation is not None and not clone_observation_done,'SOURCE_CLONE_RESPONSE_ORDER')
            response=message['response'];require(type(response) is dict and len(canonical(response))<=16384 and response.get('nonce')==clone_observation['nonce'] and response.get('backup_id')==clone_observation['backup_id'] and response.get('clone_container_id')==clone_setup['clone_container_id'] and response.get('original_container_id')==clone_setup['original_container_id'],'SOURCE_CLONE_RESPONSE_IDENTITY')
            new_private(root/'proofs'/('source-endpoint-observed-'+response['nonce']+'.json'),canonical(response));clone_observation_done=True;answer=dict(accepted=True)
        elif operation=='source-clone-result':
            require(set(message)=={'op'} and clone_observation_done,'SOURCE_CLONE_RESULT_ORDER')
            raw,_=private_read(root/'proofs/source-clone-prewrite-result.json',4096)
            answer=json.loads(raw,object_pairs_hook=unique_pairs)
            require(canonical(answer)==raw and answer.get('backup_id')==clone_observation['backup_id'] and answer.get('actual_admitted_clone_observation') is True,'SOURCE_CLONE_RESULT_SCOPE')
        elif operation=='denial':
            require(set(message)=={'op','nonce','exit_code','stderr'} and last is not None,'BROKER_DENIAL')
            ledger.require_active(message['nonce'])
            validate_denial(message['exit_code'],message['stderr'].encode(),ident['database'])
            proof=dict(backup_id=last['backup_id'],database=ident['database'],result='runtime_connect_denied')
            path=root/'proofs'/('runtime-connect-denied-'+last['backup_id']+'.json')
            require(not path.exists(),'DENIAL_REUSED')
            new_private(path,canonical(proof))
            new_private(root/'proofs/generations'/last['request_id']/'runtime-probe.json',canonical(message))
            answer=dict(nonce=last['request_id'],denial_sha256=digest(canonical(proof)))
        elif operation=='observe':
            require(set(message)=={'op','nonce','boundary','acl'} and message['nonce'] in approved_requests and
                    message['boundary'] in ('before_ack','acl_open','acl_closed','before_owned_commit','next_request','test_exit') and message['acl'] in ('t','f'),'BROKER_STATE_OBSERVATION')
            row=approved_requests[message['nonce']];count=observation_counts.get(message['nonce'],0)
            require(count<8,'GENERATION_OBSERVATION_COUNT')
            state=attempt_snapshot(control_fd,pin_fd,row['backup_id'],digest(canonical(binding)))
            answer=dict(nonce=message['nonce'],backup_id=row['backup_id'],boundary=message['boundary'],acl=message['acl'],observed_unix_ms=int(time.time()*1000),attempt_state=state)
            new_private(root/'proofs/generations'/message['nonce']/f'boundary-{count:02d}.json',canonical(answer))
            observation_counts[message['nonce']]=count+1
            if message['boundary']=='test_exit':fault_test_exit=True
        elif operation in ('audit','audit-incomplete-publication'):
            require(ident is not None,'BROKER_AUDIT')
            if operation=='audit':
                require(set(message) in ({'op','binaries'},{'op','binaries','registry_fault_test'}),'BROKER_AUDIT')
                if 'registry_fault_test' in message:
                    require(message['registry_fault_test'] in REGISTRY_INCOMPLETE_TESTS and fault_baseline is None and not approved_requests,'BROKER_FAULT_BASELINE_ORDER')
            else:
                require(not fault_audited,'BROKER_FAULT_AUDIT_REUSED')
                validate_fault_audit_request(fault_baseline,message,ident['case_id'],fault_test_exit)
            # The held original root is authoritative even when case 8 renames it.
            meta=os.fstat(control_fd)
            fd=os.open('source-binding.json',os.O_RDONLY|os.O_NOFOLLOW,dir_fd=control_fd)
            with os.fdopen(fd,'rb') as handle:
                m=os.fstat(handle.fileno());require(m.st_uid==0 and stat.S_IMODE(m.st_mode)==0o600 and m.st_nlink==1 and 0<m.st_size<=4096,'BINDING_AUDIT')
                raw=handle.read(4097)
            roots={}
            for name,pin in root_pins.items():
                candidates=[root/name]
                if name in ('control','pins'):candidates.append(root/(name+'-held'))
                matches=[p for p in candidates if p.exists() and (p.lstat().st_dev,p.lstat().st_ino)==pin]
                require(len(matches)==1,'HELD_ROOT_IDENTITY')
                roots[name]=dict(path=str(matches[0]),dev=pin[0],ino=pin[1])
            binaries={}
            for path in message['binaries']:
                require(re.fullmatch('/target/retained/[a-z0-9-]+',path),'BINARY_PATH')
                value,_=private_read(Path(path),128*1024**2,mode=0o500)
                binaries[path]=digest(value)
            from p0c4_storage_registry import validate_installed_registry,capture_fault_audit_baseline,audit_incomplete_publication,FAULT_BASELINE_NAME
            if operation=='audit':
                if 'registry_fault_test' in message:
                    fault_baseline=capture_fault_audit_baseline(registry_receipt,message['registry_fault_test'])
                    baseline_raw=canonical(fault_baseline);require(len(baseline_raw)<=65536,'BROKER_FAULT_BASELINE_BYTES')
                    new_private(root/'proofs'/FAULT_BASELINE_NAME,baseline_raw)
                    require(canonical(read_fault_baseline())==baseline_raw,'BROKER_FAULT_BASELINE_READBACK')
                else:
                    registry_observation=validate_installed_registry()
                    require(registry_observation.hashes['authority.json']==registry_receipt['authority_sha256'],'REGISTRY_AUTHORITY_AFTER_BUILD')
            else:
                require(canonical(read_fault_baseline())==canonical(fault_baseline),'BROKER_FAULT_BASELINE_CHANGED')
                unusable=audit_incomplete_publication(fault_baseline);fault_audited=True
            answer=dict(binding_sha256=digest(raw),control_dev=meta.st_dev,control_ino=meta.st_ino,roots=roots,binaries=binaries)
            if operation=='audit-incomplete-publication':answer=dict(audit=answer,registry_unusable=unusable)
        elif operation=='inventory':
            require(set(message)=={'op'} and ident is not None,'BROKER_INVENTORY')
            answer=bounded_inventory(root)
        else:
            require(set(message)=={'op'},'BROKER_STOP')
            if control_fd is not None:os.close(control_fd)
            if pin_fd is not None:os.close(pin_fd)
            print(canonical(dict(stopped=True)).decode(),flush=True);break
        print(canonical(answer).decode(),flush=True)


def bounded_inventory(root):
    entries={};total=0
    for path in sorted(root.rglob('*')):
        require(len(entries)<4096,'INVENTORY_COUNT')
        meta=path.lstat();relative=path.relative_to(root).as_posix()
        fs_evidence=re.fullmatch(r'fs-[0-9a-f-]{36}',relative.split('/')[0]) is not None
        if fs_evidence and (stat.S_ISLNK(meta.st_mode) or stat.S_ISFIFO(meta.st_mode)):
            entries[relative]=dict(type='symlink' if stat.S_ISLNK(meta.st_mode) else 'fifo',mode=stat.S_IMODE(meta.st_mode),uid=meta.st_uid)
            if stat.S_ISLNK(meta.st_mode):entries[relative]['target']=os.readlink(path)
            continue
        require(stat.S_ISDIR(meta.st_mode) or stat.S_ISREG(meta.st_mode),'INVENTORY_SPECIAL')
        if path.parts[-2:-1]==('secrets',):continue
        if stat.S_ISREG(meta.st_mode):
            require((fs_evidence or meta.st_nlink==1) and meta.st_size<=128*1024**2,'INVENTORY_FILE_BUDGET')
            total+=meta.st_size;require(total<=512*1024**2,'INVENTORY_BYTE_BUDGET')
            raw=path.read_bytes()
            entries[relative]=dict(bytes=len(raw),sha256=digest(raw),mode=stat.S_IMODE(meta.st_mode),uid=meta.st_uid,links=meta.st_nlink)
            if path.name in ('manifest.json','asset-index.json') and len(raw)<=1024**2:
                # Preserve malformed retained negative evidence as bytes too.
                entries[relative]['raw_hex']=raw.hex()
        else:entries[relative]=dict(directory=True,dev=meta.st_dev,ino=meta.st_ino,mode=stat.S_IMODE(meta.st_mode),uid=meta.st_uid)
    return entries


def destination_source_inventory(inventory):
    """Fixed destination-only clients must leave the four fresh source roots exact."""
    roots=('control','pins','assets','asset-staging')
    require(type(inventory) is dict,'DESTINATION_SOURCE_INVENTORY')
    selected={path:value for path,value in inventory.items() if any(path==root or path.startswith(root+'/') for root in roots)}
    require(set(selected)==set(roots)|{'control/source-binding.json'},'DESTINATION_SOURCE_FRESH_INVENTORY')
    for root in roots:
        row=selected[root]
        require(type(row) is dict and set(row)=={'directory','dev','ino','mode','uid'} and row['directory'] is True and
                all(type(row[k]) is int and row[k]>0 for k in ('dev','ino')) and type(row['uid']) is int and row['uid']==0 and type(row['mode']) is int and row['mode']==0o700,'DESTINATION_SOURCE_ROOT')
    row=selected['control/source-binding.json']
    require(type(row) is dict and set(row)=={'bytes','sha256','mode','uid','links'} and type(row['bytes']) is int and 0<row['bytes']<=4096 and
            type(row['sha256']) is str and HEX64.fullmatch(row['sha256']) and all(type(row[k]) is int for k in ('mode','uid','links')) and
            (row['mode'],row['uid'],row['links'])==(0o600,0,1),'DESTINATION_SOURCE_BINDING')
    return selected


def parse_pg_root(output):
    lines=output.splitlines()
    require(len(lines)==3,'PG_ROOT_AUDIT_SHAPE')
    try:
        dev,ino,uid,mode=lines[0].split();fuid,fmode,links,size=lines[1].split()
        require(uid==fuid=='0' and mode=='700' and fmode=='600' and links=='1' and
                int(dev)>0 and int(ino)>0 and 0<int(size)<=4096 and HEX64.fullmatch(lines[2]),'PG_ROOT_AUDIT_IDENTITY')
    except (ValueError,TypeError):raise GateError('PG_ROOT_AUDIT_SHAPE') from None
    return dict(control_dev=int(dev),control_ino=int(ino),binding_sha256=lines[2])


def validate_cohort(rows,pins):
    seen=set()
    for row in rows:
        pid=row['pid'];require(pid not in seen and type(pid) is int and pid>0,'PROCESS_COHORT_PID');seen.add(pid)
        if row['uid']==999 and row['exe']=='/usr/lib/postgresql/18/bin/postgres':continue
        pin=pins.get(pid)
        require(row['uid']==0 and pin is not None and row['starttime']==pin['starttime'] and row['exe']==pin['exe'] and
                sorted(row['env_keys'])==sorted(pin['env_keys']),'PROCESS_COHORT_UNAPPROVED')
    require(set(pins)<=seen,'PROCESS_COHORT_MISSING')


def validate_dump_process(row,consumer_pid):
    require(row['exe']=='/usr/lib/postgresql/18/bin/pg_dump' and row['uid']==0 and row['ppid']==consumer_pid and
            set(row['env_keys']) in ({'LC_ALL'},{'LC_ALL','PGCONNECT_TIMEOUT'},{'LC_ALL','PGPASSFILE','PGAPPNAME','PGCONNECT_TIMEOUT'}),'PGDUMP_COHORT')


def validate_index(raw,rows,fixture_counts=True):
    try:index=json.loads(raw,object_pairs_hook=unique_pairs)
    except (ValueError,UnicodeDecodeError):raise GateError('INDEX_JSON') from None
    ordered=[{key:row[key] for key in ('space_id','id','sha256','byte_size','storage_key')} for row in rows]
    expected=json.dumps(dict(format_version=1,assets=ordered),ensure_ascii=False,separators=(',',':')).encode()
    require(type(index) is dict and set(index)=={'format_version','assets'} and index['format_version']==1 and index['assets']==ordered,'INDEX_DB_ROWS')
    require(raw==expected,'INDEX_CANONICAL')
    if fixture_counts:
        sizes={row['sha256']:row['byte_size'] for row in ordered}
        require(len(ordered)==4 and len({row['space_id'] for row in ordered})==2 and len(sizes)==3 and sum(sizes.values())==96,'FIXTURE_COUNTS')
    return dict(index_sha256=digest(raw),logical_rows=len(rows),spaces=len({row['space_id'] for row in rows}),objects=len({row['sha256'] for row in rows}))


PG_ROOT_AUDIT = r'''
set -eu
p="$1"
test -d "$p" && test ! -L "$p"
test -f "$p/source-binding.json" && test ! -L "$p/source-binding.json"
stat -c '%d %i %u %a' "$p"
stat -c '%u %a %h %s' "$p/source-binding.json"
sha256sum "$p/source-binding.json" | cut -d ' ' -f 1
'''

ANCHOR = r'''
set -eu
exec 9<>"$1"
/usr/bin/flock -n -E 73 9
printf 'ANCHOR %s\n' "$$"
while IFS= read -r command; do test "$command" = keep; done
'''

ANCHOR_AUDIT = r'''
set -eu
p="$1"; lock="$2"
test -d "/proc/$p"
printf '%s\n' "$p"
cat "/proc/$p/stat"
awk '/^Uid:/{print $2,$3,$4,$5}' "/proc/$p/status"
stat -Lc '%d %i' "/proc/$p/fd/9"
stat -c '%d %i' "$lock"
readlink "/proc/$p/ns/pid"
set +e
/usr/bin/flock -n -E 73 "$lock" /bin/true
e=$?
set -e
printf '%s\n' "$e"
'''

PG_NAMESPACE_SAMPLER = r'''
set -eu
printf 'PG_PID1_NAMESPACE_V1\n%s\n' "$$"
cat "/proc/$$/stat"
awk '/^Uid:/{print $2,$3,$4,$5}' "/proc/$$/status"
awk '/^CapEff:/{print $2}' "/proc/$$/status"
awk '/^CapPrm:/{print $2}' "/proc/$$/status"
awk '/^CapAmb:/{print $2}' "/proc/$$/status"
awk '/^NoNewPrivs:/{print $2}' "/proc/$$/status"
cat /proc/1/stat
awk '/^Uid:/{print $2,$3,$4,$5}' /proc/1/status
readlink /proc/1/exe
readlink /proc/1/ns/pid
cat /proc/1/stat
awk '/^Uid:/{print $2,$3,$4,$5}' /proc/1/status
readlink /proc/1/exe
'''
PG_NAMESPACE_SAMPLER_SHA256=digest(PG_NAMESPACE_SAMPLER.encode())
PG_RUNNING_FORMAT='{"id":{{json .Id}},"image":{{json .Image}},"running":{{json .State.Running}},"status":{{json .State.Status}},"pid":{{json .State.Pid}},"started_at":{{json .State.StartedAt}},"restart_count":{{json .RestartCount}},"dead":{{json .State.Dead}},"oom_killed":{{json .State.OOMKilled}},"error":{{json .State.Error}}}'


def namespace_lines(raw,count):
    require(type(raw) is bytes and 0<len(raw)<=4096 and raw.endswith(b'\n') and b'\r' not in raw,'NAMESPACE_FRAMING')
    try:lines=raw.decode('ascii').split('\n')[:-1]
    except UnicodeDecodeError:raise GateError('NAMESPACE_ENCODING') from None
    require(len(lines)==count and all(lines),'NAMESPACE_FRAMING');return lines


def namespace_stat(value,pid,comm=None):
    match=re.fullmatch(r'([1-9][0-9]*) \((.*)\) ([A-Za-z]) (.*)',value)
    require(match is not None and int(match[1])==pid and (comm is None or match[2]==comm) and
            match[3] not in ('Z','X','x'),'NAMESPACE_STAT_IDENTITY')
    tail=match[4].split(' ')
    require(len(tail)>=19 and all(re.fullmatch(r'-?[0-9]+',v) for v in tail) and
            re.fullmatch(r'[1-9][0-9]*',tail[18]),'NAMESPACE_STAT_STARTTIME')
    return int(tail[18])


def parse_namespace_sampler(raw):
    rows=namespace_lines(raw,15)
    require(rows[0]=='PG_PID1_NAMESPACE_V1' and re.fullmatch(r'[1-9][0-9]*',rows[1]) and int(rows[1])>1,'NAMESPACE_SAMPLER_PID')
    namespace_stat(rows[2],int(rows[1]))
    require(rows[3]=='999 999 999 999' and rows[4:7]==['0000000000000000']*3 and rows[7]=='1','NAMESPACE_SAMPLER_AUTHORITY')
    start=namespace_stat(rows[8],1,'postgres');end=namespace_stat(rows[12],1,'postgres')
    require(start==end and rows[9]==rows[13]=='999 999 999 999' and
            rows[10]==rows[14]=='/usr/lib/postgresql/18/bin/postgres','NAMESPACE_PID1_IDENTITY')
    require(re.fullmatch(r'pid:\[[1-9][0-9]*\]',rows[11]),'NAMESPACE_PID1_VALUE')
    return dict(pid=1,starttime=start,uids=[999]*4,exe=rows[10],pid_ns=rows[11])


def namespace_command(container):
    require(type(container) is str and HEX64.fullmatch(container),'NAMESPACE_CONTAINER_ID')
    return [DOCKER,'exec','--user','999:999',container,'/usr/bin/env','-i','PATH=/usr/bin:/bin','LC_ALL=C','/bin/sh','-ec',PG_NAMESPACE_SAMPLER]


def assemble_anchor(raw,sampler):
    rows=namespace_lines(raw,7)
    require(re.fullmatch(r'[1-9][0-9]*',rows[0]) and rows[2]=='0 0 0 0' and
            all(re.fullmatch(r'[1-9][0-9]* [1-9][0-9]*',rows[n]) for n in (3,4)) and
            re.fullmatch(r'pid:\[[1-9][0-9]*\]',rows[5]) and rows[6]=='73','NAMESPACE_ROOT_IDENTITY')
    pid=int(rows[0]);start=namespace_stat(rows[1],pid)
    row=dict(pid=pid,starttime=start,uids=[0]*4,fd_dev=int(rows[3].split()[0]),fd_ino=int(rows[3].split()[1]),
             lock_dev=int(rows[4].split()[0]),lock_ino=int(rows[4].split()[1]),pid_ns=rows[5],pg_pid_ns=sampler['pid_ns'],flock_exit=73)
    return validate_anchor(row)


def running_projection(facts):
    s=facts['State']
    return dict(id=facts['Id'],image=facts['Image'],running=s['Running'],status=s['Status'],pid=s['Pid'],started_at=s['StartedAt'],
                restart_count=facts['RestartCount'],dead=s['Dead'],oom_killed=s['OOMKilled'],error=s['Error'])


def validate_running(row,pin):
    require(type(row) is dict and set(row)=={'id','image','running','status','pid','started_at','restart_count','dead','oom_killed','error'} and
            row['running'] is True and row['status']=='running' and row['dead'] is False and row['oom_killed'] is False and row['error']=='' and
            type(row['pid']) is int and row['pid']>0 and type(row['restart_count']) is int and row['restart_count']>=0 and
            type(row['started_at']) is str and row['started_at'] not in ('','0001-01-01T00:00:00Z') and
            all(row[k]==pin[k] for k in ('id','image','pid','started_at','restart_count')),'NAMESPACE_RUNNING_PIN')
    return row

# Snapshot namespace PIDs before spawning scan children. Only scanner's own PID
# is omitted; all surviving rows carry real exe/starttime/UID/environment keys.
PROC_SCAN = r'''
set -eu
printf 'SCANNER %s\n' "$$"
for d in /proc/[0-9]*; do
 p=${d##*/}; test "$p" != "$$" || continue
 test -r "$d/stat" || continue
 s=$(cat "$d/stat") || continue
 e=$(readlink "$d/exe") || continue
 u=$(awk '/^Uid:/{print $2}' "$d/status") || continue
 keys=$(tr '\000' '\n' < "$d/environ" | sed 's/=.*//' | sort | tr '\n' ',') || continue
 printf 'ROW\t%s\t%s\t%s\t%s\n' "$u" "$e" "$s" "$keys"
done
'''


def parse_proc(output):
    lines=output.splitlines();require(lines and re.fullmatch(r'SCANNER [1-9][0-9]*',lines[0]),'PROCESS_SCAN_HEADER')
    rows=[]
    for line in lines[1:]:
        parts=line.split('\t');require(len(parts)==5 and parts[0]=='ROW','PROCESS_SCAN_ROW')
        uid,exe,raw,keys=parts[1:]
        end=raw.rfind(')');require(end>0,'PROCESS_STAT')
        tail=raw[end+2:].split()
        require(len(tail)>=20,'PROCESS_STAT')
        rows.append(dict(pid=int(raw.split(' ',1)[0]),starttime=int(tail[19]),uid=int(uid),exe=exe,ppid=int(tail[1]),env_keys=[k for k in keys.split(',') if k]))
    return rows


class HelperContainer:
    """Compose pinned public helper validator with explicit trusted /var/lib tmpfs.

    The public predicate remains byte-pinned. Added tmpfs is validated exactly,
    then removed only from the copy passed to its older single-tmpfs predicate.
    All other security/mount/network/created/running facts are checked unchanged.
    """
    def __init__(self,helper,binding,runner,result,image,mounts,command,*,env=None,interactive=False,fs=False,_full_profile=None):
        self.h,self.b,self.runner,self.result=helper,binding,runner,result
        self._full_profile=_full_profile
        if _full_profile is not None:
            from p0c4_completion.full_target_fs import _HelperProfile
            require(type(_full_profile) is _HelperProfile,'FULL_HELPER_PROFILE')
            _full_profile._request(mounts,command,interactive,fs,env)
        name='knowweave-lifecycle-helper-'+uuid.uuid4().hex
        labels={**(image['Config'].get('Labels') or {}),'knowweave.source-lifecycle.batch':result['batch_id']}
        supplied=env or {}
        require(not any(k in supplied for k in ('PGPASSWORD','DATABASE_URL','ADMIN_DSN','TEST_ADMIN_DATABASE_URL')),'HELPER_SECRET_ENV')
        args=['create','--name',name,'--pull=never','--network=none','--read-only','--cap-drop=ALL','--security-opt=no-new-privileges',
              '--cpus=4','--memory=8g','--memory-swap=8g','--pids-limit=512','--tmpfs=/tmp:rw,nosuid,nodev,size=1g','--user=0:0','--workdir=/reviewed','--entrypoint='+command[0]]
        if _full_profile is not None:args=_full_profile._args(args)
        if fs:args+=['--tmpfs=/var/lib:rw,nosuid,nodev,size=64m,mode=0755']
        if interactive:args+=['--interactive']
        for key,value in labels.items():args+=['--label',key+'='+value]
        expected_mounts=set();volume_names={}
        for kind,src,dest,readonly in mounts:
            suffix=',readonly' if readonly else ''
            if kind=='volume':
                suffix+=',volume-nocopy';volume_names[dest]=src
                actual=runner.inspect('volume',src)['Mountpoint']
            else:actual=src
            args+=['--mount',f'type={kind},src={src},dst={dest}'+suffix]
            expected_mounts.add((kind,actual,dest,not readonly))
        for key,value in supplied.items():args+=['--env',key+'='+value]
        self.fs=fs;self.child=None;self.closed=False
        self.expected=dict(id=None,name=name,image=image['Id'],labels=labels,cmd=command[1:],entrypoint=[command[0]],
                           env={**helper.env_dict(image['Config']['Env']),**supplied},mounts=expected_mounts,volume_names=volume_names,
                           network='none',network_id=None,stdin=interactive)
        self.record=dict(id=None,name=name,image=image['Id'],removed=False)
        self.owner=runner_base(runner).ownership.acquire('helper',name,self.expected,None,self.record);self.owner['object']=self
        result['helpers'].append(self.record);result['resource_creation_unknown']=True
        ident=creation_call(runner,self.owner,lambda:runner.docker(*args,image['Id'],*command[1:])).decode().strip()
        require(HEX64.fullmatch(ident),'HELPER_CREATED_ID')
        self.record['id']=self.expected['id']=ident;runner_base(runner).ownership.known(self.owner,ident)
        self.inspect();require(self.inspect()['State']['Status']=='created','HELPER_NOT_FRESH_CREATED')
        result['resource_creation_unknown']=False

    def validate(self,facts):
        adjusted=copy.deepcopy(facts) if getattr(self,'_full_profile',None) is None else self._full_profile._checked_default_copy(facts)
        if self.fs:
            require(facts['HostConfig']['Tmpfs']=={'/tmp':'rw,nosuid,nodev,size=1g','/var/lib':'rw,nosuid,nodev,size=64m,mode=0755'},'HELPER_TRUSTED_TMPFS')
            adjusted['HostConfig']['Tmpfs']={'/tmp':'rw,nosuid,nodev,size=1g'}
        self.b.validate_helper(self.h,adjusted,self.expected)
        return facts

    def inspect(self,runner=None):return self.validate((runner or self.runner).inspect('container',self.record['id']))

    def run(self,stdin=None,timeout=7200,allowed=(0,)):
        args=['start','-a']+(['-i'] if stdin is not None else [])+[self.record['id']]
        code,out,err=self.runner.run([DOCKER,*args],stdin=stdin,timeout=timeout,allowed=allowed)
        facts=self.inspect()
        require(facts['State']['Status']=='exited' and facts['State']['ExitCode']==code and not facts['State'].get('OOMKilled'),'HELPER_PROCESS_EXIT')
        self.close();return code,out,err

    def persistent(self,logs,timeout):
        self.child=MonitoredChild([DOCKER,'start','-a','-i',self.record['id']],logs,timeout=timeout,interactive=True)
        deadline=time.monotonic()+5
        while True:
            facts=self.inspect()
            if facts['State']['Running']:break
            require(time.monotonic()<deadline,'HELPER_START_TIMEOUT');time.sleep(0.05)
        return self.child

    def close(self,phase='close'):
        if self.closed:return
        require(phase in ('close','fallback') and not self.owner.get(phase+'_attempted'),'HELPER_CLEANUP_REPEATED');self.owner[phase+'_attempted']=True
        clean=CleanupRunner(self.runner,self.owner,phase);error=None
        try:
            self.inspect(clean)
            try:clean.docker('stop','--timeout','5',self.record['id'],timeout=10)
            except BaseException as caught:error=caught
            stopped=self.inspect(clean);require(stopped['State']['Running'] is False and stopped['State']['Pid']==0,'HELPER_STOP_UNKNOWN')
            if self.child and not self.child.finished and not self.child.retire_attempted:
                self.child.close_input()
                try:self.child.retire(timeout=TRANSPORT_TEARDOWN)
                except BaseException as caught:error=error or caught
            clean.docker('rm',self.record['id'])
            require(self.record['id'] not in clean.docker('ps','-aq','--no-trunc').decode().split(),'HELPER_REMOVAL_UNKNOWN')
            self.record['removed']=True;self.closed=True;runner_base(self.runner).ownership.release(self.owner,'absent')
            if error is not None:raise error
        except BaseException:
            self.owner['close_failed']=True;raise


class BrokerClient:
    def __init__(self,container,logs,budget):
        self.container,self.budget=container,budget
        self.child=container.persistent(logs,timeout=budget.remaining());self.buffer=bytearray();self.broken=False

    def transition(self,phase):
        require(not self.broken or phase=='CLEANUP','BROKER_PROTOCOL_LOST')
        self.budget.transition(phase)
        self.child.deadline=self.budget.deadline

    def call(self,value,timeout=5):
        require(not self.broken,'BROKER_PROTOCOL_LOST')
        require(0<timeout<=5,'BROKER_RPC_BUDGET')
        self.budget.begin_rpc()
        try:
            raw=canonical(value);require(len(raw)<=1024**2,'BROKER_INPUT_BUDGET')
            self.child.send(raw+b'\n');deadline=time.monotonic()+timeout
            while b'\n' not in self.buffer:
                self.budget.check_rpc();self.child.check()
                require(self.child.p.poll() is None,'BROKER_EXIT')
                require(time.monotonic()<deadline,'BROKER_TIMEOUT')
                try:index,chunk=self.child.events.get(timeout=0.05)
                except queue.Empty:continue
                require(index==0,'BROKER_STDERR');self.buffer.extend(chunk)
                require(len(self.buffer)<=1024**2,'BROKER_OUTPUT_BUDGET')
            self.budget.check_rpc()
            line,_,rest=self.buffer.partition(b'\n');self.buffer=bytearray(rest)
            row=json.loads(line,object_pairs_hook=unique_pairs)
            require(canonical(row)==line,'BROKER_OUTPUT_CANONICAL')
            return row
        except BaseException:
            self.broken=True;raise
        finally:self.budget.end_rpc()

    def close(self):
        try:
            if not self.broken:require(self.call(dict(op='stop'))==dict(stopped=True),'BROKER_STOP_ACK')
        finally:self.container.close()


def run_helper(helper,binding,runner,result,image,mounts,command,*,env=None,timeout=7200,fs=False,allowed=(0,)):
    return HelperContainer(helper,binding,runner,result,image,mounts,command,env=env,fs=fs).run(timeout=timeout,allowed=allowed)


def mounts(source,ident,write_build=True,write_root=False):
    return [('bind',str(source),'/reviewed',True),('volume',ident['build_volume'],'/target',not write_build),('volume',ident['source_volume'],ROOT,not write_root),('volume',ident['registry_volume'],'/var/lib/knowweave-c4/registry',not write_root)]


def compile_case(h,b,runner,result,image,source,ident,manifest,archive_sha,record,*,_full_context=None):
    pin=compile_binding(record)
    env=dict(CARGO_TARGET_DIR='/target/build',CARGO_BUILD_JOBS='4',CARGO_NET_OFFLINE='true',RUSTUP_AUTO_INSTALL='0',
             KNOWWEAVE_SOURCE_COMMIT=manifest['base_commit'],KNOWWEAVE_BUILD_ID_SHA256=archive_sha,**{PIN_ENV:pin},**BUILD_PROFILE_ENV)
    if re.fullmatch(r'kwc4c-[0-9a-f]{32}',ident['project']):env['CARGO_INCREMENTAL']='0'
    if _full_context is not None:env.update(_full_context._compile_pins(record))
    record['compile_source']=dict(application_commit=env['KNOWWEAVE_SOURCE_COMMIT'],application_build_sha256=env['KNOWWEAVE_BUILD_ID_SHA256'],source_binding_sha256=env[PIN_ENV])
    require(not any(k in h.env_dict(image['Config']['Env']) for k in (PIN_ENV,'KNOWWEAVE_C4_VERIFIER_KEY_SHA256')),'UNEXPECTED_IMAGE_PIN')
    compiled={}
    for kind,command in (('lib',['cargo','test','--locked','--offline','-p','learning-backup','--lib','--no-run','--message-format=json']),
                         ('example',['cargo','build','--locked','--offline','-p','learning-backup','--example','c4_task3_migrate','--message-format=json'])):
        code,output,err=run_helper(h,b,runner,result,image,mounts(source,ident),command,env=env)
        artifact=discover_artifact(output.decode(),'learning-backup',kind)
        profile=validate_compiler_profile(output.decode(),artifact,kind=='lib')
        intermediate,receipt=materialize_artifact(h,b,runner,result,image,source,ident,artifact,kind)
        retained='/target/retained/'+ident['case_id']+('-test' if kind=='lib' else '-migrate')
        _,copied,stderr=run_helper(h,b,runner,result,image,mounts(source,ident),['python3','-c',b.COPY_BINARY,intermediate,retained])
        require(stderr==b'','BINARY_COPY_STDERR');row=json.loads(copied,object_pairs_hook=unique_pairs)
        require(set(row)=={'binary_sha256'} and HEX64.fullmatch(row['binary_sha256']),'BINARY_COPY_IDENTITY')
        require(row['binary_sha256']==receipt['sha256'],'BINARY_MATERIALIZED_IDENTITY')
        compiled[kind]=dict(binary=retained,sha256=row['binary_sha256'],artifact=artifact,compiler_stdout_sha256=digest(output),compiler_stderr_sha256=digest(err),exit_code=code)
        record.setdefault('artifact_provenance',{})[kind]=dict(profile=profile,materialization=receipt)
    record['compiled']=compiled
    return env


def fs_and_contract_gates(h,b,runner,result,image,source,ident,env,record):
    # Writable /var/lib tmpfs supplies trusted ancestors to ordinary nonignored
    # FS regressions too; readonly /reviewed is never made writable.
    for command in (['cargo','test','--locked','--offline','-p','learning-backup','--test','maintenance_contract','--test','maintenance_journal'],
                    ['cargo','test','--locked','--offline','-p','learning-backup','--lib','source::binding::'],
                    ['cargo','test','--locked','--offline','-p','learning-backup','--lib','sealed::']):
        code,out,err=run_helper(h,b,runner,result,image,mounts(source,ident),command,env=env,fs=True)
        result.setdefault('selected_regressions',[]).append(dict(command=command,exit_code=code,stdout_sha256=digest(out),stderr_sha256=digest(err),counts=b.parse_regression(out.decode()),classification='LIBRARY_CONTRACT_NOT_LIVE_PG'))
        require(any(row['passed']>0 for row in result['selected_regressions'][-1]['counts']),'REGRESSION_ZERO_TESTS')
    assets_command=['cargo','test','--locked','--offline','-p','learning-assets','--lib','--no-run','--message-format=json']
    code,out,build_err=run_helper(h,b,runner,result,image,mounts(source,ident),assets_command,env=env,fs=True)
    assets=discover_artifact(out.decode(),'learning-assets','lib')
    profile=validate_compiler_profile(out.decode(),assets,True)
    intermediate,receipt=materialize_artifact(h,b,runner,result,image,source,ident,assets,'assets')
    assets_retained='/target/retained/'+ident['case_id']+'-assets'
    compiler_evidence=dict(compiler_stdout_sha256=digest(out),compiler_stderr_sha256=digest(build_err),exit_code=code)
    _,out,err=run_helper(h,b,runner,result,image,mounts(source,ident),['python3','-c',b.COPY_BINARY,intermediate,assets_retained])
    require(err==b'','FS_COPY_STDERR');copied=json.loads(out,object_pairs_hook=unique_pairs)
    require(type(copied) is dict and set(copied)=={'binary_sha256'} and copied['binary_sha256']==receipt['sha256'],'FS_COPY_IDENTITY')
    record['fs_binary']=dict(binary=assets_retained,**copied)
    record.setdefault('artifact_provenance',{})['assets']=dict(profile=profile,materialization=receipt,compiler=compiler_evidence)
    for package,name in FS_TESTS:
        new=str(uuid.uuid4());fsroot=ROOT+'/fs-'+new;other='/var/lib/knowweave-exdev-'+new
        binary=assets_retained if package=='learning-assets' else record['compiled']['lib']['binary']
        script='mkdir -m 700 "$1" "$2"; test "$(stat -c %d "$1")" != "$(stat -c %d "$2")"; exec /usr/bin/env -i PATH=/usr/bin:/bin HOME=/root LC_ALL=C TEST_C4_LIFECYCLE_FS_ROOT="$1" TEST_C4_LIFECYCLE_FS_OTHER_DEVICE="$2" "$3" --ignored --exact "$4" --test-threads=1'
        code,out,err=run_helper(h,b,runner,result,image,mounts(source,ident,False,True),['/bin/sh','-ec',script,'fs',fsroot,other,binary,name],fs=True)
        outcome=parse_test(code,out.decode(),err,name)
        result.setdefault('filesystem_gates',[]).append(dict(outcome,package=package,root=fsroot,other_device=other,classification='SYNTHETIC_FILESYSTEM_PROTOCOL_ACTUAL_BODY'))


def pg_exec(runner,container,command,*,stdin=None,timeout=5,allowed=(0,),clear=True):
    args=[DOCKER,'exec']+(['-i'] if stdin is not None else [])+['--user','0:0',container]
    if clear:args+=['/usr/bin/env','-i','PATH=/usr/bin:/bin','HOME=/root','LC_ALL=C']
    return runner.run([*args,*command],stdin=stdin,timeout=timeout,allowed=allowed)


def sql(runner,container,statement,database='postgres',role='postgres',timeout=5):
    return pg_exec(runner,container,['psql','-X','-qAt','-v','ON_ERROR_STOP=1','-U',role,'-d',database],stdin=statement.encode(),timeout=timeout)[1].decode().strip()


def _prepare_source_control_system_privileges(runner,container,ident,full_context):
    if full_context is None:
        for database in (ident['database'],ident['other_database']):
            sql(runner,container,'GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO learning_admin;',database)
        return
    # Full capture must retain the producer fixture's native default ACL.
    # The real administrator identity query in live_case remains mandatory;
    # missing native privileges reject this candidate, never trigger a GRANT.
    statement=("BEGIN READ ONLY; SET LOCAL statement_timeout='5000ms'; "
               "SELECT p.proacl IS NULL, "
               "has_function_privilege('learning_admin',p.oid,'EXECUTE') "
               "FROM pg_catalog.pg_proc p "
               "WHERE p.oid='pg_catalog.pg_control_system()'::pg_catalog.regprocedure; "
               "ROLLBACK;")
    for database in (ident['database'],ident['other_database']):
        code,out,err=pg_exec(runner,container,['psql','-X','-qAt','-v','ON_ERROR_STOP=1',
                                            '-U','learning_admin','-d',database],stdin=statement.encode())
        require(code==0 and out==b't|t\n' and err==b'','FULL_SOURCE_CONTROL_PRIVILEGES')


def case_source_rpc(runner,record,container,operation,call):
    require(container==record['container_id'] and HEX64.fullmatch(container),'SOURCE_OPERATION_IDENTITY')
    require(operation in ('tools_before','migration_setup','migration_readback','tools_after'),'SOURCE_OPERATION_NAME')
    refs=record.setdefault('source_operation_rpcs',dict(case_id=record['identity']['case_id'],container_id=container))
    require(refs['case_id']==record['identity']['case_id'] and refs['container_id']==container,'SOURCE_OPERATION_IDENTITY')
    require(operation not in refs,'SOURCE_OPERATION_REUSE')
    result=None
    def actual():
        nonlocal result
        result=call();return result
    _,rpc=runner.observed(actual)
    refs[operation]=rpc
    return result


def pg_facts(h,runner,record,expected):
    ident=record['identity']
    raw,rpc=runner.observed(lambda:runner.run([DOCKER,'container','inspect',expected['id']],timeout=5))
    rows=json.loads(raw,object_pairs_hook=unique_pairs);require(type(rows) is list and len(rows)==1,'INSPECT_COUNT')
    facts=rows[0];h.validate_container(facts,expected)
    state=facts['State'];require(state['Status'] in ('created','running','exited') and state['Running']==(state['Status']=='running'),'PG_STATE')
    if state['Status']=='created':require(state['StartedAt']=='0001-01-01T00:00:00Z' and state['Pid']==0,'PG_CREATED_STATE')
    else:require(state['StartedAt']!='0001-01-01T00:00:00Z','PG_STARTED_STATE')
    network_raw,network_rpc=runner.observed(lambda:runner.run([DOCKER,'network','inspect',record['network_id']],timeout=5))
    network_rows=json.loads(network_raw,object_pairs_hook=unique_pairs)
    require(type(network_rows) is list and len(network_rows)==1,'INSPECT_COUNT');network=network_rows[0]
    runner.last_pg_observations=dict(container=rpc,network=network_rpc)
    import ipaddress
    require(network['Id']==record['network_id'] and network['Name']==ident['network'] and network['Internal'] is True and network['Driver']=='bridge' and
            network['IPAM']['Config']==[{'Subnet':record['subnet'],'Gateway':str(ipaddress.ip_network(record['subnet']).network_address+1)}] and
            network['Labels'].get('com.docker.compose.project')==ident['project'] and
            set(network.get('Containers') or {})==({expected['id']} if state['Running'] else set()),'PG_NETWORK_IDENTITY')
    attachments=facts['NetworkSettings']['Networks']
    require(set(attachments)=={ident['network']} and attachments[ident['network']]['NetworkID'] in ('',network['Id']),'PG_NETWORK_ATTACHMENT')
    for key,destination in (('volume','/var/lib/postgresql'),('source_volume',ROOT),('build_volume','/target')):
        volume=runner.inspect('volume',ident[key]);saved=record['volumes'][key]
        equal_audit(saved,volume)
        actual=[m for m in facts['Mounts'] if m['Destination']==destination]
        declared=[m for m in facts['HostConfig']['Mounts'] if m.get('Target')==destination]
        require(len(actual)==len(declared)==1 and actual[0]['Name']==ident[key] and declared[0].get('VolumeOptions',{}).get('NoCopy') is True,'PG_VOLUME_NOCOPY')
    containers=runner.docker('ps','-aq','--no-trunc','--filter','label=com.docker.compose.project='+ident['project']).decode().split()
    require(containers==[expected['id']],'PG_PROJECT_MEMBERSHIP')
    if state['Running'] and runner.namespace.pin is None:
        projected=running_projection(facts)
        pin={key:projected[key] for key in ('id','image','pid','started_at','restart_count')}
        require(pin['id']==expected['id'] and pin['image']==expected['image'],'NAMESPACE_RUNNING_IDENTITY')
        validate_running(projected,pin);runner.namespace.pin=dict(pin,rpc=rpc)
    return facts,network


class SourceFaultController:
    """Fixed test-only ancestor STOP/CONT for this controller's owned PG."""
    def __init__(self,h,runner,record,expected,isolation,broker):
        require(record['test'] in SOURCE_FAULT_TESTS,'SOURCE_FAULT_FIXED_TEST')
        self.h,self.runner,self.record,self.expected,self.isolation,self.broker=h,runner,record,expected,isolation,broker
        self.pending=None
        record['source_fault_transitions']=[]

    def identity(self,row,claim):
        validate_source_fault_request(row,row['backup_id'],claim)
        generation=self.record['generations'][-1]['request']
        require(row['backup_id']==generation['backup_id'] and row['epoch']==generation['request_id'],'SOURCE_FAULT_ACTIVE_EPOCH')
        cid=self.record['container_id'];require(cid==self.expected['id'],'SOURCE_FAULT_OWNED_ID')
        pg_facts(self.h,self.runner,self.record,self.expected)
        actual=self.isolation.observe_source_endpoint(cid,self.record['identity']['project'],row['epoch'],docker=lambda *args:self.runner.docker(*args).decode())
        require(actual==claim,'SOURCE_FAULT_EPOCH_CHANGED')
        return cid

    def state(self,cid,row):
        raw=pg_exec(self.runner,cid,['cat','/proc/1/stat'])[1].decode()
        require(raw.startswith('1 (postgres) '),'SOURCE_FAULT_POSTMASTER')
        tail=raw.removeprefix('1 (postgres) ').split()
        require(len(tail)>=20 and int(tail[19])==row['postmaster_start_ticks'],'SOURCE_FAULT_POSTMASTER_EPOCH')
        return tail[0]

    def transition(self,row,claim):
        cid=self.identity(row,claim)
        if row['phase']=='pause':
            require(self.pending is None and row['database_oid']==self.record['sql_identity']['database_oid'],'SOURCE_FAULT_ADMISSION')
            a,b=row['challenge_keys'];pid,oid=row['backend_pid'],row['database_oid']
            pairs=[((key & ((1<<64)-1))>>32,key & 0xffffffff) for key in (a,b)]
            statement="SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND pid=%d AND database::bigint=%d AND objsubid=1 AND mode='ExclusiveLock' AND granted AND (classid::bigint,objid::bigint) IN ((%d,%d),(%d,%d));"%(pid,oid,*pairs[0],*pairs[1])
            require(sql(self.runner,cid,statement)=='2','SOURCE_FAULT_ORIGINAL_LOCKS')
            self.pending=(row,claim)  # Guard is armed before any signal send.
        else:
            require(self.pending is not None and all(row[k]==self.pending[0][k] for k in ('backup_id','epoch','backend_pid','database_oid','challenge_keys','postmaster_start_ticks')),'SOURCE_FAULT_RESUME_SCOPE')
        signal='SIGSTOP' if row['phase']=='pause' else 'SIGCONT'
        require(self.runner.docker('kill','--signal='+signal,cid)==(cid+'\n').encode(),'SOURCE_FAULT_SIGNAL_RECEIPT')
        observed=self.state(cid,row)
        require(observed=='T' if row['phase']=='pause' else observed in ('R','S','D','I'),'SOURCE_FAULT_SIGNAL_UNCONFIRMED')
        self.identity(row,claim)
        response={key:row[key] for key in ('backup_id','epoch','nonce','phase','postmaster_start_ticks')};response['observed_state']=observed
        self.record['source_fault_transitions'].append(dict(response,container_id=cid,external_signal=signal,actual_postmaster_restart=False))
        if row['phase']=='resume':self.pending=None
        return response

    def service(self):
        rows=self.broker.call(dict(op='source-fault-poll'))
        require(type(rows) is list and len(rows)<=1,'SOURCE_FAULT_POLL_COUNT')
        for row in rows:
            response=self.transition(row['request'],row['claim'])
            self.broker.call(dict(op='source-fault-observed',response=response))

    def resume_before_stop(self):
        if self.pending is not None:
            row,claim=self.pending
            self.transition(dict(row,phase='resume'),claim)
            self.record['source_fault_finally_resume']=True


def root_audit(runner,container,path):
    return parse_pg_root(pg_exec(runner,container,['/bin/sh','-ec',PG_ROOT_AUDIT,'audit',path])[1].decode().strip())


def audit_binaries_in_pg(runner,container,compiled):
    answer={}
    for row in compiled.values():
        script='test -f "$1" && test ! -L "$1"; stat -c "%u %a %h %s" "$1"; sha256sum "$1" | cut -d " " -f 1'
        out=pg_exec(runner,container,['/bin/sh','-ec',script,'binary',row['binary']])[1].decode().splitlines()
        require(len(out)==2 and out[0].split()[:3]==['0','500','1'] and int(out[0].split()[3])>0 and out[1]==row['sha256'],'PG_BINARY_AUDIT')
        answer[row['binary']]=out[1]
    return answer


def migration_rows(runner,container,ident,files,*,record=None):
    expected=[]
    for path,raw in sorted(files.items()):
        if path.startswith('migrations/') and path.endswith('.sql'):
            require(re.fullmatch(r'migrations/[0-9]+_[a-z0-9_]+\.sql',path),'MIGRATION_SOURCE_NAME')
            expected.append(dict(version=int(Path(path).name.split('_',1)[0]),checksum_hex=hashlib.sha384(raw).hexdigest()))
    query="SELECT version::text,encode(checksum,'hex'),success::text FROM public._sqlx_migrations AS migrations ORDER BY migrations.version;"
    if record is None:observed=sql(runner,container,query,ident['database'],'learning_admin')
    else:
        observed=case_source_rpc(runner,record,container,'migration_readback',lambda:pg_exec(runner,container,
            ['psql','-X','-qAt','-v','ON_ERROR_STOP=1','-U','learning_admin','-d',ident['database']],stdin=query.encode()))[1].decode().strip()
    actual=[]
    for line in observed.splitlines():
        version,checksum,success=line.split('|');require(success=='true','MIGRATION_NOT_SUCCESSFUL')
        actual.append(dict(version=int(version),checksum_hex=checksum))
    require(expected and actual==expected,'GENUINE_MIGRATOR_CHECKSUMS')
    return actual


def tool_check(runner,container,*,record=None,operation='tools_before'):
    script=r'''set -eu
for t in /bin/sh /usr/bin/env /usr/bin/flock /usr/bin/stat /usr/bin/readlink /usr/bin/sleep /usr/bin/cat /usr/bin/sha256sum /usr/bin/awk /usr/bin/tr /usr/bin/sed /usr/bin/sort /usr/bin/cut /usr/lib/postgresql/18/bin/psql; do test -x "$t"; done
test -r /proc/1/stat
for t in /usr/lib/postgresql/18/bin/pg_dump /usr/lib/postgresql/18/bin/pg_restore; do
 test -f "$t" && test ! -L "$t"
 stat -c '%u %a %h' "$t"
 "$t" --version
 sha256sum "$t"
done'''
    call=lambda:pg_exec(runner,container,['/bin/sh','-ec',script])
    out=(call() if record is None else case_source_rpc(runner,record,container,operation,call))[1].decode().splitlines()
    require(len(out)==6,'PG_TOOLS_OUTPUT')
    pins={}
    for offset,name in ((0,'pg_dump'),(3,'pg_restore')):
        owner,mode,links=out[offset].split()
        require(owner=='0' and int(mode,8)&0o022==0 and int(links)>=1 and re.fullmatch(name+r' \(PostgreSQL\) 18\.[0-9]+.*',out[offset+1]),'PG_TOOL_IDENTITY')
        sha,path=out[offset+2].split();require(HEX64.fullmatch(sha) and path=='/usr/lib/postgresql/18/bin/'+name,'PG_TOOL_HASH')
        pins[name]=dict(path=path,sha256=sha,version=out[offset+1])
    return pins


class NamespaceAudits:
    def __init__(self,runner,record):
        self.runner,self.record,self.case_id=runner,record,record['identity']['case_id']
        self.pin,self.pid1,self.active=None,None,None;self.audits=[]
        self.ledger_reserved=True
        self.state=LOG_RESERVATIONS[str(runner.runner.logs)]

    def capacity(self):
        logs=self.runner.runner.logs;count=sum(1 for _ in logs.iterdir())
        available=min(256-len(self.audits),512-self.state['audits'],
                      (8192-count-3*self.state['archives']-self.state['ledgers'])//12)
        require(available>0,'NAMESPACE_AUDIT_CAPACITY');log_capacity(logs,12)

    def guard(self,container):
        raw,rpc=self.runner.observed(lambda:self.runner.run([DOCKER,'container','inspect','--format',PG_RUNNING_FORMAT,container],timeout=5))
        require(len(raw)<=4096 and raw.endswith(b'\n') and raw.count(b'\n')==1,'NAMESPACE_RUNNING_FRAMING')
        row=json.loads(raw,object_pairs_hook=unique_pairs)
        return validate_running(row,self.pin),rpc

    def reserve_generation(self):
        with LOG_RESERVATION_LOCK:
            require(self.active not in self.state.setdefault('archive_tokens',set()),'ARCHIVE_TOKEN_REUSED')
            log_capacity(self.runner.runner.logs,3);self.state['archives']+=1;self.state['archive_tokens'].add(self.active)

    def finalize(self):
        require(self.pin is not None,'NAMESPACE_RUNNING_MISSING')
        raw=canonical(dict(format_version=1,case_id=self.case_id,pg_running_pin=self.pin,audits=self.audits))
        require(len(raw)<=1024**2,'NAMESPACE_LEDGER_BUDGET')
        name='namespace-audits-'+self.case_id+'.json';path=self.runner.runner.logs/name
        token='ledger-'+self.case_id
        with LOG_RESERVATION_LOCK:
            require(self.ledger_reserved and self.state['ledgers']>0,'NAMESPACE_LEDGER_REUSED')
            self.state['ledgers']-=1;self.ledger_reserved=False;self.state.setdefault('work',{})[token]={name}
        try:
            new_private(path,raw)
            meta=path.lstat();self.state.setdefault('producer_identities',{})[name]=(meta.st_dev,meta.st_ino)
        finally:
            with LOG_RESERVATION_LOCK:self.state['work'].pop(token)
        self.record['namespace_audits']=dict(path=name,bytes=len(raw),sha256=digest(raw))

    def archives(self,container,inventory):
        generations=self.record['generations']
        for index,generation in enumerate(generations):
            request=generation['request'];nonce=v4(request['request_id']);attempt=v4(request['backup_id'])
            prefix='proofs/generations/'+nonce+'/'
            paths=[prefix+'anchor.json',prefix+'host-observations.json']
            proof_prefix='proofs/' if index==len(generations)-1 else prefix
            paths += [proof_prefix+stem+attempt+'.json' for stem in ('isolation-','inspection-')]
            for path in paths:
                meta=inventory.get(path,{})
                require(all(type(meta.get(k)) is int for k in ('bytes','uid','mode','links')) and meta['uid']==0 and
                        meta['mode']==0o600 and meta['links']==1 and 0<meta['bytes']<=65536 and HEX64.fullmatch(meta.get('sha256','')),'NAMESPACE_ARCHIVE_META')
            require(sum(inventory[p]['bytes']+1 for p in paths)<=65536,'NAMESPACE_ARCHIVE_BUDGET')
            # Only admitted UUID paths; a fixed read command and existing root authority.
            script='set -eu; for p do cat "$p"; printf "\\n"; done'
            raw,rpc=self.runner.observed(lambda:reserved_call(self.runner,('archive',nonce),
                lambda:pg_exec(self.runner,container,['/bin/sh','-ec',script,'archive',*[ROOT+'/'+p for p in paths]])))
            require(len(raw)<=65536 and raw.endswith(b'\n') and len(raw.split(b'\n'))==5,'NAMESPACE_ARCHIVE_FRAMING')
            for path,value in zip(paths,raw.split(b'\n')[:-1]):
                require(canonical(json.loads(value,object_pairs_hook=unique_pairs))==value and len(value)==inventory[path]['bytes'] and
                        digest(value)==inventory[path]['sha256'],'NAMESPACE_ARCHIVE_HASH')
            generation['archive_rpc']=rpc


class Anchor:
    def __init__(self,runner,container,attempt,logs):
        self.runner,self.container,self.attempt,self.lock=runner,container,attempt,ROOT+'/proofs/isolation-'+attempt+'.lock'
        self.child=MonitoredChild([DOCKER,'exec','-i','--user','0:0',container,'/usr/bin/env','-i','PATH=/usr/bin:/bin','HOME=/root','LC_ALL=C','/bin/sh','-ec',ANCHOR,'anchor',self.lock],logs,timeout=1800,interactive=True)
        buffer=bytearray();deadline=min(time.monotonic()+5,runner.budget.deadline,runner.budget.refresh_deadline or runner.budget.deadline)
        while b'\n' not in buffer:
            self.child.check();require(time.monotonic()<deadline,'ANCHOR_READY_TIMEOUT')
            try:index,raw=self.child.events.get(timeout=0.05)
            except queue.Empty:continue
            require(index==0,'ANCHOR_STDERR');buffer.extend(raw)
        match=re.fullmatch(rb'ANCHOR ([1-9][0-9]*)\n',buffer);require(match is not None,'ANCHOR_READY_SHAPE')
        self.pid=int(match[1]);self.audit_pin=self.audit('initial')

    def audit(self,purpose='periodic'):
        self.child.check();require(self.child.p.poll() is None,'ANCHOR_DRIVER_EXIT')
        ns=self.runner.namespace;ns.capacity();started=time.monotonic_ns()
        pre,pre_rpc=ns.guard(self.container)
        root,root_rpc=self.runner.observed(lambda:pg_exec(self.runner,self.container,['/bin/sh','-ec',ANCHOR_AUDIT,'audit',str(self.pid),self.lock]))
        raw,sampler_rpc=self.runner.observed(lambda:self.runner.run(namespace_command(self.container),timeout=5))
        sampler=parse_namespace_sampler(raw)
        if ns.pid1 is not None:require(sampler==ns.pid1,'NAMESPACE_PID1_CHANGED')
        post,post_rpc=ns.guard(self.container);require(pre==post,'NAMESPACE_RUNNING_CHANGED')
        self.child.check();require(self.child.p.poll() is None,'ANCHOR_DRIVER_EXIT')
        row=assemble_anchor(root,sampler);require(row['pid']==self.pid,'ANCHOR_PID_CHANGED')
        validate_anchor(row,getattr(self,'audit_pin',None))
        ns.pid1=sampler
        audit=dict(sequence=len(ns.audits),case_id=ns.case_id,backup_id=self.attempt,active_request_id=v4(ns.active),purpose=purpose,
                   started_monotonic_ns=started,finished_monotonic_ns=time.monotonic_ns(),anchor=row,pre=pre_rpc,root=root_rpc,sampler=sampler_rpc,post=post_rpc)
        ns.audits.append(audit);ns.state['audits']+=1;self.last_audit=audit
        return row

    def retire(self,timeout=TRANSPORT_TEARDOWN):
        return self.child.retire(timeout=timeout)


def teardown_transports(stop_pg,transports,*,failed,timeout=TRANSPORT_TEARDOWN):
    receipt=dict(pg_absence_verified=False,transports_reaped=True,errors=[],transports=[],transport_scope='CLI_ONLY_NOT_EXEC_PROOF')
    def stop():
        try:stop_pg();receipt['pg_absence_verified']=True
        except BaseException as error:receipt['errors'].append(str(error) if isinstance(error,GateError) else 'PG_CLEANUP_UNKNOWN')
    def retire():
        for transport in transports:
            try:
                actual=transport.retire(timeout=timeout)
                receipt['transports'].append(actual)
                if not failed:require(actual.get('exit_code')==0 and actual.get('forced_cli_kill') is False,'TRANSPORT_GRACEFUL_EXIT')
            except BaseException as error:
                receipt['transports_reaped']=False
                receipt['errors'].append(str(error) if isinstance(error,GateError) else 'TRANSPORT_TEARDOWN_UNKNOWN')
    if failed:stop();retire()
    else:retire();stop()
    return receipt


def registry_holder_cohort(record,consumer_rows):
    holder_rows=[]
    if record.get('test')=='protection_tests::real_registry_concurrent_capture' and len(consumer_rows)==2:
        main=[row for row in consumer_rows if not any(row['ppid']==other['pid'] for other in consumer_rows)]
        require(len(main)==1,'REGISTRY_HOLDER_PARENT')
        holder_rows=[row for row in consumer_rows if row['ppid']==main[0]['pid']]
        require(len(holder_rows)==1 and holder_rows[0]['uid']==0 and holder_rows[0]['starttime']>0 and sorted(holder_rows[0]['env_keys'])==sorted(consumer_env(record['identity'])),'REGISTRY_HOLDER_COHORT')
        return main,holder_rows
    return consumer_rows,holder_rows


def observe_cohort(runner,container,record,anchors,consumer_active=False,runtime_child=None,allow_startup_absence=False,*,_full_context=None):
    capture_legacy=record['kind']=='LEGACY_CLOSE_ONLY_SYNTHETIC_PHASES_NO_DUMP' and consumer_active and allow_startup_absence
    if capture_legacy:
        require(container==record['container_id'],'LEGACY_COHORT_CONTAINER')
        raw,rpc=runner.observed(lambda:pg_exec(runner,container,['/bin/sh','-ec',PROC_SCAN]))
    else:raw=pg_exec(runner,container,['/bin/sh','-ec',PROC_SCAN])[1]
    rows=parse_proc(raw.decode())
    pins={};consumer=record['compiled']['lib']['binary']
    consumer_rows=[row for row in rows if row['exe']==consumer]
    # Pinned executable hash is independently checked before the unique actual
    # /proc executable/starttime is admitted; subsequent PID reuse is rejected.
    consumer_rows,holder_rows=registry_holder_cohort(record,consumer_rows)
    expected_counts=(0,1) if consumer_active and allow_startup_absence else ((1,) if consumer_active else (0,))
    require(len(consumer_rows) in expected_counts,'CONSUMER_PROCESS_COUNT')
    if consumer_rows:
        row=consumer_rows[0]
        values=legacy_env(record['identity']) if record['kind']=='LEGACY_CLOSE_ONLY_SYNTHETIC_PHASES_NO_DUMP' else consumer_env(record['identity'],_full_context=_full_context)
        expected=dict(starttime=row['starttime'],exe=consumer,env_keys=sorted(values))
        if 'consumer_process' in record:equal_audit(record['consumer_process'],dict(pid=row['pid'],**expected))
        else:record['consumer_process']=dict(pid=row['pid'],**expected)
        pins[row['pid']]=expected
    for child in holder_rows:
        require(consumer_active and child['ppid']==consumer_rows[0]['pid'],'REGISTRY_HOLDER_SCOPE')
        observation=dict(pid=child['pid'],parent_pid=child['ppid'],starttime=child['starttime'],exe=child['exe'],env_keys=sorted(child['env_keys']))
        previous=record.get('registry_holder_process')
        if previous is not None:equal_audit(previous,observation)
        else:record['registry_holder_process']=observation
        pins[child['pid']]=dict(starttime=child['starttime'],exe=child['exe'],env_keys=child['env_keys'])
    for anchor in anchors.values():
        audit=anchor.audit('cohort')
        pins[audit['pid']]=dict(starttime=audit['starttime'],exe='/usr/bin/dash',env_keys=['HOME','LC_ALL','PATH'])
    if runtime_child is not None:
        runtime_rows=[row for row in rows if row['exe']=='/usr/lib/postgresql/18/bin/psql' and set(row['env_keys'])=={'HOME','LC_ALL','PATH'}]
        require(len(runtime_rows)==1,'OWNED_RUNTIME_PROCESS')
        row=runtime_rows[0]
        pin=dict(pid=row['pid'],starttime=row['starttime'],exe=row['exe'],env_keys=['HOME','LC_ALL','PATH'])
        key='legacy_holder_process' if record['kind']=='LEGACY_CLOSE_ONLY_SYNTHETIC_PHASES_NO_DUMP' else 'runtime_process'
        if key in record:equal_audit(record[key],pin)
        else:record[key]=pin
        pins[row['pid']]={key:pin[key] for key in ('starttime','exe','env_keys')}
    for row in rows:
        if row['exe']=='/usr/lib/postgresql/18/bin/psql' and consumer_rows and row['ppid']==consumer_rows[0]['pid'] and set(row['env_keys'])=={'LC_ALL','PGCONNECT_TIMEOUT'}:
            require(row['uid']==0,'SOURCE_OBSERVER_COHORT');pins[row['pid']]=dict(starttime=row['starttime'],exe=row['exe'],env_keys=row['env_keys'])
        if row['exe']=='/usr/lib/postgresql/18/bin/pg_dump':
            require(consumer_rows,'PGDUMP_COHORT');validate_dump_process(row,consumer_rows[0]['pid'])
            pins[row['pid']]=dict(starttime=row['starttime'],exe=row['exe'],env_keys=row['env_keys'])
    if _full_context is not None:_full_context._pin_management_children(rows,consumer_rows,pins)
    validate_cohort(rows,pins)
    if capture_legacy and consumer_rows:
        require('legacy_cohort_rpc' not in record,'LEGACY_COHORT_RECEIPT_REUSE')
        record['legacy_cohort_rpc']=dict(case_id=record['identity']['case_id'],container_id=container,rpc=rpc)
    return rows



LEGACY_BACKEND_SCAN = r"""
set -eu
p=$1
case "$p" in ''|*[!0-9]*|0) exit 1;; esac
test "$p" != "$$"
d=/proc/$p
u=$(/usr/bin/id -u)
s=$(/usr/bin/cat "$d/stat")
e=$(/usr/bin/readlink "$d/exe")
t=$(/usr/bin/awk '/^Uid:/{print $2}' "$d/status")
exec 3< "$d/environ"
k=$(/usr/bin/sha256sum <&3)
exec 3<&-
case "$k" in *'  -') ;; *) exit 1;; esac
h=${k%  -}
test "${#h}" -eq 64
case "$h" in *[!0-9a-f]*) exit 1;; esac
printf 'BACKEND_OBSERVER %s\nROW\t%s\t%s\t%s\t%s\n' "$u" "$t" "$e" "$s" "$h"
"""


def parse_legacy_backend(raw,backend_pid):
    require(type(backend_pid)is int and backend_pid>1 and len(raw)<=2*1024**2 and raw.endswith(b'\n') and raw.count(b'\n')==2 and b'\r' not in raw,'LEGACY_BACKEND_RAW')
    lines=raw.decode().split('\n')[:-1]
    require(len(lines)==2 and lines[0]=='BACKEND_OBSERVER 999','LEGACY_BACKEND_OBSERVER')
    parts=lines[1].split('\t');require(len(parts)==5 and parts[0]=='ROW','LEGACY_BACKEND_ROW')
    uid,exe,stat,environ_sha256=parts[1:];end=stat.rfind(')');tail=stat[end+2:].split()
    require(end>0 and len(tail)>=20,'LEGACY_BACKEND_STAT')
    pid=stat.split(' ',1)[0]
    require(all(re.fullmatch(r'[1-9][0-9]*',value) for value in (pid,uid,tail[1],tail[19])),'LEGACY_BACKEND_IDENTITY')
    # Opaque association with this RPC's actual read, not environment policy or stability.
    row=dict(pid=int(pid),starttime=int(tail[19]),uid=int(uid),exe=exe,ppid=int(tail[1]),environ_sha256=environ_sha256)
    require(row['pid']==backend_pid and row['starttime']>0 and row['uid']==999 and row['ppid']==1 and
            row['exe']=='/usr/lib/postgresql/18/bin/postgres' and re.fullmatch(r'[0-9a-f]{64}',environ_sha256),'LEGACY_BACKEND_IDENTITY')
    return row


def observe_legacy_backend(runner,container,record,alive,deadline):
    require('legacy_backend_probe' not in record,'LEGACY_BACKEND_PROBE_REUSE')
    require(record['kind']=='LEGACY_CLOSE_ONLY_SYNTHETIC_PHASES_NO_DUMP' and container==record['container_id'] and
            record.get('legacy_cohort') and record.get('legacy_cohort_rpc',{}).get('container_id')==container,'LEGACY_BACKEND_CASE')
    receipt=record['legacy_rendezvous'];pid=receipt.get('backend_pid')
    require(receipt.get('lock_confirmed') is True and type(pid)is int and pid>1 and
            deadline==receipt['host_deadline_monotonic'],'LEGACY_BACKEND_RENDEZVOUS')
    require(alive(),'LEGACY_START_TRANSPORT_EXIT')
    remaining=deadline-time.monotonic();require(remaining>0,'LEGACY_RENDEZVOUS_TIMEOUT')
    raw,rpc=runner.observed(lambda:runner.run([DOCKER,'exec','--user','999:999',container,'/usr/bin/env','-i',
        'PATH=/usr/bin:/bin','HOME=/var/lib/postgresql','LC_ALL=C','/bin/sh','-ec',LEGACY_BACKEND_SCAN,'legacy-backend',str(pid)],timeout=min(5,remaining)))
    require(alive(),'LEGACY_START_TRANSPORT_EXIT')
    require(time.monotonic()<deadline,'LEGACY_RENDEZVOUS_TIMEOUT')
    row=parse_legacy_backend(raw,pid)
    for existing in record['legacy_cohort']:
        if existing['pid']==pid:equal_audit({key:existing[key] for key in ('pid','starttime','uid','exe','ppid')},
                                          {key:row[key] for key in ('pid','starttime','uid','exe','ppid')})
    record['legacy_backend_probe']=dict(case_id=record['identity']['case_id'],container_id=container,backend_pid=pid,observer_uid=999,row=row,rpc=rpc)


def wait_legacy_consumer(observe,alive,binary,deadline,*,clock=time.monotonic,sleep=time.sleep):
    """Retry only a fully validated startup absence under the SAME holder lease."""
    while True:
        require(alive(),'LEGACY_START_TRANSPORT_EXIT')
        require(clock()<deadline,'LEGACY_RENDEZVOUS_TIMEOUT')
        rows=observe()  # Errors/invalid cohort are immediate; never swallowed.
        require(alive(),'LEGACY_START_TRANSPORT_EXIT')
        require(clock()<deadline,'LEGACY_RENDEZVOUS_TIMEOUT')
        consumers=[row for row in rows if row['exe']==binary]
        require(len(consumers)<=1,'CONSUMER_PROCESS_COUNT')
        if consumers:return rows
        sleep(min(0.05,deadline-clock()))


def owned_psql_marker(child,pattern,deadline,token):
    raw=bytearray()
    while b'\n' not in raw:
        child.check();require(child.p.poll() is None and time.monotonic()<deadline,token)
        try:index,chunk=child.events.get(timeout=0.05)
        except queue.Empty:continue
        require(index==0,token+'_STDERR');raw.extend(chunk);require(len(raw)<=1024,token+'_BUDGET')
    match=re.fullmatch(pattern,raw);require(match is not None,token+'_SHAPE');return match


def start_legacy_holder(container,logs,record):
    deadline=time.monotonic()+20
    holder=MonitoredChild([DOCKER,'exec','-i','--user','0:0',container,'/usr/bin/env','-i','PATH=/usr/bin:/bin','HOME=/root','LC_ALL=C',
                           '/usr/lib/postgresql/18/bin/psql','-X','-qAt','-v','ON_ERROR_STOP=1','-U','postgres','-d','postgres'],logs,timeout=30,interactive=True)
    # The fresh ninth fixture's shared catalog is only a startup rendezvous.
    # While held, the host performs Docker/proc observations exclusively.
    record['legacy_rendezvous']=dict(lock_confirmed=False,deadline_expired=False,commit_exit=None,backend_absent=False,
                                    shared_relation='pg_catalog.pg_database',scope='FRESH_LEGACY_FIXTURE_ONLY_NO_DATA_MUTATION',host_deadline_monotonic=deadline)
    statement=b"SET lock_timeout='5s'; SET statement_timeout='5s'; SET idle_in_transaction_session_timeout='30s'; BEGIN; LOCK TABLE pg_catalog.pg_database IN ACCESS EXCLUSIVE MODE; SELECT 'RENDEZVOUS '||pg_backend_pid()::text||' '||(EXISTS(SELECT 1 FROM pg_locks WHERE pid=pg_backend_pid() AND relation='pg_catalog.pg_database'::regclass AND mode='AccessExclusiveLock' AND granted))::text;\n"
    # Return the transport before waiting, so every failure can stop PG first.
    return holder,deadline,statement


def observe_generation(runner,container,ident,broker,generation,boundary,acl=None):
    if acl is None:
        acl=sql(runner,container,"SELECT has_database_privilege('learning_runtime',oid,'CONNECT') FROM pg_database WHERE datname='"+ident['database']+"';")
    require(acl in ('t','f'),'ACL_OBSERVER')
    observation=broker.call(dict(op='observe',nonce=generation['request']['request_id'],boundary=boundary,acl=acl))
    generation['boundary_observations'].append(observation)
    return observation


def close_generation(runner,container,ident,broker,generation,boundary):
    observe_generation(runner,container,ident,broker,generation,boundary)
    generation['classification']=generation_classification(generation['boundary_observations'],generation['denial'])


def real_index_postcheck(runner,container,record,inventory):
    ident=record['identity']
    raw=sql(runner,container,"SELECT space_id::text,id::text,sha256,byte_size::text,storage_key FROM public.asset WHERE status='ready' ORDER BY space_id,id;",ident['database'],'learning_admin')
    rows=[]
    for line in raw.splitlines():
        space,asset,sha,size,key=line.split('|');rows.append(dict(space_id=space,id=asset,sha256=sha,byte_size=int(size),storage_key=key))
    indexed=[path for path in inventory if re.fullmatch(r'pins/[0-9a-f-]{36}\.sealed/asset-index\.json',path)]
    require(len(indexed)==1,'CAPTURE_INDEX_INVENTORY')
    path=indexed[0];absolute=ROOT+'/'+path
    index_raw=pg_exec(runner,container,['cat',absolute])[1]
    checked=validate_index(index_raw,rows)
    prefix=path.removesuffix('asset-index.json');manifest_entry=inventory[prefix+'manifest.json']
    manifest_raw=bytes.fromhex(manifest_entry['raw_hex']);manifest=json.loads(manifest_raw,object_pairs_hook=unique_pairs)
    require(list(manifest)==['capability','format_version','backup_id','source','logical_asset_count','unique_asset_bytes','files'],'MANIFEST_FIELD_ORDER')
    require(json.dumps(manifest,ensure_ascii=False,separators=(',',':')).encode()==manifest_raw,'MANIFEST_CANONICAL')
    require(manifest['capability']=='backup_full_v1' and type(manifest['format_version']) is int and manifest['format_version']==1 and
            manifest['backup_id']==prefix.split('/')[1].removesuffix('.sealed') and manifest['logical_asset_count']==4 and manifest['unique_asset_bytes']==96 and
            manifest['source']['application_commit']==BASE_COMMIT and manifest['source']['application_build_sha256']==record['archive_sha256'] and
            manifest['source']['postgres_major']==18 and manifest['source']['migrations']==record['migrations'],'MANIFEST_SOURCE_IDENTITY')
    source=manifest['source']
    require(list(source)==['application_build_sha256','application_commit','postgres_major','migration_version','migrations','migration_fingerprint'] and
            source['migration_version']==record['migrations'][-1]['version'] and
            source['migration_fingerprint']==digest(json.dumps(record['migrations'],separators=(',',':')).encode()),'MANIFEST_MIGRATION_FINGERPRINT')
    expected={prefix+'manifest.json'};expected_dirs=set()
    paths=[file['path'] for file in manifest['files']]
    require(paths==sorted(set(paths)) and {'database.dump','asset-index.json','roles.json'}<=set(paths),'MANIFEST_REQUIRED_FILES')
    for file in manifest['files']:
        require(set(file)=={'path','size','sha256'} and not any(part in ('','..','.') for part in file['path'].split('/')),'MANIFEST_FILE_RECORD')
        entry=inventory.get(prefix+file['path']);require(entry is not None and entry.get('bytes')==file['size'] and entry.get('sha256')==file['sha256'],'RETAINED_FILE_HASH')
        expected.add(prefix+file['path'])
        parts=file['path'].split('/')
        for end in range(1,len(parts)):expected_dirs.add(prefix+'/'.join(parts[:end]))
    actual={p for p,v in inventory.items() if p.startswith(prefix) and not v.get('directory')}
    require(expected==actual,'SEALED_EXACT_INVENTORY')
    actual_dirs={p for p,v in inventory.items() if p.startswith(prefix) and v.get('directory')}
    require(actual_dirs==expected_dirs,'SEALED_EXACT_DIRECTORIES')
    for row in rows:
        require(inventory[prefix+'assets/'+row['storage_key']]['sha256']==row['sha256'] and inventory[prefix+'assets/'+row['storage_key']]['bytes']==row['byte_size'],'SEALED_ORIGINAL_BYTES')
    if record.get('test') in (SOURCE_BOUND_DUMP_TEST,*FULL_RETAINED_ORIGINAL_TESTS):
        # These exact native/full capture fixtures retain healthy source originals;
        # legacy retention fixtures still prove the original-loss contract.
        originals={p:v for p,v in inventory.items() if p.startswith('assets/') and not v.get('directory')}
        expected_originals={}
        for row in rows:
            sha,size,key=row['sha256'],row['byte_size'],row['storage_key']
            require(type(sha) is str and HEX64.fullmatch(sha) and type(size) is int and 0<=size<=96 and key=='sha256/'+sha[:2]+'/'+sha,'FIXTURE_ORIGINAL_INDEX_LINK')
            path='assets/'+key;value=dict(sha256=sha,bytes=size)
            require(path not in expected_originals or expected_originals[path]==value,'FIXTURE_ORIGINAL_ALIAS')
            expected_originals[path]=value
        require(set(originals)==set(expected_originals) and len(originals)==3 and sum(v['bytes'] for v in expected_originals.values())==96,'FIXTURE_ORIGINAL_EXACT_SET')
        for path,expected in expected_originals.items():
            entry=originals[path];sealed=inventory[prefix+path]
            require(type(entry) is dict and set(entry)=={'bytes','sha256','mode','uid','links'} and type(entry['bytes']) is int and entry['bytes']==expected['bytes']==sealed['bytes'] and entry['sha256']==expected['sha256']==sealed['sha256'] and type(entry['uid']) is int and entry['uid']==0 and type(entry['links']) is int and entry['links']==1 and type(entry['mode']) is int and 0<=entry['mode']<=0o777 and not entry['mode']&0o133,'FIXTURE_ORIGINAL_HEALTHY_BYTES')
        policy='source_bound_retained_healthy' if record['test']==SOURCE_BOUND_DUMP_TEST else 'full_import_retained_healthy'
        checked.update(original_policy=policy,original_files=3,original_unique_asset_bytes=96,original_hashes_verified=True)
    else:
        require(not any(p.startswith('assets/') and not v.get('directory') for p,v in inventory.items()),'FIXTURE_ORIGINAL_REMOVAL')
    code,toc,err=pg_exec(runner,container,['/usr/lib/postgresql/18/bin/pg_restore','--list',ROOT+'/'+prefix+'database.dump'])
    toc_receipt=validate_toc(code,toc,err,ident['database'])
    new_private(Path(record['evidence_directory'])/'capture.toc',toc)
    checked.update(unique_asset_bytes=96,full_retained_hashes=True,toc_sha256=digest(toc),toc_only_no_restore=True,
                   migration_rows_sha256=digest(canonical(record['migrations'])),sql_ready_rows=rows,toc=toc_receipt)
    return checked


def admit_created_pg(h,owner,facts,network,record):
    planned=owner['intent'];expected=copy.deepcopy(planned['expected']);net=planned['network']
    require(type(facts.get('Id')) is str and HEX64.fullmatch(facts['Id']) and type(network.get('Id')) is str and HEX64.fullmatch(network['Id']),'PG_CREATED_ID')
    require(all(facts['Config']['Labels'].get(k)==v for k,v in expected['labels'].items()) and facts['Config'].get('User','')==expected['user'],'PG_COMPOSE_LABELS')
    expected.update(id=facts['Id'],labels=copy.deepcopy(facts['Config']['Labels']));h.validate_container(facts,expected)
    import ipaddress
    require(network['Name']==net['name'] and network['Internal'] is True and network['Driver']=='bridge' and
            network['Labels'].get('com.docker.compose.project')==net['project'] and
            network['IPAM']['Config']==[dict(Subnet=net['subnet'],Gateway=str(ipaddress.ip_network(net['subnet']).network_address+1))] and
            set(network.get('Containers') or {})<={facts['Id']},'PG_NETWORK_IDENTITY')
    attachments=facts['NetworkSettings']['Networks']
    require(set(attachments)=={net['name']} and attachments[net['name']]['NetworkID'] in ('',network['Id']),'PG_NETWORK_ATTACHMENT')
    record.update(container_id=facts['Id'],network_id=network['Id']);return expected


def lifecycle_fixture_initdb(raw):
    """Adapt only the pinned admission fixture's one role membership grant."""
    old=b'GRANT learning_auth_lock TO learning_admin WITH SET TRUE;\n'
    new=b'GRANT learning_auth_lock TO learning_admin WITH INHERIT FALSE, SET TRUE;\n'
    lines=raw.splitlines(keepends=True)
    require(lines.count(old)==1,'LIFECYCLE_FIXTURE_MEMBERSHIP')
    return b''.join(new if line==old else line for line in lines)


def live_case(h,b,isolation,runner,batch,source,ident,subnet,name,images,result,manifest,files,archive_sha,first,legacy=False,total_deadline=None,source_clone=None,_full_context=None):
    if _full_context is not None:
        if __package__:
            from .p0c4_completion.full_import import FullRehearsalContext
        else:
            from p0c4_completion.full_import import FullRehearsalContext
        require(type(_full_context) is FullRehearsalContext and not legacy and source_clone is None,'FULL_FIXED_CONTEXT')
        _full_context._admit_call(ident,subnet,name,archive_sha)
    else:
        require(name==LEGACY_TEST if legacy else name in (*PG_TESTS,*REGISTRY_TESTS,*SOURCE_ENDPOINT_TESTS,*DESTINATION_TESTS),'CASE_TEST_IDENTITY')
    log_capacity(runner.logs,93)
    registry=runner.ownership;registry.case_id=ident['case_id'];registry.deadline=None;child_start=len(registry.children)
    require((source_clone is not None)==(name==SOURCE_CLONE_TEST),'SOURCE_CLONE_FIXED_CASE')
    registry.source_clone_pg_names={ident['pg_name'],completion_case_identity(source_clone['case_id'])['pg_name']} if source_clone is not None else None
    budget=CaseBudget(total_deadline=total_deadline);runner=BudgetRunner(runner,budget)
    case=batch/ident['case_id'];h.mkdir_new(case);h.mkdir_new(case/'secrets')
    passwords={role:secrets.token_hex(32) for role in ('postgres','admin')}
    for role,password in passwords.items():h.write_new(case/'secrets'/(role+'_password'),password.encode(),0o400)
    h.write_new(case/'initdb.sh',lifecycle_fixture_initdb(h.INITDB),0o444)
    record=dict(identity=ident,subnet=subnet,test=name,stopped=False,execs_absent=False,transports_reaped=False,stage='new-volumes',volumes={},archive_sha256=archive_sha,generations=[],evidence_directory=str(case),
                kind='LEGACY_CLOSE_ONLY_SYNTHETIC_PHASES_NO_DUMP' if legacy else 'REAL_SOURCE_ENDPOINT' if name in SOURCE_ENDPOINT_TESTS else 'REAL_DESTINATION_FILES_SYNTHETIC_HISTORY_NOT_COMPLETE' if name in DESTINATION_TESTS else 'REAL_LIFECYCLE')
    result['cases'].append(record)
    if _full_context is not None:
        try:_full_context._prepare(h,b,runner,batch,source,case,ident,images,result,record,files)
        except BaseException:
            record['preparation_failure']=_full_context._failure_observation()
            try:
                runner.refresh_deadline=budget.refresh_deadline=None
                budget.transition('CLEANUP');registry.enter(budget)
                record['phase_budget_history']=budget.history
                _full_context._cleanup_target(record)
            except BaseException:record['preparation_cleanup']='ISOLATION_UNCONFIRMED'
            raise
        h=_full_context._container_adapter(h)
    if name==SOURCE_FAULT_TESTS[0]:
        record.update(actual_postmaster_restart=False,source_failure_exercise='actual_random_session_lock_loss')
    runner.namespace=NamespaceAudits(runner,record)
    pg_environment=dict(POSTGRES_USER='postgres',POSTGRES_DB='postgres',POSTGRES_PASSWORD_FILE='/run/secrets/postgres_password',
                        POSTGRES_INITDB_ARGS='--auth-host=scram-sha-256 --auth-local=trust',C4_DATABASE=ident['database'],C4_OTHER_DATABASE=ident['other_database'])
    binds=[dict(type='bind',source=str(case/'initdb.sh'),target='/docker-entrypoint-initdb.d/10-lifecycle.sh',read_only=True)]
    binds.extend(dict(type='bind',source=str(case/'secrets'/(role+'_password')),target='/run/secrets/'+role+'_password',read_only=True) for role in passwords)
    volume_spec={'pg':(ident['volume'],'/var/lib/postgresql',False),'source':(ident['source_volume'],ROOT,False),'build':(ident['build_volume'],'/target',True),'registry':(ident['registry_volume'],'/var/lib/knowweave-c4/registry',False)}
    document={'name':ident['project'],'services':{'pg':{'image':h.PG_IMAGE,'pull_policy':'never','container_name':ident['pg_name'],
        'command':['postgres','-c','max_prepared_transactions=16'],'cpus':2,'mem_limit':'4g','memswap_limit':'4g','security_opt':['no-new-privileges'],
        'environment':pg_environment,'volumes':[*[dict(type='volume',source=key,target=target,read_only=ro,volume={'nocopy':True}) for key,(_,target,ro) in volume_spec.items()],*binds],
        'networks':['test']}},'networks':{'test':{'name':ident['network'],'internal':True,'ipam':{'config':[{'subnet':subnet}]}}},
        'volumes':{key:{'name':v[0],'external':True} for key,v in volume_spec.items()}}
    if _full_context is not None:
        _full_context._extend_source_document(document,binds,volume_spec)
    expected=None;broker=None;anchors={};consumer=None;runtime=None;holder=None;pg_owner=None;clone_peer=None;source_fault=None
    try:
        if source_clone is not None:
            from p0c4_completion.resources import SourceClone
            clone_peer=SourceClone(h,b,runner,batch,source_clone,result)
            document['volumes']['clonepg']={'name':clone_peer.ident['volume'],'external':True}
            document['services']['pg']['volumes'].append(dict(type='volume',source='clonepg',target=clone_peer.COPY_ROOT,volume={'nocopy':True}))
        h.write_new(case/'compose.json',canonical(document))
        for key in ('volume','source_volume','build_volume','registry_volume'):
            result['resource_creation_unknown']=True
            labels={'com.docker.compose.project':ident['project'],'knowweave.source-lifecycle.batch':result['batch_id']}
            pending=record['pending_volume']=dict(name=ident[key],labels=labels)
            owner=registry.acquire('volume',ident[key],dict(name=ident[key],labels=labels),None,pending)
            record['volumes'][key]=creation_call(runner,owner,lambda:b.create_volume(h,runner,ident[key],labels))
            registry.known(owner,ident[key]);registry.release(owner,'retained');record.pop('pending_volume')
            result['resource_creation_unknown']=False
        compose=[DOCKER,'compose','--project-name',ident['project'],'--project-directory',str(case),'-f',str(case/'compose.json')]
        record['stage']='compose-create';result['resource_creation_unknown']=True
        expected_mounts={('bind',item['source'],item['target'],False) for item in binds}
        for key,(_,target,ro) in volume_spec.items():
            saved=record['volumes'][{'pg':'volume','source':'source_volume','build':'build_volume','registry':'registry_volume'}[key]]
            expected_mounts.add(('volume',saved['Mountpoint'],target,not ro))
        if clone_peer is not None:expected_mounts.add(('volume',clone_peer.volume['Mountpoint'],clone_peer.COPY_ROOT,True))
        if _full_context is not None:expected_mounts.update(_full_context._source_extra_mounts())
        expected=dict(id=None,name=ident['pg_name'],image=images[h.PG_IMAGE]['Id'],image_ref=h.PG_IMAGE,
                      env={**h.env_dict(images[h.PG_IMAGE]['Config']['Env']),**pg_environment},
                      labels={'com.docker.compose.project':ident['project'],'com.docker.compose.service':'pg',
                              'com.docker.compose.project.working_dir':str(case),'com.docker.compose.project.config_files':str(case/'compose.json')},
                      cmd=document['services']['pg']['command'],entrypoint=images[h.PG_IMAGE]['Config']['Entrypoint'],
                      mounts=expected_mounts,network=ident['network'],builder=False,user=images[h.PG_IMAGE]['Config'].get('User',''))
        planned_network=dict(name=ident['network'],project=ident['project'],subnet=subnet)
        pg_owner=registry.acquire('pg',ident['pg_name'],expected,planned_network,record)
        creation_call(runner,pg_owner,lambda:runner.run([*compose,'create','--no-build','--pull','never']))
        facts=runner.inspect('container',ident['pg_name']);network=runner.inspect('network',ident['network'])
        expected=admit_created_pg(h,pg_owner,facts,network,record);registry.known(pg_owner,facts['Id'])
        pg_facts(h,runner,record,expected);result['resource_creation_unknown']=False
        runner.docker('start',expected['id']);container=expected['id'];deadline=time.monotonic()+90
        while True:
            code=pg_exec(runner,container,['pg_isready','-h','localhost','-p','5432','-U','postgres','-d','postgres'],allowed=(0,1,2,3))[0]
            if code==0:break
            require(time.monotonic()<deadline,'PG_READY_TIMEOUT');time.sleep(0.2)
        pg_facts(h,runner,record,expected)
        state=sql(runner,container,"SELECT current_setting('server_version_num')::int/10000,current_setting('max_prepared_transactions');\nSELECT count(*)>0 AND bool_and(auth_method='scram-sha-256') FROM pg_hba_file_rules WHERE type LIKE 'host%';\nSELECT count(*)>0 AND bool_and(auth_method='trust') FROM pg_hba_file_rules WHERE type='local';\nSELECT count(*)=2 AND bool_and(rolpassword IS NULL) FROM pg_authid WHERE rolname IN ('learning_admin','learning_runtime');")
        require(state=='18|16\nt\nt\nt','PG_VERSION_HBA_ROLES')
        # No runtime password exists in host/broker/PG mounts or env.
        pg_exec(runner,container,['/bin/sh','-ec',"a=$(cat /run/secrets/admin_password); psql -X -q -v ON_ERROR_STOP=1 -U postgres -d postgres <<SQL\nALTER ROLE learning_admin PASSWORD '$a';\nSQL"])
        _prepare_source_control_system_privileges(runner,container,ident,_full_context)
        require(sql(runner,container,'SELECT 1;',ident['database'],'learning_runtime')=='1','INITIAL_RUNTIME_LOGIN')
        require(sql(runner,container,"SELECT has_database_privilege('learning_runtime',oid,'CONNECT'),EXISTS(SELECT 1 FROM aclexplode(coalesce(datacl,acldefault('d',datdba))) WHERE grantee=0 AND privilege_type='CONNECT') FROM pg_database WHERE datname='"+ident['database']+"';")=='t|f','INITIAL_CONNECT_ACL')
        require(sql(runner,container,"SELECT (SELECT count(*) FROM pg_stat_activity WHERE datname='"+ident['database']+"'),(SELECT count(*) FROM pg_prepared_xacts WHERE database='"+ident['database']+"');")=='0|0','INITIAL_SESSIONS_PREPARED')
        h.validate_pg_processes(runner.docker('top',container,'-eo','uid,pid,comm').decode())
        row=sql(runner,container,"SELECT current_database(),oid::text,(pg_control_system()).system_identifier::text,current_user,pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database();",ident['database'],'learning_admin').split('|')
        require(len(row)==5 and row[0]==ident['database'] and row[3:]==['learning_admin','learning_admin'],'PG_SQL_IDENTITY')
        observed=dict(database=row[0],database_oid=int(row[1]),system_identifier=row[2]);record['sql_identity']=observed
        broker_container=HelperContainer(h,b,runner,result,images[h.BUILDER],mounts(source,ident,False,True),['python3','-u','-c',BROKER],interactive=True)
        broker=BrokerClient(broker_container,batch/'evidence/logs',budget)
        record['stage']='independent-issuer'
        record['issuer']=validate_issuer(broker.call(dict(op='init',identity=ident,sql=observed,admin_password=passwords['admin'],registry_case=dict(batch_id=result['batch_id'],case_id=ident['case_id'],source_package_sha256=archive_sha,application_commit=manifest['base_commit'],application_build_sha256=archive_sha))),observed)
        if name in SOURCE_FAULT_TESTS:
            broker.call(dict(op='source-fault-enable',test=name))
            source_fault=SourceFaultController(h,runner,record,expected,isolation,broker)
        h.write_new(case/'issuer.json',canonical(record['issuer']))
        h.write_new(case/'registry-provisioning.json',canonical(record['issuer']['registry_provisioning']))
        record['pg_root_before']=root_audit(runner,container,ROOT+'/control')
        compile_binding(record)
        if _full_context is not None:_full_context._finalize_profile(record,expected)
        record['stage']='independent-consumer-build'
        env=compile_case(h,b,runner,result,images[h.BUILDER],source,ident,manifest,archive_sha,record,_full_context=_full_context)
        if first:fs_and_contract_gates(h,b,runner,result,images[h.BUILDER],source,ident,env,record)
        binaries=[row['binary'] for row in record['compiled'].values()]
        before_message=dict(op='audit',binaries=binaries)
        if name in REGISTRY_INCOMPLETE_TESTS:before_message['registry_fault_test']=name
        record['audit_before']=broker.call(before_message)
        require(record['audit_before']['binding_sha256']==compile_binding(record),'BINDING_AFTER_BUILD')
        if name in DESTINATION_TESTS:
            record['destination_source_before']=destination_source_inventory(broker.call(dict(op='inventory')))
            record['destination_acl_before']=sql(runner,container,"SELECT has_database_privilege('learning_runtime',oid,'CONNECT') FROM pg_database WHERE datname='"+ident['database']+"';")
            require(record['destination_acl_before']=='t','DESTINATION_SOURCE_INITIAL_ACL')
        record['pg_binary_before']=audit_binaries_in_pg(runner,container,record['compiled']);record['tools_before']=tool_check(runner,container,record=record,operation='tools_before')
        migration=record['compiled']['example']['binary']
        migration_script='export TEST_ADMIN_DATABASE_URL="$(cat /var/lib/knowweave-source/secrets/admin.dsn)"; export TEST_C4_TASK3_DATABASE_NAME="$1"; exec "$2"'
        code,out,err=case_source_rpc(runner,record,container,'migration_setup',lambda:pg_exec(runner,container,['/bin/sh','-ec',migration_script,'migrate',ident['database'],migration],timeout=90))
        require(code==0 and out==b'C4_TASK3_MIGRATED\n' and err==b'','GENUINE_MIGRATOR_EXIT')
        record['migration_setup']=dict(exit_code=code,stdout_sha256=digest(out),stderr_sha256=digest(err),secret_scope='only_short_lived_inside_pg_setup_environment')
        record['migrations']=migration_rows(runner,container,ident,files,record=record)
        require(sql(runner,container,'SELECT count(*) FROM public.asset;',ident['database'],'learning_admin')=='0','EMPTY_MIGRATED_ASSETS')
        if clone_peer is not None:
            clone_peer.start(container,document,expected)
            record['source_clone']=clone_peer.record
            broker.call(dict(op='source-clone-setup',setup=dict(physical_backup_verified=True,original_container_id=container,clone_container_id=clone_peer.record['container_id'],clone_project=clone_peer.ident['project'],database=ident['database'])))
        _,listing,err=pg_exec(runner,container,[record['compiled']['lib']['binary'],'--list','--ignored'])
        names=[line.removesuffix(': test') for line in listing.decode().splitlines() if line.endswith(': test')]
        require(err==b'' and all(names.count(test)==1 for test in PG_TESTS),'PG_IGNORED_DISCOVERY')
        if legacy:require(names.count(LEGACY_TEST)==1,'LEGACY_DISCOVERY')
        if name in SOURCE_ENDPOINT_TESTS:require(names.count(name)==1,'SOURCE_ENDPOINT_IGNORED_DISCOVERY')
        if name in DESTINATION_TESTS:require(names.count(name)==1,'DESTINATION_IGNORED_DISCOVERY')
        if _full_context is not None:require(names.count(name)==1,'FULL_IGNORED_DISCOVERY')
        record['discovery']=dict(names=list(PG_TESTS),stdout_sha256=digest(listing),classification='DISCOVERY_NOT_ACCEPTANCE')
        broker.transition('LIVE')
        # Only this owned runtime client drains itself; never terminate a foreign
        # session or guess a pg_backend_pid after transport ambiguity.
        if first and clone_peer is None and name not in DESTINATION_TESTS:
            runtime=MonitoredChild([DOCKER,'exec','-i','--user','0:0',container,'/usr/bin/env','-i','PATH=/usr/bin:/bin','HOME=/root','LC_ALL=C',
                                   '/usr/lib/postgresql/18/bin/psql','-X','-qAt','-v','ON_ERROR_STOP=1','-U','learning_runtime','-d',ident['database']],batch/'evidence/logs',timeout=300,interactive=True)
            runtime.send(b"BEGIN; SELECT 'DRAIN '||pg_backend_pid()::text;\n")
            marker=bytearray();deadline=time.monotonic()+5
            while b'\n' not in marker:
                require(time.monotonic()<deadline,'DRAIN_READY_TIMEOUT')
                try:index,raw=runtime.events.get(timeout=0.05)
                except queue.Empty:continue
                require(index==0,'DRAIN_STDERR');marker.extend(raw)
            match=re.fullmatch(rb'DRAIN ([1-9][0-9]*)\n',marker);require(match is not None,'DRAIN_MARKER')
            record['drain_backend_pid']=int(match[1])
            actual=sql(runner,container,"SELECT pid::text,state,xact_start IS NOT NULL FROM pg_stat_activity WHERE pid="+str(record['drain_backend_pid'])+';')
            require(actual==str(record['drain_backend_pid'])+'|idle in transaction|t','DRAIN_ACTUAL_TRANSACTION')
            record['drain_observed_before']=actual
        record['stage']='actual-libtest'
        if legacy:
            values=legacy_env(ident)
            holder,legacy_deadline,legacy_statement=start_legacy_holder(container,batch/'evidence/logs',record)
            holder.send(legacy_statement)
            marker=owned_psql_marker(holder,rb'RENDEZVOUS ([1-9][0-9]*) true\n',legacy_deadline,'LEGACY_LOCK_MARKER')
            record['legacy_rendezvous'].update(backend_pid=int(marker[1]),lock_confirmed=True)
            command=[DOCKER,'exec','--user','0:0',container,'/usr/bin/env','-i',*[key+'='+value for key,value in values.items()],record['compiled']['lib']['binary'],'--ignored','--exact',name,'--test-threads=1']
        else:command=consumer_command(container,record['compiled']['lib']['binary'],name,consumer_env(ident,_full_context=_full_context),_full_context=_full_context)
        consumer=MonitoredChild(command,batch/'evidence/logs',timeout=budget.remaining())
        if legacy:
            # pg_database is locked: no SQL observation until this SAME owned
            # holder commits. Actual proc/Docker state, never fabricated data.
            def startup_alive():
                consumer.check();holder.check()
                return consumer.p.poll() is None and holder.p.poll() is None
            record['legacy_cohort']=wait_legacy_consumer(
                lambda:observe_cohort(runner,container,record,{},True,holder,allow_startup_absence=True),
                startup_alive,record['compiled']['lib']['binary'],legacy_deadline)
            observe_legacy_backend(runner,container,record,startup_alive,legacy_deadline)
            record['legacy_rendezvous']['deadline_expired']=time.monotonic()>=legacy_deadline
            require(not record['legacy_rendezvous']['deadline_expired'],'LEGACY_RENDEZVOUS_TIMEOUT')
            holder.send(b'COMMIT;\n\\q\n');holder.close_input()
            holder.deadline=min(holder.deadline,legacy_deadline)
            commit_exit,_,commit_errors=holder.finish(terminate_on_error=False);holder=None
            record['legacy_rendezvous']['commit_exit']=commit_exit
            require(commit_exit==0 and commit_errors==b'','LEGACY_COMMIT_EXIT')
            absent=sql(runner,container,'SELECT count(*) FROM pg_stat_activity WHERE pid='+str(record['legacy_rendezvous']['backend_pid'])+';')=='0'
            record['legacy_rendezvous']['backend_absent']=absent
            record['legacy_rendezvous']['deadline_expired']=time.monotonic()>=legacy_deadline
            require_legacy_observation(record,values)
        active=None;denied=False;last_anchor_check=0;last_acl_poll=0
        while not legacy and consumer.p.poll() is None:
            consumer.check()
            if source_fault is not None:
                source_fault.service()
                if source_fault.pending is not None:
                    # New backend creation is unavailable while PID1 is stopped;
                    # preserve the existing native admission and fixed resume RPC.
                    time.sleep(0.05);continue
            if clone_peer is not None:
                from p0c4_completion.resources import service_source_clone_observer
                service_source_clone_observer(clone_peer,broker,record,anchors,isolation)
            requests=broker.call(dict(op='poll'))
            if requests:
                refresh_started=time.monotonic()
                runner.refresh_deadline=budget.refresh_deadline=refresh_started+20
                if active:close_generation(runner,container,ident,broker,record['generations'][-1],'next_request')
                info=requests[0];request=info['request'];attempt=request['backup_id']
                active=request;denied=False
                runner.namespace.active=request['request_id'];runner.namespace.reserve_generation()
                if attempt not in anchors:
                    require(info['lock_new'] is True,'ANCHOR_LOCK_REUSE_UNKNOWN')
                    anchors[attempt]=Anchor(runner,container,attempt,batch/'evidence/logs')
                else:require(info['lock_new'] is False,'ANCHOR_LOCK_REPLACED')
                anchor=anchors[attempt].audit('refresh');fresh_audit=anchors[attempt].last_audit
                actual_pg,network=pg_facts(h,runner,record,expected)
                processes=observe_cohort(runner,container,record,anchors,True,runtime,_full_context=_full_context)
                if clone_peer is not None and request['compose_project']==clone_peer.ident['project']:
                    clone_pg,clone_network=clone_peer.facts()
                    inspection=isolation.assess_project(clone_peer.ident['project'],[clone_pg],[clone_network])
                    inspection['source_endpoint']=isolation.observe_source_endpoint(clone_pg['Id'],clone_peer.ident['project'],request['request_id'],docker=lambda *args:runner.docker(*args).decode())
                else:inspection=isolation.assess_project(ident['project'],[actual_pg],[network])
                if clone_peer is None:
                    inspection['source_endpoint']=(isolation.observe_source_endpoint(container,ident['project'],request['request_id'],docker=lambda *args:runner.docker(*args).decode()) if _full_context is None else isolation._observe_full_rehearsal_endpoint(_full_context,request['request_id'],docker=lambda *args:runner.docker(*args).decode()))
                # assess_project is deliberately reused only after the narrow
                # actual /proc cohort audit established its zero-admin premise.
                observations=dict(container_sha256=digest(canonical(actual_pg)),network_sha256=digest(canonical(network)),processes=processes,
                                  issuer_binding_sha256=record['issuer']['binding_sha256'],request_sha256=info['request_sha256'],test=name,
                                  namespace_audit=dict(sequence=fresh_audit['sequence'],sha256=digest(canonical(fresh_audit))))
                generation=dict(request=request,receipt=None,denial=None,boundary_observations=[],classification=dict(state='UNKNOWN',operation_outcome='UNPROVEN'),
                                anchor_audit_sequence=fresh_audit['sequence'],observation_rpcs=runner.last_pg_observations)
                record['generations'].append(generation)
                observations['boundary_state']=observe_generation(runner,container,ident,broker,generation,'before_ack')
                require(time.monotonic()-refresh_started<20,'REFRESH_HOST_BUDGET')
                receipt=broker.call(dict(op='refresh',nonce=request['request_id'],inspection=inspection,anchor=anchor,observations=observations))
                budget.check()
                generation['receipt']=receipt
                runner.refresh_deadline=budget.refresh_deadline=None
            now=time.monotonic()
            if active and not denied and now-last_acl_poll>=0.10:
                last_acl_poll=now
                acl=sql(runner,container,"SELECT has_database_privilege('learning_runtime',oid,'CONNECT') FROM pg_database WHERE datname='"+ident['database']+"';")
                require(acl in ('t','f'),'ACL_OBSERVER')
                generation=record['generations'][-1]
                if generation.get('last_observed_acl')!=acl:
                    observe_generation(runner,container,ident,broker,generation,'acl_closed' if acl=='f' else 'acl_open',acl)
                    generation['last_observed_acl']=acl
                if acl=='f':
                    code,out,err=pg_exec(runner,container,['psql','-X','-q','-U','learning_runtime','-d',ident['database'],'-c','SELECT 1'],allowed=(0,1,2))
                    validate_denial(code,err,ident['database']);require(out==b'','RUNTIME_DENIAL_STDOUT')
                    # Serial probe and active nonce recheck prevent late evidence
                    # from satisfying the following proof generation.
                    receipt=broker.call(dict(op='denial',nonce=active['request_id'],exit_code=code,stderr=err.decode()))
                    denied=True;record['generations'][-1]['denial']=receipt
                    if runtime is not None:
                        observed=sql(runner,container,"SELECT has_database_privilege('learning_runtime',d.oid,'CONNECT'),(SELECT state FROM pg_stat_activity WHERE pid="+str(record['drain_backend_pid'])+") FROM pg_database d WHERE datname='"+ident['database']+"';").split('|')
                        require(len(observed)==2,'DRAIN_OBSERVATION_SHAPE')
                        barrier=observe_generation(runner,container,ident,broker,generation,'before_owned_commit',observed[0])
                        barrier['runtime_state']=observed[1];validate_drain_barrier(barrier)
                        h.write_new(case/'owned-drain-before-commit.json',canonical(barrier));record['drain_before_commit']=barrier
                        runtime.send(b'COMMIT;\n\\q\n');runtime.close_input();exit_code,_,errors=runtime.finish(terminate_on_error=False)
                        require(exit_code==0 and errors==b'','DRAIN_COMMIT_EXIT')
                        require(sql(runner,container,'SELECT count(*) FROM pg_stat_activity WHERE pid='+str(record['drain_backend_pid'])+';')=='0','DRAIN_BACKEND_DISAPPEARANCE')
                        record['drain_committed_after_denial']=True;runtime=None
            if now-last_anchor_check>=1:
                for anchor in anchors.values():anchor.audit()
                last_anchor_check=now
            time.sleep(0.05)
        code,out,err=consumer.finish(terminate_on_error=False);consumer=None
        if clone_peer is not None:
            record['source_clone_prewrite_result']=broker.call(dict(op='source-clone-result'))
            if code!=0:
                lines=out.decode().splitlines()
                require(code==101 and lines.count('test '+SOURCE_CLONE_TEST+' ... FAILED')==1 and
                        sum(re.fullmatch(r'test result: FAILED\. 0 passed; 1 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in [0-9.]+s',line) is not None for line in lines)==1 and
                        record['source_clone_prewrite_result'].get('control_journal_dump_unchanged') is False,'SOURCE_CLONE_RED_BODY_UNCONFIRMED')
                record['outcome']=dict(test=name,exit_code=code,passed=0,failed=1,ignored=0,stdout_sha256=digest(out),stderr_sha256=digest(err))
                raise GateError('SOURCE_CLONE_OLD_ROUTE_PREWRITE_RED_NOT_ACCEPTANCE')
        record['outcome']=parse_test(code,out.decode(),err,name)
        if active:close_generation(runner,container,ident,broker,record['generations'][-1],'test_exit')
        if name in DESTINATION_TESTS:require(record['generations']==[] and runtime is None and source_fault is None,'DESTINATION_SOURCE_NO_ADMISSION')
        elif not legacy and clone_peer is None:require(record['generations'] and any(g['denial'] is not None for g in record['generations']) and
                              (not first or record.get('drain_committed_after_denial') is True),'ACTUAL_REFRESH_DRAIN_EVIDENCE')
        completed_case_audit(broker,record,binaries,code,out,err)
        immutable_before={key:value for key,value in record['audit_before'].items() if key!='roots'}
        immutable_after={key:value for key,value in record['audit_after'].items() if key!='roots'}
        equal_audit(immutable_before,immutable_after)
        record['pg_root_after']=root_audit(runner,container,record['audit_after']['roots']['control']['path'])
        equal_audit(record['pg_root_before'],record['pg_root_after'])
        equal_audit(record['pg_binary_before'],audit_binaries_in_pg(runner,container,record['compiled']))
        equal_audit(record['tools_before'],tool_check(runner,container,record=record,operation='tools_after'))
        inventory=broker.call(dict(op='inventory'));h.write_new(case/'inventory.json',canonical(inventory));record['inventory_sha256']=digest(canonical(inventory))
        runner.namespace.archives(container,inventory);runner.namespace.finalize()
        if first and name not in (*REGISTRY_TESTS,*SOURCE_FAULT_TESTS,*DESTINATION_TESTS,SOURCE_CLONE_TEST):record['independent_capture_audit']=real_index_postcheck(runner,container,record,inventory)
        record['acl_after']=sql(runner,container,"SELECT has_database_privilege('learning_runtime',oid,'CONNECT') FROM pg_database WHERE datname='"+ident['database']+"';")
        if name in DESTINATION_TESTS:
            record['destination_source_after']=destination_source_inventory(inventory)
            equal_audit(record['destination_source_before'],record['destination_source_after'])
            equal_audit(record['audit_before'],record['audit_after'])
            require(record['acl_after']==record['destination_acl_before']=='t','DESTINATION_SOURCE_ACL_CHANGED')
        if name in REGISTRY_INCOMPLETE_TESTS:require(record['acl_after']==('t' if name==REGISTRY_INCOMPLETE_TESTS[0] else 'f'),'FAULT_EXPECTED_ACL')
        if legacy:
            require(record['acl_after']=='f','LEGACY_NOT_CLOSED')
            recovered=[path for path in inventory if re.fullmatch(r'control/[0-9a-f-]{36}\.release-recovery/closed\.json',path)]
            require(len(recovered)==1,'LEGACY_RECOVERY_RECEIPT')
            actual=pg_exec(runner,container,['cat',ROOT+'/'+recovered[0]])[1]
            attempt=recovered[0].split('/')[1].removesuffix('.release-recovery')
            proof=dict(format_version=1,backup_id=attempt,database=ident['database'],result='runtime_connect_revoked_and_sessions_drained')
            require(actual==canonical(proof) and 'control/'+attempt+'.control/released.json' not in inventory,'LEGACY_CANONICAL_CLOSE_ONLY')
            result['legacy_close_regression']=dict(record['outcome'],classification=record['kind'],receipt_sha256=digest(actual),executed=True)
        record['target_sessions_after']=sql(runner,container,"SELECT count(*) FROM pg_stat_activity WHERE datname='"+ident['database']+"';")
        require(record['target_sessions_after']=='0','TARGET_SESSIONS_RETAINED')
        if _full_context is not None:_full_context._verify_body(record)
        record['stage']='completed'
    except BaseException as error:
        record['primary_reason']=str(error) if isinstance(error,GateError) else 'UNEXPECTED_ERROR'
        raise
    finally:
        # Transition once after a returned/failed RPC; never renew an in-flight
        # call. Failure stops exact PG before any transport grace/reap.
        runner.refresh_deadline=budget.refresh_deadline=None
        if broker is not None:broker.transition('CLEANUP')
        else:budget.transition('CLEANUP')
        registry.enter(budget)
        if clone_peer is not None:
            try:clone_peer.stop()
            except BaseException as error:
                record['clone_cleanup_error']=str(error) if isinstance(error,(GateError,ValueError)) else 'SOURCE_CLONE_STOP_UNCONFIRMED'
        record['phase_budget_history']=budget.history
        cleanup_errors=[]
        if _full_context is not None:
            try:_full_context._cleanup_target(record)
            except BaseException:cleanup_errors.append('FULL_TARGET_STOP_UNCONFIRMED')
        if source_fault is not None:
            try:source_fault.resume_before_stop()
            except BaseException as error:cleanup_errors.append(str(error) if isinstance(error,GateError) else 'SOURCE_FAULT_RESUME_UNKNOWN')
        if 'primary_reason' in record:
            state=LOG_RESERVATIONS[str(runner.runner.logs)]
            record['canceled_archive_requests']=sorted(state.get('archive_tokens',set()))
            state['archive_tokens']=set();state['archives']=0
            if 'namespace_audits' not in record:
                record['namespace_ledger_status']='NOT_FINALIZED_RAW_RETAINED'
                if runner.namespace.ledger_reserved:state['ledgers']-=1;runner.namespace.ledger_reserved=False
        for owner in registry.owners.values():
            if owner['intent']['kind']!='volume' or owner['intent']['case_id']!=ident['case_id'] or owner['state'] in ('retained','absent'):continue
            try:
                if owner['state']=='planned':registry.release(owner,'absent');continue
                facts=registry.reconcile(owner,'volume');receipt=owner['record']['cleanup_reconciliation']
                if facts is None:receipt.update(outcome='ABSENT');registry.release(owner,'absent')
                else:
                    plan=owner['intent']['expected']
                    require(facts['Name']==plan['name'] and facts['Driver']=='local' and not facts.get('Options') and facts['Labels']==plan['labels'],'VOLUME_IDENTITY')
                    receipt.update(outcome='MATCHED');owner['record']['retained_volume']=facts;registry.release(owner,'retained')
            except BaseException as error:owner['state']='unknown';cleanup_errors.append(str(error))
        def stop_pg():
            nonlocal expected
            if pg_owner is None or pg_owner['state']=='planned':
                if pg_owner is not None:registry.release(pg_owner,'absent')
                record.update(stopped=True,execs_absent=True,retained_network_empty=True);return
            if pg_owner['state']=='sent':
                facts=registry.reconcile(pg_owner);receipt=record['cleanup_reconciliation']
                if facts is None:
                    receipt.update(outcome='ABSENT');registry.release(pg_owner,'absent')
                    record.update(stopped=True,execs_absent=True,retained_network_empty=True);return
                network=registry.reconcile(pg_owner,'network',True)
                require(network is not None,'PG_NETWORK_DISCOVERY_UNKNOWN')
                expected=admit_created_pg(h,pg_owner,facts,network,record);registry.known(pg_owner,facts['Id'])
                receipt.update(outcome='MATCHED',discovered_id=facts['Id'])
            require(pg_owner['state']=='known' and expected is not None,'PG_CREATED_ID_UNKNOWN')
            clean=CleanupRunner(runner,pg_owner,'pg');h.validate_container(clean.inspect('container',expected['id']),expected)
            stop_error=None
            try:clean.docker('stop','--timeout','10',expected['id'],timeout=15)
            except BaseException as error:stop_error=error
            try:
                facts,network=pg_facts(h,clean,record,expected)
                require(facts['State']['Running'] is False and facts['State']['Pid']==0 and not facts.get('ExecIDs'),'PG_EXEC_CLEANUP_UNKNOWN')
                record['stopped']=True;record['execs_absent']=True
                require(not network.get('Containers'),'RETAINED_NETWORK_NOT_EMPTY')
                record['retained_network_empty']=True
                clean.docker('logs',expected['id']);registry.release(pg_owner,'stopped')
            except BaseException:pg_owner['state']='unknown';raise
            if stop_error is not None:raise stop_error
        helper_children={id(o.get('child')) for o in registry.owners.values() if o['intent']['kind']=='helper'}
        transports=[child for child in registry.children[child_start:] if id(child) not in helper_children and
                    not child.retire_attempted and not (child.finished and child.reason is None)]
        cleanup=teardown_transports(stop_pg,transports,failed='primary_reason' in record,
                                   timeout=min(TRANSPORT_TEARDOWN,max(0.001,registry.deadline-time.monotonic())))
        record['teardown']=cleanup;record['transports_reaped']=cleanup['transports_reaped'];cleanup_errors.extend(cleanup['errors'])
        if broker is not None:
            try:broker.close()
            except BaseException as error:cleanup_errors.append(str(error) if isinstance(error,GateError) else 'BROKER_CLEANUP_UNKNOWN')
        cleanup_errors.extend(cleanup_known_helpers(runner.runner,result))
        record['transports_reaped']=all(c.retired or (c.finished and c.p.poll() is not None and not any(t.is_alive() for t in c.threads)) for c in registry.children[child_start:])
        result['resource_creation_unknown']=any(o['state'] in ('planned','sent','unknown') for o in registry.owners.values())
        if cleanup_errors:
            record['cleanup_errors']=cleanup_errors
            if 'primary_reason' not in record:raise GateError('CASE_CLEANUP_UNKNOWN')


def final_volume_audit():
    """Read-only final retained bytes audit, invoked only in a pinned helper."""
    expected=json.loads(sys.argv[1],object_pairs_hook=unique_pairs)
    roots=expected['roots'];control=Path(roots['control']['path'])
    require(control in (Path(ROOT)/'control',Path(ROOT)/'control-held'),'FINAL_ROOT_PATH')
    for name,pin in roots.items():
        path=Path(pin['path']);require(path.parent==Path(ROOT) and path.name in (name,name+'-held'),'FINAL_ROOT_PATH')
        meta=path.lstat();require(stat.S_ISDIR(meta.st_mode) and meta.st_uid==0 and stat.S_IMODE(meta.st_mode)==0o700 and
                                (meta.st_dev,meta.st_ino)==(pin['dev'],pin['ino']),'FINAL_ROOT_IDENTITY')
    raw,_=private_read(control/'source-binding.json',4096)
    require(digest(raw)==expected['binding_sha256'],'FINAL_BINDING_HASH')
    binaries={}
    for binary,sha in expected['binaries'].items():
        require(re.fullmatch('/target/retained/[a-z0-9-]+',binary),'FINAL_BINARY_PATH')
        raw,_=private_read(Path(binary),128*1024**2,mode=0o500);require(digest(raw)==sha,'FINAL_BINARY_HASH');binaries[binary]=sha
    answer=dict(binding_sha256=expected['binding_sha256'],control_dev=roots['control']['dev'],control_ino=roots['control']['ino'],roots=roots,binaries=binaries)
    if len(sys.argv)==4:
        require(sys.argv[2]=='destination-task3','FINAL_DESTINATION_SELECTOR')
        previous=json.loads(sys.argv[3],object_pairs_hook=unique_pairs)
        require(destination_source_inventory(previous)==previous,'FINAL_DESTINATION_BASELINE')
        observed=destination_source_inventory(bounded_inventory(Path(ROOT)))
        equal_audit(previous,observed)
        answer=dict(audit=answer,destination_source_final=observed)
    elif len(sys.argv)==3:
        expected_unusable=json.loads(sys.argv[2],object_pairs_hook=unique_pairs)
        observed=final_fault_registry_audit(expected_unusable)
        answer=dict(audit=answer,registry_unusable=observed)
    else:require(len(sys.argv)==2,'FINAL_AUDIT_ARGUMENTS')
    print(canonical(answer).decode())


def final_audits(h,b,runner,result,images,source):
    runner.ownership.case_id=None;runner.ownership.deadline=None
    for record in result['cases']:
        log_capacity(runner.logs,54)
        require(record['stopped'] and record['execs_absent'] and record.get('audit_after'),'FINAL_CASE_NOT_COMPLETED')
        script="import sys;sys.path.insert(0,'/reviewed/scripts');from p0c4_source_lifecycle_gate_acceptance import final_volume_audit;final_volume_audit()"
        command=['python3','-c',script,canonical(record['audit_after']).decode()]
        if record['test'] in REGISTRY_INCOMPLETE_TESTS:
            require('registry_unusable_after' in record,'FINAL_REGISTRY_UNUSABLE_MISSING');command.append(canonical(record['registry_unusable_after']).decode())
        else:require('registry_unusable_after' not in record,'FINAL_REGISTRY_UNUSABLE_UNEXPECTED')
        if record['test'] in DESTINATION_TESTS:
            command.extend(['destination-task3',canonical(record['destination_source_before']).decode()])
        _,out,err=run_helper(h,b,runner,result,images[h.BUILDER],mounts(source,record['identity'],False,False),command)
        require(err==b'','FINAL_AUDIT_STDERR');observed=json.loads(out,object_pairs_hook=unique_pairs)
        if record['test'] in REGISTRY_INCOMPLETE_TESTS:
            require(set(observed)=={'audit','registry_unusable'} and canonical(observed['registry_unusable'])==canonical(record['registry_unusable_after']),'FINAL_REGISTRY_UNUSABLE_CHANGED')
            record['audit_final']=observed['audit'];record['registry_unusable_final']=observed['registry_unusable']
        elif record['test'] in DESTINATION_TESTS:
            require(type(observed) is dict and set(observed)=={'audit','destination_source_final'},'FINAL_DESTINATION_SHAPE')
            record['audit_final']=observed['audit'];record['destination_source_final']=destination_source_inventory(observed['destination_source_final'])
            equal_audit(record['destination_source_before'],record['destination_source_final'])
        else:record['audit_final']=observed
        equal_audit(record['audit_after'],record['audit_final'])


def cleanup_known_helpers(runner,result):
    """At most two owned helpers, one normal close and one protected fallback each."""
    registry=runner_base(runner).ownership;registry.enter();errors=[]
    for owner in registry.owners.values():
        if owner['intent']['kind']!='helper' or owner['record']['removed']:continue
        helper=owner['object']
        try:
            profile=getattr(helper,'_full_profile',None)
            if profile is not None:
                from p0c4_completion.full_target_fs import _HelperProfile
                from p0c4_completion.full_import import FullRehearsalContext
                context=getattr(registry,'_full_pair',None)
                require(type(profile) is _HelperProfile,'FULL_HELPER_PROFILE')
                controlled=getattr(registry,'_controlled_pair',None)
                if controlled is not None:
                    from p0c4_completion.controlled_fixture import ControlledFixtureContext
                    require(type(controlled) is ControlledFixtureContext and context is None and getattr(registry,'source_clone_pg_names',None) is None,'CONTROLLED_HELPER_PAIR')
                    if profile._controlled is controlled and profile._kind in ('controlled-owner','controlled-native') and controlled._retain_fs_owner(helper):
                        errors.append('CONTROLLED_OWNER_RETAINED')
                        controlled._retire_fs_owner_transport(helper)
                        continue
                if profile._kind=='owner' and type(context) is FullRehearsalContext and context._retain_fs_owner(helper):
                    errors.append('FULL_FS_OWNER_RETAINED')
                    context._retire_fs_owner_transport(helper)
                    continue
            if owner['state']=='planned':
                owner['record']['removed']=True;registry.release(owner,'absent');continue
            if owner['state']=='sent':
                facts=registry.reconcile(owner);receipt=owner['record']['cleanup_reconciliation']
                if facts is None:
                    receipt.update(outcome='ABSENT');owner['record']['removed']=True;registry.release(owner,'absent');continue
                identity=facts.get('Id');require(type(identity) is str and HEX64.fullmatch(identity),'HELPER_DISCOVERY_ID')
                helper.expected['id']=identity;helper.validate(facts)
                helper.record['id']=identity;registry.known(owner,identity);receipt.update(outcome='MATCHED',discovered_id=identity)
            require(owner['state']=='known','HELPER_DISCOVERY_UNKNOWN')
            for phase in ('close','fallback'):
                if helper.closed or owner.get(phase+'_attempted'):continue
                try:helper.close(phase)
                except BaseException as error:errors.append(str(error) if isinstance(error,GateError) else 'HELPER_CLEANUP_UNKNOWN')
        except BaseException as error:
            owner['state']='unknown';errors.append(str(error) if isinstance(error,GateError) else 'HELPER_DISCOVERY_UNKNOWN')
    result['resource_creation_unknown']=any(o['state'] in ('planned','sent','unknown') for o in registry.owners.values())
    return errors


def verify_prerequisite(raw,pin):
    require(HEX64.fullmatch(pin) and digest(raw)==pin,'PREREQUISITE_DIGEST')
    row=json.loads(raw,object_pairs_hook=unique_pairs)
    require(row.get('status')=='LINUX_PREREQUISITES_PASSED_NOT_PG' and row.get('exit_code')==0 and row.get('docker_attach_exit')==0 and
            row.get('builder_removed') is True and row.get('before_source_sha256')==row.get('after_source_sha256') and
            row.get('tests_pg_executed') is False and row.get('fs_ignored_executed') is False,'PREREQUISITE_RECEIPT')
    return row


SCOPES=(
    'real_capture_all_ready_and_retained_pin','finish_real_pin_after_sealed_rename',
    'finish_real_pin_from_pins_durable','finish_real_release_ready_before_and_after_grant',
    'abandon_early_and_late_attempts','abandon_crash_retry_and_terminal_ambiguity',
    'lifecycle_release_failure_compensates_same_session','lifecycle_admission_and_held_roots',
    'legacy-close','all')
ALL_SUCCESS='FOUR_FS_EIGHT_PG_LIFECYCLE_GATES_PASSED_NOT_COMPLETE_NOT_RESTORE'
LEAF_SUCCESS='SCOPED_LIFECYCLE_LEAF_PASSED_PARTIAL_SUITE_NOT_COMPLETE_NOT_RESTORE'


class SingleArgument(argparse.Action):
    def __call__(self,parser,namespace,value,option_string=None):
        marker='_seen_'+self.dest
        if getattr(namespace,marker,False):parser.error('duplicate '+option_string)
        setattr(namespace,marker,True);setattr(namespace,self.dest,value)


def parse_cli(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('archive','archive-sha256','manifest-sha256','runner-sha256','batch-id','prerequisite','prerequisite-sha256'):
        parser.add_argument('--'+name,required=True)
    parser.add_argument('--subnet',action='append',required=True,help='New unused private IPv4 /24s: eight for all, one or two for a fixed leaf.')
    parser.add_argument('--legacy-close-subnet',help='Optional ninth unused /24; one separately counted legacy close-only regression.')
    parser.add_argument('--scope',default='all',action=SingleArgument,help='Fixed scope: '+', '.join(SCOPES))
    parser.add_argument('--suite-id',action=SingleArgument,help='Canonical UUIDv4 required for a fixed non-all leaf; forbidden for all.')
    args=parser.parse_args(argv)
    for marker in ('_seen_scope','_seen_suite_id'):
        if hasattr(args,marker):delattr(args,marker)
    return args


def admit_schedule(args):
    require(args.scope in SCOPES,'SCOPE_UNKNOWN')
    if args.scope=='all':
        require(args.suite_id is None,'SUITE_ID_FORBIDDEN')
        require(len(args.subnet)==8,'EIGHT_SUBNETS_REQUIRED')
        return tuple((name,index==0,False) for index,name in enumerate(PG_TESTS))+(((LEGACY_TEST,False,True),) if args.legacy_close_subnet else ())
    v4(args.suite_id,'SUITE_UUID')
    legacy=args.scope=='legacy-close'
    require(bool(args.legacy_close_subnet)==legacy,'SCOPE_LEGACY_SUBNET')
    capture=args.scope==SCOPES[0]
    require(len(args.subnet)==(1 if capture or legacy else 2),'SCOPE_SUBNET_COUNT')
    if capture:return ((PG_TESTS[0],True,False),)
    primary=LEGACY_TEST if legacy else PG_TESTS[SCOPES.index(args.scope)]
    return ((PG_TESTS[0],True,False),(primary,False,legacy))


def suite_leaf(args,schedule):
    require(args.scope!='all' and schedule==admit_schedule(args),'SCOPE_SCHEDULE')
    return dict(format_version=1,suite_id=args.suite_id,scope=args.scope,primary_test=schedule[-1][0],
                executions=[dict(ordinal=index,test=name,role='primary' if index==len(schedule)-1 else 'prelude',first=first,legacy=legacy)
                            for index,(name,first,legacy) in enumerate(schedule)])


def scheduled_gates(args,schedule,result):
    cases=result['cases']
    gates=result.get('filesystem_gates',[])
    require(len(gates)==len(FS_TESTS) and
            all((record.get('package'),record.get('test'))==(package,name) and
                all(type(record.get(key)) is int and record[key]==expected for key,expected in dict(exit_code=0,passed=1,failed=0,ignored=0).items())
                for record,(package,name) in zip(gates,FS_TESTS)) and len(cases)==len(schedule) and
            all(record.get('test')==name and record.get('kind')==('LEGACY_CLOSE_ONLY_SYNTHETIC_PHASES_NO_DUMP' if legacy else 'REAL_LIFECYCLE') and
                record.get('outcome',{}).get('passed')==1 for record,(name,first,legacy) in zip(cases,schedule)), 'GATES_INCOMPLETE')
    if args.scope!='all':require(canonical(result.get('suite_leaf'))==canonical(suite_leaf(args,schedule)),'SUITE_LEAF_METADATA')


def completion_case_identity(case_id):
    v4(case_id);project='kwc4c-'+uuid.UUID(case_id).hex
    return dict(case_id=case_id,project=project,pg_name=project+'-pg-1',network=project+'-net',volume=project+'-pg',
                source_volume=project+'-source',build_volume=project+'-build',registry_volume=project+'-registry',
                database='learning_backup_c4_task3_'+case_id,other_database='learning_backup_c4_task3_'+str(uuid.uuid4()))


def owned_log_runner(h,batch,result,ledger_count):
    logs=batch/'evidence/logs'
    LOG_RESERVATIONS[str(logs)]=dict(archives=0,ledgers=ledger_count,pending=0,audits=0)
    class OwnedRunner(h.Runner):
        def run(self,command,**kwargs):
            prefix=begin_log_rpc(self)
            try:
                owner=getattr(self,'pending_create',None)
                controlled=getattr(self.ownership,'_controlled_pair',None)
                controlled_up=False
                if controlled is not None and owner is not None:
                    from p0c4_completion.controlled_fixture import ControlledFixtureContext
                    require(type(controlled) is ControlledFixtureContext and getattr(self.ownership,'_full_pair',None) is None and getattr(self.ownership,'source_clone_pg_names',None) is None,'CONTROLLED_CREATE_PAIR')
                    controlled_up=command[1:2]==['compose'] and 'up' in command and owner['intent']['kind']=='pg' and owner['intent']['name'] in controlled._owned_pg_names()
                if owner is not None and (command[1:2]==['create'] or command[1:3]==['volume','create'] or controlled_up or (command[1:2]==['compose'] and ('create' in command or 'up' in command and getattr(self.ownership,'_full_pair',None) is not None and owner['intent']['name'] in self.ownership._full_pair._owned_pg_names()))):
                    require(owner['state']=='planned','CREATE_INTENT_REUSED');owner['state']='sent'
                return super().run(command,**kwargs)
            finally:end_log_rpc(self,prefix)
    runner=OwnedRunner(logs);runner.ownership=CleanupReservations(runner,result)
    LOG_RESERVATIONS[str(logs)]['ownership']=runner.ownership
    runner.run=functools.partial(runner.run,timeout=5)
    return runner


def expected_raw_logs(runner,result):
    expected={f'{n:04d}.{suffix}' for n in range(1,runner.counter+1) for suffix in ('stdout','stderr','process.json')}
    for child in runner.ownership.children:
        expected.update(child.prefix.name+'.'+suffix for suffix in ('stdout','stderr'))
        if child.finished:expected.add(child.prefix.name+'.process.json')
        if child.retired:expected.add(child.prefix.name+'.teardown.json')
        require(child.retired or child.finished,'RAW_TRANSPORT_LIVE')
    for record in result['cases']:
        if 'namespace_audits' in record:expected.add(record['namespace_audits']['path'])
    return expected


def raw_log_records(logs,expected,budget):
    require(type(expected) is set and len(expected)<=4096 and all(re.fullmatch(r'(?:[0-9]{4,}|child-[0-9a-f]{32})\.(?:stdout|stderr|process\.json|teardown\.json)|namespace-audits-[0-9a-f-]{36}\.json',name) for name in expected),'RAW_KNOWN_LOG_NAMES')
    entries=[]
    with os.scandir(logs) as scan:
        for entry in scan:
            require(len(entries)<4096,'RAW_ENTRY_CAP');entries.append(entry.name)
    require(set(entries)==expected,'RAW_INVENTORY')
    rows=[]
    for name in sorted(expected):
        flags=os.O_RDONLY|getattr(os,'O_NOFOLLOW',0)|getattr(os,'O_NONBLOCK',0)
        try:fd=os.open(logs/name,flags)
        except OSError as error:raise GateError('RAW_PRIVATE_FILE') from error
        try:
            before=os.fstat(fd)
            require(stat.S_ISREG(before.st_mode) and before.st_uid==os.geteuid() and stat.S_IMODE(before.st_mode)==0o600 and before.st_nlink==1 and before.st_size<=LIMIT,'RAW_PRIVATE_FILE')
            require(budget[0]+before.st_size<=536870912,'RAW_SCAN_BYTES');budget[0]+=before.st_size
            value=hashlib.sha256();size=0
            while size<before.st_size:
                raw=os.read(fd,min(65536,before.st_size-size))
                require(bool(raw),'RAW_CHANGED')
                size+=len(raw);require(size<=before.st_size,'RAW_CHANGED');value.update(raw)
            after=os.fstat(fd)
            signature=lambda m:(m.st_dev,m.st_ino,m.st_uid,m.st_mode,m.st_nlink,m.st_size,m.st_mtime_ns,m.st_ctime_ns)
            require(size==before.st_size and signature(before)==signature(after),'RAW_CHANGED')
            os.fsync(fd);rows.append(dict(path='logs/'+name,size=size,sha256=value.hexdigest(),dev=before.st_dev,ino=before.st_ino))
        finally:os.close(fd)
    return rows


def publish_raw_index(batch,runner,result,*,budget=None):
    """Integrity for only known retained logs, separate from test-result credit."""
    try: from p0c4_storage_registry import rename_no_replace
    except ModuleNotFoundError: from scripts.p0c4_storage_registry import rename_no_replace
    expected=expected_raw_logs(runner,result);budget=budget if budget is not None else [0]
    scan_start=budget[0]
    files=raw_log_records(runner.logs,expected,budget)
    identities=dict(LOG_RESERVATIONS[str(runner.logs)].get('producer_identities',{}))
    for child in runner.ownership.children:identities.update(child.log_identities)
    require(set(identities)==expected and all(identities[Path(row['path']).name]==(row['dev'],row['ino']) for row in files),'RAW_PRODUCER_IDENTITY')
    require(raw_log_records(runner.logs,expected,budget)==files,'RAW_CHANGED')
    require(raw_log_records(runner.logs,expected,budget)==files,'RAW_INDEX_READBACK')
    value=dict(format_version=1,capability='c4_source_raw_index_v1',files=files,scan_bytes=budget[0]-scan_start);raw=canonical(value)
    require(len(raw)<=512*1024,'RAW_INDEX_BYTES')
    evidence=batch/'evidence';temporary='.raw-index-'+str(uuid.uuid4());new_private(evidence/temporary,raw)
    fd=os.open(evidence,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:rename_no_replace(fd,temporary,fd,'raw-index.json')
    finally:os.close(fd)
    readback,_=private_read(evidence/'raw-index.json',512*1024,uid=os.geteuid())
    require(readback==raw,'RAW_INDEX_READBACK')
    return dict(path='evidence/raw-index.json',size=len(raw),sha256=digest(raw))

def main():
    args=parse_cli();result=dict(status='FAILED',cases=[],helpers=[],cleanup_verified=False);batch=None;h=None;b=None;runner=None
    try:
        schedule=admit_schedule(args)
        if args.scope!='all':result['suite_leaf']=suite_leaf(args,schedule)
        require(sys.platform=='linux' and os.geteuid()!=0 and Path.home()==Path('/home/hans'),'ORDINARY_HANS_LINUX_REQUIRED')
        os.umask(0o077)
        path=Path(__file__).absolute();stage=path.parent;archive=Path(args.archive);prerequisite=Path(args.prerequisite)
        require(stage.parent==BASE and path.name==Path(ENTRY).name and archive.is_absolute() and archive.parent==stage and
                prerequisite.is_absolute() and prerequisite.parent==stage,'FIXED_STAGE_REQUIRED')
        v4(stage.name,'STAGE_UUID');v4(args.batch_id,'BATCH_UUID')
        for parent in (Path('/home/hans'),BASE,stage):
            meta=parent.lstat();require(stat.S_ISDIR(meta.st_mode) and meta.st_uid==os.geteuid() and not meta.st_mode&0o022,'STAGE_PARENT_IDENTITY')
        require(stat.S_IMODE(stage.lstat().st_mode)==0o700,'STAGE_PRIVATE')
        h,b,isolation=load_public_helpers(stage,private=True)
        require(digest(h.owned_file(path,(0o400,0o500)))==args.runner_sha256,'RUNNER_DIGEST')
        manifest,files=verify_package(h,h.owned_file(archive,(0o400,)),args.archive_sha256,args.manifest_sha256,args.runner_sha256)
        receipt=verify_prerequisite(h.owned_file(prerequisite,(0o400,)),args.prerequisite_sha256)
        # Prerequisite receipt has an older source package. Require every frozen
        # product pin in current package; carry earlier static evidence separately.
        requested=[*args.subnet]+([args.legacy_close_subnet] if args.legacy_close_subnet else [])
        subnets=h.admit_subnets(requested,[])
        batch=b.create_batch(h,stage,args.batch_id)
        result.update(batch_id=args.batch_id,base_commit=BASE_COMMIT,snapshot_kind=manifest['snapshot_kind'],archive_sha256=args.archive_sha256,
                      manifest_sha256=args.manifest_sha256,runner_sha256=args.runner_sha256,public_helper_pins=PUBLIC_PINS,product_pins=PRODUCT_PINS,
                      broker_sha256=digest(BROKER.encode()),prerequisite=dict(sha256=args.prerequisite_sha256,receipt=receipt,classification='CONTROLLER_PRIOR_STATIC_ONLY'),
                      classification='SOURCE_ATTEMPT_LIFECYCLE_REAL_LOCAL_CAPTURE_NOT_RESTORE',root_trust_scope='same_fresh_fixture_root',
                      exclusions=['offhost','independent_fault_domain','production','complete','restore','full_workspace_DB','four_old_version_upgrades',
                                  'broader_parent_drains','SQL_backend_fault_matrix','remaining_three_legacy_live_binding_entries_not_replayed'])
        if not args.legacy_close_subnet:result['legacy_close_regression']=dict(executed=False,reason='OPTIONAL_SEPARATE_SUBNET_NOT_SUPPLIED')
        source=batch/'source';b.extract_public_source(h,source,files)
        result['source_sha256_before']=b.source_digest(h,source,manifest)
        LOG_RESERVATIONS[str(batch/'evidence/logs')]=dict(archives=0,ledgers=len(schedule),pending=0,audits=0)
        class BoundedLogRunner(h.Runner):
            def run(self,command,**kwargs):
                prefix=begin_log_rpc(self)
                try:
                    owner=getattr(self,'pending_create',None)
                    if owner is not None and (command[1:2]==['create'] or command[1:3]==['volume','create'] or (command[1:2]==['compose'] and 'create' in command)):
                        require(owner['state']=='planned','CREATE_INTENT_REUSED');owner['state']='sent'
                    return super().run(command,**kwargs)
                finally:end_log_rpc(self,prefix)
        runner=BoundedLogRunner(batch/'evidence/logs')
        runner.ownership=CleanupReservations(runner,result)
        LOG_RESERVATIONS[str(runner.logs)]['ownership']=runner.ownership
        result['namespace_sampler_sha256']=PG_NAMESPACE_SAMPLER_SHA256
        # Compose only the default operation budget on this private instance.
        # Explicit build/migration waits keep their separately declared limits.
        runner.run=functools.partial(runner.run,timeout=5)
        require(len(subnets)==len(schedule),'SCOPE_SUBNET_ASSIGNMENT')
        identities=[identity() for _ in schedule]
        fresh_resource_preflight(h,runner,identities,subnets,result)
        planned=[ident[key] for ident in identities for key in ('volume','source_volume','build_volume','registry_volume')]
        admit_resources(planned,runner.docker('volume','ls','--format','{{.Name}}').decode().splitlines())
        images={ref:runner.inspect('image',ref) for ref in (h.PG_IMAGE,h.BUILDER)}
        require(images[h.BUILDER]['Id']==h.BUILDER and any(ref.split('@')[-1]==h.PG_IMAGE.split('@')[-1] for ref in images[h.PG_IMAGE].get('RepoDigests',[])),'CACHED_IMAGE_PIN')
        require(all(not any(key in h.env_dict(image['Config']['Env']) for key in (PIN_ENV,'KNOWWEAVE_C4_VERIFIER_KEY_SHA256','PGPASSWORD','DATABASE_URL')) for image in images.values()),'UNEXPECTED_IMAGE_ENV')
        result['image_ids']={ref:row['Id'] for ref,row in images.items()}
        require(os.statvfs(batch).f_bavail*os.statvfs(batch).f_frsize>=64*1024**3,'FREE_SPACE_BUDGET')
        result['preflight_complete']=True
        for ident,subnet,(name,first,legacy) in zip(identities,subnets,schedule):
            # Sequential only. Exception ends the batch; no failed gate replay.
            fresh_resource_preflight(h,runner,[ident],[subnet],result,ident['case_id'])
            live_case(h,b,isolation,runner,batch,source,ident,subnet,name,images,result,manifest,files,args.archive_sha256,first,legacy)
            require(b.source_digest(h,source,manifest)==result['source_sha256_before'],'SOURCE_CHANGED')
        scheduled_gates(args,schedule,result)
        final_audits(h,b,runner,result,images,source)
        result['source_sha256_after']=b.source_digest(h,source,manifest)
        equal_audit(result['source_sha256_before'],result['source_sha256_after'])
        result['status']=ALL_SUCCESS if args.scope=='all' else LEAF_SUCCESS
    except BaseException as error:
        token=str(error) if isinstance(error,GateError) else 'UNEXPECTED_ERROR'
        result.update(status='FAILED',reason=token,primary_reason=token)
        if result['cases']:result['failed_stage']=result['cases'][-1]['stage']
    finally:
        if batch is not None:
            try:
                if runner is not None:
                    errors=cleanup_known_helpers(runner,result)
                    if errors:result['global_cleanup_errors']=errors
                    result['registered_transports_reaped']=all(c.retired or (c.finished and c.p.poll() is not None and not any(t.is_alive() for t in c.threads)) for c in runner.ownership.children)
                    state=LOG_RESERVATIONS[str(runner.logs)]
                    if result['status']=='FAILED':
                        result['unstarted_namespace_ledgers_canceled']=state['ledgers'];state['ledgers']=0
                result['cleanup_verified']=cleanup_verified(result)
                if not result['cleanup_verified']:
                    result.setdefault('primary_reason',result.get('reason','CLEANUP_UNKNOWN'))
                    result.update(status='FAILED',reason='CLEANUP_UNKNOWN')
                b.finalize_result(h,batch,result)
            except BaseException:
                result.setdefault('primary_reason',result.get('reason','RESULT_PERSISTENCE_UNKNOWN'))
                result.update(status='FAILED',reason='RESULT_PERSISTENCE_UNKNOWN')
        # Console exposes only fixed tokens and nonsecret canonical receipts.
        print(canonical(result).decode())
    return 0 if result['status'] in (ALL_SUCCESS,LEAF_SUCCESS) else 1


if __name__=='__main__':
    sys.exit(main())
