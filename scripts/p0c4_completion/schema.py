"""Fixed schema-content lane contracts; no source/destination authority.

Pure validation is intentionally separate from resource operations. The lane
will reuse Root/Ops' reviewed fresh fixture recipe; it must not manufacture a
source admission issuer or pass source lifecycle checks for content evidence.
"""
from . import plan as envelope
from . import evidence
from . import schema_resources
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import secrets
import signal
import stat
import time
import uuid
if __package__.startswith('scripts.'):
    from .. import p0c4_storage_registry as registry
else:
    import p0c4_storage_registry as registry

CASE_TESTS={
    'full-schema-contract':'full_restore::live_tests::real_full_schema_contract',
    'full-schema-altered':'full_restore::live_tests::real_full_schema_altered',
    'full-copy-edge-data':'full_restore::live_tests::real_full_copy_edge_data',
}
PASS='REAL_PG18_SCHEMA_CONTENT_NOT_RESTORE'
CLIENT_SHA='eba5f8a8361918f873e273b1107dd727c19fc4269e8f98854ed5589555d668c6'
REASONS={'INPUT_REJECTED','SOURCE_MISMATCH','RESOURCE_UNKNOWN','BUILD_FAILED','MIGRATION_FAILED','CAPTURE_FAILED','BODY_FAILED','STOP_UNCONFIRMED','EVIDENCE_FAILED','DEADLINE'}
RECEIPT_KEYS={
    'format_version','capability','batch_id','case_id','case_name','plan_sha256',
    'source_archive_sha256','source_manifest_sha256','application_build_sha256',
    'application_commit','postgres_image','container_id','client_sha256',
    'migration_fingerprint','binary_sha256','test_name','test_exit','passed',
    'failed','ignored','capture_index','raw_index','resource_receipt','status','reason_code',
}

class SchemaError(envelope.CompletionError):pass

def require(ok,code):
    if not ok:raise SchemaError(code)

def validate_plan(plan,case):
    require(type(plan)is dict and set(plan)==envelope.PLAN_KEYS,'SCHEMA_PLAN_FIELDS')
    require(type(case)is str and case in CASE_TESTS and plan['case_name']==case and plan['capability']=='c4_completion_schema_plan_v1' and plan['budget_profile']=='full_schema_task4_v1','SCHEMA_FIXED_CASE')
    # Reuse only canonical source/image/path/UUID/subnet validation. This is not
    # an enrollment operation or an issuer; the old registry lane is never run.
    base=dict(plan,capability='c4_completion_case_plan_v1',case_name='registry-reopen',budget_profile='registry_task1_v1')
    try:envelope.validate_plan(base,'registry-reopen')
    except envelope.CompletionError as error:raise SchemaError('SCHEMA_PLAN_IDENTITY') from error
    return plan

def file_ref(value,path,maximum):
    require(type(value)is dict and set(value)=={'path','size','sha256'} and value['path']==path and registry.integer(value['size'],1,maximum) and type(value['sha256'])is str and registry.HEX64.fullmatch(value['sha256']),'SCHEMA_FILE_REF')

def validate_receipt(receipt,plan):
    """Shape/identity only. A PASS additionally needs independent raw readback.

    Returning FAILED_UNUSABLE stays failed even if all available files rehash.
    No repair, lifecycle authority, outcome upgrade, or publication is implied.
    """
    require(type(receipt)is dict and set(receipt)==RECEIPT_KEYS,'SCHEMA_RECEIPT_FIELDS')
    validate_plan(plan,receipt['case_name'])
    require(type(receipt['format_version'])is int and receipt['format_version']==1 and receipt['capability']=='c4_schema_content_receipt_v1','SCHEMA_RECEIPT_VERSION')
    for key in ('batch_id','case_id','case_name'):
        require(receipt[key]==plan[key],'SCHEMA_RECEIPT_CASE')
    source=plan['source']
    joins={
        'source_archive_sha256':source['archive']['sha256'],
        'source_manifest_sha256':source['manifest']['sha256'],
        'application_build_sha256':source['application_build_sha256'],
        'application_commit':source['application_commit'],
        'postgres_image':plan['images']['postgres'],
        'test_name':CASE_TESTS[receipt['case_name']],
    }
    require(all(receipt[key]==value for key,value in joins.items()),'SCHEMA_RECEIPT_BINDING')
    require(type(receipt['status'])is str and receipt['status'] in (PASS,'FAILED_UNUSABLE','UNCONFIRMED_UNUSABLE'),'SCHEMA_RECEIPT_STATUS')
    require(registry.integer(receipt['test_exit'],-1,255) and all(registry.integer(receipt[key],0,1) for key in ('passed','failed','ignored')),'SCHEMA_BODY_COUNTS')
    for key in ('plan_sha256','container_id','client_sha256','migration_fingerprint','binary_sha256'):
        require(type(receipt[key])is str and registry.HEX64.fullmatch(receipt[key]),'SCHEMA_RECEIPT_HASH')
    for key,path,cap in [('capture_index','capture/index.json',1024*1024),('raw_index','raw/index.json',512*1024),('resource_receipt','resources.json',4*1024*1024)]:
        if receipt[key]is not None:file_ref(receipt[key],path,cap)
    if receipt['status']==PASS:
        require(receipt['reason_code']=='NONE' and (receipt['test_exit'],receipt['passed'],receipt['failed'],receipt['ignored'])==(0,1,0,0),'SCHEMA_PASS_BODY')
        require(receipt['client_sha256']==CLIENT_SHA and all(receipt[key]!='0'*64 for key in ('container_id','migration_fingerprint','binary_sha256')),'SCHEMA_PASS_BINARY')
        require(all(receipt[key]is not None for key in ('capture_index','raw_index','resource_receipt')),'SCHEMA_PASS_EVIDENCE')
    else:
        require(type(receipt['reason_code'])is str and receipt['reason_code'] in REASONS,'SCHEMA_FAILURE_REASON')
    return receipt

def parse_body(code,raw,name):
    require(type(code)is int and code in (0,101) and name in CASE_TESTS.values(),'SCHEMA_BODY_EXIT')
    try:lines=raw.decode('utf-8').splitlines()
    except UnicodeError as error:raise SchemaError('SCHEMA_BODY_ENCODING') from error
    bodies=[line for line in lines if line.startswith('test ') and not line.startswith('test result:')]
    summaries=[line for line in lines if line.startswith('test result:')]
    label='ok' if code==0 else 'FAILED'
    require(bodies==['test '+name+' ... '+label] and len(summaries)==1 and lines.count('running 1 test')==1,'SCHEMA_BODY_NAME')
    counts='1 passed; 0 failed' if code==0 else '0 passed; 1 failed'
    require(re.fullmatch(r'test result: '+label+r'\. '+counts+r'; 0 ignored; 0 measured; [0-9]+ filtered out; finished in [0-9.]+s',summaries[0]),'SCHEMA_BODY_COUNTS')
    return dict(exit_code=code,passed=int(code==0),failed=int(code!=0),ignored=0)

def _modules():
    if __package__.startswith('scripts.'):
        from .. import p0c4_source_admission_gate_acceptance as h
        from .. import p0c4_source_binding_gate_acceptance as b
        from .. import p0c4_source_lifecycle_gate_acceptance as d
        from .. import p0c4_full_schema_contract as producer
    else:
        import p0c4_source_admission_gate_acceptance as h
        import p0c4_source_binding_gate_acceptance as b
        import p0c4_source_lifecycle_gate_acceptance as d
        import p0c4_full_schema_contract as producer
    return h,b,d,producer

EDGE_TITLE='中文🙂 \\N \\路径\n下一行\t\\. \\connect other; COMMIT;'
EDGE_REASON='引用\\回车\r换行\n制表\tCOMMIT;'
EDGE_SQL="""BEGIN;
INSERT INTO public.app_user(id) VALUES (:'schema_case_id'::uuid);
INSERT INTO public.space(id,owner_id) VALUES (:'schema_case_id'::uuid,:'schema_case_id'::uuid);
INSERT INTO public.composition(id,space_id,kind,head_revision_id,created_at)
VALUES (:'schema_case_id'::uuid,:'schema_case_id'::uuid,'document',:'schema_case_id'::uuid,'2026-01-01 00:00:00+00');
INSERT INTO public.composition_revision(id,space_id,composition_id,parent_revision_id,kind,title,content_sha256,author_id,reason,created_at)
VALUES (:'schema_case_id'::uuid,:'schema_case_id'::uuid,:'schema_case_id'::uuid,NULL,'document',
convert_from(decode('"""+EDGE_TITLE.encode().hex()+"""','hex'),'UTF8'),repeat('0',64),:'schema_case_id'::uuid,
convert_from(decode('"""+EDGE_REASON.encode().hex()+"""','hex'),'UTF8'),'2026-01-01 00:00:00+00');
COMMIT;
"""
VARIANTS=(
    ('extra','CREATE TABLE public.c4_schema_reject_extra (id integer);','DROP TABLE public.c4_schema_reject_extra;'),
    ('function','ALTER FUNCTION public.p0c3_canonical_json(jsonb) SECURITY DEFINER;','ALTER FUNCTION public.p0c3_canonical_json(jsonb) SECURITY INVOKER;'),
    ('acl','GRANT UPDATE ON public.app_user TO learning_runtime;','REVOKE UPDATE ON public.app_user FROM learning_runtime;'),
)

