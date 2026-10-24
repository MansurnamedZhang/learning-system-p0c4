"""Portable fixed-FD protocol tests; no Docker, Root FD or native credit."""
import base64
import builtins
import copy
import io
import json
from pathlib import Path
import stat
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from scripts import p0c4_source_lifecycle_gate_acceptance as driver
from scripts.p0c4_completion import attention as a, full_import as full
from scripts.test_p0c4_attention_acceptance import plan


class _ProofDisk:
    """Linux FD/proc facade. Open FDs retain nodes across pathname replacement."""
    O_RDONLY=0;O_RDWR=2;O_CREAT=64;O_EXCL=128;O_NOFOLLOW=256
    O_DIRECTORY=512;O_CLOEXEC=1024;O_NONBLOCK=2048
    def __init__(self,data):
        self.nodes={};self.fds={};self.serial=3;self.inode=100;self.pid=1;self.ticks=777
        self.argv=[];self.cmdline=b'';self.opens=[];self.synced=[]
        self._node('/attention-proofs',b'',0o700,kind='dir')
        for name,raw in data.items():self._node('/attention-proofs/'+name,raw,0o600)
    def _node(self,path,data,mode,kind='file'):
        self.inode+=1;node=dict(path=path,data=data,mode=mode,kind=kind,ino=self.inode,nlink=2 if kind=='dir'else 1,uid=0,gid=0,mtime=10,ctime=11)
        self.nodes[path]=node;return node
    def _path(self,path,dir_fd=None):return self.fds[dir_fd]['node']['path']+'/'+str(path) if dir_fd is not None else str(path)
    def _stat(self,node):
        kind={'dir':stat.S_IFDIR,'file':stat.S_IFREG,'symlink':stat.S_IFLNK,'fifo':stat.S_IFIFO}[node['kind']]
        return SimpleNamespace(st_dev=1,st_ino=node['ino'],st_uid=node['uid'],st_gid=node['gid'],st_mode=kind|node['mode'],
            st_nlink=node['nlink'],st_size=4096 if node['kind']=='dir'else len(node['data']),st_mtime_ns=node['mtime'],st_ctime_ns=node['ctime'])
    def stat(self,path,dir_fd=None,follow_symlinks=False):return self._stat(self.nodes[self._path(path,dir_fd)])
    def lstat(self,path):return self.stat(path)
    def fstat(self,fd):return self._stat(self.fds[fd]['node'])
    def open(self,path,flags,mode=0o777,dir_fd=None):
        path=self._path(path,dir_fd);self.opens.append((path,flags))
        if path.startswith('/proc/1/fd/'):
            node=self.fds[int(path.rsplit('/',1)[1])]['node']
        else:
            if flags&self.O_CREAT:
                if path in self.nodes and flags&self.O_EXCL:raise FileExistsError(path)
                if path not in self.nodes:
                    node=self._node(path,b'',mode);parent=self.nodes.get(path.rsplit('/',1)[0])
                    if parent:parent['mtime']+=1;parent['ctime']+=1
            node=self.nodes[path]
            if node['kind']=='symlink'and flags&self.O_NOFOLLOW:raise OSError('nofollow')
            if flags&self.O_DIRECTORY and node['kind']!='dir':raise NotADirectoryError(path)
        self.serial+=1;self.fds[self.serial]=dict(node=node,offset=0);return self.serial
    def mkdir(self,path,mode):
        if path in self.nodes:raise FileExistsError(path)
        self._node(path,b'',mode,kind='dir')
    def pread(self,fd,count,offset):return self.fds[fd]['node']['data'][offset:offset+count]
    def write(self,fd,raw):
        state=self.fds[fd];node=state['node'];offset=state['offset'];node['data']=node['data'][:offset]+raw+node['data'][offset+len(raw):]
        state['offset']+=len(raw);node['mtime']+=1;node['ctime']+=1;return len(raw)
    def fchmod(self,fd,mode):self.fds[fd]['node']['mode']=mode;self.fds[fd]['node']['ctime']+=1
    def fsync(self,fd):self.synced.append(self.fds[fd]['node']['path'])
    def close(self,fd):self.fds.pop(fd)
    def getpid(self):return self.pid
    def geteuid(self):return 0
    def getegid(self):return 0
    def readlink(self,path):
        node=self.fds[int(str(path).rsplit('/',1)[1])]['node']
        return node['path']+(' (deleted)'if node['nlink']==0 else '')
    def proc_open(self,path,mode):
        if path=='/proc/1/cmdline':return io.BytesIO(self.cmdline)
        if path=='/proc/1/stat':return io.BytesIO(('1 (python3) S '+'0 '*18+str(self.ticks)+' 0 0\n').encode())
        raise AssertionError('unapproved builtin open')
    def replace(self,name,raw):
        path='/attention-proofs/'+name;old=self.nodes[path];old['nlink']=0;old['ctime']+=1;self._node(path,raw,0o600)
    def inventory(self):
        root=self.stat('/attention-proofs')
        value={'proofs':dict(directory=True,dev=root.st_dev,ino=root.st_ino,uid=0,mode=0o700)}
        for path,node in self.nodes.items():
            if path.startswith('/attention-proofs/'):
                value['proofs/'+path.rsplit('/',1)[1]]=dict(bytes=len(node['data']),sha256=a.sha(node['data']),mode=node['mode'],uid=node['uid'],links=node['nlink'])
        return value


