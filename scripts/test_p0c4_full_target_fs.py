"""Fixed helper protocol tests; portable tests are not Linux authority evidence."""
import unittest
import uuid
import os
import sys
from contextlib import contextmanager


@unittest.skipUnless(sys.platform=='linux' and getattr(os,'geteuid',lambda:1)()==0,
                     'actual root Linux initdb contract; portable skips give no authority')
class SetupInitdbContractTests(unittest.TestCase):
    @contextmanager
    def fixture(self,full=False):
        import importlib.util,tempfile
        from pathlib import Path
        from types import SimpleNamespace
        names=('p0c4_restore_target','p0c4_restore_target_pin_prepare',
               'p0c4_restore_target_pin','p0c4_restore_target_birth','p0c4_restore_birth_acceptance')
        previous={name:sys.modules.get(name) for name in names};previous_path=list(sys.path)
        owners=[]
        with tempfile.TemporaryDirectory(prefix='kwc4-setup-contract-',dir='/root') as directory:
            base=Path(directory);batch,source_id,target_id=(str(uuid.uuid4()) for _ in range(3))
            suite=base/'volumes'/('kwc4c-suite-'+uuid.uuid4().hex)/'_data'
            case=suite/'batches'/('batch-'+batch)/source_id if full else suite/'controlled-import/batches'/batch
            source=suite/'source' if full else case/'source'
            repo=Path(__file__).resolve().parents[1]
            files=('scripts/p0c4_completion/full_target_fs.py',*(('scripts/'+name+'.py') for name in names),
                   'deploy/p0c4_restore_initdb.sh','deploy/p0c4_full_restore_initdb.sh')
            for name in files:
                path=source/name;path.parent.mkdir(parents=True,exist_ok=True)
                path.write_bytes((repo/name).read_bytes());path.chmod(0o444 if full else 0o400)
            for path in source.rglob('*'):
                if path.is_dir():path.chmod(0o755 if full else 0o700)
            source.chmod(0o755 if full else 0o700)
            case.mkdir(mode=0o700,parents=True,exist_ok=True)
            runtime=case/'full-profile/initdb.sh' if full else case/'initdb.sh'
            runtime.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
            reviewed=source/'deploy'/('p0c4_full_restore_initdb.sh' if full else 'p0c4_restore_initdb.sh')
            runtime.write_bytes(reviewed.read_bytes());runtime.chmod(0o444)
            roots={'target':base/'volumes'/('learning-system-p0c4-restore-'+target_id+'_control')/'_data'} if full else {role:case/(role+'-control') for role in ('source','target')}
            for root in roots.values():root.mkdir(mode=0o700,parents=True)
            config=dict(batch_id=batch,source_case_id=source_id,target_case_id=target_id,
                        subnet='172.30.241.0/24',root=str(roots['target' if full else 'source']),
                        suite_root=str(suite),source_root=str(source),case_root=str(case))
            if not full:config['role']='source'
            spec=importlib.util.spec_from_file_location('setup_contract_native',source/files[0])
            native=importlib.util.module_from_spec(spec);spec.loader.exec_module(native)
            for name in names:sys.modules.pop(name,None)
            sys.path.insert(0,str(source/'scripts'))
            try:
                yield SimpleNamespace(native=native,config=config,roots=roots,source=source,
                                      reviewed=reviewed,runtime=runtime,owners=owners,full=full)
            finally:
                for owner in reversed(owners):owner.close()
                sys.path[:]=previous_path
                for name,old in previous.items():
                    if old is None:sys.modules.pop(name,None)
                    else:sys.modules[name]=old

    def setup_reply(self,f,role='source'):
        owner=f.native._NativeOwner() if f.full else f.native._new_controlled_owner()
        f.owners.append(owner);config=dict(f.config)
        if not f.full:config.update(role=role,root=str(f.roots[role]))
        protocol=f.native._Protocol(f.native._TOKEN,str(uuid.uuid4()),owner)
        reply=protocol.handle(dict(session=protocol._nonce,seq=0,op='setup',value=config))
        return owner,reply

    def assert_rejected(self,f):
        before=set(os.listdir('/proc/self/fd'))
        owner,reply=self.setup_reply(f,'target' if f.full else 'source')
        self.assertFalse(reply['ok'],reply);self.assertIn(reply['code'],('FS_POLICY','FS_UNREACHABLE'))
        self.assertIsNone(owner.config);self.assertIsNone(owner.root_fd);self.assertIsNone(owner.lock)
        self.assertEqual(set(os.listdir('/proc/self/fd')),before,'rejected reader leaked native FDs')
        if not f.full:
            for root in f.roots.values():self.assertEqual(list(root.iterdir()),[])

    @contextmanager
    def mutate_at_read(self,f,path,mutation):
        from unittest.mock import patch
        original=os.read;identity=(path.stat().st_dev,path.stat().st_ino);hits=[]
        def read(fd,count):
            data=original(fd,count);meta=os.fstat(fd)
            if not hits and (meta.st_dev,meta.st_ino)==identity:
                hits.append(True);mutation()
            return data
        with patch.object(f.native.os,'read',side_effect=read):yield hits

    def test_controlled_source_0400_runtime_0444_reaches_both_held_setups(self):
        import fcntl,stat
        with self.fixture() as f:
            before=f.reviewed.read_bytes()
            for role in ('source','target'):
                owner,reply=self.setup_reply(f,role)
                self.assertTrue(reply['ok'],reply)
                self.assertEqual(set(reply['value']),{'root','lock_held'})
                self.assertIs(reply['value']['lock_held'],True)
                held=os.fstat(owner.root_fd);named=f.roots[role].stat()
                self.assertEqual((held.st_dev,held.st_ino),(named.st_dev,named.st_ino))
                self.assertEqual(reply['value']['root']['ino'],held.st_ino)
                with (f.roots[role]/'.restore-target.lock').open('rb') as other:
                    with self.assertRaises(BlockingIOError):fcntl.flock(other,fcntl.LOCK_EX|fcntl.LOCK_NB)
            self.assertEqual(f.reviewed.read_bytes(),before)
            self.assertEqual(stat.S_IMODE(f.reviewed.stat().st_mode),0o400)
            self.assertEqual(stat.S_IMODE(f.runtime.stat().st_mode),0o444)

    def test_controlled_source_0444_is_rejected_even_with_equal_bytes(self):
        with self.fixture() as f:
            f.reviewed.chmod(0o444);self.assert_rejected(f)

    def test_full_source_0444_and_runtime_0444_keep_actual_legacy_setup(self):
        with self.fixture(full=True) as f:
            owner,reply=self.setup_reply(f,'target');self.assertTrue(reply['ok'],reply)
            self.assertIsNotNone(owner.root_fd)
            self.assertTrue((f.roots['target']/'targets').is_dir())

    def test_full_source_0400_and_runtime_0400_are_each_rejected(self):
        for selected in ('reviewed','runtime'):
            with self.subTest(selected=selected),self.fixture(full=True) as f:
                getattr(f,selected).chmod(0o400);self.assert_rejected(f)

    def test_old_public_runtime_gates_keep_exact_0444(self):
        with self.fixture() as f:
            import p0c4_restore_target as target
            import p0c4_restore_target_pin as pin
            for mode in (0o400,0o440,0o644):
                with self.subTest(mode=oct(mode)):
                    f.runtime.chmod(mode)
                    with self.assertRaises(target.AdmissionError):target._trusted_initdb(f.runtime)
                    with self.assertRaises(target.AdmissionError):pin._initdb_digest(f.runtime)
            f.runtime.chmod(0o444);target._trusted_initdb(f.runtime)
            import hashlib
            self.assertEqual(pin._initdb_digest(f.runtime),hashlib.sha256(f.runtime.read_bytes()).hexdigest())

    def test_pair_rejects_wrong_mode_owner_links_and_special_files(self):
        import signal,time
        for selected in ('reviewed','runtime'):
            for fault in ('mode','writable','owner','links','symlink','fifo','directory'):
                with self.subTest(selected=selected,fault=fault),self.fixture() as f:
                    path=getattr(f,selected)
                    if fault=='mode':path.chmod(0o444 if selected=='reviewed' else 0o400)
                    elif fault=='writable':path.chmod(0o600)
                    elif fault=='owner':os.chown(path,999,999)
                    elif fault=='links':os.link(path,path.with_name('extra-hardlink'))
                    elif fault=='symlink':
                        saved=path.with_name('retained-file');path.rename(saved);path.symlink_to(saved)
                    elif fault=='directory':path.unlink();path.mkdir(mode=0o400)
                    else:path.unlink();os.mkfifo(path,0o400)
                    old=signal.signal(signal.SIGALRM,lambda *args:(_ for _ in ()).throw(AssertionError('FIFO open blocked')))
                    started=time.monotonic();signal.alarm(2)
                    try:self.assert_rejected(f)
                    finally:signal.alarm(0);signal.signal(signal.SIGALRM,old)
                    self.assertLess(time.monotonic()-started,1,'special file reader blocked')

    def test_pair_rejects_untrusted_ancestors(self):
        for selected in ('reviewed','runtime'):
            for fault in ('mode','owner','symlink'):
                with self.subTest(selected=selected,fault=fault),self.fixture() as f:
                    parent=getattr(f,selected).parent
                    if fault=='mode':parent.chmod(0o777)
                    elif fault=='owner':os.chown(parent,999,999)
                    else:
                        saved=parent.with_name(parent.name+'-retained');parent.rename(saved);parent.symlink_to(saved,target_is_directory=True)
                    self.assert_rejected(f)

    def test_same_size_different_content_is_rejected(self):
        with self.fixture() as f:
            content=f.reviewed.read_bytes();f.reviewed.write_bytes(b'X'+content[1:])
            self.assert_rejected(f)

    def test_reader_uses_actual_nonblocking_cloexec_nofollow_fds(self):
        import fcntl
        from unittest.mock import patch
        with self.fixture() as f:
            original=os.open;seen={}
            identities={path.stat().st_ino:name for name,path in (('source',f.reviewed),('runtime',f.runtime))}
            def opened(*args,**kwargs):
                fd=original(*args,**kwargs);meta=os.fstat(fd)
                if meta.st_ino in identities:
                    name=identities[meta.st_ino];seen.setdefault(name,[]).append(args[1])
                    self.assertTrue(fcntl.fcntl(fd,fcntl.F_GETFL)&os.O_NONBLOCK)
                    self.assertTrue(fcntl.fcntl(fd,fcntl.F_GETFD)&fcntl.FD_CLOEXEC)
                    self.assertTrue(args[1]&os.O_NOFOLLOW)
                return fd
            with patch.object(f.native.os,'open',side_effect=opened):
                _,reply=self.setup_reply(f);self.assertTrue(reply['ok'],reply)
            self.assertEqual(set(seen),{'source','runtime'})
            self.assertEqual(len(seen['runtime']),2,'legacy runtime gate plus stable runtime read')

    def test_reader_reads_full_fd_with_short_chunks_and_original_1mib_boundary(self):
        from unittest.mock import patch
        with self.fixture() as f:
            content=b'A'*(1024*1024);f.reviewed.write_bytes(content);f.runtime.write_bytes(content)
            original=os.read
            with patch.object(f.native.os,'read',side_effect=lambda fd,count:original(fd,min(count,8191))):
                _,reply=self.setup_reply(f);self.assertTrue(reply['ok'],reply)

    def test_empty_over_cap_and_short_read_are_rejected(self):
        from unittest.mock import patch
        for content in (b'',b'A'*(1024*1024+1)):
            with self.subTest(size=len(content)),self.fixture() as f:
                f.reviewed.write_bytes(content);self.assert_rejected(f)
        with self.fixture() as f,patch.object(f.native.os,'read',return_value=b''):
            self.assert_rejected(f)

    def test_fd_and_named_file_mutations_reject_without_leaking_fds(self):
        for selected in ('reviewed','runtime'):
            for fault in ('mode','owner','gid','links','size','same-size-content','mtime','replacement'):
                with self.subTest(selected=selected,fault=fault),self.fixture() as f:
                    path=getattr(f,selected);meta=path.stat();raw=path.read_bytes()
                    def mutate():
                        if fault=='mode':path.chmod(0o600)
                        elif fault=='owner':os.chown(path,999,meta.st_gid)
                        elif fault=='gid':os.chown(path,0,999)
                        elif fault=='links':os.link(path,path.with_name('late-hardlink'))
                        elif fault=='size':path.write_bytes(raw+b'X')
                        elif fault=='mtime':os.utime(path,ns=(meta.st_atime_ns,meta.st_mtime_ns+1000000000))
                        elif fault=='same-size-content':
                            path.write_bytes(b'X'+raw[1:]);os.utime(path,ns=(meta.st_atime_ns,meta.st_mtime_ns))
                        else:
                            path.rename(path.with_name('retained-file'));path.write_bytes(raw);path.chmod(meta.st_mode&0o777)
                    with self.mutate_at_read(f,path,mutate) as hits:self.assert_rejected(f)
                    self.assertEqual(hits,[True],'did not reach real FD read mutation')

    def test_source_ancestor_changes_and_runtime_change_after_its_read_reject(self):
        for fault in ('ancestor-mode','ancestor-replacement','runtime-after-read'):
            with self.subTest(fault=fault),self.fixture() as f:
                def mutate():
                    if fault=='ancestor-mode':f.reviewed.parent.chmod(0o777)
                    elif fault=='ancestor-replacement':
                        parent=f.reviewed.parent;parent.rename(parent.with_name('retained-deploy'))
                        parent.mkdir(mode=0o700);f.reviewed.write_bytes(f.runtime.read_bytes());f.reviewed.chmod(0o400)
                    else:f.runtime.write_bytes(b'X'+f.runtime.read_bytes()[1:])
                with self.mutate_at_read(f,f.reviewed,mutate) as hits:self.assert_rejected(f)
                self.assertEqual(hits,[True],'did not reach real source FD read mutation')

    def test_old_runtime_digest_must_match_second_stable_runtime_read(self):
        from unittest.mock import patch
        with self.fixture() as f:
            original=os.fdopen;identity=f.runtime.stat().st_ino;changed=[]
            def fdopen(fd,*args,**kwargs):
                stream=original(fd,*args,**kwargs)
                if os.fstat(fd).st_ino!=identity:return stream
                class Drift:
                    def __enter__(self):stream.__enter__();return self
                    def __exit__(self,*args):return stream.__exit__(*args)
                    def read(self,*args):
                        data=stream.read(*args);raw=b'X'+data[1:]
                        f.runtime.write_bytes(raw);f.reviewed.write_bytes(raw);changed.append(True)
                        return data
                return Drift()
            with patch.object(f.native.os,'fdopen',side_effect=fdopen):self.assert_rejected(f)
            self.assertEqual(changed,[True],'old runtime gate did not perform real original read')