def _fixture_variants(case_name,case_id,container,database,capture,record):
    _,_,_,p=_modules()
    def collect(name,operation,cap,query=None,archive=None,edge=False):
        command=p.fixed_command(container,database,operation)
        if edge:command+=['-v','schema_case_id='+case_id]
        with (capture/name).open('xb') as output:
            if archive:
                with (capture/archive).open('rb') as inp:p.collect(command,output,cap,input_file=inp)
            else:p.collect(command,output,cap,input_bytes=query.encode() if query else None)
            output.flush();os.fsync(output.fileno())
        return (capture/name).read_bytes() if cap<=p.MAX_METADATA else None
    if case_name=='full-schema-contract':return
    if case_name=='full-copy-edge-data':
        collect('edge-seed.stdout','catalog',8192,EDGE_SQL,edge=True)
        require((capture/'edge-seed.stdout').read_bytes()==b'','SCHEMA_EDGE_SEED_OUTPUT')
        collect('edge.dump','dump',p.MAX_DUMP)
        collect('edge-copy.bin','catalog',8192,'COPY public.composition_revision TO STDOUT;')
        record['fixture_variants']=['edge'];return
    require(case_name=='full-schema-altered','SCHEMA_FIXED_CASE')
    baseline_owned=p.normalize_restrict((capture/'owned-schema.raw.sql').read_bytes())
    baseline_acl=json.loads((capture/'acl.raw.json').read_bytes())
    baseline_counts=json.loads((capture/'counts.raw.json').read_bytes())
    require(next(f for f in baseline_acl['functions'] if f['name']=='p0c3_canonical_json')['security_definer'] is False,'SCHEMA_FUNCTION_BASELINE')
    access=collect('acl-precondition.json','catalog',8192,"SELECT has_table_privilege('learning_runtime','public.app_user','UPDATE');")
    require(access==b'f\n','SCHEMA_UPDATE_BASELINE')
    columns=json.loads((capture/'columns.json').read_bytes())
    count_sql='SELECT json_build_array('+','.join('(SELECT count(*) FROM public.'+p.native_identifier(t['table'],t['sql_name'])+')' for t in columns)+');'
    record['fixture_variants']=[]
    for name,change,restore in VARIANTS:
        require(collect(name+'-change.stdout','catalog',8192,change)==b'','SCHEMA_VARIANT_OUTPUT')
        collect('bad-'+name+'.dump','dump',p.MAX_DUMP)
        require(collect(name+'-restore.stdout','catalog',8192,restore)==b'','SCHEMA_VARIANT_OUTPUT')
        collect('restored-'+name+'.dump','dump',p.MAX_DUMP)
        owned=collect('restored-'+name+'.owned.sql','owned-schema',p.MAX_METADATA,archive='restored-'+name+'.dump')
        acl=collect('restored-'+name+'.acl.json','catalog',p.MAX_METADATA,p.ACL_SQL)
        counts=collect('restored-'+name+'.counts.json','catalog',p.MAX_METADATA,count_sql)
        require(p.normalize_restrict(owned)==baseline_owned and p.canonical(json.loads(acl))==p.canonical(baseline_acl) and p.canonical(json.loads(counts))==p.canonical(baseline_counts),'SCHEMA_VARIANT_NOT_RESTORED')
        record['fixture_variants'].append(dict(name=name,owned_sha256=p.sha(p.normalize_restrict(owned)),catalog_sha256=p.sha(p.canonical(json.loads(acl))),counts_sha256=p.sha(p.canonical(json.loads(counts)))))

def _compile_artifacts(helper,d,b,case_id,env):
    answer={}
    for kind,command in [('lib',['cargo','test','--locked','--offline','-p','learning-backup','--lib','--no-run','--message-format=json']),('example',['cargo','build','--locked','--offline','-p','learning-backup','--example','c4_task3_migrate','--message-format=json'])]:
        code,out,err=helper(command,env)
        artifact=d.discover_artifact(out.decode(),'learning-backup',kind)
        profile=d.validate_compiler_profile(out.decode(),artifact,kind=='lib')
        intermediate='/target/materialized/'+case_id+'-'+kind
        _,raw,errors=helper(['python3','-c','import sys;sys.path.insert(0,"/reviewed/scripts");from p0c4_source_lifecycle_gate_acceptance import materialize_main;materialize_main()',artifact,intermediate])
        require(errors==b'','SCHEMA_MATERIALIZE_STDERR')
        material=validate_materialization(d,json.loads(raw,object_pairs_hook=d.unique_pairs),artifact,intermediate)
        binary='/target/retained/'+case_id+('-test' if kind=='lib' else '-migrate')
        _,raw,errors=helper(['python3','-c',b.COPY_BINARY,intermediate,binary])
        copied=json.loads(raw,object_pairs_hook=d.unique_pairs)
        require(errors==b'' and type(copied)is dict and set(copied)=={'binary_sha256'} and copied['binary_sha256']==material['sha256'],'SCHEMA_RETAINED_BINARY')
        answer[kind]=dict(binary=binary,sha256=material['sha256'],artifact=artifact,profile=profile,materialization=material,compiler_exit=code,compiler_stdout_sha256=envelope.digest(out),compiler_stderr_sha256=envelope.digest(err))
    return answer

# Per-file dump/decoded limits are unchanged. This derived evidence-tree cap
# bounds the finite baseline + three mutation/restoration snapshots + spools.
MAX_OUTPUT_BYTES=32*1024**3
MAX_OUTPUT_FILES=512
def _output_tree(path):
    """Handle-relative, no-follow, bounded hashing of only this fresh output."""
    root_fd=registry.open_directory(str(path));rows=[];total=0;entries=0
    def walk(fd,prefix,depth):
        nonlocal total,entries
        require(depth<=4,'SCHEMA_OUTPUT_DEPTH')
        names=registry.names(fd,MAX_OUTPUT_FILES)
        for name in names:
            entries+=1;require(entries<=MAX_OUTPUT_FILES,'SCHEMA_OUTPUT_ENTRIES')
            require(re.fullmatch(r'[A-Za-z0-9_.-]{1,120}',name) and name not in ('.','..'),'SCHEMA_OUTPUT_NAME')
            before=os.stat(name,dir_fd=fd,follow_symlinks=False);mode=stat.S_IMODE(before.st_mode)
            require(before.st_uid==0,'SCHEMA_OUTPUT_OWNER')
            relative=prefix+name
            if stat.S_ISDIR(before.st_mode):
                require(mode in (0o700,0o500),'SCHEMA_OUTPUT_DIRECTORY')
                child=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd)
                try:walk(child,relative+'/',depth+1)
                finally:os.close(child)
                continue
            require(stat.S_ISREG(before.st_mode) and before.st_nlink==1 and mode in (0o600,0o400) and len(rows)<MAX_OUTPUT_FILES,'SCHEMA_OUTPUT_FILE')
            cap=4*1024**3 if name=='decoded' else 1024**3 if name.endswith('.dump') or name=='dump' else 4*1024**2
            require(before.st_size<=cap and total+before.st_size<=MAX_OUTPUT_BYTES,'SCHEMA_OUTPUT_BYTES')
            handle=os.open(name,os.O_RDONLY|os.O_NOFOLLOW,dir_fd=fd)
            try:
                opened=os.fstat(handle);require((opened.st_dev,opened.st_ino)==(before.st_dev,before.st_ino),'SCHEMA_OUTPUT_REPLACED')
                digest=hashlib.sha256();size=0
                while True:
                    raw=os.read(handle,65536)
                    if not raw:break
                    size+=len(raw);require(size<=before.st_size,'SCHEMA_OUTPUT_CHANGED');digest.update(raw)
                after=os.fstat(handle)
                require((after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns)==(before.st_dev,before.st_ino,before.st_size,before.st_mtime_ns,before.st_ctime_ns) and size==before.st_size,'SCHEMA_OUTPUT_CHANGED')
                rows.append(dict(path=relative,size=size,sha256=digest.hexdigest(),mode=mode,dev=before.st_dev,ino=before.st_ino));total+=size
            finally:os.close(handle)
    try:walk(root_fd,'',0)
    finally:os.close(root_fd)
    return dict(format_version=1,capability='c4_schema_output_index_v1',files=sorted(rows,key=lambda r:r['path']),bytes=total)

def _batch(root,plan):return root/'schema-run'/('batch-'+plan['batch_id'])

