"""Fixed Attention asset observer tests; no Docker, native or authority proof."""
import copy,hashlib,json,tempfile
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from scripts.p0c4_completion import attention as a,snapshot_worker as worker,full_import as full
from scripts import p0c4_source_lifecycle_gate_acceptance as d

def rows():
 result=[];counter=1
 for item in a.fixed_originals():
  for ordinal in range(item['logical_registrations']):
   result.append(dict(space_id=f'{ordinal+1}50e8400-e29b-41d4-a716-446655440000',id=f'{counter:08x}-e29b-41d4-a716-446655440000',sha256=item['sha256'],byte_size=item['bytes'],storage_key=item['storage_key']));counter+=1
 return sorted(result,key=lambda x:(x['space_id'],x['id']))
def seed_observation():
 return dict(scope='FIXED_FRESH_ATTENTION_BYTES_PRESEEDED_NOT_ASSET_REGISTRATION',physical_files=5,logical_ready_assets=6,unique_original_bytes=601,files=[dict(storage_key=x['storage_key'],bytes=x['bytes'],sha256=x['sha256'],dev=10,ino=i+1,uid=0,gid=0,mode=0o600,nlink=1)for i,x in enumerate(a.fixed_originals())])
def worker_observation():
 return dict(snapshot_worker_binary=dict(exit_code=0,sha256='9'*64),snapshot_worker_results=[dict(op='snapshot-worker-result',request_id=f'{i+1:08x}-e29b-41d4-a716-446655440000',client_exit=0,disposition=status,client_stdout_sha256=a.sha(worker.scheduling_output(status)),client_stderr_sha256=a.sha(b''),client_container_id=str(i+1)*64,worker_binary_sha256='9'*64)for i,status in enumerate(('COMPLETED','REVOKED_BEFORE_WRITE','REVOKED_BEFORE_WRITE'))])
