"""Actual Linux filesystem/lock tests for the closed two-role owner seam.

No Docker/PG body, source capture or restore authority is produced here.
"""
import importlib.util
import importlib
import hashlib
import io
import shutil
import stat
import os
from pathlib import Path
import json
import subprocess
import time
import sys
import tempfile
import unittest
import uuid
import zipfile
from types import SimpleNamespace

from p0c4_completion import full_target_fs as fs
from p0c4_completion import controlled_fixture as fixture


class ContextPlanTests(unittest.TestCase):
    def plan(self):
        values = [str(uuid.uuid4()) for _ in range(4)]
        return dict(format_version=1, capability='c4_current11_first_success_plan_v1', case='success',
            batch_id=values[0], source_case_id=values[1], target_case_id=values[2], cache_scope=values[3],
            source_subnet='172.30.241.0/24', target_subnet='172.30.242.0/24', source_commit='a'*40,
            public_archive_sha256='b'*64, public_manifest_sha256='c'*64,
            legacy_archive_sha256='d'*64, legacy_manifest_sha256='e'*64,
            builder='sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b',
            postgres='postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d',
            budget_profile='controlled_fixture_two_role_first_success_v1')

    def validate(self, value):
        method = getattr(fixture, '_validate_plan', None)
        self.assertTrue(callable(method), 'missing fixed-success plan validator')
        return method(value)

    def test_outer_projection_rejects_extra_mount_admin_cap_and_writable_tool(self):
        suite='/var/lib/docker/volumes/kwc4c-suite-'+uuid.uuid4().hex+'/_data'
        facts=dict(Id='a'*64,Image=fixture._BUILDER,Config=dict(User='0:0',Image=fixture._BUILDER),State=dict(Running=True),
            HostConfig=dict(NetworkMode='host',ReadonlyRootfs=True,Privileged=False,PidMode='',CapAdd=None,CapDrop=['ALL'],SecurityOpt=['no-new-privileges']),
            Mounts=[dict(Type='volume',Name=Path(suite).parent.name,Source=suite,Destination=suite,RW=True),
                *[dict(Type='bind',Source=p,Destination=p,RW=False) for p in ('/usr/bin/docker','/usr/libexec/docker/cli-plugins/docker-compose','/var/run/docker.sock')],
                dict(Type='bind',Source='/proc/1/ns/net',Destination='/run/knowweave-c4/host-netns',RW=False)])
        method=getattr(fixture,'_outer_projection',None)
        self.assertTrue(callable(method),'missing actual fixed outer capsule projection')
        self.assertEqual(method(facts,Path(suite))['container_id'],'a'*64)
        for mutation in ('extra','cap','tool','ancestor','rootfs','container-local-netns','alias-netns'):
            row=json.loads(json.dumps(facts))
            if mutation=='extra':row['Mounts'].append(dict(Type='bind',Source='/var',Destination='/var',RW=False))
            elif mutation=='cap':row['HostConfig']['CapAdd']=['SYS_ADMIN']
            elif mutation=='tool':row['Mounts'][1]['RW']=True
            elif mutation=='ancestor':row['Mounts'][0]['Source']='/var/lib/docker/volumes'
            elif mutation=='rootfs':row['HostConfig']['ReadonlyRootfs']=False
            elif mutation=='container-local-netns':row['Mounts'][-1]['Source']='/run/knowweave-c4/host-netns'
            else:row['Mounts'][-1]['Source']='/proc/self/ns/net'
            with self.subTest(mutation=mutation),self.assertRaises(fixture.ControlledRejected):method(row,Path(suite))

    def test_fixed_subnets_avoid_actual_parser_rows_and_refuse_unknown_observations(self):
        import struct
        from p0c4_completion import resources
        payload=struct.pack('=BBBBBBBBI',2,24,0,0,254,2,0,1,0)+struct.pack('=HH',8,1)+bytes((172,29,0,0))
        route=struct.pack('=IHHII',16+len(payload),24,2,7,9)+payload
        done=struct.pack('=IHHIIi',20,3,2,7,9,0)
        parser=resources.RouteParser(7,9);parser.feed(route+done)
        self.assertEqual(fixture._choose_subnets([r['dst'] for r in parser.finish()]),['172.29.1.0/24','172.29.2.0/24'])
        for occupied in (['172.29.0.0/16'],['not-a-route'],[True]):
            with self.subTest(occupied=occupied),self.assertRaises(fixture.ControlledRejected):fixture._choose_subnets(occupied)

    def test_closed_plan_is_only_metadata_and_rejects_other_cases_or_selectors(self):
        plan = self.plan()
        self.assertEqual(self.validate(plan), plan)
        for key, value in (('case','ready-eof'), ('case','wrong-endpoint'), ('case',True),
            ('format_version',True), ('source_case_id',plan['target_case_id']), ('cache_scope',plan['batch_id']),
            ('source_subnet',plan['target_subnet']), ('target_subnet','172.30.241.4/24'),
            ('source_subnet','192.0.2.0/24'), ('builder','latest'), ('budget_profile','unbounded')):
            changed = dict(plan, **{key:value})
            with self.subTest(key=key, value=value), self.assertRaises(fixture.ControlledRejected):self.validate(changed)
        for key, value in (('root','/caller'), ('key','caller'), ('command',['true']), ('sql','SELECT 1;'),
                           ('adopted',True), ('expected_inode',1)):
            with self.subTest(key=key), self.assertRaises(fixture.ControlledRejected):self.validate(dict(plan, **{key:value}))

    def test_context_cannot_be_constructed_from_plan_or_completed_residue(self):
        self.validate(self.plan())
        cls = getattr(fixture, 'ControlledFixtureContext', None)
        self.assertTrue(isinstance(cls, type), 'missing opaque exact controlled context')
        for residue in (self.plan(), {}, {'state':'COMPLETED'}, None):
            with self.subTest(residue=residue), self.assertRaises((fixture.ControlledRejected, TypeError)):
                cls(residue)