def _read_stream(root,plan,domain,name,rows,budget):
    """Read only indexed raw/native streams, including actual zero-byte files.

    Control records deliberately keep evidence.private_read's nonempty rule.
    Domain, directory, names and capacities are fixed here, never caller caps.
    All directory handles stay held through the final identity rechecks.
    """
    require(domain in ('raw','native') and Path(root)==Path(plan['evidence_root']) and plan['case_name'] in CASE_TESTS,'SCHEMA_STREAM_DOMAIN')
    registry.v4(plan['batch_id']);registry.v4(plan['case_id'])
    require(plan['batch_id']!=plan['case_id'] and type(name)is str and type(rows)is dict,'SCHEMA_STREAM_DOMAIN')
    if domain=='raw':
        require(re.fullmatch(r'(?:[0-9]{4,}|child-[0-9a-f]{32})\.(?:stdout|stderr)',name),'SCHEMA_STREAM_NAME')
        parts=('schema-run','batch-'+plan['batch_id'],'evidence','logs');relative='logs/'+name
        cap=2*1024**2;maximum=4096;total_cap=536870912;keys={'path','size','sha256','dev','ino'}
    else:
        require(plan['case_name']=='full-schema-contract' and re.fullmatch(r'native-(?:receiver_cancel|deadline|output_cap|native_exit)\.(?:stdout|stderr)',name),'SCHEMA_STREAM_NAME')
        parts=('schema-run','batch-'+plan['batch_id'],plan['case_id'],'schema','native-supervisor');relative='native-supervisor/'+name
        cap=1 if name=='native-output_cap.stdout' else 8192
        maximum=MAX_OUTPUT_FILES;total_cap=7*8192+1;keys={'path','size','sha256','dev','ino','mode'}
    require(len(rows)<=maximum and relative in rows,'SCHEMA_STREAM_INDEX')
    row=rows[relative]
    require(type(row)is dict and set(row)==keys and row['path']==relative and registry.integer(row['size'],0,cap) and type(row['sha256'])is str and registry.HEX64.fullmatch(row['sha256']) and registry.integer(row['dev'],1) and registry.integer(row['ino'],1),'SCHEMA_STREAM_INDEX')
    if domain=='native':require(type(row['mode'])is int and row['mode']==0o600,'SCHEMA_STREAM_INDEX')
    require(type(budget)is list and len(budget)==1 and registry.integer(budget[0],0,total_cap) and budget[0]+row['size']<=total_cap,'SCHEMA_STREAM_BUDGET')
    budget[0]+=row['size']
    def signature(meta):
        return (meta.st_dev,meta.st_ino,meta.st_uid,meta.st_mode,meta.st_nlink,meta.st_size,meta.st_mtime_ns,meta.st_ctime_ns)
    def private(meta):
        require(stat.S_ISREG(meta.st_mode) and meta.st_uid==0 and stat.S_IMODE(meta.st_mode)==0o600 and meta.st_nlink==1,'SCHEMA_STREAM_PRIVATE')
        require((meta.st_dev,meta.st_ino,meta.st_size)==(row['dev'],row['ino'],row['size']),'SCHEMA_STREAM_IDENTITY')
    held=[];file=None
    try:
        parent=registry.open_directory(str(root));held.append((None,None,parent,signature(os.fstat(parent))))
        for part in parts:
            child=registry.child_directory(parent,part);held.append((parent,part,child,signature(os.fstat(child))));parent=child
        file=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC,dir_fd=parent)
        before=os.stat(name,dir_fd=parent,follow_symlinks=False);private(before)
        opened=os.fstat(file);private(opened);require(signature(opened)==signature(before),'SCHEMA_STREAM_CHANGED')
        value=hashlib.sha256();chunks=[];size=0
        while True:
            raw=os.read(file,min(65536,row['size']-size+1))
            if not raw:break
            size+=len(raw);require(size<=row['size'],'SCHEMA_STREAM_CHANGED');value.update(raw);chunks.append(raw)
        require(size==row['size'] and value.hexdigest()==row['sha256'],'SCHEMA_STREAM_HASH')
        require(signature(os.fstat(file))==signature(before) and signature(os.stat(name,dir_fd=parent,follow_symlinks=False))==signature(before),'SCHEMA_STREAM_CHANGED')
        for ancestor,part,handle,expected in held:
            require(signature(os.fstat(handle))==expected,'SCHEMA_STREAM_DIRECTORY_CHANGED')
            if ancestor is not None:require(signature(os.stat(part,dir_fd=ancestor,follow_symlinks=False))==expected,'SCHEMA_STREAM_DIRECTORY_CHANGED')
        reopened=registry.open_directory(str(root))
        try:require(signature(os.fstat(reopened))==held[0][3],'SCHEMA_STREAM_DIRECTORY_CHANGED')
        finally:os.close(reopened)
        return b''.join(chunks)
    finally:
        if file is not None:os.close(file)
        for _,_,handle,_ in reversed(held):os.close(handle)

def _read_content(root,plan,rows,name,native_budget):
    require(name in rows and rows[name]['size']<=4*1024**2,'SCHEMA_CONTENT_INPUT')
    if name.startswith('native-supervisor/'):
        return _read_stream(root,plan,'native',name.removeprefix('native-supervisor/'),rows,native_budget)
    output=_batch(root,plan)/plan['case_id']/'schema'
    raw=evidence.private_read(output/name,4*1024**2)
    require(len(raw)==rows[name]['size'] and envelope.digest(raw)==rows[name]['sha256'],'SCHEMA_CONTENT_HASH');return raw

def validate_identity(identity,case_id):
    project='kwc4c-'+uuid.UUID(case_id).hex
    expected=dict(case_id=case_id,project=project,pg_name=project+'-pg-1',network=project+'-net',volume=project+'-pg',source_volume=project+'-source',build_volume=project+'-build',registry_volume=project+'-registry',database='learning_backup_c4_task3_'+case_id)
    require(type(identity)is dict and set(identity)==set(expected)|{'other_database'} and all(identity[k]==v for k,v in expected.items()),'SCHEMA_RESOURCE_IDENTITY')
    other=identity['other_database'];prefix='learning_backup_c4_task3_'
    require(type(other)is str and other.startswith(prefix) and other!=identity['database'],'SCHEMA_SETUP_DATABASE')
    try:registry.v4(other.removeprefix(prefix))
    except registry.RegistryError as error:raise SchemaError('SCHEMA_SETUP_DATABASE') from error

def join_process(originals,row):
    """Join one output pair to an actual completed original process record."""
    matches=[]
    for out,err,process in originals:
        if envelope.digest(out)!=row['stdout_sha256'] or envelope.digest(err)!=row['stderr_sha256']:continue
        if type(process)is not dict or process.get('reason') is not None:continue
        if type(process.get('exit_code'))is not int or process['exit_code']!=row['exit_code']:continue
        if any(type(process.get(k))is not int or process[k]!=v for k,v in [('stdout_bytes',len(out)),('stderr_bytes',len(err))]):continue
        matches.append(out)
    require(bool(matches),'SCHEMA_ORIGINAL_PROCESS_JOIN')
    return matches[0]

def _capture_contract(read,source,plan,container):
    """Regenerate the fixed canonical projections from retained native bytes."""
    _,_,_,p=_modules()
    contract=registry.parse_record(read('capture/contract.json'))
    expected_source=dict(case_id=plan['case_id'],container_id=container,database='learning_backup_c4_task3_'+plan['case_id'],application_commit=plan['source']['application_commit'],application_build_sha256=plan['source']['application_build_sha256'])
    require(contract['source']==expected_source and contract['producer_sha256']==p.sha((source/'scripts/p0c4_full_schema_contract.py').read_bytes()),'SCHEMA_CAPTURE_SOURCE')
    migrations=p.migration_identity(source)
    observed=json.loads(read('capture/migrations.raw.json'))
    require(all(row.pop('success',None)is True for row in observed),'SCHEMA_MIGRATION_SUCCESS');p.require_migrations(observed,migrations)
    require(contract['migrations']==migrations and contract['migration_fingerprint']==p.sha(json.dumps(migrations,separators=(',',':')).encode()),'SCHEMA_MIGRATION_FINGERPRINT')
    columns=json.loads(read('capture/columns.raw.json'));acl=json.loads(read('capture/acl.raw.json'))
    require(len(columns)==64 and json.loads(read('capture/counts.raw.json'))==[15 if t['table']=='_sqlx_migrations' else 0 for t in columns],'SCHEMA_CAPTURE_EMPTY')
    template,data=p.split_copy(read('capture/decoded.raw.sql'),columns)
    canonical={'pg18-schema.sql.in':p.normalize_restrict(template),'pg18-toc.json':p.canonical(p.normalize_toc(read('capture/toc.raw.txt'))),'columns.json':p.canonical(columns),'acl.json':p.canonical(dict(catalog=acl,owned_schema_template=p.normalize_restrict(read('capture/owned-schema.raw.sql')).decode()))}
    require(contract['migration_copy_sha256']==p.sha(data['_sqlx_migrations']),'SCHEMA_MIGRATION_COPY')
    require(set(contract['files'])==set(canonical),'SCHEMA_CAPTURE_ARTIFACTS')
    for name,raw in canonical.items():
        require(read('capture/'+name)==raw and contract['files'][name]==dict(size=len(raw),sha256=p.sha(raw)),'SCHEMA_CAPTURE_CANONICAL')
        require((source/'crates/learning-backup/tests/fixtures/c4-full-schema'/name).read_bytes()==raw,'SCHEMA_CAPTURE_REVIEWED_CONTRACT')
    expected=dict(format_version=1,capability='full_schema_content_v1',postgres_image=p.PG_IMAGE,postgres_version=p.PG_VERSION,producer_sha256=p.sha((source/'scripts/p0c4_full_schema_contract.py').read_bytes()),source=expected_source,migrations=migrations,migration_fingerprint=contract['migration_fingerprint'],files={name:dict(size=len(raw),sha256=p.sha(raw)) for name,raw in canonical.items()},migration_copy_sha256=p.sha(data['_sqlx_migrations']))
    require(p.canonical(contract)==p.canonical(expected),'SCHEMA_CAPTURE_CONTRACT')
    return contract

