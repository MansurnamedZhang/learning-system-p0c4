"""Pure negative gates for the private full-rehearsal topology, not live proof."""
import copy
import unittest
from scripts.p0c4_completion import full_import as full

S='550e8400-e29b-41d4-a716-446655440000'
T='650e8400-e29b-41d4-a716-446655440000'

def profile():
    def inode(path,uid=0,mode=0o700):return dict(path=path,dev=10,ino=100,uid=uid,gid=0,mode=mode)
    source='kwc4c-'+S.replace('-','')
    target='learning-system-p0c4-restore-'+T
    values={}
    for ordinal,(key,name) in enumerate([('source_pg',source+'-pg'),('source_data',source+'-source'),('source_build',source+'-build'),('source_registry',source+'-registry'),('target_pg',target+'_pg'),('target_control',target+'_control'),('target_socket',target+'_socket')]):
        values[key]=dict(name=name,**inode('/var/lib/docker/volumes/'+name+'/_data'));values[key]['ino']+=ordinal
    values['target_socket'].update(uid=999,gid=999,mode=0o3775)
    return dict(format_version=1,capability='full_rehearsal_profile_v1',source_case_id=S,target_case_id=T,
        source_container_id='a'*64,target_container_id='b'*64,source_network_id='c'*64,target_network_id='d'*64,
        daemon_id='fixed-daemon',application_build_sha256='e'*64,target_birth_sha256='f'*64,
        source_bind_root=inode('/private/full/'+S),initdb=dict(sha256='2'*64,**inode('/private/full/'+S+'/full-profile/initdb.sh',mode=0o444)),volumes=values,
        docker_client=dict(sha256='1'*64,**inode('/usr/bin/docker',mode=0o755)),
        docker_socket=inode('/var/run/docker.sock',mode=0o660))

class FullProfileGate(unittest.TestCase):
    def test_recovery_lease_body_requires_same_closed_full_context(self):
        from scripts import p0c4_completion_acceptance as runner
        name = runner.ALL_CASE_TESTS['recovery-leases']
        self.assertEqual(name, 'full_restore::live_tests::recovery_invalidates_unexpired_and_expired_source_tokens')
        for context in (None, {}, object()):
            with self.assertRaises(runner.driver.GateError):
                runner.driver.live_case(None,None,None,None,None,None,None,None,name,None,None,None,None,None,False,_full_context=context)
        with self.assertRaises(TypeError):
            full.CASE_TESTS['recovery-arbitrary'] = 'caller::test'

    def test_only_fixed_fourteen_and_five_mount_topologies_are_projected(self):
        value=full._validate_profile(profile())
        source=full._mounts(value,'source');target=full._mounts(value,'target')
        self.assertEqual(len(source),14);self.assertEqual(len(target),5)
        self.assertIn(('volume',value['volumes']['target_socket']['path'],'/var/run/knowweave-target',False),source)
        self.assertIn(('volume',value['volumes']['target_socket']['path'],'/var/run/postgresql',True),target)
        self.assertNotIn(('bind','/','/',True),source)

    def test_wrong_uuid_daemon_tool_and_socket_volume_are_rejected(self):
        rows=[]
        for key,val in [('target_case_id',S),('daemon_id',''),('application_build_sha256','X'*64)]:
            bad=profile();bad[key]=val;rows.append(bad)
        bad=profile();bad['docker_client']['path']='/bin/sh';rows.append(bad)
        bad=profile();bad['volumes']['target_socket']['name']='old-socket';rows.append(bad)
        bad=profile();bad['volumes']['target_socket']['uid']=0;rows.append(bad)
        for bad in rows:
            with self.assertRaises(full.FullImportError):full._validate_profile(bad)

    def test_extra_mount_field_and_noncanonical_or_replaced_identity_refuse(self):
        bad=profile();bad['extra_mount']='/host'
        with self.assertRaises(full.FullImportError):full._validate_profile(bad)
        for key,val in [('ino',0),('dev',True),('path','/var/lib/docker/volumes/../secret')]:
            bad=profile();bad['volumes']['target_pg'][key]=val
            with self.assertRaises(full.FullImportError):full._validate_profile(bad)
        good=profile();expected=full._mounts(good,'source')
        with self.assertRaises(full.FullImportError):full._require_mounts(expected+[('bind','/host','/host',False)],good,'source')
        changed=expected.copy();kind,path,target,rw=changed[-1];changed[-1]=(kind,path,target,not rw)
        with self.assertRaises(full.FullImportError):full._require_mounts(changed,good,'source')

    def test_profile_pin_is_exact_bytes_and_missing_pin_never_admits(self):
        raw=full._canonical(profile())
        for pin in (None,'0'*64,'BAD'):
            with self.assertRaises(full.FullImportError):full._parse_pinned(raw,pin)
        import hashlib
        self.assertEqual(full._parse_pinned(raw,hashlib.sha256(raw).hexdigest()),profile())
        with self.assertRaises(full.FullImportError):full._parse_pinned(raw+b'\n',hashlib.sha256(raw+b'\n').hexdigest())

    def test_context_requires_private_reader_and_legacy_lane_rejects_full_body_before_mutation(self):
        with self.assertRaises(full.FullImportError):full.FullRehearsalContext(None,{})
        from scripts import p0c4_source_lifecycle_gate_acceptance as driver
        name=full.CASE_TESTS['full-import']
        with self.assertRaises(driver.GateError):
            driver.live_case(None,None,None,None,None,None,None,None,name,None,None,None,None,None,False)
        with self.assertRaises(driver.GateError):
            driver.live_case(None,None,None,None,None,None,None,None,name,None,None,None,None,None,False,_full_context={})

    def test_ordinary_source_observer_never_accepts_full_projection(self):
        from scripts import p0c4_source_isolation as isolation
        value=profile();rows=[]
        names={v['path']:v['name'] for v in value['volumes'].values()}
        for kind,source,destination,rw in full._mounts(value,'source'):
            row=dict(Type=kind,Source=source,Destination=destination,RW=rw)
            if kind=='volume':row['Name']=names[source]
            rows.append(row)
        with self.assertRaises(isolation.IsolationError):isolation._source_mount_projection(rows)
        with self.assertRaises(isolation.IsolationError):isolation._observe_full_rehearsal_endpoint({},S)

