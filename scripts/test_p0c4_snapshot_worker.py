import copy
import unittest
import uuid
from types import SimpleNamespace
from unittest.mock import patch
from scripts.p0c4_completion import snapshot_worker as worker


class SnapshotWorkerTests(unittest.TestCase):
    def request(self, lease=30000):
        ids=[str(uuid.uuid4()) for _ in range(4)]
        ident=dict(database='db',project='project')
        row=dict(format_version=2,operation='snapshot_broker_signal',backup_id=ids[0],database='db',compose_project='project',request_id=ids[1],epoch_id=ids[2],binding_id=ids[3],work_socket_path=worker.SOCKET_ROOT+'/'+ids[3]+'/'+ids[2]+'/work.sock',lease_ms=lease)
        return row,ident

    def test_fixed_short_and_live_capture_leases_are_accepted(self):
        for lease in (30000,180000):
            row,ident=self.request(lease)
            self.assertEqual(worker.validate_request(worker.canonical(row),'snapshot-request-'+row['request_id']+'.json',ident),row)

    def test_request_never_accepts_worker_authority_or_nonfixed_paths(self):
        row,ident=self.request()
        for key,value in [('job_id',str(uuid.uuid4())),('actor_id',str(uuid.uuid4())),('token',str(uuid.uuid4())),('fd',3),('already_committed',True)]:
            wrong=dict(row,**{key:value})
            with self.assertRaises(worker.SnapshotDriverError):worker.validate_request(worker.canonical(wrong),'snapshot-request-'+row['request_id']+'.json',ident)
        for key,value in [('lease_ms',30001),('lease_ms',True),('work_socket_path','/tmp/work.sock'),('format_version',True),('database','other')]:
            wrong=dict(row,**{key:value})
            with self.assertRaises(worker.SnapshotDriverError):worker.validate_request(worker.canonical(wrong),'snapshot-request-'+row['request_id']+'.json',ident)
        duplicate=worker.canonical(row)[:-1]+b',"lease_ms":30000}'
        with self.assertRaises(worker.SnapshotDriverError):worker.validate_request(duplicate,'snapshot-request-'+row['request_id']+'.json',ident)

    def test_only_natural_zero_and_exact_closed_scheduling_output_are_acknowledged(self):
        for disposition in worker.DISPOSITIONS:
            output=worker.scheduling_output(disposition)
            self.assertEqual(worker.validate_client_exit(0,output,b''),disposition)
            for code,out,err in [(1,output,b''),(0,output,b'warning'),(0,output+b'job=secret',b''),(0,output.upper(),b''),(0,b'WRITER_DISABLED_UNKNOWN\n',b'')]:
                if out==output:continue
                with self.assertRaises(worker.SnapshotDriverError):worker.validate_client_exit(code,out,err)

    def test_controller_starts_worker_without_waiting_and_never_acknowledges_live_child(self):
        row,ident=self.request(180000);events=[];running=[True]
        class Client:
            child=SimpleNamespace(check=lambda:None,p=SimpleNamespace(poll=lambda:None if running[0] else 0))
            def __init__(self,*args):events.append(('start',args[-2],args[-1]))
            def finish(self):
                events.append(('natural',))
                return dict(client_exit=0,disposition='COMPLETED',client_stdout_sha256=worker.digest(worker.scheduling_output('COMPLETED')),client_stderr_sha256=worker.digest(b''),client_container_id='a'*64)
        class Broker:
            polls=0
            def call(self,message):
                events.append(('rpc',message['op']))
                if message['op']=='snapshot-poll':
                    self.polls+=1
                    return [dict(request=row,request_sha256='b'*64)]if self.polls==1 else []
                return dict(request_id=row['request_id'],disposition='COMPLETED')
        record=dict(identity=ident,snapshot_worker_binary=dict(sha256='c'*64))
        controller=worker.Controller(None,None,None,None,None,record,Broker())
        with patch.object(worker,'Worker',Client):
            controller.service();self.assertIsNotNone(controller.pending)
            controller.service();self.assertFalse(any(e==('rpc','snapshot-worker-result')for e in events))
            running[0]=False;controller.service();controller.completed()
        self.assertLess(events.index(('natural',)),events.index(('rpc','snapshot-worker-result')))
        self.assertEqual(record['snapshot_worker_results'][0]['worker_binary_sha256'],'c'*64)
        self.assertEqual(events[1][1]['lease_ms'],180000)

    def test_actual_worker_inspect_only_adapts_verified_uid_and_source_pid_namespace(self):
        from scripts import p0c4_source_binding_gate_acceptance as binding
        from scripts import p0c4_source_admission_gate_acceptance as helper
        mounts=[dict(Type='bind',Source='/worker',Destination=worker.WORKER,RW=False),dict(Type='bind',Source='/guest',Destination=worker.CLIENT_ROOT+'/epoch',RW=False),dict(Type='bind',Source='/actual/work.sock',Destination=worker.CLIENT_ROOT+'/epoch/work.sock',RW=False)]
        expected=dict(id='a'*64,name='worker',image='sha256:'+'b'*64,labels={},cmd=['--snapshot-broker-client'],entrypoint=[worker.WORKER],env={'PATH':'/usr/bin'},stdin=False,mounts={(m['Type'],m['Source'],m['Destination'],m['RW'])for m in mounts},volume_names={},network='none',network_id=None)
        facts=dict(Id=expected['id'],Name='/worker',Image=expected['image'],Config=dict(Image=expected['image'],User='65532:65532',WorkingDir='/reviewed',Labels={},Cmd=expected['cmd'],Entrypoint=expected['entrypoint'],Env=['PATH=/usr/bin'],OpenStdin=False),HostConfig=dict(Privileged=False,CapAdd=None,CapDrop=['ALL'],SecurityOpt=['no-new-privileges'],NanoCpus=4*10**9,Memory=8*1024**3,MemorySwap=8*1024**3,PidsLimit=512,ReadonlyRootfs=True,Tmpfs={'/tmp':'rw,nosuid,nodev,size=1g'},PidMode='container:'+'d'*64,IpcMode='private',NetworkMode='none',CgroupnsMode='private'),Mounts=mounts,State=dict(Status='created',Running=False,StartedAt='0001-01-01T00:00:00Z',FinishedAt='0001-01-01T00:00:00Z'),NetworkSettings=dict(Ports={},Networks={}))
        client=worker.Worker.__new__(worker.Worker);client.pid_mode=facts['HostConfig']['PidMode'];client.b=binding;client.h=helper;client.expected=expected
        self.assertIs(client.validate(facts),facts)
        for mutate in (lambda x:x['Config'].__setitem__('User','0:0'),lambda x:x['HostConfig'].__setitem__('PidMode','host'),lambda x:x['HostConfig'].__setitem__('CapAdd',['SYS_PTRACE']),lambda x:x['HostConfig'].__setitem__('ReadonlyRootfs',False),lambda x:x['Mounts'].append(dict(Type='bind',Source='/root',Destination='/root',RW=False))):
            wrong=copy.deepcopy(facts);mutate(wrong)
            with self.assertRaises((worker.SnapshotDriverError,binding.GateError)):client.validate(wrong)

    def test_snapshot_pass_status_and_controller_source_closure_are_separate(self):
        from scripts import p0c4_completion_acceptance as completion
        for case in completion.full_lane.SNAPSHOT_CASE_TESTS:self.assertEqual(completion.full_case_pass_status(case),completion.SNAPSHOT_PASS)
        for case in completion.full_lane.RECOVERY_CASE_TESTS:self.assertEqual(completion.full_case_pass_status(case),completion.RECOVERY_PASS)
        seen=[]
        def installed(path):seen.append(path);return path.encode()
        from pathlib import Path
        files={'scripts/'+path.relative_to(Path('scripts')).as_posix():path.relative_to(Path('scripts')).as_posix().encode()for path in Path('scripts').rglob('*.py')}
        with patch.object(completion,'_installed_full_controller_bytes',side_effect=installed):
            completion._verify_full_import_closure(files);self.assertNotIn('p0c4_completion/snapshot_worker.py',seen)
            completion._verify_full_import_closure(files,snapshot=True);self.assertIn('p0c4_completion/snapshot_worker.py',seen)
            files['scripts/p0c4_completion/snapshot_worker.py']=b'drift'
            with self.assertRaises(completion.CompletionError):completion._verify_full_import_closure(files,snapshot=True)

    def test_runtime_passwords_are_domain_separated_from_each_other_and_admin(self):
        admin='a'*64
        values={worker.login_password(admin,role)for role in ('learning_runtime','learning_snapshot_claim_broker')}
        self.assertEqual(len(values),2);self.assertNotIn(admin,values)
        with self.assertRaises(worker.SnapshotDriverError):worker.login_password(admin,'postgres')

    def test_snapshot_credentials_keep_original_inode_metadata_and_bytes_without_exposing_dsn(self):
        import tempfile,stat
        from pathlib import Path
        driver=worker._driver()
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'proofs').mkdir()
            for name in ('broker-dsn','runtime-dsn'):(root/name).write_bytes((name+':private-secret').encode())
            changed=[None]
            def read(path,limit):
                raw=path.read_bytes();m=path.stat();require=len(raw)<=limit
                self.assertTrue(require)
                meta=SimpleNamespace(st_dev=1,st_ino=123 if path.name=='broker-dsn' else 456,st_uid=0,st_gid=0,st_mode=stat.S_IFREG|0o600,st_nlink=1)
                if path.name=='broker-dsn'and changed[0]:setattr(meta,*changed[0])
                return raw,meta
            with patch.object(driver,'private_read',side_effect=read),patch.object(driver,'new_private',side_effect=lambda path,raw:path.write_bytes(raw)):
                protocol=worker.BrokerProtocol(root,{})
                protocol.pin_credentials();protocol.audit_credentials()
                self.assertNotIn(b'private-secret',(root/'proofs/snapshot-credential-pins.json').read_bytes())
                for field,value in [('st_ino',999),('st_nlink',2),('st_mode',stat.S_IFREG|0o644),('st_gid',65532)]:
                    changed[0]=(field,value)
                    with self.subTest(field=field),self.assertRaisesRegex(worker.SnapshotDriverError,'SNAPSHOT_CREDENTIAL_CHANGED'):protocol.audit_credentials()
                changed[0]=None;(root/'broker-dsn').write_bytes(b'changed-secret')
                with self.assertRaisesRegex(worker.SnapshotDriverError,'SNAPSHOT_CREDENTIAL_CHANGED'):protocol.audit_credentials()

    def test_real_worker_materializer_pair_reaches_the_original_target_fd_gate(self):
        from scripts import p0c4_source_lifecycle_gate_acceptance as driver
        destination='/target/materialized/'+str(uuid.uuid4())+'-worker'
        with patch.object(driver.sys,'argv',['materialize','/target/build/debug/learning-worker',destination]),patch.object(driver.os,'O_DIRECTORY',0,create=True),patch.object(driver.os,'O_NOFOLLOW',0,create=True),patch.object(driver.os,'open',side_effect=RuntimeError('ORIGINAL_TARGET_FD_GATE')):
            with self.assertRaisesRegex(RuntimeError,'ORIGINAL_TARGET_FD_GATE'):driver.materialize_main()
        for source,dest in [('/target/build/debug/other',destination),('/target/build/debug/learning-worker',destination.removesuffix('worker')+'lib'),('/target/build/debug/deps/learning_backup-123abc',destination)]:
            with patch.object(driver.sys,'argv',['materialize',source,dest]),self.assertRaisesRegex(driver.GateError,'MATERIALIZE_ARGUMENTS'):driver.materialize_main()

    def test_zero_asset_capture_audit_checks_real_manifest_index_tree_and_toc(self):
        import json
        from scripts import p0c4_source_lifecycle_gate_acceptance as driver
        backup=str(uuid.uuid4());prefix='pins/'+backup+'.sealed/'
        migrations=[dict(version=14,checksum_hex='d'*96)]
        source=dict(application_build_sha256='a'*64,application_commit=driver.BASE_COMMIT,postgres_major=18,migration_version=14,migrations=migrations,migration_fingerprint=worker.digest(json.dumps(migrations,separators=(',',':')).encode()))
        files=[dict(path=name,size=9,sha256='b'*64)for name in ('asset-index.json','database.dump','roles.json')]
        manifest=dict(capability='backup_full_v1',format_version=1,backup_id=backup,source=source,logical_asset_count=0,unique_asset_bytes=0,files=files)
        raw=json.dumps(manifest,separators=(',',':')).encode()
        inventory={prefix+row['path']:dict(bytes=row['size'],sha256=row['sha256'])for row in files}
        inventory[prefix+'manifest.json']=dict(raw_hex=raw.hex())
        record=dict(identity=dict(database='db'),migrations=migrations,archive_sha256='a'*64,evidence_directory='/private')
        toc=b'; Archive created at now\n; dbname: db\n; Format: CUSTOM\n; Dumped from database version: 18.6\n; Dumped by pg_dump version: 18.6\n; TOC Entries: 1\n; Selected TOC Entries:\n1; 1 1 TABLE public job learning_admin\n'
        def execute(runner,container,command):return(0,b'{"format_version":1,"assets":[]}',b'')if command[0]=='cat'else(0,toc,b'')
        with patch.object(driver,'sql',return_value='0'),patch.object(driver,'pg_exec',side_effect=execute),patch.object(driver,'new_private'):
            audited=worker.capture_postcheck(None,'container',record,inventory)
            self.assertEqual(audited['logical_rows'],0);self.assertTrue(audited['toc_only_no_restore'])
            for mutation in ('extra','bad-hash','nonempty-index','wrong-source'):
                changed=copy.deepcopy(inventory)
                if mutation=='extra':changed[prefix+'extra']=dict(bytes=1,sha256='c'*64)
                elif mutation=='bad-hash':changed[prefix+'database.dump']['sha256']='c'*64
                elif mutation=='wrong-source':changed[prefix+'manifest.json']['raw_hex']=raw.replace(b'"logical_asset_count":0',b'"logical_asset_count":1').hex()
                else:changed['assets/unexpected']=dict(bytes=1,sha256='c'*64)
                with self.subTest(mutation=mutation),self.assertRaises(worker.SnapshotDriverError):worker.capture_postcheck(None,'container',record,changed)