@unittest.skipUnless(sys.platform == 'linux' and os.geteuid() == 0, 'actual root Linux private plan test')
class NativePlanTests(ContextPlanTests):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='kwc4-plan-unit-',dir='/root')
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'plan.json'
        self.raw = json.dumps(self.plan(), sort_keys=True, separators=(',', ':')).encode()
        self.path.write_bytes(self.raw);self.path.chmod(0o600)

    def read(self):
        method = getattr(fixture, '_read_plan', None)
        self.assertTrue(callable(method), 'missing actual nofollow private plan reader')
        return method(self.path)

    def test_native_reader_binds_actual_single_link_private_canonical_bytes(self):
        self.validate(self.plan())
        value, pin = self.read()
        observed = os.stat(self.path, follow_symlinks=False)
        self.assertEqual(pin['dev'],observed.st_dev);self.assertEqual(pin['ino'],observed.st_ino)
        self.assertEqual(value,json.loads(self.raw))
        self.assertEqual(pin['sha256'],__import__('hashlib').sha256(self.raw).hexdigest())
        self.assertNotIn('permit',pin)

    def test_reader_rejects_links_mode_duplicates_and_noncanonical_bytes(self):
        self.validate(self.plan())
        for mutation in ('hardlink','symlink','mode','duplicate','newline','extra'):
            with self.subTest(mutation=mutation):
                self.path.write_bytes(self.raw);self.path.chmod(0o600)
                extra = self.path.with_name('extra')
                if mutation=='hardlink':os.link(self.path,extra)
                elif mutation=='symlink':self.path.rename(extra);self.path.symlink_to(extra)
                elif mutation=='mode':self.path.chmod(0o644)
                elif mutation=='duplicate':self.path.write_bytes(self.raw.replace(b'{',b'{"format_version":1,',1))
                elif mutation=='newline':self.path.write_bytes(self.raw+b'\n')
                else:self.path.write_bytes(self.raw[:-1]+b',"sql":"SELECT 1;"}')
                with self.assertRaises((fixture.ControlledRejected,OSError)):self.read()
                self.path.unlink()
                if extra.exists():extra.unlink()
                self.path.write_bytes(self.raw);self.path.chmod(0o600)


