"""Portable lifecycle regressions; no Docker, SQL, or Linux acceptance claim."""
import ast
import base64
import builtins
import contextlib
import inspect
import io
import json
from pathlib import Path, PurePosixPath
import stat
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from scripts import p0c4_source_lifecycle_gate_acceptance as driver
from scripts.p0c4_completion import attention, full_import, full_target_fs, snapshot_worker
from scripts.test_p0c4_attention_acceptance import plan
from scripts.test_p0c4_storage_registry import receipt_fixture


class _Disk:
    """Small Linux-FD test double executing the actual fixed helper programs."""
    O_RDONLY=0; O_RDWR=2; O_CREAT=64; O_EXCL=128; O_NOFOLLOW=256
    O_DIRECTORY=512; O_CLOEXEC=1024; SEEK_SET=0

    def __init__(self):
        self.nodes={};self.fds={};self.serial=100;self.synced=[]
        for path in ('/var/lib/knowweave-source','/fresh-effects','/fresh-export'):
            self._node(path,True,0o755)

    def _node(self,path,directory,mode):
        self.serial+=1
        self.nodes[path]=dict(directory=directory,mode=mode,ino=self.serial,data=b'')

    def _path(self,path,dir_fd=None):
        value=str(path)
        return str(PurePosixPath(self.fds[dir_fd]['path'])/value) if dir_fd is not None else value

    def Path(self,path):
        disk=self
        class Entry:
            def __init__(self,value):self.value=str(value)
            def __str__(self):return self.value
            def __truediv__(self,other):return disk.Path(PurePosixPath(self.value)/other)
            @property
            def name(self):return PurePosixPath(self.value).name
            def lstat(self):return disk.stat(self.value)
            def iterdir(self):return [disk.Path(PurePosixPath(self.value)/name) for name in disk.listdir_path(self.value)]
            def mkdir(self,mode=0o777):disk.mkdir(self.value,mode)
            def chmod(self,mode):disk.nodes[self.value]['mode']=mode
        return Entry(path)

    def listdir_path(self,path):
        return [PurePosixPath(key).name for key in self.nodes if str(PurePosixPath(key).parent)==path]

    def stat(self,path,dir_fd=None,follow_symlinks=False):
        path=self._path(path,dir_fd);node=self.nodes[path]
        return SimpleNamespace(st_dev=1 if path.startswith('/var/') else 2 if path.startswith('/fresh-effects') else 3,
            st_ino=node['ino'],st_uid=0,st_gid=0,st_mode=(stat.S_IFDIR if node['directory'] else stat.S_IFREG)|node['mode'],
            st_nlink=2+sum(self.nodes[path+'/'+name]['directory'] for name in self.listdir_path(path)) if node['directory'] else 1,
            st_size=len(node['data']))

    def mkdir(self,path,mode,dir_fd=None):
        path=self._path(path,dir_fd)
        if path in self.nodes:raise FileExistsError(path)
        self._node(path,True,mode)

    def open(self,path,flags,mode=0o777,dir_fd=None):
        path=self._path(path,dir_fd)
        if flags&self.O_CREAT:
            if path in self.nodes and flags&self.O_EXCL:raise FileExistsError(path)
            if path not in self.nodes:self._node(path,False,mode)
        node=self.nodes[path]
        if flags&self.O_DIRECTORY and not node['directory']:raise NotADirectoryError(path)
        self.serial+=1;fd=self.serial;self.fds[fd]=dict(path=path,offset=0);return fd

    def fstat(self,fd):return self.stat(self.fds[fd]['path'])
    def listdir(self,fd):return self.listdir_path(self.fds[fd]['path'])
    def close(self,fd):del self.fds[fd]
    def dup(self,fd):self.serial+=1;self.fds[self.serial]=self.fds[fd];return self.serial
    def fsync(self,fd):self.synced.append(self.fds[fd]['path'])
    def lseek(self,fd,offset,whence):self.fds[fd]['offset']=offset
    def geteuid(self):return 0
    def getegid(self):return 0

    def read(self,fd,count):
        handle=self.fds[fd];raw=self.nodes[handle['path']]['data'];start=handle['offset']
        handle['offset']+=len(raw[start:start+count]);return raw[start:start+count]

    def fdopen(self,fd,mode):
        disk=self
        class Writer:
            def __enter__(self):return self
            def __exit__(self,*args):disk.close(fd)
            def fileno(self):return fd
            def flush(self):pass
            def write(self,raw):
                handle=disk.fds[fd];node=disk.nodes[handle['path']];offset=handle['offset']
                node['data']=node['data'][:offset]+raw+node['data'][offset+len(raw):]
                handle['offset']+=len(raw);return len(raw)
        return Writer()

    def execute(self,program,request=b''):
        original=builtins.__import__
        def imports(name,*args,**kwargs):
            if name=='os':return self
            if name=='pathlib':return SimpleNamespace(Path=self.Path)
            if name=='sys':return SimpleNamespace(platform='linux',stdin=SimpleNamespace(buffer=io.BytesIO(request)))
            return original(name,*args,**kwargs)
        output=io.StringIO()
        with contextlib.redirect_stdout(output):
            exec(compile(program,'actual-fixed-helper','exec'),{'__builtins__':dict(vars(builtins),__import__=imports)})
        return output.getvalue().encode()