class AttentionAssetsTests(unittest.TestCase):
 def test_fixed_five_byte_sources_and_six_logical_registrations(self):
  fixed=a.fixed_originals();self.assertEqual((len(fixed),sum(x['bytes']for x in fixed),sum(x['logical_registrations']for x in fixed)),(5,601,6));self.assertEqual(a.validate_ready_rows(rows()),dict(logical_assets=6,physical_files=5,unique_asset_bytes=601))
 def test_bad_bytes_and_storage_key_reject(self):
  for field,value in [('base64','AAAA'),('bytes',414.0),('sha256','0'*64),('storage_key','../caller')]:
   source=copy.deepcopy(a.ORIGINALS);source[0][field]=value
   with patch.object(a,'ORIGINALS',source),self.assertRaises(RuntimeError):a.fixed_originals()
 def test_ready_duplicate_missing_extra_or_unshared_pdf_reject(self):
  for change in (lambda v:v.pop(),lambda v:v.append(copy.deepcopy(v[0])),lambda v:v[1].update(id=v[0]['id'],space_id=v[0]['space_id']),lambda v:v[0].update(byte_size=True),lambda v:v[0].update(storage_key='../caller')):
   value=rows();change(value)
   with self.assertRaises(RuntimeError):a.validate_ready_rows(value)
  value=rows();pdf=[x for x in value if x['sha256']==a.ORIGINALS[0]['sha256']];pdf[1]['space_id']=pdf[0]['space_id']
  with self.assertRaises(RuntimeError):a.validate_ready_rows(value)
 def test_preseed_observation_strict_metadata_and_counts(self):
  a.validate_preseed_observation(seed_observation())
  for change in (lambda v:v.update(physical_files=5.0),lambda v:v['files'][0].update(ino=True),lambda v:v['files'][0].update(nlink=2),lambda v:v['files'][0].update(mode=0o644),lambda v:v['files'][0].update(uid=999),lambda v:v['files'][1].update(ino=v['files'][0]['ino']),lambda v:v['files'].pop()):
   value=seed_observation();change(value)
   with self.assertRaises(RuntimeError):a.validate_preseed_observation(value)
 def test_preseed_is_a_separate_fixed_fd_program(self):
  program=a.preseed_program();compile(program,'fixed-preseed','exec');self.assertIn('os.O_EXCL|os.O_NOFOLLOW',program)
  self.assertNotIn('objects=',worker.PREPARE_ROOTS);self.assertNotIn('snapshot-sockets',program)
 def test_positive_and_negative_labels_are_distinct(self):
  self.assertEqual(a.classification('attention-full-chain'),a.POSITIVE)
  for case in a.EXPECTED_REJECTIONS:self.assertEqual(a.classification(case),a.NEGATIVE)
  with self.assertRaises(RuntimeError):a.classification('attention-caller')
 def fixture(self):
  data=rows();index=json.dumps(dict(format_version=1,assets=data),separators=(',',':')).encode();backup='550e8400-e29b-41d4-a716-446655440000';prefix='pins/'+backup+'.sealed/';fixed=a.fixed_originals();inventory={};record=dict(identity=dict(database='fixed'),archive_sha256='a'*64,migrations=[dict(version=1,checksum='b'*96)],evidence_directory='/not-written')
  record.update(worker_observation())
  def file(path,size,digest,mode):inventory[path]=dict(bytes=size,sha256=digest,uid=0,mode=mode,links=1)
  def directory(path):inventory[path]=dict(directory=True,uid=0,mode=0o700 if path.startswith('assets/')else 0o500,links=2)
  file(prefix+'asset-index.json',len(index),a.sha(index),0o400);file(prefix+'database.dump',8,'d'*64,0o400);file(prefix+'roles.json',8,'e'*64,0o400);directory('assets/sha256')
  for item in fixed:
   path='assets/'+item['storage_key'];file(path,item['bytes'],item['sha256'],0o600);file(prefix+path,item['bytes'],item['sha256'],0o400);directory('assets/sha256/'+item['sha256'][:2])
   for length in range(1,len(path.split('/'))):directory(prefix+'/'.join(path.split('/')[:length]))
  source=dict(application_build_sha256=record['archive_sha256'],application_commit='c'*40,postgres_major=18,migration_version=1,migrations=record['migrations'],migration_fingerprint=a.sha(json.dumps(record['migrations'],separators=(',',':')).encode()))
  fields=[dict(path=p.removeprefix(prefix),size=x['bytes'],sha256=x['sha256'])for p,x in sorted(inventory.items())if p.startswith(prefix)and not x.get('directory')];manifest=dict(capability='backup_full_v1',format_version=1,backup_id=backup,source=source,logical_asset_count=6,unique_asset_bytes=601,files=fields);manifest_raw=json.dumps(manifest,separators=(',',':')).encode();file(prefix+'manifest.json',len(manifest_raw),a.sha(manifest_raw),0o400);inventory[prefix+'manifest.json']['raw_hex']=manifest_raw.hex()
  def pg_exec(r,c,argv):return(0,index,b'')if argv[0]=='cat'else(0,b'FAKE TOC FOR OBSERVER UNIT ONLY\n',b'')
  sql='\n'.join('|'.join((x['space_id'],x['id'],x['sha256'],str(x['byte_size']),x['storage_key']))for x in data)
  driver=SimpleNamespace(sql=lambda *args:sql,pg_exec=pg_exec,validate_index=d.validate_index,unique_pairs=d.unique_pairs,BASE_COMMIT='c'*40,validate_toc=lambda *args:{'unit_only':True},new_private=lambda *args:None)
  return driver,record,inventory
 def test_capture_observer_binds_sql_index_and_all_original_bytes(self):
  driver,record,inventory=self.fixture();value=a.capture_full_chain(driver,None,'fake',record,inventory);self.assertEqual((value['logical_rows'],value['physical_files'],value['unique_asset_bytes']),(6,5,601));self.assertIs(value['positive_effect_authority'],False)
 def test_capture_observer_rejects_extra_corrupt_alias_or_missing(self):
  for kind in ('extra','corrupt','hardlink','missing','sealed-writable','dump-bytecount'):
   driver,record,inventory=self.fixture();original=next(p for p,v in inventory.items()if p.startswith('assets/')and not v.get('directory'));sealed=next(p for p in inventory if p.startswith('pins/')and p.endswith(original))
   if kind=='extra':inventory['assets/extra']=dict(bytes=1,sha256='f'*64,uid=0,mode=0o600,links=1)
   if kind=='corrupt':inventory[original]['sha256']='0'*64
   if kind=='hardlink':inventory[original]['links']=2
   if kind=='missing':del inventory[original]
   if kind=='sealed-writable':inventory[sealed]['mode']=0o600
   if kind=='dump-bytecount':next(v for p,v in inventory.items()if p.endswith('/database.dump'))['bytes']=9
   with self.subTest(kind=kind),self.assertRaises(RuntimeError):a.capture_full_chain(driver,None,'fake',record,inventory)
 def test_wire_is_still_failclosed_until_rust_contract_is_frozen(self):
  with patch.object(a,'WIRE_CONTRACT',None),self.assertRaisesRegex(RuntimeError,'ATTENTION_PROOF_CONTRACT_NOT_FROZEN'):a.validate_bundle({}, {}, {}, {})
 def test_three_worker_natural_receipts_are_required_without_widening_leaf_profile(self):
  self.assertEqual(a.validate_worker_results(worker_observation())['worker_signals'],3)
  for change in (lambda r:r['snapshot_worker_results'].pop(),lambda r:r['snapshot_worker_results'][0].update(client_exit=True),lambda r:r['snapshot_worker_results'][1].update(disposition='COMPLETED'),lambda r:r['snapshot_worker_results'][0].update(worker_binary_sha256='8'*64),lambda r:r['snapshot_worker_results'][0].update(client_stderr_sha256='0'*64)):
   record=worker_observation();change(record)
   with self.assertRaises(a.AttentionError):a.validate_worker_results(record)