def _verify_contents(root,receipt,plan):
    h,b,d,p=_modules();batch=_batch(root,plan);output=batch/plan['case_id']/'schema'
    # Rehash the accepted immutable source, all source files, original process
    # outputs and the fresh output tree; never execute a command during readback.
    source=plan['source'];archive=evidence.private_read(source['archive']['path'],32*1024**2)
    require(len(archive)==source['archive']['size'] and p.sha(archive)==source['archive']['sha256'],'SCHEMA_SOURCE_ARCHIVE')
    manifest,files=h.verify_package(archive,source['archive']['sha256'],source['manifest']['sha256'],p.sha(Path(h.__file__).read_bytes()),'green')
    require(evidence.private_read(source['manifest']['path'],2*1024**2)==envelope.canonical(manifest) and manifest['base_commit']==source['application_commit'],'SCHEMA_SOURCE_MANIFEST')
    source_digest=b.source_digest(h,root/'code',manifest)
    result=evidence.parse_driver_result(evidence.relative_ref(root,receipt['resource_receipt'],4*1024**2))
    require(result['batch_id']==plan['batch_id'] and result['case_id']==plan['case_id'] and result['base_commit']==source['application_commit'] and result['archive_sha256']==source['archive']['sha256'] and result['classification']==PASS and len(result['cases'])==1,'SCHEMA_RESOURCE_JOIN')
    row=result['cases'][0];validate_identity(row['identity'],plan['case_id']);require(row['test']==CASE_TESTS[plan['case_name']],'SCHEMA_CASE_JOIN')
    originals=[]
    if receipt['raw_index']is not None:
        wrapper=registry.parse_record(evidence.relative_ref(root,receipt['raw_index'],512*1024))
        require(set(wrapper)=={'format_version','capability','original_index'} and wrapper['format_version']==1 and wrapper['capability']=='c4_schema_raw_index_v1' and wrapper['original_index']['path']==(batch/'evidence/raw-index.json').relative_to(root).as_posix(),'SCHEMA_RAW_INDEX')
        index=registry.parse_record(evidence.relative_ref(root,wrapper['original_index'],512*1024))
        require(set(index)=={'format_version','capability','files','scan_bytes'} and index['format_version']==1 and index['capability']=='c4_source_raw_index_v1','SCHEMA_RAW_INDEX')
        expected={Path(item['path']).name for item in index['files']}
        require(len(expected)==len(index['files']) and index['scan_bytes']==3*sum(item['size'] for item in index['files']),'SCHEMA_RAW_COUNTS')
        logs=batch/'evidence/logs'
        raw_budget=[0]
        require(d.raw_log_records(logs,expected,raw_budget)==index['files'],'SCHEMA_RAW_READBACK')
        raw_rows={item['path']:item for item in index['files']}
        for name in sorted(expected):
            if not re.fullmatch(r'(?:[0-9]{4,}|child-[0-9a-f]{32})\.process\.json',name):continue
            prefix=name.removesuffix('.process.json')
            require({prefix+'.stdout',prefix+'.stderr'}<=expected,'SCHEMA_RAW_PAIR')
            stdout=_read_stream(root,plan,'raw',prefix+'.stdout',raw_rows,raw_budget)
            stderr=_read_stream(root,plan,'raw',prefix+'.stderr',raw_rows,raw_budget)
            process_size=raw_rows['logs/'+name]['size']
            require(raw_budget[0]+process_size<=536870912,'SCHEMA_STREAM_BUDGET');raw_budget[0]+=process_size
            originals.append((stdout,stderr,registry.parse_record(evidence.private_read(logs/name,8192))))
    inventory=None
    if receipt['capture_index']is not None:
        inventory=registry.parse_record(evidence.relative_ref(root,receipt['capture_index'],1024*1024))
        require(_output_tree(output)==inventory,'SCHEMA_OUTPUT_READBACK')
    cleanup=schema_cleanup_verified(result)
    require(type(result.get('cleanup_verified'))is bool and (not result['cleanup_verified'] or cleanup),'SCHEMA_CLEANUP_CLAIM')
    # Failure does not waive original resource evidence. Partial creation may
    # lack a PG/network, but every positive claim still needs its own original.
    _verify_resource_originals(originals,row,plan,partial=receipt['status']!=PASS and not result['cleanup_verified'])
    if receipt['status']!=PASS:
        # A late integrity failure can demote an otherwise completed execution.
        require(receipt['reason_code']!='NONE','SCHEMA_FAILURE_REASON');return
    require(result['status']==PASS and result['reason_code']=='NONE' and result['cleanup_verified'] is True and schema_cleanup_verified(result) and result.get('registered_transports_reaped')is True,'SCHEMA_RESOURCE_PASS')
    require(result['source_sha256_before']==result['source_sha256_after']==source_digest,'SCHEMA_SOURCE_UNCHANGED')
    require(evidence.private_read(batch/'evidence/result.json',4*1024**2)==envelope.canonical(result),'SCHEMA_ORIGINAL_RESULT')
    require(row['container_id']==receipt['container_id'],'SCHEMA_CONTAINER_JOIN')
    body=join_process(originals,row['body']);outcome=parse_body(row['body']['exit_code'],body,receipt['test_name'])
    require(row['body']['stderr_bytes']==0 and row['outcome']==outcome and all(receipt[k]==outcome[k if k!='test_exit' else 'exit_code'] for k in ('test_exit','passed','failed','ignored')),'SCHEMA_BODY_JOIN')
    require(join_process(originals,dict(row['migrate'],exit_code=row['migrate']['exit']))==b'C4_TASK3_MIGRATED\n','SCHEMA_MIGRATOR_JOIN')
    for kind,value in row['compiled'].items():
        require(kind in ('lib','example') and value['compiler_exit']==0,'SCHEMA_COMPILER_EXIT')
        raw=join_process(originals,dict(exit_code=value['compiler_exit'],stdout_sha256=value['compiler_stdout_sha256'],stderr_sha256=value['compiler_stderr_sha256']))
        artifact=d.discover_artifact(raw.decode(),'learning-backup',kind)
        require(artifact==value['artifact'] and d.validate_compiler_profile(raw.decode(),artifact,kind=='lib')==value['profile'],'SCHEMA_COMPILER_PROFILE')
        validate_materialization(d,value['materialization'],artifact,'/target/materialized/'+plan['case_id']+'-'+kind)
        require(value['sha256']==value['materialization']['sha256'],'SCHEMA_BINARY_MATERIALIZATION')
    require(set(row['compiled'])=={'lib','example'} and row['compiled']['lib']['sha256']==receipt['binary_sha256'] and row['pg_binary_before']==row['pg_binary_after']=={v['binary']:v['sha256'] for v in row['compiled'].values()},'SCHEMA_BINARY_JOIN')
    require(row['tools_before']==row['tools_after'] and row['tools_before']['pg_restore']['sha256']==receipt['client_sha256']==CLIENT_SHA,'SCHEMA_CLIENT_JOIN')
    successful=[out for out,err,proc in originals if not err and type(proc.get('exit_code'))is int and proc['exit_code']==0 and proc.get('reason')is None]
    for value in row['compiled'].values():
        require(sum(out==envelope.canonical(value['materialization'])+b'\n' for out in successful)>=1,'SCHEMA_MATERIALIZATION_ORIGINAL')
        expected=('0 500 1 '+str(value['materialization']['bytes'])+'\n'+value['sha256']+'\n').encode()
        require(successful.count(expected)>=2,'SCHEMA_BINARY_ORIGINAL')
    tools=[]
    for raw in successful:
        try:lines=raw.decode().splitlines()
        except UnicodeError:continue
        if len(lines)!=6:continue
        expected=row['tools_before'];valid=True
        for offset,name in ((0,'pg_dump'),(3,'pg_restore')):
            fields=lines[offset].split()
            valid=valid and len(fields)==3 and fields[0]=='0' and re.fullmatch(r'[0-7]{3,4}',fields[1]) and int(fields[1],8)&0o022==0 and fields[2]=='1' and lines[offset+1]==expected[name]['version'] and lines[offset+2]==expected[name]['sha256']+'  '+expected[name]['path']
        if valid:tools.append(raw)
    require(len(tools)>=2 and tools[0]==tools[-1] and row['tools_before']['pg_restore']['version']=='pg_restore (PostgreSQL) '+p.PG_VERSION,'SCHEMA_CLIENT_ORIGINAL')
    rows={item['path']:item for item in inventory['files']}
    native_budget=[0]
    def read(name):return _read_content(root,plan,rows,name,native_budget)
    capture=registry.parse_record(read('capture/capture.json'))
    require(set(capture)=={'status','files'} and capture['status']=='CANDIDATE_REQUIRES_REVIEW_NOT_RESTORE_AUTHORITY' and set(capture['files'])=={'migrations.raw.json','columns.raw.json','counts.raw.json','acl.raw.json','database.dump','toc.raw.txt','decoded.raw.sql','owned-schema.raw.sql','pg18-schema.sql.in','pg18-toc.json','columns.json','acl.json','contract.json'},'SCHEMA_CAPTURE_INDEX')
    for name,pointer in capture['files'].items():require({k:rows['capture/'+name][k] for k in ('size','sha256')}==pointer,'SCHEMA_CAPTURE_INDEX_JOIN')
    contract=_capture_contract(read,root/'code',plan,row['container_id'])
    require(contract['migration_fingerprint']==receipt['migration_fingerprint'],'SCHEMA_MIGRATIONS_JOIN')
    _verify_case_output(read,rows,row,plan)

