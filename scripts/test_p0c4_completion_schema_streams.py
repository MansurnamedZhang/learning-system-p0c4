"""Stream contract logic and separately labelled root/Linux filesystem gates.

Windows logic tests read real temporary file bytes. Their OS boundary bridge
supplies Linux metadata/descriptor semantics and earns NO Linux FS credit.
"""
from contextlib import ExitStack, contextmanager
import hashlib
import os
from pathlib import Path
import stat
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import uuid

from scripts.p0c4_completion import schema as lane


class LogicalFileBoundary:
    """Only a unit-test OS boundary: payload opens/reads are actual file I/O."""
    def __init__(self):
        self.paths={};self.next_fd=1000000;self.opened=[];self.mutate=None
        self.real_open=os.open;self.real_close=os.close
        self.real_fstat=os.fstat;self.real_stat=os.stat;self.real_read=os.read

    def open(self,path,flags,*args,dir_fd=None,**kwargs):
        path=Path(path) if dir_fd is None else self.paths[dir_fd]/path
        self.opened.append(path)
        if path.is_symlink():raise OSError('logical NOFOLLOW rejection')
        if path.is_dir():
            self.next_fd+=1;self.paths[self.next_fd]=path;return self.next_fd
        return self.real_open(path,os.O_RDONLY|getattr(os,'O_BINARY',0))

    def close(self,fd):
        if fd in self.paths:del self.paths[fd]
        else:self.real_close(fd)

    def metadata(self,value,directory=False):
        return SimpleNamespace(**{key:getattr(value,key) for key in ('st_dev','st_ino','st_size','st_mtime_ns','st_ctime_ns')},st_uid=0,st_mode=(stat.S_IFDIR|0o700) if directory else (stat.S_IFREG|0o600),st_nlink=1)

    def fstat(self,fd):
        if fd in self.paths:return self.metadata(self.real_stat(self.paths[fd]),True)
        return self.metadata(self.real_fstat(fd))

    def stat(self,path,*,dir_fd=None,follow_symlinks=True):
        path=Path(path) if dir_fd is None else self.paths[dir_fd]/path
        value=self.real_stat(path,follow_symlinks=follow_symlinks)
        return self.metadata(value,stat.S_ISDIR(value.st_mode))

    def read(self,fd,size):
        if self.mutate:
            action=self.mutate;self.mutate=None;action()
        return self.real_read(fd,size)

    @contextmanager
    def active(self):
        with ExitStack() as stack:
            for name,value in [('open',self.open),('close',self.close),('fstat',self.fstat),('stat',self.stat),('read',self.read),('geteuid',lambda:0),('O_DIRECTORY',0),('O_NOFOLLOW',0),('O_CLOEXEC',0),('O_NONBLOCK',0)]:
                stack.enter_context(patch.object(lane.os,name,value,create=True))
            stack.enter_context(patch.object(lane.registry,'open_directory',side_effect=lambda path:self.open(path,0)))
            stack.enter_context(patch.object(lane.registry,'child_directory',side_effect=lambda fd,name:self.open(name,0,dir_fd=fd)))
            # Windows cannot fsync a read-only CRT descriptor. This is a logic
            # bridge only; the unbridged Linux tests keep actual fsync/FD rules.
            stack.enter_context(patch.object(lane.os,'fsync',lambda fd:None))
            yield self


class StreamFixtures(unittest.TestCase):
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory(dir=getattr(self,'filesystem_root',None));self.addCleanup(self.temporary.cleanup)
        self.root=Path(self.temporary.name)
        self.plan=dict(evidence_root=str(self.root),batch_id=str(uuid.uuid4()),case_id=str(uuid.uuid4()),case_name='full-schema-contract')
        self.logs=lane._batch(self.root,self.plan)/'evidence/logs'
        self.native=lane._batch(self.root,self.plan)/self.plan['case_id']/'schema/native-supervisor'
        for path in (self.logs,self.native):path.mkdir(parents=True)
        self.boundary=LogicalFileBoundary()

    def leaf(self,domain,name,raw=b''):
        parent=self.logs if domain=='raw' else self.native
        path=parent/name;path.write_bytes(raw)
        meta=path.stat();relative='logs/'+name if domain=='raw' else 'native-supervisor/'+name
        row=dict(path=relative,size=len(raw),sha256=hashlib.sha256(raw).hexdigest(),dev=meta.st_dev,ino=meta.st_ino)
        if domain=='native':row['mode']=0o600
        return path,row

    def read(self,domain,name,rows,budget=None):
        return lane._read_stream(self.root,self.plan,domain,name,rows,[0] if budget is None else budget)