def proof_fixture(case):
 """Serializer fixtures only; these objects grant no native acceptance."""
 from scripts.test_p0c4_attention_acceptance import plan
 value=plan(case);ready=rows();counter=100
 def uid():
  nonlocal counter
  counter+=1;return f'{counter:08x}-e29b-41d4-a716-446655440000'
 backup=uid();manifest='c'*64;target=dict(container_id='d'*64);space,source_space=sorted({r['space_id']for r in ready});block=uid();view=uid()
 roster={key:None for key in 'actor peer space source_space hypothesis_historical hypothesis_current historical_reading current_reading reuse_reading compositions original_figure personal_figure attachment source_run evidence relations epistemic_review conclusion release_id lineage_operation lineage_outputs migration_proposal unplaced resources segments assets unlinked_ready shared_digest import_request import_manifest_sha256 job_edge_reading expected_h1 expected_h2 reachability functions_and_acl'.split()}
 roster.update(actor=uid(),peer=uid(),space=space,source_space=source_space,hypothesis_historical=dict(block_id=block,revision_id=uid()),hypothesis_current=dict(block_id=block,revision_id=uid()),historical_reading=dict(view_id=view,revision_id=uid()),current_reading=dict(view_id=view,revision_id=uid()),reuse_reading=dict(view_id=uid(),revision_id=uid()),job_edge_reading=dict(view_id=uid(),revision_id=uid()),release_id=uid(),lineage_operation=uid(),migration_proposal=uid(),import_request=uid(),assets=[dict(space_id=r['space_id'],asset_id=r['id'])for r in ready],shared_digest=a.fixed_originals()[0]['sha256'],import_manifest_sha256='f'*64,unplaced=[{}, {}, {}],compositions=[{}, {}],evidence=[{}, {}, {}],expected_h1='H@1: toy attention score improves',expected_h2='H@2: narrower claim',reachability='EVERY_ID_FIXED_TYPED_API_PROBED_OR_INTENTIONALLY_DENIED',functions_and_acl='SEPARATE_CLOSED_SOURCE_INVENTORY');roster['unlinked_ready']=roster['assets'][4]
 roster['fixed_drafts']=[[dict(block_id=uid(),revision_id=uid()),dict(body=dict(payload=dict(text='UNIT ONLY')))]for _ in range(16)]+[[roster['hypothesis_historical'],dict(body=dict(payload=dict(text=roster['expected_h1'])))],[roster['hypothesis_current'],dict(body=dict(payload=dict(text=roster['expected_h2'])))]];roster['dependency_revocation']=dict(content_denied=3,review_incomplete=1,visible_relation_assertions=1,release_denied=True,hidden_wire_checked=True,restored_exact_reading_and_release=True)
 reading=dict(format_version=1,capability='recovery_application_read_projections_v1',execution_context='EXACT_HELD_MANAGEMENT_TRANSACTION_NOT_ORDINARY_RUNTIME_CONNECTION',sql_runtime_privileges='SEPARATE_ROLE_CATALOG_AND_CONNECT_GATE',view_revisions=3,current_revisions=1,historical_revisions=2,owner_present=3,owner_without_grant_denied=0,nonexistent_actor_denied=9,existing_granted_nonowner_denied=9,views_without_existing_granted_nonowner=0,modes=['original','personal','fused'],projection_bytes=100,projection_sha256='1'*64,business_writes=0,preview_generator='ABSENT_NOT_REBUILT',search_generator='ABSENT_NOT_REBUILT')
 tables=[dict(table=t,rows=0,raw_bytes=0,digest=[i%256]*32)for i,t in enumerate(a.TABLES)]
 catalog=dict(database=dict(owner='learning_admin',acl='{learning_admin=CTc/learning_admin}'),roles=[dict(name='learning_snapshot_claim_broker',login=False)]);source=copy.deepcopy(catalog);source['roles'][0]['login']=True;source['database']['acl']='{learning_admin=CTc/learning_admin,learning_runtime=T/learning_admin}'
 inventory=dict(format_version=1,capability='attention_authoritative_inventory_v1',backup_id=backup,source_view='CLOSED_DRAINED_SAME_ADMITTED_CONNECTION_REPEATABLE_READ_READ_ONLY_BEFORE_DUMP',all_table_count=64,rows=dict(format_version=1,tables=tables,excluded_columns=[],observation='CLOSED_SOURCE_SAME_ADMITTED_CONNECTION_ALL_64_PK_ORDERED_ROWS'),catalog=source,target_expected_catalog=catalog,function_definitions=[dict(name=f'f{i:02}',args='',definition='FIXED UNIT ONLY '+str(i))for i in range(48)],reading=reading,typed_roster=roster,catalog_recovery_delta=[dict(role='learning_snapshot_claim_broker',field='login',source=True,target=False),dict(object='database',field='acl',source='{learning_admin=CTc/learning_admin,learning_runtime=T/learning_admin}',target='{learning_admin=CTc/learning_admin}')],complete_backup=False,usable=False)
 objects={'full-attention-inventory.json':inventory};normal=case not in ('attention-corrupt-dump','attention-root-replacement');unknown=case=='attention-unknown-effect'
 proof=dict(format_version=1,classification=a.classification(case),case=case,backup_id=backup,manifest_sha256=manifest,target_database='learning_restore_c4_'+value['target']['case_id'],target_container_id=target['container_id'],target_stopped=True,target_import_started=normal,source_inventory_sha256=a.sha(a.canonical(inventory)),normalization_report_sha256=None,pre_normalization_all_64=normal,covered_tables=64 if normal else 0,logical_assets=6,distinct_originals=5,unique_original_bytes=601,expected_rejection=a.EXPECTED_REJECTIONS.get(case),negative=None,reading_comparison=None,typed_semantics=None,snapshot_delivery=None,complete_backup=False,usable=False,task7_complete=False,stop_required=True)
 capture=dict(logical_rows=6,physical_files=5,unique_asset_bytes=601,full_retained_hashes=True,positive_effect_authority=False,complete_backup=False,observed_manifest_sha256=manifest,observed_backup_id=backup,observed_ready_asset_rows=ready)
 if case!='attention-root-replacement':
  original=dict(format_version=1,classification='FULL_IMPORT_CASE_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE',case='full-import'if normal else 'full-bad-input',backup_id=backup,manifest_sha256=manifest,target_database=proof['target_database'],target_container_id=target['container_id'],target_stopped=True,commit_attempted=normal,rollback_verified=False,import_catalog_verified=normal,originals_verified=normal,logical_assets=6,unique_asset_bytes=601,wrong_endpoint_proof=None,negative_evidence=None);objects['full-import-result.json']=original
 if normal:
  before=dict(tables={k:[]for k in a.REFERENCE_TABLES});before['tables']['job']=[dict(id=uid(),status='succeeded',attempt_count=i,lease_token=None,lease_expires_at=None)for i in range(10)];after=copy.deepcopy(before)
  normalization=dict(format_version=1,capability='recovery_job_normalization_v1',status='COMMITTED_NORMALIZATION_NOT_RECOVERY_ELIGIBILITY',identity=dict(backup_id=backup,manifest_sha256=manifest,database=proof['target_database'],container_id=target['container_id'],complete_backed=False),before=before,after=after,recovery_epoch_micros=1,planned_delta=[],applied_delta=[],holds=[] if not unknown else [dict(job_id=str(i))for i in range(3)],held_jobs=3 if unknown else 0,has_holds=unknown,external_snapshot_bytes_verified=not unknown,positive_snapshot_findings=0 if unknown else 3,normalization_commit_known=True,native_old_token_rejection_verified=False,usable=False);objects['full-attention-normalization.json']=normalization;normsha=a.sha(a.canonical(normalization))
  proof.update(old_token_rejections=40,jobs=10,snapshot_jobs=3,snapshot_claims=3,snapshot_holds=3 if unknown else 0,typed_semantics=copy.deepcopy(roster),post_normalization=None,normalization_report_sha256=normsha)
  if not unknown:
   unchanged=dict(format_version=1,capability='recovery_post_normalization_unchanged_rows_v1',expected_sql_sha256='2'*64,tables=[r for r in tables if r['table']!='job'],unchanged_tables=63,separately_verified_normalized_table='job',excluded_columns=[],timestamptz_comparison='exact_microsecond_instant_including_infinities');normalized=dict(format_version=1,capability='recovery_committed_job_rows_rechecked_v1',tables=list(a.REFERENCE_TABLES),rows_sha256=a.sha(a.canonical(after)),normalization_report_sha256=normsha,all_columns_compared=True)
   proof['post_normalization']=dict(format_version=1,capability='recovery_all_64_tables_after_normalization_v1',unchanged=unchanged,normalized=normalized,covered_tables=64);proof['reading_comparison']=copy.deepcopy(reading);proof['snapshot_delivery']=dict(count=1,deliveries=[dict(job_id=uid(),stage_token=uid(),manifest_sha256='3'*64,files=[dict(path=p,bytes=1,sha256='4'*64)for p in ('manifest.json','reading.html','reading.md','validation.json')])],execution='REAL_SNAPSHOT_DELIVERY_CORE_ON_HELD_MANAGEMENT_TRANSACTION_RECOVERY_CONTROL_COPY',recovery_copy_receipt_sha256='5'*64,source_access=False,runtime_reconnected=False,republished=False)
 if case!='attention-full-chain':
  negative=dict(boundary={'attention-missing-asset':'DESTINATION_DECLARED_ORIGINAL_OPEN','attention-corrupt-dump':'DUMP_FREEZE_CUSTOM_HEADER','attention-interrupted-transfer':'DESTINATION_AFTER_TARGET_CHUNK','attention-unknown-effect':'POST_IMPORT_EXTERNAL_EFFECT_VALIDATION','attention-root-replacement':'HELD_TARGET_CONTROL_CURRENT_NAME_IDENTITY'}[case],injected=True,refused=True,unchanged=not unknown,bad_input_import_started=unknown,previous_pin_verified=True,previous_pin_restore_verified=case in ('attention-missing-asset','attention-interrupted-transfer'));proof['negative']=negative
  if unknown:negative.update(snapshot_holds=3,snapshot_runnable=0)
  elif case=='attention-corrupt-dump':
   data=dict(catalog={},assets={},attempts={});part=a.canonical(data)
   auditraw=b'{"audit_failed":false,"injected":"dump_freeze","refused":"dump_freeze","before":'+part+b',"after":'+part+b'}'
   before_name='full-attention-corrupt-before.json';after_name='full-attention-corrupt-after.json'
   objects[before_name]=copy.deepcopy(data);objects[after_name]=copy.deepcopy(data)
   objects['full-attention-corrupt-audit-index.json']=dict(format_version=1,capability='attention_legacy_dump_audit_split_v1',case=case,injected='dump_freeze',refused='dump_freeze',audit_failed=False,before=dict(path=before_name,size=len(part),sha256=a.sha(part)),after=dict(path=after_name,size=len(part),sha256=a.sha(part)),legacy=dict(path='full-negative-inventory.json',size=len(auditraw),sha256=a.sha(auditraw),encoding='SERDE_NEGATIVE_EVIDENCE_DUMP_FREEZE_V1'))
   negative.update(before_sha256=a.sha(part),after_sha256=a.sha(part),original_negative_inventory_sha256=a.sha(auditraw));original['negative_evidence']=dict(stage='dump_freeze',sha256=a.sha(auditraw),bytes=len(auditraw),unchanged=True)
  else:
   data=dict(catalog={},assets={},attempts={});objects['full-attention-negative-before.json']=copy.deepcopy(data);objects['full-attention-negative-after.json']=copy.deepcopy(data);negative.update(before_sha256=a.sha(a.canonical(data)),after_sha256=a.sha(a.canonical(data)))
   if case=='attention-root-replacement':negative.update(held_identity=[1,2],replacement_identity=[1,3])
   else:negative.update(audit_timing='GOOD_SOURCE_PACKAGE_ADMITTED_READ_ONLY_BEFORE_ANY_IMPORT',failed_new_capture_preserves_previous_pin=case=='attention-interrupted-transfer')
   if case=='attention-interrupted-transfer':negative.update(failed_new_backup_id=uid(),failed_new_capture_stage='DUMP_DURABLE_BEFORE_LOCAL_SEAL',previous_pin_verified_after_failed_capture=True,operation_order=['GOOD_CAPTURE','FAILED_NEW_CAPTURE_DUMP_DURABLE','INTERRUPTED_NEW_DESTINATION_COPY','PREVIOUS_PIN_REAL_RESTORE_AND_EXACT_STOP'])
 if case!='attention-root-replacement':proof['original_full_import_proof_sha256']=a.sha(a.canonical(original))
 objects['full-attention-result.json']=proof;return objects,value,target,capture