class Fix1AuthorityTests(unittest.TestCase):
    def test_recovery_plan_selects_only_its_named_body_and_bound_target(self):
        from unittest.mock import patch
        from scripts.p0c4_completion import evidence, plan as envelope
        _, value = self.issued()
        value['case_name'] = 'recovery-leases'
        self.assertIs(envelope.validate_plan(value, 'recovery-leases'), value)
        with patch.object(evidence, 'private_read', return_value=full._canonical(value)):
            context = full.read_context('/root/case/plan.json', 'recovery-leases')
        context._admit_call(dict(case_id=S), '10.254.20.0/24', full.CASE_TESTS['recovery-leases'], 'a'*64)
        with self.assertRaises(full.FullImportError):
            context._admit_call(dict(case_id=S), '10.254.20.0/24', full.CASE_TESTS['full-import'], 'a'*64)
        value['target']['case_id'] = S
        with self.assertRaises(full.FullImportError):
            envelope.validate_plan(value, 'recovery-leases')

    def issued(self):
        # Exercise the real validated plan reader; only Linux private-file IO
        # is replaced by deterministic bytes in this portable parser test.
        from unittest.mock import patch
        from scripts.p0c4_completion import evidence
        plan=dict(format_version=1,capability='c4_full_import_plan_v1',batch_id='11111111-1111-4111-8111-111111111111',case_id=S,cache_scope='33333333-3333-4333-8333-333333333333',case_name='full-import',source=dict(archive=dict(path='/root/frozen/source.zip',size=42,sha256='a'*64),manifest=dict(path='/root/frozen/manifest.json',size=43,sha256='b'*64),application_commit='c'*40,application_build_sha256='a'*64),images=dict(builder='sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b',postgres='sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d'),subnet='10.254.20.0/24',evidence_root='/root/case',budget_profile='full_import_task5_v1',target=dict(case_id=T,subnet='10.254.21.0/24'))
        with patch.object(evidence,'private_read',return_value=full._canonical(plan)):
            return full.read_context('/root/case/plan.json','full-import'),plan

    def test_issued_context_public_api_cannot_retarget_or_expose_transport(self):
        context,plan=self.issued()
        for name,value in [('plan',{}),('test_name','arbitrary::test'),('runner',object()),('target_identity',{}),('target_root','/foreign')]:
            with self.assertRaises(AttributeError,msg=name):setattr(context,name,value)
        self.assertFalse(hasattr(context,'docker'))
        self.assertFalse(hasattr(context,'runner'))
        observed=context.observation();observed['case']='full-dirty-target'
        self.assertEqual(context.observation()['case'],'full-import')
        plan['case_name']='full-dirty-target'
        context._admit_call(dict(case_id=S),'10.254.20.0/24',full.CASE_TESTS['full-import'],'a'*64)
        with self.assertRaises(full.FullImportError):context._admit_call(dict(case_id=S),'10.254.20.0/24','arbitrary::test','a'*64)

    def test_role_profile_exposes_no_mutable_management_state(self):
        from scripts.p0c4_completion import roles
        from scripts.test_p0c4_full_restore_roles import recipe
        # Same validated carriers consumed by normal provision_full_restore_roles;
        # no cluster is created and this does not claim a Complete positive.
        verified=roles.VerifiedRoleRecipe(roles._TOKEN,full._canonical(recipe()),{})
        target=roles.FreshTargetPlan(roles._TOKEN,{},None,None)
        profile=roles._FullRoleProfile(roles._TOKEN,verified,target)
        for name in ('recipe','plan','runner','docker'):
            self.assertFalse(hasattr(profile,name))
            with self.assertRaises(AttributeError):setattr(profile,name,{})

    def test_negative_case_observation_cannot_claim_earlier_failure(self):
        stages={'full-bad-input':'dump_freeze','full-bad-role':'roles','full-dirty-target':'clean_target','full-wrong-endpoint':'writer_route','full-asset-corrupt':'original_set'}
        for case,stage in stages.items():
            good=dict(stage=stage,sha256='a'*64,bytes=100,unchanged=True)
            full._validate_negative_observation(case,good)
            for bad in (None,dict(good,stage='admission'),dict(good,unchanged=False),dict(good,bytes=True)):
                with self.assertRaises(full.FullImportError):full._validate_negative_observation(case,bad)

    def test_missing_or_changed_indirect_controller_is_rejected_before_import(self):
        from unittest.mock import patch
        from scripts import p0c4_completion_acceptance as controller
        paths=('p0c4_restore_target.py','p0c4_restore_target_birth.py','p0c4_restore_target_pin_prepare.py','p0c4_restore_target_pin.py','p0c4_restore_birth_acceptance.py','p0c4_completion/roles.py','p0c4_completion/full_import.py','p0c4_completion/full_target_fs.py','p0c4_controlled_import_acceptance.py','p0c4_maintenance_gate_acceptance.py')
        files={'scripts/'+p:b'# reviewed private code\n' for p in paths}
        with patch.object(controller,'_installed_full_controller_bytes',return_value=b'# reviewed private code\n'):
            controller._verify_full_import_closure(files)
            missing=dict(files);del missing['scripts/p0c4_restore_birth_acceptance.py']
            with self.assertRaises(controller.CompletionError):controller._verify_full_import_closure(missing)
        with patch.object(controller,'_installed_full_controller_bytes',side_effect=FileNotFoundError('missing installed indirect module')):
            with self.assertRaises(FileNotFoundError):controller._verify_full_import_closure(files)
        def changed(path):return b'raise RuntimeError("must not execute")' if path=='p0c4_restore_birth_acceptance.py' else b'# reviewed private code\n'
        with patch.object(controller,'_installed_full_controller_bytes',side_effect=changed):
            with self.assertRaises(controller.CompletionError):controller._verify_full_import_closure(files)