def _verify_resource_originals(originals,row,plan,*,partial=False):
    h,_,d,_=_modules();objects=[]
    for out,err,process in originals:
        if err or process.get('exit_code')!=0 or process.get('reason')is not None:continue
        try:value=json.loads(out,object_pairs_hook=d.unique_pairs)
        except (ValueError,UnicodeError,RecursionError):continue
        if type(value)is list and len(value)==1 and type(value[0])is dict:objects.append(value[0])
    ident=row['identity'];case=PurePosixPath(row['evidence_directory'])
    require(case==_batch(PurePosixPath(plan['evidence_root']),plan)/plan['case_id'],'SCHEMA_RESOURCE_PATH')
    claims=('stopped','execs_absent','retained_network_empty','retained_volumes_verified')
    require(all(type(row.get(key,False))is bool for key in claims),'SCHEMA_RESOURCE_CLAIMS')
    if not partial:require(all(row.get(key)is True for key in claims),'SCHEMA_RESOURCE_COMPLETE')
    keys={'volume','source_volume','build_volume'};volumes=row.get('volumes',{})
    require(type(volumes)is dict and set(volumes)<=keys,'SCHEMA_VOLUME_SCOPE')
    if row.get('retained_volumes_verified'):require(set(volumes)==keys,'SCHEMA_RETAINED_VOLUME_COUNT')
    for key,saved in volumes.items():
        found=[o for o in objects if o.get('Name')==ident[key] and 'Mountpoint'in o]
        require(found and envelope.canonical(found[-1])==envelope.canonical(saved) and saved['Driver']=='local' and not saved.get('Options') and saved['Labels']=={'com.docker.compose.project':ident['project'],'knowweave.source-lifecycle.batch':plan['batch_id']},'SCHEMA_RETAINED_VOLUME_ORIGINAL')
    # Network readback is independent of whether PG creation was admitted.
    # False/missing flags retain unknown observations; they never become true.
    network_id=row.get('network_id')
    networks=[o for o in objects if network_id is not None and o.get('Id')==network_id and 'Containers'in o]
    if row.get('retained_network_empty'):require(networks,'SCHEMA_NETWORK_ORIGINAL')
    if network_id is not None:require(type(network_id)is str and registry.HEX64.fullmatch(network_id),'SCHEMA_NETWORK_IDENTITY')
    if networks:
        network=networks[-1]
        import ipaddress
        require(network['Name']==ident['network'] and network['Internal']is True and network['Driver']=='bridge' and network['IPAM']['Config']==[dict(Subnet=plan['subnet'],Gateway=str(ipaddress.ip_network(plan['subnet']).network_address+1))] and network['Labels']['com.docker.compose.project']==ident['project'],'SCHEMA_NETWORK_IDENTITY')
        if row.get('retained_network_empty'):require(not network['Containers'],'SCHEMA_NETWORK_EMPTY')
    container_id=row.get('container_id')
    containers=[o for o in objects if container_id is not None and o.get('Id')==container_id and 'State'in o]
    if row.get('stopped') or row.get('execs_absent'):require(containers,'SCHEMA_CONTAINER_ORIGINAL')
    if container_id is not None:require(type(container_id)is str and registry.HEX64.fullmatch(container_id),'SCHEMA_CONTAINER_IDENTITY')
    if not containers:
        require(partial,'SCHEMA_CONTAINER_ORIGINAL');return
    require(set(volumes)==keys,'SCHEMA_CONTAINER_VOLUME_SCOPE')
    images=[o for o in objects if o.get('Id')==plan['images']['postgres'] and 'RepoDigests'in o]
    require(bool(images),'SCHEMA_IMAGE_ORIGINAL');image=images[-1]
    require('postgres@'+plan['images']['postgres'] in image['RepoDigests'],'SCHEMA_IMAGE_DIGEST')
    mounts={('volume',row['volumes'][key]['Mountpoint'],target,rw) for key,target,rw in [('volume','/var/lib/postgresql',True),('source_volume',d.ROOT,True),('build_volume','/target',False)]}
    mounts|={('bind',str(case/'initdb.sh'),'/docker-entrypoint-initdb.d/10-lifecycle.sh',False),('bind',str(case/'schema'),'/var/lib/knowweave-schema',True)}
    mounts|={('bind',str(case/'secrets'/(role+'_password')),'/run/secrets/'+role+'_password',False) for role in ('postgres','admin')}
    require(len(mounts)==7,'SCHEMA_SEVEN_MOUNTS')
    stopped=containers[-1]
    pg_env=dict(POSTGRES_USER='postgres',POSTGRES_DB='postgres',POSTGRES_PASSWORD_FILE='/run/secrets/postgres_password',POSTGRES_INITDB_ARGS='--auth-host=scram-sha-256 --auth-local=trust',C4_DATABASE=ident['database'],C4_OTHER_DATABASE=ident['other_database'])
    labels=stopped['Config']['Labels']
    require(all(labels.get(k)==v for k,v in {'com.docker.compose.project':ident['project'],'com.docker.compose.service':'pg','com.docker.compose.project.working_dir':str(case),'com.docker.compose.project.config_files':str(case/'compose.json')}.items()),'SCHEMA_COMPOSE_ORIGINAL')
    expected=dict(id=row['container_id'],name=ident['pg_name'],image=image['Id'],image_ref=h.PG_IMAGE,env={**h.env_dict(image['Config']['Env']),**pg_env},labels=labels,cmd=['postgres','-c','max_prepared_transactions=16'],entrypoint=image['Config']['Entrypoint'],mounts=mounts,network=ident['network'],builder=False)
    with schema_resources._SchemaPgAdapter(h,case,volumes) as profile:
        profile.validate_container(stopped,expected)
    if row.get('stopped'):require(stopped['State']['Running']is False and type(stopped['State']['Pid'])is int and stopped['State']['Pid']==0,'SCHEMA_STOP_ORIGINAL')
    if row.get('execs_absent'):require(not stopped.get('ExecIDs'),'SCHEMA_NOEXEC_ORIGINAL')

