"""Finite same-ID clone contracts; portable tests confer no PG execution credit."""
import copy
import importlib
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import json
import hashlib
from types import SimpleNamespace
import unittest


class CloneContracts(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(importlib.util.find_spec('p0c4_completion.controlled_clone'),
                             'missing finite same-ID clone implementation')
        self.clone=importlib.import_module('p0c4_completion.controlled_clone')

    def identity(self):
        suffix='44444444-4444-4444-8444-444444444444'
        return dict(project='learning-system-p0c4-restore-'+suffix,
                    network='learning-system-p0c4-restore-'+suffix+'_test',
                    volume='learning-system-p0c4-restore-'+suffix+'_pg',
                    image='postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d',
                    database='learning_restore_c4_'+suffix)

    def test_clone_plan_requires_three_distinct_private_networks_and_five_ids(self):
        from test_p0c4_controlled_fixture_context import ContextPlanTests
        plan=ContextPlanTests().plan()
        plan.update(capability='c4_current11_same_id_clone_plan_v1',case='wrong-endpoint',
                    budget_profile='controlled_fixture_same_id_clone_v1',
                    clone_case_id='44444444-4444-4444-8444-444444444444',clone_subnet='172.30.243.0/24')
        self.assertEqual(self.clone._validate_plan(plan),plan)
        for key,value in [('clone_case_id',plan['target_case_id']),('clone_subnet',plan['source_subnet']),
                          ('clone_subnet','8.8.8.0/24'),('case','success'),('budget_profile','controlled_fixture_two_role_fixed_case_v1')]:
            with self.subTest(key=key),self.assertRaises(self.clone.CloneRejected):
                self.clone._validate_plan(dict(plan,**{key:value}))
        with self.assertRaises(self.clone.CloneRejected):self.clone._validate_plan(dict(plan,command=['true']))

    def test_pg_helper_contract_grants_only_one_fixed_copy_route(self):
        identity=self.identity();cid='a'*64
        setup=self.clone._pg_helper_contract('setup',identity,cid)
        copy_contract=self.clone._pg_helper_contract('copy',identity,cid)
        verify=self.clone._pg_helper_contract('verify',identity,cid)
        self.assertEqual(setup['user'],'0:0');self.assertEqual(setup['caps'],['CHOWN'])
        self.assertEqual(copy_contract['user'],'999:999');self.assertEqual(copy_contract['caps'],[])
        self.assertEqual(copy_contract['network'],'container:'+cid)
        self.assertEqual(copy_contract['mounts'],[('volume',identity['volume'],'/var/lib/postgresql',False)])
        self.assertEqual(verify['network'],'none')
        self.assertEqual(verify['mounts'],[('volume',identity['volume'],'/var/lib/postgresql',True)])
        self.assertNotIn('-R',copy_contract['command'])
        with self.assertRaises(self.clone.CloneRejected):self.clone._pg_helper_contract('shell',identity,cid)
        with self.assertRaises(self.clone.CloneRejected):self.clone._pg_helper_contract('copy',dict(identity,volume='foreign'),cid)

    def test_auth_rejects_scram_and_later_trust_after_nonexact_first_match(self):
        row=dict(rule_number=1,type='host',database=['replication'],user_name=['all'],
                 address='127.0.0.1',netmask='255.255.255.255',options=None,auth_method='trust',error=None)
        self.assertEqual(self.clone._trust_replication([row]),'EXACT_LOOPBACK_TRUST')
        for changed in [dict(row,auth_method='scram-sha-256'),dict(row,address='0.0.0.0',netmask='0.0.0.0'),dict(row,error='bad')]:
            with self.subTest(changed=changed),self.assertRaises(self.clone.CloneRejected):
                self.clone._trust_replication([changed,dict(row,rule_number=2)])

    def test_same_id_observation_rejects_oid_db_and_recovery_substitution(self):
        expected=dict(database='learning_restore_c4_11111111-1111-4111-8111-111111111111',
                      pg_system_identifier='7555512345678901234',database_oid=16384)
        line=b'learning_restore_c4_11111111-1111-4111-8111-111111111111|7555512345678901234|16384|f\n'
        self.assertEqual(self.clone._same_id_line(line,expected),expected)
        for raw in [line.replace(b'16384',b'16385'),line.replace(b'|f',b'|t'),line.replace(b'75555',b'75556'),line+line]:
            with self.subTest(raw=raw),self.assertRaises(self.clone.CloneRejected):self.clone._same_id_line(raw,expected)

    def test_pg_helper_observation_rejects_extra_mount_and_privilege(self):
        contract=self.clone._pg_helper_contract('copy',self.identity(),'a'*64)
        image=dict(Id='sha256:'+'b'*64,Config=dict(Env=['PATH=/usr/lib/postgresql/18/bin:/usr/bin:/bin']))
        row=dict(Id='c'*64,Name='/'+contract['name'],Image=image['Id'],
                 Config=dict(Image=image['Id'],User='999:999',Entrypoint=contract['command'][:1],Cmd=contract['command'][1:],
                             Env=image['Config']['Env'],Labels={'knowweave.controlled.clone':self.identity()['project'],'knowweave.controlled.clone.helper':'copy'},WorkingDir='/',OpenStdin=False,Tty=False),
                 HostConfig=dict(NetworkMode='container:'+'a'*64,CapDrop=['ALL'],CapAdd=None,SecurityOpt=['no-new-privileges'],Privileged=False,
                                 PidMode='',ReadonlyRootfs=True,PortBindings={},Binds=None,Devices=[],DeviceRequests=None,RestartPolicy={'Name':'no','MaximumRetryCount':0},
                                 Memory=1024**3,MemorySwap=1024**3,NanoCpus=2*10**9,PidsLimit=128,Tmpfs={'/tmp':'rw,nosuid,nodev,size=16m'}),
                 Mounts=[dict(Type='volume',Name=self.identity()['volume'],Source='/var/lib/docker/volumes/'+self.identity()['volume']+'/_data',Destination='/var/lib/postgresql',RW=True)],
                 NetworkSettings=dict(Ports={},Networks={}),State=dict(Status='created',Running=False,Pid=0,Dead=False,OOMKilled=False,Error=''),RestartCount=0,ExecIDs=None)
        self.assertEqual(self.clone._validate_pg_helper(row,contract,image,'c'*64),row)
        for mutate in ['socket','cap','user','network','env','write','pid']:
            item=copy.deepcopy(row)
            if mutate=='socket':item['Mounts'].append(dict(Type='bind',Source='/var/run/docker.sock',Destination='/var/run/docker.sock',RW=False))
            elif mutate=='cap':item['HostConfig']['CapAdd']=['SYS_ADMIN']
            elif mutate=='user':item['Config']['User']='0:0'
            elif mutate=='network':item['HostConfig']['NetworkMode']='host'
            elif mutate=='env':item['Config']['Env'].append('PGPASSWORD=secret')
            elif mutate=='write':item['HostConfig']['ReadonlyRootfs']=False
            else:item['HostConfig']['PidMode']='host'
            with self.subTest(mutate=mutate),self.assertRaises(self.clone.CloneRejected):
                self.clone._validate_pg_helper(item,contract,image,'c'*64)

    def test_target_owner_after_birth_keeps_original_lock_on_disconnect_only_for_clone(self):
        from p0c4_completion import full_target_fs as fs
        factory=getattr(fs,'_new_controlled_clone_target_owner',None)
        self.assertTrue(callable(factory),'missing original-target post-birth retention policy')
        target=factory();ordinary=fs._new_controlled_owner()
        for owner in (target,ordinary):
            owner.root_fd=43;owner._files_ready=True;owner._birth_complete=True
        self.assertTrue(target._disconnect_requires_hold())
        self.assertFalse(ordinary._disconnect_requires_hold())
        target.root_fd=None
        self.assertFalse(target._disconnect_requires_hold())

    def test_creation_lock_handoff_is_unavailable_before_real_birth(self):
        from p0c4_completion import full_target_fs as fs
        owner=fs._new_controlled_clone_target_owner()
        method=getattr(owner,'_release_creation_after_birth',None)
        self.assertTrue(callable(method),'missing separate directory custody handoff')
        with self.assertRaises(fs.FsRejected):method()

    def test_shared_plan_accepts_only_explicit_clone_capability(self):
        from p0c4_completion import controlled_fixture as fixture
        from test_p0c4_controlled_fixture_context import ContextPlanTests
        plan=ContextPlanTests().plan()
        plan.update(capability='c4_current11_same_id_clone_plan_v1',case='wrong-endpoint',
                    budget_profile='controlled_fixture_same_id_clone_v1',
                    clone_case_id='44444444-4444-4444-8444-444444444444',clone_subnet='172.30.243.0/24')
        try:observed=fixture._validate_plan(plan)
        except fixture.ControlledRejected as error:self.fail('new exact three-role plan is rejected: '+str(error))
        self.assertEqual(observed,plan)
        del plan['clone_subnet']
        with self.assertRaises((fixture.ControlledRejected,self.clone.CloneRejected)):fixture._validate_plan(plan)

    def test_clone_compose_starts_nothing_and_has_no_initdb_or_target_mount(self):
        method=getattr(self.clone,'_compose_document',None)
        self.assertTrue(callable(method),'missing closed physical-clone compose recipe')
        document=method(self.identity(),'172.30.243.0/24')
        pg=document['services']['pg']
        self.assertEqual(pg['user'],'999:999');self.assertEqual(pg['command'],['postgres'])
        self.assertEqual(pg['environment'],{'PGDATA':'/var/lib/postgresql/18/docker'})
        self.assertEqual(len(pg['volumes']),1);self.assertNotIn('ports',pg)
        self.assertEqual(pg['volumes'][0]['source'],self.identity()['volume'])
        self.assertEqual(pg['volumes'][0]['volume'],{'nocopy':True})
        self.assertEqual(pg['restart'],'no');self.assertEqual(pg['cap_drop'],['ALL'])

    def test_clone_lane_cannot_be_minted_from_plan_or_completed_observation(self):
        cls=getattr(self.clone,'_CloneLane',None)
        self.assertTrue(isinstance(cls,type),'missing original-owner clone lane')
        for value in ({},{'state':'BORN'},{'target_id':'a'*64},None):
            with self.subTest(value=value),self.assertRaises(self.clone.CloneRejected):cls(value)

    def test_clone_owner_protocol_rejects_replay_and_birth_before_setup(self):
        cls=getattr(self.clone,'_RootProtocol',None)
        self.assertTrue(isinstance(cls,type),'missing finite clone-root owner protocol')
        class Backend:
            def perform(self,op,value):return {'recorded':True}
        nonce='55555555-5555-4555-8555-555555555555'
        protocol=cls(self.clone._TOKEN,nonce,Backend())
        early=protocol.handle(dict(session=nonce,seq=0,op='birth',value={}))
        self.assertFalse(early['ok'])
        self.assertFalse(protocol.handle(dict(session=nonce,seq=0,op='setup',value={}))['ok'])
        protocol=cls(self.clone._TOKEN,nonce,Backend())
        first=protocol.handle(dict(session=nonce,seq=0,op='setup',value={}))
        self.assertTrue(first['ok'])
        self.assertFalse(protocol.handle(dict(session=nonce,seq=0,op='setup',value={}))['ok'])

    def test_clone_custody_requires_original_live_target_owner_and_creation_object(self):
        from p0c4_completion.controlled_fixture import ControlledFixtureContext
        context=object.__new__(ControlledFixtureContext)
        self.assertIn('_clone',ControlledFixtureContext.__slots__,'missing exact context clone slot')
        context._closed=False;context._native_broken=False;context._install=SimpleNamespace(recheck=lambda:None)
        creation=dict(state='known',id='a'*64,intent={'name':'target'})
        class Owner:
            closed=False
            def held(self):
                if self.closed:raise RuntimeError('closed original')
        original=Owner()
        context._sides={'target':dict(owner=original,creation=creation,ids={'container_id':'a'*64})}
        context._runner=SimpleNamespace(ownership=SimpleNamespace(owners={'target':creation}))
        lane=object.__new__(self.clone._CloneLane);lane.context=context;lane.state='new';lane.owner=None;lane.target_owner=original
        context._clone=lane;lane.held()
        original.closed=True
        with self.assertRaises(RuntimeError):lane.held()
        original.closed=False;context._runner.ownership.owners['target']=copy.deepcopy(creation)
        with self.assertRaises(self.clone.CloneRejected):lane.held()

    def test_three_network_selector_avoids_existing_route_and_exhaustion(self):
        method=getattr(self.clone,'_choose_subnets',None)
        self.assertTrue(callable(method),'missing finite three-network admission')
        self.assertEqual(method(['172.29.0.0/24']),['172.29.1.0/24','172.29.2.0/24','172.29.3.0/24'])
        with self.assertRaises(self.clone.CloneRejected):method(['172.29.0.0/16','10.252.0.0/16'])

    def test_clone_deadline_never_resets_on_exhaustion(self):
        from unittest.mock import patch
        lane=object.__new__(self.clone._CloneLane);lane.deadline=300.0
        with patch.object(self.clone.time,'monotonic',return_value=295.0):self.assertEqual(lane.remaining(120),5.0)
        with patch.object(self.clone.time,'monotonic',return_value=301.0),self.assertRaises(self.clone.CloneRejected):lane.remaining(120)
        self.assertEqual(lane.deadline,300.0)

    def test_clone_send_stack_cannot_inspect_start_stop_or_remove_after_deadline(self):
        from unittest.mock import patch
        factory=getattr(self.clone,'_deadline_runner',None)
        self.assertTrue(callable(factory),'missing whole-stack clone deadline wrapper')
        calls=[]
        class Sink:
            ownership=object();logs=Path('/fixed/logs');counter=0
            def run(self,command,**kwargs):calls.append((command,kwargs));return 0,b'[]\n',b''
        lane=object.__new__(self.clone._CloneLane);lane.deadline=300.0
        wrapped=factory(lane,Sink())
        with patch.object(self.clone.time,'monotonic',return_value=299.0):wrapped.docker('inspect','a'*64,timeout=30)
        self.assertEqual(calls[0][1]['timeout'],1.0)
        with patch.object(self.clone.time,'monotonic',return_value=300.0):
            for verb in ('inspect','start','stop','rm'):
                with self.subTest(verb=verb),self.assertRaises(self.clone.CloneRejected):wrapped.docker(verb,'a'*64,timeout=30)
        self.assertEqual(len(calls),1)

    def test_explicit_clone_profile_owns_three_pg_names_without_broadening_pair(self):
        from p0c4_completion import controlled_fixture as fixture
        from test_p0c4_controlled_fixture_context import ContextPlanTests
        from unittest.mock import patch
        import p0c4_restore_target as target
        context=object.__new__(fixture.ControlledFixtureContext);context._backend=SimpleNamespace(provisioner=target)
        pair=ContextPlanTests().plan();context._plan=pair
        with patch.object(fixture.ControlledFixtureContext,'_alive'):
            self.assertEqual(len(context._owned_pg_names()),2);self.assertFalse(context._clone_profile())
            context._plan=dict(pair,case='wrong-endpoint',capability='c4_current11_same_id_clone_plan_v1',budget_profile='controlled_fixture_same_id_clone_v1',clone_case_id='44444444-4444-4444-8444-444444444444',clone_subnet='172.30.243.0/24')
            self.assertEqual(len(set(context._owned_pg_names())),3);self.assertTrue(context._clone_profile())
            context._plan['budget_profile']='controlled_fixture_two_role_fixed_case_v1'
            with self.assertRaises(self.clone.CloneRejected):context._clone_profile()


@unittest.skipUnless(sys.platform=='linux' and getattr(os,'geteuid',lambda:1)()==0,'Linux root filesystem/UID test; no Docker or PG')
class NativeCloneFilesystem(unittest.TestCase):
    def test_real_postbirth_eof_retains_original_directory_custody(self):
        import fcntl
        import select
        import time
        import uuid
        from test_p0c4_controlled_fixture_context import NativeOwnerTests
        from test_p0c4_restore_target import empty_snapshot,created,SUBNET
        from test_p0c4_restore_target_birth import clean_facts
        fixture=NativeOwnerTests('test_native_setup_holds_actual_same_lock_and_actual_root_inode');fixture.setUp();child=None
        try:
            fixture.owner.close();fixture.root.rename(fixture.root.with_name('target-control'));fixture.root=fixture.root.with_name('target-control')
            fixture.config.update(role='target',root=str(fixture.root),subnet=SUBNET)
            script=Path(fixture.config['source_root'])/'scripts/p0c4_completion/full_target_fs.py'
            child=subprocess.Popen([sys.executable,'-B','-u',str(script),'--fixed-controlled-clone-target-owner'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                                   env={'PATH':'/usr/bin:/bin','PYTHONPATH':str(script.parents[1]),'LC_ALL':'C'})
            import p0c4_restore_target as target
            identity=target.identity_for(fixture.config['target_case_id']);before=empty_snapshot();after=created(identity)
            ids=target.verify_created(identity,SUBNET,before,after)
            status=dict(state='CREATED_QUARANTINED',batch_id=fixture.config['target_case_id'],project=identity['project'],database=identity['database'],network=identity['network'],subnet=SUBNET,**ids)
            operations=[('setup',fixture.config),('pin',dict(before=before)),('files',dict(before=before)),
                        ('created',dict(before=before,after=after,ids=ids)),
                        ('birth',dict(status=status,ids=ids,image_id='sha256:'+'d'*64,started_at='2026-09-29T00:00:00Z',volume_id=[42,43],facts=clean_facts(),daemon_id='daemon-1'))]
            nonce=str(uuid.uuid4());last=None
            for seq,(op,value) in enumerate(operations):
                child.stdin.write(json.dumps(dict(session=nonce,seq=seq,op=op,value=value)).encode()+b'\n');child.stdin.flush()
                self.assertTrue(select.select([child.stdout],[],[],5)[0],'actual owner protocol response deadline')
                last=json.loads(child.stdout.readline());self.assertTrue(last['ok'],last)
            # Synthetic valid birth records exercise the real fixed owner loop;
            # no PG process, Docker fact, or native acceptance is claimed.
            birth_path=Path(last['value']['birth_path']);birth_bytes=birth_path.read_bytes()
            self.assertEqual(hashlib.sha256(birth_bytes).hexdigest(),last['value']['birth_sha256'])
            child.stdin.close();time.sleep(0.15);self.assertIsNone(child.poll())
            with (fixture.root/'.restore-target.lock').open('rb') as ordinary:fcntl.flock(ordinary,fcntl.LOCK_EX|fcntl.LOCK_NB)
            contender=os.open(fixture.root,os.O_RDONLY|os.O_DIRECTORY)
            try:
                with self.assertRaises(BlockingIOError):fcntl.flock(contender,fcntl.LOCK_EX|fcntl.LOCK_NB)
            finally:os.close(contender)
            self.assertEqual(birth_path.read_bytes(),birth_bytes)
            self.assertTrue((fixture.root/'targets'/fixture.config['target_case_id']/'failure.json').is_file())
        finally:
            if child is not None:
                if not child.stdin.closed:child.stdin.close()
                if child.poll() is None:child.terminate()
                child.wait(timeout=5);child.stdout.close();child.stderr.close()
            fixture.tearDown()

    def test_original_directory_flock_survives_creation_lock_handoff_and_disconnect(self):
        import fcntl
        from test_p0c4_controlled_fixture_context import NativeOwnerTests
        fixture=NativeOwnerTests('test_native_setup_holds_actual_same_lock_and_actual_root_inode')
        fixture.setUp()
        try:
            fixture.owner.close();fixture.root.rename(fixture.root.with_name('target-control'));fixture.root=fixture.root.with_name('target-control')
            fixture.config.update(role='target',root=str(fixture.root))
            fixture.owner=fixture.native._new_controlled_clone_target_owner();owner=fixture.owner
            owner.perform('setup',fixture.config);original=owner.root_fd;inode=os.fstat(original).st_ino
            contender=os.open(fixture.root,os.O_RDONLY|os.O_DIRECTORY)
            try:
                with self.assertRaises(BlockingIOError):fcntl.flock(contender,fcntl.LOCK_EX|fcntl.LOCK_NB)
                with (fixture.root/'.restore-target.lock').open('rb') as ordinary:
                    with self.assertRaises(BlockingIOError):fcntl.flock(ordinary,fcntl.LOCK_EX|fcntl.LOCK_NB)
                    # This test exercises lock handoff only; it does not claim a PG birth.
                    owner._birth_complete=True;owner._files_ready=True
                    owner._release_creation_after_birth()
                    fcntl.flock(ordinary,fcntl.LOCK_EX|fcntl.LOCK_NB)
                    self.assertEqual(owner.root_fd,original);self.assertEqual(os.fstat(original).st_ino,inode)
                    owner._recheck();self.assertTrue(owner._disconnect_requires_hold())
                    with self.assertRaises(BlockingIOError):fcntl.flock(contender,fcntl.LOCK_EX|fcntl.LOCK_NB)
                    self.assertEqual(sorted(p.name for p in fixture.root.iterdir()),['.restore-target.lock','targets'])
                    fcntl.flock(ordinary,fcntl.LOCK_UN)
                owner.close();fcntl.flock(contender,fcntl.LOCK_EX|fcntl.LOCK_NB)
            finally:os.close(contender)
        finally:fixture.tearDown()

    def test_capabilityless_root_holds_pgdata_with_opath_without_dac_bypass(self):
        with tempfile.TemporaryDirectory(prefix='clone-opath-') as temp:
            base=Path(temp);base.chmod(0o755);leaf=base/'pgdata';leaf.mkdir(mode=0o700);os.chown(leaf,999,999)
            meta=leaf.stat();expected=dict(path=str(leaf),dev=meta.st_dev,ino=meta.st_ino,uid=999,gid=999,mode=0o700)
            program=r'''
import ctypes,json,os,sys
sys.path.insert(0,sys.argv[1])
from p0c4_completion import controlled_clone as clone
class Header(ctypes.Structure):_fields_=[('version',ctypes.c_uint32),('pid',ctypes.c_int)]
class Data(ctypes.Structure):_fields_=[('effective',ctypes.c_uint32),('permitted',ctypes.c_uint32),('inheritable',ctypes.c_uint32)]
libc=ctypes.CDLL(None,use_errno=True);header=Header(0x20080522,0);data=(Data*2)()
assert libc.capset(ctypes.byref(header),ctypes.byref(data))==0
try:os.open(sys.argv[2],os.O_RDONLY|os.O_DIRECTORY)
except PermissionError:pass
else:raise AssertionError('read permission unexpectedly available')
fd=clone._open_native_inode(__import__('pathlib').Path(sys.argv[2]),json.loads(sys.argv[3]))
assert os.fstat(fd).st_ino==json.loads(sys.argv[3])['ino'];os.close(fd)
print('OPATH_HELD_WITHOUT_DAC')
'''
            result=subprocess.run([sys.executable,'-I','-B','-c',program,str(Path(__file__).parent),str(leaf),json.dumps(expected)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=10)
            os.chown(leaf,0,0)
            self.assertEqual((result.returncode,result.stdout,result.stderr),(0,b'OPATH_HELD_WITHOUT_DAC\n',b''))

    def test_held_clone_directory_rejects_replacement_and_retains_original_fd(self):
        from p0c4_completion import controlled_clone as clone
        with tempfile.TemporaryDirectory(prefix='clone-held-') as temp:
            path=Path(temp)/'root';path.mkdir(mode=0o700);meta=path.stat()
            expected=dict(path=str(path),dev=meta.st_dev,ino=meta.st_ino,uid=meta.st_uid,gid=meta.st_gid,mode=0o700)
            fd=os.open(path,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
            try:
                clone._check_held_inode(fd,path,expected);path.rename(Path(temp)/'retained');path.mkdir(mode=0o700)
                with self.assertRaises(clone.CloneRejected):clone._check_held_inode(fd,path,expected)
                self.assertEqual(os.fstat(fd).st_ino,expected['ino'])
            finally:os.close(fd)


if __name__=='__main__':unittest.main()
