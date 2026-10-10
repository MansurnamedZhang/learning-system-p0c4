"""Pure format tests; Linux root/inode cases run in the fixed acceptance namespace."""
import unittest
import os
import sys
import uuid
import copy
import json
import tempfile
import stat
from pathlib import Path
from unittest.mock import patch
from scripts import p0c4_storage_registry as registry


class FaultRegistryFixture:
    """Real files/bytes; portable adapter models Linux directory/owner metadata.

    No validator is mocked. Linux no-follow and root ownership still require
    the actual root fixtures and fresh public-runner execution.
    """
    def __enter__(self):
        from contextlib import ExitStack
        from types import SimpleNamespace
        self.stack=ExitStack();self.temp=self.stack.enter_context(tempfile.TemporaryDirectory())
        self.root=Path(self.temp);self.dirs={};self.fds={};self.meta={};self.nextfd=100000
        self.real_os=registry.os
        self.receipt=receipt_fixture();self.mount='/var/lib/knowweave-c4/registry'
        def mapped(path):return self.root/str(path).lstrip('/')
        self.mapped=mapped
        def directory(path):
            key=str(path);local=mapped(key);local.mkdir(parents=True,exist_ok=True)
            if key not in self.dirs:
                self.nextfd+=1;self.dirs[key]=self.nextfd;self.fds[self.nextfd]=key
            return self.dirs[key]
        self.directory=directory
        def identity(fd):return dict(dev=1,ino=fd)
        rootfd=directory(self.mount)
        for name in registry.DIRECTORIES:directory(self.mount+'/'+name)
        directory(self.mount+'/protection/'+self.receipt['source_group_id'])
        kinds={'assets':'source_assets','control':'source_control','local_pins':'local_pins','staging':'source_staging'}
        rootmap={};rootrefs=[]
        for row in self.receipt['roots']:
            fd=directory(row['path']);row.update(identity(fd));key=next(k for k,v in kinds.items() if v==row['kind']);rootmap[key]=row['id']
            value=dict(format_version=1,capability='backup_root_v1',deployment_id=self.receipt['deployment_id'],enrollment_id=row['id'],group_id=self.receipt['source_group_id'],kind=row['kind'],path=row['path'],dev=row['dev'],ino=row['ino'],uid=0,mode=448,enrolled_generation=1)
            raw=self.write('roots/'+row['id']+'.json',value);row['record_sha256']=registry.digest(raw);rootrefs.append(dict(id=row['id'],sha256=registry.digest(raw)))
        control=next(row for row in self.receipt['roots'] if row['kind']=='source_control')
        binding=registry.canonical(dict(format_version=1,capability='source_control_binding_v1',binding_id=str(uuid.uuid4()),control_path=control['path'],control_dev=control['dev'],control_ino=control['ino'],database='learning_backup_c4_task3_'+self.receipt['case_id'],database_oid=42,system_identifier='123'))
        mapped(control['path']+'/source-binding.json').write_bytes(binding);self.receipt['source_binding_sha256']=registry.digest(binding)
        self.group=dict(format_version=1,capability='backup_source_group_v1',deployment_id=self.receipt['deployment_id'],group_id=self.receipt['source_group_id'],database='learning_backup_c4_task3_'+self.receipt['case_id'],database_oid=42,system_identifier='123',source_binding_sha256=self.receipt['source_binding_sha256'],application_commit=self.receipt['application_commit'],application_build_sha256=self.receipt['application_build_sha256'],roots=rootmap)
        groupraw=self.write('groups/'+self.receipt['source_group_id']+'.json',self.group)
        generation=dict(format_version=1,capability='backup_registry_generation_v1',deployment_id=self.receipt['deployment_id'],generation=1,previous_generation_sha256=None,roots=sorted(rootrefs,key=lambda x:x['id']),groups=[dict(id=self.receipt['source_group_id'],sha256=registry.digest(groupraw))])
        genraw=self.write('generations/00000000000000000001.json',generation)
        self.write('registry.lock',b'');lock=mapped(self.mount+'/registry.lock').stat()
        authority=dict(format_version=1,capability='backup_registry_v1',deployment_id=self.receipt['deployment_id'],registry_path=self.mount,registry_dev=1,registry_ino=rootfd,lock_dev=lock.st_dev,lock_ino=lock.st_ino,directories={n:identity(self.dirs[self.mount+'/'+n]) for n in registry.DIRECTORIES},initial_generation=1,initial_generation_sha256=registry.digest(genraw))
        araw=self.write('authority.json',authority);self.receipt.update(authority_sha256=registry.digest(araw),generation_sha256=registry.digest(genraw),registry_dev=1,registry_ino=rootfd)
        actual=self.real_os;fdpaths={}
        class PortableOS:
            def __getattr__(self,key):return getattr(actual,key,0) if key.startswith('O_') else getattr(actual,key)
            def open(_,name,flags,mode=0o777,*,dir_fd=None):
                path=mapped(self.fds[dir_fd]+'/'+name) if dir_fd is not None else Path(name)
                if path.is_dir():return self.dirs['/'+path.relative_to(self.root).as_posix()]
                fd=actual.open(path,actual.O_RDONLY|getattr(actual,'O_BINARY',0));fdpaths[fd]=path;return fd
            def close(_,fd):
                if fd not in self.fds:fdpaths.pop(fd,None);actual.close(fd)
            def fstat(_,fd):
                if fd in self.fds:return SimpleNamespace(st_dev=1,st_ino=fd,st_uid=0,st_mode=stat.S_IFDIR|0o500)
                m=actual.fstat(fd);values={k:getattr(m,k) for k in ('st_dev','st_ino','st_size','st_mtime_ns','st_ctime_ns')}
                values.update(st_uid=0,st_mode=stat.S_IFREG|0o600,st_nlink=1)
                values.update(self.meta.get(str(fdpaths.get(fd)),{}));return SimpleNamespace(**values)
        for patcher in (patch.object(registry,'os',PortableOS()),patch.object(registry,'open_directory',side_effect=lambda path:self.dirs[str(path)]),patch.object(registry,'child_directory',side_effect=lambda fd,name:self.dirs[self.fds[fd]+'/'+name]),patch.object(registry,'directory',side_effect=lambda fd,*args:identity(fd)),patch.object(registry,'names',side_effect=lambda fd,cap:self.names(fd,cap))):self.stack.enter_context(patcher)
        return self
    def __exit__(self,*args):return self.stack.__exit__(*args)
    def names(self,fd,cap):
        values=sorted(p.name for p in self.mapped(self.fds[fd]).iterdir());registry.require(len(values)<=cap,'REGISTRY_ENTRY_CAP');return values
    def write(self,path,value):
        raw=value if type(value) is bytes else registry.canonical(value)
        local=self.mapped(self.mount+'/'+path);local.parent.mkdir(parents=True,exist_ok=True);local.write_bytes(raw);return raw
    def pending(self,backup):
        return dict(format_version=1,capability='source_protection_v1',deployment_id=self.receipt['deployment_id'],group_id=self.receipt['source_group_id'],backup_id=backup,registry_generation=1,registry_generation_sha256=self.receipt['generation_sha256'],source_binding_sha256=self.receipt['source_binding_sha256'],application_commit=self.receipt['application_commit'],application_build_sha256=self.receipt['application_build_sha256'],state='capture_pending',previous_sha256=None,roots=self.group['roots'])
    def residue(self,test):
        tx='eeeeeeee-1111-4111-8111-111111111111';backup='ffffffff-1111-4111-8111-111111111111'
        self.directory(self.mount+'/staging/'+tx)
        pending=self.pending(backup)
        if test.endswith('pending_staged_fault'):self.write('staging/'+tx+'/00-pending.json',pending)
        else:
            folder='protection/'+self.receipt['source_group_id']+'/'+backup;self.directory(self.mount+'/'+folder)
            raw=self.write(folder+'/00-pending.json',pending);index=b'{"format_version":1,"assets":[]}'
            self.write(folder+'/asset-index.json',index)
            catalog=dict(pending,state='catalog_durable',previous_sha256=registry.digest(raw),asset_index=dict(path='asset-index.json',size=len(index),sha256=registry.digest(index)),logical_asset_count=0);catalog.pop('roots')
            self.write('staging/'+tx+'/10-catalog.json',catalog)
            self.journal(backup,False)
        return tx,backup
    def journal(self,backup,released):
        control=next(r['path'] for r in self.receipt['roots'] if r['kind']=='source_control');path=control+'/'+backup+'.control';self.directory(path)
        phases=('intent','closed','drained','dump_and_index_durable','pins_durable','release_ready','released')
        for n,phase in enumerate(phases if released else phases[:3]):
            raw=json.dumps(dict(backup_id=backup,phase=phase,dump_and_index_sha256='a'*64 if n>=3 else None,pins_sha256='b'*64 if n>=4 else None),separators=(',',':')).encode()
            self.mapped(path+'/'+phase.replace('_','-')+'.json').write_bytes(raw)
    def retained(self):
        backup='99999999-1111-4111-8111-111111111111';prefix='protection/'+self.receipt['source_group_id']+'/'+backup;self.directory(self.mount+'/'+prefix)
        p=self.pending(backup);raw=self.write(prefix+'/00-pending.json',p);index=b'{"format_version":1,"assets":[]}'
        self.write(prefix+'/asset-index.json',index)
        c=dict(p,state='catalog_durable',previous_sha256=registry.digest(raw),asset_index=dict(path='asset-index.json',size=len(index),sha256=registry.digest(index)),logical_asset_count=0);c.pop('roots');craw=self.write(prefix+'/10-catalog.json',c)
        pins=next(r['path'] for r in self.receipt['roots'] if r['kind']=='local_pins');sealed=pins+'/'+backup+'.sealed';fd=self.directory(sealed)
        manifest=self.mapped(sealed+'/manifest.json');manifest.write_bytes(b'{}');self.meta[str(manifest)]=dict(st_mode=stat.S_IFREG|0o400)
        retained=dict(p,state='retained',previous_sha256=registry.digest(craw),asset_index_sha256=registry.digest(index),manifest_sha256=registry.digest(b'{}'),sealed_name=backup+'.sealed',sealed_dev=1,sealed_ino=fd);retained.pop('roots');self.write(prefix+'/20-retained.json',retained)
        self.journal(backup,True)

    def snapshot(self):return {str(p.relative_to(self.root)):(p.stat().st_ino,p.read_bytes()) for p in self.root.rglob('*') if p.is_file()}