class ProtocolTests(unittest.TestCase):
    def protocol(self):
        from scripts.p0c4_completion import full_target_fs as fs
        class Backend:
            def __init__(self):self.locked=False;self.calls=[]
            def perform(self,op,value):
                self.calls.append(op)
                if op=='setup':self.locked=True
                if op in ('pin','files','created','check','birth'):assert self.locked
                if op=='release':self.locked=False
                # Secret material is generated by native backend, not sent by protocol.
                if op=='setup':return dict(root=dict(path="/owned",dev=1,ino=2,uid=0,gid=0,mode=0o700),lock_held=True)
                if op=='pin':return dict(precreation_sha256='a'*64)
                if op=='files':return dict(compose_sha256='a'*64)
                if op in ('created','failure'):return dict(recorded=True)
                if op=='check':return dict(checked=True)
                if op=='birth':return dict(birth_sha256='a'*64,issuance_sha256='a'*64,birth_path='/root/birth',destination_root='/root/dest',control_root='/root/control',asset_root='/root/assets')
                if op=='release':return dict(released=True)
        backend=Backend();nonce=str(uuid.uuid4())
        return fs,fs._Protocol(fs._TOKEN,nonce,backend),backend,nonce

    def test_fixed_sequence_holds_owner_until_birth_and_release(self):
        fs,owner,backend,nonce=self.protocol()
        for seq,op in enumerate(('setup','pin','files','created','check','birth','release')):
            reply=owner.handle(dict(session=nonce,seq=seq,op=op,value={}))
            self.assertTrue(reply['ok'])
            self.assertEqual(reply['session'],nonce)
            self.assertEqual(backend.locked,op!='release')
        self.assertEqual(backend.calls,['setup','pin','files','created','check','birth','release'])

    def test_reorder_replay_bad_nonce_or_opcode_latches_rejection(self):
        for alteration in ('nonce','order','replay','op'):
            fs,owner,backend,nonce=self.protocol()
            owner.handle(dict(session=nonce,seq=0,op='setup',value={}))
            request=dict(session=nonce,seq=1,op='pin',value={})
            if alteration=='nonce':request['session']=str(uuid.uuid4())
            if alteration=='order':request['op']='birth'
            if alteration=='replay':request['seq']=0
            if alteration=='op':request['op']='exec'
            self.assertFalse(owner.handle(request)['ok'])
            self.assertFalse(owner.handle(dict(session=nonce,seq=2,op='files',value={}))['ok'])
            self.assertEqual(backend.calls,['setup'])

    def test_failure_response_has_no_exception_or_secret(self):
        fs,owner,backend,nonce=self.protocol()
        def failed(op,value):raise OSError('postgresql://private:SECRET@example')
        backend.perform=failed
        reply=owner.handle(dict(session=nonce,seq=0,op='setup',value={}))
        self.assertFalse(reply['ok'])
        self.assertNotIn('SECRET',str(reply));self.assertNotIn('postgresql',str(reply))

    def test_unexpected_response_fields_cannot_export_generated_secrets(self):
        fs,owner,backend,nonce=self.protocol()
        backend.perform=lambda op,value:dict(root={},lock_held=True,postgres_password='SECRET')
        reply=owner.handle(dict(session=nonce,seq=0,op='setup',value={}))
        self.assertFalse(reply['ok']);self.assertNotIn('SECRET',str(reply))

