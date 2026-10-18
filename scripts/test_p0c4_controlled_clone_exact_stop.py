"""Local synthetic RPCs through actual stop/helper/owner consumers; no Docker/PG."""
import copy
import json
import os
import stat
import sys
import tempfile
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import p0c4_controlled_import_acceptance as runner
import p0c4_source_lifecycle_gate_acceptance as driver
from p0c4_completion import controlled_clone as clone, controlled_fixture as controlled, full_target_fs as fs
import test_p0c4_controlled_exact_stop as ordinary
import test_p0c4_controlled_fixture_context as context_tests


class CloneExactStopTests(unittest.TestCase):
    _install_recheck=ordinary.ExactStopTests._install_recheck

    def transport_run(self,command,**kwargs):
        answer=ordinary.ExactStopTests.transport_run(self,command,**kwargs)
        if command[1]=='stop' and command[-1] in ('a'*64,'b'*64,'e'*64) and self.stop_result is not None:return self.stop_result
        return answer

    def setUp(self):
        ordinary.ExactStopTests.setUp(self)
        c=self.context;b=self.backend
        plan=context_tests.ContextPlanTests().plan();plan.update(c._plan)
        plan.update(case='wrong-endpoint',capability=clone._CAPABILITY,budget_profile=clone._PROFILE,
                    clone_case_id='00000000-0000-4000-8000-000000000004',clone_subnet='172.30.243.0/24')
        c._plan.clear();c._plan.update(plan)
        self.install.plan_pin={'sha256':clone._digest(clone._canonical(plan))}
        self.events=[];self.release_fault=None;self.cleanup_fault=None;self.stop_result=None;self.rpc_times=[]
        self.owner_helpers={};self.roots={}
        lane=clone._CloneLane(c);c._clone=lane;self.lane=lane
        lane.state='born';lane.deadline=None
        target=c._sides['target'];lane.target_id=target['ids']['container_id']
        lane.target_started_at=lane.target_owner_started='2026-10-18T00:00:00Z'
        target['owner_root']=self.root('/private/target-control',51)
        helper,broker=self.owner_transport('target','8'*64)
        lane.target_owner=fs._OwnerClient(fs._TOKEN,helper,broker);lane.target_owner._state='birth';target['owner']=lane.target_owner
        lane.identity=b.provisioner.identity_for(plan['clone_case_id'])
        lane.ids=dict(container_id='e'*64,network_id='f'*64,volume_name=lane.identity['volume'],
                      volume_mountpoint='/var/lib/docker/volumes/'+lane.identity['volume']+'/_data',daemon_id='original-daemon')
        lane.root=self.root('/private/controlled-import/batches/'+plan['batch_id']+'/clone-control',52)
        helper,broker=self.owner_transport('clone','9'*64)
        lane.owner=clone._RootClient(helper,broker);lane.owner.state='birth';lane.owner_started=lane.target_owner_started
        lane.started_at='2026-10-18T00:00:00Z';lane.image={'Id':'sha256:'+'c'*64}
        lane.sql_identity=dict(database=target['identity']['database'],pg_system_identifier='7555512345678901234',database_oid=16384)
        lane.compose=clone._canonical(clone._compose_document(lane.identity,plan['clone_subnet']))
        lane.helper_proofs=[dict(kind=k,container_id=v*64,stdout_sha256='1'*64,stderr_sha256=clone._digest(b''),exit_code=0) for k,v in zip(('setup','copy','verify'),('5','6','7'))]
        lane.measurement=dict(identity=dict(path=lane.ids['volume_mountpoint'],dev=10,ino=23,uid=0,gid=0,mode=0o755),
                              pgdata_identity=dict(path=lane.ids['volume_mountpoint']+'/18/docker',dev=10,ino=24,uid=999,gid=999,mode=0o700),tree_sha256='d'*64,entries=10,bytes=100)
        # Independent native roots: same SQL identity never aliases physical resources.
        c._sides['target']['record']['pg_root_measurement']['roots']['target_pg']['identity']['ino']=21
        for role in ('source','target'):
            side=c._sides[role];p=side['root']/'targets'/side['case_id']
            raw=clone._canonical(dict(database_name=side['identity']['database'],pg_system_identifier='7555512345678901234',database_oid=16384))
            pin=clone._digest(raw);side['status']['birth_sha256']=side['record']['birth_sha256']=c._compiled['live'][role+'_birth_sha256']=pin
            self.files[str(p/'control'/(side['identity']['database']+'.birth.json'))]=raw
            evidence=json.loads(self.files[str(p/'birth-evidence.json')]);evidence.update(birth_sha256=pin,volume_mount_ino=20 if role=='source' else 21)
            self.files[str(p/'birth-evidence.json')]=clone._canonical(evidence)
            self.containers[side['ids']['container_id']]['State'].update(Status='exited',ExitCode=0,OOMKilled=False,Dead=False,Error='')
        lane.row=dict(role='clone',identity=lane.identity,control=b.batch/'clone-control',batch_id=plan['clone_case_id'],subnet=plan['clone_subnet'],container_id=lane.ids['container_id'],before={'daemon_id':'original-daemon'})
        expected=dict(name=lane.identity['project']+'-pg-1',labels={'com.docker.compose.project':lane.identity['project'],'com.docker.compose.service':'pg'},clone_resources=dict(network=lane.identity['network'],volume=lane.identity['volume'],subnet=plan['clone_subnet'],compose_sha256=clone._digest(lane.compose)))
        lane.creation=dict(intent=dict(kind='pg',name=expected['name'],case_id=plan['source_case_id'],batch_id=plan['batch_id'],expected=expected,network={'name':lane.identity['network']}),state='known',id=lane.ids['container_id'],record=lane.row)
        c._runner.ownership.owners[expected['name']]=lane.creation;b.resources.append(lane.row);b.clone_record=lane.row
        lane.birth=dict(format_version=1,capability='c4_controlled_clone_birth_v1',plan_sha256=self.install.plan_pin['sha256'],clone_case_id=plan['clone_case_id'],
                        source_birth_sha256=c._sides['source']['status']['birth_sha256'],target_birth_sha256=target['status']['birth_sha256'],root=lane.root,ids=lane.ids,image_id=lane.image['Id'],
                        started_at=lane.started_at,target_started_at=lane.target_started_at,restart_count=0,sql_identity=lane.sql_identity,compose_sha256=clone._digest(lane.compose),pg_measurement=lane.measurement,
                        helpers=lane.helper_proofs,passfile_state='NOT_CREATED',replication_auth='EXACT_LOOPBACK_TRUST',no_standby=True)
        clone._validate_birth(lane.birth,plan)
        lane.birth_sha256=clone._digest(clone._canonical(lane.birth));lane.row.update(birth_sha256=lane.birth_sha256,inspection_sha256=lane.birth_sha256)
        self.files[str(b.batch/'clone-control/birth.json')]=clone._canonical(lane.birth)
        self.files[str(b.batch/'clone-control/compose.json')]=lane.compose
        self.containers['e'*64]=dict(Id='e'*64,Name='/'+lane.identity['project']+'-pg-1',Image=lane.image['Id'],Config=dict(Image=lane.identity['image'],Labels=expected['labels'].copy(),Env=['SECRET_SENTINEL=never-log']),
            Mounts=[dict(Type='volume',Name=lane.identity['volume'],Source=lane.ids['volume_mountpoint'],Destination='/var/lib/postgresql',RW=True)],State=dict(Running=False,Pid=0,Status='exited',ExitCode=0,StartedAt=lane.started_at,OOMKilled=False,Dead=False,Error=''),ExecIDs=None,RestartCount=0)
        self.volumes[lane.identity['volume']]=dict(Name=lane.identity['volume'],Mountpoint=lane.ids['volume_mountpoint'],Driver='local',Scope='local',Options=None,Labels={'com.docker.compose.project':lane.identity['project']},CreatedAt=lane.started_at)
        c._runner.logs=Path('/private/logs');c._runner.counter=0
        c._runner.inspect=lambda kind,identifier:json.loads(self.docker(kind,'inspect',identifier,timeout=5))[0]
        private=patch.object(controlled,'_private_bytes',side_effect=lambda p,*args:(self.files[str(p)],{'sha256':clone._digest(self.files[str(p)])}));private.start();self.addCleanup(private.stop)
        lstat=os.lstat
        native=patch.object(clone.os,'lstat',side_effect=lambda path,*a,**k:self.roots[str(path)] if str(path) in self.roots else lstat(path,*a,**k));native.start();self.addCleanup(native.stop)

    def root(self,path,ino):
        self.roots[path]=SimpleNamespace(st_mode=stat.S_IFDIR|0o700,st_dev=10,st_ino=ino,st_uid=0,st_gid=0)
        return dict(path=path,dev=10,ino=ino,uid=0,gid=0,mode=0o700)

    def owner_transport(self,role,cid):
        fixture=self
        class Helper:
            closed=False
            def __init__(self):self.runner=fixture.context._runner;self.record={'id':cid}
            def inspect(self,runner=None):
                return (runner or self.runner).inspect('container',cid)
            def close(self):
                fixture.events.append(role+'-close')
                if getattr(self,'real_close',False):return driver.HelperContainer.close(self)
                self.runner.docker('stop','--timeout','5',cid,timeout=10)
                self.runner.docker('rm',cid,timeout=10)
                self.runner.docker('ps','-aq','--no-trunc',timeout=30)
                if fixture.cleanup_fault==role:raise RuntimeError('helper cleanup failed')
                self.closed=True
        helper=Helper();self.owner_helpers[cid]=helper
        budget=driver.CaseBudget();budget.remaining=lambda:1000;budget.refresh_deadline=None
        class Broker:
            broken=False
            child=SimpleNamespace(check=lambda:None,p=SimpleNamespace(poll=lambda:None),finished=False,retired=False)
            def call(self,request):
                fixture.events.append(role+'-'+request['op'])
                if fixture.release_fault==role:raise TimeoutError('release ack lost')
                if fixture.release_fault==role+'-ack':value={'released':False}
                elif fixture.release_fault==role+'-integer':value={'released':1}
                else:value={'released':True}
                return dict(**{k:request[k] for k in ('session','seq','op')},ok=True,value=value)
        broker=Broker();broker.budget=budget
        self.containers[cid]=dict(Id=cid,State=dict(Running=True,StartedAt='2026-10-18T00:00:00Z'),RestartCount=0)
        return helper,broker

    def docker(self,*args,**kwargs):
        self.rpc_times.append((self.clock[0],kwargs['timeout']))
        if args[:2]==('container','inspect'):args=('inspect',*args[2:])
        if args[0]=='stop' and args[-1] in ('a'*64,'b'*64,'e'*64):self.containers[args[-1]]['State']['Status']='exited'
        if args[0]=='stop' and args[-1] in self.owner_helpers or args[0]=='rm':
            self.calls.append((args,kwargs,self.backend.commands.isolation_deadline));self.clock[0]+=0.1
            if args[0]=='stop':self.containers[args[-1]]['State'].update(Running=False,Pid=0)
            return (args[-1]+'\n').encode()
        if args[:2]==('network','inspect'):
            self.calls.append((args,kwargs,self.backend.commands.isolation_deadline));self.clock[0]+=0.1
            value=dict(Id=args[-1],Containers={})
            return (json.dumps(value)+'\n').encode() if '--format' in args else json.dumps([value]).encode()
        return ordinary.ExactStopTests.docker(self,*args,**kwargs)

    def fail_stop(self):
        owner=self.context._sides['target']['owner']
        with self.assertRaises(runner.ImportRejected):self.backend.stop()
        self.assertTrue(self.context._native_broken);self.assertEqual(self.lane.state,'unknown');self.assertFalse(self.lane.isolated)
        self.assertIs(self.context._sides['target']['owner'],owner)

    def test_postbody_actual_stop_ignores_foreign_inventory(self):
        self.containers['a'*64]['State'].update(Running=True,Pid=123,Status='running')
        error=None
        try:result=self.backend.stop()
        except BaseException as e:error=e;result=None
        self.assertIsNone(error,f'actual typed clone stop failed: {error}; inventory_timeout={self.inventory_timeout}; calls={len(self.calls)}')
        self.assertEqual(result,dict(source_stopped=True,target_stopped=True,clone_stopped=True,volumes_retained=True))
        self.assertEqual([a for a,_,_ in self.calls if a[0]=='stop' and a[-1] in ('a'*64,'b'*64,'e'*64)],[('stop','--timeout','1','a'*64)])
        self.assertEqual(self.events,['clone-release','clone-close','target-release','target-close'])
        self.assertFalse(self.inventory_timeout)

    def test_original_target_release_ack_failure_never_enters_close(self):
        self.release_fault='target'
        # Exercise actual confirmation and original _OwnerClient.close consumer.
        self.backend.commands.isolation_deadline=115.0
        error=None
        try:self.lane.confirm_isolated()
        except BaseException as e:error=e
        self.assertIsNotNone(error)
        self.assertFalse(self.lane.target_owner._container.closed,'lost release ack must HOLD the original target helper')
        self.assertNotIn('target-close',self.events)
        self.assertFalse(self.lane.isolated)

    def test_confirmation_explicit_rpc_timeouts_are_capped_at_five(self):
        self.backend.commands.isolation_deadline=115.0
        self.lane.confirm_isolated()
        self.assertTrue(self.calls)
        self.assertTrue(all(0<kw['timeout']<=5 for _,kw,_ in self.calls),self.calls)

    def fresh_mutations(self,mutations):
        for name,mutate in mutations:
            with self.subTest(name=name):
                self.doCleanups();case=CloneExactStopTests();case.setUp()
                try:
                    mutate(case)
                    with patch.object(runner.ContractBackend,'stop',side_effect=AssertionError('postbody fallback forbidden')):case.fail_stop()
                    self.assertFalse(any(a[0]=='stop' for a,_,_ in case.calls),case.calls)
                    self.assertEqual(case.events,[])
                finally:case.doCleanups()

    def test_role_row_registry_and_original_owner_contradictions_fail_before_mutation(self):
        self.fresh_mutations([
            ('role',lambda t:t.lane.row.update(role='target')),
            ('duplicate row',lambda t:t.backend.resources.append(t.lane.row)),
            ('missing row',lambda t:t.backend.resources.remove(t.lane.row)),
            ('clone row replacement',lambda t:setattr(t.backend,'clone_record',copy.deepcopy(t.lane.row))),
            ('clone registry replacement',lambda t:t.context._runner.ownership.owners.update({t.lane.creation['intent']['name']:copy.deepcopy(t.lane.creation)})),
            ('ordinary registry replacement',lambda t:t.context._runner.ownership.owners.update({t.context._sides['source']['creation']['intent']['name']:copy.deepcopy(t.context._sides['source']['creation'])})),
            ('target owner replacement',lambda t:t.context._sides['target'].update(owner=fs._OwnerClient(fs._TOKEN,t.lane.target_owner._container,t.lane.target_owner._broker))),
            ('source owner revived',lambda t:t.context._sides['source'].update(owner=t.lane.target_owner)),
            ('clone owner failed',lambda t:setattr(t.lane.owner,'failed',True)),
            ('clone broker broken',lambda t:setattr(t.lane.owner.broker,'broken',True)),
            ('target broker broken',lambda t:setattr(t.lane.target_owner._broker,'broken',True)),
            ('target root inode',lambda t:setattr(t.roots[t.context._sides['target']['owner_root']['path']],'st_ino',999)),
            ('clone root inode',lambda t:setattr(t.roots[t.lane.root['path']],'st_ino',999)),
            ('clone helper restart',lambda t:t.containers['9'*64]['State'].update(StartedAt='restarted')),
            ('target helper restart',lambda t:t.containers['8'*64]['State'].update(StartedAt='restarted')),
            ('original install changed',lambda t:setattr(t,'install_fault',True)),
            ('resource creation unknown',lambda t:t.context._result.update(resource_creation_unknown=True)),
        ])

    def test_same_oid_never_authorizes_clone_birth_or_physical_identity_replacement(self):
        self.fresh_mutations([
            ('CID alias',lambda t:t.lane.ids.update(container_id='b'*64)),
            ('volume alias',lambda t:t.lane.ids.update(volume_name=t.context._sides['target']['ids']['volume_name'])),
            ('network alias',lambda t:t.lane.ids.update(network_id='b'*64)),
            ('native inode alias',lambda t:t.lane.measurement['identity'].update(ino=21)),
            ('native root binding',lambda t:t.lane.measurement['identity'].update(path='/replacement')),
            ('birth pin',lambda t:setattr(t.lane,'birth_sha256','0'*64)),
            ('copy proof',lambda t:t.lane.helper_proofs[1].update(exit_code=1)),
            ('copy proof kind',lambda t:t.lane.helper_proofs[1].update(kind='setup')),
            ('copy proof container',lambda t:t.lane.helper_proofs[1].update(container_id='e'*64)),
            ('target birth',lambda t:t.lane.birth.update(target_birth_sha256='1'*64)),
            ('SQL OID',lambda t:t.lane.sql_identity.update(database_oid=16385)),
            ('compose',lambda t:setattr(t.lane,'compose',b'{}')),
            ('clone birth bytes',lambda t:t.files.update({str(t.backend.batch/'clone-control/birth.json'):b'{}'})),
            ('target volume birth measurement',lambda t:t.context._sides['target']['record']['pg_root_measurement']['roots']['target_pg']['identity'].update(ino=999)),
        ])

    def test_object_and_transport_replacements_during_rpc_abort_without_next_rpc(self):
        mutations=[
            lambda t:t.context._sides['target'].update(record=copy.deepcopy(t.context._sides['target']['record'])),
            lambda t:setattr(t.lane,'owner',clone._RootClient(t.lane.owner.container,t.lane.owner.broker)),
            lambda t:setattr(t.lane.owner,'broker',copy.copy(t.lane.owner.broker)),
            lambda t:setattr(t.lane.owner.broker,'child',copy.copy(t.lane.owner.broker.child)),
            lambda t:setattr(t.context,'_runner',SimpleNamespace(ownership=t.context._runner.ownership,run=t.context._runner.run)),
            lambda t:setattr(t.backend.commands,'isolation_deadline',116.0),
            lambda t:setattr(t.lane,'birth',copy.deepcopy(t.lane.birth)),
            lambda t:setattr(t.backend,'commands',copy.copy(t.backend.commands)),
        ]
        for index,mutation in enumerate(mutations):
            with self.subTest(index=index):
                self.doCleanups();case=CloneExactStopTests();case.setUp()
                try:
                    case.hook=lambda args:mutation(case) if len(case.calls)==1 else None
                    case.fail_stop();self.assertEqual(len(case.calls),1)
                finally:case.doCleanups()

    def test_three_running_roles_stop_same_exact_id_once_with_fresh_post_reads(self):
        for cid in ('a'*64,'b'*64,'e'*64):self.containers[cid]['State'].update(Running=True,Pid=123,Status='running')
        self.assertTrue(self.backend.stop()['clone_stopped'])
        self.assertEqual([a for a,_,_ in self.calls if a[0]=='stop' and a[-1] in ('a'*64,'b'*64,'e'*64)],
                         [('stop','--timeout','1',v*64) for v in ('a','b','e')])
        for cid in ('a'*64,'b'*64,'e'*64):
            self.assertGreaterEqual(sum(a[0]=='inspect' and a[-1]==cid for a,_,_ in self.calls),2)
        self.assertTrue(all(d==115.0 for _,_,d in self.calls))
        self.assertTrue(all(0<timeout<=min(5,115-started) for started,timeout in self.rpc_times))

    def test_stop_warning_nonzero_unknown_and_unexpected_stdout_fail_without_retry(self):
        for answer in [(0,b'',b'deprecated warning'),(1,b'',b''),(None,b'',b''),(0,b'foreign-id\n',b''),(0,('a'*64+'\n\n').encode(),b'')]:
            with self.subTest(answer=answer):
                self.doCleanups();case=CloneExactStopTests();case.setUp()
                try:
                    case.stop_result=answer;case.containers['a'*64]['State'].update(Running=True,Pid=123,Status='running')
                    case.fail_stop();self.assertEqual(sum(a[0]=='stop' for a,_,_ in case.calls),1)
                    count=len(case.calls)
                    with self.assertRaises(runner.ImportRejected):case.backend.stop()
                    self.assertEqual(len(case.calls),count);self.assertEqual(case.events,[])
                finally:case.doCleanups()

    def test_stop_helper_extra_argv_wrong_id_or_replay_is_rejected(self):
        for args in [('stop','--time','1','b'*64),('stop','--time','1','a'*64,'extra'),('stop','--timeout','1','a'*64)]:
            with self.subTest(args=args):
                self.doCleanups();case=CloneExactStopTests();case.setUp()
                try:
                    case.containers['a'*64]['State'].update(Running=True,Pid=123,Status='running')
                    def attack(view,identity,cid):view.snapshot();view._docker(*args)
                    case.backend.helper.stop_verified_pg=attack;case.fail_stop()
                    self.assertFalse(any(a[0]=='stop' for a,_,_ in case.calls))
                finally:case.doCleanups()

    def test_original_deadline_expiry_at_every_rpc_boundary_stops_advancement(self):
        # Positive trace has 33 RPCs with two helper cleanup inventories.
        self.backend.stop();count=len(self.calls)
        for boundary in range(count+1):
            with self.subTest(boundary=boundary):
                self.doCleanups();case=CloneExactStopTests();case.setUp()
                try:
                    case.backend.commands.isolation_deadline=115.0
                    if boundary==0:case.clock[0]=115.0
                    else:
                        base=case.context._runner.run
                        def expire(command,_base=base,**kwargs):
                            result=_base(command,**kwargs)
                            if len(case.calls)==boundary:case.clock[0]=115.0
                            return result
                        case.context._runner.run=expire
                    case.fail_stop();self.assertEqual(len(case.calls),boundary)
                finally:case.doCleanups()

    def test_unsettled_or_unknown_transport_never_advances_to_next_case_or_release(self):
        self.backend.commands.unusable=True;self.fail_stop();self.assertEqual(self.calls,[])
        count=len(self.calls)
        with self.assertRaises(runner.ImportRejected):self.backend.stop()
        with patch.object(driver,'cleanup_known_helpers',return_value=[]):
            with self.assertRaises(runner.ImportRejected):self.backend.release()
        self.assertEqual(len(self.calls),count);self.assertFalse(self.context._closed);os.fstat(self.fd)
        self.assertEqual(self.events,[])

    def test_partial_handoff_ack_and_cleanup_failures_hold_remaining_original_owners(self):
        for release,cleanup,events in [('clone',None,['clone-release']),('clone-ack',None,['clone-release']),
                                       (None,'clone',['clone-release','clone-close']),('target',None,['clone-release','clone-close','target-release']),
                                       ('target-ack',None,['clone-release','clone-close','target-release']),
                                       (None,'target',['clone-release','clone-close','target-release','target-close'])]:
            with self.subTest(release=release,cleanup=cleanup):
                self.doCleanups();case=CloneExactStopTests();case.setUp()
                try:
                    case.release_fault=release;case.cleanup_fault=cleanup;case.fail_stop()
                    self.assertEqual(case.events,events)
                    if cleanup!='target':self.assertFalse(case.lane.target_owner._container.closed)
                    count=len(case.calls)
                    with self.assertRaises(runner.ImportRejected):case.backend.stop()
                    self.assertEqual(len(case.calls),count)
                finally:case.doCleanups()

    def test_prebody_keeps_original_conservative_fallback(self):
        self.context._body_used=False;expected={'source_stopped':True,'target_stopped':True,'clone_stopped':True,'volumes_retained':True}
        with patch.object(runner.ContractBackend,'stop',return_value=expected) as fallback,patch.object(clone._CloneLane,'confirm_isolated'):
            self.assertIs(self.backend.stop(),expected)
        fallback.assert_called_once_with();self.assertEqual(self.calls,[])

    def test_postbody_missing_lane_has_no_fallback(self):
        self.context._clone=None
        with patch.object(runner.ContractBackend,'stop',side_effect=AssertionError('forbidden fallback')):
            with self.assertRaises(runner.ImportRejected):self.backend.stop()
        self.assertTrue(self.context._native_broken);self.assertEqual(self.calls,[])

    def test_reentry_after_success_has_no_io_and_does_not_reacquire_released_owners(self):
        self.backend.stop();count=len(self.calls);events=list(self.events)
        with self.assertRaises(runner.ImportRejected):self.backend.stop()
        self.assertEqual(len(self.calls),count);self.assertEqual(self.events,events)
        self.assertIsNone(self.context._sides['target']['owner'])

    def test_clone_release_integer_ack_does_not_authorize_helper_close(self):
        self.release_fault='clone-integer';self.fail_stop()
        self.assertEqual(self.events,['clone-release'])
        self.assertFalse(self.lane.owner.container.closed)

    def test_original_broker_budget_is_not_extended_by_isolation_scope(self):
        budget=self.lane.owner.broker.budget;del budget.remaining
        budget.clock=lambda:self.clock[0];budget.deadline=100.15
        self.fail_stop()
        self.assertLessEqual(len(self.calls),2)
        self.assertTrue(all(timeout<=100.15-started for started,timeout in self.rpc_times))

    def enable_real_helper_cleanup(self):
        self.context._result['batch_id']=self.context._plan['batch_id']
        directory=tempfile.TemporaryDirectory(prefix='clone-real-close-');self.addCleanup(directory.cleanup)
        base=self.context._runner;base.logs=Path(directory.name);base.next_log_token=None
        state=dict(archives=0,ledgers=0,pending=0,audits=0);driver.LOG_RESERVATIONS[str(base.logs)]=state
        self.addCleanup(driver.LOG_RESERVATIONS.pop,str(base.logs),None)
        registry=driver.CleanupReservations(base,self.context._result);registry.case_id=self.context._plan['source_case_id'];registry._controlled_pair=self.context
        registry.owners=base.ownership.owners;base.ownership=registry
        for role,cid in (('target','8'*64),('clone','9'*64)):
            helper=self.owner_helpers[cid];helper.real_close=True;helper.child=None
            helper.record.update(removed=False,name='original-'+role)
            helper.owner=registry.acquire('helper','original-'+role,dict(stdin=True),None,helper.record)
            registry.known(helper.owner,cid);self.context._result['helpers'].append(helper.record)
        original_run=base.run
        def receipted(command,**kwargs):
            prefix=driver.begin_log_rpc(base);base.counter+=1
            code,out,err=original_run(command,**kwargs)
            for suffix,raw in [('stdout',out),('stderr',err),('process.json',driver.canonical(dict(exit_code=code,reason=None,stdout_bytes=len(out),stderr_bytes=len(err))))]:
                (base.logs/(prefix+'.'+suffix)).write_bytes(raw)
            driver.end_log_rpc(base,prefix)
            return code,out,err
        base.run=receipted

    def test_actual_helper_cleanup_consumers_keep_receipts_and_same_original_deadline(self):
        self.enable_real_helper_cleanup();self.backend.stop()
        self.assertEqual(self.events,['clone-release','clone-close','target-release','target-close'])
        for helper in self.owner_helpers.values():
            self.assertTrue(helper.closed);self.assertEqual(helper.owner['state'],'absent')
            self.assertEqual(len(helper.record['cleanup_rpcs']['close']),5)
        self.assertTrue(all(d==115 for _,_,d in self.calls))
        self.assertTrue(all(0<timeout<=min(5,115-started) for started,timeout in self.rpc_times))

    def test_real_compose_extra_labels_are_not_mistaken_for_foreign_clone(self):
        self.containers['e'*64]['Config']['Labels'].update({'com.docker.compose.config-hash':'2'*64,'com.docker.compose.container-number':'1'})
        self.volumes[self.lane.ids['volume_name']]['Labels'].update({'com.docker.compose.volume':'pg','com.docker.compose.version':'2.39.1'})
        self.assertTrue(self.backend.stop()['clone_stopped'])

    def test_original_remaining_target_custody_is_checked_after_clone_handoff(self):
        original_run=self.context._runner.run
        def replace(command,**kwargs):
            answer=original_run(command,**kwargs)
            if command[1:]==['rm','9'*64]:self.lane.target_owner._broker=copy.copy(self.lane.target_owner._broker)
            return answer
        self.context._runner.run=replace
        self.fail_stop()
        self.assertNotIn('target-release',self.events);self.assertNotIn('target-close',self.events)