@unittest.skipUnless(sys.platform == 'linux' and os.geteuid() == 0, 'actual sealed installation/held fd tests')
class NativeContextTests(unittest.TestCase):
    def setUp(self):
        self.repo=Path(__file__).resolve().parents[1]
        parents=Path('/var/lib/docker/volumes')
        parents.mkdir(parents=True,exist_ok=True)
        self.home=parents/('kwc4c-suite-'+uuid.uuid4().hex);self.home.mkdir(mode=0o700)
        self.addCleanup(self.remove_owned_fixture)
        self.suite=self.home/'_data';self.suite.mkdir(mode=0o700)
        for name in ('incoming','current11','installed'):(self.suite/name).mkdir(mode=0o700)
        self.previous_path=list(sys.path)
        self.previous_modules={n:m for n,m in sys.modules.items() if n=='p0c4_completion' or n.startswith('p0c4_') or n.startswith('p0c4_completion.')}
        self.addCleanup(self.restore_modules)
        for name in self.previous_modules:sys.modules.pop(name,None)
        files={}
        selected=('scripts/p0c4_controlled_import_acceptance.py','scripts/p0c4_restore_birth_acceptance.py',
            'scripts/p0c4_import_fixture.py','scripts/p0c4_restore_target.py','scripts/p0c4_restore_target_birth.py',
            'scripts/p0c4_restore_target_pin.py','scripts/p0c4_restore_target_pin_prepare.py','scripts/p0c4_restore_pin_acceptance.py',
            'scripts/p0c4_source_admission_gate_acceptance.py','scripts/test_p0c4_source_admission_gate_acceptance.py',
            'scripts/p0c4_source_binding_gate_acceptance.py','scripts/p0c4_source_lifecycle_gate_acceptance.py',
            'scripts/p0c4_source_isolation.py','scripts/p0c4_storage_registry.py','scripts/p0c4_maintenance_gate_acceptance.py',
            'scripts/p0c4_completion/controlled_fixture.py','scripts/p0c4_completion/full_target_fs.py','scripts/p0c4_completion/resources.py',
            'scripts/p0c4_completion/plan.py','scripts/p0c4_completion/full_import.py','scripts/p0c4_completion/roles.py',
            'deploy/p0c4_restore_initdb.sh','Cargo.toml','Cargo.lock',
            'crates/learning-backup/src/source.rs','crates/learning-backup/src/source/admission_tests.rs')
        for name in selected:files[name]=(self.repo/name).read_bytes()
        # This is a synthetic exact-byte local installation contract. Its
        # declared commit is test metadata, never a reviewed Git/body claim.
        self.plan=ContextPlanTests().plan()
        def canon(value):return json.dumps(value,sort_keys=True,separators=(',',':')).encode()
        public=dict(schema=1,base_commit=self.plan['source_commit'],snapshot_kind='working-tree-green',
            files=[dict(path=p,bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest()) for p,raw in sorted(files.items())])
        legacy=dict(format_version=1,commit=self.plan['source_commit'],files=[dict(path=p,sha256=hashlib.sha256(raw).hexdigest(),size=len(raw)) for p,raw in sorted(files.items())])
        def archive(dialect,manifest):
            out=io.BytesIO()
            with zipfile.ZipFile(out,'w') as z:
                for name,raw in [*sorted(files.items()),(dialect,canon(manifest))]:
                    info=zipfile.ZipInfo(name);info.create_system=3;info.external_attr=0o100444<<16;z.writestr(info,raw)
            return out.getvalue()
        self.public_raw=archive('_knowweave_source_manifest.json',public);self.legacy_raw=archive('SOURCE_MANIFEST.json',legacy)
        for name,raw in (('source-public.zip',self.public_raw),('source-legacy.zip',self.legacy_raw)):
            path=self.suite/'incoming'/name;path.write_bytes(raw);path.chmod(0o400)
        self.plan.update(public_archive_sha256=hashlib.sha256(self.public_raw).hexdigest(),public_manifest_sha256=hashlib.sha256(canon(public)).hexdigest(),legacy_archive_sha256=hashlib.sha256(self.legacy_raw).hexdigest(),legacy_manifest_sha256=hashlib.sha256(canon(legacy)).hexdigest())
        plan=self.suite/'current11/plan.json';plan.write_bytes(canon(self.plan));plan.chmod(0o600)
        # Opaque installed adapter bytes for metadata-only mint tests. Never
        # executed or accepted as an operational packet; Ops tests install the
        # actual separately sealed adapter. No SDD dependency in source tests.
        adapter=b'# opaque local install unit data; never executed\n'
        installed_adapter=self.suite/'current11/adapter.py';installed_adapter.write_bytes(adapter);installed_adapter.chmod(0o500)
        install=dict(format_version=1,capability='c4_current11_install_v1',**{k:self.plan[k] for k in ('source_commit','public_archive_sha256','public_manifest_sha256','legacy_archive_sha256','legacy_manifest_sha256','builder','postgres','budget_profile')},adapter_sha256=hashlib.sha256(adapter).hexdigest(),docker_sha256='a'*64,compose_sha256='b'*64)
        installed_record=self.suite/'current11/install.json';installed_record.write_bytes(canon(install));installed_record.chmod(0o400)
        bootstrap=self.suite/'current11/bootstrap'
        for name,raw in files.items():
            path=bootstrap/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(raw);path.chmod(0o400)
        for path in bootstrap.rglob('*'):
            if path.is_dir():path.chmod(0o700)
        bootstrap.chmod(0o700)
        sys.path.insert(0,str(bootstrap/'scripts'))
        self.runner=importlib.import_module('p0c4_controlled_import_acceptance');self.runner.BASE=self.suite
        for name in ('p0c4_controlled_import_acceptance.py','p0c4_restore_birth_acceptance.py','p0c4_import_fixture.py'):
            path=self.suite/'installed'/name;path.write_bytes(files['scripts/'+name]);path.chmod(0o500 if name.startswith('p0c4_controlled') else 0o400)
        helper=self.runner._load(self.suite/'installed/p0c4_restore_birth_acceptance.py','controlled_fixture_archive_unit')
        helper.BASE=self.suite;helper.ENTRY=self.runner.ENTRY;helper.REQUIRED=self.runner.REQUIRED|{self.runner.CLONE_HELPER}
        helper.__file__=str(self.suite/'installed/p0c4_controlled_import_acceptance.py')
        self.runner.fixture=self.runner._load(self.suite/'installed/p0c4_import_fixture.py','controlled_fixture_clients_unit')
        self.args=SimpleNamespace(batch_id=self.plan['batch_id'],source_batch_id=self.plan['source_case_id'],target_batch_id=self.plan['target_case_id'],source_subnet=self.plan['source_subnet'],target_subnet=self.plan['target_subnet'],case='success',phase='import',commit=self.plan['source_commit'],archive=self.suite/'incoming/source-legacy.zip',archive_sha256=self.plan['legacy_archive_sha256'],manifest_sha256=self.plan['legacy_manifest_sha256'])
        self.backend=self.runner.ImportBackend(self.args,helper,legacy,self.legacy_raw)
        self.addCleanup(self.release_backend)
        # Import the context from the actual extracted immutable source, not
        # the bootstrap. Restore the complete caller graph on test exit.
        for name in tuple(sys.modules):
            if name=='p0c4_completion' or name.startswith('p0c4_completion.') or name=='p0c4_source_admission_gate_acceptance':sys.modules.pop(name,None)
        sys.path.insert(0,str(self.backend.source/'scripts'))
        self.context_module=importlib.import_module('p0c4_completion.controlled_fixture')
        self.context=None

    def remove_owned_fixture(self):
        resolved=self.home.resolve()
        self.assertEqual(resolved.parent,Path('/var/lib/docker/volumes'))
        self.assertEqual(resolved.name,self.home.name)
        self.assertRegex(resolved.name,r'^kwc4c-suite-[0-9a-f]{32}$')
        shutil.rmtree(resolved)
    def restore_modules(self):
        sys.path[:]=self.previous_path
        for name in tuple(sys.modules):
            if name=='p0c4_completion' or name.startswith('p0c4_') or name.startswith('p0c4_completion.'):sys.modules.pop(name,None)
        sys.modules.update(self.previous_modules)
    def release_backend(self):
        if self.context is not None:
            self.context._install.close();self.backend._controlled_context=None
            self.runner._UNSETTLED_BACKENDS.discard(self.backend)
        self.backend.release()
    def mint(self):
        self.context=self.context_module._mint_from_backend(self.backend)
        return self.context

    def test_actual_dual_package_mint_holds_original_fds_and_is_not_reconstructible(self):
        import pickle
        context=self.mint()
        self.assertEqual(os.fstat(context._install.suite_fd).st_ino,os.stat(self.suite).st_ino)
        self.assertEqual(os.fstat(context._install.source_fd).st_ino,os.stat(self.backend.source).st_ino)
        self.assertEqual(context._owned_pg_names(),tuple('learning-system-p0c4-restore-'+self.plan[k]+'-pg-1' for k in ('source_case_id','target_case_id')))
        with self.assertRaises(TypeError):pickle.dumps(context)
        with self.assertRaises(self.context_module.ControlledRejected):self.context_module._mint_from_backend(self.backend)
        marker=json.loads((self.backend.batch/'context.minted').read_bytes())
        self.assertEqual(marker['scope'],'CONTEXT_MINTED_NOT_AUTHORITY')

    def test_plan_and_archive_residue_without_actual_install_record_cannot_mint(self):
        (self.suite/'current11/install.json').unlink()
        with self.assertRaises((self.context_module.ControlledRejected,OSError)):self.mint()
        self.assertFalse((self.backend.batch/'context.minted').exists())

    def test_original_plan_or_source_inode_replacement_invalidates_held_context(self):
        context=self.mint();old=context._install.source_id
        self.backend.source.rename(self.backend.source.with_name('retained-source'))
        self.backend.source.mkdir(mode=0o700)
        with self.assertRaises(self.context_module.ControlledRejected):context._alive()
        self.assertEqual(os.fstat(context._install.source_fd).st_ino,old[1])

    def test_different_public_members_cannot_mint_even_when_archive_hash_is_updated(self):
        raw=self.suite/'incoming/source-public.zip'
        out=io.BytesIO()
        with zipfile.ZipFile(io.BytesIO(self.public_raw)) as old,zipfile.ZipFile(out,'w') as new:
            doc=json.loads(old.read('_knowweave_source_manifest.json'))
            changed=b'# changed local unit input\n'+old.read('Cargo.toml')
            for row in doc['files']:
                if row['path']=='Cargo.toml':row.update(bytes=len(changed),sha256=hashlib.sha256(changed).hexdigest())
            encoded=json.dumps(doc,sort_keys=True,separators=(',',':')).encode()
            for info in old.infolist():
                new.writestr(info,encoded if info.filename=='_knowweave_source_manifest.json' else changed if info.filename=='Cargo.toml' else old.read(info.filename))
        raw.write_bytes(out.getvalue())
        self.plan.update(public_archive_sha256=hashlib.sha256(out.getvalue()).hexdigest(),public_manifest_sha256=hashlib.sha256(encoded).hexdigest())
        (self.suite/'current11/plan.json').write_bytes(json.dumps(self.plan,sort_keys=True,separators=(',',':')).encode())
        install_path=self.suite/'current11/install.json';install=json.loads(install_path.read_bytes())
        install.update(public_archive_sha256=self.plan['public_archive_sha256'],public_manifest_sha256=self.plan['public_manifest_sha256'])
        install_path.write_bytes(self.context_module._canonical(install))
        with self.assertRaises((self.context_module.ControlledRejected,RuntimeError,ValueError)):
            self.context_module._mint_from_backend(self.backend)
        self.assertFalse((self.backend.batch/'context.minted').exists())

    def test_actual_ledger_admits_only_derived_two_services_before_send(self):
        context=self.mint()
        driver=importlib.import_module('p0c4_source_lifecycle_gate_acceptance')
        h=importlib.import_module('p0c4_source_admission_gate_acceptance')
        logs=self.backend.batch/'evidence/logs';logs.mkdir(mode=0o700)
        result=dict(batch_id=self.plan['batch_id'],helpers=[])
        runner=driver.owned_log_runner(h,self.backend.batch,result,0)
        runner.ownership.case_id=self.plan['source_case_id'];runner.ownership._controlled_pair=context
        first,second=context._owned_pg_names()
        one=runner.ownership.acquire('pg',first,{},None,{})
        runner.ownership.known(one,'a'*64)
        two=runner.ownership.acquire('pg',second,{},None,{})
        self.assertEqual(two['state'],'planned')
        with self.assertRaises(driver.GateError):runner.ownership.acquire('pg','caller-pg',{},None,{})
        runner.ownership.known(two,'b'*64)
        with self.assertRaisesRegex(driver.GateError,'CLEANUP_PG_COUNT'):runner.ownership.acquire('pg',first,{},None,{})

    def test_ledger_rejects_mixed_clone_full_unminted_or_foreign_names(self):
        context=self.mint()
        driver=importlib.import_module('p0c4_source_lifecycle_gate_acceptance')
        h=importlib.import_module('p0c4_source_admission_gate_acceptance')
        for mutation in ('clone','full','unminted','foreign'):
            unit=self.backend.batch/('ledger-'+mutation);(unit/'evidence/logs').mkdir(parents=True,mode=0o700)
            result=dict(batch_id=self.plan['batch_id'],helpers=[])
            runner=driver.owned_log_runner(h,unit,result,0)
            runner.ownership._controlled_pair={'state':'COMPLETED'} if mutation=='unminted' else context
            if mutation=='clone':runner.ownership.source_clone_pg_names=context._owned_pg_names()
            if mutation=='full':runner.ownership._full_pair=object()
            name='foreign-pg' if mutation=='foreign' else context._owned_pg_names()[0]
            with self.subTest(mutation=mutation),self.assertRaises(driver.GateError):runner.ownership.acquire('pg',name,{},None,{})
            self.assertEqual(runner.ownership.owners,{})

    def test_owned_runner_records_sent_before_real_child_and_retains_unknown_transport(self):
        from unittest.mock import patch
        context=self.mint()
        driver=importlib.import_module('p0c4_source_lifecycle_gate_acceptance')
        h=importlib.import_module('p0c4_source_admission_gate_acceptance')
        logs=self.backend.batch/'evidence/logs';logs.mkdir(mode=0o700)
        result=dict(batch_id=self.plan['batch_id'],helpers=[])
        runner=driver.owned_log_runner(h,self.backend.batch,result,0);runner.ownership._controlled_pair=context
        first=context._owned_pg_names()[0]
        owner=runner.ownership.acquire('pg',first,{},None,{})
        child_dir=self.backend.batch/'actual-unit-child';child_dir.mkdir(mode=0o700)
        script=child_dir/'compose';script.write_text("import time\nprint('actual unit transport',flush=True)\ntime.sleep(5)\n")
        original=h.subprocess.Popen;observed=[]
        def observe(*args,**kwargs):
            observed.append(owner['state'])
            return original(*args,**kwargs)
        cwd=Path.cwd()
        try:
            os.chdir(child_dir)
            with patch.object(h.subprocess,'Popen',side_effect=observe),self.assertRaises(h.GateError):
                driver.creation_call(runner,owner,lambda:runner.run([sys.executable,'compose','up'],timeout=0.15))
        finally:os.chdir(cwd)
        self.assertEqual(observed,['sent'])
        self.assertEqual(owner['state'],'sent')
        self.assertEqual(json.loads((logs/'0001.process.json').read_bytes())['reason'],'PROCESS_TIMEOUT')
        self.assertEqual((logs/'0001.stdout').read_bytes(),b'actual unit transport\n')
        context._runner=runner;context._result=result
        context.release()
        self.assertFalse(context._closed,'unknown real dispatch must retain original installation ownership')
        self.assertTrue(context._native_broken)
        self.assertGreater(os.fstat(context._install.suite_fd).st_ino,0)

    def test_peer_and_body_cannot_run_from_install_plan_without_live_births_or_compile(self):
        context=self.mint()
        for role in ('source','target','clone','caller'):
            with self.subTest(role=role),self.assertRaises(self.context_module.ControlledRejected):context._peer_inspect(role)
        with self.assertRaises(self.context_module.ControlledRejected):context._execute_body()
        self.assertFalse(context._body_used)

    def test_install_plan_alone_cannot_create_cache_before_daemon_tool_admission(self):
        context=self.mint()
        with self.assertRaises(self.context_module.ControlledRejected):context._prepare_cache()
        self.assertFalse((self.suite/'current11/cache').exists())

    def test_compile_cannot_start_from_plan_or_foreign_birth_pins(self):
        context=self.mint()
        for source,target,placeholder in (('0'*64,'0'*64,True),('a'*64,'b'*64,False),('a'*64,'a'*64,False)):
            with self.subTest(placeholder=placeholder),self.assertRaises(self.context_module.ControlledRejected):
                context._begin_compile(source,target,placeholder)
        self.assertFalse((self.suite/'current11/cache').exists())
        self.assertEqual(context._compiled,{})

    def test_backend_rejects_other_ten_before_budget_or_native_body(self):
        self.mint()
        for case in set(self.runner.IMPORT_CASES)-{'success'}:
            with self.subTest(case=case),self.assertRaises(self.context_module.ControlledRejected):
                self.backend.execute_import(case)
        self.assertEqual(self.backend.resources,[])

    def test_unknown_context_release_retains_actual_install_fds_and_global_lock(self):
        import fcntl
        context=self.mint();context._native_broken=True
        with self.assertRaises(self.runner.ImportRejected):self.backend.release()
        self.assertFalse(context._closed)
        self.assertGreater(os.fstat(context._install.suite_fd).st_ino,0)
        self.assertIn(self.backend,self.runner._UNSETTLED_BACKENDS)
        fd=os.open(self.suite/'controlled-import/.birth-acceptance.lock',os.O_RDWR|os.O_NOFOLLOW)
        try:
            with self.assertRaises(BlockingIOError):fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
        finally:os.close(fd)
        self.assertFalse((self.backend.batch/'evidence/result.json').exists())

    def test_expired_isolation_budget_cannot_restart_a_new_thirty_second_call(self):
        context=self.mint();self.backend.commands.isolation_deadline=time.monotonic()-1
        with self.assertRaises(self.context_module.ControlledRejected):context._timeout(30)
        self.assertIsNone(context._observation_deadline)