class ProfileTests(unittest.TestCase):
    def test_owner_transport_failure_latches_and_never_replays(self):
        from scripts.p0c4_completion import full_target_fs as fs
        from types import SimpleNamespace
        for error in (EOFError(),TimeoutError()):
            calls=[]
            def call(value):calls.append(value);raise error
            client=fs._OwnerClient(fs._TOKEN,SimpleNamespace(close=lambda:None),SimpleNamespace(call=call))
            with self.assertRaises(type(error)):client.call('setup',{})
            with self.assertRaises(fs.FsRejected):client.call('setup',{})
            with self.assertRaises(fs.FsRejected):client.held()
            client.close();self.assertEqual(len(calls),1)

    def test_capability_uid_and_mount_profile_is_exact(self):
        from scripts.p0c4_completion import full_target_fs as fs
        import copy
        mounts=[('volume','owned','/owned',False),('volume','suite','/suite',True)]
        command=['python3','fixed-owner']
        spec=fs._HelperProfile(fs._TOKEN,'owner',mounts,command,True)
        spec._request(mounts,command,True,False,None)
        for changed in [mounts+[('bind','/','/host',False)],mounts[::-1]]:
            with self.assertRaises(fs.FsRejected):spec._request(changed,command,True,False,None)
        facts={'Config':{'User':'0:0'},'HostConfig':{'CapAdd':['CHOWN'],'CapDrop':['ALL'],'Privileged':False,'NetworkMode':'none','PidMode':''}}
        self.assertIsNone(spec._checked_default_copy(facts)['HostConfig']['CapAdd'])
        self.assertEqual(facts['HostConfig']['CapAdd'],['CHOWN'])
        for key,value in [('CapAdd',['CHOWN','DAC_OVERRIDE']),('Privileged',True),('NetworkMode','host'),('PidMode','host')]:
            bad=copy.deepcopy(facts);bad['HostConfig'][key]=value
            with self.assertRaises(fs.FsRejected):spec._checked_default_copy(bad)
        pg=fs._HelperProfile(fs._TOKEN,'pg',[('volume','exact-pg','/pg',True)],['python3','fixed-measure'],True)
        facts['Config']['User']='999:999';facts['HostConfig']['CapAdd']=None
        pg._checked_default_copy(facts)
        facts['Config']['User']='0:0'
        with self.assertRaises(fs.FsRejected):pg._checked_default_copy(facts)

    def test_same_measurement_program_rejects_extra_volume_or_foreign_uuid(self):
        from scripts.p0c4_completion import full_target_fs as fs
        # Load only its fixed stdlib function definitions, never native main.
        scope={'__name__':'contract_test'};exec(compile(fs.MEASURE_PROGRAM,'fixed-measure','exec'),scope)
        source='550e8400-e29b-41d4-a716-446655440000';target='650e8400-e29b-41d4-a716-446655440000'
        name='learning-system-p0c4-restore-'+target+'_pg'
        value=dict(phase='birth',source_case_id=source,target_case_id=target,roots=dict(target_pg=dict(name=name,path='/var/lib/docker/volumes/'+name+'/_data')))
        scope['validate'](value)
        import copy
        for mutation in ('uuid','volume','extra','escape','backslash'):
            bad=copy.deepcopy(value)
            if mutation=='uuid':bad['target_case_id']=source
            elif mutation=='volume':bad['roots']['target_pg']['name']='foreign'
            elif mutation=='extra':bad['roots']['source_pg']=bad['roots']['target_pg']
            elif mutation=='escape':bad['roots']['target_pg']['path']='/var/../volumes/'+name+'/_data'
            else:bad['roots']['target_pg']['path']='/var/a\\b/volumes/'+name+'/_data'
            with self.assertRaises((RuntimeError,ValueError),msg=mutation):scope['validate'](bad)

