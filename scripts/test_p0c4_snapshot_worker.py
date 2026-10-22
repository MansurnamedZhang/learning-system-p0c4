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


if __name__=='__main__':unittest.main()