class BudgetRunnerPreparationTests(unittest.TestCase):
    def test_compose_uses_verified_stdin_without_manager_target_files(self):
        import hashlib
        from pathlib import Path
        from types import SimpleNamespace
        context,plan=Fix1AuthorityTests().issued()
        calls=[]
        context._target_root=Path('/unmounted/control')
        raw=b'{"fixed":"document"}'
        context._compose_sha=hashlib.sha256(raw).hexdigest()
        context._verify_compose_bytes(raw)
        context._runner=SimpleNamespace(docker=lambda *args,**kwargs:(calls.append((args,kwargs)) or b''))
        path=str(context._target_root/'targets'/plan['target']['case_id']/'compose.json')
        context._docker('compose','-f',path,'config','-q')
        self.assertEqual(calls[0][0],('compose','-f','-','config','-q'))
        self.assertEqual(calls[0][1]['stdin'],raw)
        with self.assertRaises(full.FullImportError):context._docker('compose','-f','/foreign','config','-q')
        self.assertEqual(len(calls),1)

    def test_actual_prepare_uses_original_ledger_and_keeps_budgeted_io(self):
        import json
        import tempfile
        from pathlib import Path
        from types import SimpleNamespace
        from unittest.mock import patch
        # These are the same module identities imported inside actual _prepare.
        import p0c4_source_lifecycle_gate_acceptance as driver
        import p0c4_source_binding_gate_acceptance as binding
        import p0c4_restore_target as target
        import p0c4_restore_target_pin_prepare as pin_prepare

        _,plan=Fix1AuthorityTests().issued()
        from p0c4_completion import full_import as actual_full, evidence, resources
        with patch.object(evidence,'private_read',return_value=actual_full._canonical(plan)):
            context=actual_full.read_context('/root/case/plan.json','full-import')
        with tempfile.TemporaryDirectory() as temp:
            batch=Path(temp)/'batch'
            logs=batch/'evidence'/'logs';logs.mkdir(parents=True)
            case=batch/S;case.mkdir()
            target_id=target.identity_for(T)
            from p0c4_completion import full_target_fs as fs
            import queue
            pg_name=target_id['project']+'_pg'
            volumes={}
            calls=[]

            class ExternalTransport:
                # The only fake is external transport/FS. OwnedRunner,
                # CleanupReservations, BudgetRunner, creation_call and
                # binding.create_volume all execute their real code.
                def __init__(self,path):self.logs=path;self.counter=0
                def run(self,command,**kwargs):
                    self.counter+=1
                    args=command[1:]
                    owner=getattr(self,'pending_create',None)
                    calls.append(dict(argv=list(command),timeout=kwargs['timeout'],owner=owner,
                                      state=None if owner is None else owner['state']))
                    if args==['volume','ls','--format','{{.Name}}']:
                        out=('\n'.join(sorted(volumes))+'\n').encode()
                    elif args[:2]==['volume','create']:
                        name=args[-1]
                        self_path=Path(temp)/'unmounted-host-volume'/name
                        labels=dict(args[i+1].split('=',1) for i,v in enumerate(args) if v=='--label')
                        volumes[name]=dict(Name=name,Driver='local',Options={},Labels=labels,Mountpoint=str(self_path))
                        out=(name+'\n').encode()
                    elif args[:3]==['volume','inspect','--format']:
                        out=b''.join(json.dumps(dict(Name=name,Project=None)).encode()+b'\n' for name in args[4:])
                    elif args[:2]==['volume','inspect']:
                        out=json.dumps([volumes[args[2]]]).encode()
                    elif args in (['ps','-aq','--no-trunc'],['network','ls','-q','--no-trunc']):out=b''
                    elif args==['info','--format','{{.ID}}']:
                        out=b'fixed-daemon\n'
                    else:raise AssertionError('unexpected external command: '+repr(command))
                    prefix=f'{self.counter:04d}'
                    for suffix,raw in [('stdout',out),('stderr',b''),('process.json',b'{}')]:
                        (self.logs/(prefix+'.'+suffix)).write_bytes(raw)
                    return 0,out,b''

            helper=SimpleNamespace(Runner=ExternalTransport,BUILDER="fixed-builder",
                mkdir_new=lambda path:path.mkdir(),
                write_new=lambda path,raw,mode:path.write_bytes(raw))
            result=dict(batch_id=plan['batch_id'],helpers=[])
            base=driver.owned_log_runner(helper,batch,result,1)
            self.addCleanup(driver.LOG_RESERVATIONS.pop,str(logs),None)
            ledger=base.ownership
            budget=driver.CaseBudget(clock=lambda:100.0,total_deadline=281.25)
            bounded=driver.BudgetRunner(base,budget)
            ledger.case_id=S
            self.assertFalse(hasattr(bounded,'ownership'))
            record={}
            def provision(root,batch_id,subnet,initdb,*,_full_profile):
                self.assertIs(_full_profile,context)
                # Later context IO requests 15s but must still be clamped by
                # the original 1.25s CaseBudget and logged by the same runner.
                self.assertEqual(context._docker('info','--format','{{.ID}}'),'fixed-daemon\n')
                volumes[pg_name]=dict(Name=pg_name,Driver='local',Options={},Labels={},Mountpoint=str(Path(temp)/'pg-data'))
                return dict(container_id='d'*64,network_id='e'*64,volume_name=pg_name,birth_sha256='f'*64)
            class FilesystemTransport:
                def __init__(self):self.events=queue.Queue();self.p=SimpleNamespace(poll=lambda:None);self.owner=None
                def send(self,raw):
                    value=json.loads(raw)
                    if self.owner is None:
                        class Backend:
                            def perform(self,op,body):
                                if op=='setup':return dict(root=dict(path="/owned",dev=1,ino=2,uid=0,gid=0,mode=0o700),lock_held=True)
                                if op=='pin':
                                    assert body['before']['daemon_id']=='fixed-daemon'
                                    assert len(body['before']['volumes'])==2
                                    return dict(precreation_sha256='a'*64)
                                raise AssertionError(op)
                        self.owner=fs._Protocol(fs._TOKEN,value['session'],Backend())
                    reply=self.owner.handle(value)
                    self.events.put((0,fs._canonical(reply)+b'\n'))
                def check(self):pass
            transport=FilesystemTransport()
            class ExternalHelper:
                def __init__(self,*args,**kwargs):
                    profile=kwargs['_full_profile'];profile._request(args[5],args[6],kwargs['interactive'],False,None)
                    self.closed=False
                def persistent(self,*args,**kwargs):return transport
                def close(self):self.closed=True
            with patch.object(fs,'_suite',return_value=('kwc4c-suite-'+'1'*32,'/private-suite')),patch.object(driver,'HelperContainer',ExternalHelper),patch.object(actual_full,'_measured_regular',return_value=(b'# fixed recipe\n',dict(uid=0,mode=0o444))),patch.object(resources,'route_observer',return_value=[]),patch.object(target,'_provision',side_effect=provision),patch('os.chmod'):
                context._prepare(helper,binding,bounded,batch,batch/'source',case,dict(case_id=S),{'fixed-builder':{'Id':'sha256:'+'1'*64}},result,record,{'deploy/p0c4_full_restore_initdb.sh':b'# fixed recipe\n'})
            self.assertEqual(transport.owner._state,'pin')
            expected={target_id['project']+'_'+kind for kind in ('control','socket')}
            self.assertIs(base.ownership,ledger)
            self.assertEqual(set(ledger.owners),expected)
            self.assertTrue(all(owner['state']=='retained' for owner in ledger.owners.values()))
            creations=[call for call in calls if call['argv'][1:3]==['volume','create']]
            self.assertEqual(len(creations),2)
            for call in creations:
                self.assertIs(call['owner'],ledger.owners[call['argv'][-1]])
                self.assertEqual(call['state'],'sent')
            self.assertTrue(calls)
            self.assertTrue(all(call['timeout']==1.25 for call in calls))
            self.assertEqual(calls[-1]['argv'][1:3],['volume','inspect'])
            self.assertEqual(len(list(logs.iterdir())),3*len(calls))
            self.assertIsNone(base.pending_create)
            self.assertFalse(hasattr(bounded,'ownership'))
            before=len(calls)
            budget.deadline=100.0
            with self.assertRaises(driver.GateError):bounded.docker('info','--format','{{.ID}}')
            self.assertEqual(len(calls),before)