class CapabilityAliasTests(unittest.TestCase):
    def helper(self,kind='owner'):
        from p0c4_completion import full_target_fs as fs
        import p0c4_source_lifecycle_gate_acceptance as driver
        from scripts.test_p0c4_source_binding_gate_acceptance import gate,helper,helper_facts
        facts,expected=helper_facts()
        mounts=[('volume','owned','/owned',kind!='owner'),('volume','suite','/suite',True)]
        facts['Mounts']=[dict(Type='volume',Name='owned',Source='/docker/volumes/owned',Destination='/owned',RW=kind=='owner'),
                         dict(Type='volume',Name='suite',Source='/docker/volumes/suite',Destination='/suite',RW=False)]
        facts['HostConfig']['Mounts']=[dict(Target='/owned',VolumeOptions={'NoCopy':True}),dict(Target='/suite',VolumeOptions={'NoCopy':True})]
        expected['mounts']={('volume','/docker/volumes/owned','/owned',kind=='owner'),('volume','/docker/volumes/suite','/suite',False)}
        expected['volume_names']={'/owned':'owned','/suite':'suite'}
        facts['Config']['User']='999:999' if kind=='pg' else '0:0'
        facts['HostConfig']['CapAdd']=['CHOWN'] if kind=='owner' else None
        spec=fs._HelperProfile(fs._TOKEN,kind,mounts,['python3','-c','fixture'],False)
        spec._request(mounts,['python3','-c','fixture'],False,False,None)
        # Exercise the real validation boundary without starting a container.
        container=object.__new__(driver.HelperContainer)
        container._full_profile=spec;container.fs=False
        container.h,container.b,container.expected=helper,gate,expected
        return fs,container,facts

    def test_owner_accepts_exact_chown_aliases_without_mutating_facts(self):
        import copy
        for capability in ('CHOWN','CAP_CHOWN'):
            with self.subTest(capability=capability):
                fs,container,facts=self.helper()
                facts['HostConfig']['CapAdd']=[capability];before=copy.deepcopy(facts)
                try:adjusted=container._full_profile._checked_default_copy(facts)
                except fs.FsRejected:self.fail('authorized singleton '+capability+' was rejected')
                expected=copy.deepcopy(before);expected['HostConfig']['CapAdd']=None
                self.assertEqual(adjusted,expected)
                self.assertIs(container.validate(facts),facts)
                self.assertEqual(facts,before)
                adjusted['Config']['User']='999:999'
                adjusted['HostConfig']['CapDrop'].append('CHOWN')
                adjusted['Mounts'][0]['RW']=False
                self.assertEqual(facts,before)

    def test_owner_rejects_missing_extra_duplicate_or_malformed_capabilities(self):
        import copy
        missing=object()
        values=(missing,None,[],['DAC_OVERRIDE'],['CAP_DAC_OVERRIDE'],
                ['CHOWN','DAC_OVERRIDE'],['CAP_CHOWN','DAC_OVERRIDE'],
                ['CHOWN','CAP_CHOWN'],['CAP_CHOWN','CHOWN'],
                ['CHOWN','CHOWN'],['CAP_CHOWN','CAP_CHOWN'],
                ['chown'],['cap_chown'],['CAP_CHOWN '],
                'CHOWN','CAP_CHOWN',{'CHOWN':True},{'CAP_CHOWN':True},
                ('CHOWN',),('CAP_CHOWN',),0,True,[None],[True],[1],[['CHOWN']])
        for index,value in enumerate(values):
            with self.subTest(case=index):
                fs,container,facts=self.helper()
                if value is missing:facts['HostConfig'].pop('CapAdd')
                else:facts['HostConfig']['CapAdd']=value
                before=copy.deepcopy(facts)
                with self.assertRaises(fs.FsRejected):container.validate(facts)
                self.assertEqual(facts,before)

    def test_owner_aliases_preserve_user_security_and_mount_rejections(self):
        import copy
        for capability in ('CHOWN','CAP_CHOWN'):
            fs,container,valid=self.helper();valid['HostConfig']['CapAdd']=[capability]
            cases=[('Config','User','999:999','FS_POLICY'),
                   ('HostConfig','NetworkMode','host','FS_POLICY'),
                   ('HostConfig','Privileged',True,'FS_POLICY'),
                   ('HostConfig','PidMode','host','FS_POLICY'),
                   ('HostConfig','CapDrop',[],'FS_POLICY'),
                   ('HostConfig','ReadonlyRootfs',False,'HELPER_BUDGET'),
                   ('HostConfig','SecurityOpt',[],'HELPER_SECURITY'),
                   ('Mounts',0,dict(valid['Mounts'][0],Source='/foreign'),'HELPER_MOUNTS'),
                   ('Mounts',0,dict(valid['Mounts'][0],RW=False),'HELPER_MOUNTS'),
                   ('Mounts',0,dict(valid['Mounts'][0],Name='foreign'),'HELPER_VOLUME_NAME'),
                   ('HostConfig','Mounts',[dict(Target='/owned',VolumeOptions={'NoCopy':False})],'HELPER_VOLUME_NOCOPY')]
            for section,key,value,code in cases:
                with self.subTest(capability=capability,section=section,key=key,code=code):
                    facts=copy.deepcopy(valid);facts[section][key]=value;before=copy.deepcopy(facts)
                    with self.assertRaisesRegex((fs.FsRejected,container.b.GateError),code):container.validate(facts)
                    self.assertEqual(facts,before)

    def test_pg_and_roots_keep_only_absent_or_empty_capabilities(self):
        import copy
        missing=object()
        for kind in ('pg','roots'):
            for value in (missing,None,[],['CHOWN'],['CAP_CHOWN'],['CHOWN','CAP_CHOWN'],'CHOWN',{},()):
                with self.subTest(kind=kind,capability=value):
                    fs,container,facts=self.helper(kind)
                    if value is missing:facts['HostConfig'].pop('CapAdd')
                    else:facts['HostConfig']['CapAdd']=value
                    before=copy.deepcopy(facts)
                    if value is missing or value is None or value==[]:
                        adjusted=container._full_profile._checked_default_copy(facts)
                        expected=copy.deepcopy(before);expected['Config']['User']='0:0';expected['HostConfig']['CapAdd']=None
                        self.assertEqual(adjusted,expected)
                        self.assertIs(container.validate(facts),facts)
                    else:
                        with self.assertRaises(fs.FsRejected):container.validate(facts)
                    self.assertEqual(facts,before)
            fs,container,facts=self.helper(kind)
            facts['Config']['User']='0:0' if kind=='pg' else '999:999'
            with self.assertRaises(fs.FsRejected):container.validate(facts)