def _verify_case_output(read,rows,row,plan):
    _,_,_,p=_modules();name=plan['case_name'];proof=json.loads(read(name+'.json'))
    require(proof['classification']==PASS,'SCHEMA_BODY_PROOF')
    if name=='full-schema-contract':
        require(set(proof)=={'classification','tables','decoded_sha256','native_cases'} and type(proof['tables'])is int and proof['tables']==64 and registry.HEX64.fullmatch(proof['decoded_sha256']),'SCHEMA_BASELINE_PROOF')
        cases=proof['native_cases'];require(len(cases)==5,'SCHEMA_NATIVE_CASES')
        for value,expected in zip(cases[:4],('receiver_cancel','deadline','output_cap','native_exit')):
            require(value['case']==expected,'SCHEMA_NATIVE_CASE')
            wait=value['wait'];require(registry.integer(wait['pid'],1,2**32-1) and wait['wait_completed']is True,'SCHEMA_NATIVE_WAIT')
            if expected in ('receiver_cancel','deadline'):require(value['held_inode_observed']is True and wait['success']is False,'SCHEMA_HELD_EXECUTION')
            if expected=='native_exit':require(type(wait['exit_code'])is int and wait['exit_code']!=0 and wait['success']is False,'SCHEMA_NATIVE_EXIT')
            for stream,source in [('stdout',value),('stderr',wait)]:
                raw=read('native-supervisor/native-'+expected+'.'+stream)
                require(len(raw)==source[stream+'_bytes'] and p.sha(raw)==source[stream+'_sha256'],'SCHEMA_NATIVE_OUTPUT')
        require(cases[4]==dict(case='held_client_hash_mismatch',child_started=False,path_substitution_claim=False),'SCHEMA_NATIVE_PRESPAWN')
    elif name=='full-schema-altered':
        require(proof==dict(classification=PASS,rejected=['bad-extra.dump','bad-function.dump','bad-acl.dump'],target_writes=0),'SCHEMA_ALTERED_PROOF')
        owned=p.normalize_restrict(read('capture/owned-schema.raw.sql'));catalog=p.canonical(json.loads(read('capture/acl.raw.json')));counts=p.canonical(json.loads(read('capture/counts.raw.json')))
        require(read('capture/acl-precondition.json')==b'f\n' and len(row['fixture_variants'])==3,'SCHEMA_ALTERED_PRECONDITION')
        for variant,_,_ in VARIANTS:
            require(p.normalize_restrict(read('capture/restored-'+variant+'.owned.sql'))==owned and p.canonical(json.loads(read('capture/restored-'+variant+'.acl.json')))==catalog and p.canonical(json.loads(read('capture/restored-'+variant+'.counts.json')))==counts,'SCHEMA_RESTORED_BASELINE')
            require(rows['capture/bad-'+variant+'.dump']['sha256']!=rows['capture/restored-'+variant+'.dump']['sha256'],'SCHEMA_ALTERED_ARCHIVE')
    else:
        raw=read('capture/edge-copy.bin')
        require(set(proof)=={'classification','source_copy_sha256','unchanged_payload_bytes','decoded_sha256'} and proof['source_copy_sha256']==p.sha(raw) and type(proof['unchanged_payload_bytes'])is int and proof['unchanged_payload_bytes']==len(raw) and len(raw)<=8192 and raw.count(b'\n')==1,'SCHEMA_EDGE_PROOF')
    if 'decoded_sha256'in proof:
        require(any(r['path'].endswith('/decoded') and r['sha256']==proof['decoded_sha256'] for r in rows.values()),'SCHEMA_DECODED_OUTPUT_JOIN')

def run(plan_path,case):
    from . import resources
    if __package__.startswith('scripts.'):
        from .. import p0c4_completion_acceptance as adapter
    else:import p0c4_completion_acceptance as adapter
    resources.actor_preflight();os.umask(0o077)
    raw=evidence.private_read(plan_path,16384);plan=validate_plan(registry.parse_record(raw),case);root=Path(plan['evidence_root'])
    require(Path(plan_path).parent==root,'SCHEMA_PLAN_DIRECTORY')
    fd=registry.open_directory(str(root))
    try:require(registry.names(fd,2)==[Path(plan_path).name],'SCHEMA_ALREADY_ATTEMPTED')
    finally:os.close(fd)
    manifest,files=adapter.accepted_source(plan);h,b,d,p=_modules()
    for module in (p,schema_resources,__import__(__name__,fromlist=[''])):
        name='scripts/'+Path(module.__file__).resolve().relative_to(Path(adapter.__file__).resolve().parent).as_posix()
        require(name in files and files[name]==Path(module.__file__).read_bytes(),'SCHEMA_INSTALLED_SOURCE')
    evidence.write_record(root/'case-plan.json',raw);b.extract_public_source(h,root/'code',files)
    result,batch=_execute_case(plan,manifest)
    evidence.write_record(root/'resources.json',envelope.canonical(result))
    h.mkdir_new(root/'raw');h.mkdir_new(root/'capture')
    raw_ref=None;capture_ref=None
    if (batch/'evidence/raw-index.json').exists():
        evidence.write_record(root/'raw/index.json',envelope.canonical(dict(format_version=1,capability='c4_schema_raw_index_v1',original_index=evidence.file_ref(batch/'evidence/raw-index.json',root,512*1024))))
        raw_ref=evidence.file_ref(root/'raw/index.json',root)
    output=batch/plan['case_id']/'schema'
    index_failed=False
    if output.exists():
        try:
            evidence.write_record(root/'capture/index.json',envelope.canonical(_output_tree(output)))
            capture_ref=evidence.file_ref(root/'capture/index.json',root)
        except (SchemaError,registry.RegistryError,OSError):index_failed=True
    row=result['cases'][0];outcome=row.get('outcome',dict(exit_code=-1,passed=0,failed=0,ignored=0))
    contract_path=output/'capture/contract.json'
    try:contract=registry.parse_record(evidence.private_read(contract_path,16384)) if contract_path.exists() else {}
    except (registry.RegistryError,OSError):contract={};index_failed=True
    receipt=dict(format_version=1,capability='c4_schema_content_receipt_v1',batch_id=plan['batch_id'],case_id=plan['case_id'],case_name=case,plan_sha256=envelope.digest(raw),source_archive_sha256=plan['source']['archive']['sha256'],source_manifest_sha256=plan['source']['manifest']['sha256'],application_build_sha256=plan['source']['application_build_sha256'],application_commit=plan['source']['application_commit'],postgres_image=plan['images']['postgres'],container_id=row.get('container_id','0'*64),client_sha256=row.get('tools_before',{}).get('pg_restore',{}).get('sha256','0'*64),migration_fingerprint=contract.get('migration_fingerprint','0'*64),binary_sha256=row.get('compiled',{}).get('lib',{}).get('sha256','0'*64),test_name=CASE_TESTS[case],test_exit=outcome['exit_code'],passed=outcome['passed'],failed=outcome['failed'],ignored=outcome['ignored'],capture_index=capture_ref,raw_index=raw_ref,resource_receipt=evidence.file_ref(root/'resources.json',root,4*1024*1024),status=result['status'],reason_code=result['reason_code'])
    if index_failed:receipt.update(status='UNCONFIRMED_UNUSABLE',reason_code='EVIDENCE_FAILED')
    try:validate_receipt(receipt,plan);_verify_contents(root,receipt,plan)
    except (SchemaError,envelope.CompletionError,registry.RegistryError,h.GateError,d.GateError,p.ContractError,KeyError,TypeError,ValueError,OSError):
        receipt.update(status='UNCONFIRMED_UNUSABLE',reason_code='EVIDENCE_FAILED')
    validate_receipt(receipt,plan)
    evidence.write_record(root/'case-receipt.json',envelope.canonical(receipt))
    return receipt

def verify(result_path):
    from . import resources
    resources.actor_preflight(False);root=Path(result_path).parent
    receipt=registry.parse_record(evidence.private_read(result_path,16384))
    plan_raw=evidence.private_read(root/'case-plan.json',16384);plan=registry.parse_record(plan_raw)
    require(root.as_posix()==plan['evidence_root'] and receipt['plan_sha256']==envelope.digest(plan_raw),'SCHEMA_PLAN_READBACK')
    validate_receipt(receipt,plan);_verify_contents(root,receipt,plan)
    return receipt

# Resource lifecycle below is adapted from Root-reviewed Ops recipe3e0b9b03.
# It reuses the existing owned resource/compiler/namespace/stop implementations.
def validate_materialization(d,receipt,artifact,intermediate):
 d.require(type(receipt) is dict and set(receipt)=={'source_path','source','destination_path','destination','bytes','sha256'} and receipt['source_path']==artifact and receipt['destination_path']==intermediate and type(receipt['bytes']) is int and 0<receipt['bytes']<=d.BUILD_DATA_LIMIT and type(receipt['sha256']) is str and d.HEX64.fullmatch(receipt['sha256']),'MATERIALIZE_RECEIPT')
 for key in ('source','destination'):
  row=receipt[key]
  d.require(type(row) is dict and set(row)=={'dev','ino','uid','mode','nlink','bytes'} and all(type(v) is int for v in row.values()) and row['dev']>=0 and row['ino']>0 and row['uid']==0 and row['bytes']==receipt['bytes'] and row['nlink']>=1 and row['mode']&0o022==0,'MATERIALIZE_RECEIPT')
 d.require(receipt['destination']['mode']==0o500 and receipt['destination']['nlink']==1 and (receipt['source']['dev'],receipt['source']['ino'])!=(receipt['destination']['dev'],receipt['destination']['ino']),'MATERIALIZE_RECEIPT')
 return receipt

def schema_cleanup_verified(result):
 return result.get('preflight_complete') is True and result.get('resource_creation_unknown') is False and result.get('registered_transports_reaped')is True and not result.get('raw_publish_error') and all(h.get('removed') is True for h in result.get('helpers',[])) and len(result.get('cases',[]))==1 and all(c.get('stopped') is True and c.get('execs_absent') is True and c.get('retained_network_empty') is True and not c.get('cleanup_errors') and c.get('retained_volumes_verified') is True for c in result['cases'])