@unittest.skipUnless(sys.platform == 'linux' and os.geteuid() == 0, 'actual cache/materialization filesystem tests')
class MaterializationTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='kwc4-materialize-unit-',dir='/root')
        self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
        self.cache=self.root/'cache';self.build=self.root/'fresh-build'
        (self.cache/'debug/deps').mkdir(parents=True,mode=0o700);self.build.mkdir(mode=0o700)
        self.cache.chmod(0o700)
        self.binary=self.cache/'debug/deps/learning_backup-a123'
        # Regenerable compiler data, deliberately never executed in this test.
        self.binary.write_bytes(b'\x7fELF-local-data-not-executable');self.binary.chmod(0o644)
        self.rows=[dict(reason='compiler-artifact',manifest_path='/reviewed/crates/learning-backup/Cargo.toml',
            target=dict(name='learning_backup',kind=['lib']),profile=dict(test=True,opt_level='0',debuginfo=0,debug_assertions=True,overflow_checks=True),executable='/cache/debug/deps/learning_backup-a123'),
            dict(reason='build-finished',success=True)]
    def output(self):return b'\n'.join(json.dumps(r,separators=(',',':')).encode() for r in self.rows)+b'\n'
    def materialize(self):
        method=getattr(fixture,'_materialize_compiler_output',None)
        self.assertTrue(callable(method),'missing fixed fresh compiler data materializer')
        return method(self.output(),self.cache,self.build,'a'*64,'b'*64)
    def test_real_copy_has_distinct_inode_single_link_readback_and_cannot_replace(self):
        receipt=self.materialize();copy_path=self.build/'materialized/learning-backup-tests'
        self.assertEqual(copy_path.read_bytes(),self.binary.read_bytes())
        self.assertNotEqual(os.stat(copy_path).st_ino,os.stat(self.binary).st_ino)
        self.assertEqual(os.stat(copy_path).st_nlink,1);self.assertEqual(stat.S_IMODE(os.stat(copy_path).st_mode),0o500)
        self.assertEqual(receipt['binary_sha256'],hashlib.sha256(copy_path.read_bytes()).hexdigest())
        self.assertFalse(receipt['executed'])
        with self.assertRaises((fixture.ControlledRejected,RuntimeError)):self.materialize()
    def test_cached_placeholder_or_wrong_profile_and_duplicate_candidate_never_materialize(self):
        self.materialize_method_exists()
        for mutation in ('duplicate','other-path','not-test','failed','full-debug','same-pins'):
            rows=json.loads(json.dumps(self.rows));source_pin,target_pin='a'*64,'b'*64
            if mutation=='duplicate':rows.insert(0,rows[0])
            elif mutation=='other-path':rows[0]['executable']='/other/learning_backup-a123'
            elif mutation=='not-test':rows[0]['profile']['test']=False
            elif mutation=='failed':rows[1]['success']=False
            elif mutation=='full-debug':rows[0]['profile']['debuginfo']=2
            else:source_pin=target_pin='a'*64
            output=b'\n'.join(json.dumps(r).encode() for r in rows)+b'\n'
            with self.subTest(mutation=mutation),self.assertRaises(fixture.ControlledRejected):
                fixture._materialize_compiler_output(output,self.cache,self.build,source_pin,target_pin)
            self.assertFalse((self.build/'materialized/learning-backup-tests').exists())
    def materialize_method_exists(self):
        self.assertTrue(callable(getattr(fixture,'_materialize_compiler_output',None)),'missing fixed fresh compiler data materializer')
    def test_symlink_source_is_rejected_by_actual_shared_materializer(self):
        self.materialize_method_exists()
        retained=self.binary.with_name('retained');self.binary.rename(retained);self.binary.symlink_to(retained)
        with self.assertRaises((fixture.ControlledRejected,RuntimeError)):self.materialize()

    def test_real_cache_scan_rejects_sparse_over_cap_and_root_inode_substitution(self):
        method=getattr(fixture,'_measure_cache',None)
        self.assertTrue(callable(method),'missing fixed bounded cache ownership scanner')
        fd=os.open(self.cache,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
        self.addCleanup(os.close,fd)
        before=method(self.cache,fd)
        self.assertEqual(before['bytes'],len(self.binary.read_bytes()))
        with (self.cache/'oversized').open('wb') as file:file.truncate(16*1024**3+1)
        with self.assertRaises(fixture.ControlledRejected):method(self.cache,fd)
        (self.cache/'oversized').unlink()
        self.cache.rename(self.cache.with_name('retained-cache'));self.cache.mkdir(mode=0o700)
        with self.assertRaises(fixture.ControlledRejected):method(self.cache,fd)

    def test_actual_materialization_receipt_and_original_compiler_bytes_are_required(self):
        result=self.materialize();receipt=self.build/'materialized.json'
        receipt.write_bytes(fixture._canonical(result));receipt.chmod(0o400)
        output=self.output()+b'CURRENT11_MATERIALIZED\t'+fixture._canonical(result)+b'\n'
        method=getattr(fixture,'_read_materialized',None)
        self.assertTrue(callable(method),'missing actual fresh materialization receipt validator')
        binary,value=method(self.build,output,'a'*64,'b'*64)
        self.assertEqual(binary,self.build/'materialized/learning-backup-tests')
        self.assertEqual(value,result)
        for changed in (output+output.splitlines(keepends=True)[-1],output.replace(b'"success":true',b'"success":false'),output):
            if changed==output:binary.write_bytes(b'changed actual copy');binary.chmod(0o500)
            with self.assertRaises(fixture.ControlledRejected):method(self.build,changed,'a'*64,'b'*64)


class ConfigurationTests(unittest.TestCase):
    def fixture(self):
        batch, source, target = (str(uuid.uuid4()) for _ in range(3))
        suite = '/var/lib/docker/volumes/kwc4c-suite-' + uuid.uuid4().hex + '/_data'
        case = suite + '/controlled-import/batches/' + batch
        return dict(batch_id=batch, source_case_id=source, target_case_id=target,
            subnet='172.30.241.0/24', root=case + '/source-control', suite_root=suite,
            source_root=case + '/source', case_root=case, role='source')

    def check(self, value):
        method = getattr(fs, '_controlled_configuration', None)
        self.assertTrue(callable(method), 'missing fixed two-role native owner configuration')
        return method(value)

    def test_roots_derive_from_actual_suite_batch_role(self):
        value = self.fixture()
        self.assertEqual(self.check(value), value)
        value['role'] = 'target'
        value['root'] = value['case_root'] + '/target-control'
        self.assertEqual(self.check(value), value)

    def test_extra_root_or_full_role_path_cannot_select_recipe(self):
        for mutation in ('role', 'root', 'source_root', 'case_root', 'suite_root', 'extra', 'duplicate'):
            value = self.fixture()
            if mutation == 'role':value['role'] = 'clone'
            elif mutation == 'root':value['root'] = value['case_root'] + '/full-profile'
            elif mutation == 'source_root':value['source_root'] = '/other/source'
            elif mutation == 'case_root':value['case_root'] += '/arbitrary'
            elif mutation == 'suite_root':value['suite_root'] = '/var/lib/docker/volumes'
            elif mutation == 'extra':value['initdb'] = '/other/initdb'
            else:value['target_case_id'] = value['source_case_id']
            with self.subTest(mutation=mutation):
                method = getattr(fs, '_controlled_configuration', None)
                self.assertTrue(callable(method), 'missing fixed two-role native owner configuration')
                with self.assertRaises(fs.FsRejected):method(value)

    def test_default_owner_configuration_stays_full_only(self):
        value = self.fixture()
        self.check(value)
        with self.assertRaises(fs.FsRejected):fs._configuration(value)

    def test_privileged_controlled_helper_profiles_reject_reconstruction_before_dispatch(self):
        for context in ({'state':'COMPLETED'}, object(), None):
            with self.subTest(context=context), self.assertRaises(fs.FsRejected):
                fs._HelperProfile(fs._TOKEN,'controlled-native',[] ,['python3','caller'],False,_controlled=context)

@unittest.skipUnless(sys.platform == 'linux' and os.geteuid() == 0, 'actual initdb and exact-type refusal')
class ProvisionDispatchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='kwc4-dispatch-unit-',dir='/root')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.initdb=self.root/'initdb.sh'
        self.initdb.write_bytes((Path(__file__).resolve().parents[1]/'deploy/p0c4_restore_initdb.sh').read_bytes())
        self.initdb.chmod(0o444)
        self.case=str(uuid.uuid4())
    def test_new_private_seam_rejects_unminted_type_without_target_files(self):
        import p0c4_restore_target as target
        for profile in (None,object(),{'state':'COMPLETED'},True):
            if profile is None:continue
            with self.subTest(profile=profile), self.assertRaises(target.AdmissionError):
                target._provision(self.root,self.case,'172.30.241.0/24',self.initdb,_controlled_profile=profile)
        self.assertEqual(sorted(p.name for p in self.root.iterdir()),['initdb.sh'])
    def test_default_public_entry_does_not_expose_context_selector(self):
        import p0c4_restore_target as target
        with self.assertRaises(TypeError):
            target.provision(self.root,self.case,'172.30.241.0/24',self.initdb,_controlled_profile=object())
    def test_legacy_facts_selector_cannot_be_switched_to_full_by_untyped_context(self):
        import p0c4_restore_target_birth as birth
        with self.assertRaises(birth.AdmissionError):
            birth.probe_pg_facts({},'a'*64,_controlled_profile={'role':'source'})