class FilesystemContractTests(unittest.TestCase):
    def test_owner_eof_holds_possible_target_without_accepting_more_input_or_publication(self):
        from scripts.p0c4_completion import full_target_fs as fs
        from types import SimpleNamespace
        from unittest.mock import patch
        import io,signal
        class EndObservation(BaseException):pass
        for birth in (False,True):
            owner=fs._NativeOwner();owner.root_fd=123;owner._files_ready=True;owner._birth_complete=birth
            actions=[]
            def perform(op,value):actions.append((op,value));return dict(recorded=True)
            owner.perform=perform
            with patch.object(owner,'close') as close,patch.object(fs,'_NativeOwner',return_value=owner),patch.object(fs.sys,'stdin',SimpleNamespace(buffer=io.BytesIO(b''))),patch.object(signal,'pause',side_effect=EndObservation,create=True) as pause:
                if birth:
                    self.assertEqual(fs._owner_main(),1);close.assert_called_once();pause.assert_not_called()
                    self.assertEqual(actions,[])
                else:
                    with self.assertRaises(EndObservation):fs._owner_main()
                    close.assert_not_called();pause.assert_called_once()
                    self.assertEqual(actions,[('failure',dict(code='ISOLATION_UNCONFIRMED'))])
                    self.assertTrue(owner._disconnect_requires_hold())

    def test_owner_held_root_replacement_and_permission_change_reject(self):
        from scripts.p0c4_completion import full_target_fs as fs
        from types import SimpleNamespace
        from unittest.mock import patch
        root=dict(path='/owned',dev=1,ino=2,uid=0,gid=0,mode=0o700)
        owner=fs._NativeOwner();owner.config={'root':'/owned'};owner.root_fd=123;owner.root_identity=root
        with patch.object(fs,'_identity',return_value=dict(root)),patch.object(fs.os,'fstat',return_value=SimpleNamespace(st_dev=1,st_ino=2)):
            owner._recheck()
        for key,value in [('dev',2),('ino',3),('uid',999),('mode',0o777)]:
            with patch.object(fs,'_identity',return_value=dict(root,**{key:value})),patch.object(fs.os,'fstat',return_value=SimpleNamespace(st_dev=1,st_ino=2)):
                with self.assertRaises(fs.FsRejected):owner._recheck()
        with patch.object(fs,'_identity',return_value=dict(root)),patch.object(fs.os,'fstat',return_value=SimpleNamespace(st_dev=1,st_ino=3)):
            with self.assertRaises(fs.FsRejected):owner._recheck()

    def test_measurement_walk_rejects_symlink_and_replaced_directory_and_closes_fds(self):
        from scripts.p0c4_completion import full_target_fs as fs
        from types import SimpleNamespace
        import stat
        scope={'__name__':'contract_test'};exec(compile(fs.MEASURE_PROGRAM,'fixed-measure','exec'),scope)
        def meta(ino,mode=stat.S_IFDIR|0o700):return SimpleNamespace(st_dev=1,st_ino=ino,st_uid=999,st_gid=999,st_mode=mode,st_size=0)
        for scenario in ('symlink','replacement','normal'):
            closed=[]
            def fstat(fd):return meta(4 if fd==12 else 2)
            def observed(name,**kwargs):
                if name=='/pg':return meta(2)
                return meta(3,stat.S_IFLNK|0o777 if scenario=='symlink' else stat.S_IFDIR|0o700)
            fake=SimpleNamespace(O_RDONLY=0,O_DIRECTORY=1,O_NOFOLLOW=2,O_CLOEXEC=4,
                fstat=fstat,dup=lambda fd:11,listdir=lambda fd:['child'] if fd==11 else [],stat=observed,
                open=lambda *a,**kw:12,close=lambda fd:closed.append(fd))
            scope['os']=fake;scope['open_root']=lambda path:10
            if scenario=='normal':fake.fstat=lambda fd:meta(3 if fd==12 else 2)
            if scenario=='normal':
                result=scope['measure']('/pg');self.assertEqual(result['entries'],1)
            else:
                with self.assertRaises(RuntimeError):scope['measure']('/pg')
            self.assertEqual(len(closed),len(set(closed)))
            self.assertIn(10,closed);self.assertIn(11,closed)
            if scenario!='symlink':self.assertIn(12,closed)

    def test_full_compose_has_only_exact_daemon_bind_inputs_and_legacy_is_unchanged(self):
        from scripts.p0c4_completion import full_target_fs as fs
        import p0c4_restore_target as target
        from pathlib import Path
        from scripts.test_p0c4_full_rehearsal_profile import T
        identity=target.identity_for(T);root=Path('/unmounted/targets')/T
        original=target.compose_document(identity,'10.254.21.0/24',root,Path('/suite/initdb.sh'))
        import copy
        document=copy.deepcopy(original);fs._extend_document(document,identity,root)
        pg=document['services']['pg']
        self.assertNotIn('secrets',document);self.assertNotIn('secrets',pg)
        self.assertEqual(pg['container_name'],identity['project']+'-pg-1')
        self.assertEqual(set(document),{'name','services','networks','volumes'})
        self.assertFalse(set(pg)&{'env_file','build','configs','extends'})
        binds=[row for row in pg['volumes'] if type(row) is dict and row['type']=='bind']
        self.assertEqual(binds,[dict(type='bind',source=str(root/'secrets'/key),target='/run/secrets/'+key,read_only=True,bind={'create_host_path':False}) for key in ('postgres_password','admin_password')])
        self.assertEqual(original,target.compose_document(identity,'10.254.21.0/24',root,Path('/suite/initdb.sh')))