class AttentionProofTests(unittest.TestCase):
 def check(self,objects,value,target,capture):
  with patch.object(a,'WIRE_CONTRACT','UNIT_FIXED_SCHEMA_ONLY'):return a.validate_bundle({k:a.canonical(v)for k,v in objects.items()},value,target,capture)
 def test_six_exact_case_shapes_and_preimport_vs_postimport_rejection(self):
  for case in ('attention-full-chain',*a.EXPECTED_REJECTIONS):
   with self.subTest(case=case):
    fixture=proof_fixture(case);self.assertEqual(self.check(*fixture)['classification'],a.classification(case))
 def test_typed_counts_and_no_production_scope(self):
  for change in (lambda p:p.update(covered_tables=64.0),lambda p:p.update(snapshot_claims=True),lambda p:p.update(complete_backup=True),lambda p:p.update(task7_complete=True),lambda p:p.update(classification=a.NEGATIVE),lambda p:p.update(target_container_id='e'*64),lambda p:p.update(manifest_sha256='e'*64),lambda p:p.update(backup_id='e'*36)):
   objects,value,target,capture=proof_fixture('attention-full-chain');change(objects['full-attention-result.json'])
   with self.assertRaises(a.AttentionError):self.check(objects,value,target,capture)
 def test_raw_inventory_and_sealed_originals_are_required(self):
  for change in (lambda c:c.update(full_retained_hashes=False),lambda c:c.update(positive_effect_authority=True),lambda c:c.update(observed_backup_id='x'),lambda c:c.update(observed_ready_asset_rows=[])):
   objects,value,target,capture=proof_fixture('attention-full-chain');change(capture)
   with self.assertRaises(a.AttentionError):self.check(objects,value,target,capture)
 def test_full_row_and_reference_delta_not_boolean_only(self):
  for change in (lambda n:n['after']['tables']['job_outbox'].append({}),lambda n:n['after']['tables']['job'][0].update(attempt_count=999),lambda n:n.update(normalization_commit_known=1),lambda n:n.update(positive_snapshot_findings=3.0)):
   objects,value,target,capture=proof_fixture('attention-full-chain');change(objects['full-attention-normalization.json']);objects['full-attention-result.json']['normalization_report_sha256']=a.sha(a.canonical(objects['full-attention-normalization.json']))
   with self.assertRaises(a.AttentionError):self.check(objects,value,target,capture)
 def test_exact_all_table_catalog_typed_and_delivery_joins(self):
  changes=(lambda p:p['post_normalization']['unchanged']['tables'].pop(),lambda p:p['post_normalization']['unchanged']['tables'][0].update(digest=[99]*32),lambda p:p['typed_semantics'].update(expected_h1='new head'),lambda p:p['reading_comparison'].update(business_writes=1),lambda p:p['snapshot_delivery']['deliveries'][0]['files'].pop(),lambda p:p['snapshot_delivery'].update(source_access=True),lambda p:p['post_normalization']['normalized'].update(all_columns_compared=1))
  for change in changes:
   objects,value,target,capture=proof_fixture('attention-full-chain');change(objects['full-attention-result.json'])
   with self.assertRaises(a.AttentionError):self.check(objects,value,target,capture)
 def test_negative_original_bytes_and_timing_refuse_tampering(self):
  for case in ('attention-missing-asset','attention-interrupted-transfer','attention-root-replacement'):
   objects,value,target,capture=proof_fixture(case);objects['full-attention-negative-after.json']['assets']['changed']=True
   with self.assertRaises(a.AttentionError):self.check(objects,value,target,capture)
  for case,change in (('attention-corrupt-dump',lambda p:p['negative'].update(original_negative_inventory_sha256='0'*64)),('attention-interrupted-transfer',lambda p:p['negative']['operation_order'].reverse()),('attention-root-replacement',lambda p:p['negative'].update(replacement_identity=p['negative']['held_identity'])),('attention-unknown-effect',lambda p:p['negative'].update(unchanged=True)),('attention-unknown-effect',lambda p:p.update(snapshot_delivery={}))):
   objects,value,target,capture=proof_fixture(case);change(objects['full-attention-result.json'])
   with self.assertRaises(a.AttentionError):self.check(objects,value,target,capture)
 def test_extra_noncanonical_duplicate_or_nonfinite_json_is_refused(self):
  objects,value,target,capture=proof_fixture('attention-full-chain');bundle={k:a.canonical(v)for k,v in objects.items()};name='full-attention-result.json';original=bundle[name]
  for raw in (b' '+original,original[:-1]+b',"covered_tables":64}',original.replace(b'"covered_tables":64',b'"covered_tables":NaN'),original.replace(b'"covered_tables":64',b'"covered_tables":Infinity')):
   with patch.object(a,'WIRE_CONTRACT','UNIT_FIXED_SCHEMA_ONLY'),self.assertRaises(a.AttentionError):a.validate_bundle(dict(bundle,**{name:raw}),value,target,capture)
  with patch.object(a,'WIRE_CONTRACT','UNIT_FIXED_SCHEMA_ONLY'),self.assertRaises(a.AttentionError):a.validate_bundle(dict(bundle,**{'full-production-proof.json':b'{}'}),value,target,capture)
 def test_attention_cap_covers_full_rows_roster_and_functions_above_16k(self):
  objects,value,target,capture=proof_fixture('attention-full-chain');inventory=objects['full-attention-inventory.json'];inventory['function_definitions'][0]['definition']='FIXED UNIT ONLY '+('x'*20000);objects['full-attention-result.json']['source_inventory_sha256']=a.sha(a.canonical(inventory));self.assertGreater(len(a.canonical(inventory)),16384);self.check(objects,value,target,capture)
 def test_only_exact_source_temp_and_broker_login_delta_is_accepted(self):
  for change in (lambda i:i['catalog']['database'].update(acl='{learning_admin=CTc/learning_admin,learning_runtime=Tc/learning_admin}'),lambda i:i['target_expected_catalog']['database'].update(acl='{learning_admin=CTc/learning_admin,learning_runtime=T/learning_admin}'),lambda i:i['catalog']['database'].update(owner='postgres'),lambda i:i['catalog']['roles'][0].update(login=False),lambda i:i['catalog_recovery_delta'].pop(),lambda i:i['catalog'].update(other_grant=True)):
   objects,value,target,capture=proof_fixture('attention-full-chain');change(objects['full-attention-inventory.json']);objects['full-attention-result.json']['source_inventory_sha256']=a.sha(a.canonical(objects['full-attention-inventory.json']))
   with self.assertRaises(a.AttentionError):self.check(objects,value,target,capture)
 def test_body_reader_saves_only_valid_fixed_originals_after_independent_capture(self):
  import sys
  objects,value,target,capture=proof_fixture('attention-full-chain');bundle={k:a.canonical(v)for k,v in objects.items()};context=full.FullRehearsalContext(full._TOKEN,value);context._runner=None;context._target_status=target;saved={};context._h=SimpleNamespace(write_new=lambda path,raw:saved.setdefault(path.name,raw));context._case=Path('/unit-only');record=dict(container_id='fake',snapshot_capture_byte_audit=capture);fake=SimpleNamespace(pg_exec=lambda r,c,args:(0,bundle[args[-1].split('/')[-1]],b''))
  inventory={'proofs':dict(directory=True,dev=1,ino=2,uid=0,mode=0o700)}
  inventory.update({'proofs/'+name:dict(bytes=len(raw),sha256=a.sha(raw),mode=0o600,uid=0,links=1)for name,raw in bundle.items()});raw_inventory=a.canonical(inventory)
  context._record=record;context._phase='COMPLETE';record.update(identity=dict(case_id=value['case_id']),test=context._selected_test(),stage='actual-libtest',full_profile_stage='VERIFIED',outcome=dict(exit_code=0,passed=1,failed=0,ignored=0),inventory_sha256=a.sha(raw_inventory))
  fake.LIMIT=2*1024**2;fake.unique_pairs=a._pairs;fake.canonical=a.canonical
  with patch.dict(sys.modules,{'p0c4_source_lifecycle_gate_acceptance':fake}),patch.object(full,'_measured_regular',return_value=(raw_inventory,{})):context._verify_attention_body(record)
  self.assertEqual(saved,bundle);self.assertEqual(record['full_attention_proof']['sha256'],a.sha(bundle['full-attention-result.json']))
if __name__=='__main__':unittest.main()
