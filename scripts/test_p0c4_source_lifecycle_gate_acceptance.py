"""Protocol negatives: these tests do not claim actual Linux/PG execution."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import uuid

FILE = Path(__file__).with_name('p0c4_source_lifecycle_gate_acceptance.py')
spec = importlib.util.spec_from_file_location('lifecycle_driver', FILE)
driver = importlib.util.module_from_spec(spec) if FILE.exists() else None
if driver is not None:
    spec.loader.exec_module(driver)

ATTEMPT = '11111111-1111-4111-8111-111111111111'
NONCE = '22222222-2222-4222-8222-222222222222'
CASE = '33333333-3333-4333-8333-333333333333'
PROJECT = 'learning-system-p0c4-33333333333343338333333333333333'
DB = 'learning_backup_c4_task3_' + CASE
NAME = 'source::lifecycle_tests::real_capture_all_ready_and_retained_pin'


def raw_request(**changes):
    row = dict(backup_id=ATTEMPT, compose_project=PROJECT, database=DB,
               format_version=1, operation='refresh_isolation', request_id=NONCE)
    row.update(changes)
    return json.dumps(row, sort_keys=True, separators=(',', ':')).encode()


class LifecycleProtocolTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(driver, 'lifecycle driver implementation is missing')
        self.ident = dict(case_id=CASE, project=PROJECT, database=DB)

    def reject(self, call, token):
        with self.assertRaisesRegex(driver.GateError, token):
            call()

    def test_exact_request_is_admitted(self):
        self.assertEqual(driver.parse_request(raw_request(), 'driver-request-' + NONCE + '.json', self.ident)['backup_id'], ATTEMPT)
        # Removing scope admission would let an unbounded/all schedule run for a leaf.
        self.assertTrue(hasattr(driver,'admit_schedule'),'fixed bounded scope admission missing')
        from types import SimpleNamespace
        suffixes=('real_capture_all_ready_and_retained_pin','finish_real_pin_after_sealed_rename',
                  'finish_real_pin_from_pins_durable','finish_real_release_ready_before_and_after_grant',
                  'abandon_early_and_late_attempts','abandon_crash_retry_and_terminal_ambiguity',
                  'lifecycle_release_failure_compensates_same_session','lifecycle_admission_and_held_roots')
        capture='source::lifecycle_tests::real_capture_all_ready_and_retained_pin'
        legacy='source::binding_tests::matching_bound_recovery_recloses_without_finishing'
        for scope in (*suffixes,'legacy-close'):
            primary=legacy if scope=='legacy-close' else 'source::lifecycle_tests::'+scope
            expected=((capture,True,False),) if scope==suffixes[0] else ((capture,True,False),(primary,False,scope=='legacy-close'))
            count=1 if scope in (suffixes[0],'legacy-close') else 2
            args=SimpleNamespace(scope=scope,suite_id=NONCE,subnet=['10.253.1.0/24']*count,
                                 legacy_close_subnet='10.253.2.0/24' if scope=='legacy-close' else None)
            with self.subTest(scope=scope):
                schedule=driver.admit_schedule(args)
                self.assertIsInstance(schedule,tuple);self.assertEqual(schedule,expected)
                leaf=driver.suite_leaf(args,schedule)
                self.assertEqual(leaf,dict(format_version=1,suite_id=NONCE,scope=scope,primary_test=primary,
                    executions=[dict(ordinal=i,test=test,role='primary' if i==len(expected)-1 else 'prelude',first=first,legacy=islegacy)
                                for i,(test,first,islegacy) in enumerate(expected)]))
        args=SimpleNamespace(scope='all',suite_id=None,subnet=['x']*8,legacy_close_subnet=None)
        self.assertEqual(driver.admit_schedule(args),tuple(('source::lifecycle_tests::'+s,i==0,False) for i,s in enumerate(suffixes)))
        args.legacy_close_subnet='x'
        self.assertEqual(driver.admit_schedule(args)[-1],(legacy,False,True))

    def scope_argv(self,scope=None,count=8,suite=None,legacy=False):
        args=[]
        for name in ('archive','archive-sha256','manifest-sha256','runner-sha256','batch-id','prerequisite','prerequisite-sha256'):
            args+=['--'+name,'unused']
        args+=sum((['--subnet','10.253.'+str(i)+'.0/24'] for i in range(count)),[])
        if scope is not None:args+=['--scope',scope]
        if suite is not None:args+=['--suite-id',suite]
        if legacy:args+=['--legacy-close-subnet','10.253.239.0/24']
        return args

    def scope_main_model(self,scope,count,suite,legacy=False,failure=None):
        """Real main/admission/accounting/final audits; doubles only external fixture IO."""
        from unittest.mock import patch
        import contextlib,io,types
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory);stage=base/NONCE;stage.mkdir();batch=stage/CASE
            events=[];preflights=[];finals=[];ledger_initial=[]
            class Runner:
                counter=0
                def __init__(self,logs):self.logs=logs
                def run(self,*args,**kwargs):raise AssertionError('unexpected transport')
                def docker(self,*args):return b''
                def inspect(self,kind,ref):return dict(Id=ref,RepoDigests=[ref],Config=dict(Env=[]))
            helper=types.SimpleNamespace(Runner=Runner,BUILDER='sha256:'+'a'*64,PG_IMAGE='pg@sha256:'+'b'*64,
                owned_file=lambda *args:b'fixture',admit_subnets=lambda requested,existing:requested,
                env_dict=lambda values:{})
            def create_batch(*args):
                (batch/'evidence/logs').mkdir(parents=True);return batch
            def finalize(h,batch,result):
                if failure=='status-prefix':result['status']='FOUR_FS_EIGHT_PG_UNKNOWN'
                if failure=='leaf-status-prefix':result['status']='SCOPED_LIFECYCLE_LEAF_PASSED_UNKNOWN'
            binding=types.SimpleNamespace(create_batch=create_batch,extract_public_source=lambda *args:None,
                source_digest=lambda *args:{'sha256':'a'*64},finalize_result=finalize)
            def preflight(h,runner,identities,subnets,result,case_id=None):
                state=driver.LOG_RESERVATIONS[str(runner.logs)]
                if not preflights:ledger_initial.append(state['ledgers'])
                preflights.append((len(identities),len(subnets),case_id))
                if failure=='second-capacity' and len(preflights)==3:
                    state['pending']=8192;driver.log_capacity(runner.logs)
            def live(h,b,isolation,runner,batch,source,ident,subnet,name,images,result,manifest,files,pin,first,legacy=False):
                events.append((name,first,legacy,subnet))
                record=dict(test=name,kind='LEGACY_CLOSE_ONLY_SYNTHETIC_PHASES_NO_DUMP' if legacy else 'REAL_LIFECYCLE',identity=ident,
                    stopped=True,execs_absent=True,transports_reaped=True,stage='completed',
                    outcome=dict(passed=1),audit_after={'sha256':'a'*64})
                result['cases'].append(record)
                driver.LOG_RESERVATIONS[str(runner.logs)]['ledgers']-=1
                if first:result['filesystem_gates']=[{}]*4
                if failure=='prelude' and first:raise driver.GateError('MODELED_PRELUDE_FAILED')
                if failure=='missing' and not first:result['cases'].pop()
                if failure=='wrong-test' and not first:record['test']=NAME
                if failure=='failed-body' and not first:record['outcome']['passed']=0
                if failure=='metadata':result['suite_leaf']['executions'][0]['first']=False
                if failure=='cleanup':record['transports_reaped']=False
                if failure=='final-capacity':driver.LOG_RESERVATIONS[str(runner.logs)]['pending']=8192
                if legacy:result['legacy_close_regression']=dict(executed=True)
            def final_helper(h,b,runner,result,image,mounted,command):
                finals.append(command[-1]);return 0,command[-1].encode(),b''
            argv=self.scope_argv(scope,count,suite,legacy)
            argv[argv.index('--archive')+1]=str(stage/'source.zip')
            argv[argv.index('--prerequisite')+1]=str(stage/'prerequisite.json')
            argv[argv.index('--runner-sha256')+1]=driver.digest(b'fixture')
            argv[argv.index('--batch-id')+1]=CASE
            patches=[patch.object(sys,'argv',['runner',*argv]),patch.object(sys,'platform','linux'),
                patch.object(driver,'__file__',str(stage/FILE.name)),patch.object(driver,'BASE',base),
                patch.object(driver.os,'geteuid',lambda:123,create=True),patch.object(driver.os,'umask',lambda mode:0),
                patch.object(driver.os,'statvfs',lambda path:types.SimpleNamespace(f_bavail=64*1024**3,f_frsize=1),create=True),
                patch.object(Path,'home',lambda:Path('/home/hans')),
                patch.object(Path,'lstat',lambda path:types.SimpleNamespace(st_mode=0o40700,st_uid=123)),
                patch.object(driver,'load_public_helpers',lambda *args,**kwargs:(helper,binding,None)),
                patch.object(driver,'verify_package',lambda *args:({'snapshot_kind':'fixture'},{})),
                patch.object(driver,'verify_prerequisite',lambda *args:{}),
                patch.object(driver,'fresh_resource_preflight',preflight),patch.object(driver,'live_case',live),
                patch.object(driver,'run_helper',final_helper)]
            with contextlib.ExitStack() as stack,contextlib.redirect_stdout(io.StringIO()) as out:
                for item in patches:stack.enter_context(item)
                try:code=driver.main()
                except SystemExit as error:self.fail('valid fixed leaf CLI rejected before scheduling: '+str(error.code))
            result=json.loads(out.getvalue());state=driver.LOG_RESERVATIONS.pop(str(batch/'evidence/logs'),{})
            return code,result,events,preflights,finals,ledger_initial,state

    def test_duplicate_unknown_and_noncanonical_fields_fail(self):
        good = raw_request()
        for raw in (good[:-1] + b',"operation":"refresh_isolation"}', raw_request(extra=1), good + b'\n', good.replace(b':1,', b':true,')):
            self.reject(lambda: driver.parse_request(raw, 'driver-request-' + NONCE + '.json', self.ident), 'REQUEST_')
        self.assertTrue(hasattr(driver,'parse_cli'),'scope CLI boundary missing')
        from unittest.mock import patch
        import contextlib,io
        default=driver.parse_cli(self.scope_argv());explicit=driver.parse_cli(self.scope_argv('all'))
        self.assertEqual(vars(default),vars(explicit))
        invalid=[self.scope_argv('unknown',1,NONCE),self.scope_argv(NAME,1,NONCE),
                 self.scope_argv('all',8,NONCE),self.scope_argv('all',7),
                 self.scope_argv('real_capture_all_ready_and_retained_pin',1),
                 self.scope_argv('real_capture_all_ready_and_retained_pin',2,NONCE),
                 self.scope_argv('real_capture_all_ready_and_retained_pin',1,NONCE,True),
                 self.scope_argv('finish_real_pin_from_pins_durable',1,NONCE),
                 self.scope_argv('finish_real_pin_from_pins_durable',2,NONCE,True),
                 self.scope_argv('legacy-close',1,NONCE),self.scope_argv('legacy-close',2,NONCE,True)]
        for bad in ('00000000-0000-0000-0000-000000000000','AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA','{'+NONCE+'}', 'bad'):
            invalid.append(self.scope_argv('legacy-close',1,bad,True))
        for argv in invalid:
            with self.subTest(argv=argv),contextlib.redirect_stdout(io.StringIO()) as out,patch.object(driver,'load_public_helpers',side_effect=AssertionError('resource boundary reached')):
                with patch.object(sys,'argv',['runner',*argv]):self.assertEqual(driver.main(),1)
                result=json.loads(out.getvalue());self.assertEqual(result['status'],'FAILED')
                self.assertTrue(result['reason'].startswith(('SCOPE_','SUITE_','EIGHT_SUBNETS')))
                self.assertEqual(result['cases'],[])
        for argv in (self.scope_argv('all')+['--scope','all'],self.scope_argv('legacy-close',1,NONCE,True)+['--suite-id',NONCE],
                     self.scope_argv('all')+['--scope=all'],self.scope_argv('legacy-close',1,NONCE,True)+['--suite-id='+NONCE]):
            with self.subTest(duplicate=argv),contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as error:driver.parse_cli(argv)
                self.assertEqual(error.exception.code,2)
        # Result declaration is exact typed metadata, and cannot credit a prelude as primary.
        args=driver.parse_cli(self.scope_argv('finish_real_pin_from_pins_durable',2,NONCE))
        schedule=driver.admit_schedule(args)
        cases=[dict(test=NAME,kind='REAL_LIFECYCLE',outcome=dict(passed=1)),
               dict(test='source::lifecycle_tests::finish_real_pin_from_pins_durable',kind='REAL_LIFECYCLE',outcome=dict(passed=1))]
        result=dict(cases=cases,filesystem_gates=[{}]*4,suite_leaf=driver.suite_leaf(args,schedule))
        driver.scheduled_gates(args,schedule,result)
        import copy
        for key,value in (('format_version',True),('suite_id',CASE),('scope','all'),('primary_test',NAME),('extra',1)):
            bad=copy.deepcopy(result);bad['suite_leaf'][key]=value
            self.reject(lambda:driver.scheduled_gates(args,schedule,bad),'SUITE_LEAF_METADATA')
        for key,value in (('ordinal',False),('first',False),('legacy',True),('role','primary'),('test',cases[1]['test']),('extra',1)):
            bad=copy.deepcopy(result);bad['suite_leaf']['executions'][0][key]=value
            self.reject(lambda:driver.scheduled_gates(args,schedule,bad),'SUITE_LEAF_METADATA')
        for changes in (dict(cases=cases[:1]),dict(cases=cases[::-1]),dict(cases=cases+[cases[0]]),dict(filesystem_gates=[{}]*3)):
            self.reject(lambda:driver.scheduled_gates(args,schedule,dict(result,**changes)),'GATES_INCOMPLETE')

    def test_foreign_database_project_nonce_operation_and_non_v4_fail(self):
        for changes in (dict(database=DB+'x'), dict(compose_project=PROJECT+'x'), dict(request_id=ATTEMPT),
                        dict(operation='grant'), dict(backup_id='00000000-0000-0000-0000-000000000000')):
            self.reject(lambda: driver.parse_request(raw_request(**changes), 'driver-request-' + NONCE + '.json', self.ident), 'REQUEST_')

    def test_request_budget_and_partial_publication(self):
        self.reject(lambda: driver.parse_request(b' '*4097, 'driver-request-'+NONCE+'.json', self.ident), 'REQUEST_BUDGET')
        self.assertTrue(driver.incomplete_publication(b''))
        self.assertTrue(driver.incomplete_publication(b'{"backup_id":'))
        self.assertFalse(driver.incomplete_publication(b'{"wrong":1}'))
        self.assertFalse(driver.incomplete_publication(b'not json'))

    def test_generation_replay_and_seventeenth_request_fail(self):
        ledger = driver.GenerationLedger(self.ident)
        ledger.admit(raw_request(), 'driver-request-'+NONCE+'.json')
        self.reject(lambda: ledger.admit(raw_request(), 'driver-request-'+NONCE+'.json'), 'REQUEST_REPLAY')
        for _ in range(15):
            nonce = str(uuid.uuid4())
            ledger.admit(raw_request(request_id=nonce), 'driver-request-'+nonce+'.json')
        nonce = str(uuid.uuid4())
        self.reject(lambda: ledger.admit(raw_request(request_id=nonce), 'driver-request-'+nonce+'.json'), 'REQUEST_COUNT')

    def test_delayed_denial_cannot_publish_for_next_nonce(self):
        ledger = driver.GenerationLedger(self.ident)
        ledger.admit(raw_request(), 'driver-request-'+NONCE+'.json')
        second = str(uuid.uuid4())
        ledger.admit(raw_request(request_id=second), 'driver-request-'+second+'.json')
        self.reject(lambda: ledger.require_active(NONCE), 'STALE_GENERATION')

    def test_denial_requires_actual_database_permission_failure(self):
        driver.validate_denial(2, ('psql: error: connection to server failed: FATAL:  permission denied for database "'+DB+'"\nDETAIL:  User does not have CONNECT privilege.\n').encode(), DB)
        for code, raw in ((0,b''),(2,b'password authentication failed'),(124,b'permission denied for database'),(2,b'No such file'),(2,b'permission denied for database "foreign"')):
            self.reject(lambda: driver.validate_denial(code, raw, DB), 'RUNTIME_DENIAL')

    def test_libtest_listing_ignored_wrong_count_exit_or_name_not_acceptance(self):
        output = f'running 1 test\ntest {NAME} ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 56 filtered out; finished in 0.12s\n'
        row = driver.parse_test(0, output, b'', NAME)
        self.assertEqual((row['passed'],row['failed'],row['ignored']), (1,0,0))
        for code, out, err in ((101,output,b''),(0,NAME+': test\n',b''),(0,output.replace('0 ignored','1 ignored'),b''),
                               (0,output.replace(NAME,NAME+'x'),b''),(0,output,b'warning'),(0,output+'running 0 tests\n',b'')):
            self.reject(lambda: driver.parse_test(code,out,err,NAME), 'EXACT_TEST_')

    def test_unique_compiler_artifact_and_build_finished(self):
        row = dict(reason='compiler-artifact', manifest_path='/reviewed/crates/learning-backup/Cargo.toml',
                   target=dict(name='learning_backup',kind=['lib']),profile=dict(test=True),executable='/target/build/debug/deps/learning_backup-123abc')
        lines = json.dumps(row)+'\n'+json.dumps(dict(reason='build-finished',success=True))+'\n'
        self.assertEqual(driver.discover_artifact(lines,'learning-backup','lib'),row['executable'])
        for bad in (lines+json.dumps(row),lines.replace('true','false'),lines.replace('/target/build','/other/build')):
            self.reject(lambda: driver.discover_artifact(bad,'learning-backup','lib'),'BUILD_ARTIFACT')

    def test_compile_requires_issuer_and_same_actual_pg_root(self):
        record=dict(issuer=dict(binding_sha256='a'*64,binding=dict(control_dev=10,control_ino=20)),
                    pg_root_before=dict(binding_sha256='a'*64,control_dev=10,control_ino=20))
        self.assertEqual(driver.compile_binding(record),'a'*64)
        for bad in ({},dict(record,pg_root_before=dict(binding_sha256='b'*64,control_dev=10,control_ino=20)),dict(record,pg_root_before=dict(binding_sha256='a'*64,control_dev=11,control_ino=20))):
            self.reject(lambda: driver.compile_binding(bad),'ISSUER_BEFORE_COMPILE')

    def test_anchor_bad_pid_starttime_uid_lock_or_namespace_fail(self):
        row=dict(pid=42,starttime=100,uids=[0,0,0,0],fd_dev=12,fd_ino=33,lock_dev=12,lock_ino=33,
                 flock_exit=73,pid_ns='pid:[123]',pg_pid_ns='pid:[123]')
        driver.validate_anchor(row)
        for changes in (dict(pid=0),dict(starttime=0),dict(uids=[0,1,0,0]),dict(fd_ino=34),dict(flock_exit=0),dict(pid_ns='pid:[222]')):
            self.reject(lambda: driver.validate_anchor(dict(row,**changes)), 'ANCHOR_')
        self.reject(lambda: driver.validate_anchor(dict(row,starttime=101),row),'ANCHOR_REUSED_PID')
        self.assertTrue(hasattr(driver,'parse_namespace_sampler'),'strict UID999 sampler boundary missing')
        def proc(pid,comm,start=100):
            return str(pid)+' ('+comm+') S '+' '.join(['0']*18+[str(start)]+['0']*30)
        sample=['PG_PID1_NAMESPACE_V1','55',proc(55,'sh'),'999 999 999 999',
                *(['0000000000000000']*3),'1',proc(1,'postgres'),'999 999 999 999',
                '/usr/lib/postgresql/18/bin/postgres','pid:[123]',proc(1,'postgres'),
                '999 999 999 999','/usr/lib/postgresql/18/bin/postgres']
        raw=('\n'.join(sample)+'\n').encode()
        self.assertEqual(driver.parse_namespace_sampler(raw)['pid_ns'],'pid:[123]')
        # Modeled actual failure: readlink exits zero with exactly a blank LF.
        for index,value in ((11,''),(11,'pid:[0123]'),(3,'0 0 0 0'),(4,'0000000000000001'),
                            (6,'0000000000000001'),(7,'0'),(8,proc(1,'sleep')),
                            (9,'999 0 999 999'),(10,sample[10]+' (deleted)'),(12,proc(1,'postgres',101))):
            bad=list(sample);bad[index]=value
            self.reject(lambda:driver.parse_namespace_sampler(('\n'.join(bad)+'\n').encode()),'NAMESPACE_')
        for bad in (raw[:-1],raw+b'\n',raw.replace(b'\n',b'\r\n'),raw+b'x',b'x'*4097):
            self.reject(lambda:driver.parse_namespace_sampler(bad),'NAMESPACE_')
        root=('42\n'+proc(42,'sh')+'\n0 0 0 0\n12 33\n12 33\npid:[123]\n73\n').encode()
        self.assertEqual(driver.assemble_anchor(root,driver.parse_namespace_sampler(raw)),row)
        self.reject(lambda:driver.assemble_anchor(root,dict(driver.parse_namespace_sampler(raw),pid_ns='pid:[222]')),'ANCHOR_')
        import types
        cmd=driver.namespace_command('a'*64)
        self.assertEqual(cmd[:11],[driver.DOCKER,'exec','--user','999:999','a'*64,'/usr/bin/env','-i','PATH=/usr/bin:/bin','LC_ALL=C','/bin/sh','-ec'])
        self.assertEqual(cmd[11],driver.PG_NAMESPACE_SAMPLER)
        self.assertNotIn('/proc/self',cmd[11]);self.assertNotIn('CapBnd',cmd[11])
        for bad in ('a'*63,'expected-namespace','1'):
            self.reject(lambda:driver.namespace_command(bad),'NAMESPACE_CONTAINER_ID')
        projected=dict(id='a'*64,image='sha256:'+'b'*64,running=True,status='running',pid=1234,
                       started_at='2026-10-04T00:00:00Z',restart_count=0,dead=False,oom_killed=False,error='')
        pin={k:projected[k] for k in ('id','image','pid','started_at','restart_count')}
        for key,value in (('id','c'*64),('image','sha256:'+'c'*64),('pid',1235),('pid',True),('started_at','later'),
                          ('restart_count',1),('running',False),('status','exited'),('dead',True),('oom_killed',True),('error','x')):
            self.reject(lambda:driver.validate_running(dict(projected,**{key:value}),pin),'NAMESPACE_RUNNING_PIN')
        with tempfile.TemporaryDirectory() as directory:
            logs=Path(directory);driver.LOG_RESERVATIONS[str(logs)]=dict(archives=0,ledgers=1,pending=0,audits=0)
            class FakeRunner:
                counter=0
                def __init__(self):self.logs=logs;self.error=b'';self.code=0;self.corrupt=False;self.calls=[]
                def run(self,command,**kwargs):
                    self.counter+=1;self.calls.append(command)
                    value=driver.canonical(projected)+b'\n' if command[1:3]==['container','inspect'] else (raw if '--user' in command and command[command.index('--user')+1]=='999:999' else root)
                    process=dict(exit_code=self.code,reason=None,stdout_bytes=len(value),stderr_bytes=len(self.error))
                    for suffix,data in [('stdout',value),('stderr',self.error),('process.json',driver.canonical(process))]:
                        (logs/(f'{self.counter:04d}.'+suffix)).write_bytes(data)
                    if self.corrupt:(logs/(f'{self.counter:04d}.process.json')).unlink()
                    return self.code,value,self.error
            underlying=FakeRunner();runner=driver.BudgetRunner(underlying,driver.CaseBudget())
            record=dict(identity=self.ident,generations=[]);runner.namespace=driver.NamespaceAudits(runner,record)
            ns=runner.namespace;ns.pin=pin;ns.active=NONCE
            anchor=driver.Anchor.__new__(driver.Anchor)
            anchor.runner=runner;anchor.container='a'*64;anchor.attempt=ATTEMPT;anchor.pid=42;anchor.lock='fixed'
            anchor.child=types.SimpleNamespace(check=lambda:None,p=types.SimpleNamespace(poll=lambda:None))
            for purpose in ('initial','refresh','cohort','periodic'):
                anchor.audit_pin=anchor.audit(purpose)
            self.assertEqual(underlying.counter,16);self.assertEqual(len(ns.audits),4)
            self.assertEqual(len({a['sampler']['log_prefix'] for a in ns.audits}),4)
            self.assertEqual([a['purpose'] for a in ns.audits],['initial','refresh','cohort','periodic'])
            anchor.child.p.poll=lambda:0
            self.reject(lambda:anchor.audit('periodic'),'ANCHOR_DRIVER_EXIT');self.assertEqual(underlying.counter,16)
            anchor.child.p.poll=lambda:None
            for code,stderr in ((1,b''),(0,b'permission denied')):
                underlying.code=code;underlying.error=stderr
                self.reject(lambda:runner.observed(lambda:runner.run(cmd)),'NAMESPACE_RPC_RECEIPT')
            underlying.code=0;underlying.error=b'';underlying.corrupt=True
            self.reject(lambda:runner.observed(lambda:runner.run(cmd)),'NAMESPACE_RPC_FILE')
            ns.state['audits']=512;self.reject(ns.capacity,'NAMESPACE_AUDIT_CAPACITY')
            ns.state['audits']=4;ns.audits*=64;self.reject(ns.capacity,'NAMESPACE_AUDIT_CAPACITY');ns.audits=ns.audits[:4]
            ns.state['audits']=4;ns.state['archives']=2730;self.reject(ns.capacity,'NAMESPACE_AUDIT_CAPACITY')
            del driver.LOG_RESERVATIONS[str(logs)]

    def test_runtime_exec_clears_env_and_keeps_credentials_file_only(self):
        env=driver.consumer_env(self.ident)
        cmd=driver.consumer_command('a'*64,'/target/retained/test',NAME,env)
        self.assertEqual(cmd[:8],['/usr/bin/docker','exec','--user','0:0','a'*64,'/usr/bin/env','-i','PATH=/usr/bin:/bin'])
        self.assertFalse(any('postgresql://' in value for value in cmd))
        self.reject(lambda: driver.consumer_command('a'*64,'/target/retained/test',NAME,dict(env,PGPASSWORD='secret')),'CONSUMER_ENV')
        from unittest.mock import patch
        import ast
        captures=[]
        def child(command,logs,**options):
            captures.append((command,logs,options));return object()
        with patch.object(driver,'MonitoredChild',side_effect=child):
            record={};holder,deadline,statement=driver.start_legacy_holder('a'*64,Path('owned-logs'),record)
            self.assertEqual(captures[-1][0],[driver.DOCKER,'exec','-i','--user','0:0','a'*64,
                '/usr/bin/env','-i','PATH=/usr/bin:/bin','HOME=/root','LC_ALL=C','/usr/lib/postgresql/18/bin/psql',
                '-X','-qAt','-v','ON_ERROR_STOP=1','-U','postgres','-d','postgres'])
            self.assertEqual(captures[-1][2],dict(timeout=30,interactive=True))
            self.assertIn(b'LOCK TABLE pg_catalog.pg_database IN ACCESS EXCLUSIVE MODE',statement)
            # Execute the real runtime-client constructor expression without creating PG resources.
            live=next(n for n in ast.parse(FILE.read_bytes()).body if isinstance(n,ast.FunctionDef) and n.name=='live_case')
            call=next(n.value for n in ast.walk(live) if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='runtime' for t in n.targets)
                      and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Name) and n.value.func.id=='MonitoredChild')
            eval(compile(ast.Expression(call),str(FILE),'eval'),dict(driver.__dict__,container='a'*64,ident=self.ident,batch=Path('owned-batch')))
            self.assertEqual(captures[-1][0],[driver.DOCKER,'exec','-i','--user','0:0','a'*64,
                '/usr/bin/env','-i','PATH=/usr/bin:/bin','HOME=/root','LC_ALL=C','/usr/lib/postgresql/18/bin/psql',
                '-X','-qAt','-v','ON_ERROR_STOP=1','-U','learning_runtime','-d',DB])
            self.assertEqual(captures[-1][2],dict(timeout=300,interactive=True))
        row=dict(pid=237,ppid=0,starttime=400,uid=0,exe='/usr/lib/postgresql/18/bin/psql',env_keys=['HOME','LC_ALL','PATH'])
        pins={237:dict(starttime=400,exe=row['exe'],env_keys=['HOME','LC_ALL','PATH'])}
        driver.validate_cohort([row],pins)
        self.reject(lambda:driver.validate_cohort([dict(row,env_keys=['HOME','LC_ALL','PATH','PGSYSCONFDIR'])],pins),'PROCESS_COHORT_UNAPPROVED')

    def test_source_or_binary_mutation_and_cleanup_ambiguity_fail(self):
        leaf_bodies=[]
        for scope,count,legacy,length in [('real_capture_all_ready_and_retained_pin',1,False,1),
            ('finish_real_pin_after_sealed_rename',2,False,2),('finish_real_pin_from_pins_durable',2,False,2),
            ('finish_real_release_ready_before_and_after_grant',2,False,2),('abandon_early_and_late_attempts',2,False,2),
            ('abandon_crash_retry_and_terminal_ambiguity',2,False,2),('lifecycle_release_failure_compensates_same_session',2,False,2),
            ('lifecycle_admission_and_held_roots',2,False,2),('legacy-close',1,True,2),('all',8,False,8),('all',8,True,9)]:
            with self.subTest(scope=scope,legacy=legacy):
                code,result,events,preflights,finals,ledgers,state=self.scope_main_model(scope,count,None if scope=='all' else NONCE,legacy)
                self.assertEqual(code,0,result);self.assertEqual(ledgers,[length]);self.assertEqual(len(events),length)
                self.assertEqual(len(preflights),length+1);self.assertEqual(preflights[0][:2],(length,length))
                self.assertEqual(len(finals),length);self.assertTrue(all('audit_final' in c for c in result['cases']))
                self.assertEqual([first for _,first,_,_ in events],[True]+[False]*(length-1))
                self.assertEqual(len({c['identity']['case_id'] for c in result['cases']}),length)
                self.assertEqual([subnet for _,_,_,subnet in events],['10.253.'+str(i)+'.0/24' for i in range(count)]+(['10.253.239.0/24'] if legacy else []))
                self.assertEqual(state['ledgers'],0);self.assertTrue(result['cleanup_verified'])
                if scope=='all':
                    self.assertNotIn('suite_leaf',result)
                    self.assertEqual(result['status'],'FOUR_FS_EIGHT_PG_LIFECYCLE_GATES_PASSED_NOT_COMPLETE_NOT_RESTORE')
                else:
                    self.assertEqual(result['status'],'SCOPED_LIFECYCLE_LEAF_PASSED_PARTIAL_SUITE_NOT_COMPLETE_NOT_RESTORE')
                    expected_primary='source::binding_tests::matching_bound_recovery_recloses_without_finishing' if legacy else 'source::lifecycle_tests::'+scope
                    self.assertEqual([name for name,_,_,_ in events],[NAME] if length==1 else [NAME,expected_primary])
                    leaf_bodies.extend(events)
        self.assertEqual(len(leaf_bodies),17)
        for failure in ('status-prefix','leaf-status-prefix'):
            code,result,*_=self.scope_main_model('finish_real_pin_from_pins_durable',2,NONCE,failure=failure)
            self.assertEqual(code,1,result)
        for failure in ('prelude','second-capacity','final-capacity','missing','wrong-test','failed-body','metadata','cleanup'):
            with self.subTest(failure=failure):
                code,result,events,preflights,finals,ledgers,state=self.scope_main_model('finish_real_pin_from_pins_durable',2,NONCE,failure=failure)
                self.assertEqual(code,1,result);self.assertEqual(result['status'],'FAILED')
                if failure in ('prelude','second-capacity'):
                    self.assertEqual(len(events),1);self.assertEqual(result['unstarted_namespace_ledgers_canceled'],1)
                if failure in ('second-capacity','final-capacity'):self.assertEqual(result['primary_reason'],'NAMESPACE_LOG_CAPACITY')
                if failure=='cleanup':self.assertFalse(result['cleanup_verified'])
                self.assertEqual(state['ledgers'],0)
        self.reject(lambda: driver.equal_audit({'sha256':'a'*64},{'sha256':'b'*64}),'AUDIT_CHANGED')
        self.assertFalse(driver.cleanup_verified(dict(preflight_complete=True,resource_creation_unknown=True,cases=[],helpers=[])))
        self.assertFalse(driver.cleanup_verified(dict(preflight_complete=True,cases=[dict(stopped=False)],helpers=[])))
        from unittest.mock import patch
        import types,time
        with tempfile.TemporaryDirectory() as directory:
            logs=Path(directory);state=dict(archives=0,ledgers=0,pending=0,audits=0)
            driver.LOG_RESERVATIONS[str(logs)]=state;base=types.SimpleNamespace(logs=logs,counter=0,next_log_token=None)
            registry=driver.CleanupReservations(base,dict(batch_id=CASE,helpers=[]));base.ownership=registry
            pg=registry.acquire('pg','pg',{},None,{});registry.known(pg,'a'*64)
            broker=registry.acquire('helper','broker',dict(stdin=True),None,{});registry.known(broker,'b'*64)
            helper=registry.acquire('helper','helper',dict(stdin=False),None,{})
            self.assertEqual(3*sum(len(v) for v in state['cleanup'].values()),90)
            self.reject(lambda:registry.acquire('helper','foreign',dict(stdin=False),None,{}),'CREATE_INTENT_PENDING')
            occupied=[logs/('old-'+str(i)) for i in range(8192-90)]
            with patch.object(Path,'iterdir',lambda path:iter(occupied)):
                self.reject(lambda:driver.begin_log_rpc(base),'NAMESPACE_LOG_CAPACITY')
                prefix=driver.reserved_call(base,('pg','pg-0'),lambda:driver.begin_log_rpc(base));base.counter+=1
                self.assertEqual(prefix,'0001');driver.log_capacity(logs)
                occupied.append(logs/'0001.stdout');driver.log_capacity(logs)
                driver.end_log_rpc(base,prefix);self.assertNotIn(prefix,state['work'])
                self.reject(lambda:driver.reserved_call(base,('pg','pg-0'),lambda:driver.begin_log_rpc(base)),'CLEANUP_TOKEN_REUSED')
            registry.known(helper,'c'*64)
            self.reject(lambda:registry.acquire('helper','third',dict(stdin=False),None,{}),'CLEANUP_HELPER_COUNT')
            registry.enter(types.SimpleNamespace(deadline=42));registry.enter(types.SimpleNamespace(deadline=100))
            with patch.object(driver.time,'monotonic',return_value=41):self.assertEqual(registry.left(),1)
            with patch.object(driver.time,'monotonic',return_value=43):self.reject(registry.left,'CASE_CLEANUP_TIMEOUT')
            del driver.LOG_RESERVATIONS[str(logs)]
        for kind in ('container','network','volume'):
            for mode in ('absent','generic','extra','wrong-name','bad-process'):
                with tempfile.TemporaryDirectory() as directory:
                    logs=Path(directory);driver.LOG_RESERVATIONS[str(logs)]=dict(archives=0,ledgers=0,pending=0,audits=0)
                    class RawRunner:
                        counter=0;next_log_token=None
                        def __init__(self):self.logs=logs
                        def run(self,command,**kwargs):
                            prefix=driver.begin_log_rpc(self);self.counter+=1
                            out=b'[]\n';err=driver.absence_error(kind,'fresh')
                            if mode=='generic':err=b'permission denied\n'
                            if mode=='extra':out+=b'\n'
                            if mode=='wrong-name':err=driver.absence_error(kind,'other')
                            process=dict(exit_code=1,reason=None,stdout_bytes=len(out),stderr_bytes=len(err))
                            if mode=='bad-process':process['exit_code']=True
                            for suffix,raw in [('stdout',out),('stderr',err),('process.json',driver.canonical(process))]:(logs/(prefix+'.'+suffix)).write_bytes(raw)
                            driver.end_log_rpc(self,prefix);return 1,out,err
                    base=RawRunner();registry=driver.CleanupReservations(base,dict(batch_id=CASE,helpers=[]));registry.enter()
                    owner=registry.acquire('volume','fresh',{},None,{});owner['state']='sent'
                    if mode=='absent':self.assertIsNone(registry.reconcile(owner,kind))
                    else:self.reject(lambda:registry.reconcile(owner,kind),'CLEANUP_')
                    self.assertEqual(len(list(logs.iterdir())),3);del driver.LOG_RESERVATIONS[str(logs)]
        with tempfile.TemporaryDirectory() as directory:
            logs=Path(directory);base=types.SimpleNamespace(logs=logs);registry=driver.CleanupReservations(base,dict(batch_id=CASE,helpers=[]))
            state=dict(archives=0,ledgers=0,pending=0,audits=0,ownership=registry);driver.LOG_RESERVATIONS[str(logs)]=state
            with patch.object(driver.threading.Thread,'start',side_effect=RuntimeError('constructor boundary')):
                with self.assertRaisesRegex(RuntimeError,'constructor boundary'):driver.MonitoredChild([sys.executable,'-c','import time;time.sleep(2)'],logs)
            self.assertEqual(len(registry.children),1);child=registry.children[0]
            child.retire(timeout=.3);self.assertTrue(child.retired);self.assertFalse(state['child_files'])
            self.reject(lambda:child.retire(timeout=.3),'TRANSPORT_ALREADY_RETIRED')
            with patch.object(driver.subprocess,'Popen',side_effect=OSError('no process')):
                with self.assertRaises(OSError):driver.MonitoredChild(['absent'],logs)
            self.assertEqual(len(registry.children),1);self.assertFalse(state['child_files'])
            del driver.LOG_RESERVATIONS[str(logs)]
        self.assertFalse(driver.cleanup_verified(dict(preflight_complete=True,registered_transports_reaped=False,cases=[],helpers=[])))
        # A failed synchronous helper leaves cleanup to the PG-first case path.
        helper=driver.HelperContainer.__new__(driver.HelperContainer);calls=[]
        helper.record=dict(id='a'*64);helper.runner=types.SimpleNamespace(run=lambda *a,**k:(_ for _ in ()).throw(driver.GateError('MODEL_FORWARD_FAILURE')))
        helper.close=lambda *a:calls.append('too-early')
        self.reject(helper.run,'MODEL_FORWARD_FAILURE');self.assertEqual(calls,[])
        with tempfile.TemporaryDirectory() as directory:
            logs=Path(directory);state=dict(archives=0,ledgers=0,pending=0,audits=0);driver.LOG_RESERVATIONS[str(logs)]=state
            result=dict(batch_id=CASE,helpers=[]);base=types.SimpleNamespace(logs=logs);base.ownership=driver.CleanupReservations(base,result);registry=base.ownership
            events=[]
            for name,interactive in (('broker',True),('helper',False)):
                record=dict(removed=False);owner=registry.acquire('helper',name,dict(stdin=interactive),None,record);registry.known(owner,('a' if interactive else 'b')*64)
                obj=types.SimpleNamespace(closed=False)
                def close(phase,owner=owner,obj=obj,name=name):
                    owner[phase+'_attempted']=True;events.append((name,phase))
                    if name=='broker':raise driver.GateError('MODEL_CLOSE_FAILURE')
                    obj.closed=True;owner['record']['removed']=True;registry.release(owner,'absent')
                obj.close=close;owner['object']=obj;result['helpers'].append(record)
            errors=driver.cleanup_known_helpers(base,result)
            self.assertEqual(events,[('broker','close'),('broker','fallback'),('helper','close')]);self.assertEqual(len(errors),2)
            driver.cleanup_known_helpers(base,result);self.assertEqual(len(events),3)
            del driver.LOG_RESERVATIONS[str(logs)]
        with tempfile.TemporaryDirectory() as directory:
            logs=Path(directory);driver.LOG_RESERVATIONS[str(logs)]=dict(archives=0,ledgers=0,pending=0,audits=0)
            class ReceiptedRunner:
                counter=0;next_log_token=None;fail=False;undispatched=False
                def __init__(self):self.logs=logs
                def run(self,command,**kwargs):
                    if self.undispatched:raise driver.GateError('MODEL_NOT_DISPATCHED')
                    prefix=driver.begin_log_rpc(self);self.counter+=1;code=1 if self.fail else 0
                    for suffix,raw in [('stdout',b''),('stderr',b''),('process.json',driver.canonical(dict(exit_code=code,reason=None,stdout_bytes=0,stderr_bytes=0)))]:
                        (logs/(prefix+'.'+suffix)).write_bytes(raw)
                    driver.end_log_rpc(self,prefix)
                    if self.fail:raise driver.GateError('MODEL_ACTUAL_FAILURE')
                    return 0,b'',b''
            base=ReceiptedRunner();base.ownership=driver.CleanupReservations(base,dict(batch_id=CASE,helpers=[]))
            owner=base.ownership.acquire('helper','owned',dict(stdin=False),None,{});base.ownership.known(owner,'a'*64)
            clean=driver.CleanupRunner(base,owner,'close');clean.run(['model'])
            base.fail=True;self.reject(lambda:clean.run(['model']),'MODEL_ACTUAL_FAILURE')
            self.assertEqual([r['log_prefix'] for r in owner['record']['cleanup_rpcs']['close']],['0001','0002'])
            base.undispatched=True;self.reject(lambda:clean.run(['model']),'MODEL_NOT_DISPATCHED')
            self.assertEqual(len(owner['record']['cleanup_rpcs']['close']),2)
            del driver.LOG_RESERVATIONS[str(logs)]

    def test_first_resource_collision_and_subnet_overlap_fail(self):
        self.reject(lambda: driver.admit_resources(['new'],['new']), 'RESOURCE_ALREADY_EXISTS')
        helper=driver.load_public_helpers(Path(__file__).parent)[0]
        self.reject(lambda: helper.admit_subnets(['172.29.1.0/24'],['172.29.0.0/16']),'SUBNET_OCCUPIED')
        self.assertTrue(hasattr(driver,'fresh_resource_preflight'),'one-invocation IPAM adapter missing')
        from unittest.mock import patch
        import types
        ident=dict(self.ident,pg_name='fresh-pg',network='fresh-net',volume='fresh-volume')
        class Model:
            def __init__(self,logs,n,mode=''):
                self.logs,self.counter,self.mode=logs,0,mode;self.calls=[];self.lists=0
                self.rows={f'{i+1:064x}':dict(name=f'old-{i}',project='retained') for i in range(n)}
            def run(self,cmd,**kwargs):
                self.counter+=1;self.calls.append(cmd);code=0;err=b''
                if cmd[0].endswith('/ip'):out=b'[{"dst":"10.0.0.0/8"}]' if self.mode=='route' else b'[]'
                elif tuple(cmd[1:])==driver.NETWORK_ID_LIST:
                    self.lists+=1;rows=self.rows.copy()
                    if self.mode=='post' and self.lists==2:rows['f'*64]=dict(name='added',project='')
                    if self.lists==2 and self.mode=='post-delete':rows={}
                    if self.lists==2 and self.mode=='post-replace':rows={'f'*64:next(iter(rows.values()))}
                    if self.lists==2 and self.mode=='post-project':rows={k:dict(v,project='changed') for k,v in rows.items()}
                    if self.lists==2 and self.mode=='post-name':rows={k:dict(v,name='changed') for k,v in rows.items()}
                    out=''.join(k+'\t'+v['name']+'\t'+v['project']+'\n' for k,v in rows.items()).encode()
                elif tuple(cmd[1:])==driver.NETWORK_NAME_LIST:
                    out=''.join(v['name']+'\t'+v['project']+'\n' for v in self.rows.values()).encode()
                    if self.mode=='helper':out+=b'extra\t\n'
                elif cmd[1:3]==['network','inspect']:
                    self.assert_ids=cmd[5:]
                    values=[driver.canonical(None if self.mode=='null' else [dict(Subnet='172.16.'+str(int(k,16)%256)+'.0/24')])+b'\n' for k in cmd[5:]]
                    out=b''.join(values)
                    if self.mode=='missing':out=b''.join(values[:-1])
                    if self.mode=='extra':out+=b'null\n'
                    if self.mode=='blank':out=b'\n'
                    if self.mode=='truncated':out=out[:-1]
                    if self.mode=='invalid':out=b'{}\n'
                    if self.mode=='duplicate-json':out=b'[{"Subnet":"172.16.0.0/24","Subnet":"172.16.1.0/24"}]\n'
                    if self.mode=='exit':code=1
                    if self.mode=='stderr':err=b'error'
                    if self.mode=='oversize':out=b'x'*(driver.LIMIT+1)
                else:out=b'fresh-pg\t\n' if self.mode=='collision' and cmd[1]=='ps' else b''
                receipt=dict(exit_code=code,reason='PROCESS_TIMEOUT' if self.mode=='timeout' else None,stdout_bytes=len(out),stderr_bytes=len(err))
                for suffix,data in [('stdout',out),('stderr',err),('process.json',driver.canonical(receipt))]:
                    (self.logs/(f'{self.counter:04d}.'+suffix)).write_bytes(data)
                return code,out,err
            def docker(self,*args,**kwargs):return self.run([driver.DOCKER,*args],**kwargs)[1]
        with patch.object(helper,'Path',lambda p:types.SimpleNamespace(is_file=lambda:True)):
            for n in (0,1,16,17,162):
                with tempfile.TemporaryDirectory() as directory:
                    model=Model(Path(directory),n);result={}
                    self.assertEqual(driver.fresh_resource_preflight(helper,model,[ident],['10.253.227.0/24'],result),['10.253.227.0/24'])
                    self.assertEqual(model.counter,(n+15)//16+6)
                    self.assertEqual(len(result['resource_preflights'][0]['batches']),(n+15)//16)
                    batches=[c[5:] for c in model.calls if c[1:3]==['network','inspect']]
                    self.assertEqual([k for chunk in batches for k in chunk],sorted(model.rows))
                    self.assertTrue(all(len(chunk)<=16 for chunk in batches))
                    # Same runner, new invocation: all real lists/batches execute again.
                    driver.fresh_resource_preflight(helper,model,[ident],['10.253.227.0/24'],result,CASE)
                    self.assertEqual(model.counter,2*((n+15)//16+6))
            for mode in ('null','route','collision','post','post-delete','post-replace','post-project','post-name','helper','missing','extra','blank','truncated','invalid','duplicate-json','exit','stderr','oversize','timeout'):
                with tempfile.TemporaryDirectory() as directory:
                    model=Model(Path(directory),1,mode);result={}
                    if mode=='null':driver.fresh_resource_preflight(helper,model,[ident],['10.253.227.0/24'],result)
                    else:
                        with self.assertRaises(driver.GateError):driver.fresh_resource_preflight(helper,model,[ident],['10.253.227.0/24'],result)
                        self.assertEqual(result['resource_preflights'],[])
            with tempfile.TemporaryDirectory() as directory:
                model=Model(Path(directory),2);adapter=driver.ResourcePreflightAdapter(helper,model,0,None)
                adapter.docker(*driver.NETWORK_NAME_LIST)
                self.assertNotEqual(adapter.docker('network','inspect','--format',driver.IPAM_FORMAT,'old-1'),adapter.docker('network','inspect','--format',driver.IPAM_FORMAT,'old-0'))
                for call in (lambda:adapter.docker('network','inspect','--format',driver.IPAM_FORMAT,'old-0'),
                             lambda:adapter.docker('network','inspect','--format','other','old-0'),
                             lambda:adapter.docker('network','inspect','--format',driver.IPAM_FORMAT,'foreign')):
                    self.reject(call,'PREFLIGHT_IPAM_LOOKUP')
                adapter.close();self.reject(lambda:adapter.docker(*driver.NETWORK_NAME_LIST),'PREFLIGHT_ADAPTER_REUSE')
            with tempfile.TemporaryDirectory() as directory:
                model=Model(Path(directory),17);driver.LOG_RESERVATIONS[str(model.logs)]=dict(archives=2730,ledgers=0,pending=0,audits=0)
                try:self.reject(lambda:driver.fresh_resource_preflight(helper,model,[ident],['10.253.227.0/24'],{}),'NAMESPACE_LOG_CAPACITY')
                finally:del driver.LOG_RESERVATIONS[str(model.logs)]
        for raw in (b'a\tn\tp\n',( ('a'*64)+'\tn\tp\n'+('a'*64)+'\tm\tp\n').encode(),
                    (('a'*64)+'\tn\tp\n'+('b'*64)+'\tn\tp\n').encode(),b'\n',b'no-final-lf'):
            self.reject(lambda:driver.network_list_rows(raw),'PREFLIGHT_')

    def test_bounded_child_timeout_and_output_limit_are_failures(self):
        with tempfile.TemporaryDirectory() as directory:
            child=driver.MonitoredChild([sys.executable,'-c','import time; time.sleep(3)'],Path(directory),timeout=0.1,limit=512)
            self.reject(lambda: child.finish(),'PROCESS_TIMEOUT')
            child=driver.MonitoredChild([sys.executable,'-c','print("x"*2048)'],Path(directory),timeout=3,limit=512)
            self.reject(lambda: child.finish(),'PROCESS_LOG_BUDGET')

    def test_payload_runner_rawbytes_path_and_product_pins_rejected(self):
        # Real ZIP fixtures use public package schema, no Docker mock acceptance.
        helper, binding, _=driver.load_public_helpers(Path(__file__).parent)
        import io, zipfile, stat, hashlib
        paths={'Cargo.toml':b'[workspace]', 'Cargo.lock':b'#lock',
               'crates/learning-backup/src/source.rs':b'x',
               'crates/learning-backup/src/source/admission_tests.rs':b'x',
               'scripts/p0c4_source_admission_gate_acceptance.py':Path(__file__).with_name('p0c4_source_admission_gate_acceptance.py').read_bytes(),
               'scripts/test_p0c4_source_admission_gate_acceptance.py':b'x',
               driver.ENTRY:FILE.read_bytes(), driver.TEST_ENTRY:Path(__file__).read_bytes(),
               driver.BINDING_ENTRY:Path(__file__).with_name('p0c4_source_binding_gate_acceptance.py').read_bytes(),
               driver.ISOLATION_ENTRY:Path(__file__).with_name('p0c4_source_isolation.py').read_bytes()}
        manifest=dict(schema=1,base_commit=driver.BASE_COMMIT,snapshot_kind='working-tree-green',files=[dict(path=p,bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest()) for p,raw in sorted(paths.items())])
        m=json.dumps(manifest,sort_keys=True,separators=(',',':')).encode()
        stream=io.BytesIO()
        with zipfile.ZipFile(stream,'w') as archive:
            for p,raw in dict(paths,**{driver.MANIFEST:m}).items():
                item=zipfile.ZipInfo(p);item.external_attr=(stat.S_IFREG|0o444)<<16;archive.writestr(item,raw)
        raw=stream.getvalue()
        # Correct archive hashes cannot excuse absent frozen lifecycle product files.
        self.reject(lambda: driver.verify_package(helper,raw,hashlib.sha256(raw).hexdigest(),hashlib.sha256(m).hexdigest(),hashlib.sha256(paths[driver.ENTRY]).hexdigest()),'PRODUCT_PIN')
        self.reject(lambda: driver.verify_package(helper,raw,'0'*64,hashlib.sha256(m).hexdigest(),'1'*64),'ARCHIVE_DIGEST')

    def test_actual_pg_root_audit_rejects_replaced_or_badmode_binding(self):
        good='10 20 0 700\n0 600 1 42\n'+'a'*64+'\n'
        self.assertTrue(hasattr(driver,'parse_pg_root'),'actual PG audit parser missing')
        self.assertEqual(driver.parse_pg_root(good),dict(control_dev=10,control_ino=20,binding_sha256='a'*64))
        for bad in (good.replace('0 700','1 700'),good.replace('600 1','644 1'),good.replace('600 1','600 2')):
            self.reject(lambda:driver.parse_pg_root(bad),'PG_ROOT_AUDIT')

    def test_cohort_unknown_root_process_wrong_env_or_pid_reuse_fail(self):
        self.assertTrue(hasattr(driver,'validate_cohort'),'actual process cohort parser missing')
        rows=[dict(pid=1,starttime=1,uid=999,exe='/usr/lib/postgresql/18/bin/postgres',ppid=0,env_keys=[]),
              dict(pid=22,starttime=20,uid=0,exe='/target/retained/test',ppid=0,env_keys=sorted(driver.consumer_env(self.ident)))]
        pins={22:dict(starttime=20,exe='/target/retained/test',env_keys=sorted(driver.consumer_env(self.ident)))}
        driver.validate_cohort(rows,pins)
        for changed in (dict(rows[1],starttime=21),dict(rows[1],env_keys=['PGPASSWORD']),dict(rows[1],exe='/bin/bash')):
            self.reject(lambda:driver.validate_cohort([rows[0],changed],pins),'PROCESS_COHORT')
        self.reject(lambda:driver.validate_cohort(rows+[dict(pid=99,starttime=90,uid=0,exe='/bin/sh',ppid=0,env_keys=[])],pins),'PROCESS_COHORT')

    def test_independent_index_compares_exact_field_order_full_db_rows(self):
        self.assertTrue(hasattr(driver,'validate_index'),'independent index verifier missing')
        rows=[dict(space_id=CASE,id=ATTEMPT,sha256='a'*64,byte_size=27,storage_key='aa/'+('a'*64))]
        good=json.dumps(dict(format_version=1,assets=rows),separators=(',',':')).encode()
        driver.validate_index(good,rows,fixture_counts=False)
        self.reject(lambda:driver.validate_index(json.dumps(dict(format_version=1,assets=rows),sort_keys=True,separators=(',',':')).encode(),rows,fixture_counts=False),'INDEX_CANONICAL')
        self.reject(lambda:driver.validate_index(good,[dict(rows[0],byte_size=28)],fixture_counts=False),'INDEX_DB_ROWS')


        # Pure query-boundary model of the observed PG18 output-alias ordering bug.
        # Real migration_rows/sql/pg_exec run; no PostgreSQL or migrator executes.
        files={'migrations/0010_ten.sql':b'SELECT 10;\n',
               'migrations/0002_two.sql':b'SELECT 2;\n',
               'migrations/0001_one.sql':b'SELECT 1;\n'}
        checksums={1:'24454095d8d7876ca8e842f34296ca645c83e0f7b50f048c99db4b2ff4da659267afc4ef3e994f3d74e18c741165d931',
                   2:'1f7e22c7c0ea23f18eaa40e47945a71f2cb479fba2257126cc756c7c3df72785d5106cb6eaf17f3144abe02570d1d7a7',
                   10:'5020296d9324bb5dba750ea1dc3e5cc40b650b4db70e3b95b955973c1fa5b4ae822e7463b4702531230534ed53aaf860'}
        expected=[dict(version=v,checksum_hex=checksums[v]) for v in (1,2,10)]
        facts=[(v,checksums[v],'true') for v in (10,2,1)]
        case=self
        class MigrationRunner:
            def __init__(self,rows,returned_order=None):self.rows,self.returned_order=rows,returned_order
            def run(self,args,**options):
                case.assertEqual(args,[driver.DOCKER,'exec','-i','--user','0:0','a'*64,
                    '/usr/bin/env','-i','PATH=/usr/bin:/bin','HOME=/root','LC_ALL=C',
                    'psql','-X','-qAt','-v','ON_ERROR_STOP=1','-U','learning_admin','-d',DB])
                case.assertEqual(set(options),{'stdin','timeout','allowed'})
                case.assertEqual((options['timeout'],options['allowed']),(5,(0,)))
                import re
                query=re.fullmatch(r"SELECT version::text,encode\(checksum,'hex'\),success::text FROM public\._sqlx_migrations( AS migrations)? ORDER BY (version|migrations\.version);",options['stdin'].decode())
                case.assertIsNotNone(query,'unexpected migration query outside the modeled ordering boundary')
                # Unqualified output label wins and is text; qualified input is numeric.
                numeric=query[2]=='migrations.version'
                if numeric:case.assertEqual(query[1],' AS migrations')
                ordered=sorted(self.rows,key=lambda row:row[0] if numeric else str(row[0]))
                if self.returned_order is not None:ordered=self.returned_order
                return 0,('\n'.join(f'{v}|{checksum}|{success}' for v,checksum,success in ordered)+'\n').encode(),b''

        with self.subTest(migrations='numeric_input_order'):
            try:observed=driver.migration_rows(MigrationRunner(facts),'a'*64,self.ident,files)
            except driver.GateError as error:self.fail('valid numeric migration sequence rejected: '+str(error))
            self.assertEqual(observed,expected)
        negatives=[('corrupt_checksum',[(1,'0'*96,'true'),facts[1],facts[0]],None,'GENUINE_MIGRATOR_CHECKSUMS'),
                   ('missing_version',facts[:2],None,'GENUINE_MIGRATOR_CHECKSUMS'),
                   ('extra_version',facts+[(11,'f'*96,'true')],None,'GENUINE_MIGRATOR_CHECKSUMS'),
                   ('duplicate_version',facts+[facts[1]],None,'GENUINE_MIGRATOR_CHECKSUMS'),
                   ('unsuccessful_migration',[(1,checksums[1],'false'),facts[1],facts[0]],None,'MIGRATION_NOT_SUCCESSFUL'),
                   ('reordered_output',facts,[facts[2],facts[0],facts[1]],'GENUINE_MIGRATOR_CHECKSUMS')]
        for name,rows,returned_order,token in negatives:
            with self.subTest(migrations=name):
                self.reject(lambda:driver.migration_rows(MigrationRunner(rows,returned_order),'a'*64,self.ident,files),token)

    def test_cleanup_cannot_infer_absent_exec_from_killed_cli(self):
        self.assertFalse(driver.cleanup_verified(dict(preflight_complete=True,cases=[dict(stopped=True)],helpers=[])))
        self.assertTrue(driver.cleanup_verified(dict(preflight_complete=True,cases=[dict(stopped=True,execs_absent=True,transports_reaped=True)],helpers=[dict(removed=True)])))
        import types
        from unittest.mock import patch
        # Execute the actual helper close and the actual nested PG-stop code,
        # with only Docker and already-validated container facts modeled.
        pg_stop=next(code for code in driver.live_case.__code__.co_consts if isinstance(code,types.CodeType) and code.co_name=='stop_pg')
        for kind,grace,transport in (('helper','5',10),('pg','10',15)):
            for remaining in (180,4):
                with self.subTest(kind=kind,remaining=remaining),tempfile.TemporaryDirectory() as directory:
                    identity=('a' if kind=='helper' else 'b')*64;calls=[];outputs=[];releases=[]
                    facts=dict(Id=identity,State=dict(Running=False,Pid=0),ExecIDs=[])
                    record=dict(id=identity,removed=False)
                    owner=dict(intent=dict(kind=kind,name='exact-owned'),record=record,state='known')
                    registry=types.SimpleNamespace(deadline=180,left=lambda:remaining,release=lambda o,state:releases.append(state))
                    class Runner:
                        counter=0;next_log_token=None;logs=Path(directory);ownership=registry
                        def run(self,args,**kwargs):
                            self.counter+=1;calls.append((args,kwargs))
                            if args[1]=='stop':
                                warning=b'Flag --time has been deprecated, use --timeout instead\n' if '--time' in args else b''
                                out=warning+(identity+'\n').encode();outputs.append(out)
                            elif args[1:3]==['container','inspect']:out=driver.canonical([facts])
                            elif args[1]=='rm':out=(identity+'\n').encode()
                            else:out=b''
                            process=dict(exit_code=0,reason=None,stdout_bytes=len(out),stderr_bytes=0)
                            for suffix,raw in (('stdout',out),('stderr',b''),('process.json',driver.canonical(process))):
                                (self.logs/(f'{self.counter:04d}.'+suffix)).write_bytes(raw)
                            return 0,out,b''
                    runner=Runner()
                    if kind=='helper':
                        helper=driver.HelperContainer.__new__(driver.HelperContainer)
                        helper.closed=False;helper.child=None;helper.owner=owner;helper.record=record;helper.runner=runner
                        helper.inspect=lambda clean:clean.inspect('container',identity)
                        helper.close();self.assertTrue(helper.closed);self.assertEqual(releases,['absent'])
                    else:
                        bindings=dict(expected=dict(id=identity),pg_owner=owner,registry=registry,record=record,runner=runner,
                                      h=types.SimpleNamespace(validate_container=lambda actual,expected:None))
                        self.assertEqual(set(pg_stop.co_freevars),set(bindings))
                        def cell(value):return (lambda:value).__closure__[0]
                        stop=types.FunctionType(pg_stop,driver.__dict__,closure=tuple(cell(bindings[name]) for name in pg_stop.co_freevars))
                        with patch.object(driver,'pg_facts',return_value=(facts,dict(Containers={}))):stop()
                        self.assertTrue(record['stopped']);self.assertTrue(record['execs_absent']);self.assertEqual(releases,['stopped'])
                    stops=[(args,options) for args,options in calls if args[1]=='stop']
                    self.assertEqual(stops,[([driver.DOCKER,'stop','--timeout',grace,identity],dict(timeout=min(transport,remaining)))])
                    self.assertEqual(outputs,[(identity+'\n').encode()])

    def test_process_parser_uses_tail_after_parentheses_and_actual_env_keys(self):
        self.assertTrue(hasattr(driver,'parse_proc'))
        raw='SCANNER 9\nROW\t0\t/target/retained/test\t22 (name with ) paren) S 1 '+'0 '*17+'456 0 0\tHOME,LC_ALL,PATH,\n'
        parsed=driver.parse_proc(raw)
        self.assertEqual(parsed[0]['pid'],22)
        self.assertEqual(parsed[0]['starttime'],456)
        self.assertEqual(parsed[0]['env_keys'],['HOME','LC_ALL','PATH'])

    def test_source_inventory_preserves_negative_fs_specials_but_refuses_live_specials(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);fs=root/'fs-33333333-3333-4333-8333-333333333333';fs.mkdir()
            import os
            (fs/'original').write_bytes(b'retained negative evidence')
            os.link(fs/'original',fs/'hardlink')
            try:inventory=driver.bounded_inventory(root)
            except driver.GateError as error:self.fail('intentional FS hardlink evidence was rejected: '+str(error))
            self.assertEqual(inventory['fs-'+CASE+'/hardlink']['links'],2)

    def test_negative_fs_symlink_is_metadata_only_and_live_link_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);fs=root/('fs-'+CASE);fs.mkdir()
            import os
            if hasattr(os,'symlink'):
                try:os.symlink('missing',fs/'link')
                except OSError:self.skipTest('Windows symlink privilege unavailable; Linux fixture executes separately')
                self.assertEqual(driver.bounded_inventory(root)['fs-'+CASE+'/link']['type'],'symlink')
                (root/'control').mkdir();os.symlink('missing',root/'control/link')
                self.reject(lambda:driver.bounded_inventory(root),'INVENTORY_SPECIAL')

    def test_prerequisite_raw_pin_and_execution_claims_are_separate(self):
        row=dict(status='LINUX_PREREQUISITES_PASSED_NOT_PG',exit_code=0,docker_attach_exit=0,builder_removed=True,
                 before_source_sha256='a'*64,after_source_sha256='a'*64,tests_pg_executed=False,fs_ignored_executed=False)
        raw=json.dumps(row,separators=(',',':')).encode()
        import hashlib
        driver.verify_prerequisite(raw,hashlib.sha256(raw).hexdigest())
        bad=json.dumps(dict(row,tests_pg_executed=True)).encode()
        self.reject(lambda:driver.verify_prerequisite(bad,hashlib.sha256(bad).hexdigest()),'PREREQUISITE_RECEIPT')
        self.reject(lambda:driver.verify_prerequisite(raw,'0'*64),'PREREQUISITE_DIGEST')

    def test_real_pg_dump_frozen_clear_environment_and_parent_are_required(self):
        self.assertTrue(hasattr(driver,'case_source_rpc'),'per-case actual source operation capture missing')
        # Actual observed_rpc reads four distinct same-prefix process triplets;
        # only the command execution boundary is modeled, with zero added calls.
        with tempfile.TemporaryDirectory() as directory:
            class SourceRunner:
                def __init__(self):self.logs=Path(directory);self.counter=0;self.calls=[];self.next=b''
                def run(self,args,**kwargs):
                    self.counter+=1;self.calls.append((args,kwargs));out=self.next
                    process=dict(exit_code=0,reason=None,stdout_bytes=len(out),stderr_bytes=0)
                    for suffix,raw in (('stdout',out),('stderr',b''),('process.json',driver.canonical(process))):
                        (self.logs/(f'{self.counter:04d}.'+suffix)).write_bytes(raw)
                    return 0,out,b''
            base=SourceRunner();runner=driver.BudgetRunner(base,driver.CaseBudget())
            record=dict(identity=self.ident,container_id='a'*64)
            tools=b'0 755 1\npg_dump (PostgreSQL) 18.6\n'+b'a'*64+b'  /usr/lib/postgresql/18/bin/pg_dump\n0 755 1\npg_restore (PostgreSQL) 18.6\n'+b'b'*64+b'  /usr/lib/postgresql/18/bin/pg_restore\n'
            base.next=tools;before=driver.tool_check(runner,'a'*64,record=record,operation='tools_before')
            base.next=b'C4_TASK3_MIGRATED\n'
            actual=driver.case_source_rpc(runner,record,'a'*64,'migration_setup',lambda:driver.pg_exec(runner,'a'*64,['model-migrator'],timeout=90))
            self.assertEqual(actual,(0,b'C4_TASK3_MIGRATED\n',b''))
            files={'migrations/1_model.sql':b'model migration'}
            import hashlib
            checksum=hashlib.sha384(files['migrations/1_model.sql']).hexdigest()
            base.next=('1|'+checksum+'|true\n').encode()
            self.assertEqual(driver.migration_rows(runner,'a'*64,self.ident,files,record=record),[dict(version=1,checksum_hex=checksum)])
            base.next=tools;after=driver.tool_check(runner,'a'*64,record=record,operation='tools_after')
            self.assertEqual(before,after);self.assertEqual(base.counter,4)
            refs=record['source_operation_rpcs'];self.assertEqual(set(refs),{'case_id','container_id','tools_before','migration_setup','migration_readback','tools_after'})
            self.assertEqual((refs['case_id'],refs['container_id']),(CASE,'a'*64))
            for number,name in enumerate(('tools_before','migration_setup','migration_readback','tools_after'),1):
                rpc=refs[name];self.assertEqual(set(rpc),{'log_prefix','stdout_sha256','stderr_sha256','process_sha256','started_monotonic_ns','finished_monotonic_ns'})
                self.assertEqual(rpc['log_prefix'],f'{number:04d}');self.assertLessEqual(rpc['started_monotonic_ns'],rpc['finished_monotonic_ns'])
                self.assertEqual(rpc['stdout_sha256'],driver.digest((base.logs/(rpc['log_prefix']+'.stdout')).read_bytes()))
            self.reject(lambda:driver.case_source_rpc(runner,record,'b'*64,'tools_after',lambda:runner.run(['forbidden'])),'SOURCE_OPERATION_IDENTITY')
            self.reject(lambda:driver.case_source_rpc(runner,record,'a'*64,'tools_after',lambda:runner.run(['forbidden'])),'SOURCE_OPERATION_REUSE')
            self.assertEqual(base.counter,4)
        self.assertTrue(hasattr(driver,'validate_dump_process'),'frozen real pg_dump cohort validator missing')
        row=dict(pid=88,ppid=22,starttime=400,uid=0,exe='/usr/lib/postgresql/18/bin/pg_dump',
                 env_keys=['LC_ALL','PGPASSFILE','PGAPPNAME','PGCONNECT_TIMEOUT'])
        driver.validate_dump_process(row,22)
        for changes in (dict(ppid=23),dict(env_keys=['PATH','PGPASSWORD']),dict(uid=999),dict(exe='/tmp/pg_dump')):
            self.reject(lambda:driver.validate_dump_process(dict(row,**changes),22),'PGDUMP_COHORT')

        # Pure runner boundary model: capabilities match the fixed minimal image.
        # This exercises tool_check and pg_exec, but is not a real shell/PG run.
        required={'/bin/sh','/usr/bin/env','/usr/bin/flock','/usr/bin/stat',
                  '/usr/bin/readlink','/usr/bin/sleep','/usr/bin/cat','/usr/bin/sha256sum',
                  '/usr/bin/awk','/usr/bin/tr','/usr/bin/sed','/usr/bin/sort','/usr/bin/cut','/usr/lib/postgresql/18/bin/psql'}
        facts=['0 755 1','pg_dump (PostgreSQL) 18.6','a'*64+'  /usr/lib/postgresql/18/bin/pg_dump',
               '0 755 1','pg_restore (PostgreSQL) 18.6','b'*64+'  /usr/lib/postgresql/18/bin/pg_restore']
        case=self
        class CapabilityRunner:
            def __init__(self,available,output):self.available,self.output=set(available),output
            def run(self,args,**options):
                case.assertEqual(args[:-1],[driver.DOCKER,'exec','--user','0:0','a'*64,
                    '/usr/bin/env','-i','PATH=/usr/bin:/bin','HOME=/root','LC_ALL=C','/bin/sh','-ec'])
                case.assertEqual(options,dict(stdin=None,timeout=5,allowed=(0,)))
                import re
                check=re.fullmatch(r'for t in (.+); do test -x "\$t"; done',args[-1].splitlines()[1])
                case.assertIsNotNone(check,'executable prerequisite loop must remain fail-closed')
                checked=set(check[1].split())
                # Every actually required executable must retain a prerequisite.
                case.assertTrue(required <= checked)
                missing=checked-self.available
                if missing:raise driver.GateError('PROCESS_EXIT missing executable: '+','.join(sorted(missing)))
                return 0,('\n'.join(self.output)+'\n').encode(),b''

        expected={name:dict(path='/usr/lib/postgresql/18/bin/'+name,sha256=digest*64,
                           version=name+' (PostgreSQL) 18.6') for name,digest in (('pg_dump','a'),('pg_restore','b'))}
        try:observed=driver.tool_check(CapabilityRunner(required,facts),'a'*64)
        except driver.GateError as error:self.fail('minimal fixed-image capabilities rejected: '+str(error))
        self.assertEqual(observed,expected)
        for missing in sorted(required):
            with self.subTest(missing_executable=missing):
                self.reject(lambda:driver.tool_check(CapabilityRunner(required-{missing},facts),'a'*64),'PROCESS_EXIT')
        for offset,name in ((0,'pg_dump'),(3,'pg_restore')):
            for index,value,token in ((offset,'1 755 1','PG_TOOL_IDENTITY'),
                                      (offset,'0 777 1','PG_TOOL_IDENTITY'),
                                      (offset,'0 755 0','PG_TOOL_IDENTITY'),
                                      (offset+1,name+' (PostgreSQL) 17.6','PG_TOOL_IDENTITY'),
                                      (offset+2,'x'*64+'  /usr/lib/postgresql/18/bin/'+name,'PG_TOOL_HASH'),
                                      (offset+2,'a'*64+'  /tmp/'+name,'PG_TOOL_HASH')):
                with self.subTest(tool=name,bad_fact=value):
                    invalid=facts.copy();invalid[index]=value
                    self.reject(lambda:driver.tool_check(CapabilityRunner(required,invalid),'a'*64),token)
        self.reject(lambda:driver.tool_check(CapabilityRunner(required,facts[:-1]),'a'*64),'PG_TOOLS_OUTPUT')

    def test_issuer_receipt_joins_database_root_digest_and_exact_schema(self):
        self.assertTrue(hasattr(driver,'validate_issuer'),'strict independent issuer receipt validator missing')
        import hashlib
        binding=dict(format_version=1,capability='source_control_binding_v1',binding_id=NONCE,control_path='/var/lib/knowweave-source/control',
                     control_dev=10,control_ino=20,database=DB,database_oid=400,system_identifier='12345')
        sql=dict(database=DB,database_oid=400,system_identifier='12345')
        row=dict(binding=binding,binding_sha256=hashlib.sha256(json.dumps(binding,sort_keys=True,separators=(',',':')).encode()).hexdigest(),
                 sql=sql,issuer_euid=0,classification='INDEPENDENT_PRECOMPILE_BINDING')
        driver.validate_issuer(row,sql)
        for changes in (dict(issuer_euid=1),dict(binding_sha256='0'*64),dict(binding=dict(binding,extra=1)),dict(sql=dict(sql,database_oid=401))):
            self.reject(lambda:driver.validate_issuer(dict(row,**changes),sql),'ISSUER_')

    def test_legacy_close_only_uses_fresh_case_file_env_and_cannot_count_as_lifecycle(self):
        self.assertTrue(hasattr(driver,'legacy_env'),'dedicated legacy file env missing')
        ident=dict(self.ident,other_database='learning_backup_c4_task3_'+NONCE)
        values=driver.legacy_env(ident)
        self.assertEqual(values['TEST_C4_BINDING_CONTROL_ROOT'],'/var/lib/knowweave-source/control')
        self.assertEqual(values['TEST_C4_BINDING_OTHER_DATABASE_NAME'],'learning_backup_c4_task3_'+NONCE)
        self.assertEqual(values['TEST_C4_BINDING_ADMIN_DSN_FILE'],'/var/lib/knowweave-source/secrets/admin.dsn')
        self.assertFalse(any('postgresql://' in value for value in values.values()))
        self.reject(lambda:driver.consumer_command('a'*64,'/target/retained/test',driver.LEGACY_TEST,driver.consumer_env(self.ident)),'CONSUMER_IDENTITY')
        # Catch lifecycle fixture membership drift at the real initdb write boundary.
        from unittest.mock import patch
        import types
        helper=driver.load_public_helpers(Path(__file__).parent)[0]
        inherited=helper.INITDB
        helper_file=Path(helper.__file__);helper_bytes=helper_file.read_bytes()
        old=b'GRANT learning_auth_lock TO learning_admin WITH SET TRUE;\n'
        canonical=b'GRANT learning_auth_lock TO learning_admin WITH INHERIT FALSE, SET TRUE;\n'
        self.assertEqual(inherited.splitlines(keepends=True).count(old),1)
        class InitdbWritten(Exception):pass
        write_new=helper.write_new
        def stop_after_initdb(path,raw,mode=0o600):
            write_new(path,raw,mode)
            if path.name=='initdb.sh':raise InitdbWritten
        for legacy in (False,True):
            for label,raw in (('canonical',inherited),('missing',inherited.replace(old,b'')),
                              ('duplicate',inherited+old)):
                with self.subTest(legacy=legacy,initdb=label),tempfile.TemporaryDirectory() as directory:
                    batch=Path(directory)
                    runner=types.SimpleNamespace(logs=batch,ownership=types.SimpleNamespace(children=[]))
                    call=lambda:driver.live_case(helper,None,None,runner,batch,None,ident,'10.253.238.0/24',
                        driver.LEGACY_TEST if legacy else NAME,None,{},None,None,'a'*64,True,legacy=legacy)
                    with patch.object(helper,'INITDB',raw),patch.object(helper,'write_new',stop_after_initdb):
                        if label=='canonical':
                            with self.assertRaises(InitdbWritten):call()
                            written=(batch/CASE/'initdb.sh').read_bytes()
                            self.assertEqual(written.splitlines(keepends=True).count(canonical),1)
                            self.assertNotIn(old,written)
                            self.assertEqual(written.replace(canonical,old),inherited)
                        else:
                            try:self.reject(call,'LIFECYCLE_FIXTURE_MEMBERSHIP')
                            except InitdbWritten:self.fail(label+' membership fixture was written instead of refused')
                            self.assertFalse((batch/CASE/'initdb.sh').exists())
                    self.assertEqual(helper.INITDB,inherited)
                    self.assertEqual(helper_file.read_bytes(),helper_bytes)



class Fix1ReviewTests(unittest.TestCase):
    setUp=LifecycleProtocolTests.setUp
    reject=LifecycleProtocolTests.reject
    # Standalone boundary fixtures are unit evidence, never live PG receipts.
    def toc(self):
        return (';\n; Archive created at 2026-10-04 01:00:00 UTC\n;     dbname: '+DB+
                '\n;     TOC Entries: 4\n;     Compression: gzip\n;     Dump Version: 1.16-0\n;     Format: CUSTOM\n;     Integer: 4 bytes\n;     Offset: 8 bytes\n;     Dumped from database version: 18.6 (Debian 18.6-1)\n;     Dumped by pg_dump version: 18.6 (Debian 18.6-1)\n;\n; Selected TOC Entries:\n;\n215; 1259 16384 TABLE public asset learning_admin\n216; 0 16384 TABLE DATA public asset learning_admin\n').encode()

    def test_r1_actual_archive_toc_not_sql_title_is_accepted(self):
        self.assertTrue(hasattr(driver,'validate_toc'),'R1 real archive TOC validator missing')
        row=driver.validate_toc(0,self.toc(),b'',DB)
        self.assertEqual(row['database'],DB)
        self.assertEqual(row['entries'],2)

    def test_r1_wrong_duplicate_empty_malformed_and_plain_sql_toc_refused(self):
        self.assertTrue(hasattr(driver,'validate_toc'),'R1 real archive TOC validator missing')
        for raw in (self.toc().replace(DB.encode(),b'foreign'),self.toc()+(';     dbname: '+DB+'\n').encode(),
                    self.toc().split(b'215;')[0],self.toc().replace(b'Format: CUSTOM',b'Format: PLAIN'),
                    self.toc().replace(b'version: 18.6',b'version: 17.6'),b'-- PostgreSQL database dump\n; dbname: '+DB.encode(),
                    self.toc().replace(b'215; 1259 16384 TABLE',b'215; bad TABLE'),self.toc()+(';dbname: '+DB+'\n').encode()):
            self.reject(lambda:driver.validate_toc(0,raw,b'',DB),'PG_RESTORE_TOC')

    def test_r3_failure_stops_pg_before_stalled_anchor_with_independent_short_budget(self):
        self.assertTrue(hasattr(driver,'teardown_transports'),'R3 bounded teardown coordinator missing')
        events=[]
        with tempfile.TemporaryDirectory() as directory:
            child=driver.MonitoredChild([sys.executable,'-c','import time;time.sleep(60)'],Path(directory),timeout=1800,interactive=True)
            class Transport:
                def retire(self,timeout):
                    events.append('transport');return child.retire(timeout=timeout)
            def stop():events.append('pg-stop-and-confirm')
            result=driver.teardown_transports(stop,[Transport()],failed=True,timeout=0.2)
            self.assertEqual(events,['pg-stop-and-confirm','transport'])
            self.assertIsNotNone(child.p.poll())
            self.assertEqual(result['transport_scope'],'CLI_ONLY_NOT_EXEC_PROOF')

    def test_r3_failed_pg_stop_is_not_cli_exit_proof(self):
        self.assertTrue(hasattr(driver,'teardown_transports'),'R3 bounded teardown coordinator missing')
        events=[]
        class Transport:
            def retire(self,timeout):events.append('retired');return dict(reaped=True)
        def stop():raise driver.GateError('PG_EXEC_CLEANUP_UNKNOWN')
        receipt=driver.teardown_transports(stop,[Transport()],failed=True,timeout=0.1)
        self.assertFalse(receipt['pg_absence_verified'])
        self.assertEqual(events,['retired'])
        self.assertIn('PG_EXEC_CLEANUP_UNKNOWN',receipt['errors'])

    def test_r3_live_finish_timeout_defers_transport_kill_until_pg_retirement(self):
        import inspect
        with tempfile.TemporaryDirectory() as directory:
            child=driver.MonitoredChild([sys.executable,'-c','import time;time.sleep(60)'],Path(directory),timeout=0.05,interactive=True)
            try:
                self.assertIn('terminate_on_error',inspect.signature(child.finish).parameters,'live finish cannot defer premature CLI kill')
                self.reject(lambda:child.finish(terminate_on_error=False),'PROCESS_TIMEOUT')
                self.assertIsNone(child.p.poll())
                order=[]
                receipt=driver.teardown_transports(lambda:order.append('pg-stopped'),[child],failed=True,timeout=0.2)
                self.assertEqual(order,['pg-stopped'])
                self.assertTrue(receipt['transports_reaped'])
            finally:
                if child.p.poll() is None:child.retire(timeout=1)

    def test_r4_constructed_legacy_command_does_not_satisfy_observation_gate(self):
        self.assertTrue(hasattr(driver,'require_legacy_observation'),'R4 actual legacy observation gate missing')
        values=driver.legacy_env(dict(self.ident,other_database='learning_backup_c4_task3_'+NONCE))
        self.reject(lambda:driver.require_legacy_observation({},values),'LEGACY_ACTUAL_OBSERVATION')
        row=dict(pid=22,starttime=100,exe='/target/retained/test',uid=0,env_keys=sorted(values),ppid=0)
        record=dict(compiled={'lib':dict(binary='/target/retained/test')},consumer_process=dict(pid=22,starttime=100,exe=row['exe'],env_keys=row['env_keys']),legacy_cohort=[row],
                    legacy_rendezvous=dict(lock_confirmed=True,backend_pid=300,deadline_expired=False,commit_exit=0,backend_absent=True))
        driver.require_legacy_observation(record,values)
        for changes in (dict(lock_confirmed=False),dict(deadline_expired=True),dict(commit_exit=1),dict(backend_absent=False)):
            self.reject(lambda:driver.require_legacy_observation(dict(record,legacy_rendezvous=dict(record['legacy_rendezvous'],**changes)),values),'LEGACY_ACTUAL_OBSERVATION')
        self.reject(lambda:driver.require_legacy_observation(dict(record,legacy_cohort=[]),values),'LEGACY_ACTUAL_OBSERVATION')

    def journal(self,phase):
        names=['intent','closed','drained','dump_and_index_durable','pins_durable','release_ready','released']
        index=names.index(phase);files={}
        for n in range(index+1):
            row=dict(backup_id=ATTEMPT,phase=names[n],dump_and_index_sha256='a'*64 if n>=3 else None,pins_sha256='b'*64 if n>=4 else None)
            files[names[n].replace('_','-')+'.json']=json.dumps(row,separators=(',',':')).encode()
        return files

    def test_r5_canonical_phase_and_digest_evidence_cannot_be_missing_or_corrupt(self):
        self.assertTrue(hasattr(driver,'classify_attempt_documents'),'R5 actual phase document classifier missing')
        closed=driver.classify_attempt_documents(ATTEMPT,'c'*64,self.journal('closed'),{},journal_exists=True)
        self.assertEqual(closed['journal']['phase'],'closed')
        self.assertEqual(closed['journal']['status'],'CANONICAL')
        self.assertEqual(len(closed['journal']['file_sha256']),2)
        missing=driver.classify_attempt_documents(ATTEMPT,'c'*64,{}, {},journal_exists=False)
        self.assertEqual(missing['journal']['status'],'ABSENT')
        corrupt=driver.classify_attempt_documents(ATTEMPT,'c'*64,dict(self.journal('closed'),**{'closed.json':b'{'}),{},journal_exists=True)
        self.assertEqual(corrupt['journal']['status'],'CORRUPT')
        self.assertIsNone(corrupt['journal']['phase'])

    def test_r5_real_canonical_sidecar_join_and_orphan_conflict_are_observed(self):
        journal=self.journal('closed');last=journal['closed.json']
        import hashlib
        ready=dict(format_version=1,capability='source_abandonment_v1',backup_id=ATTEMPT,action='abandon',source_binding_sha256='c'*64,
                   journal_phase='closed',journal_record_sha256=hashlib.sha256(last).hexdigest(),retained_artifacts='keep_all',state='release_ready')
        raw=json.dumps(ready,separators=(',',':')).encode()
        terminal=dict(format_version=1,capability='source_abandonment_v1',backup_id=ATTEMPT,action='abandon',source_binding_sha256='c'*64,
                      ready_sha256=hashlib.sha256(raw).hexdigest(),state='abandoned')
        final=json.dumps(terminal,separators=(',',':')).encode()
        state=driver.classify_attempt_documents(ATTEMPT,'c'*64,journal,{'ready.json':raw,'abandoned.json':final},journal_exists=True,sidecar_exists=True)
        self.assertEqual(state['sidecar']['status'],'ABANDONED')
        self.assertEqual(state['sidecar']['file_sha256']['ready.json'],hashlib.sha256(raw).hexdigest())
        for files in ({'abandoned.json':final},{'ready.json':raw.replace(b'keep_all',b'delete')},{'ready.json':raw,'unknown.json':b'x'}):
            self.assertEqual(driver.classify_attempt_documents(ATTEMPT,'c'*64,journal,files,journal_exists=True,sidecar_exists=True)['sidecar']['status'],'CORRUPT')

    def test_r5_distinct_terminal_preflight_and_unknown_are_not_operation_success(self):
        self.assertTrue(hasattr(driver,'generation_classification'),'R5 generation evidence classifier missing')
        released=driver.classify_attempt_documents(ATTEMPT,'c'*64,self.journal('released'),{},journal_exists=True)
        absent=driver.classify_attempt_documents(ATTEMPT,'c'*64,{}, {},journal_exists=False)
        absent['publication']=dict(dump_files=[],manifest_files=[],scan_complete=True)
        before=dict(acl='t',attempt_state=released);after=dict(before)
        self.assertEqual(driver.generation_classification([before,after],None)['state'],'OBSERVED_READ_ONLY_TERMINAL_STATE')
        pre=[dict(acl='t',attempt_state=absent)]*2
        self.assertEqual(driver.generation_classification(pre,None)['state'],'OBSERVED_PREFLIGHT_STATE')
        self.assertEqual(driver.generation_classification(pre[:1],None)['state'],'UNKNOWN')
        self.assertEqual(driver.generation_classification(pre,None)['operation_outcome'],'UNPROVEN')
        corrupt=dict(released,sidecar=dict(status='CORRUPT'))
        self.assertEqual(driver.generation_classification([dict(acl='t',attempt_state=corrupt)]*2,None)['state'],'UNKNOWN')

    def test_r5_owned_drain_commit_requires_closed_predump_no_publication_barrier(self):
        self.assertTrue(hasattr(driver,'validate_drain_barrier'),'R5 independent pre-dump barrier missing')
        state=driver.classify_attempt_documents(ATTEMPT,'c'*64,self.journal('closed'),{},journal_exists=True)
        state['publication']={'dump_files':[],'manifest_files':[],'scan_complete':True}
        observation=dict(acl='f',runtime_state='idle in transaction',attempt_state=state)
        driver.validate_drain_barrier(observation)
        for changes in (dict(acl='t'),dict(runtime_state='active'),dict(attempt_state=dict(state,publication=dict(dump_files=['database.dump'],manifest_files=[],scan_complete=True))),
                        dict(attempt_state=dict(state,journal=dict(status='ABSENT',phase=None))),dict(attempt_state=dict(state,journal=dict(status='CANONICAL',phase='drained')))):
            self.reject(lambda:driver.validate_drain_barrier(dict(observation,**changes)),'DRAIN_PREDUMP_BARRIER')

    def test_r6_long_legal_setup_then_live_and_cleanup_use_coherent_finite_clock(self):
        self.assertTrue(hasattr(driver,'CaseBudget'),'R6 coherent phase budget missing')
        now=[0.0];budget=driver.CaseBudget(clock=lambda:now[0])
        now[0]=1801;self.assertGreater(budget.remaining(),5000)
        now[0]=7190;budget.transition('LIVE')
        now[0]=8291;budget.begin_rpc();budget.end_rpc()
        self.assertGreater(budget.remaining(),90)
        budget.transition('CLEANUP');self.assertEqual(budget.remaining(),180)
        now[0]+=181;self.reject(budget.check,'CASE_CLEANUP_TIMEOUT')

    def test_r6_pending_rpc_cannot_renew_phase_or_five_second_call(self):
        self.assertTrue(hasattr(driver,'CaseBudget'),'R6 coherent phase budget missing')
        now=[0.0];budget=driver.CaseBudget(clock=lambda:now[0]);budget.begin_rpc()
        self.reject(lambda:budget.transition('LIVE'),'CASE_PENDING_RPC')
        now[0]=6;self.reject(budget.check_rpc,'BROKER_TIMEOUT')
        budget.end_rpc();now[0]=7201
        self.reject(lambda:budget.transition('LIVE'),'CASE_SETUP_TIMEOUT')
        budget.transition('CLEANUP');self.assertEqual(budget.remaining(),180)
        now[0]=0;budget=driver.CaseBudget(clock=lambda:now[0]);budget.transition('LIVE')
        original=budget.deadline;budget.refresh_deadline=20;now[0]=18;budget.begin_rpc()
        self.assertEqual(budget.rpc_deadline,20);self.assertEqual(budget.remaining(),2)
        now[0]=20;self.reject(budget.check_rpc,'BROKER_TIMEOUT');budget.end_rpc()
        self.reject(budget.remaining,'REFRESH_HOST_BUDGET');self.assertEqual(budget.deadline,original)
        budget.refresh_deadline=None;budget.transition('CLEANUP');self.assertEqual(budget.remaining(),180)

    def test_r6_real_echo_transport_survives_long_legal_setup_then_live(self):
        self.assertTrue(hasattr(driver.BrokerClient,'transition'),'R6 persistent broker phase transition missing')
        now=[0.0];budget=driver.CaseBudget(clock=lambda:now[0])
        script='import sys,json\nfor line in sys.stdin:\n sys.stdout.buffer.write(json.dumps(json.loads(line),sort_keys=True,separators=(",",":")).encode()+b"\\n");sys.stdout.buffer.flush()'
        class LocalTransport:
            def persistent(self,logs,timeout):return driver.MonitoredChild([sys.executable,'-u','-c',script],logs,timeout=timeout,interactive=True,clock=budget.clock)
        with tempfile.TemporaryDirectory() as directory:
            client=driver.BrokerClient(LocalTransport(),Path(directory),budget)
            try:
                now[0]=1801;self.assertEqual(client.call({'op':'setup'}),{'op':'setup'})
                now[0]=7190;client.transition('LIVE')
                now[0]=8291;self.assertEqual(client.call({'op':'refresh'}),{'op':'refresh'})
                client.transition('CLEANUP');self.assertEqual(client.call({'op':'stop'}),{'op':'stop'})
            finally:client.child.retire(timeout=1)

    def test_r6_timed_out_real_rpc_cannot_be_renewed_or_reuse_a_late_frame(self):
        self.assertTrue(hasattr(driver.BrokerClient,'transition'),'R6 persistent broker phase transition missing')
        budget=driver.CaseBudget()
        script='import sys,time\nfor line in sys.stdin.buffer:\n time.sleep(0.2);sys.stdout.buffer.write(line);sys.stdout.buffer.flush()'
        class LocalTransport:
            def persistent(self,logs,timeout):return driver.MonitoredChild([sys.executable,'-u','-c',script],logs,timeout=timeout,interactive=True)
        with tempfile.TemporaryDirectory() as directory:
            client=driver.BrokerClient(LocalTransport(),Path(directory),budget)
            try:
                self.reject(lambda:client.call({'op':'first'},timeout=0.05),'BROKER_TIMEOUT')
                self.reject(lambda:client.transition('LIVE'),'BROKER_PROTOCOL_LOST')
                self.reject(lambda:client.call({'op':'second'}),'BROKER_PROTOCOL_LOST')
                client.transition('CLEANUP')
            finally:client.child.retire(timeout=1)


class Fix2Tests(unittest.TestCase):
    setUp=LifecycleProtocolTests.setUp
    reject=LifecycleProtocolTests.reject

    def test_f1_delayed_actual_observation_retries_only_absence(self):
        self.assertTrue(hasattr(driver,'wait_legacy_consumer'),'bounded actual legacy startup wait missing')
        now=[0.0];samples=[[],[dict(pid=22,starttime=100,uid=0,exe='/target/retained/test',ppid=0,env_keys=['PATH'])]]
        def observe():
            rows=samples.pop(0)
            if rows:driver.validate_cohort(rows,{22:dict(starttime=100,exe='/target/retained/test',env_keys=['PATH'])})
            return rows
        found=driver.wait_legacy_consumer(observe,lambda:True,'/target/retained/test',20,clock=lambda:now[0],sleep=lambda delay:now.__setitem__(0,now[0]+delay))
        self.assertEqual(found[0]['pid'],22)
        self.assertGreater(now[0],0)
        self.assertLess(now[0],20)
        # Capture only the real PROC_SCAN occurrence which the waiter returns;
        # a startup absence must not be retained as the successful observation.
        with tempfile.TemporaryDirectory() as directory:
            ident=dict(self.ident,other_database='other')
            record=dict(identity=ident,container_id='a'*64,kind='LEGACY_CLOSE_ONLY_SYNTHETIC_PHASES_NO_DUMP',compiled={'lib':dict(binary='/target/retained/test')})
            keys=sorted(driver.legacy_env(ident))
            def process(pid,exe,env):
                stat=str(pid)+' (model) '+' '.join(['S','1']+['0']*17+['100'])
                return 'ROW\t0\t'+exe+'\t'+stat+'\t'+','.join(env)+',\n'
            holder=process(23,'/usr/lib/postgresql/18/bin/psql',['HOME','LC_ALL','PATH'])
            outputs=[('SCANNER 90\n'+holder).encode(),('SCANNER 90\n'+holder+process(22,'/target/retained/test',keys)).encode()]
            class ActualScanRunner:
                counter=0;logs=Path(directory)
                def run(self,args,**kwargs):
                    self.counter+=1;out=outputs.pop(0)
                    if args[args.index('--user')+1]=='999:999':
                        expected=[driver.DOCKER,'exec','--user','999:999','a'*64,'/usr/bin/env','-i','PATH=/usr/bin:/bin','HOME=/var/lib/postgresql','LC_ALL=C','/bin/sh','-ec',driver.LEGACY_BACKEND_SCAN,'legacy-backend','79']
                        if args!=expected or not 0<kwargs['timeout']<=5:raise AssertionError((args,kwargs))
                    elif args[args.index('--user')+1]!='0:0':raise AssertionError(args)
                    for suffix,raw in (('stdout',out),('stderr',b''),('process.json',driver.canonical(dict(exit_code=0,reason=None,stdout_bytes=len(out),stderr_bytes=0)))):
                        (self.logs/(f'{self.counter:04d}.'+suffix)).write_bytes(raw)
                    return 0,out,b''
            base=ActualScanRunner();runner=driver.BudgetRunner(base,driver.CaseBudget())
            def observe_actual():
                rows=driver.observe_cohort(runner,'a'*64,record,{},True,object(),allow_startup_absence=True)
                if base.counter==1:self.assertNotIn('legacy_cohort_rpc',record)
                return rows
            matched=driver.wait_legacy_consumer(observe_actual,lambda:True,'/target/retained/test',20,clock=lambda:0,sleep=lambda _:None)
            self.assertEqual(base.counter,2);self.assertEqual(len(matched),2)
            self.assertIn('legacy_cohort_rpc',record,'successful actual legacy PROC_SCAN occurrence missing')
            envelope=record['legacy_cohort_rpc'];self.assertEqual(set(envelope),{'case_id','container_id','rpc'})
            self.assertEqual(envelope['rpc']['log_prefix'],'0002')
            self.assertEqual(envelope['rpc']['stdout_sha256'],driver.digest((base.logs/'0002.stdout').read_bytes()))
            self.assertTrue(hasattr(driver,'observe_legacy_backend'),'same-UID actual backend observation missing')
            record['legacy_cohort']=matched
            record['legacy_rendezvous']=dict(backend_pid=79,lock_confirmed=True,host_deadline_monotonic=driver.time.monotonic()+20)
            opaque=driver.digest(b'\0'*404+b'in transaction with spaces=opaque\0')
            backend=process(79,'/usr/lib/postgresql/18/bin/postgres',[]).replace('ROW\t0\t','ROW\t999\t').rsplit('\t',1)[0]+'\t'+opaque+'\n'
            outputs.append(('BACKEND_OBSERVER 999\n'+backend).encode())
            driver.observe_legacy_backend(runner,'a'*64,record,lambda:True,record['legacy_rendezvous']['host_deadline_monotonic'])
            self.assertEqual(base.counter,3)
            self.assertEqual(record['legacy_backend_probe']['rpc']['log_prefix'],'0003')
            self.assertEqual(record['legacy_backend_probe']['row'],dict(pid=79,starttime=100,uid=999,exe='/usr/lib/postgresql/18/bin/postgres',ppid=1,environ_sha256=opaque))
            self.reject(lambda:driver.observe_legacy_backend(runner,'a'*64,record,lambda:True,record['legacy_rendezvous']['host_deadline_monotonic']),'LEGACY_BACKEND_PROBE_REUSE')
            self.assertEqual(base.counter,3)


            valid=('BACKEND_OBSERVER 999\n'+backend).encode()
            for raw in (b'',valid.replace(b'OBSERVER 999',b'OBSERVER 0'),valid.replace(b'ROW\t999',b'ROW\t0'),
                        valid.replace(b'79 (model)',b'80 (model)'),valid.replace(b'postgres\t',b'psql\t'),
                        valid.replace(b'S 1 ',b'S 2 '),valid.replace(b' 100\t',b' 0\t'),
                        valid+backend.encode(),valid.replace(b'\n',b'\r\n'),
                        *(valid.replace(opaque.encode(),bad) for bad in (b'',b'HOME=secret',b'a'*63,b'g'*64,opaque.upper().encode(),b'a'*65)),
                        valid.replace(b'79 (model)',b'079 (model)'),valid.replace(b'ROW\t999',b'ROW\t0999'),
                        valid.replace(b'S 1 ',b'S 01 '),valid.replace(b' 100\t',b' 0100\t')):
                with self.subTest(raw=raw):
                    with self.assertRaises((driver.GateError,ValueError)):driver.parse_legacy_backend(raw,79)
            baseline=dict(record);baseline.pop('legacy_backend_probe')
            # A visible root0 backend retains its independent key representation.
            visible=dict(baseline,legacy_cohort=matched+[dict(pid=79,starttime=100,uid=999,exe='/usr/lib/postgresql/18/bin/postgres',ppid=1,env_keys=['HOME'])])
            outputs.append(valid);driver.observe_legacy_backend(runner,'a'*64,visible,lambda:True,visible['legacy_rendezvous']['host_deadline_monotonic'])
            self.assertEqual(visible['legacy_backend_probe']['row']['environ_sha256'],opaque)
            for key,value in [('starttime',101),('uid',0),('exe','/bin/false'),('ppid',2)]:
                bad=dict(baseline,legacy_cohort=matched+[dict(visible['legacy_cohort'][-1],**{key:value})]);outputs.append(valid)
                with self.assertRaises(driver.GateError):driver.observe_legacy_backend(runner,'a'*64,bad,lambda:True,bad['legacy_rendezvous']['host_deadline_monotonic'])
                self.assertNotIn('legacy_backend_probe',bad)
            before_guards=base.counter
            for mutate,alive in ((lambda r:r.update(kind='REAL_LIFECYCLE'),lambda:True),
                                 (lambda r:r.pop('legacy_cohort_rpc'),lambda:True),
                                 (lambda r:None,lambda:False)):
                bad=dict(baseline);mutate(bad)
                with self.assertRaises(driver.GateError):driver.observe_legacy_backend(runner,'a'*64,bad,alive,bad['legacy_rendezvous']['host_deadline_monotonic'])
                self.assertEqual(base.counter,before_guards)
            from unittest.mock import patch
            with patch.object(driver.time,'monotonic',return_value=baseline['legacy_rendezvous']['host_deadline_monotonic']):
                self.reject(lambda:driver.observe_legacy_backend(runner,'a'*64,baseline,lambda:True,baseline['legacy_rendezvous']['host_deadline_monotonic']),'LEGACY_RENDEZVOUS_TIMEOUT')
            self.assertEqual(base.counter,before_guards)

            # Deadline expiry and loss of either live transport after the RPC
            # refuse without a durable success envelope or a second probe.
            for expiry in (False,True):
                bad=dict(baseline);outputs.append(valid);before=base.counter
                live=iter((True,False)) if not expiry else iter((True,True))
                deadline=baseline['legacy_rendezvous']['host_deadline_monotonic']
                ticks=iter((deadline-1,deadline if expiry else deadline-0.5))
                with patch.object(driver.time,'monotonic',side_effect=lambda:next(ticks)):
                    with self.assertRaises(driver.GateError):driver.observe_legacy_backend(runner,'a'*64,bad,lambda:next(live),deadline)
                self.assertEqual(base.counter,before+1);self.assertNotIn('legacy_backend_probe',bad)
            import ast
            tree=ast.parse(FILE.read_bytes())
            live_case=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='live_case')
            backend_calls=[n for n in ast.walk(live_case) if isinstance(n,ast.Call) and isinstance(n.func,ast.Name) and n.func.id=='observe_legacy_backend']
            self.assertEqual(len(backend_calls),1)
            guard=next(n for n in ast.walk(live_case) if isinstance(n,ast.If) and isinstance(n.test,ast.Name) and n.test.id=='legacy' and backend_calls[0] in list(ast.walk(n)))
            text=ast.get_source_segment(FILE.read_text(),guard)
            self.assertLess(text.index('wait_legacy_consumer('),text.index('observe_legacy_backend('))
            self.assertLess(text.index('observe_legacy_backend('),text.index("holder.send(b'COMMIT;"))

    def test_f1_never_appears_cannot_renew_same_holder_deadline(self):
        self.assertTrue(hasattr(driver,'wait_legacy_consumer'),'bounded actual legacy startup wait missing')
        now=[19.9]
        self.reject(lambda:driver.wait_legacy_consumer(lambda:[],lambda:True,'/target/retained/test',20,clock=lambda:now[0],sleep=lambda delay:now.__setitem__(0,now[0]+delay)),'LEGACY_RENDEZVOUS_TIMEOUT')
        self.assertLess(now[0],20.1)

    def test_f1_exit_invalid_cohort_and_duplicate_are_immediate_refusals(self):
        self.assertTrue(hasattr(driver,'wait_legacy_consumer'),'bounded actual legacy startup wait missing')
        self.reject(lambda:driver.wait_legacy_consumer(lambda:[],lambda:False,'/target/retained/test',20,clock=lambda:0),'LEGACY_START_TRANSPORT_EXIT')
        def bad():raise driver.GateError('PROCESS_COHORT_UNAPPROVED')
        self.reject(lambda:driver.wait_legacy_consumer(bad,lambda:True,'/target/retained/test',20,clock=lambda:0),'PROCESS_COHORT_UNAPPROVED')
        rows=[dict(exe='/target/retained/test')]*2
        self.reject(lambda:driver.wait_legacy_consumer(lambda:rows,lambda:True,'/target/retained/test',20,clock=lambda:0),'CONSUMER_PROCESS_COUNT')

    def snapshot(self,root,mutate_stage_listing=False):
        # Windows uses a test-only fd adapter over actual filesystem entries.
        # It does not prove POSIX no-follow/held-fd semantics or root ownership.
        import os,errno
        actual=driver.os
        if os.name=='nt':
            class Adapter:
                O_DIRECTORY=0x40000000;O_NOFOLLOW=0x20000000;O_NONBLOCK=0
                def __init__(self):self.dirs={};self.next=1000000
                def __getattr__(self,key):return getattr(actual,key)
                def path(self,name,fd=None):return Path(name) if fd is None else self.dirs[fd]/name
                def open(self,name,flags,mode=0o777,dir_fd=None):
                    path=self.path(name,dir_fd)
                    if path.is_symlink():raise OSError(errno.ELOOP,'test no-follow refused')
                    if flags&self.O_DIRECTORY:
                        if not path.exists():raise FileNotFoundError(path)
                        if not path.is_dir():raise NotADirectoryError(path)
                        self.next+=1;self.dirs[self.next]=path;return self.next
                    return actual.open(path,flags&~(self.O_DIRECTORY|self.O_NOFOLLOW),mode)
                def listdir(self,fd):
                    path=self.dirs[fd];names=actual.listdir(path)
                    if mutate_stage_listing and path.name.startswith(ATTEMPT+'.staging-') and not (path/'manifest.json').exists():
                        (path/'manifest.json').write_bytes(b'appeared during traversal')
                    return names
                def fstat(self,fd):return self.dirs[fd].stat() if fd in self.dirs else actual.fstat(fd)
                def stat(self,name,dir_fd=None,follow_symlinks=True):return actual.stat(self.path(name,dir_fd),follow_symlinks=follow_symlinks)
                def close(self,fd):
                    if fd in self.dirs:self.dirs.pop(fd)
                    else:actual.close(fd)
            driver.os=Adapter()
        elif mutate_stage_listing:
            class RaceAdapter:
                def __getattr__(self,key):return getattr(actual,key)
                def listdir(self,fd):
                    names=actual.listdir(fd);path=Path(actual.readlink('/proc/self/fd/'+str(fd)))
                    if path.name.startswith(ATTEMPT+'.staging-') and not (path/'manifest.json').exists():
                        (path/'manifest.json').write_bytes(b'appeared during traversal')
                    return names
            driver.os=RaceAdapter()
        try:
            c=driver.os.open(root/'control',driver.os.O_RDONLY|driver.os.O_DIRECTORY|driver.os.O_NOFOLLOW)
            p=driver.os.open(root/'pins',driver.os.O_RDONLY|driver.os.O_DIRECTORY|driver.os.O_NOFOLLOW)
            try:return driver.attempt_snapshot(c,p,ATTEMPT,'c'*64)
            finally:driver.os.close(c);driver.os.close(p)
        finally:driver.os=actual

    def roots(self,directory):
        root=Path(directory);(root/'control').mkdir();(root/'pins').mkdir();return root

    def test_f2_true_multiple_producer_stages_are_scanned_and_foreign_is_not_opened(self):
        with tempfile.TemporaryDirectory() as directory:
            root=self.roots(directory)
            for suffix,leaf in ((NONCE,'database.dump'),(CASE,'manifest.json')):
                path=root/'pins'/(ATTEMPT+'.staging-'+suffix);path.mkdir();(path/leaf).write_bytes(b'kept')
            foreign=root/'pins'/(NONCE+'.staging-'+CASE);foreign.mkdir();(foreign/'database.dump').write_bytes(b'foreign kept')
            before=(foreign/'database.dump').read_bytes()
            result=self.snapshot(root)['publication']
            self.assertEqual(set(result),{'dump_files','manifest_files','scan_complete','entries'})
            self.assertTrue(result['scan_complete'])
            self.assertEqual(result['dump_files'],['pins/'+ATTEMPT+'.staging-'+NONCE+'/database.dump'])
            self.assertEqual(result['manifest_files'],['pins/'+ATTEMPT+'.staging-'+CASE+'/manifest.json'])
            self.assertEqual((foreign/'database.dump').read_bytes(),before)
            self.assertFalse(any(NONCE+'.staging-'+CASE in row['path'] for row in result['entries']))

    def test_f2_malformed_matching_names_and_non_directory_stages_are_incomplete(self):
        for name in (ATTEMPT+'.staging-not-v4',ATTEMPT+'.staging',ATTEMPT+'.staging-00000000-0000-0000-0000-000000000000'):
            with tempfile.TemporaryDirectory() as directory:
                root=self.roots(directory);(root/'pins'/name).mkdir()
                self.assertFalse(self.snapshot(root)['publication']['scan_complete'])
        with tempfile.TemporaryDirectory() as directory:
            root=self.roots(directory);(root/'pins'/(ATTEMPT+'.staging-'+NONCE)).write_bytes(b'not a directory')
            self.assertFalse(self.snapshot(root)['publication']['scan_complete'])

    def test_f2_existing_entry_budget_exhaustion_cannot_claim_complete_absence(self):
        with tempfile.TemporaryDirectory() as directory:
            root=self.roots(directory);stage=root/'pins'/(ATTEMPT+'.staging-'+NONCE);stage.mkdir()
            for number in range(129):(stage/str(number)).write_bytes(b'x')
            self.assertFalse(self.snapshot(root)['publication']['scan_complete'])

    def test_f2_matching_stage_symlink_is_refused_without_reading_target(self):
        import os
        with tempfile.TemporaryDirectory() as directory:
            root=self.roots(directory);outside=root/'outside';outside.mkdir();(outside/'database.dump').write_bytes(b'untouched')
            try:os.symlink(outside,root/'pins'/(ATTEMPT+'.staging-'+NONCE),target_is_directory=True)
            except OSError:self.skipTest('Windows symlink privilege unavailable; actual Linux no-follow test required')
            result=self.snapshot(root)['publication']
            self.assertFalse(result['scan_complete']);self.assertEqual(result['dump_files'],[])
            self.assertEqual((outside/'database.dump').read_bytes(),b'untouched')

    def test_f2_real_namespace_mutation_during_listing_is_not_complete(self):
        with tempfile.TemporaryDirectory() as directory:
            root=self.roots(directory);stage=root/'pins'/(ATTEMPT+'.staging-'+NONCE);stage.mkdir();(stage/'database.dump').write_bytes(b'kept')
            result=self.snapshot(root,mutate_stage_listing=True)['publication']
            self.assertFalse(result['scan_complete'])
            self.assertEqual((stage/'database.dump').read_bytes(),b'kept')
            self.assertTrue((stage/'manifest.json').exists())

    def test_f2_inaccessible_stage_never_claims_complete_absence(self):
        import os
        if os.name=='nt':self.skipTest('POSIX directory permissions require actual Linux scoped execution')
        if os.geteuid()==0:self.skipTest('Run this permission negative as ordinary Linux user; root may bypass DAC')
        with tempfile.TemporaryDirectory() as directory:
            root=self.roots(directory);stage=root/'pins'/(ATTEMPT+'.staging-'+NONCE);stage.mkdir();stage.chmod(0)
            try:self.assertFalse(self.snapshot(root)['publication']['scan_complete'])
            finally:stage.chmod(0o700)


class Fix3Tests(unittest.TestCase):
    setUp=LifecycleProtocolTests.setUp
    reject=LifecycleProtocolTests.reject

    def test_emitted_profile_requires_no_debug_assertions_on_and_unoptimized(self):
        self.assertTrue(hasattr(driver,'validate_compiler_profile'),'actual compiler profile validator missing')
        profile=dict(opt_level='0',debuginfo=0,debug_assertions=True,overflow_checks=True,test=True)
        row=dict(reason='compiler-artifact',executable='/target/build/debug/deps/learning_backup-aabb',profile=profile)
        self.assertEqual(driver.validate_compiler_profile(json.dumps(row),row['executable'],True),profile)
        for change in (dict(debuginfo=2),dict(debuginfo=False),dict(debug_assertions=False),dict(overflow_checks=False),dict(opt_level='3'),dict(test=False)):
            bad=json.dumps(dict(row,profile=dict(profile,**change)))
            self.reject(lambda:driver.validate_compiler_profile(bad,row['executable'],True),'BUILD_PROFILE')
        self.reject(lambda:driver.validate_compiler_profile(json.dumps(row)+'\n'+json.dumps(row),row['executable'],True),'BUILD_PROFILE')

    def test_fresh_profile_env_keeps_dev_and_test_safety_and_no_strip(self):
        self.assertTrue(hasattr(driver,'BUILD_PROFILE_ENV'),'fresh explicit profile env missing')
        for kind in ('DEV','TEST'):
            self.assertEqual(driver.BUILD_PROFILE_ENV['CARGO_PROFILE_'+kind+'_DEBUG'],'0')
            self.assertEqual(driver.BUILD_PROFILE_ENV['CARGO_PROFILE_'+kind+'_DEBUG_ASSERTIONS'],'true')
            self.assertEqual(driver.BUILD_PROFILE_ENV['CARGO_PROFILE_'+kind+'_OPT_LEVEL'],'0')
            self.assertEqual(driver.BUILD_PROFILE_ENV['CARGO_PROFILE_'+kind+'_OVERFLOW_CHECKS'],'true')
            self.assertEqual(driver.BUILD_PROFILE_ENV['CARGO_PROFILE_'+kind+'_STRIP'],'none')

    def materialization_fixture(self):
        self.assertTrue(hasattr(driver,'materialize_build_data'),'secure compiler-data materializer missing')
        import os
        if os.name!='posix' or os.geteuid()!=0:self.skipTest('Actual Linux root no-follow/materialization fixture required; production checks unchanged')
        temp=tempfile.TemporaryDirectory(prefix='knowweave-materialize-',dir='/var/lib')
        root=Path(temp.name);source=root/'source';destination=root/'private';source.mkdir(mode=0o700);destination.mkdir(mode=0o700)
        file=source/'c4_task3_migrate';file.write_bytes(b'compiler bytes '*2048);file.chmod(0o755)
        return temp,file,destination/'intermediate'

    def test_actual_hardlink_data_materializes_identical_private_singlelink(self):
        temp,source,destination=self.materialization_fixture()
        import os,hashlib
        with temp:
            alias=source.with_name('c4_task3_migrate-aabb');os.link(source,alias)
            old=source.stat();raw=source.read_bytes()
            receipt=driver.materialize_build_data(source,destination)
            self.assertEqual(destination.read_bytes(),raw)
            self.assertEqual(destination.stat().st_nlink,1)
            self.assertEqual(receipt['sha256'],hashlib.sha256(raw).hexdigest())
            self.assertEqual(source.stat().st_ino,old.st_ino);self.assertEqual(source.stat().st_nlink,2)
            self.assertEqual(alias.read_bytes(),raw)
            # Pass actual newly materialized bytes through UNCHANGED helper.
            _,binding,_=driver.load_public_helpers(Path(__file__).parent)
            retained=destination.with_name('retained')
            result=subprocess.run([sys.executable,'-c',binding.COPY_BINARY,str(destination),str(retained)],capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(retained.read_bytes(),raw);self.assertEqual(retained.stat().st_nlink,1)
            self.assertEqual(retained.stat().st_mode&0o777,0o500)

    def test_symlink_foreign_writable_nonregular_oversize_and_collision_refuse(self):
        temp,source,destination=self.materialization_fixture()
        import os
        with temp:
            destination.write_bytes(b'keep destination')
            self.reject(lambda:driver.materialize_build_data(source,destination),'MATERIALIZE_DESTINATION_EXISTS')
            self.assertEqual(destination.read_bytes(),b'keep destination')
            source.chmod(0o777)
            self.reject(lambda:driver.materialize_build_data(source,destination.with_name('wide')),'MATERIALIZE_SOURCE')
            source.chmod(0o755);foreign=source.with_name('foreign');foreign.write_bytes(b'foreign');os.chown(foreign,65534,65534)
            self.reject(lambda:driver.materialize_build_data(foreign,destination.with_name('foreign')),'MATERIALIZE_SOURCE')
            link=source.with_name('link');link.symlink_to(source)
            self.reject(lambda:driver.materialize_build_data(link,destination.with_name('link')),'MATERIALIZE_')
            fifo=source.with_name('fifo');os.mkfifo(fifo)
            self.reject(lambda:driver.materialize_build_data(fifo,destination.with_name('fifo')),'MATERIALIZE_SOURCE')
            huge=source.with_name('huge')
            with huge.open('wb') as handle:handle.truncate(128*1024**2+1)
            self.reject(lambda:driver.materialize_build_data(huge,destination.with_name('huge')),'MATERIALIZE_SOURCE')

    def test_changed_source_and_interrupted_sync_leave_no_success_or_original_mutation(self):
        temp,source,destination=self.materialization_fixture()
        with temp:
            actual=driver.os
            class ChangingRead:
                changed=False
                def __getattr__(self,key):return getattr(actual,key)
                def read(self,fd,size):
                    raw=actual.read(fd,size)
                    if not self.changed:
                        self.changed=True
                        with source.open('r+b') as handle:handle.write(b'X')
                    return raw
            driver.os=ChangingRead()
            try:self.reject(lambda:driver.materialize_build_data(source,destination),'MATERIALIZE_SOURCE_CHANGED')
            finally:driver.os=actual
            self.assertTrue(destination.exists())
            before=source.read_bytes()
            class FailedSync:
                def __getattr__(self,key):return getattr(actual,key)
                def fsync(self,fd):raise OSError('injected sync refusal')
            driver.os=FailedSync()
            try:self.reject(lambda:driver.materialize_build_data(source,destination.with_name('interrupted')),'MATERIALIZE_IO')
            finally:driver.os=actual
            self.assertEqual(source.read_bytes(),before)
            self.assertTrue(destination.with_name('interrupted').exists())


if __name__ == '__main__':
    unittest.main()