class BoundedIntegrationTests(unittest.TestCase):
    def test_actual_inventory_scale_preserves_safe_projection_and_original_budget(self):
        import json,tempfile
        from pathlib import Path
        from types import SimpleNamespace
        from unittest.mock import patch
        import p0c4_source_lifecycle_gate_acceptance as driver
        import p0c4_controlled_import_acceptance as bounded
        import p0c4_maintenance_gate_acceptance as metadata
        from scripts.test_p0c4_full_rehearsal_profile import Fix1AuthorityTests
        context,_=Fix1AuthorityTests().issued()
        calls=[];mode=['normal']
        with tempfile.TemporaryDirectory() as temp:
            batch=Path(temp);logs=batch/'evidence/logs';logs.mkdir(parents=True)
            class Transport:
                def __init__(self,path):self.logs=path;self.counter=0
                def run(self,command,**kwargs):
                    self.counter+=1;calls.append((list(command),kwargs))
                    index=command.index('--format');kind='container' if command[1]=='inspect' else command[1]
                    self_format=command[index+1]
                    assert self_format==metadata._inventory_format(kind)
                    assert all(value not in self_format for value in ('.Env','.Cmd','.Mounts'))
                    ids=command[index+2:];assert 1<=len(ids)<=64
                    rows=[]
                    for identity in ids:
                        row=dict(Name=identity if kind=='volume' else '/foreign-'+identity[:12] if kind=='container' else 'foreign-'+identity[:12],Project=None)
                        if kind!='volume':row['Id']=identity
                        if kind=='network':row['Subnets']=['172.20.0.0/16']
                        rows.append(row)
                    if mode[0]=='missing':rows.pop()
                    if mode[0]=='duplicate':rows.append(rows[0])
                    if mode[0]=='secret':rows[0]['Env']=['SECRET=never-export']
                    out=b''.join(json.dumps(row).encode()+b'\n' for row in rows)
                    for suffix,raw in [('stdout',out),('stderr',b''),('process.json',driver.canonical(dict(exit_code=0,reason=None,stdout_bytes=len(out),stderr_bytes=0)))]:
                        (logs/(f'{self.counter:04d}.'+suffix)).write_bytes(raw)
                    return 0,out,b''
            base=driver.owned_log_runner(SimpleNamespace(Runner=Transport),batch,dict(batch_id='batch',helpers=[]),0)
            self.addCleanup(driver.LOG_RESERVATIONS.pop,str(logs),None)
            budget=driver.CaseBudget(clock=lambda:100,total_deadline=281.25)
            context._runner=driver.BudgetRunner(base,budget)
            for kind,count in [('container',934),('network',270),('volume',903)]:
                ids=[f'{i:064x}' if kind!='volume' else 'volume-'+str(i) for i in range(count)]
                rows=context._inventory(kind,ids)
                self.assertEqual(len(rows),count)
            self.assertEqual(len(calls),35)
            self.assertTrue(all(row[1]['timeout']==1.25 for row in calls))
            self.assertEqual(len(list(logs.iterdir())),105)
            for mode[0] in ('missing','duplicate','secret'):
                with self.assertRaises(Exception):context._inventory('volume',['owned'])
            mode[0]='normal';count=len(calls)
            with self.assertRaises(Exception):context._inventory('volume',['owned','owned'])
            with self.assertRaises(Exception):context._inventory('volume',['v'+str(i) for i in range(4097)])
            self.assertEqual(len(calls),count)
            with patch('time.monotonic',side_effect=[0,31]):
                with self.assertRaises(Exception):context._inventory('volume',['owned'])
            self.assertEqual(len(calls),count)
            with patch.object(bounded,'INSPECT_TOTAL_STDOUT_LIMIT',1):
                with self.assertRaises(Exception):context._inventory('volume',['owned'])
            # Capacity is reserved before issuing any inventory RPC.
            driver.LOG_RESERVATIONS[str(logs)]['pending']=8192;count=len(calls)
            with self.assertRaises(Exception):context._inventory('volume',['owned'])
            self.assertEqual(len(calls),count)

    def test_target_cleanup_uses_reserved_original_runner_and_never_marks_live_retained(self):
        import json,tempfile
        from pathlib import Path
        from types import SimpleNamespace
        from scripts.test_p0c4_full_rehearsal_profile import Fix1AuthorityTests,T
        import p0c4_source_lifecycle_gate_acceptance as driver
        import p0c4_restore_target as target
        for variant in ('known','unknown','wrong-label','stop-timeout','attached'):
            with self.subTest(variant=variant),tempfile.TemporaryDirectory() as temp:
                context,_=Fix1AuthorityTests().issued();context._target_identity=target.identity_for(T);context._record={}
                logs=Path(temp)/'evidence/logs';logs.mkdir(parents=True)
                project=context._target_identity['project'];cid='a'*64;nid='b'*64;calls=[]
                state={'running':True}
                class Transport:
                    def __init__(self,path):self.logs=path;self.counter=0
                    def run(self,command,**kwargs):
                        self.counter+=1;calls.append((command,kwargs));args=command[1:]
                        if args[0]=='stop':
                            if variant!='stop-timeout':state['running']=False
                            out=(cid+'\n').encode()
                        elif args[0]=='container':
                            row=dict(Id=cid,Name='/'+project+'-pg-1',Config=dict(Labels={'com.docker.compose.project':'foreign' if variant=='wrong-label' else project,'com.docker.compose.service':'pg'}),State=dict(Running=state['running'],Pid=123 if state['running'] else 0),ExecIDs=[])
                            out=json.dumps([row]).encode()
                        elif args[0]=='network':
                            out=json.dumps([dict(Id=nid,Name=project+'_test',Labels={'com.docker.compose.project':project},Containers={'foreign':{}} if variant=='attached' else {})]).encode()
                        else:raise AssertionError(command)
                        for suffix,raw in [('stdout',out),('stderr',b''),('process.json',driver.canonical(dict(exit_code=0,reason=None,stdout_bytes=len(out),stderr_bytes=0)))]:
                            (logs/(f'{self.counter:04d}.'+suffix)).write_bytes(raw)
                        return 0,out,b''
                base=driver.owned_log_runner(SimpleNamespace(Runner=Transport),Path(temp),dict(batch_id='batch',helpers=[]),0)
                self.addCleanup(driver.LOG_RESERVATIONS.pop,str(logs),None)
                budget=driver.CaseBudget();budget.deadline=0;budget.transition('CLEANUP')
                context._runner=driver.BudgetRunner(base,budget);ledger=base.ownership;ledger.enter(budget)
                owner=ledger.acquire('pg',project+'-pg-1',{},dict(name=project+'_test'),{})
                context._target_owner=owner;context._target_created_ids=dict(network_id=nid)
                if variant=='unknown':owner['state']='sent'
                else:ledger.known(owner,cid)
                # Ordinary capacity is now unavailable: the original PG cleanup
                # tokens must be consumed instead of unreserved operations.
                remaining=len(driver.LOG_RESERVATIONS[str(logs)]['cleanup'][project+'-pg-1'])
                driver.LOG_RESERVATIONS[str(logs)]['pending']=8192-3*remaining
                record={}
                if variant in ('known','unknown'):
                    context._cleanup_target(record)
                    self.assertEqual(owner['state'],'stopped');self.assertFalse(state['running'])
                    self.assertGreaterEqual(len(owner['record']['cleanup_rpcs']['pg']),3)
                else:
                    with self.assertRaises(Exception):context._cleanup_target(record)
                    self.assertNotIn(owner['state'],('retained','stopped'))
                    self.assertEqual(record['preparation_cleanup'],'ISOLATION_UNCONFIRMED')
                self.assertLessEqual(len(calls),7)
                self.assertFalse(any('ls' in row[0] for row in calls))

    def test_uid_measurement_uses_only_known_target_and_rejects_wrong_missing_or_failed_exec(self):
        from scripts.test_p0c4_full_rehearsal_profile import _SocketProfileHarness
        for output in (b'999\n999\n',b'0\n0\n',b'',TimeoutError()):
            harness=_SocketProfileHarness(self,uid_output=output)
            context=harness.context;cid=harness.cid
            self.assertEqual(context._postgres_uid(),(999,999));self.assertEqual(harness.calls,[])
            if output==b'999\n999\n':
                context._configure(context._target_identity,cid)
                self.assertEqual(context._record['target_pg_identity'],dict(container_id=cid,uid=999,gid=999))
                self.assertEqual(context._record['target_socket_stage'],'VERIFIED')
            else:
                with self.assertRaises((harness.full.FullImportError,TimeoutError)):context._configure(context._target_identity,cid)
                self.assertNotIn('target_pg_identity',context._record)
                self.assertEqual(context._phase,'PG_UID_CHECK')
            uid_calls=[call['argv'][1:] for call in harness.calls if call['argv'][-1]=='id -u postgres; id -g postgres']
            self.assertEqual(uid_calls,[['exec','--user','0:0',cid,'sh','-c','id -u postgres; id -g postgres']])
            before=len(harness.calls)
            with self.assertRaises(harness.full.FullImportError):context._configure(context._target_identity,'e'*64)
            self.assertEqual(len(harness.calls),before)

    def test_early_preparation_failure_preserves_primary_and_enters_cleanup_budget(self):
        import tempfile
        from pathlib import Path
        from types import SimpleNamespace
        from unittest.mock import patch
        import p0c4_source_lifecycle_gate_acceptance as driver
        from p0c4_completion import full_import,evidence
        from scripts.test_p0c4_full_rehearsal_profile import Fix1AuthorityTests,S
        _,plan=Fix1AuthorityTests().issued()
        with patch.object(evidence,'private_read',return_value=full_import._canonical(plan)):
            context=full_import.read_context('/root/case/plan.json','full-import')
        with tempfile.TemporaryDirectory() as temp:
            batch=Path(temp);logs=batch/'evidence/logs';logs.mkdir(parents=True)
            driver.LOG_RESERVATIONS[str(logs)]=dict(archives=0,ledgers=0,pending=0,audits=0)
            self.addCleanup(driver.LOG_RESERVATIONS.pop,str(logs),None)
            result=dict(batch_id=plan['batch_id'],cases=[],helpers=[])
            runner=SimpleNamespace(logs=logs);runner.ownership=driver.CleanupReservations(runner,result)
            h=SimpleNamespace(INITDB=b'GRANT learning_auth_lock TO learning_admin WITH SET TRUE;\n',mkdir_new=lambda path:path.mkdir(),write_new=lambda path,raw,mode:path.write_bytes(raw))
            def unavailable(self,h,b,wrapped,batch,source,case,ident,images,result,record,files):
                self._runner=wrapped;self._record=record;self._phase='TARGET_FILES'
                wrapped.budget.deadline=0
                raise OSError('PRIVATE-PATH-MUST-NOT-APPEAR')
            observations=[]
            def isolate(self,record):
                observations.append((self._runner.budget.phase,runner.ownership.deadline,self._runner.budget.deadline))
                raise RuntimeError('PRIVATE-CLEANUP-TEXT')
            with patch.object(full_import.FullRehearsalContext,'_prepare',unavailable),patch.object(full_import.FullRehearsalContext,'_cleanup_target',isolate):
                with self.assertRaises(OSError):driver.live_case(h,None,None,runner,batch,batch/'source',dict(case_id=S),plan['subnet'],context._selected_test(),{},result,{}, {},plan['source']['application_build_sha256'],False,_full_context=context)
            self.assertEqual(len(observations),1)
            self.assertEqual(observations[0][0],'CLEANUP');self.assertEqual(observations[0][1],observations[0][2])
            record=result['cases'][0]
            self.assertEqual(record['preparation_failure'],dict(phase='TARGET_FILES',code='PREPARATION_REJECTED'))
            self.assertEqual(record['preparation_cleanup'],'ISOLATION_UNCONFIRMED')
            self.assertNotIn('PRIVATE-',str(record))
            context._phase='COMPLETE'
            self.assertEqual(context._failure_observation(),record['preparation_failure'])

    def test_unknown_target_retains_only_its_exact_filesystem_owner(self):
        import tempfile
        from pathlib import Path
        from types import SimpleNamespace
        from unittest.mock import patch
        import p0c4_source_lifecycle_gate_acceptance as driver
        from p0c4_completion import full_import,evidence,full_target_fs as fs
        from scripts.test_p0c4_full_rehearsal_profile import Fix1AuthorityTests
        _,plan=Fix1AuthorityTests().issued()
        for broken in (False,True):
            with self.subTest(broken=broken),tempfile.TemporaryDirectory() as temp:
                with patch.object(evidence,'private_read',return_value=full_import._canonical(plan)):
                    context=full_import.read_context('/root/case/plan.json','full-import')
                result=dict(batch_id=plan['batch_id'],helpers=[],cases=[dict(stopped=False,execs_absent=False)],preflight_complete=True)
                registry=driver.CleanupReservations(SimpleNamespace(logs=Path(temp)),result);registry._full_pair=context
                base=SimpleNamespace(ownership=registry)
                context._record={};context._target_owner=dict(state='unknown');calls=[]
                def helper(name,profile):
                    record=dict(name=name,id='a'*64,removed=False);result['helpers'].append(record)
                    obj=SimpleNamespace(closed=False,_full_profile=profile,record=record)
                    def close(phase):calls.append(name);obj.closed=True;record['removed']=True
                    obj.close=close
                    registry.owners[name]=dict(intent=dict(kind='helper'),state='known',object=obj,record=record)
                    return obj
                held=helper('fixed-owner',fs._HelperProfile(fs._TOKEN,'owner',[],[],True))
                ordinary=helper('ordinary',None)
                child=SimpleNamespace(retired=False,retire_attempted=False,finished=False,reason=None)
                retired=[]
                def retire(**kwargs):child.retired=True;retired.append('CLI_ONLY')
                child.retire=retire
                client=fs._OwnerClient(fs._TOKEN,held,SimpleNamespace(child=child,broken=broken));client._state='failure';client._failure='HELPER_PROTOCOL' if broken else None
                context._fs_owner=client
                errors=driver.cleanup_known_helpers(base,result)
                self.assertEqual(calls,['ordinary'])
                self.assertIn('FULL_FS_OWNER_RETAINED',errors)
                self.assertFalse(driver.cleanup_verified(result))
                self.assertFalse(held.record['removed'])
                observed=held.record['owner_retained']
                self.assertEqual(observed['lock_state'],'UNKNOWN')
                self.assertEqual(retired,['CLI_ONLY'])
                self.assertTrue(client._broker.broken)
                self.assertTrue(observed['no_replay'])
                self.assertNotIn('birth_sha256',observed)

    def test_exact_full_pair_does_not_change_default_single_pg_limit(self):
        import tempfile
        from pathlib import Path
        from types import SimpleNamespace
        from unittest.mock import patch
        import p0c4_source_lifecycle_gate_acceptance as driver
        from p0c4_completion import full_import,evidence
        from scripts.test_p0c4_full_rehearsal_profile import Fix1AuthorityTests,S,T
        _,plan=Fix1AuthorityTests().issued()
        with patch.object(evidence,'private_read',return_value=full_import._canonical(plan)):
            context=full_import.read_context('/root/case/plan.json','full-import')
        import p0c4_restore_target as target
        context._target_identity=target.identity_for(T);context._ident=driver.completion_case_identity(S)
        with tempfile.TemporaryDirectory() as temp:
            logs=Path(temp);driver.LOG_RESERVATIONS[str(logs)]=dict(archives=0,ledgers=0,pending=0,audits=0)
            self.addCleanup(driver.LOG_RESERVATIONS.pop,str(logs),None)
            ledger=driver.CleanupReservations(SimpleNamespace(logs=logs),dict(batch_id=plan['batch_id'],helpers=[]))
            first=ledger.acquire('pg',context._ident['pg_name'],{},None,{})
            ledger.known(first,'a'*64)
            with self.assertRaises(driver.GateError):ledger.acquire('pg',context._owned_pg_names()[1],{},None,{})
            ledger._full_pair=context
            with self.assertRaises(driver.GateError):ledger.acquire('pg','foreign',{},None,{})
            second=ledger.acquire('pg',context._owned_pg_names()[1],{},None,{})
            ledger.known(second,'b'*64)
            with self.assertRaises(driver.GateError):ledger.acquire('pg',context._owned_pg_names()[0],{},None,{})
            self.assertEqual([first['state'],second['state']],['known','known'])

if __name__=='__main__':unittest.main()