@unittest.skipUnless(sys.platform=='linux' and getattr(os,'geteuid',lambda:1)()==0,'Linux root FD/flock test; no Docker/PG')
class NativeCloneOriginalRootTests(unittest.TestCase):
    def owner(self,base):
        import p0c4_source_admission_gate_acceptance as admission
        owner=object.__new__(clone._CloneRootOwner)
        owner.plan=context_tests.ContextPlanTests().plan();owner.plan.update(clone_case_id='44444444-4444-4444-8444-444444444444',clone_subnet='172.30.243.0/24')
        owner.suite=base/'suite';owner.source=base/'source';owner.root=base/'clone-control'
        owner.pin={'sha256':'1'*64};owner.record={};owner.install_pin={};owner.manifest={};owner.source_hash='2'*64
        owner.fd=owner.lock=owner.identity=None;owner.released=False
        for context in (patch.object(controlled,'_read_plan',return_value=(owner.plan,owner.pin)),
                        patch.object(controlled,'_read_install',return_value=(owner.record,owner.install_pin)),
                        patch.object(admission,'source_digest',return_value=owner.source_hash)):
            context.start();self.addCleanup(context.stop)
        owner.perform('setup',{})
        return owner

    def test_original_clone_root_fd_and_lock_are_held_until_original_release(self):
        import fcntl
        with tempfile.TemporaryDirectory(prefix='clone-original-lock-',dir='/root') as directory:
            owner=self.owner(Path(directory));original=owner.fd;meta=os.fstat(original)
            contender=os.open(owner.root/'.restore-target.lock',os.O_RDWR)
            try:
                owner.recheck();self.assertEqual((os.fstat(original).st_dev,os.fstat(original).st_ino),(meta.st_dev,meta.st_ino))
                with self.assertRaises(BlockingIOError):fcntl.flock(contender,fcntl.LOCK_EX|fcntl.LOCK_NB)
                self.assertEqual(owner.perform('release',{}),{'released':True})
                self.assertIsNone(owner.fd);self.assertIsNone(owner.lock)
                with self.assertRaises(OSError):os.fstat(original)
                fcntl.flock(contender,fcntl.LOCK_EX|fcntl.LOCK_NB)
            finally:
                os.close(contender)
                if owner.fd is not None:os.close(owner.fd);owner.fd=None;owner.lock.__exit__(None,None,None)

    def test_original_clone_root_replacement_rejects_release_and_preserves_old_fd_lock(self):
        import fcntl
        with tempfile.TemporaryDirectory(prefix='clone-original-replaced-',dir='/root') as directory:
            base=Path(directory);owner=self.owner(base);original=owner.fd;ino=os.fstat(original).st_ino
            contender=os.open(owner.root/'.restore-target.lock',os.O_RDWR)
            try:
                owner.root.rename(base/'held-original');owner.root.mkdir(mode=0o700)
                with self.assertRaises(clone.CloneRejected):owner.perform('release',{})
                self.assertEqual(owner.fd,original);self.assertEqual(os.fstat(original).st_ino,ino);self.assertFalse(owner.released)
                with self.assertRaises(BlockingIOError):fcntl.flock(contender,fcntl.LOCK_EX|fcntl.LOCK_NB)
            finally:
                os.close(contender);os.close(original);owner.fd=None;owner.lock.__exit__(None,None,None)


if __name__=='__main__':unittest.main()