class SnapshotSourceBindAdapterTests(unittest.TestCase):
    def fixture(self):
        # Scrubbed actual P6 0334 shape: 18 Mounts, 10 Binds, exactly one
        # rw/rslave bind at the fixed snapshot socket root. No native credit.
        import stat
        from pathlib import PurePosixPath
        from scripts import p0c4_source_admission_gate_acceptance as helper
        from scripts.p0c4_completion import full_import as full
        from scripts.test_p0c4_full_rehearsal_profile import profile
        p=profile();p.update(format_version=2,capability='full_rehearsal_snapshot_profile_v2')
        project='kwc4c-'+p['source_case_id'].replace('-','')
        for key,suffix,ino in [('snapshot_effects','snapshot-effects',107),('snapshot_export','snapshot-export',108)]:
            name=project+'-'+suffix
            p['volumes'][key]=dict(name=name,path='/var/lib/docker/volumes/'+name+'/_data',dev=10,ino=ino,uid=0,gid=0,mode=0o700)
        plan=dict(format_version=3,case_id=p['source_case_id'],batch_id='750e8400-e29b-41d4-a716-446655440000',case_name='snapshot-p6',schema_profile='snapshot_c0_d1_v1',snapshot_profile=worker.PROFILE)
        context=full.FullRehearsalContext(full._TOKEN,plan)
        context._ident=dict(case_id=p['source_case_id'],project=project,pg_name=project+'-pg-1')
        context._case=PurePosixPath(p['source_bind_root']['path'])
        context._target_root=PurePosixPath(p['volumes']['target_control']['path'])
        context._target_volumes={key:dict(Name=p['volumes'][other]['name'],Mountpoint=p['volumes'][other]['path'])for key,other in [('control','target_control'),('socket','target_socket')]}
        context._snapshot_volumes={key:dict(Name=p['volumes'][key]['name'],Mountpoint=p['volumes'][key]['path'])for key in ('snapshot_effects','snapshot_export')}
        root=p['volumes']['source_data'];context._snapshot_source_mountpoint=root['path']
        volume=dict(Name=root['name'],Mountpoint=root['path'],Driver='local',Options=None,Labels={'com.docker.compose.project':project,'knowweave.source-lifecycle.batch':plan['batch_id']})
        context._record=dict(volumes=dict(source_volume=volume),snapshot_root_prepare={key:{k:p['volumes'][other][k]for k in ('dev','ino','uid','gid','mode')}for key,other in [('source','source_data'),('snapshot_effects','snapshot_effects'),('snapshot_export','snapshot_export')]})
        for key in ('snapshot_effects','snapshot_export'):
            value=p['volumes'][key]
            measured=dict(Name=value['name'],Mountpoint=value['path'],Driver='local',Options=None,Labels=copy.deepcopy(volume['Labels']))
            context._snapshot_volumes[key]=measured
            context._record['volumes'][key+'_volume']=copy.deepcopy(measured)
        mounts=[];names={v['path']:v['name']for v in p['volumes'].values()}
        for kind,source,target,rw in full._mounts(p,'source'):
            row=dict(Type=kind,Source=source,Destination=target,RW=rw,Mode=('rw'if rw else'ro')if kind=='bind'else'z',Propagation='rslave'if kind=='bind'else'')
            if kind=='volume':row['Name']=names[source]
            mounts.append(row)
        declared=[dict(Type='volume',Source=m['Name'],Target=m['Destination'],ReadOnly=not m['RW'],VolumeOptions={'NoCopy':True})for m in mounts if m['Type']=='volume']
        expected=dict(id='a'*64,name=context._ident['pg_name'],image='sha256:'+'b'*64,image_ref=helper.PG_IMAGE,env={},labels={'com.docker.compose.project':project,'com.docker.compose.service':'pg'},cmd=['postgres'],entrypoint=['docker-entrypoint.sh'],mounts={(m['Type'],m['Source'],m['Destination'],m['RW'])for m in mounts},network=project+'-net',builder=False,user='')
        facts=dict(Id=expected['id'],Name='/'+expected['name'],Image=expected['image'],Config=dict(Image=expected['image_ref'],Env=[],Labels=expected['labels'],Cmd=expected['cmd'],Entrypoint=expected['entrypoint'],User=''),HostConfig=dict(Privileged=False,CapAdd=None,CapDrop=None,SecurityOpt=['no-new-privileges'],NanoCpus=2*10**9,Memory=4*1024**3,MemorySwap=4*1024**3,IpcMode='private',NetworkMode=expected['network'],Binds=[m['Source']+':'+m['Destination']+':'+m['Mode']for m in mounts if m['Type']=='bind'],Mounts=declared),Mounts=mounts,NetworkSettings=dict(Ports={}),State=dict(Status='created',Running=False,Pid=0))
        rootmeta=SimpleNamespace(st_mode=stat.S_IFDIR|0o700,st_uid=0,st_gid=0,st_dev=root['dev'],st_ino=root['ino'])
        socketmeta=SimpleNamespace(**dict(vars(rootmeta),st_ino=9001))
        return full,helper,context,facts,expected,rootmeta,socketmeta

    def filesystem(self,rootmeta,socketmeta):
        # The control capsule has no host source-volume mount. Resource-only
        # validation must not attempt a host path open/stat. Metadata is from
        # the fixed Root preparation observation, not a synthetic live FD.
        from contextlib import ExitStack
        import os
        from scripts import p0c4_storage_registry as registry
        stack=ExitStack()
        for module,name in [(registry,'open_directory'),(registry,'child_directory'),(os,'open'),(os,'stat'),(os,'fstat')]:
            stack.enter_context(patch.object(module,name,side_effect=AssertionError('control capsule cannot open host source volume')))
        return stack

    def test_actual_p6_rw_socket_shape_reaches_unchanged_legacy_validation(self):
        full,helper,context,facts,expected,rootmeta,socketmeta=self.fixture()
        self.assertEqual(len(facts['Mounts']),18);self.assertEqual(len(facts['HostConfig']['Binds']),10)
        with self.assertRaisesRegex(helper.GateError,'PG_BINDS_MODE'):helper.validate_container(facts,expected)
        original=copy.deepcopy(facts);original_expected=copy.deepcopy(expected)
        with self.filesystem(rootmeta,socketmeta),patch.object(helper,'validate_container',wraps=helper.validate_container)as legacy:
            full._FullContainerAdapter(helper,context).validate_container(facts,expected)
            view,passed_expected=legacy.call_args.args
            self.assertEqual(view['Mounts'],facts['Mounts']);self.assertIs(passed_expected,expected)
            self.assertEqual(view['HostConfig']['Binds'],[x for x in facts['HostConfig']['Binds']if not x.endswith(':rw')])
        self.assertEqual(facts,original);self.assertEqual(expected,original_expected)
        with self.assertRaisesRegex(helper.GateError,'PG_BINDS_MODE'):helper.validate_container(facts,expected)

    def test_socket_exception_rejects_bind_mount_and_legacy_security_drift(self):
        for change in ('source','destination','readonly','options','duplicate','extra_rw','missing_bind','missing_mount','mount_source','mount_mode','propagation','mount_rw','expected_count','nocopy','security','identity'):
            with self.subTest(change=change):
                full,helper,context,facts,expected,rootmeta,socketmeta=self.fixture()
                binds=facts['HostConfig']['Binds'];index=next(i for i,v in enumerate(binds)if v.endswith(':rw'))
                row=next(v for v in facts['Mounts']if v['Destination']=='/run/kwc4/sfx')
                if change=='source':binds[index]='/foreign:/run/kwc4/sfx:rw'
                elif change=='destination':binds[index]=binds[index].replace('/run/kwc4/sfx','/foreign')
                elif change=='readonly':binds[index]=binds[index][:-2]+'ro'
                elif change=='options':binds[index]+=',shared'
                elif change=='duplicate':binds[0]=binds[index]
                elif change=='extra_rw':binds.append('/foreign:/foreign:rw')
                elif change=='missing_bind':binds.pop(index)
                elif change=='missing_mount':facts['Mounts'].remove(row)
                elif change=='mount_source':row['Source']='/foreign'
                elif change=='mount_mode':row['Mode']='rw,shared'
                elif change=='propagation':row['Propagation']='rshared'
                elif change=='mount_rw':row['RW']=False
                elif change=='expected_count':expected['mounts'].add(('bind','/foreign','/foreign',False))
                elif change=='nocopy':
                    next(v for v in facts['HostConfig']['Mounts']if v['Target']=='/var/lib/knowweave-c4/snapshot-effects-v2')['VolumeOptions']['NoCopy']=False
                elif change=='security':facts['HostConfig']['Privileged']=True
                else:facts['Id']='9'*64
                with self.filesystem(rootmeta,socketmeta),self.assertRaises((full.FullImportError,helper.GateError)):
                    full._FullContainerAdapter(helper,context).validate_container(facts,expected)

    def test_snapshot_source_missing_null_or_empty_binds_never_bypasses_fd_gate(self):
        from scripts import p0c4_storage_registry as registry
        for change in ('absent','null','empty'):
            with self.subTest(change=change):
                full,helper,context,facts,expected,_,_=self.fixture()
                if change=='absent':facts['HostConfig'].pop('Binds')
                else:facts['HostConfig']['Binds']=None if change=='null'else[]
                with patch.object(registry,'open_directory',side_effect=AssertionError('invalid binds reached filesystem'))as opened,patch.object(helper,'validate_container',wraps=helper.validate_container)as legacy:
                    with self.assertRaisesRegex(full.FullImportError,'FULL_SNAPSHOT_SOCKET_BINDS'):
                        full._FullContainerAdapter(helper,context).validate_container(facts,expected)
                    legacy.assert_not_called();opened.assert_not_called()

    def test_only_fixed_snapshot_source_can_omit_the_redundant_rw_entry(self):
        for change in ('foundation','ordinary','target','builder','schema','source_volume','source_label'):
            with self.subTest(change=change):
                full,helper,context,facts,expected,rootmeta,socketmeta=self.fixture()
                if change=='foundation':context._plan.update(format_version=2,case_name='recovery-leases')
                elif change=='ordinary':context._plan['snapshot_profile']='ordinary'
                elif change=='target':expected['name']='target'
                elif change=='builder':expected['builder']=True
                elif change=='schema':context._plan['schema_profile']='base_v1'
                elif change=='source_volume':context._record['volumes']['source_volume']['Mountpoint']='/foreign'
                else:context._record['volumes']['source_volume']['Labels']['knowweave.source-lifecycle.batch']='foreign'
                with self.filesystem(rootmeta,socketmeta),self.assertRaises((full.FullImportError,helper.GateError)):
                    full._FullContainerAdapter(helper,context).validate_container(facts,expected)

    def test_socket_exception_requires_closed_root_preparation_and_volume_facts(self):
        for change in ('missing_root','extra_root','missing_field','extra_field','uid','gid','mode','zero_dev','bool_ino','huge_ino','root_alias','volume_alias','volume_name','volume_driver','volume_options','volume_labels','volume_record'):
            with self.subTest(change=change):
                full,helper,context,facts,expected,rootmeta,socketmeta=self.fixture()
                roots=context._record['snapshot_root_prepare'];row=roots['source']
                if change=='missing_root':roots.pop('snapshot_export')
                elif change=='extra_root':roots['other']=copy.deepcopy(row)
                elif change=='missing_field':row.pop('mode')
                elif change=='extra_field':row['authority']=True
                elif change=='uid':row['uid']=65532
                elif change=='gid':row['gid']=65532
                elif change=='mode':row['mode']=0o750
                elif change=='zero_dev':row['dev']=0
                elif change=='bool_ino':row['ino']=True
                elif change=='huge_ino':row['ino']=2**64
                elif change=='root_alias':roots['snapshot_effects']=copy.deepcopy(row)
                elif change=='volume_alias':context._snapshot_volumes['snapshot_effects']['Mountpoint']=context._snapshot_source_mountpoint
                elif change=='volume_name':context._snapshot_volumes['snapshot_export']['Name']='foreign'
                elif change=='volume_driver':context._snapshot_volumes['snapshot_export']['Driver']='foreign'
                elif change=='volume_options':context._snapshot_volumes['snapshot_export']['Options']={'device':'foreign'}
                elif change=='volume_labels':context._snapshot_volumes['snapshot_export']['Labels']={}
                else:context._record['volumes']['snapshot_export_volume']['Mountpoint']='/foreign'
                with self.filesystem(rootmeta,socketmeta),patch.object(helper,'validate_container',wraps=helper.validate_container)as legacy:
                    with self.assertRaises(full.FullImportError):
                        full._FullContainerAdapter(helper,context).validate_container(facts,expected)
                    legacy.assert_not_called()