def _scope(fs,*,stdin=b'',stdout=None):
    original=builtins.__import__;fake_sys=SimpleNamespace(platform='linux',argv=fs.argv,stdin=SimpleNamespace(buffer=stdin if hasattr(stdin,'readline')else io.BytesIO(stdin)),stdout=SimpleNamespace(buffer=stdout or io.BytesIO()))
    def imports(name,*args,**kwargs):
        if name=='os':return fs
        if name=='sys':return fake_sys
        return original(name,*args,**kwargs)
    return {'__builtins__':dict(vars(builtins),__import__=imports,open=fs.proc_open)}


def _small_bundle(case):return {name:a.canonical(dict(unit_only=name))for name,cap,key in a.fixed_proof_files(case)}
def _sized(raw_size):
    prefix=b'{"unit_only":"';suffix=b'"}'
    return prefix+b'x'*(raw_size-len(prefix)-len(suffix))+suffix


class _Harness:
    """Host API facade around the actual actor and chunk program source."""
    def __init__(self,case='attention-full-chain',data=None):
        self.case=case;self.data=_small_bundle(case)if data is None else data;self.fs=_ProofDisk(self.data);self.helpers=[];self.calls=[];self.events=[]
        self.chunk_request_change=None;self.chunk_response_change=None;self.before_verify=None;self.inspect_change=None
        self.finish_code=0;self.finish_stderr=b'';self.extra_owner_stdout=b''
        self.runner=SimpleNamespace(budget=SimpleNamespace(remaining=lambda:180),logs=Path('/unit-only-logs'))
        value=plan(case);self.context=full.FullRehearsalContext(full._TOKEN,value);self.context._runner=self.runner
        self.context._phase='COMPLETE';self.context._case=Path('/unit-case');self.context._snapshot_source_mountpoint='/fixed-source'
        self.context._h=SimpleNamespace(LIMIT=driver.LIMIT,BUILDER='builder');self.context._binding=object();self.context._images={'builder':dict(Id='pinned-builder')};self.context._result=dict(helpers=[])
        self.record=dict(identity=dict(case_id=value['case_id']),container_id='source',test=self.context._selected_test(),stage='actual-libtest',full_profile_stage='VERIFIED',outcome=dict(exit_code=0,passed=1,failed=0,ignored=0),volumes=dict(source_volume=dict(Name='source-volume',Mountpoint='/fixed-source',Driver='local',Options=None)))
        self.context._record=self.record;self.inventory_raw=driver.canonical(self.fs.inventory());self.record['inventory_sha256']=a.sha(self.inventory_raw)
        self.profile=dict(source_case_id=value['case_id'],source_container_id='source',volumes=dict(source_data=dict(name='source-volume',path='/fixed-source',dev=1)))
        outer=self
        class Helper:
            def __init__(self,*args,**kwargs):
                assert kwargs==dict(interactive=True)
                assert args[5]==[('bind','/fixed-source/proofs',a.PROOF_FD_ROOT,True)]
                assert args[6][:4]==['python3','-I','-u','-c']and args[6][4]==a._proof_fd_program()
                self.command=args[6];self.record=dict(id='f'*64,removed=False);self.closed=False;self.owner=dict(state='known',id='f'*64)
                self.facts=dict(RestartCount=0,State=dict(Status='created',Running=False,Pid=0,ExitCode=0,OOMKilled=False,StartedAt='0001-01-01T00:00:00Z'))
                outer.helpers.append(self);outer.events.extend(('ownership-before-create','returned-id-validated'));self.validate(self.facts)
            def validate(self,facts):return facts
            def inspect(self):
                if outer.inspect_change:outer.inspect_change(self.facts)
                return self.validate(self.facts)
            def close(self):
                outer.events.append('exact-cleanup');self.inspect();self.record['removed']=True;self.closed=True;self.owner['state']='absent'
                if hasattr(self,'actor')and self.actor.handles:self.actor.close()
        class Child:
            def __init__(self,helper):self.helper=helper;self.out=bytearray();self.closed=False
            def close_input(self):
                outer.events.append('stdin-eof');self.closed=True;self.helper.actor.final();self.helper.actor.close()
                self.helper.facts['State'].update(Status='exited',Running=False,Pid=0,ExitCode=0)
            def finish(self,terminate_on_error):
                assert terminate_on_error is False and self.closed;outer.events.append('natural-exit')
                return outer.finish_code,bytes(self.out)+outer.extra_owner_stdout,outer.finish_stderr
        class Broker:
            def __init__(self,helper,logs,budget):
                self.helper=helper;self.buffer=bytearray();self.child=Child(helper);helper.child=self.child
                outer.fs.argv=['-c',helper.command[-2],helper.command[-1]];outer.fs.cmdline=b'\0'.join(arg.encode()for arg in helper.command)+b'\0'
                helper.facts['State'].update(Status='running',Running=True,Pid=123,StartedAt='2026-10-24T00:00:01Z');helper.inspect()
                scope=_scope(outer.fs);common=a._proof_fd_program().split(a._PROOF_OWNER_MAIN)[0];exec(compile(common,'actual-fixed-proof-common','exec'),scope)
                helper.scope=scope;helper.actor=scope['FixedProofActor']()
            def call(self,value):
                outer.events.append(value['op'])
                if value['op']=='init':result=self.helper.actor.open(value)
                elif value['op']=='verify':
                    if outer.before_verify:outer.before_verify(outer)
                    result=self.helper.actor.verify(value)
                else:result=self.helper.actor.stop(value)
                raw=driver.canonical(result)+b'\n';assert len(raw)<=16384;self.child.out.extend(raw);assert len(self.child.out)<65536
                return result
        self.Helper,self.Broker=Helper,Broker
    def pg_exec(self,runner,container,argv,**kwargs):
        assert runner is self.runner
        if container=='source':
            assert not kwargs and argv[0]=='cat'and argv[1].startswith('/var/lib/knowweave-source/proofs/')
            name=argv[1].rsplit('/',1)[1];raw=self.fs.nodes['/attention-proofs/'+name]['data']
            if len(raw)>driver.LIMIT:raise driver.GateError('PROCESS_LOG_BUDGET')
            self.calls.append(dict(kind='cat',name=name,bytes=len(raw)));return 0,raw,b''
        assert container=='f'*64 and argv==['python3','-I','-c',a._proof_fd_program(chunk=True)]and set(kwargs)=={'stdin'}
        request=json.loads(kwargs['stdin'])
        if self.chunk_request_change:self.chunk_request_change(self,request)
        out=io.BytesIO();self.fs.pid=2
        try:exec(compile(argv[-1],'actual-fixed-proof-chunk','exec'),_scope(self.fs,stdin=driver.canonical(request),stdout=out))
        finally:self.fs.pid=1
        raw=out.getvalue()
        if self.chunk_response_change:
            value=json.loads(raw);self.chunk_response_change(value);raw=driver.canonical(value)+b'\n'
        assert len(raw)<driver.LIMIT;self.calls.append(dict(kind='chunk',name=request['name'],bytes=len(raw),offset=request['offset']))
        return 0,raw,b''
    def read(self):
        with patch.object(full,'_measured_regular',return_value=(self.inventory_raw,{})),patch.object(full.FullRehearsalContext,'_profile_observation',return_value=self.profile),\
             patch.object(driver,'HelperContainer',self.Helper),patch.object(driver,'BrokerClient',self.Broker),patch.object(driver,'pg_exec',side_effect=self.pg_exec),patch.object(driver,'runner_base',side_effect=lambda r:r):
            return a.read_fixed_bundle(driver,self.runner,'source',self.case,context=self.context)
    def cleanup(self):
        for helper in self.helpers:
            if not helper.closed:helper.close()