class _SocketProfileHarness:
    """Only Docker transport, shell OS calls, and local metadata IO are doubles."""
    def __init__(self, test, *, scenario='normal', uid_output=b'999\n999\n', socket_mode=0o1775):
        import inspect, json, tempfile
        from pathlib import Path, PurePosixPath
        from types import SimpleNamespace
        from unittest.mock import patch
        import p0c4_source_lifecycle_gate_acceptance as driver
        import p0c4_source_admission_gate_acceptance as admission
        import p0c4_restore_target as target
        from p0c4_completion import full_import, full_target_fs, evidence
        self.full,self.fs,self.driver,self.admission=full_import,full_target_fs,driver,admission
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
        self.calls=[];self.actions=[];self.measurements=[];self.writes={};self.scenario=scenario
        self.socket=dict(dev=64512,ino=5260842,uid=999,gid=0 if socket_mode==0o1775 else 999,mode=socket_mode)
        _,plan=Fix1AuthorityTests().issued()
        with patch.object(evidence,'private_read',return_value=full_import._canonical(plan)):
            context=full_import.read_context('/root/case/plan.json','full-import')
        self.context=context;self.plan=plan;self.cid='b'*64
        self.value=profile();self.record={};self.test=test
        context._target_identity=target.identity_for(T);context._ident=dict(case_id=S,pg_name='kwc4c-'+S.replace('-','')+'-pg-1')
        context._case=PurePosixPath('/private/full/'+S);context._profile_root=context._case/'full-profile';context._initdb=context._profile_root/'initdb.sh'
        context._record=self.record;context._result=dict(batch_id=plan['batch_id'],helpers=[])
        context._images={'builder':dict(Id=plan['images']['builder'])}
        context._h=SimpleNamespace(BUILDER='builder',write_new=lambda path,raw,mode:self.writes.__setitem__(str(path),raw))
        context._binding=None;context._batch=self.root
        context._target_volumes={kind:dict(Name=self.value['volumes']['target_'+kind]['name'],Mountpoint=self.value['volumes']['target_'+kind]['path'],Driver='local',Options={},Labels={'knowweave.full-rehearsal.batch':plan['batch_id'],'knowweave.full-rehearsal.target':T,'knowweave.full-rehearsal.kind':kind}) for kind in ('pg','control','socket')}
        context._target_root=PurePosixPath(context._target_volumes['control']['Mountpoint'])
        context._target_created_ids=dict(container_id=self.cid,network_id='d'*64,volume_name=context._target_identity['volume'],volume_mountpoint=context._target_volumes['pg']['Mountpoint'])
        context._fs_owner=full_target_fs._OwnerClient(full_target_fs._TOKEN,SimpleNamespace(closed=False),None)
        context._fs_owner._state='created'
        project=context._target_identity['project'];socket=context._target_volumes['socket']
        self.facts=dict(Id=self.cid,Name='/'+project+'-pg-1',Image=plan['images']['postgres'],Config=dict(Image=context._target_identity['image'],Labels={'com.docker.compose.project':project,'com.docker.compose.service':'pg'}),State=dict(Running=True,StartedAt='2026-10-09T00:00:00.000000000Z',Health=dict(Status='healthy')),Mounts=[dict(Type='volume',Name=socket['Name'],Source=socket['Mountpoint'],Destination='/var/run/postgresql',RW=True)],HostConfig=dict(Mounts=[dict(Type='volume',Source=socket['Name'],Target='/var/run/postgresql',VolumeOptions=dict(NoCopy=True))]))
        self.facts['Mounts'] += [dict(Type='volume',Name=context._target_identity['volume'],Source=context._target_volumes['pg']['Mountpoint'],Destination='/var/lib/postgresql',RW=True),dict(Type='bind',Source=str(context._initdb),Destination='/docker-entrypoint-initdb.d/10-restore.sh',RW=False)]
        self.facts['Mounts'] += [dict(Type='bind',Source=str(context._target_root/'targets'/T/'secrets'/(role+'_password')),Destination='/run/secrets/'+role+'_password',RW=False) for role in ('postgres','admin')]
        logs=self.root/'evidence/logs';logs.mkdir(parents=True)
        harness=self
        class Transport:
            def __init__(self,path):self.logs=path;self.counter=0
            def run(self,command,**kwargs):
                # Bind the trusted production signature before simulated process
                # execution; wrappers must not invent keywords or exit bypasses.
                bound=inspect.signature(admission.Runner.run).bind(self,command,**kwargs)
                bound.apply_defaults();options=bound.arguments
                self.counter+=1;harness.calls.append(dict(argv=list(command),timeout=options['timeout'],allowed=list(options['allowed'])))
                args=command[1:];code=0;err=b'';reason=None
                if args==['exec','--user','0:0',harness.cid,'sh','-c','id -u postgres; id -g postgres']:
                    if isinstance(uid_output,Exception):code,out,reason=124,b'','TIMEOUT'
                    else:out=uid_output
                elif args[:2]==['container','inspect'] and args[2]==harness.cid:
                    facts=copy.deepcopy(harness.facts)
                    if scenario=='mount-drift' and harness.actions:facts['Mounts'][0]['Source']='/foreign'
                    if scenario=='restart' and harness.actions:facts['State']['StartedAt']='2026-10-09T00:00:01.000000000Z'
                    out=json.dumps([facts]).encode()
                elif args[:2]==['volume','inspect'] and args[2]==socket['Name']:
                    out=json.dumps([socket]).encode()
                elif args[:4]==['exec','--user','0:0',harness.cid]:
                    test.assertEqual(args[4:6],['sh','-ceu'])
                    test.assertEqual(len(args),7)
                    if scenario=='exec-failure':code,out=1,b''
                    elif scenario in ('recipe-timeout','recipe-log-budget'):
                        code,out,reason=137,b'',{'recipe-timeout':'PROCESS_TIMEOUT','recipe-log-budget':'PROCESS_LOG_BUDGET'}[scenario]
                    elif scenario=='receipt-missing':out=b''
                    elif scenario=='receipt-malformed':out=b'not-metadata\n'
                    else:code,out,err=harness.shell(args[6],options['timeout'])
                elif args==['info','--format','{{.ID}}']:out=b'fixed-daemon\n'
                elif args==['container','inspect','a'*64]:out=json.dumps([dict(Mounts=harness.source_mounts())]).encode()
                else:raise AssertionError('unexpected Docker boundary: '+repr(args))
                for suffix,raw in [('stdout',out),('stderr',err),('process.json',driver.canonical(dict(exit_code=code,reason=reason,stdout_bytes=len(out),stderr_bytes=len(err))))]:
                    (self.logs/(f'{self.counter:04d}.'+suffix)).write_bytes(raw)
                if reason=='TIMEOUT':raise TimeoutError('fixed external timeout')
                admission.require(reason is None and code in options['allowed'],reason or 'PROCESS_EXIT')
                return code,out,err
        self.base=driver.owned_log_runner(SimpleNamespace(Runner=Transport),self.root,context._result,0)
        self.budget=driver.CaseBudget(clock=lambda:100,total_deadline=281.25)
        context._runner=driver.BudgetRunner(self.base,self.budget)
        ledger=self.base.ownership;ledger.case_id=S;ledger._full_pair=context
        context._target_owner=ledger.acquire('pg',project+'-pg-1',dict(name=project+'-pg-1'),dict(name=context._target_identity['network']),{})
        ledger.known(context._target_owner,self.cid)
        test.addCleanup(self.close)

    def close(self):
        import json,os,shutil
        from pathlib import Path
        output=os.environ.get('FIX5_EVIDENCE_DIR')
        if output:
            ordinal=1;folder=Path(output)/f'case-{ordinal:03d}'
            while folder.exists():ordinal+=1;folder=Path(output)/f'case-{ordinal:03d}'
            shutil.copytree(self.root,folder)
            (folder/'observations.json').write_text(json.dumps(dict(test=self.test.id(),scenario=self.scenario,calls=self.calls,actions=self.actions,socket=self.socket,measurements=self.measurements,record=self.record,profile_issued=self.context._profile is not None),sort_keys=True),encoding='utf-8')
        self.driver.LOG_RESERVATIONS.pop(str(self.base.logs),None);self.temp.cleanup()

    def shell(self,program,timeout):
        import os,shlex,subprocess
        from pathlib import Path
        shell=Path('C:/Program Files/Git/bin/sh.exe') if os.name=='nt' else Path('/bin/sh')
        # Execute the production POSIX shell bytes. Only stat/readlink/chdir/
        # chown/chmod are substituted, so no host directory is touched.
        prefix=r'''
uid=999;gid=0;mode=1775;bits=43fd;held=0
stat() {
 [ "$#" = 4 ] && [ "$1" = -c ] && [ "$2" = '%d %i %u %g %a %f' ] && [ "$3" = -- ] || return 91
 case "$4" in
 /|/var|/run)
  case "$scenario:$4" in
   parent-owner:/run) printf '1 2 999 0 755 41ed\n';;
   parent-write:/run) printf '1 2 0 0 777 41ff\n';;
   parent-symlink:/var) printf '1 2 0 0 777 a1ff\n';;
   parent-stat-failure:/run) printf '1 2 0 0 755 41ed\n';return 1;;
   *) printf '1 2 0 0 755 41ed\n';;
  esac;;
 /var/run/postgresql|.)
  node=5260842;kind=$bits
  case "$scenario:$held:$4" in
   symlink:0:*) kind=a3fd;;
   non-dir:0:*) kind=83fd;;
   socket-owner:*) uid=42;;
   socket-group:*) gid=42;;
   changed-open:1:.) node=5260843;;
   changed-path:1:/var/run/postgresql) node=5260843;;
  esac
  printf '64512 %s %s %s %s %s\n' "$node" "$uid" "$gid" "$mode" "$kind";;
 *) return 92;;
 esac
}
readlink() {
 [ "$#" = 2 ] && [ "$1" = -- ] && [ "$2" = /var/run ] || return 93
 if [ "$scenario" = run-alias ];then printf '/foreign\n';else printf '/run\n';fi
}
cd() { [ "$#" = 2 ] && [ "$1" = -P ] && [ "$2" = /var/run/postgresql ] || return 94;held=1; }
chown() {
 [ "$#" = 4 ] && [ "$1" = -h ] && [ "$2" = -- ] && [ "$3" = 999:999 ] && [ "$4" = . ] && [ "$held" = 1 ] || return 95
 [ "$scenario" != chown-failure ] || return 1
 printf 'chown\n' >> "$trace";uid=999;gid=999
}
chmod() {
 [ "$#" = 3 ] && [ "$1" = -- ] && [ "$2" = 3775 ] && [ "$3" = . ] && [ "$held" = 1 ] || return 96
 [ "$scenario" != chmod-failure ] || return 1
 printf 'chmod\n' >> "$trace";mode=3775;bits=47fd
 case "$scenario" in readback-mode) mode=1775;bits=43fd;;readback-owner) uid=0;;readback-group) gid=0;;readback-type) bits=87fd;;esac
}
'''
        trace=self.root/'shell-actions.txt'
        setup='scenario='+shlex.quote(self.scenario)+'\ntrace='+shlex.quote(trace.as_posix())+'\n'
        command=setup+prefix+program
        (self.root/'recipe.sh').write_text(program,encoding='utf-8')
        (self.root/'shell-os-double.sh').write_text(setup+prefix,encoding='utf-8')
        completed=subprocess.run([str(shell),'-ceu',command],capture_output=True,timeout=timeout)
        (self.root/'shell.stdout').write_bytes(completed.stdout);(self.root/'shell.stderr').write_bytes(completed.stderr)
        if trace.exists():
            self.actions=trace.read_text().splitlines()
            if 'chown' in self.actions:self.socket.update(uid=999,gid=999)
            if 'chmod' in self.actions:self.socket['mode']=0o3775
        return completed.returncode,completed.stdout,completed.stderr

    def configure(self):self.context._configure(self.context._target_identity,self.cid)

    def source_mounts(self):
        v=self.value['volumes'];root='/private/full/'+S
        rows=[('volume',v[k]['path'],dest,rw) for k,dest,rw in [('source_pg','/var/lib/postgresql',True),('source_data','/var/lib/knowweave-source',True),('source_build','/target',False),('source_registry','/var/lib/knowweave-c4/registry',True),('target_control',v['target_control']['path'],True),('target_socket','/var/run/knowweave-target',False)]]
        rows += [('bind',root+'/initdb.sh','/docker-entrypoint-initdb.d/10-lifecycle.sh',False),('bind','/usr/bin/docker','/usr/bin/docker',False),('bind','/var/run/docker.sock','/var/run/docker.sock',False),('bind',v['target_pg']['path'],v['target_pg']['path'],False),('bind',root+'/full-profile','/var/lib/knowweave-full-rehearsal',False),('bind',root+'/full-profile/initdb.sh',root+'/full-profile/initdb.sh',False)]
        rows += [('bind',root+'/secrets/'+role+'_password','/run/secrets/'+role+'_password',False) for role in ('postgres','admin')]
        names={row['path']:row['name'] for row in v.values()}
        return [dict(Type=kind,Source=source,Destination=dest,RW=rw,**({'Name':names[source]} if kind=='volume' else {})) for kind,source,dest,rw in rows]

    def finalize(self,metadata_failure=None):
        import json,os,stat
        from types import SimpleNamespace
        from unittest.mock import patch
        h=self;context=self.context;v=self.value
        context._target_status=dict(container_id=self.cid,network_id='d'*64,birth_sha256='f'*64)
        self.record.update(container_id='a'*64,network_id='c'*64,volumes={name:dict(Name=v['volumes'][kind]['name'],Mountpoint=v['volumes'][kind]['path']) for kind,name in [('source_pg','volume'),('source_data','source_volume'),('source_build','build_volume'),('source_registry','registry_volume')]})
        class HelperTransport:
            def __init__(self,*args,**kwargs):
                spec=kwargs['_full_profile'];spec._request(args[5],args[6],kwargs['interactive'],False,None)
            def run(self,*,stdin,timeout):
                request=json.loads(stdin);roots={}
                for key,row in request['roots'].items():
                    identity={k:x for k,x in v['volumes'][key].items() if k!='name'}
                    if key=='target_socket':identity.update(h.socket)
                    roots[key]=dict(identity=identity,tree_sha256='a'*64,entries=0,bytes=0)
                result=dict(request,roots=roots);raw=h.full._canonical(result)
                h.measurements.append(json.loads(raw));(h.root/(request['phase']+'-measure.stdout')).write_bytes(raw)
                return 0,raw,b''
        files={'/usr/bin/docker':b'fixed Docker executable',str(context._initdb):b'fixed initdb recipe'};fds={};seq=[10]
        def open_file(path,flags):
            path=str(path)
            if path==metadata_failure:raise OSError('PRIVATE-METADATA-MUST-NOT-APPEAR')
            self.test.assertTrue(flags & os.O_NOFOLLOW)
            seq[0]+=1;fds[seq[0]]=[path,0];return seq[0]
        def fstat(fd):
            path=fds[fd][0];raw=files.get(path,self.writes.get(path))
            mode=0o755 if path=='/usr/bin/docker' else 0o444
            return SimpleNamespace(st_dev=10,st_ino=200+fd,st_uid=0,st_gid=0,st_mode=stat.S_IFREG|mode,st_size=len(raw),st_nlink=1,st_mtime_ns=1,st_ctime_ns=1)
        def read(fd,cap):
            path,offset=fds[fd];raw=files.get(path,self.writes.get(path));chunk=raw[offset:offset+cap];fds[fd][1]+=len(chunk);return chunk
        def lstat(path):
            path=str(path)
            if path==metadata_failure:raise OSError('PRIVATE-METADATA-MUST-NOT-APPEAR')
            socket=path=='/var/run/docker.sock'
            self.test.assertIn(path,('/var/run/docker.sock',str(context._case)))
            return SimpleNamespace(st_dev=10,st_ino=400 if socket else 401,st_uid=0,st_gid=0,st_mode=(stat.S_IFSOCK|0o660) if socket else (stat.S_IFDIR|0o700))
        with patch.object(self.driver,'HelperContainer',HelperTransport),patch.object(os,'O_NOFOLLOW',getattr(os,'O_NOFOLLOW',0x20000),create=True),patch.object(os,'O_CLOEXEC',getattr(os,'O_CLOEXEC',0x80000),create=True),patch.object(os,'open',open_file),patch.object(os,'fstat',fstat),patch.object(os,'read',read),patch.object(os,'close',lambda fd:fds.pop(fd)),patch.object(os,'lstat',lstat):
            context._finalize_profile(self.record,{})