class SnapshotFixedParentRecipeTests(unittest.TestCase):
    MARKER=b'SNAPSHOT_PRIVATE_CONTAINER_PARENT_READY\n'

    class Driver:
        def __init__(self,reply):self.reply=reply;self.events=[]
        def sql(self,runner,container,statement):self.events.append(('sql',container,statement))
        def pg_exec(self,runner,container,command):
            self.events.append(('pg_exec',container,command))
            if isinstance(self.reply,Exception):raise self.reply
            return self.reply

    def prepare(self,reply=None):
        driver=self.Driver((0,self.MARKER,b'')if reply is None else reply)
        record=dict(container_id='a'*64)
        with patch.object(worker,'_driver',return_value=driver):
            worker.prepare_source_container(object(),record,'b'*64)
        return driver,record

    def test_both_fixed_parents_are_guarded_and_hardened_before_ready(self):
        import re
        import shlex
        driver,record=self.prepare()
        self.assertEqual([x[0]for x in driver.events],['sql','pg_exec'])
        command=driver.events[1][2]
        self.assertEqual(command[:2],['/bin/sh','-ec'])
        recipe=command[2]
        loop=re.fullmatch(r'set -eu; for p in ([^;]+); do (.+); done; printf "SNAPSHOT_PRIVATE_CONTAINER_PARENT_READY\\n"',recipe)
        self.assertIsNotNone(loop,'both fixed fresh parent paths must be guarded before READY')
        self.assertEqual(shlex.split(loop.group(1)),['/var/lib/knowweave-c4','/run/kwc4'])
        operations=loop.group(2).split('; ')
        self.assertEqual(operations,['test -d "$p"','test ! -L "$p"','test "$(stat -c "%u %g" "$p")" = "0 0"','chmod 700 "$p"','test "$(stat -c "%u %g %a" "$p")" = "0 0 700"'])
        self.assertNotIn('mkdir',recipe)
        self.assertNotIn('/run/kwc4/sfx',recipe)
        self.assertNotIn('chmod -R',recipe)
        self.assertEqual(record['snapshot_source_login_setup'],dict(scram_configured=True,container_private_parent=True,classification='FRESH_TEST_FIXTURE_ADMINISTRATION_NOT_CAPTURE'))

    def test_ready_requires_known_zero_exact_marker_and_no_stderr(self):
        for reply in [(1,self.MARKER,b''),(0,self.MARKER,b'warning'),(0,b'',b''),(0,self.MARKER+b'extra',b''),(0,self.MARKER.replace(b'\n',b'\r\n'),b'')]:
            with self.subTest(reply=reply):
                driver=self.Driver(reply);record=dict(container_id='a'*64)
                with patch.object(worker,'_driver',return_value=driver),self.assertRaises(worker.SnapshotDriverError):
                    worker.prepare_source_container(object(),record,'b'*64)
                self.assertNotIn('snapshot_source_login_setup',record)
                self.assertEqual([x[0]for x in driver.events],['sql','pg_exec'])

    def test_login_role_and_container_binding_stay_fixed_without_parent_retry(self):
        driver,record=self.prepare()
        self.assertTrue(all(event[1]==record['container_id']for event in driver.events))
        expected=''.join("\nALTER ROLE "+role+" PASSWORD '"+worker.login_password('b'*64,role)+"';"for role in ('learning_runtime','learning_snapshot_claim_broker'))
        self.assertEqual(driver.events[0][2],expected)
        error=RuntimeError('synthetic parent precondition rejection')
        driver=self.Driver(error);record=dict(container_id='a'*64)
        with patch.object(worker,'_driver',return_value=driver),self.assertRaises(RuntimeError)as caught:
            worker.prepare_source_container(object(),record,'b'*64)
        self.assertIs(caught.exception,error)
        self.assertEqual(len(driver.events),2)
        self.assertNotIn('snapshot_source_login_setup',record)

if __name__=='__main__':unittest.main()