class StreamLogicTests(StreamFixtures):
    def test_raw_consumer_reads_real_empty_stderr_before_resource_validation(self):
        # Reintroducing private_read for stream bytes fails this consumer test.
        h,b,d,p=lane._modules();archive=b'archive';manifest=dict(base_commit='a'*40)
        self.plan['source']=dict(archive=dict(path=str(self.root/'source.zip'),size=len(archive),sha256=p.sha(archive)),manifest=dict(path=str(self.root/'manifest.json'),sha256=p.sha(lane.envelope.canonical(manifest))),application_commit='a'*40)
        (self.root/'source.zip').write_bytes(archive);(self.root/'manifest.json').write_bytes(lane.envelope.canonical(manifest))
        row=dict(identity=d.completion_case_identity(self.plan['case_id']),test=lane.CASE_TESTS[self.plan['case_name']])
        result=dict(batch_id=self.plan['batch_id'],case_id=self.plan['case_id'],base_commit='a'*40,archive_sha256=p.sha(archive),classification=lane.PASS,cases=[row],cleanup_verified=False)
        files=[]
        for name,raw in [('0001.stdout',b'actual stdout'),('0001.stderr',b''),('0001.process.json',lane.envelope.canonical(dict(exit_code=0,reason=None,stdout_bytes=13,stderr_bytes=0)))]:
            files.append(self.leaf('raw',name,raw)[1])
        index=dict(format_version=1,capability='c4_source_raw_index_v1',files=sorted(files,key=lambda v:v['path']),scan_bytes=3*sum(v['size'] for v in files))
        indexpath=lane._batch(self.root,self.plan)/'evidence/raw-index.json';indexraw=lane.envelope.canonical(index);indexpath.write_bytes(indexraw)
        original=dict(path=indexpath.relative_to(self.root).as_posix(),size=len(indexraw),sha256=p.sha(indexraw))
        def ref(name,raw):
            path=self.root/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(raw)
            return dict(path=name,size=len(raw),sha256=p.sha(raw))
        receipt=dict(raw_index=ref('raw/index.json',lane.envelope.canonical(dict(format_version=1,capability='c4_schema_raw_index_v1',original_index=original))),resource_receipt=ref('resources.json',lane.envelope.canonical(result)),capture_index=None,status='FAILED_UNUSABLE',reason_code='BODY_FAILED')
        observed=[]
        class ReachedResourceValidation(Exception):pass
        def stop(originals,*args,**kwargs):observed.extend(originals);raise ReachedResourceValidation()
        with self.boundary.active(),patch.object(h,'verify_package',return_value=(manifest,{})),patch.object(b,'source_digest',return_value='unused'),patch.object(lane,'_verify_resource_originals',side_effect=stop):
            with self.assertRaises(ReachedResourceValidation):lane._verify_contents(self.root,receipt,self.plan)
        self.assertEqual([(o,e) for o,e,_ in observed],[(b'actual stdout',b'')])
        self.assertIn(self.logs/'0001.stderr',self.boundary.opened)

    def test_native_consumer_compares_real_empty_and_nonempty_streams(self):
        # The production content reader is shared with _verify_contents.
        rows={};cases=[]
        for i,name in enumerate(('receiver_cancel','deadline','output_cap','native_exit')):
            streams={}
            for stream in ('stdout','stderr'):
                raw=b'actual error' if name=='native_exit' and stream=='stderr' else b''
                _,row=self.leaf('native','native-'+name+'.'+stream,raw);rows[row['path']]=row;streams[stream]=raw
            wait=dict(pid=i+1,wait_completed=True,success=False,exit_code=1,stderr_bytes=len(streams['stderr']),stderr_sha256=hashlib.sha256(streams['stderr']).hexdigest())
            cases.append(dict(case=name,held_inode_observed=True,wait=wait,stdout_bytes=0,stdout_sha256=hashlib.sha256(b'').hexdigest()))
        cases.append(dict(case='held_client_hash_mismatch',child_started=False,path_substitution_claim=False))
        proof=dict(classification=lane.PASS,tables=64,decoded_sha256='b'*64,native_cases=cases)
        path=self.native.parent/'full-schema-contract.json';raw=lane.envelope.canonical(proof);path.write_bytes(raw)
        rows[path.name]=dict(path=path.name,size=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        rows['decoded']=dict(path='validation-x/decoded',sha256='b'*64)
        native_budget=[0]
        def read(name):
            return lane._read_content(self.root,self.plan,rows,name,native_budget)
        with self.boundary.active():lane._verify_case_output(read,rows,{},self.plan)
        for name in rows:
            if name.startswith('native-supervisor/'):self.assertIn(self.native/Path(name).name,self.boundary.opened)

    def test_fixed_domain_index_and_capacity_rejections_precede_io(self):
        _,row=self.leaf('raw','0001.stderr')
        for domain,name in [('control','0001.stderr'),('raw','0001.process.json'),('raw','../0001.stderr'),('native','native-unknown.stdout'),('native','first-version.stderr')]:
            with self.subTest(domain=domain,name=name),self.assertRaises(lane.SchemaError):self.read(domain,name,{row['path']:row})
        for bad in [dict(row,size=2*1024**2+1),dict(row,path='logs/0002.stderr'),dict(row,sha256='invalid')]:
            with self.subTest(row=bad),self.assertRaises(lane.SchemaError):self.read('raw','0001.stderr',{row['path']:bad})
        with self.assertRaises(lane.SchemaError):self.read('raw','0001.stderr',{})
        with self.assertRaises(lane.SchemaError):self.read('raw','0001.stderr',{str(i):row for i in range(4097)})
        for name,size in [('native-output_cap.stdout',2),('native-deadline.stderr',8193)]:
            _,row=self.leaf('native',name)
            with self.subTest(name=name),self.assertRaises(lane.SchemaError):self.read('native',name,{row['path']:dict(row,size=size)})
        _,row=self.leaf('raw','0001.stdout',b'ab')
        with self.assertRaises(lane.SchemaError):self.read('raw','0001.stdout',{row['path']:row},[536870911])

    def test_empty_control_fileref_stays_rejected(self):
        with self.assertRaises(lane.envelope.CompletionError):lane.evidence.validate_ref(dict(path='empty.json',size=0,sha256=hashlib.sha256(b'').hexdigest()),8192)


@unittest.skipUnless(sys.platform=='linux' and hasattr(os,'geteuid') and os.geteuid()==0 and os.environ.get('TEST_C4_SCHEMA_STREAM_FS_ROOT'),'requires actual root/Linux and fresh private TEST_C4_SCHEMA_STREAM_FS_ROOT; Windows logic is not this gate')
class LinuxPhysicalStreamTests(StreamFixtures):
    def setUp(self):
        # Ops supplies a fresh private root below trusted non-writable ancestors.
        # /tmp is deliberately unsuitable for the production trusted-root walk.
        self.filesystem_root=os.environ['TEST_C4_SCHEMA_STREAM_FS_ROOT']
        super().setUp()
        for path in [self.root,*self.root.rglob('*')]:
            if path.is_dir():path.chmod(0o700)

    def private_leaf(self,domain,name,raw=b''):
        path,row=self.leaf(domain,name,raw);path.chmod(0o600);return path,row

    def test_linux_empty_nonempty_missing_and_symlink(self):
        for domain,name in [('raw','0001.stdout'),('raw','child-'+'a'*32+'.stderr'),('native','native-deadline.stderr')]:
            for raw in (b'',b'actual Linux bytes'):
                path,row=self.private_leaf(domain,name,raw)
                self.assertEqual(self.read(domain,name,{row['path']:row}),raw)
                path.unlink()
                with self.assertRaises(OSError):self.read(domain,name,{row['path']:row})
                target=path.with_name('target');target.write_bytes(raw);target.chmod(0o600);path.symlink_to(target)
                with self.assertRaises(OSError):self.read(domain,name,{row['path']:row})
                path.unlink();target.unlink()

    def test_linux_wrong_owner_mode_hardlink_and_parent_symlink(self):
        path,row=self.private_leaf('raw','0001.stderr')
        for action in ('mode','owner','link'):
            if action=='mode':path.chmod(0o644)
            elif action=='owner':os.chown(path,1,-1)
            else:os.link(path,self.logs/'second-link')
            with self.subTest(action=action),self.assertRaises(lane.SchemaError):self.read('raw','0001.stderr',{row['path']:row})
            path.chmod(0o600);os.chown(path,0,-1)
            (self.logs/'second-link').unlink(missing_ok=True)
        moved=self.logs.with_name('moved');self.logs.rename(moved);self.logs.symlink_to(moved,target_is_directory=True)
        with self.assertRaises(OSError):self.read('raw','0001.stderr',{row['path']:row})

    def test_linux_nonregular_fifo_is_rejected_without_blocking(self):
        path,row=self.private_leaf('raw','0001.stderr');path.unlink();os.mkfifo(path,0o600)
        value=path.stat();row.update(dev=value.st_dev,ino=value.st_ino)
        with self.assertRaises(lane.SchemaError):self.read('raw','0001.stderr',{row['path']:row})

    def test_linux_wrong_hash_device_inode_size_and_budget(self):
        path,row=self.private_leaf('raw','0001.stdout',b'ab')
        for change in [dict(dev=row['dev']+1),dict(ino=row['ino']+1),dict(size=1),dict(sha256='0'*64)]:
            with self.subTest(change=change),self.assertRaises(lane.SchemaError):self.read('raw','0001.stdout',{row['path']:dict(row,**change)})
        budget=[536870910]
        self.assertEqual(self.read('raw','0001.stdout',{row['path']:row},budget),b'ab')
        with self.assertRaises(lane.SchemaError):self.read('raw','0001.stdout',{row['path']:row},budget)
        self.assertEqual(budget,[536870912])

    def test_linux_replacement_growth_truncation_and_timestamp_changes(self):
        for kind in ('replace','grow','truncate','time','parent'):
            path,row=self.private_leaf('raw','0001.stdout',b'original')
            real_read=os.read;triggered=False
            def read(fd,size):
                nonlocal triggered
                if not triggered:
                    triggered=True
                    if kind=='replace':
                        replacement=path.with_suffix('.replacement');replacement.write_bytes(b'original');replacement.chmod(0o600);os.replace(replacement,path)
                    elif kind=='grow':path.write_bytes(b'original extra')
                    elif kind=='truncate':path.write_bytes(b'')
                    elif kind=='time':os.utime(path,ns=(1,1))
                    else:
                        moved=self.logs.with_name('replaced-logs');self.logs.rename(moved);self.logs.mkdir(mode=0o700)
                return real_read(fd,size)
            with self.subTest(kind=kind),patch.object(lane.os,'read',side_effect=read),self.assertRaises(lane.SchemaError):self.read('raw','0001.stdout',{row['path']:row})

    def test_linux_empty_process_and_control_records_keep_nonempty_rule(self):
        for name in ('0001.process.json','0001.teardown.json','manifest.json'):
            path,_=self.private_leaf('raw',name)
            with self.subTest(name=name),self.assertRaisesRegex(lane.registry.RegistryError,'REGISTRY_PRIVATE_FILE'):lane.evidence.private_read(path,8192)


if __name__=='__main__':unittest.main()