class FreshSocketProfileTests(unittest.TestCase):
    def test_actual_bounded_runner_contract_reaches_recipe_and_rejects_default_nonzero(self):
        import json
        for scenario,reason in [('normal',None),('exec-failure','PROCESS_EXIT'),('recipe-timeout','PROCESS_TIMEOUT'),('recipe-log-budget','PROCESS_LOG_BUDGET')]:
            with self.subTest(scenario=scenario):
                h=_SocketProfileHarness(self,scenario=scenario)
                if reason is None:
                    h.configure()
                    self.assertEqual(h.record['target_socket_stage'],'VERIFIED')
                    self.assertEqual((h.socket['uid'],h.socket['gid'],h.socket['mode']),(999,999,0o3775))
                    self.assertEqual(h.actions,['chown','chmod'])
                else:
                    with self.assertRaisesRegex(h.admission.GateError,'^'+reason+'$'):h.configure()
                    self.assertEqual(h.record['target_socket_stage'],'DIRECTORY_CONFIGURE')
                    self.assertNotIn('target_socket_configuration',h.record)
                    self.assertEqual(h.actions,[])
                self.assertIsNone(h.context._target_status);self.assertIsNone(h.context._profile)
                self.assertEqual(h.writes,{})
                recipes=[call for call in h.calls if call['argv'][1:3]==['exec','--user'] and call['argv'][5:7]==['sh','-ceu']]
                self.assertEqual(len(recipes),1)
                self.assertEqual(recipes[0]['allowed'],[0]);self.assertEqual(recipes[0]['timeout'],1.25)
                logs=sorted(h.base.logs.glob('*.process.json'))
                self.assertEqual(len(logs),len(h.calls))
                recipe_log=json.loads(logs[3].read_bytes())
                self.assertEqual(recipe_log['reason'],None if reason=='PROCESS_EXIT' else reason)
                self.assertEqual(recipe_log['exit_code'],0 if reason is None else 1 if reason=='PROCESS_EXIT' else 137)
                state=h.driver.LOG_RESERVATIONS[str(h.base.logs)]
                self.assertEqual(state['work'],{})
                self.assertEqual(len(state['producer_identities']),3*len(h.calls))

    def test_actual_producer_establishes_3775_before_strict_profile_publication(self):
        h=_SocketProfileHarness(self);h.configure()
        self.assertEqual((h.socket['uid'],h.socket['gid'],h.socket['mode']),(999,999,0o3775))
        self.assertIsNone(h.context._target_status);self.assertEqual(h.writes,{})
        h.finalize()
        self.assertEqual(h.context._profile['volumes']['target_socket']['mode'],0o3775)
        self.assertEqual(h.context._phase,'COMPLETE')
        self.assertEqual(h.actions,['chown','chmod'])
        self.assertTrue(all(call['timeout']==1.25 for call in h.calls))
        self.assertEqual(len(list(h.base.logs.iterdir())),3*len(h.calls))
        self.assertIs(h.driver.runner_base(h.context._runner).ownership,h.base.ownership)

    def test_recorded_pg_observation_is_not_expanded_by_finalizer_merge(self):
        h=_SocketProfileHarness(self,socket_mode=0o3775);h.finalize()
        self.assertEqual(set(h.record['target_measurements'][0]['roots']),{'source_pg','target_pg'})
        self.assertEqual(h.record['target_measurements'],h.measurements)
        self.assertEqual(len(h.context._profile['volumes']),7)

    def test_actual_1775_socket_stays_a_strict_negative(self):
        bad=profile();bad['volumes']['target_socket'].update(uid=999,gid=0,mode=0o1775)
        with self.assertRaisesRegex(full.FullImportError,'FULL_PROFILE_SOCKET_OWNER_MODE'):full._validate_profile(bad)

    def test_wrong_target_authority_or_mount_rejects_before_recipe(self):
        for variant in ('cid','owner-id','owner-state','owner-ledger','already-born','image','name','project','service','stopped','missing-start','mount-type','mount-name','mount-source','mount-rw','mount-duplicate','nocopy','nocopy-type','declared-source','declared-rw'):
            with self.subTest(variant=variant):
                h=_SocketProfileHarness(self,scenario=variant);c=h.context;f=h.facts
                if variant=='cid':c._target_created_ids['container_id']='e'*64
                elif variant=='owner-id':c._target_owner['id']='e'*64
                elif variant=='owner-state':c._target_owner['state']='sent'
                elif variant=='owner-ledger':c._target_owner=copy.deepcopy(c._target_owner)
                elif variant=='already-born':c._fs_owner._state='birth'
                elif variant=='image':f['Image']='sha256:'+'0'*64
                elif variant=='name':f['Name']='/old-target'
                elif variant in ('project','service'):f['Config']['Labels']['com.docker.compose.'+variant]='foreign'
                elif variant=='stopped':f['State']['Running']=False
                elif variant=='missing-start':del f['State']['StartedAt']
                elif variant.startswith('mount-'):
                    if variant=='mount-duplicate':f['Mounts'].append(copy.deepcopy(f['Mounts'][0]))
                    else:f['Mounts'][0][{'mount-type':'Type','mount-name':'Name','mount-source':'Source','mount-rw':'RW'}[variant]]=False if variant=='mount-rw' else 'foreign'
                elif variant=='nocopy':f['HostConfig']['Mounts'][0]['VolumeOptions']['NoCopy']=False
                elif variant=='nocopy-type':f['HostConfig']['Mounts'][0]['VolumeOptions']['NoCopy']=1
                elif variant=='declared-source':f['HostConfig']['Mounts'][0]['Source']='old-volume'
                elif variant=='declared-rw':f['HostConfig']['Mounts'][0]['ReadOnly']=True
                with self.assertRaises(h.full.FullImportError):h.configure()
                self.assertEqual(h.actions,[]);self.assertEqual(h.writes,{})
                self.assertIsNone(c._target_status);self.assertIsNone(c._profile)
                if variant in ('cid','owner-id','owner-state','owner-ledger','already-born'):self.assertEqual(h.calls,[])

    def test_fixed_shell_rejects_bad_paths_metadata_and_exec_before_publication(self):
        early={'parent-owner','parent-write','parent-symlink','parent-stat-failure','run-alias','symlink','non-dir','socket-owner','socket-group','changed-open'}
        for scenario in sorted(early|{'changed-path','chown-failure','chmod-failure','readback-mode','readback-owner','readback-group','readback-type','exec-failure','receipt-missing','receipt-malformed','mount-drift','restart'}):
            with self.subTest(scenario=scenario):
                h=_SocketProfileHarness(self,scenario=scenario)
                with self.assertRaises((h.full.FullImportError,h.admission.GateError)):h.configure()
                self.assertIsNone(h.context._profile);self.assertIsNone(h.context._target_status)
                self.assertEqual(h.writes,{})
                if scenario in early:self.assertEqual(h.actions,[])

    def test_recipe_io_cannot_bypass_deadline_or_raw_log_capacity(self):
        for variant in ('deadline','logs'):
            h=_SocketProfileHarness(self,scenario=variant)
            if variant=='deadline':h.budget.deadline=100
            else:h.driver.LOG_RESERVATIONS[str(h.base.logs)]['pending']=8192
            with self.assertRaises(h.driver.GateError):h.configure()
            self.assertEqual(h.calls,[]);self.assertEqual(h.actions,[]);self.assertEqual(h.writes,{})

    def test_finalize_metadata_failures_have_fixed_nonsecret_stage(self):
        for path,stage in [('/usr/bin/docker','DOCKER_METADATA'),('/private/full/'+S+'/full-profile/initdb.sh','INITDB_METADATA'),('/private/full/'+S,'SOURCE_ROOT_METADATA'),('/var/run/docker.sock','DOCKER_SOCKET_METADATA')]:
            h=_SocketProfileHarness(self,socket_mode=0o3775)
            with self.assertRaises(OSError):h.finalize(metadata_failure=path)
            self.assertEqual(h.record.get('full_profile_stage'),stage)
            self.assertNotIn('PRIVATE-',str(h.record));self.assertEqual(h.writes,{})
            self.assertIsNone(h.context._profile)


if __name__=='__main__':unittest.main()