class RegistryFaultDiagnosticTests(unittest.TestCase):
    def test_pending_and_catalog_diagnostic_preserve_strict_failure_and_bytes(self):
        for name in ('pending_staged','catalog'):
            test='protection_tests::real_registry_'+name+'_fault'
            with self.subTest(test=test),FaultRegistryFixture() as fixture:
                baseline=registry.capture_fault_audit_baseline(fixture.receipt,test)
                fixture.residue(test)
                with self.assertRaisesRegex(registry.RegistryError,'REGISTRY_INCOMPLETE_PUBLICATION'):registry.validate_installed_registry()
                # The fixture deliberately lacks the required retained first
                # attempt: a partial fragment alone must never obtain credit.
                with self.assertRaisesRegex(registry.RegistryError,'REGISTRY_FAULT_RETAINED'):
                    registry.audit_incomplete_publication(baseline)

    def test_expected_residue_is_readonly_and_original_authority_stays_unusable(self):
        for suffix in ('pending_staged','catalog'):
            test='protection_tests::real_registry_'+suffix+'_fault'
            with self.subTest(test=test),FaultRegistryFixture() as f:
                baseline=registry.capture_fault_audit_baseline(f.receipt,test);f.retained();f.residue(test);before=f.snapshot()
                result=registry.audit_incomplete_publication(baseline)
                self.assertEqual(result['classification'],'DIAGNOSTIC_NOT_AUTHORITY')
                self.assertEqual(result['reason'],'REGISTRY_INCOMPLETE_PUBLICATION')
                self.assertEqual(result,registry.audit_incomplete_publication(baseline));self.assertEqual(before,f.snapshot())
                with self.assertRaisesRegex(registry.RegistryError,'REGISTRY_INCOMPLETE_PUBLICATION'):registry.validate_installed_registry()

    def test_changed_immutable_partial_unknown_or_metadata_never_obtain_diagnostic(self):
        for change in ('authority-bytes','authority-inode','authority-missing','lock-inode','generation','group','root','extra-staging','extra-transaction','partial','extra-field','bad-mode','bad-link','bad-uid','symlink','missing'):
            with self.subTest(change=change),FaultRegistryFixture() as f:
                test='protection_tests::real_registry_pending_staged_fault';baseline=registry.capture_fault_audit_baseline(f.receipt,test);f.retained();tx,_=f.residue(test)
                path=f.mapped(f.mount+'/authority.json')
                if change=='authority-bytes':path.write_bytes(path.read_bytes()+b' ')
                elif change=='authority-inode':f.meta[str(path)]=dict(st_ino=path.stat().st_ino+1)
                elif change=='authority-missing':path.unlink()
                elif change=='lock-inode':
                    path=f.mapped(f.mount+'/registry.lock');f.meta[str(path)]=dict(st_ino=path.stat().st_ino+1)
                elif change=='generation':f.write('generations/00000000000000000001.json',b'{}')
                elif change=='group':f.write('groups/'+f.receipt['source_group_id']+'.json',b'{}')
                elif change=='root':f.write('roots/'+f.receipt['roots'][0]['id']+'.json',b'{}')
                elif change=='extra-staging':f.write('staging/'+tx+'/unexpected.json',b'{}')
                elif change=='extra-transaction':f.directory(f.mount+'/staging/'+str(uuid.uuid4()))
                else:
                    path=f.mapped(f.mount+'/staging/'+tx+'/00-pending.json')
                    if change=='partial':path.write_bytes(b'{}')
                    elif change=='extra-field':value=json.loads(path.read_bytes());value['unknown']=1;path.write_bytes(registry.canonical(value))
                    elif change=='bad-mode':f.meta[str(path)]=dict(st_mode=stat.S_IFREG|0o644)
                    elif change=='bad-link':f.meta[str(path)]=dict(st_nlink=2)
                    elif change=='bad-uid':f.meta[str(path)]=dict(st_uid=1000)
                    elif change=='symlink':f.meta[str(path)]=dict(st_mode=stat.S_IFLNK|0o600)
                    else:path.unlink()
                before=f.snapshot()
                with self.assertRaises((registry.RegistryError,OSError,KeyError)):registry.audit_incomplete_publication(baseline)
                self.assertEqual(before,f.snapshot())

    def test_unarmed_wrong_case_or_usable_registry_cannot_be_diagnostic(self):
        with FaultRegistryFixture() as f:
            with self.assertRaisesRegex(registry.RegistryError,'REGISTRY_FAULT_TEST'):registry.capture_fault_audit_baseline(f.receipt,'protection_tests::real_registry_pending_visible_fault')
            baseline=registry.capture_fault_audit_baseline(f.receipt,'protection_tests::real_registry_pending_staged_fault')
            with self.assertRaisesRegex(registry.RegistryError,'REGISTRY_FAULT_EXPECTED_REFUSAL'):registry.audit_incomplete_publication(baseline)

    def test_catalog_wrong_index_or_count_and_shared_read_overflow_refuse(self):
        for change in ('index','count','overflow'):
            with self.subTest(change=change),FaultRegistryFixture() as f:
                test='protection_tests::real_registry_catalog_fault';baseline=registry.capture_fault_audit_baseline(f.receipt,test);f.retained();tx,backup=f.residue(test)
                if change=='index':f.write('protection/'+f.receipt['source_group_id']+'/'+backup+'/asset-index.json',b'{"assets":[],"format_version":1}')
                elif change=='count':
                    path=f.mapped(f.mount+'/staging/'+tx+'/10-catalog.json');row=json.loads(path.read_bytes());row['logical_asset_count']=1;path.write_bytes(registry.canonical(row))
                with patch.object(registry,'MAX_SCAN',100 if change=='overflow' else 536870912),self.assertRaises(registry.RegistryError):registry.audit_incomplete_publication(baseline)

    def test_catalog_unique_digest_cap_is_shared_across_retained_and_partial_indexes(self):
        def index(sha):
            row=dict(space_id='11111111-1111-4111-8111-111111111111',id='22222222-2222-4222-8222-222222222222',sha256=sha,byte_size=0,storage_key='sha256/'+sha[:2]+'/'+sha)
            return json.dumps(dict(format_version=1,assets=[row]),separators=(',',':')).encode()
        shared={format(n,'064x'):0 for n in range(99999)}
        self.assertEqual(registry._fault_index(index('f'*64),shared),1);self.assertEqual(len(shared),100000)
        self.assertEqual(registry._fault_index(index('f'*64),shared),1);self.assertEqual(len(shared),100000)
        with self.assertRaisesRegex(registry.RegistryError,'REGISTRY_FAULT_INDEX_UNIQUE_CAP'):registry._fault_index(index('e'*64),shared)

