"""Fixed Attention admission only; no native/fixture success is fabricated."""
import copy
from types import MappingProxyType
import unittest
from unittest.mock import patch
from scripts.p0c4_completion import full_import as full,plan as envelope,snapshot_worker
from scripts import p0c4_completion_acceptance as runner

CASES={
 'attention-full-chain':'attention_full_chain_preserves_authoritative_inventory_and_originals',
 'attention-missing-asset':'attention_missing_asset',
 'attention-corrupt-dump':'attention_corrupt_dump',
 'attention-interrupted-transfer':'attention_interrupted_transfer',
 'attention-unknown-effect':'attention_unknown_effect',
 'attention-root-replacement':'attention_root_replacement',
}
PREFIX='restore_preflight::target_binding::controlled_import::full::attention_tests::'
S='550e8400-e29b-41d4-a716-446655440000';T='650e8400-e29b-41d4-a716-446655440000'
def plan(case='attention-full-chain'):
 return dict(format_version=3,capability='c4_full_snapshot_import_plan_v2',batch_id='750e8400-e29b-41d4-a716-446655440000',case_id=S,cache_scope='850e8400-e29b-41d4-a716-446655440000',case_name=case,source=dict(archive=dict(path='/frozen/source-public.zip',size=1024,sha256='a'*64),manifest=dict(path='/frozen/source-manifest.json',size=1024,sha256='b'*64),application_commit='c'*40,application_build_sha256='a'*64),images=dict(builder=envelope.BUILDER,postgres=envelope.POSTGRES),subnet='172.30.100.0/24',evidence_root='/private/attention/'+S,budget_profile='full_import_task5_v1',target=dict(case_id=T,subnet='172.30.101.0/24'),schema_profile='snapshot_c0_d1_v1',snapshot_profile='root_snapshot_broker_v2')
class AttentionAdmissionTests(unittest.TestCase):
 def test_exact_six_fixed_tests_not_caller_selected(self):
  self.assertEqual(dict(full.ATTENTION_CASE_TESTS),{k:PREFIX+v for k,v in CASES.items()});self.assertIsInstance(full.ATTENTION_CASE_TESTS,MappingProxyType)
  for case in CASES:self.assertEqual(runner.ALL_CASE_TESTS[case],PREFIX+CASES[case])
  with self.assertRaises(TypeError):full.ATTENTION_CASE_TESTS['attention-arbitrary']='caller'
 def test_all_six_use_fixed_full_c_snapshot_profile(self):
  for case in CASES:
   value=plan(case);self.assertIs(envelope.validate_plan(value,case),value);context=full.FullRehearsalContext(full._TOKEN,value);self.assertTrue(context._full_c());self.assertTrue(context._snapshot_effects());self.assertEqual(context._selected_test(),PREFIX+CASES[case])
 def test_legacy_or_caller_selected_profiles_are_rejected(self):
  for case in CASES:
   base=plan(case)
   for key,value in [('format_version',1),('format_version',2),('format_version',True),('snapshot_profile','caller'),('schema_profile','base_v1'),('capability','c4_full_c_import_plan_v1')]:
    bad=copy.deepcopy(base);bad[key]=value
    with self.subTest(case=case,key=key,value=value),self.assertRaises(full.FullImportError):full.validate_plan(bad,case)
 def test_distinct_fresh_id_and_nonoverlap_requirements(self):
  for change in (lambda v:v.update(cache_scope=S),lambda v:v['target'].update(case_id=S),lambda v:v['target'].update(case_id=v['batch_id']),lambda v:v['target'].update(subnet=v['subnet']),lambda v:v.update(case_id='caller')):
   value=plan();change(value)
   with self.assertRaises((full.FullImportError,envelope.CompletionError)):envelope.validate_plan(value,value['case_name'])
 def test_source_images_budget_and_extra_authority_failclosed(self):
  for change in (lambda v:v['source'].update(application_build_sha256='f'*64),lambda v:v['source']['archive'].update(size=32*1024**2+1),lambda v:v['images'].update(builder='sha256:'+'f'*64),lambda v:v.update(budget_profile='unlimited'),lambda v:v.update(test_name='caller'),lambda v:v.update(sql='SELECT 1'),lambda v:v.update(assets_rw=True)):
   value=plan();change(value)
   with self.assertRaises((full.FullImportError,envelope.CompletionError)):envelope.validate_plan(value,value['case_name'])
 def test_old_full_and_task6_case_profiles_are_unchanged(self):
  self.assertEqual(len(full.RECOVERY_CASE_TESTS),5);self.assertEqual(len(full.SNAPSHOT_CASE_TESTS),11)
  value=plan('snapshot-p6');full.validate_plan(value,'snapshot-p6');self.assertTrue(full.FullRehearsalContext(full._TOKEN,value)._snapshot_effects())
  old=plan('full-import');old.update(format_version=1,capability='c4_full_import_plan_v1');del old['schema_profile'];del old['snapshot_profile'];full.validate_plan(old,'full-import');self.assertFalse(full.FullRehearsalContext(full._TOKEN,old)._snapshot_effects())
 def test_attention_label_is_quarantined_not_complete(self):
  for case in CASES:self.assertEqual(runner.full_case_pass_status(case),'FULL_RESTORE_REHEARSAL_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE'if case=='attention-full-chain'else'EXPECTED_REJECTION_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE')
  self.assertEqual(runner.full_case_pass_status('snapshot-p6'),runner.SNAPSHOT_PASS);self.assertEqual(runner.full_case_pass_status('recovery-effects'),runner.RECOVERY_PASS)
 def test_unimplemented_attention_evidence_cannot_adopt_old_import_success(self):
  value=plan();context=full.FullRehearsalContext(full._TOKEN,value);record=dict(test=PREFIX+CASES[value['case_name']],outcome=dict(exit_code=0,passed=1,failed=0,ignored=0))
  from scripts.p0c4_completion import attention
  with patch.object(attention,'WIRE_CONTRACT',None):
   with self.assertRaisesRegex(full.FullImportError,'ATTENTION_PROOF_CONTRACT_NOT_FROZEN'):context._verify_body(record)
   with self.assertRaisesRegex(full.FullImportError,'ATTENTION_PROOF_CONTRACT_NOT_FROZEN'):full.validate_result(value,dict(cases=[record]))
 def test_original_task6_text_only_guard_is_not_removed(self):
  record=dict(test=full.SNAPSHOT_CASE_TESTS['snapshot-p6'],identity=dict(database='fake'))
  with patch.object(snapshot_worker,'_driver')as driver:
   driver.return_value.sql.return_value='1'
   with self.assertRaisesRegex(snapshot_worker.SnapshotDriverError,'SNAPSHOT_TEXT_ONLY_ASSETS'):snapshot_worker.capture_postcheck(None,'fake',record,{})
  from scripts.p0c4_completion import attention
  with patch.object(snapshot_worker,'_driver')as driver,patch.object(attention,'capture_full_chain',return_value={'unit_only':True})as capture:
   record['test']=PREFIX+CASES['attention-missing-asset']
   self.assertEqual(snapshot_worker.capture_postcheck(None,'fake',record,{}),{'unit_only':True})
   capture.assert_called_once();driver.return_value.sql.assert_not_called()
if __name__=='__main__':unittest.main()
