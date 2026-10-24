"""Fixed test-only Attention bytes and Root observations; no effect authority."""
import base64
from collections import Counter
import hashlib,json,re

POSITIVE='FULL_RESTORE_REHEARSAL_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE'
NEGATIVE='EXPECTED_REJECTION_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE'
EXPECTED_REJECTIONS={
 'attention-missing-asset':'MISSING_ASSET',
 'attention-corrupt-dump':'CORRUPT_DUMP',
 'attention-interrupted-transfer':'INTERRUPTED_TRANSFER',
 'attention-unknown-effect':'UNKNOWN_EFFECT',
 'attention-root-replacement':'ROOT_REPLACEMENT',
}
# Frozen native producer PROOF-SCHEMA.json; not an execution/acceptance claim.
WIRE_CONTRACT='attention_native_full_c_c0_d1_proof_v1'
PRODUCER_SCHEMA_SHA256='2ff4bc788785b1809970d46e67657046e6a1fa11d6aa946954ee3cfe1709bf2c'
ORIGINALS=({'base64': 'JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCAyMDAgMjAwXSAvQ29udGVudHMgNCAwIFIgPj4KZW5kb2JqCjQgMCBvYmoKPDwgL0xlbmd0aCAwID4+CnN0cmVhbQoKZW5kc3RyZWFtCmVuZG9iagp4cmVmCjAgNQowMDAwMDAwMDAwIDY1NTM1IGYgCjAwMDAwMDAwMDkgMDAwMDAgbiAKMDAwMDAwMDA1OCAwMDAwMCBuIAowMDAwMDAwMTE1IDAwMDAwIG4gCjAwMDAwMDAyMDIgMDAwMDAgbiAKdHJhaWxlcgo8PCAvU2l6ZSA1IC9Sb290IDEgMCBSID4+CnN0YXJ0eHJlZgoyNTEKJSVFT0YK', 'bytes': 414, 'logical_registrations': 2, 'media_type': 'application/pdf', 'name': 'attention.pdf', 'sha256': 'a2b4945b6565b105cbf8ab4dc1d86bfc925d1e3eaa91cfb6ea770d292c333d28', 'storage_key': 'sha256/a2/a2b4945b6565b105cbf8ab4dc1d86bfc925d1e3eaa91cfb6ea770d292c333d28'}, {'base64': 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQjt78HwAEBwIxvmzkhQAAAABJRU5ErkJggg==', 'bytes': 70, 'logical_registrations': 1, 'media_type': 'image/png', 'name': 'figure.png', 'sha256': '952468c8647eb44c6285be3698c2ce9335240313f36250a24abc1dc59edc78a4', 'storage_key': 'sha256/95/952468c8647eb44c6285be3698c2ce9335240313f36250a24abc1dc59edc78a4'}, {'base64': 'eyJjZWxscyI6W3siY2VsbF90eXBlIjoiY29kZSIsInNvdXJjZSI6WyJwcmludCg0MikiXX1dLCJuYmZvcm1hdCI6NH0=', 'bytes': 68, 'logical_registrations': 1, 'media_type': 'application/x-ipynb+json', 'name': 'analysis.ipynb', 'sha256': 'e60b1dd5444bb8b1e8adb9f808058b2b953b57c3a359fae605bae25721687e44', 'storage_key': 'sha256/e6/e60b1dd5444bb8b1e8adb9f808058b2b953b57c3a359fae605bae25721687e44'}, {'base64': 'iVBORw0KGgoAAAAASUVORA==', 'bytes': 16, 'logical_registrations': 1, 'media_type': 'image/png', 'name': 'personal.png', 'sha256': 'df79318095b5d776a44103b1a7b1c419c09e981bc75bd6ff8d2d39d411482bd1', 'storage_key': 'sha256/df/df79318095b5d776a44103b1a7b1c419c09e981bc75bd6ff8d2d39d411482bd1'}, {'base64': 'QzQgVGFzazcgdW5saW5rZWQgcmVhZHkgb3JpZ2luYWwK', 'bytes': 33, 'logical_registrations': 1, 'media_type': 'application/octet-stream', 'name': 'unlinked.bin', 'sha256': '11328a8416de987871fba959e6fd700e48f67f9d658a2e5f15349ae41792cf86', 'storage_key': 'sha256/11/11328a8416de987871fba959e6fd700e48f67f9d658a2e5f15349ae41792cf86'})

class AttentionError(RuntimeError):pass
def need(ok,code):
 if not ok:raise AttentionError(code)
def sha(raw):return hashlib.sha256(raw).hexdigest()
def canonical(value):return json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=False,allow_nan=False).encode()
def fixed_originals():
 rows=[]
 for value in ORIGINALS:
  raw=base64.b64decode(value['base64'],validate=True);digest=sha(raw)
  need(type(value['bytes'])is int and len(raw)==value['bytes']and digest==value['sha256']and value['storage_key']=='sha256/'+digest[:2]+'/'+digest,'ATTENTION_FIXED_ORIGINAL_BYTES')
  need(type(value['logical_registrations'])is int and value['logical_registrations']in (1,2),'ATTENTION_FIXED_REGISTRATION_COUNT');rows.append(dict(value))
 need(len(rows)==5 and sum(x['bytes']for x in rows)==601 and sum(x['logical_registrations']for x in rows)==6 and len({x['sha256']for x in rows})==5,'ATTENTION_FIXED_ORIGINAL_ROSTER')
 return rows
def classification(case):
 need(case=='attention-full-chain'or case in EXPECTED_REJECTIONS,'ATTENTION_FIXED_CASE')
 return POSITIVE if case=='attention-full-chain'else NEGATIVE

def validate_ready_rows(rows):
 fixed=fixed_originals();by_sha={x['sha256']:x for x in fixed};expected=Counter({x['sha256']:x['logical_registrations']for x in fixed});actual=Counter()
 need(type(rows)is list and len(rows)==6,'ATTENTION_READY_ASSET_COUNT')
 for row in rows:
  need(type(row)is dict and set(row)=={'space_id','id','sha256','byte_size','storage_key'},'ATTENTION_READY_ASSET_FIELDS')
  for key in ('space_id','id'):need(type(row[key])is str and re.fullmatch('[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}',row[key])is not None,'ATTENTION_READY_ASSET_UUID')
  item=by_sha.get(row['sha256']);need(item is not None and type(row['byte_size'])is int and row['byte_size']==item['bytes']and row['storage_key']==item['storage_key'],'ATTENTION_READY_ASSET_ORIGINAL_LINK');actual[row['sha256']]+=1
 need(actual==expected and len({(r['space_id'],r['id'])for r in rows})==6 and len({r['space_id']for r in rows})==2,'ATTENTION_READY_ASSET_EXACT_MULTIPLICITY')
 shared=[r for r in rows if r['sha256']==fixed[0]['sha256']];need(len(shared)==2 and shared[0]['space_id']!=shared[1]['space_id'],'ATTENTION_SHARED_PDF_TWO_SPACES')
 return dict(logical_assets=6,physical_files=5,unique_asset_bytes=601)

# This separate root helper runs only after the unchanged issuer has admitted
# empty roots. It mounts only the registered assets directory; no credentials,
# SQL, source root, Docker socket, or plan-provided pathname/content is accepted.
PRESEED_PROGRAM=r"""
import base64,hashlib,json,os,stat,sys
from pathlib import Path
def require(v):
 if not v:raise RuntimeError('attention fixed preseed refused')
def pairs(rows):
 value={}
 for k,v in rows:require(k not in value);value[k]=v
 return value
require(sys.platform=='linux' and os.geteuid()==os.getegid()==0)
request_raw=sys.stdin.buffer.read(4097);require(0<len(request_raw)<=4096)
request=json.loads(request_raw,object_pairs_hook=pairs)
require(type(request)is dict and set(request)=={'format_version','capability','source_binding_sha256','assets'})
require(type(request['format_version'])is int and request['format_version']==1 and request['capability']=='attention_fixed_originals_preseed_v1')
pin=request['source_binding_sha256'];require(type(pin)is str and len(pin)==64 and all(c in '0123456789abcdef'for c in pin))
root=request['assets'];require(type(root)is dict and set(root)=={'dev','ino'} and all(type(root[k])is int and 0<root[k]<2**64 for k in root))
out={}
objects=__FIXED_ORIGINALS__
assets=Path('/var/lib/knowweave-source/assets')
rootfd=os.open(assets,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
fds=[rootfd]
try:
 before=os.fstat(rootfd);require((before.st_dev,before.st_ino)==(root['dev'],root['ino']) and stat.S_ISDIR(before.st_mode) and before.st_uid==before.st_gid==0 and stat.S_IMODE(before.st_mode)==0o700 and before.st_nlink==2 and not os.listdir(rootfd))
 os.mkdir('sha256',0o700,dir_fd=rootfd);digestfd=os.open('sha256',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=rootfd);fds.append(digestfd);prefixes={};files=[]
 for value in objects:
  raw=base64.b64decode(value['base64'],validate=True);digest=hashlib.sha256(raw).hexdigest();require(type(value['bytes'])is int and len(raw)==value['bytes']and digest==value['sha256']and value['storage_key']=='sha256/'+digest[:2]+'/'+digest)
  prefix=digest[:2]
  if prefix not in prefixes:
   os.mkdir(prefix,0o700,dir_fd=digestfd);prefixes[prefix]=os.open(prefix,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=digestfd);fds.append(prefixes[prefix])
  parent=prefixes[prefix];fd=os.open(digest,os.O_RDWR|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,0o600,dir_fd=parent)
  try:
   m=os.fstat(fd);require(stat.S_ISREG(m.st_mode)and m.st_uid==m.st_gid==0 and m.st_nlink==1 and m.st_size==0 and stat.S_IMODE(m.st_mode)==0o600)
   with os.fdopen(os.dup(fd),'wb')as writer:require(writer.write(raw)==len(raw));writer.flush();os.fsync(writer.fileno())
   os.lseek(fd,0,os.SEEK_SET);observed=os.read(fd,len(raw)+1);end=os.fstat(fd);named=os.stat(digest,dir_fd=parent,follow_symlinks=False)
   require(observed==raw and end.st_size==len(raw)and end.st_nlink==named.st_nlink==1 and (end.st_dev,end.st_ino)==(m.st_dev,m.st_ino)==(named.st_dev,named.st_ino))
   files.append(dict(storage_key=value['storage_key'],bytes=len(raw),sha256=digest,dev=end.st_dev,ino=end.st_ino,uid=end.st_uid,gid=end.st_gid,mode=stat.S_IMODE(end.st_mode),nlink=end.st_nlink))
  finally:os.close(fd)
  os.fsync(parent)
 require(len(files)==5 and sum(x['bytes']for x in files)==601 and sorted(os.listdir(rootfd))==['sha256']and sorted(os.listdir(digestfd))==sorted(prefixes))
 for prefix,fd in prefixes.items():require(sorted(os.listdir(fd))==sorted(x['sha256']for x in files if x['sha256'].startswith(prefix)))
 os.fsync(digestfd);os.fsync(rootfd);named=assets.lstat();require((named.st_dev,named.st_ino)==(before.st_dev,before.st_ino)and named.st_uid==named.st_gid==0 and stat.S_IMODE(named.st_mode)==0o700)
 out['attention_originals']=dict(scope='FIXED_FRESH_ATTENTION_BYTES_PRESEEDED_NOT_ASSET_REGISTRATION',physical_files=5,logical_ready_assets=6,unique_original_bytes=601,files=files)
 out.update(request_sha256=hashlib.sha256(request_raw).hexdigest(),assets=dict(dev=before.st_dev,ino=before.st_ino))
finally:
 for fd in reversed(fds):os.close(fd)
print(json.dumps(out,sort_keys=True,separators=(',',':')))
"""

def preseed_program():
 program=PRESEED_PROGRAM.replace('__FIXED_ORIGINALS__',repr(fixed_originals()));need('__FIXED_ORIGINALS__'not in program,'ATTENTION_FIXED_PRESEED_ONLY')
 need(len(program.encode())<=65536,'ATTENTION_PRESEED_PROGRAM_CAP');compile(program,'fixed-attention-preseed','exec');return program

def preseed_after_issuer(context,record):
 from . import full_import,snapshot_worker
 d=snapshot_worker._driver()
 need(record is context._record and context._case_name in full_import.ATTENTION_CASE_TESTS and record.get('test')==context._selected_test() and record.get('stage')=='independent-issuer' and record.get('full_profile_stage')=='VERIFIED' and context._phase=='PROFILE_MEASURE' and 'attention_originals_preseed'not in record,'ATTENTION_PRESEED_FIXED_PHASE_ONCE')
 issuer=d.validate_issuer(record.get('issuer'),record.get('sql_identity'));d.compile_binding(record);receipt=issuer['registry_provisioning']
 expected=dict(batch_id=context._plan['batch_id'],case_id=context._plan['case_id'],source_package_sha256=record['archive_sha256'],application_commit=context._plan['source']['application_commit'],application_build_sha256=context._plan['source']['application_build_sha256'])
 need(record['identity']==context._ident and all(receipt[k]==v for k,v in expected.items()),'ATTENTION_PRESEED_ISSUER_CASE')
 for name,value in [('issuer.json',issuer),('registry-provisioning.json',receipt)]:
  raw,_=full_import._measured_regular(context._case/name,16384);need(raw==d.canonical(value),'ATTENTION_PRESEED_ISSUER_DURABLE_READBACK')
 asset=next(row for row in receipt['roots']if row['kind']=='source_assets')
 need(asset['path']==snapshot_worker.ROOT+'/assets' and asset['dev']==record['snapshot_root_prepare']['source']['dev'] and context._snapshot_source_mountpoint is not None,'ATTENTION_PRESEED_REGISTERED_ASSETS')
 root={key:asset[key]for key in ('dev','ino')};request=dict(format_version=1,capability='attention_fixed_originals_preseed_v1',source_binding_sha256=issuer['binding_sha256'],assets=root);raw=d.canonical(request)
 mounts=[('bind',context._snapshot_source_mountpoint+'/assets',snapshot_worker.ROOT+'/assets',False)]
 # Construction registers ownership and validates the fresh created container
 # ID before run sends the fixed request; run verifies natural exit and closes
 # that exact helper through the existing reserved cleanup path and case budget.
 helper=d.HelperContainer(context._h,context._binding,context._runner,context._result,context._images[context._h.BUILDER],mounts,['python3','-I','-u','-c',preseed_program()],interactive=True)
 code,out,err=helper.run(stdin=raw,timeout=30)
 need(code==0 and type(code)is int and err==b'' and 0<len(out)<=16384,'ATTENTION_PRESEED_NATURAL_EXIT')
 observed=json.loads(out,object_pairs_hook=d.unique_pairs)
 need(type(observed)is dict and set(observed)=={'attention_originals','request_sha256','assets'} and observed['request_sha256']==sha(raw) and type(observed['assets'])is dict and observed['assets']==root and all(type(observed['assets'][k])is int for k in root) and out==d.canonical(observed)+b'\n' and helper.record['removed']is True,'ATTENTION_PRESEED_REGISTERED_READBACK')
 record['attention_originals_preseed']=validate_preseed_observation(observed['attention_originals'])
 record['attention_preseed_helper']=dict(container_id=helper.record['id'],request_sha256=sha(raw),source_binding_sha256=issuer['binding_sha256'],assets=root,exit_code=code,removed=helper.record['removed'])

def validate_preseed_observation(value):
 fixed={x['storage_key']:x for x in fixed_originals()}
 need(type(value)is dict and set(value)=={'scope','physical_files','logical_ready_assets','unique_original_bytes','files'}and value['scope']=='FIXED_FRESH_ATTENTION_BYTES_PRESEEDED_NOT_ASSET_REGISTRATION','ATTENTION_PRESEED_FIELDS')
 need(all(type(value[k])is int for k in ('physical_files','logical_ready_assets','unique_original_bytes'))and(value['physical_files'],value['logical_ready_assets'],value['unique_original_bytes'])==(5,6,601),'ATTENTION_PRESEED_COUNTS')
 need(type(value['files'])is list and len(value['files'])==5 and {x['storage_key']for x in value['files']}==set(fixed),'ATTENTION_PRESEED_EXACT_ROSTER')
 ids=set()
 for row in value['files']:
  need(set(row)=={'storage_key','bytes','sha256','dev','ino','uid','gid','mode','nlink'}and all(type(row[k])is int for k in ('bytes','dev','ino','uid','gid','mode','nlink')),'ATTENTION_PRESEED_TYPED_META')
  original=fixed[row['storage_key']];identity=(row['dev'],row['ino']);need(row['bytes']==original['bytes']and row['sha256']==original['sha256']and row['dev']>0 and row['ino']>0 and identity not in ids and row['uid']==row['gid']==0 and row['mode']==0o600 and row['nlink']==1,'ATTENTION_PRESEED_ACTUAL_BYTES_AND_IDENTITY');ids.add(identity)
 return value

def fixed_proof_files(case):
 classification(case)
 rows=[('full-attention-result.json',16*1024**2,'full_attention_proof'),('full-attention-inventory.json',16*1024**2,'full_attention_inventory')]
 if case!='attention-root-replacement':rows.append(('full-import-result.json',16384,'full_import_proof'))
 if case not in ('attention-corrupt-dump','attention-root-replacement'):rows.append(('full-attention-normalization.json',16*1024**2,'full_attention_normalization'))
 if case in ('attention-missing-asset','attention-interrupted-transfer','attention-root-replacement'):
  rows.extend([('full-attention-negative-before.json',16*1024**2,'full_attention_negative_before'),('full-attention-negative-after.json',16*1024**2,'full_attention_negative_after')])
 if case=='attention-corrupt-dump':rows.append(('full-negative-inventory.json',16*1024**2,'full_attention_original_negative'))
 return tuple(rows)

def read_fixed_bundle(driver,runner,container,case):
 result={}
 for name,cap,key in fixed_proof_files(case):
  code,raw,err=driver.pg_exec(runner,container,['cat','/var/lib/knowweave-source/proofs/'+name]);need(type(code)is int and code==0 and err==b''and type(raw)is bytes and 0<len(raw)<=cap,'ATTENTION_BOUNDED_FIXED_PROOF_READ');result[name]=raw
 return result

def validate_bundle(bundle,plan,target,independent_capture):
 need(WIRE_CONTRACT is not None,'ATTENTION_PROOF_CONTRACT_NOT_FROZEN')
 try:return _validate_bundle(bundle,plan,target,independent_capture)
 except (KeyError,ValueError,TypeError,IndexError,UnicodeError,OverflowError)as error:raise AttentionError('ATTENTION_MALFORMED_FIXED_PROOF')from error

def _pairs(rows):
 value={}
 for key,item in rows:
  need(key not in value,'ATTENTION_DUPLICATE_JSON_KEY');value[key]=item
 return value

def _json(raw):
 need(type(raw)is bytes and 0<len(raw)<=16*1024**2,'ATTENTION_PROOF_BYTES_CAP')
 value=json.loads(raw,object_pairs_hook=_pairs,parse_constant=lambda _:need(False,'ATTENTION_NONFINITE_JSON'))
 need(type(value)is dict and canonical(value)==raw,'ATTENTION_ORIGINAL_CANONICAL_JSON');return value

def _keys(value,keys,code):need(type(value)is dict and set(value)==set(keys.split()),code)
def _int(value,want=None):return type(value)is int and (value>=0 if want is None else value==want)
def _hex(value):return type(value)is str and re.fullmatch('[0-9a-f]{64}',value)is not None
def _uuid(value):return type(value)is str and re.fullmatch('[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}',value)is not None and value!='00000000-0000-0000-0000-000000000000'
def _same(left,right):return canonical(left)==canonical(right)

def validate_worker_results(record):
 from . import snapshot_worker as worker
 rows=record.get('snapshot_worker_results');binary=record.get('snapshot_worker_binary',{})
 need(type(rows)is list and len(rows)==3 and _int(binary.get('exit_code'),0)and _hex(binary.get('sha256')),'ATTENTION_THREE_REAL_LEAF_SIGNALS')
 need([row['disposition']for row in rows]==['COMPLETED','REVOKED_BEFORE_WRITE','REVOKED_BEFORE_WRITE']and len({row['request_id']for row in rows})==3 and len({row['client_container_id']for row in rows})==3,'ATTENTION_FIXED_SIGNAL_ORDER_AND_DISTINCT_CLIENTS')
 for row in rows:
  need(type(row)is dict and set(row)==worker.RESULT_KEYS and row['op']=='snapshot-worker-result'and _uuid(row['request_id'])and _hex(row['client_container_id'])and row['worker_binary_sha256']==binary['sha256']and _int(row['client_exit'],0)and row['client_stdout_sha256']==sha(worker.scheduling_output(row['disposition']))and row['client_stderr_sha256']==sha(b''),'ATTENTION_NATURAL_TYPED_WORKER_RECEIPT')
 return dict(worker_signals=3,completed=1,revoked_before_write=2)

# The already accepted full-C content roster, including empty migrations.
TABLES=('_sqlx_migrations','app_user','asset','asset_integrity_result','block','block_asset_use','block_revision','composition','composition_occurrence','composition_receipt','composition_revision','epistemic_review','epistemic_review_receipt','epistemic_stream','job','job_outbox','lineage_input','lineage_operation','lineage_output','lineage_receipt','migration_receipt','mutation_receipt','outbox_event','overlay','overlay_group','overlay_group_identity','overlay_placement','overlay_placement_identity','overlay_revision','placement_manual_decision','placement_migration','placement_migration_decision','placement_migration_group','placement_migration_mapping','reading_epistemic_selection','reading_receipt','reading_receipt_block','reading_relation_selection','reading_view','reading_view_revision','reference_dependency','reference_object','relation','relation_receipt','relation_review','relation_review_head','relation_review_receipt','relation_revision','release','release_manifest_composition','release_manifest_object','release_reading','release_receipt','release_root','request_key','resource','resource_version','snapshot_export_request','snapshot_export_result','snapshot_import_batch','source_segment','space','space_grant','upload_receipt')
REFERENCE_TABLES=('asset_integrity_result','job','job_outbox','request_key','snapshot_export_request','snapshot_export_result')

def _tables(rows,names):
 need(type(rows)is list and [r['table']for r in rows]==list(names),'ATTENTION_EXACT_TABLE_ROSTER')
 for row in rows:
  _keys(row,'table rows raw_bytes digest','ATTENTION_TABLE_FIELDS')
  need(_int(row['rows'])and _int(row['raw_bytes'])and type(row['digest'])is list and len(row['digest'])==32 and all(type(x)is int and 0<=x<=255 for x in row['digest']),'ATTENTION_TYPED_ROW_OBSERVATION')

def _reading(value):
 _keys(value,'format_version capability execution_context sql_runtime_privileges view_revisions current_revisions historical_revisions owner_present owner_without_grant_denied nonexistent_actor_denied existing_granted_nonowner_denied views_without_existing_granted_nonowner modes projection_bytes projection_sha256 business_writes preview_generator search_generator','ATTENTION_READING_FIELDS')
 need(_int(value['format_version'],1)and value['capability']=='recovery_application_read_projections_v1'and value['execution_context']=='EXACT_HELD_MANAGEMENT_TRANSACTION_NOT_ORDINARY_RUNTIME_CONNECTION'and value['sql_runtime_privileges']=='SEPARATE_ROLE_CATALOG_AND_CONNECT_GATE','ATTENTION_REAL_READING_CONTEXT')
 counts=('view_revisions','current_revisions','historical_revisions','owner_present','owner_without_grant_denied','nonexistent_actor_denied','existing_granted_nonowner_denied','views_without_existing_granted_nonowner','projection_bytes','business_writes')
 need(all(_int(value[k])for k in counts)and value['current_revisions']>0 and value['historical_revisions']>0 and value['view_revisions']==value['current_revisions']+value['historical_revisions']and value['owner_present']>0 and value['nonexistent_actor_denied']>0 and value['existing_granted_nonowner_denied']>0 and value['business_writes']==0 and value['projection_bytes']>0 and _hex(value['projection_sha256'])and value['modes']==['original','personal','fused']and value['preview_generator']==value['search_generator']=='ABSENT_NOT_REBUILT','ATTENTION_READING_HISTORY_DENIAL_AND_MODES')

def _inventory(value,backup):
 _keys(value,'format_version capability backup_id source_view all_table_count rows catalog target_expected_catalog function_definitions reading typed_roster catalog_recovery_delta complete_backup usable','ATTENTION_SOURCE_INVENTORY_FIELDS')
 need(_int(value['format_version'],1)and value['capability']=='attention_authoritative_inventory_v1'and value['backup_id']==backup and value['source_view']=='CLOSED_DRAINED_SAME_ADMITTED_CONNECTION_REPEATABLE_READ_READ_ONLY_BEFORE_DUMP'and _int(value['all_table_count'],64)and value['complete_backup']is False and value['usable']is False,'ATTENTION_CLOSED_SOURCE_INVENTORY')
 rows=value['rows'];_keys(rows,'format_version tables excluded_columns observation','ATTENTION_SOURCE_ROWS_FIELDS');need(_int(rows['format_version'],1)and rows['excluded_columns']==[]and rows['observation']=='CLOSED_SOURCE_SAME_ADMITTED_CONNECTION_ALL_64_PK_ORDERED_ROWS','ATTENTION_SOURCE_ROWS_CONTEXT');_tables(rows['tables'],TABLES)
 expected=json.loads(canonical(value['target_expected_catalog']));need(_same(expected['database'],dict(owner='learning_admin',acl='{learning_admin=CTc/learning_admin}')),'ATTENTION_TARGET_DATABASE_ADMIN_ONLY');expected['database']=dict(owner='learning_admin',acl='{learning_admin=CTc/learning_admin,learning_runtime=T/learning_admin}');roles=expected['roles'];need(type(roles)is list and sum(r['name']=='learning_snapshot_claim_broker'for r in roles)==1,'ATTENTION_CATALOG_BROKER_ROLE')
 for role in roles:
  if role['name']=='learning_snapshot_claim_broker':need(role['login']is False,'ATTENTION_TARGET_BROKER_NOLOGIN');role['login']=True
 need(_same(expected,value['catalog'])and _same(value['catalog_recovery_delta'],[dict(role='learning_snapshot_claim_broker',field='login',source=True,target=False),dict(object='database',field='acl',source='{learning_admin=CTc/learning_admin,learning_runtime=T/learning_admin}',target='{learning_admin=CTc/learning_admin}')]),'ATTENTION_CATALOG_ONLY_TWO_EXACT_RECOVERY_DELTAS')
 functions=value['function_definitions'];need(type(functions)is list and len(functions)==48 and len({(f['name'],f['args'])for f in functions})==48,'ATTENTION_ALL_FUNCTION_DEFINITIONS')
 for row in functions:_keys(row,'name args definition','ATTENTION_FUNCTION_FIELDS');need(all(type(row[k])is str for k in row)and row['definition'],'ATTENTION_FUNCTION_DEFINITION_BYTES')
 _reading(value['reading']);return value['typed_roster']

def _typed(roster,ready):
 keys='actor peer space source_space hypothesis_historical hypothesis_current historical_reading current_reading reuse_reading compositions original_figure personal_figure attachment source_run evidence relations epistemic_review conclusion release_id lineage_operation lineage_outputs migration_proposal unplaced resources segments assets unlinked_ready shared_digest import_request import_manifest_sha256 job_edge_reading expected_h1 expected_h2 fixed_drafts dependency_revocation reachability functions_and_acl'
 _keys(roster,keys,'ATTENTION_TYPED_ROSTER_FIELDS')
 need(all(_uuid(roster[k])for k in ('actor','peer','space','source_space','release_id','lineage_operation','migration_proposal','import_request'))and roster['actor']!=roster['peer']and roster['space']!=roster['source_space'],'ATTENTION_TYPED_PERMISSION_IDENTITIES')
 for field,fields in (('hypothesis_historical','block_id revision_id'),('hypothesis_current','block_id revision_id'),('historical_reading','view_id revision_id'),('current_reading','view_id revision_id'),('reuse_reading','view_id revision_id'),('job_edge_reading','view_id revision_id')):
  _keys(roster[field],fields,'ATTENTION_TYPED_REFERENCE_FIELDS');need(all(_uuid(v)for v in roster[field].values()),'ATTENTION_TYPED_REFERENCE_UUID')
 h1,h2=roster['hypothesis_historical'],roster['hypothesis_current'];old,current,reuse=roster['historical_reading'],roster['current_reading'],roster['reuse_reading']
 need(h1['block_id']==h2['block_id']and h1['revision_id']!=h2['revision_id']and old['view_id']==current['view_id']and old['revision_id']!=current['revision_id']and old['view_id']!=reuse['view_id']and roster['expected_h1']=='H@1: toy attention score improves'and roster['expected_h2']=='H@2: narrower claim','ATTENTION_TYPED_HISTORICAL_CURRENT_IDENTITIES')
 assets=roster['assets'];need(type(assets)is list and len(assets)==6,'ATTENTION_TYPED_SIX_ASSETS')
 for row in assets:_keys(row,'space_id asset_id','ATTENTION_TYPED_ASSET_FIELDS');need(_uuid(row['space_id'])and _uuid(row['asset_id']),'ATTENTION_TYPED_ASSET_UUID')
 need({(r['space_id'],r['asset_id'])for r in assets}=={(r['space_id'],r['id'])for r in ready}and roster['shared_digest']==fixed_originals()[0]['sha256']and _hex(roster['import_manifest_sha256'])and roster['unlinked_ready']in assets and type(roster['unplaced'])is list and len(roster['unplaced'])==3 and type(roster['compositions'])is list and len(roster['compositions'])==2 and type(roster['evidence'])is list and len(roster['evidence'])==3 and roster['reachability']=='EVERY_ID_FIXED_TYPED_API_PROBED_OR_INTENTIONALLY_DENIED'and roster['functions_and_acl']=='SEPARATE_CLOSED_SOURCE_INVENTORY','ATTENTION_TYPED_REACHABILITY_ORIGINAL_ROSTER')
 drafts=roster['fixed_drafts'];need(type(drafts)is list and len(drafts)==18,'ATTENTION_ALL_FIXED_TYPED_DRAFTS');indexed={}
 for pair in drafts:
  need(type(pair)is list and len(pair)==2,'ATTENTION_FIXED_DRAFT_PAIR');reference,draft=pair;_keys(reference,'block_id revision_id','ATTENTION_FIXED_DRAFT_REFERENCE');key=(reference['block_id'],reference['revision_id']);need(all(_uuid(v)for v in key)and key not in indexed and type(draft)is dict and bool(draft),'ATTENTION_FIXED_DRAFT_DISTINCT_CONTENT');indexed[key]=draft
 for reference,label in ((h1,roster['expected_h1']),(h2,roster['expected_h2'])):
  draft=indexed.get((reference['block_id'],reference['revision_id']));need(type(draft)is dict and draft.get('body',{}).get('payload',{}).get('text')==label,'ATTENTION_HISTORICAL_CURRENT_DRAFT_PAYLOAD')
 revoked=roster['dependency_revocation'];_keys(revoked,'content_denied review_incomplete visible_relation_assertions release_denied hidden_wire_checked restored_exact_reading_and_release','ATTENTION_DEPENDENCY_PERMISSION_FIELDS');need(_int(revoked['content_denied'],3)and _int(revoked['review_incomplete'],1)and _int(revoked['visible_relation_assertions'],1)and all(revoked[k]is True for k in ('release_denied','hidden_wire_checked','restored_exact_reading_and_release')),'ATTENTION_DEPENDENCY_PERMISSION_REAL_DENIALS')

def _normalization(value,proof,raw):
 _keys(value,'format_version capability status identity before after recovery_epoch_micros planned_delta applied_delta holds held_jobs has_holds external_snapshot_bytes_verified positive_snapshot_findings normalization_commit_known native_old_token_rejection_verified usable','ATTENTION_NORMALIZATION_FIELDS')
 unknown=proof['case']=='attention-unknown-effect'
 need(_int(value['format_version'],1)and value['capability']=='recovery_job_normalization_v1'and value['status']=='COMMITTED_NORMALIZATION_NOT_RECOVERY_ELIGIBILITY'and value['normalization_commit_known']is True and value['native_old_token_rejection_verified']is False and value['usable']is False and type(value['has_holds'])is bool and value['has_holds']is unknown and _int(value['held_jobs'],3 if unknown else 0)and _int(value['positive_snapshot_findings'],0 if unknown else 3)and value['external_snapshot_bytes_verified']is (not unknown)and _int(value['recovery_epoch_micros']),'ATTENTION_COMMITTED_JOB_REPORT_NOT_USABLE')
 identity=value['identity'];need(identity['backup_id']==proof['backup_id']and identity['manifest_sha256']==proof['manifest_sha256']and identity['database']==proof['target_database']and identity['container_id']==proof['target_container_id']and identity['complete_backed']is False,'ATTENTION_NORMALIZATION_SAME_PRIVATE_TARGET')
 before,after=value['before'],value['after']
 for data in (before,after):
  _keys(data,'tables','ATTENTION_SIX_TABLE_INVENTORY');need(set(data['tables'])==set(REFERENCE_TABLES)and all(type(x)is list for x in data['tables'].values()),'ATTENTION_ALL_SIX_REFERENCE_TABLES')
 for name in REFERENCE_TABLES:
  if name!='job':need(_same(before['tables'][name],after['tables'][name]),'ATTENTION_REFERENCE_HISTORY_UNCHANGED')
 old={r['id']:r for r in before['tables']['job']};new={r['id']:r for r in after['tables']['job']};need(len(old)==len(before['tables']['job'])==len(new)==len(after['tables']['job'])==10 and set(old)==set(new),'ATTENTION_ALL_TEN_JOB_IDENTITIES')
 changes={r['job_id']:r for r in value['applied_delta']};plans={r['job_id']:r for r in value['planned_delta']};need(len(changes)==len(value['applied_delta'])==len(plans)==len(value['planned_delta'])and set(changes)==set(plans)<=set(old),'ATTENTION_EXACT_PLANNED_JOB_DELTAS')
 allowed={'status','lease_token','lease_expires_at','next_attempt_at','last_error_class','updated_at'}
 for job,row in old.items():
  actual=new[job]
  if job not in changes:need(_same(row,actual),'ATTENTION_UNPLANNED_JOB_UNCHANGED');continue
  change=changes[job];_keys(change,'job_id before after changed_fields','ATTENTION_JOB_DELTA_FIELDS');need(_same(change['before'],row)and _same(change['after'],actual)and set(row)==set(actual),'ATTENTION_JOB_DELTA_FULL_ROWS')
  changed=sorted(k for k in row if not _same(row[k],actual[k]));need(change['changed_fields']==changed and set(changed)<=allowed and _same(row['attempt_count'],actual['attempt_count'])and actual['lease_token']is None and actual['lease_expires_at']is None,'ATTENTION_JOB_DELTA_ALLOWED_COLUMNS_NO_ATTEMPT_RESET')
 need(sha(raw)==proof['normalization_report_sha256'],'ATTENTION_NORMALIZATION_ORIGINAL_PIN');return after

def _post(value,inventory,normalization_sha,after):
 _keys(value,'format_version capability unchanged normalized covered_tables','ATTENTION_FINAL_ROW_BARRIER_FIELDS');need(_int(value['format_version'],1)and value['capability']=='recovery_all_64_tables_after_normalization_v1'and _int(value['covered_tables'],64),'ATTENTION_FINAL_ALL_64')
 unchanged=value['unchanged'];_keys(unchanged,'format_version capability expected_sql_sha256 tables unchanged_tables separately_verified_normalized_table excluded_columns timestamptz_comparison','ATTENTION_UNCHANGED_63_FIELDS');need(_int(unchanged['format_version'],1)and unchanged['capability']=='recovery_post_normalization_unchanged_rows_v1'and _int(unchanged['unchanged_tables'],63)and unchanged['separately_verified_normalized_table']=='job'and unchanged['excluded_columns']==[]and unchanged['timestamptz_comparison']=='exact_microsecond_instant_including_infinities'and _hex(unchanged['expected_sql_sha256']),'ATTENTION_NO_GENERAL_COLUMN_EXCLUSION');_tables(unchanged['tables'],tuple(t for t in TABLES if t!='job'))
 source={r['table']:r for r in inventory['rows']['tables']}
 for row in unchanged['tables']:need(_same(row['digest'],source[row['table']]['digest'])and row['rows']==source[row['table']]['rows'],'ATTENTION_SOURCE_TARGET_ROW_DIGEST_JOIN')
 normalized=value['normalized'];_keys(normalized,'format_version capability tables rows_sha256 normalization_report_sha256 all_columns_compared','ATTENTION_COMMITTED_READBACK_FIELDS');need(_int(normalized['format_version'],1)and normalized['capability']=='recovery_committed_job_rows_rechecked_v1'and normalized['tables']==list(REFERENCE_TABLES)and normalized['normalization_report_sha256']==normalization_sha and normalized['all_columns_compared']is True and normalized['rows_sha256']==sha(canonical(after)),'ATTENTION_COMPLETE_COMMITTED_SIX_TABLE_READBACK')

def _delivery(value):
 _keys(value,'count deliveries execution recovery_copy_receipt_sha256 source_access runtime_reconnected republished','ATTENTION_HELD_DELIVERY_FIELDS');need(_int(value['count'],1)and type(value['deliveries'])is list and len(value['deliveries'])==1 and value['execution']=='REAL_SNAPSHOT_DELIVERY_CORE_ON_HELD_MANAGEMENT_TRANSACTION_RECOVERY_CONTROL_COPY'and _hex(value['recovery_copy_receipt_sha256'])and all(value[k]is False for k in ('source_access','runtime_reconnected','republished')),'ATTENTION_REAL_MANAGEMENT_HELD_DELIVERY')
 row=value['deliveries'][0];_keys(row,'job_id stage_token manifest_sha256 files','ATTENTION_HELD_DELIVERY_IDENTITY');need(_uuid(row['job_id'])and _uuid(row['stage_token'])and _hex(row['manifest_sha256']),'ATTENTION_HELD_DELIVERY_IDENTIFIERS');files=row['files'];need(type(files)is list and [r['path']for r in files]==['manifest.json','reading.html','reading.md','validation.json'],'ATTENTION_EXACT_FOUR_READING_COPY_FILES')
 for row in files:_keys(row,'path bytes sha256','ATTENTION_HELD_FILE_FIELDS');need(_int(row['bytes'])and row['bytes']>0 and _hex(row['sha256']),'ATTENTION_HELD_FILE_ACTUAL_HASH')

def _validate_bundle(bundle,plan,target,capture):
 case=plan['case_name'];files=fixed_proof_files(case);need(type(bundle)is dict and set(bundle)=={r[0]for r in files},'ATTENTION_EXACT_CASE_PROOF_FILES');objects={name:_json(raw)for name,raw in bundle.items()};proof=objects['full-attention-result.json'];inventory=objects['full-attention-inventory.json']
 normal=case not in ('attention-corrupt-dump','attention-root-replacement');base='format_version classification case backup_id manifest_sha256 target_database target_container_id target_stopped target_import_started source_inventory_sha256 normalization_report_sha256 pre_normalization_all_64 covered_tables logical_assets distinct_originals unique_original_bytes expected_rejection negative reading_comparison typed_semantics snapshot_delivery complete_backup usable task7_complete stop_required';extra=' old_token_rejections jobs snapshot_jobs snapshot_claims snapshot_holds post_normalization'if normal else '';extra+=' original_full_import_proof_sha256'if case!='attention-root-replacement'else '';_keys(proof,base+extra,'ATTENTION_RESULT_EXACT_FIELDS')
 need(_int(proof['format_version'],1)and proof['classification']==classification(case)and proof['case']==case and _uuid(proof['backup_id'])and _hex(proof['manifest_sha256'])and proof['target_database']=='learning_restore_c4_'+plan['target']['case_id']and proof['target_container_id']==target['container_id']and proof['target_stopped']is True,'ATTENTION_RESULT_SAME_CASE_TARGET')
 need(all(proof[k]is False for k in ('complete_backup','usable','task7_complete'))and proof['stop_required']is True and proof['expected_rejection']==EXPECTED_REJECTIONS.get(case)and all(_int(proof[k],v)for k,v in (('logical_assets',6),('distinct_originals',5),('unique_original_bytes',601)))and proof['target_import_started']is normal and proof['pre_normalization_all_64']is normal and _int(proof['covered_tables'],64 if normal else 0),'ATTENTION_RESULT_SCOPE_AND_COUNTS')
 need(type(capture)is dict and capture.get('full_retained_hashes')is True and capture.get('positive_effect_authority')is False and capture.get('complete_backup')is False and capture.get('observed_manifest_sha256')==proof['manifest_sha256']and capture.get('observed_backup_id')==proof['backup_id']and _int(capture.get('logical_rows'),6)and _int(capture.get('physical_files'),5)and _int(capture.get('unique_asset_bytes'),601),'ATTENTION_INDEPENDENT_SOURCE_BYTES_JOIN')
 need(proof['source_inventory_sha256']==sha(bundle['full-attention-inventory.json']),'ATTENTION_SOURCE_INVENTORY_ORIGINAL_SHA');roster=_inventory(inventory,proof['backup_id']);_typed(roster,capture['observed_ready_asset_rows'])
 if case!='attention-root-replacement':
  raw=bundle['full-import-result.json'];original=objects['full-import-result.json'];_keys(original,'format_version classification case backup_id manifest_sha256 target_database target_container_id target_stopped commit_attempted rollback_verified import_catalog_verified originals_verified logical_assets unique_asset_bytes wrong_endpoint_proof negative_evidence','ATTENTION_ORIGINAL_IMPORT_FIELDS')
  need(proof['original_full_import_proof_sha256']==sha(raw)and _int(original['format_version'],1)and original['classification']=='FULL_IMPORT_CASE_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE'and original['case']==('full-bad-input'if not normal else 'full-import')and all(_same(proof[k],original[k])for k in ('backup_id','manifest_sha256','target_database','target_container_id','target_stopped'))and original['commit_attempted']is normal and original['import_catalog_verified']is normal and original['originals_verified']is normal and original['rollback_verified']is False and original['wrong_endpoint_proof']is None and _int(original['logical_assets'],6)and _int(original['unique_asset_bytes'],601),'ATTENTION_ORIGINAL_IMPORT_RAW_IDENTITY_AND_OUTCOME')
  if normal:need(original['negative_evidence']is None,'ATTENTION_GOOD_RESTORE_NO_BAD_SOURCE')
 if normal:
  need(all(_int(proof[k],v)for k,v in (('old_token_rejections',40),('jobs',10),('snapshot_jobs',3),('snapshot_claims',3),('snapshot_holds',3 if case=='attention-unknown-effect'else 0)))and _same(proof['typed_semantics'],roster),'ATTENTION_FIXED_NATIVE_JOB_AND_TYPED_COVERAGE')
  normalization=objects['full-attention-normalization.json'];after=_normalization(normalization,proof,bundle['full-attention-normalization.json'])
  if case=='attention-unknown-effect':need(all(proof[k]is None for k in ('reading_comparison','snapshot_delivery','post_normalization')),'ATTENTION_UNKNOWN_NO_VALIDATION_OR_DELIVERY_CLAIM')
  else:need(_same(proof['reading_comparison'],inventory['reading']),'ATTENTION_EXACT_READING_PROJECTION_EQUALITY');_post(proof['post_normalization'],inventory,proof['normalization_report_sha256'],after);_delivery(proof['snapshot_delivery'])
 else:need(all(proof[k]is None for k in ('normalization_report_sha256','reading_comparison','typed_semantics','snapshot_delivery')),'ATTENTION_PREIMPORT_NO_NORMALIZATION_CLAIM')
 _negative(proof,objects,bundle)
 return proof

def _negative(proof,objects,bundle):
 case=proof['case'];negative=proof['negative']
 if case=='attention-full-chain':need(negative is None,'ATTENTION_POSITIVE_NO_INJECTED_FAILURE');return
 common='boundary injected refused unchanged bad_input_import_started previous_pin_verified previous_pin_restore_verified';extra=''
 if case in ('attention-missing-asset','attention-interrupted-transfer'):extra=' before_sha256 after_sha256 audit_timing failed_new_capture_preserves_previous_pin'
 if case=='attention-interrupted-transfer':extra+=' failed_new_backup_id failed_new_capture_stage previous_pin_verified_after_failed_capture operation_order'
 if case=='attention-corrupt-dump':extra=' before_sha256 after_sha256 original_negative_inventory_sha256'
 if case=='attention-root-replacement':extra=' before_sha256 after_sha256 held_identity replacement_identity'
 if case=='attention-unknown-effect':extra=' snapshot_holds snapshot_runnable'
 _keys(negative,common+extra,'ATTENTION_NEGATIVE_EXACT_FIELDS');need(negative['injected']is True and negative['refused']is True and negative['previous_pin_verified']is True,'ATTENTION_REAL_INJECTION_AND_REFUSAL')
 boundaries={'attention-missing-asset':'DESTINATION_DECLARED_ORIGINAL_OPEN','attention-corrupt-dump':'DUMP_FREEZE_CUSTOM_HEADER','attention-interrupted-transfer':'DESTINATION_AFTER_TARGET_CHUNK','attention-unknown-effect':'POST_IMPORT_EXTERNAL_EFFECT_VALIDATION','attention-root-replacement':'HELD_TARGET_CONTROL_CURRENT_NAME_IDENTITY'};need(negative['boundary']==boundaries[case],'ATTENTION_FIXED_REJECTION_BOUNDARY')
 unknown=case=='attention-unknown-effect';need(negative['unchanged']is (not unknown)and negative['bad_input_import_started']is unknown and negative['previous_pin_restore_verified']is (case in ('attention-missing-asset','attention-interrupted-transfer')),'ATTENTION_NEGATIVE_IMPORT_TIMING_NOT_FALSE_NEVER_WRITE')
 if unknown:need(_int(negative['snapshot_holds'],3)and _int(negative['snapshot_runnable'],0),'ATTENTION_UNKNOWN_THREE_HOLDS_NONRUNNABLE');return
 if case=='attention-corrupt-dump':
  raw=bundle['full-negative-inventory.json'];audit=objects['full-negative-inventory.json'];_keys(audit,'before after injected refused audit_failed','ATTENTION_ORIGINAL_DUMP_AUDIT_FIELDS');need(audit['injected']==audit['refused']=='dump_freeze'and audit['audit_failed']is False and _same(audit['before'],audit['after'])and negative['before_sha256']==sha(canonical(audit['before']))and negative['after_sha256']==sha(canonical(audit['after']))and negative['original_negative_inventory_sha256']==sha(raw),'ATTENTION_ORIGINAL_DUMP_AUDIT_RAW_JOIN');ref=objects['full-import-result.json']['negative_evidence'];_keys(ref,'stage sha256 bytes unchanged','ATTENTION_ORIGINAL_DUMP_REFERENCE');need(ref['stage']=='dump_freeze'and ref['sha256']==sha(raw)and _int(ref['bytes'],len(raw))and ref['unchanged']is True,'ATTENTION_ORIGINAL_DUMP_REFERENCE_JOIN')
 else:
  before,after=bundle['full-attention-negative-before.json'],bundle['full-attention-negative-after.json'];need(before==after and negative['before_sha256']==sha(before)and negative['after_sha256']==sha(after),'ATTENTION_FULL_NEGATIVE_BEFORE_AFTER_ORIGINAL_BYTES');_keys(objects['full-attention-negative-before.json'],'catalog assets attempts','ATTENTION_COMPLETE_NEGATIVE_TARGET_INVENTORY')
 if case in ('attention-missing-asset','attention-interrupted-transfer'):need(negative['audit_timing']=='GOOD_SOURCE_PACKAGE_ADMITTED_READ_ONLY_BEFORE_ANY_IMPORT'and negative['failed_new_capture_preserves_previous_pin']is (case=='attention-interrupted-transfer'),'ATTENTION_GOOD_ADMISSION_BEFORE_BAD_TRANSFER')
 if case=='attention-interrupted-transfer':need(_uuid(negative['failed_new_backup_id'])and negative['failed_new_backup_id']!=proof['backup_id']and negative['failed_new_capture_stage']=='DUMP_DURABLE_BEFORE_LOCAL_SEAL'and negative['previous_pin_verified_after_failed_capture']is True and negative['operation_order']==['GOOD_CAPTURE','FAILED_NEW_CAPTURE_DUMP_DURABLE','INTERRUPTED_NEW_DESTINATION_COPY','PREVIOUS_PIN_REAL_RESTORE_AND_EXACT_STOP'],'ATTENTION_FAILED_NEW_CAPTURE_THEN_OLD_PIN_REAL_RESTORE')
 if case=='attention-root-replacement':need(type(negative['held_identity'])is list and type(negative['replacement_identity'])is list and len(negative['held_identity'])==len(negative['replacement_identity'])==2 and all(type(x)is int and x>0 for row in (negative['held_identity'],negative['replacement_identity'])for x in row)and negative['held_identity']!=negative['replacement_identity'],'ATTENTION_CURRENT_NAME_REPLACED_HELD_INODE')


def capture_full_chain(driver,runner,container,record,inventory):
 """Independent fixed byte/SQL observation, never a snapshot effect issuer."""
 worker_observation=validate_worker_results(record)
 raw=driver.sql(runner,container,"SELECT space_id::text,id::text,sha256,byte_size::text,storage_key FROM public.asset WHERE status='ready' ORDER BY space_id,id;",record['identity']['database'],'learning_admin')
 rows=[]
 for line in raw.splitlines():
  parts=line.split('|');need(len(parts)==5 and parts[3].isdigit(),'ATTENTION_READY_SQL_ROW');space,asset,digest,size,key=parts;rows.append(dict(space_id=space,id=asset,sha256=digest,byte_size=int(size),storage_key=key))
 counts=validate_ready_rows(rows);indexed=[path for path in inventory if re.fullmatch(r'pins/[0-9a-f-]{36}\.sealed/asset-index\.json',path)];need(len(indexed)==1,'ATTENTION_UNIQUE_CAPTURE_INDEX');path=indexed[0];prefix=path.removesuffix('asset-index.json')
 code,index_raw,err=driver.pg_exec(runner,container,['cat','/var/lib/knowweave-source/'+path]);need(type(code)is int and code==0 and err==b'','ATTENTION_CAPTURE_INDEX_READ');checked=driver.validate_index(index_raw,rows,fixture_counts=False)
 manifest_entry=inventory.get(prefix+'manifest.json');need(type(manifest_entry)is dict and type(manifest_entry.get('raw_hex'))is str,'ATTENTION_CAPTURE_MANIFEST_ORIGINAL');manifest_raw=bytes.fromhex(manifest_entry['raw_hex'])
 manifest=json.loads(manifest_raw,object_pairs_hook=driver.unique_pairs);need(list(manifest)==['capability','format_version','backup_id','source','logical_asset_count','unique_asset_bytes','files']and json.dumps(manifest,ensure_ascii=False,separators=(',',':')).encode()==manifest_raw,'ATTENTION_CAPTURE_MANIFEST_CANONICAL')
 need(manifest['capability']=='backup_full_v1'and type(manifest['format_version'])is int and manifest['format_version']==1 and manifest['backup_id']==prefix.split('/')[1].removesuffix('.sealed')and type(manifest['logical_asset_count'])is int and manifest['logical_asset_count']==6 and type(manifest['unique_asset_bytes'])is int and manifest['unique_asset_bytes']==601,'ATTENTION_CAPTURE_MANIFEST_COUNTS')
 migrations=record['migrations'];expected_source=dict(application_build_sha256=record['archive_sha256'],application_commit=driver.BASE_COMMIT,postgres_major=18,migration_version=migrations[-1]['version'],migrations=migrations,migration_fingerprint=sha(json.dumps(migrations,separators=(',',':')).encode()))
 need(canonical(manifest['source'])==canonical(expected_source),'ATTENTION_CAPTURE_MANIFEST_CURRENT_SOURCE')
 fixed={x['storage_key']:x for x in fixed_originals()};expected={'asset-index.json','database.dump','roles.json'}|{'assets/'+key for key in fixed};file_rows=manifest['files'];need(type(file_rows)is list and [x['path']for x in file_rows]==sorted(expected),'ATTENTION_CAPTURE_MANIFEST_FIXED_FILES')
 expected_paths={prefix+'manifest.json'};expected_dirs=set()
 for row in file_rows:
  need(type(row)is dict and set(row)=={'path','size','sha256'}and type(row['size'])is int and row['size']>=0 and type(row['sha256'])is str and re.fullmatch('[0-9a-f]{64}',row['sha256'])is not None,'ATTENTION_CAPTURE_FILE_RECORD')
  entry=inventory.get(prefix+row['path']);need(type(entry)is dict and type(entry.get('bytes'))is int and entry['bytes']==row['size']and entry.get('sha256')==row['sha256'],'ATTENTION_RETAINED_FILE_HASH');expected_paths.add(prefix+row['path'])
  need(type(entry.get('uid'))is int and entry['uid']==0 and type(entry.get('mode'))is int and entry['mode']==0o400 and type(entry.get('links'))is int and entry['links']==1,'ATTENTION_SEALED_READONLY_ROOT_FILE_METADATA')
  parts=row['path'].split('/')
  for end in range(1,len(parts)):expected_dirs.add(prefix+'/'.join(parts[:end]))
 need({p for p,v in inventory.items()if p.startswith(prefix)and not v.get('directory')}==expected_paths and {p for p,v in inventory.items()if p.startswith(prefix)and v.get('directory')}==expected_dirs,'ATTENTION_SEALED_EXACT_TREE')
 source_files={p:v for p,v in inventory.items()if p.startswith('assets/')and not v.get('directory')};need(set(source_files)=={'assets/'+key for key in fixed},'ATTENTION_SOURCE_ORIGINAL_EXACT_ROSTER')
 source_dirs={p for p,v in inventory.items()if p.startswith('assets/')and v.get('directory')};need(source_dirs=={'assets/sha256'}|{'assets/sha256/'+x['sha256'][:2]for x in fixed.values()},'ATTENTION_SOURCE_ORIGINAL_EXACT_DIRECTORIES')
 for paths,mode in ((source_dirs,0o700),(expected_dirs,0o500)):
  need(all(type(inventory[p].get('uid'))is int and inventory[p]['uid']==0 and type(inventory[p].get('mode'))is int and inventory[p]['mode']==mode for p in paths),'ATTENTION_SOURCE_AND_SEALED_DIRECTORY_MODES')
 for key,item in fixed.items():
  for entry,mode in ((source_files['assets/'+key],0o600),(inventory.get(prefix+'assets/'+key),0o400)):
   need(type(entry)is dict and all(type(entry.get(k))is int for k in ('bytes','uid','mode','links'))and entry['bytes']==item['bytes']and entry.get('sha256')==item['sha256']and entry['uid']==0 and entry['links']==1 and entry['mode']==mode,'ATTENTION_SOURCE_AND_SEALED_FIXED_ORIGINAL_BYTES')
 code,toc,err=driver.pg_exec(runner,container,['/usr/lib/postgresql/18/bin/pg_restore','--list','/var/lib/knowweave-source/'+prefix+'database.dump']);toc_receipt=driver.validate_toc(code,toc,err,record['identity']['database'])
 from pathlib import Path
 driver.new_private(Path(record['evidence_directory'])/'attention-capture.toc',toc)
 checked.update(counts,original_policy='attention_fixed_five_physical_six_logical_originals',full_retained_hashes=True,toc_sha256=sha(toc),toc_only_no_restore=True,toc=toc_receipt,observed_manifest_sha256=sha(manifest_raw),observed_backup_id=manifest['backup_id'],observed_ready_asset_rows=rows,observed_workers=worker_observation,positive_effect_authority=False,complete_backup=False)
 return checked