def receipt_fixture(binding_sha='a'*64,control_dev=10,control_ino=20):
    roots=[]
    for n,(kind,name) in enumerate((('source_assets','assets'),('source_control','control'),('local_pins','pins'),('source_staging','asset-staging')),1):
        roots.append(dict(id=f'{n:08x}-1111-4111-8111-111111111111',kind=kind,path='/var/lib/knowweave-source/'+name,dev=control_dev,ino=control_ino if kind=='source_control' else control_ino+n,record_sha256='b'*64))
    return dict(format_version=1,capability='independent_registry_provisioning_v1',batch_id='aaaaaaaa-1111-4111-8111-111111111111',case_id='bbbbbbbb-1111-4111-8111-111111111111',source_package_sha256='c'*64,application_commit='d'*40,application_build_sha256='c'*64,source_binding_sha256=binding_sha,deployment_id='cccccccc-1111-4111-8111-111111111111',source_group_id='dddddddd-1111-4111-8111-111111111111',authority_sha256='e'*64,generation=1,generation_sha256='f'*64,registry_dev=1,registry_ino=2,roots=roots,issuer_euid=0,state='READBACK_DURABLE')

class RegistryTests(unittest.TestCase):
    def test_canonical_cross_language_golden(self):
        value = dict(capability='backup_registry_generation_v1',deployment_id='11111111-1111-4111-8111-111111111111',format_version=1,generation=1,groups=[],previous_generation_sha256=None,roots=[])
        self.assertEqual(registry.canonical(value), b'{"capability":"backup_registry_generation_v1","deployment_id":"11111111-1111-4111-8111-111111111111","format_version":1,"generation":1,"groups":[],"previous_generation_sha256":null,"roots":[]}')

    def test_no_deserialize_constructor_or_caller_pin(self):
        with self.assertRaises(TypeError): registry.FreshProvisioningContext(**{})
        self.assertNotIn('expected_pin', registry.validate_installed_registry.__annotations__)

    def test_strict_parser_rejects_duplicates_float_unknown_and_lf(self):
        for raw in (b'{"a":1,"a":1}', b'{"a":1.0}', b'{"a":1}\n'):
            with self.assertRaises(registry.RegistryError): registry.parse_record(raw)