@unittest.skipUnless(sys.platform == 'linux' and os.geteuid() == 0, 'actual root Linux filesystem test')
class NativeOwnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='kwc4-controlled-unit-', dir='/root')
        batch, source, target = (str(uuid.uuid4()) for _ in range(3))
        suite = Path(self.temp.name) / 'volumes' / ('kwc4c-suite-' + uuid.uuid4().hex) / '_data'
        case = suite / 'controlled-import/batches' / batch
        src = case / 'source'
        self.repo = Path(__file__).resolve().parents[1]
        files = ('scripts/p0c4_completion/full_target_fs.py', 'scripts/p0c4_restore_target.py',
            'scripts/p0c4_restore_target_pin_prepare.py', 'scripts/p0c4_restore_target_pin.py',
            'scripts/p0c4_restore_target_birth.py', 'scripts/p0c4_restore_birth_acceptance.py',
            'deploy/p0c4_restore_initdb.sh',
            'deploy/p0c4_full_restore_initdb.sh')
        for name in files:
            path = src / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((self.repo / name).read_bytes())
            path.chmod(0o444 if name.startswith('deploy/') else 0o400)
        for path in sorted(suite.rglob('*')):
            if path.is_dir():path.chmod(0o700)
        suite.chmod(0o700)
        self.root = case / 'source-control'
        self.root.mkdir(mode=0o700)
        (case / 'initdb.sh').write_bytes((src / 'deploy/p0c4_restore_initdb.sh').read_bytes())
        (case / 'initdb.sh').chmod(0o444)
        self.config = dict(batch_id=batch, source_case_id=source, target_case_id=target,
            subnet='172.30.241.0/24', root=str(self.root), suite_root=str(suite),
            source_root=str(src), case_root=str(case), role='source')
        spec = importlib.util.spec_from_file_location('native_controlled_owner_unit', src / files[0])
        self.native = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.native)
        self.previous_path = list(sys.path)
        self.previous_modules = {n: sys.modules.get(n) for n in ('p0c4_restore_target',
            'p0c4_restore_target_pin_prepare', 'p0c4_restore_target_pin', 'p0c4_restore_target_birth')}
        for name in self.previous_modules:sys.modules.pop(name, None)
        sys.path.insert(0, str(src / 'scripts'))
        factory = getattr(self.native, '_new_controlled_owner', None)
        self.assertTrue(callable(factory), 'missing fixed two-role native owner factory')
        self.owner = factory()

    def tearDown(self):
        if hasattr(self, 'owner'):self.owner.close()
        if hasattr(self, 'previous_path'):sys.path[:] = self.previous_path
        for name, old in getattr(self, 'previous_modules', {}).items():
            if old is None:sys.modules.pop(name, None)
            else:sys.modules[name] = old
        self.temp.cleanup()

    def test_native_setup_holds_actual_same_lock_and_actual_root_inode(self):
        import fcntl
        result = self.owner.perform('setup', self.config)
        observed = os.fstat(self.owner.root_fd)
        self.assertEqual((result['root']['dev'], result['root']['ino']), (observed.st_dev, observed.st_ino))
        with (self.root / '.restore-target.lock').open('rb') as other:
            with self.assertRaises(BlockingIOError):fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.owner.close()
            fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def test_actual_root_replacement_rejected_with_original_handle_still_open(self):
        self.owner.perform('setup', self.config)
        original = os.fstat(self.owner.root_fd)
        saved = self.root.with_name('retained-original')
        self.root.rename(saved)
        self.root.mkdir(mode=0o700)
        with self.assertRaises(self.native.FsRejected):self.owner._recheck()
        self.assertEqual(os.fstat(self.owner.root_fd).st_ino, original.st_ino)

    def test_full_initdb_in_fixture_location_rejected_before_lock_or_roots(self):
        initdb = Path(self.config['case_root']) / 'initdb.sh'
        initdb.write_bytes((self.repo / 'deploy/p0c4_full_restore_initdb.sh').read_bytes())
        with self.assertRaises(Exception):self.owner.perform('setup', self.config)
        self.assertIsNone(self.owner.root_fd)
        self.assertFalse((self.root / '.restore-target.lock').exists())
        self.assertEqual(list(self.root.iterdir()), [])

    def test_setup_symlink_root_cannot_replace_native_directory(self):
        self.root.rmdir()
        self.root.symlink_to(self.root.parent, target_is_directory=True)
        with self.assertRaises(Exception):self.owner.perform('setup', self.config)
        self.assertIsNone(self.owner.root_fd)

    def test_malformed_created_or_birth_before_files_publishes_nothing(self):
        self.owner.perform('setup', self.config)
        # Actual holder has not published files, pinned absence or owned create.
        with self.assertRaises(self.native.FsRejected):self.owner.perform('birth', {})
        with self.assertRaises(self.native.FsRejected):self.owner.perform('created', {})
        self.assertEqual(list(self.root.glob('targets/*')), [])

    def test_actual_eof_after_file_creation_retains_native_lock_without_new_requests(self):
        import fcntl
        script = Path(self.config['source_root']) / 'scripts/p0c4_completion/full_target_fs.py'
        child = subprocess.Popen([sys.executable, '-B', '-u', str(script), '--fixed-controlled-owner'],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env={'PATH': '/usr/bin:/bin', 'PYTHONPATH': str(script.parents[1]), 'LC_ALL': 'C'})
        session = str(uuid.uuid4())
        before = dict(daemon_id='local-unit-no-docker', containers=[], networks=[], volumes=[], routes=[])
        try:
            for seq, (op, value) in enumerate((('setup', self.config), ('pin', dict(before=before)), ('files', dict(before=before)))):
                child.stdin.write(json.dumps(dict(session=session, seq=seq, op=op, value=value)).encode() + b'\n')
                child.stdin.flush()
                import select
                self.assertTrue(select.select([child.stdout], [], [], 5)[0], 'fixed owner response deadline')
                reply = json.loads(child.stdout.readline())
                self.assertTrue(reply['ok'], reply)
            child.stdin.close()
            time.sleep(0.1)
            self.assertIsNone(child.poll())
            with (self.root / '.restore-target.lock').open('rb') as other:
                with self.assertRaises(BlockingIOError):fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertTrue((self.root / 'targets' / self.config['source_case_id'] / 'failure.json').is_file())
        finally:
            if not child.stdin.closed:child.stdin.close()
            child.terminate()
            child.wait(timeout=5)
            child.stdout.close();child.stderr.close()


if __name__ == '__main__':unittest.main(verbosity=2)
