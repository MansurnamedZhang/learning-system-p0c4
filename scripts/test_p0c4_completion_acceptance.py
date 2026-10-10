import unittest
import copy
from pathlib import Path
from unittest.mock import patch
from scripts import p0c4_completion_acceptance as runner
from scripts.test_p0c4_storage_registry import receipt_fixture

class CompletionTests(unittest.TestCase):
    def test_destination_pass_requires_actual_body_and_cleanup_receipt(self):
        plan,_,nested,receipt=self.envelope()
        row=copy.deepcopy(self.lifecycle_envelope()[1]['cases'][0]);issuer=row['issuer']['registry_provisioning']
        name='destination-negative';test=runner.ALL_CASE_TESTS[name]
        plan.update(capability='c4_completion_destination_plan_v1',case_name=name,budget_profile='destination_task3_v1',case_id=issuer['case_id'])
        receipt.update(case_id=issuer['case_id'],case_name=name,test_name=test,status=runner.DESTINATION_PASS,plan_sha256=runner.digest(runner.canonical(plan)),registry_provisioning_sha256=runner.digest(runner.canonical(issuer)))
        state={root:dict(directory=True,dev=row['audit_before']['roots'][root]['dev'],ino=row['audit_before']['roots'][root]['ino'],uid=0,mode=0o700) for root in ('control','pins','assets','asset-staging')}
        state['control/source-binding.json']=dict(bytes=1024,sha256=issuer['source_binding_sha256'],uid=0,mode=0o600,links=1)
        row.update(test=test,kind='REAL_DESTINATION_FILES_SYNTHETIC_HISTORY_NOT_COMPLETE',generations=[],destination_acl_before='t',acl_after='t',destination_source_before=copy.deepcopy(state),destination_source_after=copy.deepcopy(state),destination_source_final=copy.deepcopy(state))
        nested['status']=runner.DESTINATION_PASS;nested['cases']=[row]
        self.check_envelope(plan,issuer,nested,receipt)
        for change in (dict(test_exit=1),dict(passed=0),dict(ignored=1)):
            with self.assertRaises(runner.CompletionError):self.check_envelope(plan,issuer,nested,dict(receipt,**change))
        bad=copy.deepcopy(nested);bad['cleanup_verified']=False
        with self.assertRaises(runner.CompletionError):self.check_envelope(plan,issuer,bad,receipt)
        for key,value in [('generations',None),('generations',[{}]),('generations',[{},{}]),('audit_before',{}),('audit_final',{}),('destination_source_after',{}),('destination_acl_before','f'),('kind','REAL_SOURCE_ENDPOINT'),('drain_backend_pid',123)]:
            bad=copy.deepcopy(nested);bad['cases'][0][key]=value
            with self.subTest(key=key,value=value),self.assertRaises((runner.CompletionError,runner.driver.GateError)):
                self.check_envelope(plan,issuer,bad,receipt)
    def test_destination_cases_are_fixed_and_never_source_or_complete_credit(self):
        for name in ('destination-negative','transfer-interrupted'):
            plan=self.envelope()[0]
            plan.update(capability='c4_completion_destination_plan_v1',case_name=name,budget_profile='destination_task3_v1')
            self.assertEqual(runner.validate_plan(plan,name),plan)
            self.assertIn(runner.ALL_CASE_TESTS[name],runner.driver.DESTINATION_TESTS)
            self.assertNotIn(name,runner.envelope.SOURCE_CASE_TESTS)
            self.assertEqual(runner.DESTINATION_PASS,'DESTINATION_TASK3_CASE_PASSED_NOT_COMPLETE')
            for change in (dict(budget_profile='source_endpoint_task2_v1'),dict(key='/tmp/private'),dict(case_name='destination-generic')):
                with self.assertRaises(runner.CompletionError):runner.validate_plan(dict(plan,**change),name)
    def test_source_cancel_requires_exact_three_distinct_fault_pairs(self):
        import uuid
        rows=[]
        for _ in range(3):
            backup=str(uuid.uuid4());epoch=str(uuid.uuid4())
            for phase,state,signal in [('pause','T','SIGSTOP'),('resume','S','SIGCONT')]:
                rows.append(dict(backup_id=backup,epoch=epoch,nonce=str(uuid.uuid4()),phase=phase,postmaster_start_ticks=55,observed_state=state,container_id='a'*64,external_signal=signal,actual_postmaster_restart=False))
        case=dict(container_id='a'*64,source_fault_transitions=rows)
        runner.validate_source_fault_transitions(case,'source-cancel')
        for badrows in [rows[:4],rows[:5],rows+rows[:2],rows[:2]*3]:
            with self.assertRaises(runner.CompletionError):runner.validate_source_fault_transitions(dict(case,source_fault_transitions=badrows),'source-cancel')
        for field,value in [('epoch',str(uuid.uuid4())),('backup_id',str(uuid.uuid4())),('nonce',rows[0]['nonce'])]:
            bad=copy.deepcopy(case);bad['source_fault_transitions'][5][field]=value
            with self.assertRaises(runner.CompletionError):runner.validate_source_fault_transitions(bad,'source-cancel')
        with self.assertRaises(runner.CompletionError):runner.validate_source_fault_transitions(case,'source-restart')
        self.assertEqual(runner.driver.source_fault_request_limit(runner.ALL_CASE_TESTS['source-cancel']),6)
        self.assertEqual(runner.driver.source_fault_request_limit(runner.ALL_CASE_TESTS['source-restart']),4)

    def test_source_fault_pass_requires_paired_external_stop_and_resume(self):
        validate=getattr(runner,'validate_source_fault_transitions',None)
        self.assertIsNotNone(validate,'external source fault evidence gate missing')
        cid='a'*64;case=dict(container_id=cid,source_fault_transitions=[])
        with self.assertRaises(runner.CompletionError):validate(case,'source-restart')
        pair=[dict(backup_id='11111111-1111-4111-8111-111111111111',epoch='22222222-2222-4222-8222-222222222222',nonce=nonce,phase=phase,postmaster_start_ticks=55,observed_state=state,container_id=cid,external_signal=signal,actual_postmaster_restart=False) for nonce,phase,state,signal in (
            ('33333333-3333-4333-8333-333333333333','pause','T','SIGSTOP'),('44444444-4444-4444-8444-444444444444','resume','S','SIGCONT'))]
        case['source_fault_transitions']=pair;validate(case,'source-restart')
        for changes in (dict(observed_state='S'),dict(container_id='b'*64),dict(actual_postmaster_restart=0),dict(external_signal='SIGKILL'),dict(postmaster_start_ticks=True)):
            bad=copy.deepcopy(case);bad['source_fault_transitions'][0].update(changes)
            with self.subTest(changes=changes),self.assertRaises(runner.CompletionError):validate(bad,'source-restart')

    def test_source_restart_receipt_requires_explicit_actual_lock_loss_without_restart_credit(self):
        plan,issuer,nested,receipt=self.envelope()
        name='source-restart';test=runner.ALL_CASE_TESTS[name]
        plan.update(capability='c4_completion_source_endpoint_plan_v1',case_name=name,budget_profile='source_endpoint_task2_v1')
        receipt.update(case_name=name,test_name=test,status=runner.SOURCE_PASS,plan_sha256=runner.digest(runner.canonical(plan)))
        nested['status']=runner.SOURCE_PASS;nested['cases'][0]['test']=test
        with self.assertRaisesRegex(runner.CompletionError,'RESULT_SOURCE_LOCK_LOSS_CLASSIFICATION'):
            self.check_envelope(plan,issuer,nested,receipt)
        nested['cases'][0].update(actual_postmaster_restart=False,source_failure_exercise='actual_random_session_lock_loss')
        cid='a'*64;nested['cases'][0]['container_id']=cid
        nested['cases'][0]['source_fault_transitions']=[dict(backup_id='11111111-1111-4111-8111-111111111111',epoch='22222222-2222-4222-8222-222222222222',nonce=nonce,phase=phase,postmaster_start_ticks=55,observed_state=state,container_id=cid,external_signal=signal,actual_postmaster_restart=False) for nonce,phase,state,signal in (
            ('33333333-3333-4333-8333-333333333333','pause','T','SIGSTOP'),('44444444-4444-4444-8444-444444444444','resume','S','SIGCONT'))]
        self.check_envelope(plan,issuer,nested,receipt)
        for change in (dict(actual_postmaster_restart=True),dict(actual_postmaster_restart=0),dict(source_failure_exercise='actual_postmaster_restart')):
            bad=copy.deepcopy(nested);bad['cases'][0].update(change)
            with self.subTest(change=change),self.assertRaisesRegex(runner.CompletionError,'RESULT_SOURCE_LOCK_LOSS_CLASSIFICATION'):
                self.check_envelope(plan,issuer,bad,receipt)

    def test_source_clone_plan_requires_fixed_distinct_two_pg_resources(self):
        plan=self.envelope()[0]
        plan.update(capability='c4_completion_source_endpoint_plan_v1',case_name='source-clone',budget_profile='source_clone_two_pg_v1',clone=dict(case_id='44444444-4444-4444-8444-444444444444',subnet='10.254.21.0/24'))
        try:accepted=runner.validate_plan(plan,'source-clone')
        except runner.CompletionError:accepted=None
        self.assertEqual(accepted,plan)
        for change in ('same-id','same-subnet','generic-cap','missing-clone','foreign-case'):
            bad=copy.deepcopy(plan)
            if change=='same-id':bad['clone']['case_id']=bad['case_id']
            elif change=='same-subnet':bad['clone']['subnet']=bad['subnet']
            elif change=='generic-cap':bad['max_pg']=2
            elif change=='missing-clone':bad.pop('clone')
            else:bad['case_name']='registry-reopen'
            with self.subTest(change=change),self.assertRaises(runner.CompletionError):runner.validate_plan(bad,'source-clone')

    def test_source_clone_ledger_keeps_default_one_and_fixed_named_two_limit(self):
        def body(h,b,owned,result,image):
            ledger=owned.ownership
            first=ledger.acquire('pg','original',dict(name='original'),dict(name='original-net'),{})
            ledger.known(first,'a'*64)
            with self.assertRaisesRegex(runner.driver.GateError,'CLEANUP_PG_COUNT'):
                ledger.acquire('pg','clone',dict(name='clone'),dict(name='clone-net'),{})
            ledger.source_clone_pg_names={'original','clone'}
            with self.assertRaisesRegex(runner.driver.GateError,'SOURCE_CLONE_PG_NAME'):
                ledger.acquire('pg','foreign',dict(name='foreign'),dict(name='foreign-net'),{})
            second=ledger.acquire('pg','clone',dict(name='clone'),dict(name='clone-net'),{})
            ledger.known(second,'b'*64)
            with self.assertRaisesRegex(runner.driver.GateError,'CLEANUP_PG_COUNT'):
                ledger.acquire('pg','third',dict(name='third'),dict(name='third-net'),{})
            ledger.release(first,'stopped');ledger.release(second,'stopped')
            raise runner.CompletionError('SOURCE_CLONE_LEDGER_ONLY')
        observed,_=self.run_with_live_boundary(body)
        self.assertEqual(str(observed['body_error']),'SOURCE_CLONE_LEDGER_ONLY')
        self.assertEqual(observed['result']['status'],'FAILED')

    def lifecycle_plan(self):
        plan=self.envelope()[0]
        plan.pop('case_id');plan.pop('subnet')
        plan.update(capability='c4_completion_source_regression_plan_v1',case_name='source-lifecycle-eight',budget_profile='source_lifecycle_eight_v1',
                    cases=[dict(case_id=f'00000000-0000-4000-8000-{i:012d}',subnet=f'10.254.{30+i}.0/24') for i in range(1,9)])
        return plan

    def test_current_lifecycle_eight_plan_accepts_only_fixed_complete_fresh_schedule(self):
        # The old registry-only route rejects a truthful current-source suite.
        # This is a plan-admission RED, not PG or physical-clone evidence.
        plan=self.lifecycle_plan()
        try:accepted=runner.validate_plan(plan,'source-lifecycle-eight')
        except runner.CompletionError:accepted=None
        self.assertEqual(accepted,plan,'current lifecycle-eight has no admitted root plan')
        for change in ('missing','duplicate-id','duplicate-subnet','arbitrary-test','legacy-capability','foreign-source'):
            bad=copy.deepcopy(plan)
            if change=='missing':bad['cases'].pop()
            elif change=='duplicate-id':bad['cases'][1]['case_id']=bad['cases'][0]['case_id']
            elif change=='duplicate-subnet':bad['cases'][1]['subnet']=bad['cases'][0]['subnet']
            elif change=='arbitrary-test':bad['cases'][0]['test']='source::arbitrary'
            elif change=='legacy-capability':bad['capability']='c4_completion_case_plan_v1'
            else:bad['cases'][0]['source']=copy.deepcopy(plan['source'])
            with self.subTest(change=change),self.assertRaises(runner.CompletionError):
                runner.validate_plan(bad,'source-lifecycle-eight')

    def test_current_lifecycle_result_requires_all_eight_bodies_and_exact_counts(self):
        # Missing bodies, reordered bodies and bool counts must never acquire
        # the suite's pass status; the actual fixed result validator is tested.
        plan=self.lifecycle_plan()
        result=dict(cases=[dict(test=name,kind='REAL_LIFECYCLE',outcome=dict(exit_code=0,passed=1,failed=0,ignored=0)) for name in runner.driver.PG_TESTS],
                    filesystem_gates=[dict(package=package,test=name,exit_code=0,passed=1,failed=0,ignored=0) for package,name in runner.driver.FS_TESTS])
        validator=getattr(runner,'validate_lifecycle_bodies',None)
        self.assertIsNotNone(validator,'fixed current eight-body completion gate is missing')
        validator(plan,result)
        for mutation in ('missing','reordered','boolean','failed','missing-fs'):
            bad=copy.deepcopy(result)
            if mutation=='missing':bad['cases'].pop()
            elif mutation=='reordered':bad['cases'][0],bad['cases'][1]=bad['cases'][1],bad['cases'][0]
            elif mutation=='boolean':bad['cases'][0]['outcome']['passed']=True
            elif mutation=='failed':bad['cases'][3]['outcome']['exit_code']=1
            else:bad['filesystem_gates'].pop()
            with self.subTest(mutation=mutation),self.assertRaises((runner.CompletionError,runner.driver.GateError)):
                validator(plan,bad)

    def lifecycle_envelope(self):
        plan=self.lifecycle_plan();_,_,base,_=self.envelope()
        nested=copy.deepcopy(base);nested.update(status=runner.LIFECYCLE_PASS,registered_transports_reaped=True,source_sha256_before='e'*64,source_sha256_after='e'*64)
        nested['cases']=[]
        for index,(item,name) in enumerate(zip(plan['cases'],runner.driver.PG_TESTS)):
            row=copy.deepcopy(base['cases'][0]);row.update(identity=runner.driver.completion_case_identity(item['case_id']),test=name,subnet=item['subnet'],kind='REAL_LIFECYCLE')
            sql=dict(database=row['identity']['database'],database_oid=16385+index,system_identifier=str(123456+index))
            binding=dict(format_version=1,capability='source_control_binding_v1',binding_id=item['case_id'],control_path=runner.driver.ROOT+'/control',control_dev=10+index,control_ino=20+index*10,**sql)
            pin=runner.digest(runner.canonical(binding));provision=receipt_fixture(pin,binding['control_dev'],binding['control_ino']);provision['case_id']=item['case_id']
            row.update(sql_identity=sql,issuer=dict(binding=binding,binding_sha256=pin,sql=copy.deepcopy(sql),issuer_euid=0,classification='INDEPENDENT_PRECOMPILE_BINDING',registry_provisioning=provision))
            row['compile_source']['source_binding_sha256']=pin
            row['pg_root_before']=dict(binding_sha256=pin,control_dev=binding['control_dev'],control_ino=binding['control_ino']);row['pg_root_after']=copy.deepcopy(row['pg_root_before'])
            row['compiled']={};row['artifact_provenance']={}
            for kind,suffix in (('lib','test'),('example','migrate')):
                sha=('9' if kind=='lib' else '8')*64;path='/target/retained/'+item['case_id']+'-'+suffix
                artifact='/target/build/debug/'+('deps/learning_backup-abc123' if kind=='lib' else 'examples/c4_task3_migrate')
                row['compiled'][kind]=dict(binary=path,sha256=sha,artifact=artifact,compiler_stdout_sha256='6'*64,compiler_stderr_sha256='7'*64,exit_code=0)
                materialized='/target/materialized/'+item['case_id']+'-'+kind
                metadata=dict(dev=1,ino=10,uid=0,mode=0o755,nlink=1,bytes=64)
                row['artifact_provenance'][kind]=dict(profile=dict(test=kind=='lib',opt_level='0',debuginfo=0,debug_assertions=True,overflow_checks=True),materialization=dict(source_path=artifact,source=metadata,destination_path=materialized,destination=dict(metadata,ino=11,mode=0o500),bytes=64,sha256=sha))
            kinds={'source_control':'control','local_pins':'pins','source_assets':'assets','source_staging':'asset-staging'}
            roots={kinds[r['kind']]:dict(path=r['path'],dev=r['dev'],ino=r['ino']) for r in provision['roots']}
            roots.update({name:dict(path=runner.driver.ROOT+'/'+name,dev=10+index,ino=900+index*10+offset) for offset,name in enumerate(('proofs','secrets','regressions'))})
            row['audit_before']=dict(row['pg_root_before'],roots=roots,binaries={value['binary']:value['sha256'] for value in row['compiled'].values()})
            row['audit_after']=copy.deepcopy(row['audit_before'])
            if index==7:
                for root in ('control','pins'):row['audit_after']['roots'][root]['path']+='-held'
            row['audit_final']=copy.deepcopy(row['audit_after']);nested['cases'].append(row)
        nested['filesystem_gates']=[dict(package=package,test=name,exit_code=0,passed=1,failed=0,ignored=0) for package,name in runner.driver.FS_TESTS]
        receipt=dict(format_version=1,capability='c4_completion_source_regression_receipt_v1',batch_id=plan['batch_id'],case_name='source-lifecycle-eight',plan_sha256=runner.digest(runner.canonical(plan)),status=runner.LIFECYCLE_PASS,driver_result=dict(path='driver/evidence/result.json',size=1,sha256='a'*64),raw_index=dict(path='driver/evidence/raw-index.json',size=1,sha256='b'*64))
        return plan,nested,receipt

    def check_lifecycle_envelope(self,plan,nested,receipt):
        index=dict(format_version=1,capability='c4_source_raw_index_v1',files=[],scan_bytes=0)
        with patch.object(runner,'private_read',return_value=runner.canonical(plan)),patch.object(runner,'relative_ref',side_effect=lambda root,ref,cap:runner.canonical(nested if ref['path'].endswith('/result.json') else index)),patch.object(runner.driver,'raw_log_records',return_value=[]):
            return runner._verify_lifecycle_envelope(Path('/root/case'),receipt)

    def test_lifecycle_receipt_checks_each_current_issuer_and_cleanup(self):
        plan,nested,receipt=self.lifecycle_envelope()
        self.assertEqual(self.check_lifecycle_envelope(plan,nested,receipt),receipt)
        for mutation in ('issuer-source','compile-source','binary','stopped','reaped','case-id','failed','missing-fs'):
            bad=copy.deepcopy(nested);row=bad['cases'][5]
            if mutation=='issuer-source':row['issuer']['registry_provisioning']['source_package_sha256']='f'*64
            elif mutation=='compile-source':row['compile_source']['application_commit']='f'*40
            elif mutation=='binary':row['artifact_provenance']['lib']['materialization']['sha256']='f'*64
            elif mutation=='stopped':row['stopped']=False
            elif mutation=='reaped':bad['registered_transports_reaped']=False
            elif mutation=='case-id':row['identity']['case_id']=bad['cases'][4]['identity']['case_id']
            elif mutation=='failed':row['outcome']['passed']=0
            else:bad['filesystem_gates'].pop()
            with self.subTest(mutation=mutation),self.assertRaises((runner.CompletionError,runner.driver.GateError,runner.registry.RegistryError)):self.check_lifecycle_envelope(plan,bad,receipt)

    def test_current8_source_digest_missing_wrong_type_or_changed_cannot_pass(self):
        plan,nested,receipt=self.lifecycle_envelope()
        for name in ('source_sha256_before','source_sha256_after'):
            for value in (None,False,64,'bad','f'*64):
                bad=copy.deepcopy(nested)
                if value is None:bad.pop(name)
                else:bad[name]=value
                with self.subTest(name=name,value=value),self.assertRaises(runner.CompletionError):self.check_lifecycle_envelope(plan,bad,receipt)

    def test_current8_accepts_original_or_fixed_held_paths_with_same_root_identity(self):
        plan,nested,receipt=self.lifecycle_envelope()
        self.assertEqual(self.check_lifecycle_envelope(plan,nested,receipt),receipt)
        for audit in ('audit_after','audit_final'):
            for root in ('control','pins'):
                nested['cases'][7][audit]['roots'][root]['path']=runner.driver.ROOT+'/'+root
        self.assertEqual(self.check_lifecycle_envelope(plan,nested,receipt),receipt)

    def test_current8_requires_real_issuer_sql_control_roots_and_binary_audit_joins(self):
        plan,nested,receipt=self.lifecycle_envelope()
        mutations=[]
        for key in ('audit_before','audit_after','audit_final'):
            for value in (None,{},False):mutations.append(((key,),value))
            for path,value in ((('binding_sha256',),'f'*64),(('control_dev',),True),(('control_ino',),1.0),(('roots',),{}),(('roots','control','ino'),777),(('roots','control','dev'),True),(('roots','proofs','path'),runner.driver.ROOT+'/proofs-held'),(('roots','proofs','ino'),777),(('binaries',),{})):
                mutations.append(((key,)+path,value))
            mutations += [((key,'control_dev'),15.0),((key,'roots','proofs','dev'),15.0),((key,'binaries',nested['cases'][5]['compiled']['example']['binary']),'f'*64)]
        mutations += [(('issuer',),None),(('sql_identity',),None),(('sql_identity','database'),'learning_backup_c4_task3_ffffffff-ffff-4fff-8fff-ffffffffffff'),(('sql_identity','database_oid'),True),(('issuer','issuer_euid'),False),(('issuer','sql','database_oid'),1.0),(('issuer','binding','database_oid'),True),(('pg_root_before',),None),(('pg_root_before','control_dev'),True),(('pg_root_after','control_ino'),1.0),(('compiled','example'),None),(('compiled','lib','binary'),'/target/retained/foreign-test'),(('compiled','example','exit_code'),False),(('artifact_provenance','example','materialization','sha256'),'f'*64),(('audit_after','roots','control','path'),runner.driver.ROOT+'/control-other')]
        mutations += [(('issuer',),False),(('sql_identity','database_oid'),16390.0),(('issuer','sql','database_oid'),16390.0),(('pg_root_after','control_ino'),70.0),(('artifact_provenance','example','materialization','destination_path'),'/target/materialized/foreign-example'),(('artifact_provenance','lib','materialization','source_path'),'/target/build/debug/deps/learning_backup-ffff')]
        for path,value in mutations:
            bad=copy.deepcopy(nested);obj=bad['cases'][5]
            for name in path[:-1]:obj=obj[name]
            if value is None:obj.pop(path[-1])
            else:obj[path[-1]]=value
            # Keep after/final equality for after mutations to expose actual
            # content/issuer checks rather than merely unequal dictionaries.
            if path[0]=='audit_after':bad['cases'][5]['audit_final']=copy.deepcopy(bad['cases'][5].get('audit_after'))
            with self.subTest(path=path,value=value),self.assertRaises((runner.CompletionError,runner.driver.GateError,runner.registry.RegistryError)):self.check_lifecycle_envelope(plan,bad,receipt)

    def run_with_live_boundary(self,body,*,finish=False,mutate_final=None):
        """Run the real producer/owned runner; stub host IO and Docker transport."""
        import os,tempfile
        from contextlib import ExitStack
        from types import SimpleNamespace
        observed={};writes=[];created=[]
        with tempfile.TemporaryDirectory() as temp,ExitStack() as stack:
            plan,_,_,_=self.envelope();root=Path(temp);plan['evidence_root']=root.as_posix();raw=runner.canonical(plan)
            if finish:(root/'input.json').write_bytes(raw)
            image=dict(Id=runner.BUILDER,Config=dict(Env=[]),RepoDigests=['postgres@'+runner.POSTGRES])
            original_close=os.close
            def docker(*args,**kwargs):
                if args==('volume','ls','--format','{{.Name}}'):return b''
                self.assertEqual(args[0],'create');created.append(args)
                return (format(len(created),'064x')+'\n').encode()
            def live(h,b,isolation,owned,batch,source,ident,subnet,name,images,result,*args,**kwargs):
                observed.update(result=result,owned=owned,batch=batch)
                try:body(h,b,owned,result,image)
                except BaseException as error:
                    observed['body_error']=error;raise
            def no_log_proof(*args,**kwargs):raise runner.CompletionError('NO_LIVE_LOG_PROOF')
            def write(path,raw):
                writes.append(path)
                if finish:path.write_bytes(raw)
            def read(path,cap):
                raw=Path(path).read_bytes()
                if mutate_final is not None and Path(path).name=='result.json':raw=mutate_final(raw)
                return raw
            def empty_index(batch,*args,**kwargs):
                path=batch/'evidence/raw-index.json'
                path.write_bytes(runner.canonical(dict(format_version=1,capability='c4_source_raw_index_v1',files=[],scan_bytes=0)))
                return path
            patches=[patch.object(runner,'actor_preflight'),patch.object(runner.os,'umask'),
                patch.object(runner,'private_read',side_effect=read if finish else lambda *args:raw),patch.object(runner,'validate_plan',return_value=plan),
                patch.object(runner.registry,'open_directory',return_value=91763),patch.object(runner.registry,'names',return_value=['input.json']),
                patch.object(runner.os,'close',side_effect=lambda fd:None if fd==91763 else original_close(fd)),
                patch.object(runner,'accepted_source',return_value=(dict(base_commit=plan['source']['application_commit']),{})),
                patch.object(runner,'write_record',side_effect=write),
                patch.object(runner.binding,'extract_public_source'),patch.object(runner.binding,'source_digest',return_value='same-source'),
                patch.object(runner.driver,'fresh_resource_preflight'),patch.object(runner.admission.Runner,'docker',side_effect=docker),
                patch.object(runner.admission.Runner,'inspect',side_effect=lambda kind,ref:image if kind=='image' else dict(State=dict(Status='created'))),
                patch.object(runner.os,'statvfs',return_value=SimpleNamespace(f_bavail=64*1024**3,f_frsize=1),create=True),
                patch.object(runner.driver.HelperContainer,'validate',side_effect=lambda facts:facts),
                patch.object(runner.driver,'live_case',side_effect=live),patch.object(runner.driver,'final_audits'),
                patch.object(runner.driver,'publish_raw_index',side_effect=empty_index if finish else no_log_proof)]
            if finish:patches += [patch.object(runner.binding,'sync_directory'),patch.object(runner.evidence,'private_read',side_effect=read),patch.object(runner.driver,'raw_log_records',return_value=[])]
            for entry in patches:stack.enter_context(entry)
            try:
                if finish:
                    observed['receipt']=runner.run(root/'input.json','registry-reopen')
                    observed['published_receipt']=runner.registry.parse_record((root/'case-receipt.json').read_bytes())
                    self.assertFalse((observed['batch']/'evidence/result.pending').exists())
                    return observed,created
                with self.assertRaises(runner.CompletionError):runner.run(root/'input.json','registry-reopen')
                self.assertEqual(writes,[root/'case-plan.json'],'setup alone cannot publish a pass receipt')
                self.assertIs(observed['owned'].ownership.result,observed['result'])
                return observed,created
            finally:
                if 'owned' in observed:runner.driver.LOG_RESERVATIONS.pop(str(observed['owned'].logs),None)

    def test_run_producer_first_helper_matches_real_consumer_and_limits(self):
        def body(h,b,owned,result,image):
            first=runner.driver.HelperContainer(h,b,owned,result,image,[],['/usr/bin/python3','-c','pass'],interactive=True)
            self.assertIs(result['helpers'][0],first.record)
            self.assertIs(owned.ownership.owners[first.record['name']],first.owner)
            second=runner.driver.HelperContainer(h,b,owned,result,image,[],['/usr/bin/python3','-c','pass'],interactive=True)
            with self.assertRaisesRegex(runner.driver.GateError,'CLEANUP_HELPER_COUNT'):
                runner.driver.HelperContainer(h,b,owned,result,image,[],['/usr/bin/python3','-c','pass'],interactive=True)
            for helper in (first,second):
                helper.record['removed']=True;helper.closed=True;owned.ownership.release(helper.owner,'absent')
            # Keep the real aggregate ledger: each new constructor appends its
            # own record, even after its exact owner has become absent.
            for _ in range(80):
                helper=runner.driver.HelperContainer(h,b,owned,result,image,[],['/usr/bin/python3','-c','pass'])
                helper.record['removed']=True;helper.closed=True;owned.ownership.release(helper.owner,'absent')
            self.assertEqual(len(result['helpers']),82)
            with self.assertRaisesRegex(runner.driver.GateError,'CLEANUP_HELPER_COUNT'):
                runner.driver.HelperContainer(h,b,owned,result,image,[],['/usr/bin/python3','-c','pass'])
            raise runner.CompletionError('CONTROLLED_SETUP_STOP')
        observed,created=self.run_with_live_boundary(body)
        error=observed.get('body_error')
        if not isinstance(error,runner.CompletionError):raise error
        self.assertEqual(str(error),'CONTROLLED_SETUP_STOP');self.assertEqual(len(created),82)
        self.assertEqual(observed['result']['status'],'FAILED')

    def test_run_zero_helpers_and_zero_body_cannot_pass(self):
        observed,created=self.run_with_live_boundary(lambda *args:None)
        self.assertEqual(created,[])
        self.assertEqual(observed['result']['helpers'],[])
        self.assertEqual(observed['result']['status'],'FAILED')
        self.assertEqual(observed['result']['reason'],'CASE_BODY_COUNTS')

    def envelope(self):
        issuer=receipt_fixture();case_id=issuer['case_id'];batch_id=issuer['batch_id'];name='registry-reopen'
        source=dict(archive=dict(path='/root/frozen/source.zip',size=42,sha256=issuer['source_package_sha256']),manifest=dict(path='/root/frozen/manifest.json',size=43,sha256='b'*64),application_commit=issuer['application_commit'],application_build_sha256=issuer['application_build_sha256'])
        plan=dict(format_version=1,capability='c4_completion_case_plan_v1',batch_id=batch_id,case_id=case_id,cache_scope='33333333-3333-4333-8333-333333333333',case_name=name,source=source,images=dict(builder=runner.BUILDER,postgres=runner.POSTGRES),subnet='10.254.20.0/24',evidence_root='/root/case',budget_profile='registry_task1_v1')
        binary='9'*64;status='REGISTRY_TASK1_CASE_PASSED_NOT_COMPLETE'
        case=dict(identity=runner.driver.completion_case_identity(case_id),archive_sha256=source['archive']['sha256'],test=runner.CASE_TESTS[name],stopped=True,execs_absent=True,transports_reaped=True,retained_network_empty=True,audit_after={},audit_final={},compiled=dict(lib=dict(sha256=binary)),artifact_provenance=dict(lib=dict(materialization=dict(sha256=binary))),compile_source=dict(application_commit=source['application_commit'],application_build_sha256=source['application_build_sha256'],source_binding_sha256=issuer['source_binding_sha256']),issuer=dict(registry_provisioning=copy.deepcopy(issuer)),outcome=dict(exit_code=0,passed=1,failed=0,ignored=0))
        nested=dict(status=status,current_source=True,cleanup_verified=True,batch_id=batch_id,base_commit=source['application_commit'],archive_sha256=source['archive']['sha256'],cases=[case])
        receipt=dict(format_version=1,capability='c4_completion_case_receipt_v1',batch_id=batch_id,case_id=case_id,case_name=name,plan_sha256=runner.digest(runner.canonical(plan)),registry_provisioning_sha256=runner.digest(runner.canonical(issuer)),source_archive_sha256=source['archive']['sha256'],source_manifest_sha256=source['manifest']['sha256'],application_build_sha256=source['application_build_sha256'],binary_sha256=binary,test_name=runner.CASE_TESTS[name],test_exit=0,passed=1,failed=0,ignored=0,driver_result=dict(path='driver/evidence/result.json',size=1,sha256='a'*64),raw_index=dict(path='driver/evidence/raw-index.json',size=1,sha256='b'*64),status=status)
        return plan,issuer,nested,receipt

    def check_envelope(self,plan,issuer,nested,receipt,budget=None):
        reads={'case-plan.json':runner.canonical(plan),'registry-provisioning.json':runner.canonical(issuer)}
        index=dict(format_version=1,capability='c4_source_raw_index_v1',files=[],scan_bytes=0)
        with patch.object(runner,'private_read',side_effect=lambda path,cap:reads[Path(path).name]),patch.object(runner,'relative_ref',side_effect=lambda root,ref,cap:runner.canonical(nested if ref['path'].endswith('/result.json') else index)),patch.object(runner.driver,'raw_log_records',return_value=[]):
            return runner._verify_envelope(Path('/root/case'),receipt,raw_budget=budget)

    def timed_envelope(self):
        fixture=self.envelope();case=fixture[2]['cases'][0]
        # Actual producer, including both history entries seen in the failed
        # public driver result. Timing is synthetic; no historical case credit.
        budget=runner.driver.CaseBudget(clock=lambda:1000.125,total_deadline=8200.125)
        budget.transition('LIVE');budget.transition('CLEANUP')
        case['phase_budget_history']=budget.history
        return fixture

    def test_envelope_accepts_actual_case_budget_float_history(self):
        fixture=self.timed_envelope()
        with self.assertRaisesRegex(runner.registry.RegistryError,'REGISTRY_INTEGER_REQUIRED'):
            runner.registry.parse_record(runner.binding.canonical(fixture[2]))
        self.assertEqual(self.check_envelope(*fixture)['status'],'REGISTRY_TASK1_CASE_PASSED_NOT_COMPLETE')

    def test_final_publisher_readback_and_verify_accept_case_budget_floats(self):
        nested=self.timed_envelope()[2]
        def body(h,b,owned,result,image):result.update(copy.deepcopy(nested))
        observed,_=self.run_with_live_boundary(body,finish=True)
        self.assertEqual(observed['receipt']['status'],'REGISTRY_TASK1_CASE_PASSED_NOT_COMPLETE')
        self.assertEqual(observed['published_receipt'],observed['receipt'])

    def test_final_readback_rejects_equal_valued_numeric_type_substitution(self):
        import json
        nested=self.timed_envelope()[2];nested['readback_count']=1
        def body(h,b,owned,result,image):result.update(copy.deepcopy(nested))
        def substitute(raw):
            value=json.loads(raw);value['readback_count']=1.0;return runner.canonical(value)
        observed,_=self.run_with_live_boundary(body,finish=True,mutate_final=substitute)
        self.assertEqual(observed['receipt']['status'],'UNCONFIRMED_UNUSABLE')

    def test_nested_authority_integers_never_accept_float_or_bool_equality(self):
        fixture=self.envelope()
        mutations=[('outcome',key) for key in ('exit_code','passed','failed','ignored')]
        mutations += [('issuer',key) for key in ('format_version','generation','registry_dev','registry_ino','issuer_euid')]
        mutations += [('root',key) for key in ('dev','ino')]
        mutations += [('case',key) for key in ('stopped','execs_absent','transports_reaped','retained_network_empty')]
        for target,key in mutations:
            for kind in ((float,int) if target=='case' else (float,bool)):
                plan,issuer,nested,receipt=copy.deepcopy(fixture);case=nested['cases'][0]
                obj={'outcome':case['outcome'],'issuer':case['issuer']['registry_provisioning'],'root':case['issuer']['registry_provisioning']['roots'][0],'case':case}[target]
                obj[key]=kind(obj[key])
                with self.subTest(target=target,key=key,kind=kind),self.assertRaises((runner.CompletionError,runner.registry.RegistryError)):
                    self.check_envelope(plan,issuer,nested,receipt)

    def test_nested_final_audit_join_preserves_numeric_types(self):
        plan,issuer,nested,receipt=self.timed_envelope();case=nested['cases'][0]
        case['audit_after']=dict(control_dev=1,control_ino=2)
        case['audit_final']=dict(control_dev=1.0,control_ino=2)
        with self.assertRaisesRegex(runner.CompletionError,'RESULT_NESTED_CASE'):
            self.check_envelope(plan,issuer,nested,receipt)

    def test_registry_plan_issuer_index_and_receipt_remain_integer_only(self):
        plan,issuer,nested,receipt=self.timed_envelope()
        index=dict(format_version=1,capability='c4_source_raw_index_v1',files=[],scan_bytes=0)
        records={'case-plan.json':plan,'registry-provisioning.json':issuer,'raw-index.json':index,'case-receipt.json':receipt}
        for name in records:
            changed=copy.deepcopy(records);changed[name]['format_version']=1.0
            reads={key:runner.canonical(value) for key,value in changed.items()}
            with self.subTest(record=name),patch.object(runner,'actor_preflight'),patch.object(runner,'private_read',side_effect=lambda path,cap:reads[Path(path).name]),patch.object(runner,'relative_ref',side_effect=lambda root,ref,cap:runner.canonical(nested) if ref['path'].endswith('/result.json') else reads['raw-index.json']),self.assertRaisesRegex(runner.registry.RegistryError,'REGISTRY_INTEGER_REQUIRED'):
                runner.verify(Path('/root/case/case-receipt.json'))

    def test_fixed_incomplete_case_requires_separate_matching_diagnostics(self):
        for name in ('registry-pending-staged-fault','registry-catalog-fault'):
            plan,issuer,nested,receipt=self.envelope();test=runner.CASE_TESTS[name]
            plan['case_name']=receipt['case_name']=name;receipt['test_name']=nested['cases'][0]['test']=test;receipt['plan_sha256']=runner.digest(runner.canonical(plan))
            with self.subTest(name=name),self.assertRaisesRegex(runner.CompletionError,'RESULT_REGISTRY_UNUSABLE'):
                self.check_envelope(plan,issuer,nested,receipt)

    def test_normal_case_cannot_use_incomplete_diagnostic(self):
        plan,issuer,nested,receipt=self.envelope();nested['cases'][0]['registry_unusable_after']={}
        with self.assertRaisesRegex(runner.CompletionError,'RESULT_REGISTRY_UNUSABLE'):
            self.check_envelope(plan,issuer,nested,receipt)

    def test_fault_envelope_joins_real_diagnostic_and_rejects_changed_final_or_issuer(self):
        from scripts.test_p0c4_storage_registry import FaultRegistryFixture
        for name in ('registry-pending-staged-fault','registry-catalog-fault'):
            with self.subTest(name=name),FaultRegistryFixture() as f:
                plan,_,nested,receipt=self.envelope();issuer=f.receipt;test=runner.CASE_TESTS[name];case=nested['cases'][0]
                plan['case_name']=receipt['case_name']=name;receipt['test_name']=case['test']=test;receipt['plan_sha256']=runner.digest(runner.canonical(plan))
                receipt['registry_provisioning_sha256']=runner.digest(runner.canonical(issuer));case['issuer']['registry_provisioning']=issuer;case['compile_source']['source_binding_sha256']=issuer['source_binding_sha256']
                baseline=runner.registry.capture_fault_audit_baseline(issuer,test);f.retained();f.residue(test);observed=runner.registry.audit_incomplete_publication(baseline)
                case['registry_unusable_after']=observed;case['registry_unusable_final']=copy.deepcopy(observed);case['acl_after']='t' if name=='registry-pending-staged-fault' else 'f'
                self.check_envelope(plan,issuer,nested,receipt)
                for key,value in (('reason','READY'),('issuer_sha256','0'*64),('case_id','00000000-0000-4000-8000-000000000000'),('scan_bytes',1.0)):
                    changed=copy.deepcopy(nested)
                    for where in ('registry_unusable_after','registry_unusable_final'):changed['cases'][0][where][key]=value
                    with self.subTest(key=key),self.assertRaises((runner.CompletionError,runner.registry.RegistryError)):self.check_envelope(plan,issuer,changed,receipt)
                changed=copy.deepcopy(nested);changed['cases'][0]['registry_unusable_final']['baseline_sha256']='0'*64
                with self.assertRaisesRegex(runner.CompletionError,'RESULT_REGISTRY_UNUSABLE'):self.check_envelope(plan,issuer,changed,receipt)
                changed=copy.deepcopy(nested);changed['cases'][0]['acl_after']='f' if case['acl_after']=='t' else 't'
                with self.assertRaisesRegex(runner.CompletionError,'RESULT_REGISTRY_UNUSABLE_ACL'):self.check_envelope(plan,issuer,changed,receipt)

    def test_nested_parser_refuses_noncanonical_invalid_or_nonfinite_json(self):
        invalid=[b'{"x":1,"x":1}',b'{"x":{"a":1,"a":1}}',b'{"x":NaN}',b'{"x":Infinity}',b'{"x":-Infinity}',b'{"x":1e309}',b'{"x":-1e309}',b'{"x":1.00}',b'{"x":1e0}',b'{"x":1}\n',b' {"x":1}',b'[]',b'null',b'{',b'{"x":"\xff"}',b'{"x":"\\ud800"}']
        for raw in invalid:
            with self.subTest(raw=raw),self.assertRaises(runner.CompletionError):runner.evidence.parse_driver_result(raw)
        # The driver domain admits finite values without inventing a timing-key
        # allowlist, but always enforces the existing 32 MiB file bound.
        self.assertEqual(runner.evidence.parse_driver_result(b'{"timing":[1.25,-0.5,1e-06]}'),dict(timing=[1.25,-0.5,1e-6]))
        cap=32*1024**2;raw=b'{"x":"'+b'a'*(cap-8)+b'"}'
        self.assertEqual(len(raw),cap)
        self.assertEqual(len(runner.evidence.parse_driver_result(raw)['x']),cap-8)
        with self.assertRaisesRegex(runner.CompletionError,'RESULT_DRIVER_BYTES'):runner.evidence.parse_driver_result(raw+b' ')

    def test_envelope_rejects_every_independent_source_issuer_nested_link(self):
        fixture=self.envelope();self.check_envelope(*fixture)
        mutations=[('receipt',key,'f'*64) for key in ('source_archive_sha256','source_manifest_sha256','application_build_sha256')]
        mutations += [('issuer',key,'f'*(40 if key=='application_commit' else 64)) for key in ('source_package_sha256','application_commit','application_build_sha256')]
        mutations += [('nested',key,'f'*(40 if key=='base_commit' else 64)) for key in ('base_commit','archive_sha256')]
        mutations += [('case','archive_sha256','f'*64),('identity','case_id','11111111-1111-4111-8111-111111111111'),('compile','application_commit','f'*40),('compile','application_build_sha256','f'*64),('compile','source_binding_sha256','f'*64),('case_issuer','source_package_sha256','f'*64),('materialization','sha256','f'*64),('case','test','foreign_body')]
        for target,key,value in mutations:
            plan,issuer,nested,receipt=copy.deepcopy(fixture);case=nested['cases'][0]
            destination={'receipt':receipt,'issuer':issuer,'nested':nested,'case':case,'identity':case['identity'],'compile':case['compile_source'],'case_issuer':case['issuer']['registry_provisioning'],'materialization':case['artifact_provenance']['lib']['materialization']}[target]
            destination[key]=value
            # Honest file references to changed evidence must still fail joins.
            receipt['registry_provisioning_sha256']=runner.digest(runner.canonical(issuer))
            with self.subTest(target=target,key=key),self.assertRaises(runner.CompletionError):self.check_envelope(plan,issuer,nested,receipt)

    def test_receipt_rejects_wrong_types_and_zero_test_pass(self):
        fixture=self.envelope()
        for key in runner.RECEIPT_KEYS-{'driver_result','raw_index'}:
            values=[None,True,[],{}]
            for value in values:
                plan,issuer,nested,receipt=copy.deepcopy(fixture);receipt[key]=value
                with self.subTest(key=key,value=value),self.assertRaises(runner.CompletionError):self.check_envelope(plan,issuer,nested,receipt)
        plan,issuer,nested,receipt=copy.deepcopy(fixture);receipt['passed']=0
        with self.assertRaisesRegex(runner.CompletionError,'RESULT_NO_FALSE_PASS'):self.check_envelope(plan,issuer,nested,receipt)

    def test_receipt_file_refs_validate_before_any_evidence_io(self):
        fixture=self.envelope()
        for field in ('driver_result','raw_index'):
            for value in (None,True,[],{},dict(path='../foreign',size=1,sha256='a'*64),dict(path='safe',size=True,sha256='a'*64),dict(path='safe',size=1,sha256='A'*64)):
                plan,issuer,nested,receipt=copy.deepcopy(fixture);receipt[field]=value
                with self.subTest(field=field,value=value),self.assertRaisesRegex(runner.CompletionError,'RESULT_FILEREF'):self.check_envelope(plan,issuer,nested,receipt)

    def test_fixed_eleven_exact_bodies(self):
        self.assertEqual(len(runner.CASE_TESTS), 11)
        self.assertEqual(len(set(runner.CASE_TESTS.values())),11)
        self.assertEqual(tuple(runner.CASE_TESTS.values()),runner.driver.REGISTRY_TESTS)
        self.assertEqual(set(runner.CASE_TESTS)-{'registry-'+n for n in ('reopen','root-replacement','corrupt-stale','concurrent-capture','protection-release')},{'registry-'+n+'-fault' for n in ('pending-staged','pending-visible','catalog','retained-release','abandon-ready','abandon-terminal')})
        for case, test in runner.CASE_TESTS.items():
            self.assertTrue(case.startswith('registry-'))
            self.assertTrue(test.startswith('protection_tests::real_registry_'))

    def test_no_plan_unknown_keys_or_generic_commands(self):
        for value in ({}, {'command':'echo untrusted'}, {'expected_pin':'0'*64}):
            with self.assertRaises(runner.CompletionError): runner.validate_plan(value, 'registry-reopen')

    def test_exact_test_count(self):
        name = runner.CASE_TESTS['registry-reopen']
        runner.exact_counts(0, f'test {name} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n'.encode())
        for raw in (b'', b'test result: ok. 0 passed; 0 failed; 0 ignored;', b'test result: ok. 1 passed; 0 failed; 1 ignored;', f'test {name} ... ok\ntest {name} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s'.encode()):
            with self.assertRaises(runner.CompletionError): runner.exact_counts(0, raw)

    def test_root_actor_is_required_before_runtime(self):
        from unittest.mock import patch
        with patch.object(runner.sys,'platform','win32'), self.assertRaisesRegex(runner.CompletionError,'COMPLETION_ROOT_ACTOR'): runner.actor_preflight()

    def test_same_run_raw_budget_is_forwarded_through_both_verifications(self):
        fixture=self.envelope();shared=[0];seen=[]
        def observe(logs,expected,budget):seen.append(budget);budget[0]+=4;return []
        # Keep the envelope fixture isolated from real private filesystem access.
        plan,issuer,nested,receipt=fixture
        reads={'case-plan.json':runner.canonical(plan),'registry-provisioning.json':runner.canonical(issuer)}
        index=dict(format_version=1,capability='c4_source_raw_index_v1',files=[],scan_bytes=0)
        with patch.object(runner,'private_read',side_effect=lambda path,cap:reads[Path(path).name]),patch.object(runner,'relative_ref',side_effect=lambda root,ref,cap:runner.canonical(nested if ref['path'].endswith('/result.json') else index)),patch.object(runner.driver,'raw_log_records',side_effect=observe):
            runner._verify_envelope(Path('/root/case'),receipt,raw_budget=shared)
            runner._verify_envelope(Path('/root/case'),receipt,raw_budget=shared)
            runner._verify_envelope(Path('/root/case'),receipt)
        self.assertIs(seen[0],shared);self.assertIs(seen[1],shared);self.assertIsNot(seen[2],shared);self.assertEqual(shared,[8])

    def test_raw_reader_cap_and_cap_plus_one_observe_actual_calls(self):
        import os,stat,tempfile
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as temp:
            logs=Path(temp);(logs/'0001.stdout').write_bytes(b'1234')
            metadata=SimpleNamespace(st_dev=1,st_ino=2,st_uid=0,st_mode=stat.S_IFREG|0o600,st_nlink=1,st_size=4,st_mtime_ns=1,st_ctime_ns=1)
            actual_read=os.read;calls=[]
            def read(fd,size):raw=actual_read(fd,size);calls.append((size,len(raw)));return raw
            with patch.object(runner.driver.os,'fstat',return_value=metadata),patch.object(runner.driver.os,'geteuid',return_value=0,create=True),patch.object(runner.driver.os,'read',side_effect=read),patch.object(runner.driver.os,'fsync'):
                budget=[536870908];runner.driver.raw_log_records(logs,{'0001.stdout'},budget)
                self.assertEqual(calls,[(4,4)]);self.assertEqual(budget,[536870912])
                with self.assertRaisesRegex(runner.driver.GateError,'RAW_SCAN_BYTES'):runner.driver.raw_log_records(logs,{'0001.stdout'},budget)
                self.assertEqual(calls,[(4,4)],'cap+1 may not touch actual reader')

    def test_index_serializes_all_three_publisher_scans_before_publication(self):
        from types import SimpleNamespace
        logs=Path('/root/case/logs');batch=logs.parent;owned=SimpleNamespace(logs=logs,ownership=SimpleNamespace(children=[]))
        files=[dict(path='logs/0001.stdout',size=4,sha256='a'*64,dev=1,ino=2)];seen=[];written=[];budget=[8]
        def scan(path,expected,actual_budget):
            self.assertIs(actual_budget,budget);budget[0]+=4;seen.append(budget[0]);return files
        runner.driver.LOG_RESERVATIONS[str(logs)]=dict(producer_identities={'0001.stdout':(1,2)})
        try:
            with patch.object(runner.driver,'expected_raw_logs',return_value={'0001.stdout'}),patch.object(runner.driver,'raw_log_records',side_effect=scan),patch.object(runner.driver,'new_private',side_effect=lambda path,raw:written.append(raw)),patch.object(runner.driver,'private_read',side_effect=lambda *a,**kw:(written[0],None)),patch.object(runner.driver.os,'open',return_value=10),patch.object(runner.driver.os,'close'),patch.object(runner.driver.os,'geteuid',return_value=0,create=True),patch.object(runner.driver.os,'O_DIRECTORY',0,create=True),patch.object(runner.driver.os,'O_NOFOLLOW',0,create=True),patch.object(runner.registry,'rename_no_replace'):
                runner.driver.publish_raw_index(batch,owned,dict(cases=[]),budget=budget)
            self.assertEqual(seen,[12,16,20]);self.assertEqual(runner.registry.parse_record(written[0])['scan_bytes'],12)
        finally:runner.driver.LOG_RESERVATIONS.pop(str(logs),None)

    def test_late_raw_recheck_capacity_is_refused_before_pass_publication(self):
        plan,issuer,nested,receipt=self.envelope();files=[dict(path='logs/0001.stdout',size=4,sha256='a'*64,dev=1,ino=2)]
        index=dict(format_version=1,capability='c4_source_raw_index_v1',files=files,scan_bytes=12)
        reads={'case-plan.json':runner.canonical(plan),'registry-provisioning.json':runner.canonical(issuer)}
        budget=[536870905]
        def scan(logs,expected,actual):self.assertIs(actual,budget);actual[0]+=4;return files
        with patch.object(runner,'private_read',side_effect=lambda path,cap:reads[Path(path).name]),patch.object(runner,'relative_ref',side_effect=lambda root,ref,cap:runner.canonical(nested if ref['path'].endswith('/result.json') else index)),patch.object(runner.driver,'raw_log_records',side_effect=scan):
            with self.assertRaisesRegex(runner.CompletionError,'RESULT_RAW_RECHECK_CAPACITY'):runner._verify_envelope(Path('/root/case'),receipt,raw_budget=budget,reserve_recheck=True)

    def test_fixed_identity_consumes_exact_plan_uuid(self):
        case='11111111-1111-4111-8111-111111111111'
        ident=runner.driver.completion_case_identity(case);project='kwc4c-'+case.replace('-','')
        self.assertEqual(ident['project'],project)
        for key,suffix in (('pg_name','pg-1'),('network','net'),('volume','pg'),('source_volume','source'),('build_volume','build'),('registry_volume','registry')):
            self.assertEqual(ident[key],project+'-'+suffix)
        for case in ('../x','00000000-0000-0000-0000-000000000000','11111111-1111-1111-8111-111111111111'):
            with self.assertRaises(runner.driver.GateError):runner.driver.completion_case_identity(case)

    def test_completion_resources_acquire_real_cleanup_owners_without_name_collision(self):
        import tempfile
        from types import SimpleNamespace
        ident=runner.driver.completion_case_identity('11111111-1111-4111-8111-111111111111')
        with tempfile.TemporaryDirectory() as temp:
            logs=Path(temp);state=dict(archives=0,ledgers=0,pending=0,audits=0)
            runner.driver.LOG_RESERVATIONS[str(logs)]=state
            try:
                raw=SimpleNamespace(logs=logs);result=dict(batch_id='22222222-2222-4222-8222-222222222222',helpers=[])
                ownership=runner.driver.CleanupReservations(raw,result);ownership.case_id=ident['case_id']
                labels={'com.docker.compose.project':ident['project'],'knowweave.source-lifecycle.batch':result['batch_id']}
                volumes=[]
                for key in ('volume','source_volume','build_volume','registry_volume'):
                    name=ident[key];expected=dict(name=name,labels=labels);record=dict(name=name,labels=labels)
                    owner=ownership.acquire('volume',name,expected,None,record)
                    ownership.known(owner,name);ownership.release(owner,'retained');volumes.append(owner)
                self.assertEqual(len(ownership.owners),4)
                self.assertTrue(all(owner['state']=='retained' for owner in volumes))
                # This reaches the real name-only ownership-table join used
                # immediately before compose create, with all four volumes kept.
                pg=ownership.acquire('pg',ident['pg_name'],dict(name=ident['pg_name']),dict(name=ident['network']),{})
                ownership.known(pg,'a'*64)
                self.assertEqual(len(ownership.owners),5)
                self.assertEqual({owner['intent']['kind'] for owner in ownership.owners.values()},{'volume','pg'})
                before=dict(ownership.owners)
                with self.assertRaisesRegex(runner.driver.GateError,'CLEANUP_OWNER_REUSED'):
                    ownership.acquire('volume',ident['volume'],dict(name=ident['volume']),None,{})
                self.assertEqual(ownership.owners,before)
                ownership.release(pg,'stopped')
                with self.assertRaisesRegex(runner.driver.GateError,'CLEANUP_OWNER_REUSED'):
                    ownership.acquire('pg',ident['pg_name'],dict(name=ident['pg_name']),dict(name=ident['network']),{})
                self.assertEqual(ownership.owners,before)
            finally:runner.driver.LOG_RESERVATIONS.pop(str(logs),None)

    def test_verify_rejects_previous_colliding_container_name(self):
        plan,issuer,nested,receipt=self.envelope()
        ident=nested['cases'][0]['identity'];ident['pg_name']=ident['volume']
        with self.assertRaisesRegex(runner.CompletionError,'RESULT_CASE_SOURCE_LINK'):
            self.check_envelope(plan,issuer,nested,receipt)

    def test_valid_plan_and_rejected_tampering(self):
        import copy
        plan=dict(format_version=1,capability='c4_completion_case_plan_v1',batch_id='11111111-1111-4111-8111-111111111111',case_id='22222222-2222-4222-8222-222222222222',cache_scope='33333333-3333-4333-8333-333333333333',case_name='registry-reopen',source=dict(archive=dict(path='/root/frozen/source.zip',size=42,sha256='a'*64),manifest=dict(path='/root/frozen/manifest.json',size=43,sha256='b'*64),application_commit='c'*40,application_build_sha256='a'*64),images=dict(builder=runner.BUILDER,postgres=runner.POSTGRES),subnet='10.254.20.0/24',evidence_root='/root/case',budget_profile='registry_task1_v1')
        self.assertEqual(runner.validate_plan(plan,'registry-reopen'),plan)
        for changes in (dict(format_version=True),dict(case_name='registry-root-replacement'),dict(images=dict(builder='latest',postgres=runner.POSTGRES)),dict(subnet='8.8.8.0/24'),dict(subnet='10.254.20.1/24'),dict(command='cargo test'),dict(evidence_root='/root/../case'),dict(cache_scope=plan['case_id'])):
            bad=copy.deepcopy(plan);bad.update(changes)
            with self.subTest(changes=changes),self.assertRaises(runner.CompletionError):runner.validate_plan(bad,'registry-reopen')