@unittest.skipUnless(sys.platform=='linux' and getattr(os,'geteuid',lambda:1)()==0,'requires actual Linux root and fresh TEST_C4_REGISTRY_ISSUER_FS_ROOT')
class RegistryPublicationTests(unittest.TestCase):
    def exercise(self,point):
        parent=Path(os.environ['TEST_C4_REGISTRY_ISSUER_FS_ROOT'])
        held=registry.open_directory(parent);os.close(held)
        root=parent/('issuer-'+uuid.uuid4().hex);root.mkdir(mode=0o700)
        source=root/'source';source.mkdir(mode=0o700);mount=root/'registry';mount.mkdir(mode=0o700)
        roots={};fds=[]
        try:
            for key in ('control','assets','staging','local_pins'):
                path=source/key;path.mkdir(mode=0o700);fd=registry.open_directory(path);fds.append(fd);roots[key]=(fd,path.as_posix())
            registry_fd=registry.open_directory(mount);fds.append(registry_fd)
            identity=registry.directory(roots['control'][0]);facts=dict(database='learning_backup_c4_task3_'+str(uuid.uuid4()),database_oid=123,system_identifier='456')
            binding=registry.canonical(dict(format_version=1,capability='source_control_binding_v1',binding_id=str(uuid.uuid4()),control_path=roots['control'][1],control_dev=identity['dev'],control_ino=identity['ino'],**facts))
            registry.new_file(roots['control'][0],'source-binding.json',binding)
            accepted=dict(batch_id=str(uuid.uuid4()),case_id=str(uuid.uuid4()),source_package_sha256='a'*64,application_commit='b'*40,application_build_sha256='a'*64)
            real_open=registry.open_directory
            # Private namespace adapter retains normative fixed authority bytes
            # and validates real held root metadata; no production path override.
            def open_test(path):return os.dup(registry_fd) if str(path)==registry.REGISTRY_PATH else real_open(path)
            with patch.object(registry,'open_directory',side_effect=open_test):
                context=registry._root_helper_context(accepted_case=accepted,source_binding_bytes=binding,source_database_facts=facts,source_roots=roots,registry_root_handle=registry_fd)
                reached=[]
                def fault(name):
                    reached.append(name)
                    if name==point:raise OSError('controlled registry publication fault')
                if point:
                    with patch.object(registry,'_crash_seam',side_effect=fault),self.assertRaisesRegex(OSError,'controlled registry publication fault'):registry.provision_registry(context)
                    self.assertEqual(reached.count(point),1)
                    self.assertEqual((mount/'authority.json').exists(),point=='after_initial_authority')
                    generation=mount/'generations/00000000000000000001.json'
                    self.assertEqual(generation.exists(),point!='before_generation_commit')
                    staging=list((mount/'staging').iterdir())
                    self.assertEqual(len(staging),int(point in ('before_initial_authority','after_initial_authority')))
                    if point=='before_initial_authority':self.assertTrue((staging[0]/'authority.json').is_file())
                    if point=='after_initial_authority':self.assertEqual(list(staging[0].iterdir()),[])
                    with self.assertRaises((registry.RegistryError,OSError)):registry.validate_installed_registry()
                else:
                    receipt=registry.provision_registry(context);registry.validate_provisioning_receipt(receipt.record)
                    self.assertEqual(registry.validate_installed_registry().authority['registry_ino'],os.fstat(registry_fd).st_ino)
                def snapshot():
                    entries={};pending=[mount]
                    while pending:
                        directory=pending.pop()
                        for path in sorted(directory.iterdir()):
                            self.assertLess(len(entries),64);meta=path.lstat()
                            entries[path.relative_to(mount).as_posix()]=(meta.st_dev,meta.st_ino,None if path.is_dir() else path.read_bytes())
                            if path.is_dir():pending.append(path)
                    return entries
                before=snapshot()
                with self.assertRaises(registry.RegistryError):registry.provision_registry(context)
                self.assertEqual(snapshot(),before,'reinitialization cannot modify partial/original bytes')
                self.assertEqual((source/'control/source-binding.json').read_bytes(),binding)
        finally:
            for fd in fds:os.close(fd)
        # Deliberately retain this exact test's new populated root as evidence.

    def test_before_generation_commit_preserves_uncommitted_roster(self):self.exercise('before_generation_commit')
    def test_after_generation_commit_without_authority_refuses_reinit(self):self.exercise('after_generation_commit')
    def test_before_initial_authority_keeps_partial_transaction(self):self.exercise('before_initial_authority')
    def test_after_initial_authority_keeps_visible_record_and_staging(self):self.exercise('after_initial_authority')
    def test_durable_registry_repeated_initialization_is_mutation_free(self):self.exercise(None)
