"""Synthetic transport tests: actual controlled stop consumer, never Docker/PG."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import p0c4_controlled_import_acceptance as runner
import p0c4_restore_birth_acceptance as birth
import p0c4_restore_target as target
import p0c4_import_fixture as clients
from p0c4_completion import controlled_fixture as controlled


class ExactStopTests(unittest.TestCase):
    def setUp(self):
        fixture_patch=patch.object(runner,'fixture',clients)
        fixture_patch.start();self.addCleanup(fixture_patch.stop)
        self.clock = [100.0]
        self.calls = []
        self.files = {}
        self.containers = {}
        self.volumes = {}
        self.hook = None
        self.rpc_error = None
        self.install_fault = False
        self.inventory = 2000
        self.inventory_timeout = False
        self.fd, name = tempfile.mkstemp()
        self.addCleanup(os.unlink, name)
        self.addCleanup(lambda:os.close(self.fd) if self.install.suite_fd is not None else None)
        self.backend = runner.ImportBackend.__new__(runner.ImportBackend)
        backend = self.backend
        backend.commands = runner.BoundedCommands()
        backend.clone_record = None
        backend.old_temp = {}
        backend.lock = SimpleNamespace(__exit__=lambda *args:None)
        backend.resources = []
        backend.batch = Path('/private/case')
        backend.initdb = Path('/private/initdb')
        backend.helper = SimpleNamespace(stop_verified_pg=birth.stop_verified_pg,
            _private_read=lambda path, limit=4096: self.files[str(path)])
        backend.provisioner = SimpleNamespace(identity_for=target.identity_for)
        self.context = context = controlled.ControlledFixtureContext.__new__(controlled.ControlledFixtureContext)
        backend._controlled_context = context
        context._backend = backend
        context._plan = dict(case='success',batch_id='00000000-0000-4000-8000-000000000003',
            source_case_id='00000000-0000-4000-8000-000000000001',
            target_case_id='00000000-0000-4000-8000-000000000002',
            source_subnet='172.30.241.0/24',target_subnet='172.30.242.0/24')
        self.install = install = controlled._VerifiedInstall.__new__(controlled._VerifiedInstall)
        install.backend = backend
        install.plan = context._plan
        install.suite_fd = self.fd
        install.source_fd = None
        context._install = install
        context._closed = context._native_broken = False
        context._body_used = True
        context._clone = None
        context._cache_fd = None
        context._native_helpers = []
        context._observation_deadline = None
        context._runtime_admission = controlled._RUNTIME_ADMITTED
        context._result = dict(resource_creation_unknown=False, helpers=[])
        context._sides = {}
        context._compiled = dict(live={})
        owners = {}
        context._runner = SimpleNamespace(docker=self.docker, run=self.transport_run, ownership=SimpleNamespace(
            owners=owners,children=[],case_id=context._plan['source_case_id'],_controlled_pair=context))
        for role, letter in (('source','a'),('target','b')):
            case_id = context._plan[role+'_case_id']
            identity = target.identity_for(case_id)
            root = backend.batch/(role+'-control')
            cid = letter*64
            volume = dict(Name=identity['volume'],Mountpoint='/var/lib/docker/volumes/'+identity['volume']+'/_data',
                Driver='local',Scope='local',Options=None,Labels={'com.docker.compose.project':identity['project']},CreatedAt='2026-10-18T00:00:00Z')
            mounts = [dict(Type='volume',Name=volume['Name'],Source=volume['Mountpoint'],Destination='/var/lib/postgresql',RW=True)]
            dest = root/'targets'/case_id
            mounts += [dict(Type='bind',Source=str(path),Destination=at,RW=False) for path,at in (
                (backend.initdb,'/docker-entrypoint-initdb.d/10-restore.sh'),
                (dest/'secrets/postgres_password','/run/secrets/postgres_password'),
                (dest/'secrets/admin_password','/run/secrets/admin_password'))]
            pg = dict(Id=cid,Name='/'+identity['project']+'-pg-1',Image='sha256:'+'c'*64,
                Config=dict(Image=identity['image'],Labels={'com.docker.compose.project':identity['project'],'com.docker.compose.service':'pg'},Env=['SECRET_SENTINEL=never-log']),
                Mounts=mounts,State=dict(Running=False,Pid=0,StartedAt='2026-10-18T00:00:00Z'),ExecIDs=None,RestartCount=0)
            self.containers[cid]=pg;self.volumes[volume['Name']]=volume
            raw = json.dumps({'private-birth':role}).encode()
            pin = hashlib.sha256(raw).hexdigest()
            ids = dict(container_id=cid,network_id=letter*64,volume_name=volume['Name'],volume_mountpoint=volume['Mountpoint'])
            status = dict(**ids,batch_id=case_id,project=identity['project'],database=identity['database'],network=identity['network'],subnet=context._plan[role+'_subnet'],birth_sha256=pin)
            measurement=dict(phase='birth',source_case_id=context._plan[('target' if role=='source' else 'source')+'_case_id'],target_case_id=case_id,roots={'target_pg':dict(identity=dict(path=volume['Mountpoint'],dev=10,ino=20,uid=0,gid=0,mode=0o755),tree_sha256='d'*64,entries=10,bytes=100)})
            row = dict(role=role,identity=identity,control=root,batch_id=case_id,subnet=context._plan[role+'_subnet'],container_id=cid,birth_sha256=pin,before={'daemon_id':'original-daemon'},pg_root_measurement=measurement)
            creation = dict(intent=dict(kind='pg',name=identity['project']+'-pg-1',case_id=context._plan['source_case_id'],batch_id=context._plan['batch_id'],expected=dict(name=identity['project']+'-pg-1',labels=pg['Config']['Labels'].copy()),network={'name':identity['network']}),state='known',id=cid,record=row)
            owners[creation['intent']['name']]=creation
            context._sides[role]=dict(role=role,case_id=case_id,subnet=row['subnet'],root=root,identity=identity,creation=creation,record=row,status=status,ids=ids,pg=copy.deepcopy(volume),owner=None)
            backend.resources.append(row)
            context._compiled['live'][role+'_birth_sha256']=pin
            self.files[str(dest/'control'/(identity['database']+'.birth.json'))]=raw
            self.files[str(dest/'birth-evidence.json')]=json.dumps(dict(docker_daemon_id='original-daemon',project=identity['project'],batch_id=case_id,container_id=cid,container_started_at=pg['State']['StartedAt'],volume_name=volume['Name'],volume_mountpoint=volume['Mountpoint'],volume_mount_dev=10,volume_mount_ino=20,image_id=pg['Image'],birth_sha256=pin)).encode()
        backend.provisioner.snapshot = context._snapshot
        self.addCleanup(runner._UNSETTLED_BACKENDS.discard, backend)
        self.install_patch = patch.object(controlled._VerifiedInstall,'recheck',autospec=True,side_effect=self._install_recheck)
        self.install_patch.start();self.addCleanup(self.install_patch.stop)
        self.time_patch=patch.object(runner.time,'monotonic',side_effect=lambda:self.clock[0])
        self.time_patch.start();self.addCleanup(self.time_patch.stop)

    def _install_recheck(self,install):
        if self.install_fault:raise controlled.ControlledRejected('injected original install/source pin change')

    def _unknown_retains(self, *, before=None):
        import p0c4_source_lifecycle_gate_acceptance as driver
        owners=list(self.context._runner.ownership.owners.values())
        if before:before()
        failure=release_failure=None
        try:
            try:self.backend.stop()
            except BaseException as error:failure=error
        finally:
            # Exercise the real release path even if the stop unexpectedly succeeds.
            with patch.object(driver,'cleanup_known_helpers',return_value=[]):
                try:self.backend.release()
                except BaseException as error:release_failure=error
        self.assertIsInstance(failure,runner.ImportRejected,'stop must remain UNKNOWN')
        self.assertIsInstance(release_failure,runner.ImportRejected,'failed stop must retain original holders in release')
        self.assertFalse(self.context._closed)
        self.assertTrue(self.context._native_broken)
        self.assertEqual(self.context._install.suite_fd,self.fd)
        os.fstat(self.fd)
        self.assertTrue(all(o['state']=='known' for o in owners))
        self.assertFalse(any(a[0]=='stop' and a[-1]=='b'*64 for a,_,_ in self.calls))

    def docker(self,*args,**kwargs):
        self.calls.append((args,kwargs,self.backend.commands.isolation_deadline))
        self.assertGreater(kwargs['timeout'],0)
        self.clock[0] += 0.1
        if self.hook:
            answer=self.hook(args)
            if answer is not None:return answer
        if args == ('info','--format','{{.ID}}'):return b'original-daemon\n'
        if args[:1] == ('ps',):return ('\n'.join(f'{i:064x}' for i in range(self.inventory))).encode()
        if args[:2] in (('network','ls'),('volume','ls')):return b''
        if args[:1] == ('stop',):
            self.assertEqual(args[:3],('stop','--timeout','1'))
            self.containers[args[-1]]['State'].update(Running=False,Pid=0)
            return b''
        if args[:1] == ('inspect',) and '--format' in args and len(args)>5:
            self.clock[0] += 16
            self.inventory_timeout = True
            raise TimeoutError('unrelated host inventory exceeds original isolation deadline')
        if args[:1] == ('inspect',):row=self.containers[args[-1]]
        elif args[:2] == ('volume','inspect'):row=self.volumes[args[-1]]
        else:raise AssertionError(('unexpected Docker command',args))
        value=copy.deepcopy(row)
        if '--format' in args:
            if args[0]=='inspect':value['Config'].pop('Env')
            return (json.dumps(value)+'\n').encode()
        return json.dumps([value]).encode()

    def transport_run(self,command,**kwargs):
        self.assertEqual(command[0],'/usr/bin/docker')
        raw=self.docker(*command[1:],**kwargs)
        if self.rpc_error=='stderr':return 0,raw,b'unexpected transport diagnostic'
        if self.rpc_error=='nonzero':return 1,raw,b''
        if self.rpc_error=='unknown':return None,raw,b''
        return 0,raw,b''

    def test_actual_consumer_ignores_large_unrelated_inventory_and_keeps_original_budget(self):
        failure=None
        try:result=self.backend.stop()
        except BaseException as error:failure=error;result=None
        self.assertIsNone(failure,'actual stop must finish its own two born roles; '+
            f'inventory_timeout={self.inventory_timeout},rpc_count={len(self.calls)},clock={self.clock[0]},deadline={self.backend.commands.isolation_deadline}')
        self.assertEqual(result,dict(source_stopped=True,target_stopped=True,volumes_retained=True))
        self.assertEqual(len(self.calls),10)
        self.assertTrue(all(d==115 for _,_,d in self.calls))
        self.assertFalse(any(a[0] in ('ps','stop') or a[:2] in (('network','ls'),('volume','ls')) for a,_,_ in self.calls))

    def test_two_running_roles_stop_each_exact_cid_once_then_independently_inspect(self):
        for pg in self.containers.values():pg['State'].update(Running=True,Pid=123)
        failure=result=None
        try:result=self.backend.stop()
        except BaseException as error:failure=error
        self.assertIsNone(failure,'fixed stop transport must use supported --timeout spelling')
        self.assertEqual(result,dict(source_stopped=True,target_stopped=True,volumes_retained=True))
        self.assertEqual([a for a,_,_ in self.calls if a[0]=='stop'],[('stop','--timeout','1','a'*64),('stop','--timeout','1','b'*64)])
        self.assertEqual(len(self.calls),12)
        self.assertTrue(all(0<k['timeout']<=4.75 and d==115 for _,k,d in self.calls))

    def test_ready_restart_keeps_birth_identity_but_can_confirm_new_session_stopped(self):
        self.context._plan['case']='ready-restart'
        self.containers['b'*64]['State']['StartedAt']='2026-10-18T00:05:00Z'
        self.containers['b'*64]['RestartCount']=1
        self.assertTrue(self.backend.stop()['target_stopped'])

    def test_expired_original_deadline_rejects_before_io_and_retains_custody(self):
        self.backend.commands.isolation_deadline=99.0
        self._unknown_retains()
        self.assertEqual(self.calls,[])

    def test_nonfinite_original_deadline_rejects_before_io(self):
        self.backend.commands.isolation_deadline=float('nan')
        self._unknown_retains()
        self.assertEqual(self.calls,[])

    def test_complete_record_replacement_never_downgrades_to_fallback(self):
        self.context._sides['source']['record']=copy.deepcopy(self.backend.resources[0])
        with patch.object(runner.ContractBackend,'stop',side_effect=AssertionError('authority contradiction fell back')):
            self._unknown_retains()
        self.assertEqual(self.calls,[])

    def test_registered_creation_replacement_rejects_before_io(self):
        self.context._runner.ownership.owners[self.context._sides['source']['creation']['intent']['name']]=copy.deepcopy(self.context._sides['source']['creation'])
        self._unknown_retains();self.assertEqual(self.calls,[])

    def test_actual_cid_disagreement_rejects_before_io(self):
        self.backend.resources[0]['container_id']='d'*64
        self._unknown_retains();self.assertEqual(self.calls,[])

    def test_birth_pin_disagreement_rejects_before_io(self):
        self.context._compiled['live']['source_birth_sha256']='d'*64
        self._unknown_retains();self.assertEqual(self.calls,[])

    def test_role_plan_subnet_disagreement_rejects_before_io(self):
        self.context._plan['target_subnet']='172.30.243.0/24'
        self._unknown_retains();self.assertEqual(self.calls,[])

    def test_birth_daemon_disagreement_rejects_before_io(self):
        self.backend.resources[0]['before']['daemon_id']='replaced-daemon'
        self._unknown_retains();self.assertEqual(self.calls,[])

    def test_daemon_before_mismatch_cannot_inspect_or_stop_roles(self):
        self.hook=lambda args:b'replaced-daemon\n' if args[0]=='info' else None
        self._unknown_retains();self.assertEqual(len(self.calls),1)

    def test_daemon_after_mismatch_is_unknown_with_holders_retained(self):
        self.hook=lambda args:b'replaced-daemon\n' if len(self.calls)==10 else None
        self._unknown_retains();self.assertEqual(len(self.calls),10)

    def test_final_rpc_expiry_is_unknown_even_after_all_role_reads(self):
        def expire(args):
            if len(self.calls)==10:self.clock[0]=115
        self.hook=expire
        self._unknown_retains();self.assertEqual(len(self.calls),10)

    def test_per_rpc_timeout_stops_before_next_role(self):
        def timeout(args):
            if len(self.calls)==2:raise TimeoutError('injected transport timeout')
        self.hook=timeout
        self._unknown_retains();self.assertEqual(len(self.calls),2)

    def test_failed_stop_is_not_retried_or_followed_by_target_work(self):
        self.containers['a'*64]['State'].update(Running=True,Pid=123)
        def fail(args):
            if args[0]=='stop':raise RuntimeError('nonzero CLI')
        self.hook=fail
        self._unknown_retains()
        self.assertEqual([a for a,_,_ in self.calls if a[0]=='stop'],[('stop','--timeout','1','a'*64)])
        self.assertFalse(any(a[-1]=='b'*64 for a,_,_ in self.calls))

    def test_duplicate_or_malformed_inspect_is_unknown(self):
        self.hook=lambda args:b'{}\n{}\n' if len(self.calls)==2 else None
        self._unknown_retains();self.assertEqual(len(self.calls),2)

    def test_image_replacement_before_snapshot_cannot_authorize_stop(self):
        self.containers['a'*64]['Image']='sha256:'+'d'*64
        self._unknown_retains();self.assertEqual(len(self.calls),3)

    def test_mount_replacement_before_snapshot_cannot_authorize_stop(self):
        self.containers['a'*64]['Mounts'][0]['Source']='/foreign/path'
        self._unknown_retains();self.assertEqual(len(self.calls),3)

    def test_volume_identity_change_between_snapshots_is_unknown(self):
        def mutate(args):
            if len(self.calls)==4:self.volumes[self.context._sides['source']['identity']['volume']]['CreatedAt']='replacement'
        self.hook=mutate
        self._unknown_retains();self.assertEqual(len(self.calls),5)

    def test_execids_before_inspection_rejects_without_normalization(self):
        self.containers['a'*64]['ExecIDs']=['inflight-exec']
        self._unknown_retains();self.assertEqual(len(self.calls),3)

    def test_final_pid_must_be_zero(self):
        def mutate(args):
            if len(self.calls)==4:self.containers['a'*64]['State']['Pid']=123
        self.hook=mutate
        self._unknown_retains();self.assertEqual(len(self.calls),5)

    def test_original_identity_object_mutation_during_rpc_stops_advancement(self):
        def mutate(args):
            if len(self.calls)==2:self.context._sides['target']['identity']['image']='replacement'
        self.hook=mutate
        self._unknown_retains();self.assertEqual(len(self.calls),2)

    def test_original_deadline_replacement_during_rpc_stops_advancement(self):
        def mutate(args):
            if len(self.calls)==2:self.backend.commands.isolation_deadline=116.0
        self.hook=mutate
        self._unknown_retains();self.assertEqual(len(self.calls),2)

    def test_reentry_rejects_without_any_new_io_and_retains_holders(self):
        self.backend.stop();before=len(self.calls)
        self._unknown_retains();self.assertEqual(len(self.calls),before)

    def test_view_rejects_unexpected_selector_before_any_mutation(self):
        self.containers['a'*64]['State'].update(Running=True,Pid=123)
        def attack(view,identity,cid):
            view.snapshot();view._docker('stop','--time','1','b'*64)
        self.backend.helper.stop_verified_pg=attack
        self._unknown_retains();self.assertEqual(len(self.calls),3)

    def test_early_context_keeps_exact_conservative_fallback(self):
        self.context._body_used=False
        expected=dict(source_stopped=True,volumes_retained=True)
        with patch.object(runner.ContractBackend,'stop',return_value=expected) as inherited:
            self.assertIs(self.backend.stop(),expected)
        inherited.assert_called_once_with();self.assertEqual(self.calls,[])

    def test_clone_context_keeps_fallback_and_clone_confirmation(self):
        self.context._clone=SimpleNamespace(confirm_isolated=lambda:None)
        expected=dict(source_stopped=True,target_stopped=True,clone_stopped=True,volumes_retained=True)
        with patch.object(runner.ContractBackend,'stop',return_value=expected) as inherited,patch.object(controlled.ControlledFixtureContext,'_confirm_clone_isolated',autospec=True) as confirm:
            self.assertIs(self.backend.stop(),expected)
        inherited.assert_called_once_with();confirm.assert_called_once_with(self.context)
        self.assertEqual(self.calls,[])

    def test_legacy_backend_keeps_inherited_behavior(self):
        self.backend._controlled_context=None
        expected=dict(source_stopped=True,target_stopped=True,volumes_retained=True)
        with patch.object(runner.ContractBackend,'stop',return_value=expected) as inherited:
            self.assertIs(self.backend.stop(),expected)
        inherited.assert_called_once_with();self.assertEqual(self.calls,[])

    def test_projection_never_requests_or_returns_config_env(self):
        self.backend.stop()
        self.assertFalse(any('SECRET_SENTINEL' in str(a) or '.Config.Env' in str(a) or '{{json .Config}}' in str(a) for a,_,_ in self.calls))

    def test_stderr_even_with_zero_exit_cannot_authorize_role_work(self):
        self.rpc_error='stderr'
        self._unknown_retains();self.assertEqual(len(self.calls),1)

    def test_nonzero_cli_status_cannot_authorize_role_work(self):
        self.rpc_error='nonzero'
        self._unknown_retains();self.assertEqual(len(self.calls),1)

    def test_unknown_cli_status_cannot_authorize_role_work(self):
        self.rpc_error='unknown'
        self._unknown_retains();self.assertEqual(len(self.calls),1)

    def test_reentry_cannot_fall_back_even_if_body_state_is_changed(self):
        self.backend.stop();before=len(self.calls)
        self.context._body_used=False
        with patch.object(runner.ContractBackend,'stop',side_effect=AssertionError('reentry downgraded')):
            self._unknown_retains()
        self.assertEqual(len(self.calls),before)

    def test_native_pg_measurement_disagreement_rejects_before_io(self):
        self.backend.resources[0]['pg_root_measurement']['roots']['target_pg']['identity']['ino']=21
        self._unknown_retains();self.assertEqual(self.calls,[])

    def test_birth_raw_replacement_during_source_snapshot_prevents_target_work(self):
        def mutate(args):
            if len(self.calls)==2:
                side=self.context._sides['source']
                self.files[str(side['root']/'targets'/side['case_id']/'control'/(side['identity']['database']+'.birth.json'))]=b'replaced'
        self.hook=mutate
        self._unknown_retains()
        self.assertFalse(any(a[-1]=='b'*64 for a,_,_ in self.calls))

    def test_birth_evidence_replacement_during_source_snapshot_prevents_target_work(self):
        def mutate(args):
            if len(self.calls)==2:
                side=self.context._sides['source']
                self.files[str(side['root']/'targets'/side['case_id']/'birth-evidence.json')]=b'{}'
        self.hook=mutate
        self._unknown_retains()
        self.assertFalse(any(a[-1]=='b'*64 for a,_,_ in self.calls))

    def test_dispatched_but_missing_original_ids_never_falls_back(self):
        self.context._sides['source']['ids']=None
        with patch.object(runner.ContractBackend,'stop',side_effect=AssertionError('complete contradiction downgraded')):
            self._unknown_retains()
        self.assertEqual(self.calls,[])

    def test_install_pin_change_after_first_rpc_blocks_every_later_rpc(self):
        def mutate(args):
            if len(self.calls)==1:self.install_fault=True
        self.hook=mutate
        self._unknown_retains();self.assertEqual(len(self.calls),1)

    def test_unused_original_inventory_is_never_copied_or_compared(self):
        class UnusedInventory:
            def __deepcopy__(self,memo):raise AssertionError('unrelated inventory copied')
            def __eq__(self,other):raise AssertionError('unrelated inventory compared')
        for row in self.backend.resources:row['before']['containers']=UnusedInventory()
        failure=result=None
        try:result=self.backend.stop()
        except BaseException as error:failure=error
        self.assertIsNone(failure,'exact known stop must not traverse unused old inventory')
        self.assertTrue(result['source_stopped'] and result['target_stopped'])
        self.assertEqual(len(self.calls),10)

    def test_original_before_daemon_change_during_rpc_prevents_any_role_mutation(self):
        def mutate(args):
            if len(self.calls)==1:self.backend.resources[0]['before']['daemon_id']='changed'
        self.hook=mutate
        self._unknown_retains();self.assertEqual(len(self.calls),1)


if __name__ == '__main__':unittest.main()