def _execute_case(plan,manifest):
 h,b,d,producer=_modules()
 from . import resources
 validate_plan(plan,plan["case_name"])
 root=Path(plan["evidence_root"]);subnet=plan["subnet"]
 COMMIT=plan["source"]["application_commit"];ARCHIVE=plan["source"]["archive"]["sha256"]
 d.require(os.name=='posix' and os.geteuid()==0,'ROOT_LINUX_ONLY');os.umask(0o077)
 source=root/'code'
 require(manifest['base_commit']==COMMIT,'SCHEMA_SOURCE_COMMIT')
 case_id=plan['case_id'];batch_id=plan['batch_id'];ident=d.completion_case_identity(case_id)
 stage=root/'schema-run';h.mkdir_new(stage);batch=b.create_batch(h,stage,batch_id);case=batch/case_id;h.mkdir_new(case);h.mkdir_new(case/'secrets');h.mkdir_new(case/'schema')
 result=dict(status='FAILED_UNUSABLE',reason_code='RESOURCE_UNKNOWN',classification=PASS,batch_id=batch_id,case_id=case_id,base_commit=COMMIT,archive_sha256=ARCHIVE,cases=[],helpers=[],resource_creation_unknown=False,preflight_complete=False,cleanup_verified=False)
 record=dict(identity=ident,subnet=subnet,volumes={},stage='new',evidence_directory=str(case),stopped=False,execs_absent=False,retained_network_empty=False,no_source_registry_drain_complete_credit=True,test=CASE_TESTS[plan['case_name']])
 result['cases'].append(record);runner=d.owned_log_runner(h,batch,result,1);registry=runner.ownership;registry.case_id=case_id
 pg_owner=None;pg_profile=None;expected=None;container=None;deadline=time.monotonic()+7200
 def remaining(cap):
  value=min(cap,deadline-time.monotonic());d.require(value>0,'CASE_DEADLINE');return value
 def alarm(_signal,_frame):raise d.GateError('CASE_DEADLINE')
 previous_signal=signal.signal(signal.SIGALRM,alarm);signal.alarm(7200)
 def helper(command,env=None):return d.run_helper(h,b,runner,result,images[h.BUILDER],helper_mounts,command,env=env,timeout=remaining(7200))
 try:
  record['stage']='preflight'
  resources.actor_preflight();d.require(os.statvfs(batch).f_bavail*os.statvfs(batch).f_frsize>=64*1024**3,'CASE_FREE_SPACE_BUDGET')
  d.fresh_resource_preflight(h,runner,[ident],[subnet],result,route_observer=lambda:resources.route_observer(batch,result))
  result['preflight_complete']=True
  images={image:runner.inspect('image',image) for image in (h.BUILDER,h.PG_IMAGE)}
  d.require(images[h.BUILDER]['Id']==h.BUILDER and images[h.PG_IMAGE]['Id']=='sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d' and 'postgres@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d' in images[h.PG_IMAGE].get('RepoDigests',[]),'FIXED_CACHED_IMAGES')
  result['source_sha256_before']=b.source_digest(h,source,manifest)
  base_runner=runner;budget=d.CaseBudget(total_deadline=deadline);runner=d.BudgetRunner(base_runner,budget);runner.namespace=d.NamespaceAudits(runner,record)
  record['stage']='volumes'
  for key in ('volume','source_volume','build_volume'):
   labels={'com.docker.compose.project':ident['project'],'knowweave.source-lifecycle.batch':batch_id};pending=dict(name=ident[key],labels=labels)
   owner=registry.acquire('volume',ident[key],pending,None,pending);result['resource_creation_unknown']=True
   record['volumes'][key]=d.creation_call(runner,owner,lambda:b.create_volume(h,runner,ident[key],labels));registry.known(owner,ident[key]);registry.release(owner,'retained');result['resource_creation_unknown']=False
  pg_profile=schema_resources._SchemaPgAdapter(h,case,record['volumes'])
  helper_mounts=[('bind',str(source),'/reviewed',True),('volume',ident['build_volume'],'/target',False),('volume',ident['source_volume'],d.ROOT,True)]
  env=dict(CARGO_TARGET_DIR='/target/build',CARGO_BUILD_JOBS='4',CARGO_NET_OFFLINE='true',RUSTUP_AUTO_INSTALL='0',CARGO_INCREMENTAL='0',KNOWWEAVE_SOURCE_COMMIT=COMMIT,KNOWWEAVE_BUILD_ID_SHA256=ARCHIVE,**d.BUILD_PROFILE_ENV)
  record['compile_source']=dict(application_commit=COMMIT,application_build_sha256=ARCHIVE)
  record['stage']='compile';record['compiled']=_compile_artifacts(helper,d,b,case_id,env)
  binary=record['compiled']['example']['binary']
  record['stage']='fixture-config'
  passwords={role:secrets.token_hex(32) for role in ('postgres','admin')}
  for role,password in passwords.items():h.write_new(case/'secrets'/(role+'_password'),password.encode(),0o400)
  h.write_new(case/'initdb.sh',d.lifecycle_fixture_initdb(h.INITDB),0o444)
  pg_env=dict(POSTGRES_USER='postgres',POSTGRES_DB='postgres',POSTGRES_PASSWORD_FILE='/run/secrets/postgres_password',POSTGRES_INITDB_ARGS='--auth-host=scram-sha-256 --auth-local=trust',C4_DATABASE=ident['database'],C4_OTHER_DATABASE=ident['other_database'])
  binds=[dict(type='bind',source=str(case/'initdb.sh'),target='/docker-entrypoint-initdb.d/10-lifecycle.sh',read_only=True)]+[dict(type='bind',source=str(case/'secrets'/(role+'_password')),target='/run/secrets/'+role+'_password',read_only=True) for role in passwords]+[dict(type='bind',source=str(case/'schema'),target='/var/lib/knowweave-schema',read_only=False)]
  specs={'pg':(ident['volume'],'/var/lib/postgresql',False),'source':(ident['source_volume'],d.ROOT,False),'build':(ident['build_volume'],'/target',True)}
  doc=dict(name=ident['project'],services=dict(pg=dict(image=h.PG_IMAGE,pull_policy='never',container_name=ident['pg_name'],command=['postgres','-c','max_prepared_transactions=16'],cpus=2,mem_limit='4g',memswap_limit='4g',security_opt=['no-new-privileges'],environment=pg_env,volumes=[*[dict(type='volume',source=key,target=target,read_only=ro,volume=dict(nocopy=True)) for key,(_,target,ro) in specs.items()],*binds],networks=['test'])),networks=dict(test=dict(name=ident['network'],internal=True,ipam=dict(config=[dict(subnet=subnet)]))),volumes={key:dict(name=value[0],external=True) for key,value in specs.items()})
  h.write_new(case/'compose.json',d.canonical(doc));compose=[d.DOCKER,'compose','--project-name',ident['project'],'--project-directory',str(case),'-f',str(case/'compose.json')]
  expected_mounts={('bind',x['source'],x['target'],not x['read_only']) for x in binds}
  keymap={'pg':'volume','source':'source_volume','build':'build_volume'}
  expected_mounts|={('volume',record['volumes'][keymap[key]]['Mountpoint'],target,not ro) for key,(_,target,ro) in specs.items()}
  expected=dict(id=None,name=ident['pg_name'],image=images[h.PG_IMAGE]['Id'],image_ref=h.PG_IMAGE,env={**h.env_dict(images[h.PG_IMAGE]['Config']['Env']),**pg_env},labels={'com.docker.compose.project':ident['project'],'com.docker.compose.service':'pg','com.docker.compose.project.working_dir':str(case),'com.docker.compose.project.config_files':str(case/'compose.json')},cmd=doc['services']['pg']['command'],entrypoint=images[h.PG_IMAGE]['Config']['Entrypoint'],mounts=expected_mounts,network=ident['network'],builder=False,user=images[h.PG_IMAGE]['Config'].get('User',''))
  net=dict(name=ident['network'],project=ident['project'],subnet=subnet);pg_owner=registry.acquire('pg',ident['pg_name'],expected,net,record);result['resource_creation_unknown']=True
  record['stage']='pg-create';pg_profile.directory.check()
  d.creation_call(runner,pg_owner,lambda:runner.run([*compose,'create','--no-build','--pull','never']))
  record['stage']='pg-admit'
  facts=runner.inspect('container',ident['pg_name']);network=runner.inspect('network',ident['network']);expected=d.admit_created_pg(pg_profile,pg_owner,facts,network,record);registry.known(pg_owner,facts['Id']);container=facts['Id'];result['resource_creation_unknown']=False
  record['stage']='pg-pre-start';d.pg_facts(pg_profile,runner,record,expected)
  record['stage']='pg-start';runner.docker('start',container);ready=time.monotonic()+90
  record['stage']='pg-ready'
  while d.pg_exec(runner,container,['pg_isready','-h','localhost','-p','5432','-U','postgres','-d','postgres'],allowed=(0,1,2,3))[0]!=0:
   d.require(time.monotonic()<ready,'PG_READY_TIMEOUT');time.sleep(0.2)
  record['stage']='pg-post-start';d.pg_facts(pg_profile,runner,record,expected)
  budget.transition('LIVE')
  d.pg_exec(runner,container,['/bin/sh','-ec',"a=$(cat /run/secrets/admin_password); psql -X -q -v ON_ERROR_STOP=1 -U postgres -d postgres <<SQL\nALTER ROLE learning_admin PASSWORD '$a';\nSQL"])
  record['pg_binary_before']=d.audit_binaries_in_pg(runner,container,record['compiled'])
  record['stage']='migrate';script='a=$(cat /run/secrets/admin_password); export TEST_ADMIN_DATABASE_URL="postgresql://learning_admin:${a}@127.0.0.1/$1" TEST_C4_TASK3_DATABASE_NAME="$1"; exec "$2"'
  code,migrated,migrateerr=d.pg_exec(runner,container,['/bin/sh','-ec',script,'migrate',ident['database'],binary],timeout=remaining(300));d.require(code==0 and migrated==b'C4_TASK3_MIGRATED\n' and not migrateerr,'FIXED_MIGRATOR_RESULT')
  record['migrate']=dict(exit=code,stdout_sha256=d.digest(migrated),stderr_sha256=d.digest(migrateerr));record['tools_before']=d.tool_check(runner,container)
  d.require(producer.sha(Path(producer.__file__).read_bytes())==producer.sha((source/'scripts/p0c4_full_schema_contract.py').read_bytes()),'FIXED_PRODUCER_BYTES')
  capture_plan=dict(format_version=1,case_id=case_id,container_id=container,database=ident['database'],source_root=str(source),application_commit=COMMIT,application_build_sha256=ARCHIVE,output_directory=str(case/'schema/capture'))
  h.write_new(case/'capture-plan.json',d.canonical(capture_plan));record['stage']='fixed-producer-capture'
  signal.alarm(max(1,int(remaining(900))));record['capture_result']=producer.capture(capture_plan);signal.alarm(max(1,int(remaining(7200))))
  record['stage']='fixture';_fixture_variants(plan['case_name'],case_id,container,ident['database'],case/'schema/capture',record)
  record['stage']='body'
  code,body,bodyerr=d.pg_exec(runner,container,[record['compiled']['lib']['binary'],'--ignored','--exact',record['test'],'--test-threads=1'],timeout=remaining(900),allowed=tuple(range(256)))
  record['body']=dict(exit_code=code,stdout_sha256=d.digest(body),stderr_sha256=d.digest(bodyerr),stdout_bytes=len(body),stderr_bytes=len(bodyerr))
  record['outcome']=parse_body(code,body,record['test']);require(code==0 and not bodyerr,'SCHEMA_BODY_FAILED')
  record['tools_after']=d.tool_check(runner,container);d.equal_audit(record['tools_before'],record['tools_after'])
  record['pg_binary_after']=d.audit_binaries_in_pg(runner,container,record['compiled']);d.equal_audit(record['pg_binary_before'],record['pg_binary_after'])
  result['source_sha256_after']=b.source_digest(h,source,manifest);d.equal_audit(result['source_sha256_before'],result['source_sha256_after'])
  record['stage']='body-passed';result['status']=PASS;result['reason_code']='NONE'
 except BaseException as error:
  schema_resources._record_failure(result,'primary',record['stage'],error)
  result['status']='FAILED_UNUSABLE';result['reason_code']={'compile':'BUILD_FAILED','migrate':'MIGRATION_FAILED','fixed-producer-capture':'CAPTURE_FAILED','fixture':'CAPTURE_FAILED','body':'BODY_FAILED'}.get(record['stage'],'RESOURCE_UNKNOWN')
 finally:
  signal.alarm(0);signal.signal(signal.SIGALRM,previous_signal);errors=[]
  if 'budget' in locals():budget.transition('CLEANUP');registry.enter(budget)
  else:registry.enter()
  if 'source_sha256_before' in result:
   try:
    result['source_sha256_after']=b.source_digest(h,source,manifest);d.equal_audit(result['source_sha256_before'],result['source_sha256_after'])
   except BaseException as error:errors.append(schema_resources._record_failure(result,'cleanup','cleanup-source',error)['exception_class']);result['source_after_error']='SOURCE_MISMATCH'
  if isinstance(runner,d.BudgetRunner):
   try:
    if runner.namespace.pin is not None:runner.namespace.finalize()
    else:
     runner.namespace.state['ledgers']-=1;runner.namespace.ledger_reserved=False;record['namespace_ledger_status']='RUNNING_PIN_UNAVAILABLE'
   except BaseException as error:errors.append(schema_resources._record_failure(result,'cleanup','cleanup-namespace',error)['exception_class'])
  for owner in registry.owners.values():
   if owner['intent']['kind']!='volume' or owner['state'] in ('retained','absent'):continue
   try:
    if owner['state']=='planned':registry.release(owner,'absent');continue
    facts=registry.reconcile(owner,'volume')
    if facts is None:registry.release(owner,'absent')
    else:
     planned=owner['intent']['expected'];d.require(facts['Name']==planned['name'] and facts['Driver']=='local' and not facts.get('Options') and facts['Labels']==planned['labels'],'VOLUME_IDENTITY')
     owner['record']['retained_volume']=facts;registry.release(owner,'retained')
   except BaseException as error:owner['state']='unknown';errors.append(schema_resources._record_failure(result,'cleanup','cleanup-volume',error)['exception_class'])
  cleanup_stage='cleanup-pg-reconcile'
  try:
   if pg_owner is not None and pg_owner['state']=='planned':registry.release(pg_owner,'absent')
   if pg_owner is not None and pg_owner['state']=='sent':
    facts=registry.reconcile(pg_owner)
    if facts is not None:
     network=registry.reconcile(pg_owner,'network',True);d.require(network is not None,'PG_NETWORK_DISCOVERY_UNKNOWN');cleanup_stage='cleanup-pg-admit';expected=d.admit_created_pg(pg_profile,pg_owner,facts,network,record);registry.known(pg_owner,facts['Id'])
    else:registry.release(pg_owner,'absent')
   if pg_owner is not None and pg_owner['state']=='known':
    cleanup_stage='cleanup-pg-validate';clean=d.CleanupRunner(runner,pg_owner,'pg');pg_profile.validate_container(clean.inspect('container',expected['id']),expected);stop_error=None
    cleanup_stage='cleanup-pg-stop'
    try:clean.docker('stop','--timeout','10',expected['id'],timeout=15)
    except BaseException as error:
     stop_error=error;schema_resources._record_failure(result,'cleanup',cleanup_stage,error)
    cleanup_stage='cleanup-pg-observe';stopped,network=d.pg_facts(pg_profile,clean,record,expected);d.require(not stopped['State']['Running'] and stopped['State']['Pid']==0 and not stopped.get('ExecIDs') and not network.get('Containers'),'EXACT_STOP_NOEXEC_EMPTY_NET')
    record.update(stopped=True,execs_absent=True,retained_network_empty=True);registry.release(pg_owner,'stopped')
    if stop_error is not None:cleanup_stage='cleanup-pg-stop';raise stop_error
  except BaseException as error:errors.append(schema_resources._record_failure(result,'cleanup',cleanup_stage,error)['exception_class']);result['status']='UNCONFIRMED_UNUSABLE';result['reason_code']='STOP_UNCONFIRMED'
  for reason in d.cleanup_known_helpers(runner,result):errors.append(schema_resources._record_failure(result,'cleanup','cleanup-helpers',d.GateError(reason))['exception_class'])
  if pg_profile is not None:
   try:pg_profile.close()
   except BaseException as error:errors.append(schema_resources._record_failure(result,'cleanup','cleanup-directory',error)['exception_class'])
  record['cleanup_errors']=errors
  result['registered_transports_reaped']=all(c.retired or (c.finished and c.p.poll() is not None and not any(t.is_alive() for t in c.threads)) for c in registry.children)
  record['retained_volumes_verified']=all(ident[k] in registry.owners and registry.owners[ident[k]]['state']=='retained' for k in ('volume','source_volume','build_volume'))
  result['resource_creation_unknown']=any(o['state'] in ('planned','sent','unknown') for o in registry.owners.values())
  if errors or result['resource_creation_unknown']:result['status']='UNCONFIRMED_UNUSABLE';result['reason_code']='STOP_UNCONFIRMED'
  result['cleanup_verified']=schema_cleanup_verified(result)
  if not result['cleanup_verified']:result['status']='UNCONFIRMED_UNUSABLE';result['reason_code']='STOP_UNCONFIRMED'
  try:result['raw_index']=d.publish_raw_index(batch,d.runner_base(runner),result,budget=[0])
  except BaseException as error:result['raw_publish_error']=schema_resources._record_failure(result,'cleanup','cleanup-raw',error)['exception_class'];result['status']='UNCONFIRMED_UNUSABLE';result['reason_code']='EVIDENCE_FAILED';result['cleanup_verified']=False
  try:b.finalize_result(h,batch,result)
  except BaseException as error:result['final_publish_error']=schema_resources._record_failure(result,'cleanup','cleanup-publish',error)['exception_class'];result['status']='UNCONFIRMED_UNUSABLE';result['reason_code']='EVIDENCE_FAILED';result['cleanup_verified']=False
 return result,batch