class AttentionProofReadBudgetTests(unittest.TestCase):
    def large(self,case='attention-missing-asset',size=None):
        data=_small_bundle(case);name=next(n for n,c,k in a.fixed_proof_files(case)if c==16*1024**2)
        data[name]=_sized(driver.LIMIT+17 if size is None else size);return _Harness(case,data),name
    def test_all_six_small_bundles_keep_exact_original_cat_order_and_zero_helpers(self):
        for case in ('attention-full-chain',*a.EXPECTED_REJECTIONS):
            with self.subTest(case=case):
                h=_Harness(case);self.assertEqual(h.read(),h.data);self.assertEqual(h.helpers,[])
                self.assertEqual([v['name']for v in h.calls],[n for n,c,k in a.fixed_proof_files(case)])
    def test_all_six_large_bundles_use_separate_bounded_exec_and_complete_bytes(self):
        for case in ('attention-full-chain',*a.EXPECTED_REJECTIONS):
            with self.subTest(case=case):
                h,name=self.large(case);self.assertEqual(h.read(),h.data);self.assertEqual(len(h.helpers),1)
                self.assertTrue(h.record['attention_proof_reader']['removed']);self.assertFalse(h.fs.fds)
                self.assertTrue(all(c['bytes']<=driver.LIMIT for c in h.calls));self.assertLess(len(h.helpers[0].child.out),65536)
                self.assertLess(h.events.index('natural-exit'),h.events.index('exact-cleanup'))
    def test_original_process_boundary_and_original_sixteen_mebibyte_cap(self):
        for size,helper in ((driver.LIMIT,False),(driver.LIMIT+1,True),(16*1024**2,True)):
            with self.subTest(size=size):
                h,name=self.large(size=size);self.assertEqual(h.read()[name],h.data[name]);self.assertEqual(bool(h.helpers),helper)
        h,name=self.large(size=16*1024**2+1)
        with self.assertRaisesRegex(a.AttentionError,'ATTENTION_PROOF_FIXED_FILE_INVENTORY'):h.read()
        self.assertFalse(h.helpers);self.assertFalse(h.calls)
    def test_every_actual_source_open_is_nofollow_and_regular_open_is_nonblocking(self):
        h,name=self.large();h.read()
        for path,flags in h.fs.opens:
            if path.startswith('/attention-proofs'):
                self.assertTrue(flags&h.fs.O_NOFOLLOW)
                if path!='/attention-proofs':self.assertTrue(flags&h.fs.O_NONBLOCK)
        state=h.fs.nodes['/tmp/attention-proof-reader/state.json'];self.assertEqual((state['mode'],state['nlink'],state['uid'],state['gid']),(0o400,1,0,0))
    def test_nonce_fd_owner_offset_and_typed_metadata_must_match_real_actor_registry(self):
        mutations=[lambda r:r.update(nonce='11111111-1111-4111-8111-111111111111'),lambda r:r.update(fd=r['fd']+1),lambda r:r.update(fd=True),
            lambda r:r.update(name='secrets/admin.dsn'),lambda r:r['owner'].update(starttime=r['owner']['starttime']+1),lambda r:r['owner'].update(pid=True),
            lambda r:r.update(offset=1),lambda r:r.update(count=True),lambda r:r['meta'].update(ino=True)]
        for index,change in enumerate(mutations):
            with self.subTest(index=index):
                h,name=self.large();h.chunk_request_change=lambda host,r:change(r)
                with self.assertRaises((RuntimeError,KeyError,a.AttentionError)):h.read()
                self.assertFalse(h.helpers[0].closed);self.assertNotIn('attention_proof_reader',h.record);h.cleanup();self.assertFalse(h.fs.fds)
    def test_file_path_replacement_cannot_substitute_for_held_fd(self):
        h,name=self.large();h.chunk_request_change=lambda host,r:host.fs.replace(name,host.data[name])
        with self.assertRaises(RuntimeError):h.read()
        self.assertFalse(h.helpers[0].closed);h.cleanup();self.assertFalse(h.fs.fds)
    def test_final_full_hash_rejects_changed_content_even_when_all_metadata_is_unchanged(self):
        h,name=self.large()
        def mutate(host):
            node=host.fs.nodes['/attention-proofs/'+name];node['data']=b'Z'+node['data'][1:]
        h.before_verify=mutate
        with self.assertRaises(RuntimeError):h.read()
        self.assertFalse(h.helpers[0].closed);h.cleanup()
    def test_registry_link_mode_or_owner_replacement_refuses_before_any_proc_fd(self):
        for key,value in (('nlink',2),('mode',0o600),('uid',1000)):
            with self.subTest(key=key):
                h,name=self.large();h.chunk_request_change=lambda host,r:host.fs.nodes['/tmp/attention-proof-reader/state.json'].update({key:value})
                with self.assertRaises(RuntimeError):h.read()
                self.assertFalse(any(path.startswith('/proc/1/fd/')for path,flags in h.fs.opens));h.cleanup()
    def test_helper_restart_epoch_is_checked_by_the_specialized_original_validator(self):
        for key,value in (('RestartCount',1),('StartedAt','2026-10-24T00:00:02Z'),('Pid',456)):
            with self.subTest(key=key):
                h,name=self.large()
                def change(host,request):
                    host.inspect_change=lambda facts:facts.update(RestartCount=value) if key=='RestartCount'else facts['State'].update({key:value})
                h.chunk_request_change=change
                with self.assertRaisesRegex(a.AttentionError,'ATTENTION_PROOF_HELPER_'):h.read()
                h.inspect_change=None;h.helpers[0].facts['RestartCount']=0;h.helpers[0].facts['State'].update(StartedAt='2026-10-24T00:00:01Z',Pid=123);h.cleanup()
    def test_wrong_short_or_reordered_chunk_receipts_never_reach_settlement(self):
        for change in (lambda r:r.update(offset=r['offset']+1),lambda r:r['owner'].update(pid=True),lambda r:r.update(base64=r['base64'][:-4]),lambda r:r.update(sha256='f'*64)):
            h,name=self.large();h.chunk_response_change=change
            with self.assertRaises((a.AttentionError,ValueError)):h.read()
            self.assertFalse(h.helpers[0].closed);self.assertNotIn('attention_proof_reader',h.record);h.cleanup()
    def test_nonzero_stderr_or_extra_owner_stdout_never_claim_natural_success(self):
        for code,err,extra in ((1,b'',b''),(0,b'warning',b''),(0,b'',b'\n')):
            h,name=self.large();h.finish_code=code;h.finish_stderr=err;h.extra_owner_stdout=extra
            with self.assertRaisesRegex(a.AttentionError,'ATTENTION_PROOF_FD_NATURAL_EXIT'):h.read()
            self.assertFalse(h.helpers[0].closed);self.assertNotIn('attention_proof_reader',h.record);h.cleanup()
    def test_inventory_digest_wrong_runner_or_wrong_case_refuses_before_cat_or_helper(self):
        for change in (lambda h:h.record.update(inventory_sha256='f'*64),lambda h:setattr(h.context,'_phase','PROFILE_MEASURE'),lambda h:h.record['identity'].update(case_id='11111111-1111-4111-8111-111111111111'),lambda h:h.record.update(outcome=True),lambda h:h.record.update(identity=True)):
            h=_Harness();change(h)
            with self.assertRaises(a.AttentionError):h.read()
            self.assertFalse(h.helpers);self.assertFalse(h.calls)
    def test_unknown_chunk_transport_failure_keeps_owned_helper_for_outer_cleanup(self):
        h,name=self.large();original=h.pg_exec
        def fail(runner,container,argv,**kwargs):
            if container!='source':raise driver.GateError('PROCESS_TIMEOUT')
            return original(runner,container,argv,**kwargs)
        h.pg_exec=fail
        with self.assertRaisesRegex(driver.GateError,'PROCESS_TIMEOUT'):h.read()
        self.assertEqual(len(h.helpers),1);self.assertFalse(h.helpers[0].closed);self.assertNotIn('attention_proof_reader',h.record)
        self.assertNotIn('natural-exit',h.events);self.assertNotIn('exact-cleanup',h.events);h.cleanup();self.assertFalse(h.fs.fds)
    def test_init_capture_mismatch_keeps_owned_helper_and_refuses_all_chunks(self):
        h,name=self.large();h.fs.nodes['/attention-proofs/'+name]['data']=b'Q'+h.data[name][1:]
        with self.assertRaises(RuntimeError):h.read()
        self.assertEqual(len(h.helpers),1);self.assertFalse(h.helpers[0].closed);self.assertFalse(h.calls)
        self.assertNotIn('attention_proof_reader',h.record);h.cleanup();self.assertFalse(h.fs.fds)
    def test_original_monitored_child_still_rejects_two_mebibytes_plus_one(self):
        with tempfile.TemporaryDirectory()as directory:
            child=driver.MonitoredChild([sys.executable,'-c','import sys;sys.stdout.buffer.write(b"x"*(2*1024**2+1))'],Path(directory),timeout=10)
            with self.assertRaisesRegex(driver.GateError,'PROCESS_LOG_BUDGET'):child.finish()
            self.assertEqual(child.limit,2*1024**2);self.assertEqual(child.reason,'PROCESS_LOG_BUDGET')
    def test_actual_actor_stdio_has_only_small_frames_and_requires_natural_eof(self):
        for extra in (b'',b'x'):
            with self.subTest(extra=extra):
                case='attention-full-chain';nonce='11111111-1111-4111-8111-111111111111';fs=_ProofDisk(_small_bundle(case));program=a._proof_fd_program()
                fs.argv=['-c',case,nonce];fs.cmdline=b'\0'.join(v.encode()for v in ['python3','-I','-u','-c',program,case,nonce])+b'\0'
                inventory=fs.inventory();root={k:inventory['proofs'][k]for k in ('dev','ino','uid','mode')};files={n:inventory['proofs/'+n]for n,c,k in a.fixed_proof_files(case)}
                queue=[driver.canonical(dict(op='init',case=case,nonce=nonce,root=root,files=files))+b'\n'];frames=[]
                class Input:
                    def readline(self,cap):return queue.pop(0)if queue else b''
                    def read(self,count):return extra
                class Output:
                    def write(self,raw):
                        frames.append(raw);value=json.loads(raw)
                        if 'format_version'in value:queue.append(driver.canonical(dict(op='verify',nonce=nonce,opened_sha256=a.sha(raw[:-1])))+b'\n')
                        elif 'verified'in value:queue.append(driver.canonical(dict(op='stop',nonce=nonce,opened_sha256=value['opened_sha256']))+b'\n')
                        return len(raw)
                    def flush(self):pass
                if extra:
                    with self.assertRaises(RuntimeError):exec(compile(program,'actual-fixed-actor-main','exec'),_scope(fs,stdin=Input(),stdout=Output()))
                else:exec(compile(program,'actual-fixed-actor-main','exec'),_scope(fs,stdin=Input(),stdout=Output()))
                self.assertEqual(len(frames),3);self.assertTrue(all(len(v)<=16384 for v in frames));self.assertLess(sum(map(len,frames)),65536);self.assertFalse(fs.fds)


if __name__=='__main__':unittest.main(verbosity=2)