def _issuer_empty_guard(disk):
    # Execute the original issuer predicates, rather than copying a permissive
    # approximation. The real driver decides when this admission is reached.
    tree=ast.parse(inspect.getsource(driver.broker_main))
    init=next(node for node in ast.walk(tree) if isinstance(node,ast.If) and ast.unparse(node.test)=="operation == 'init'")
    guard=next(node for node in init.body if isinstance(node,ast.If) and ast.unparse(node.test)=='snapshot')
    root_guard=init.body[init.body.index(guard)-1]
    exec(compile(ast.Module(body=[root_guard,guard],type_ignores=[]),'actual-issuer-empty-guard','exec'),
         dict(root=disk.Path(snapshot_worker.ROOT),snapshot=True,stat=stat,require=driver.require))


class AttentionPreseedLifecycleTests(unittest.TestCase):
    def _run_actual_driver_schedule(self,before_finalize=None,helper_result=None,events=None):
        disk=_Disk();events=[] if events is None else events;value=plan();source_id=value['case_id'];test=full_import.ATTENTION_CASE_TESTS[value['case_name']]
        context=full_import.FullRehearsalContext(full_import._TOKEN,value)
        source_name='kwc4c-'+source_id.replace('-','')+'-source'
        source=dict(Name=source_name,Mountpoint='/docker/volumes/'+source_name+'/_data',Driver='local',Options=None)
        record=dict(test=test,identity=driver.completion_case_identity(source_id),volumes=dict(source_volume=source),stage='independent-issuer',
                    archive_sha256=value['source']['application_build_sha256'])
        context._record=record;context._case=Path('/unit-case');context._profile_root=Path('/unit-profile')
        context._ident=record['identity'];context._initdb=Path('/unit-initdb');context._target_identity=dict(project='target')
        context._target_status=dict(container_id='b'*64,network_id='c'*64,birth_sha256='d'*64)
        for key in ('volume','build_volume','registry_volume'):record['volumes'][key]=dict(Name=key,Mountpoint='/unit/'+key)
        context._target_volumes={key:dict(Name=key,Mountpoint='/unit/target-'+key) for key in ('pg','control','socket')}
        record.update(container_id='a'*64,network_id='e'*64)
        written={};context._h=SimpleNamespace(BUILDER='builder',write_new=lambda path,raw,*args:written.__setitem__(str(path),raw))
        context._binding=object();context._result=dict(batch_id=value['batch_id']);context._images={'builder':{}}
        context._runner=SimpleNamespace(docker=lambda *args:b'unit-daemon',inspect=lambda *args:dict(Mounts=[]))
        sql=dict(database='learning_backup_c4_task3_'+source_id,database_oid=400,system_identifier='12345')
        record['sql_identity']=sql

        def prepare_helper(h,b,runner,result,image,mounts,command,**kwargs):
            events.append('prepare-empty-roots')
            return 0,disk.execute(command[-1]),b''

        class Broker:
            def call(self,message):
                events.append('issuer-empty-admission');_issuer_empty_guard(disk)
                binding=dict(format_version=1,capability='source_control_binding_v1',binding_id='22222222-2222-4222-8222-222222222222',
                    control_path=snapshot_worker.ROOT+'/control',control_dev=1,control_ino=900,**sql)
                pin=driver.digest(driver.canonical(binding));receipt=receipt_fixture(pin,1,900)
                receipt.update(batch_id=value['batch_id'],case_id=source_id,source_package_sha256=record['archive_sha256'],
                               application_commit=value['source']['application_commit'],application_build_sha256=record['archive_sha256'])
                root=next(row for row in receipt['roots'] if row['kind']=='source_assets');root.update(dev=1,ino=disk.stat(root['path']).st_ino)
                return dict(binding=binding,binding_sha256=pin,sql=sql,issuer_euid=0,
                            classification='INDEPENDENT_PRECOMPILE_BINDING',registry_provisioning=receipt)

        class Helper:
            def __init__(self,*args,**kwargs):
                events.append('preseed-created');self.record=dict(id='f'*64,name='unit-helper',removed=False)
                self.command=args[6];self.mounts=args[5]
                self_test.assertEqual(self.mounts,[('bind',source['Mountpoint']+'/assets',snapshot_worker.ROOT+'/assets',False)])
                self_test.assertEqual(kwargs,dict(interactive=True))
            def run(self,stdin,timeout):
                self_test.assertEqual(timeout,30);self_test.assertIn('issuer-empty-admission',events)
                self_test.assertEqual(events[-1],'preseed-created');events.append('preseed-request')
                out=disk.execute(self.command[-1],stdin);self.record['removed']=True
                events.append('preseed-natural-exit-cleanup')
                return helper_result(out) if helper_result is not None else (0,out,b'')

        self_test=self
        def measure(c,phase,volumes):
            return {key:dict(identity=dict(path=row['Mountpoint'],dev=1,ino=index+1,uid=0,gid=0,mode=0o700))
                    for index,(key,row) in enumerate(volumes.items())}
        def regular(path,cap):
            return written.get(str(path),b'unit-file'),dict(path=str(path),dev=1,ino=1,uid=0,gid=0,mode=0o444)
        def projection(c,mounts):events.append('source-mounts-verified-readonly')
        def compile_case(*args,**kwargs):
            events.append('compile');self.assertEqual(sum(len(n['data']) for n in disk.nodes.values()),601)
            self.assertEqual(sum(not n['directory'] for n in disk.nodes.values()),5)
            expected={snapshot_worker.ROOT+'/assets/'+row['storage_key']:base64.b64decode(row['base64']) for row in attention.fixed_originals()}
            self.assertEqual({path:node['data'] for path,node in disk.nodes.items() if not node['directory']},expected)
            self.assertTrue(set(expected)<=set(disk.synced),'each original must be fsynced')

        # Select and execute the actual driver's relevant statements in source
        # order. No test-written ordering of prepare/issuer/finalize/compile.
        tree=ast.parse(inspect.getsource(driver.live_case));selected=[]
        for node in ast.walk(tree):
            if not isinstance(node,(ast.Expr,ast.Assign)):continue
            text=ast.unparse(node)
            if (text.startswith('snapshot_driver.prepare_volumes(') or text.startswith("record['issuer'] = validate_issuer(")
                or text.startswith("h.write_new(case / 'issuer.json'") or text.startswith("h.write_new(case / 'registry-provisioning.json'")
                or text.startswith("record['pg_root_before'] = root_audit(") or text=='compile_binding(record)'
                or text.startswith('_full_context._finalize_profile(') or text.startswith('env = compile_case(')):
                selected.append(node)
        self.assertEqual(len(selected),8)
        scope=dict(vars(driver),snapshot_effects=True,snapshot_driver=snapshot_worker,h=context._h,b=context._binding,runner=context._runner,
            result=context._result,images=context._images,source=Path('/source'),ident=record['identity'],record=record,_full_context=context,
            expected={},broker=Broker(),init={},observed=sql,case=context._case,container=record['container_id'],manifest={},archive_sha=record['archive_sha256'])
        scope['compile_case']=compile_case
        scope['root_audit']=lambda *args:dict(binding_sha256=record['issuer']['binding_sha256'],control_dev=1,control_ino=900)
        with patch.dict(sys.modules,{'p0c4_source_lifecycle_gate_acceptance':driver}),patch.object(driver,'run_helper',side_effect=prepare_helper),\
             patch.object(driver,'HelperContainer',Helper),patch.object(driver,'creation_call',side_effect=lambda r,o,fn:fn()),\
             patch.object(driver,'runner_base',return_value=SimpleNamespace(ownership=SimpleNamespace(acquire=lambda *args:{},known=lambda *args:None,release=lambda *args:None))),\
             patch.object(full_target_fs,'_measure',side_effect=measure),patch.object(full_import,'_measured_regular',side_effect=regular),\
             patch.object(full_import,'_measured_identity',return_value=dict(path='/unit',dev=1,ino=1,uid=0,gid=0,mode=0o700)),\
             patch.object(full_import,'_validate_profile',side_effect=lambda v:v),patch.object(full_import.FullRehearsalContext,'_source_mount_projection',projection):
            scope['b']=SimpleNamespace(create_volume=lambda h,r,name,labels:dict(Name=name,Mountpoint='/docker/volumes/'+name+'/_data',Driver='local',Options=None,Labels=labels))
            for node in sorted(selected,key=lambda n:n.lineno):
                if before_finalize is not None and ast.unparse(node).startswith('_full_context._finalize_profile('):
                    before_finalize(context,record,written,disk)
                exec(compile(ast.Module(body=[node],type_ignores=[]),'actual-live-case-schedule','exec'),scope)
        self.assertEqual(events,['prepare-empty-roots','issuer-empty-admission','source-mounts-verified-readonly',
            'preseed-created','preseed-request','preseed-natural-exit-cleanup','compile'])
        self.assertEqual(record['attention_originals_preseed']['physical_files'],5)
        self.assertEqual(context._phase,'COMPLETE');self.assertFalse(disk.fds)

    def test_actual_driver_schedule_admits_empty_issuer_before_fixed_bytes_and_compile(self):
        self._run_actual_driver_schedule()

    def test_missing_tampered_or_wrong_case_issuer_cannot_create_helper(self):
        def bad_registry_file(context,record,written,disk):written[str(context._case/'registry-provisioning.json')]+=b' '
        def missing_issuer(context,record,written,disk):record.pop('issuer')
        def wrong_case(context,record,written,disk):record['issuer']['registry_provisioning']['case_id']='11111111-1111-4111-8111-111111111111'
        def bad_binding(context,record,written,disk):record['issuer']['binding_sha256']='0'*64
        for change in (bad_registry_file,missing_issuer,wrong_case,bad_binding):
            events=[]
            with self.subTest(change=change.__name__),self.assertRaises((driver.GateError,attention.AttentionError)):
                self._run_actual_driver_schedule(before_finalize=change,events=events)
            self.assertNotIn('preseed-created',events);self.assertNotIn('compile',events)

    def test_helper_receipt_failure_never_reaches_compile(self):
        def bad_root(out):
            value=json.loads(out);value['assets']['ino']+=1
            return 0,driver.canonical(value)+b'\n',b''
        for result in (lambda out:(1,out,b''),lambda out:(0,out,b'warning'),lambda out:(0,out+b'\n',b''),bad_root):
            events=[]
            with self.subTest(result=result),self.assertRaises(attention.AttentionError):
                self._run_actual_driver_schedule(helper_result=result,events=events)
            self.assertNotIn('compile',events)

    def test_fixed_helper_rejects_wrong_registered_inode_before_creating_any_file(self):
        disk=_Disk();disk.execute(snapshot_worker.PREPARE_ROOTS);root=disk.stat(snapshot_worker.ROOT+'/assets')
        request=dict(format_version=1,capability='attention_fixed_originals_preseed_v1',source_binding_sha256='a'*64,
                     assets=dict(dev=root.st_dev,ino=root.st_ino+1))
        before=dict(disk.nodes)
        with self.assertRaisesRegex(RuntimeError,'attention fixed preseed refused'):
            disk.execute(attention.preseed_program(),driver.canonical(request))
        self.assertEqual(disk.nodes,before);self.assertFalse(disk.fds)

    def test_fixed_helper_rejects_repeat_without_changing_original_bytes(self):
        disk=_Disk();disk.execute(snapshot_worker.PREPARE_ROOTS);root=disk.stat(snapshot_worker.ROOT+'/assets')
        request=driver.canonical(dict(format_version=1,capability='attention_fixed_originals_preseed_v1',source_binding_sha256='a'*64,
                                assets=dict(dev=root.st_dev,ino=root.st_ino)))
        observed=json.loads(disk.execute(attention.preseed_program(),request));attention.validate_preseed_observation(observed['attention_originals'])
        before={path:dict(node) for path,node in disk.nodes.items()}
        with self.assertRaisesRegex(RuntimeError,'attention fixed preseed refused'):
            disk.execute(attention.preseed_program(),request)
        self.assertEqual(disk.nodes,before);self.assertFalse(disk.fds)

    def test_source_pg_assets_mount_stays_readonly(self):
        context=full_import.FullRehearsalContext(full_import._TOKEN,plan())
        context._target_volumes={key:dict(Mountpoint='/unit/'+key,Name=key) for key in ('pg','control','socket')}
        context._profile_root=Path('/unit-profile');context._initdb=Path('/unit-initdb');context._target_root=Path('/unit-target')
        context._snapshot_source_mountpoint='/unit-source';context._snapshot_volumes={key:dict(Name=key) for key in ('snapshot_effects','snapshot_export')}
        document=dict(services=dict(pg=dict(volumes=[])),volumes={});binds=[]
        context._extend_source_document(document,binds,{})
        assets=[row for row in document['services']['pg']['volumes'] if row['target']==snapshot_worker.ROOT+'/assets']
        self.assertEqual(assets,[dict(type='bind',source='/unit-source/assets',target=snapshot_worker.ROOT+'/assets',read_only=True)])


if __name__=='__main__':unittest.main()
