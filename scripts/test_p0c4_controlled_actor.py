"""Focused private actor contracts. Docker boundary fixtures are not native credit."""
import copy,hashlib,json,os,pickle,stat,subprocess,sys,tempfile,time,unittest
from pathlib import Path
from unittest.mock import patch
from p0c4_completion import controlled_fixture as f

class ActorContracts(unittest.TestCase):
 def snapshot(self):
  return dict(format_version=1,capability='controlled_actor_live_observation_v1',pidns=dict(dev=4,ino=5),mntns=dict(dev=4,ino=6),pid1=dict(starttime=700,uids=[0]*4,cmdline=['python3','-c','import time; time.sleep(86400)'],pidns=dict(dev=4,ino=5),mntns=dict(dev=4,ino=6)),suite=dict(dev=8,ino=9,uid=0,mode=0o700))
 def test_same_pid1_suite_but_different_mount_namespace_rejected(self):
  value=self.snapshot();raw=f._canonical(value)+b'\n'
  self.assertEqual(f._actor_validate_sample(raw,value),value)
  for group in ('pidns','mntns','suite'):
   changed=copy.deepcopy(value);changed[group]['ino']+=1
   with self.subTest(group=group),self.assertRaises(f.ControlledRejected):f._actor_validate_sample(f._canonical(changed)+b'\n',value)
  for mutation in ('tick','uid','extra','bool','multiline','noeof'):
   changed=copy.deepcopy(value)
   if mutation=='tick':changed['pid1']['starttime']+=1
   if mutation=='uid':changed['pid1']['uids']=[1]*4
   if mutation=='extra':changed['unknown']=True
   if mutation=='bool':changed['format_version']=True
   raw=f._canonical(changed)+b'\n'
   if mutation=='multiline':raw+=raw
   if mutation=='noeof':raw=raw.rstrip(b'\n')
   with self.subTest(mutation=mutation),self.assertRaises(f.ControlledRejected):f._actor_validate_sample(raw,value)

 def test_single_candidate_does_not_choose_first_or_admit_repeated_short_ids(self):
  cid='a'*64
  self.assertEqual(f._actor_candidate((cid+'\n').encode()),cid)
  for raw in (b'',(cid+'\n'+cid+'\n').encode(),('a'*12+'\n').encode(),b'x'*513,(cid+' '+('b'*64)).encode()):
   with self.assertRaises(f.ControlledRejected):f._actor_candidate(raw)

 def test_exact_main_exec_contracts_no_caller_supplied_argv(self):
  suite=Path('/var/lib/docker/volumes/kwc4c-suite-'+'a'*32+'/_data');adapter=str(suite/'current11/adapter.py');eid='b'*64;cid='c'*64
  with patch.object(sys,'argv',[adapter,'--fixed-prepare-plan']):entry,args=f._actor_main_contract(suite)
  self.assertEqual(entry,'python3');self.assertEqual(args,['-B','-u',adapter,'--fixed-prepare-plan'])
  value=dict(ID=eid,Running=True,ExitCode=None,ProcessConfig=dict(privileged=False,user='0:0',tty=False,entrypoint=entry,arguments=args),OpenStdin=False,OpenStdout=True,OpenStderr=True,CanRemove=False,ContainerID=cid,DetachKeys='',Pid=121)
  f._actor_validate_main(value,eid,cid,(entry,args))
  for mutation in ('wrongcid','otherargv','notrunning','boolpid','stdin','privileged','extra'):
   changed=copy.deepcopy(value)
   if mutation=='wrongcid':changed['ContainerID']='d'*64
   if mutation=='otherargv':changed['ProcessConfig']['arguments']=['-c','print(1)']
   if mutation=='notrunning':changed['Running']=False
   if mutation=='boolpid':changed['Pid']=True
   if mutation=='stdin':changed['OpenStdin']=True
   if mutation=='privileged':changed['ProcessConfig']['privileged']=True
   if mutation=='extra':changed['unknown']=0
   with self.subTest(mutation=mutation),self.assertRaises(f.ControlledRejected):f._actor_validate_main(changed,eid,cid,(entry,args))
  with patch.object(sys,'argv',[adapter,'--unknown']):
   with self.assertRaises(f.ControlledRejected):f._actor_main_contract(suite)
  for code in (0,1,True,False,'0',[],{}):
   changed=copy.deepcopy(value);changed['ExitCode']=code
   with self.subTest(main_exit_code=code),self.assertRaises(f.ControlledRejected):f._actor_validate_main(changed,eid,cid,(entry,args))

 @unittest.skipUnless(sys.platform=='linux' and os.geteuid()==0,'actual Linux held namespace/proc/root directory')
 def test_actual_held_pid_mount_and_suite_fds_recheck(self):
  with tempfile.TemporaryDirectory() as folder,patch.object(f,'_ACTOR_INTERPRETER',None):
   os.chmod(folder,0o700);held=f._ActorHeld(Path(folder))
   try:
    first=held.sample();self.assertEqual(first,held.sample())
    with self.assertRaises(TypeError):pickle.dumps(held)
    self.assertEqual(first['pidns'],first['pid1']['pidns'])
    self.assertEqual(first['mntns'],first['pid1']['mntns'])
    self.assertEqual(first['suite']['ino'],os.stat(folder).st_ino)
    self.assertGreater(first['pid1']['starttime'],0)
    moved=Path(folder).with_name(Path(folder).name+'-moved');os.rename(folder,moved);Path(folder).mkdir(mode=0o700)
    try:
     with self.assertRaises(f.ControlledRejected):held.sample()
    finally:Path(folder).rmdir();os.rename(moved,folder)
   finally:held.close()
   with self.assertRaises(f.ControlledRejected):held.sample()

 def test_unadopted_interpreter_fails_before_rpc_and_no_hostname_fallback(self):
  class NoRunner:
   def inspect(self,*a,**k):raise AssertionError('NO_RPC_BEFORE_INTERPRETER_ADOPTION')
   def docker(self,*a,**k):raise AssertionError('NO_RPC_BEFORE_INTERPRETER_ADOPTION')
  with patch.object(f,'_ACTOR_INTERPRETER',None):
   with self.assertRaisesRegex(f.ControlledRejected,'CONTROLLED_ACTOR_INTERPRETER_NOT_ADOPTED'):f._measure_outer(NoRunner(),Path('/var/lib/docker/volumes/kwc4c-suite-'+'a'*32+'/_data'))

 def test_rpc_order_once_unknown_main_and_post_exit_drift(self):
  suite=Path('/var/lib/docker/volumes/kwc4c-suite-'+'a'*32+'/_data');cid='b'*64;eid='c'*64;local=self.snapshot();adapter=str(suite/'current11/adapter.py');calls=[]
  facts=dict(Id=cid,Name='/kwc4c-control-'+'d'*32,Image=f._BUILDER,RestartCount=0,ExecIDs=[eid],
   Config=dict(Image=f._BUILDER,User='0:0',Hostname='dev-linux',Entrypoint=['python3'],Cmd=['-c','import time; time.sleep(86400)'],Env=['PYTHONDONTWRITEBYTECODE=1','PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin','RUSTUP_HOME=/usr/local/rustup','CARGO_HOME=/usr/local/cargo','RUST_VERSION=1.97.0','CARGO_TARGET_DIR=/app/target'],Labels={'knowweave.c4.ops':'completion-prep','org.opencontainers.image.source':'https://github.com/rust-lang/docker-rust'}),
   HostConfig=dict(NetworkMode='host',ReadonlyRootfs=True,Privileged=False,PidMode='',CapAdd=None,CapDrop=['ALL'],SecurityOpt=['no-new-privileges'],Memory=2*1024**3,NanoCpus=2*10**9,PidsLimit=256),
   State=dict(Running=True,Status='running',Pid=123,StartedAt='2026-10-11T01:00:00Z',Paused=False,Restarting=False,OOMKilled=False,Dead=False,ExitCode=0,Error=''),
   Mounts=[dict(Type='volume',Name=suite.parent.name,Source=str(suite),Destination=str(suite),RW=True),*[dict(Type='bind',Source=p,Destination=p,RW=False) for p in ('/usr/bin/docker','/usr/libexec/docker/cli-plugins/docker-compose','/var/run/docker.sock')],dict(Type='bind',Source='/proc/1/ns/net',Destination='/run/knowweave-c4/host-netns',RW=False)])
  main=dict(ID=eid,Running=True,ExitCode=None,ProcessConfig=dict(privileged=False,user='0:0',tty=False,entrypoint='python3',arguments=['-B','-u',adapter,'--fixed-prepare-plan']),OpenStdin=False,OpenStdout=True,OpenStderr=True,CanRemove=False,ContainerID=cid,DetachKeys='',Pid=124)
  f._actor_live(facts,suite)
  poisoned=copy.deepcopy(facts);poisoned['Config']['Env'].append('LD_PRELOAD=/unexpected.so')
  with self.assertRaises(f.ControlledRejected):f._actor_live(poisoned,suite)
  class Held:
   def __init__(self,path):pass
   def sample(self):calls.append('held');return copy.deepcopy(local)
   def close(self):calls.append('close')
  class Runner:
   def run(self,argv,**kw):
    self.calls.append(argv);self.kw.append(kw)
    if argv[1:2]==['ps']:
     calls.append('inventory')
     return 0,((cid+'\n')*2 if self.mode=='duplicate' else cid+'\n').encode(),b''
    if argv[1:3]==['container','inspect']:
     calls.append('inspect');row=copy.deepcopy(facts)
     if self.mode=='drift' and calls.count('inspect')==2:row['State']['StartedAt']='2026-10-11T02:00:00Z'
     if self.mode=='extra-exec' and calls.count('inspect')==2:row['ExecIDs'].append('e'*64)
     return 0,f._canonical([row]),b''
    if argv[1:2]==['exec']:
     calls.append('sampler')
     if self.mode=='unknown':raise OSError('UNKNOWN_ONCE')
     if self.mode=='stderr':return 0,f._canonical(local)+b'\n',b'warning'
     if self.mode=='noexit':return None,f._canonical(local)+b'\n',b''
     if self.mode=='wrong-singleton':
      other=copy.deepcopy(local);other['mntns']['ino']+=1;other['pid1']['mntns']['ino']+=1
      return 0,f._canonical(other)+b'\n',b''
     return 0,f._canonical(local)+b'\n',b''
    raise AssertionError(argv)
   def __init__(self,mode):self.mode=mode;self.calls=[];self.kw=[]
  def get(runner,e,c,expected,deadline):calls.append('get');return copy.deepcopy(main)
  with patch.object(sys,'argv',[adapter,'--fixed-prepare-plan']),patch.object(f,'_actor_interpreter',return_value='/actual/from-local-fixture'),patch.object(f,'_ActorHeld',Held),patch.object(f,'_actor_main_get',side_effect=get),patch.object(f,'_actor_sampler_program',return_value='PINNED_LOCAL_BOUNDARY_FIXTURE'):
   runner=Runner('safe');result=f._measure_outer(runner,suite)
   self.assertEqual(result['container_id'],cid)
   self.assertEqual(calls,['held','inventory','inspect','get','sampler','inspect','get','inventory','held','close'])
   self.assertTrue(all(0<row['timeout']<=3 for row in runner.kw))
   for mode in ('unknown','stderr','noexit','drift','extra-exec','wrong-singleton'):
    calls.clear();runner=Runner(mode)
    with self.subTest(mode=mode),self.assertRaises((f.ControlledRejected,OSError)):f._measure_outer(runner,suite)
    self.assertEqual(calls.count('sampler'),1);self.assertEqual(calls[-1],'close')
    if mode in ('unknown','stderr','noexit','wrong-singleton'):self.assertEqual(calls.count('inspect'),1)
    self.assertEqual(calls.count('inventory'),1)
   calls.clear();runner=Runner('duplicate')
   with self.assertRaises(f.ControlledRejected):f._measure_outer(runner,suite)
   self.assertEqual(calls,['held','inventory','close'])
   calls.clear();facts['ExecIDs']=None;runner=Runner('safe')
   f._measure_outer(runner,suite)
   self.assertEqual(calls,['held','inventory','inspect','sampler','inspect','inventory','held','close'])
   self.assertEqual(calls.count('get'),0)

 @unittest.skipUnless(sys.platform=='linux' and os.geteuid()==0,'actual isolated Linux process')
 def test_actual_actor_sampler_startup_injection_and_runner_timeout(self):
  path=Path(sys.executable).resolve();meta=path.stat();pin=dict(path=str(path),bytes=meta.st_size,sha256=hashlib.sha256(path.read_bytes()).hexdigest(),uid=meta.st_uid,gid=meta.st_gid,mode=stat.S_IMODE(meta.st_mode),nlink=meta.st_nlink)
  with tempfile.TemporaryDirectory() as folder:
   root=Path(folder);os.chmod(root,0o700);marker=root/'startup-marker'
   (root/'sitecustomize.py').write_text('open('+repr(str(marker))+',"w").write("INJECTED")\n')
   with patch.object(f,'_ACTOR_INTERPRETER',pin):program=f._actor_sampler_program(root)
   env=dict(os.environ,PYTHONPATH=folder,PYTHONSTARTUP=str(root/'sitecustomize.py'))
   answer=subprocess.run([str(path),'-I','-S','-B','-u','-c',program],cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=5)
   # WSL PID1 is /init, not the controlled fixed sleep. Actual entry refuses.
   self.assertEqual(answer.returncode,1);self.assertEqual(answer.stdout,b'');self.assertFalse(marker.exists())
   self.assertEqual(answer.stderr,b'CONTROLLED_ACTOR_SAMPLER_UNKNOWN\n')
   for flags in (['-I','-B','-u'],['-S','-B','-u'],['-I','-S','-u']):
    bad=subprocess.run([str(path),*flags,'-c',program],cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=5)
    self.assertNotEqual(bad.returncode,0)
   import p0c4_source_admission_gate_acceptance as admission
   logs=root/'logs';logs.mkdir(mode=0o700);runner=admission.Runner(logs)
   with self.assertRaises(admission.GateError):f._actor_rpc(runner,[str(path),'-I','-S','-B','-u','-c','import time;print("partial",flush=True);time.sleep(2)'],time.monotonic()+.2)
   record=json.loads((logs/'0001.process.json').read_bytes())
   self.assertEqual(record['reason'],'PROCESS_TIMEOUT');self.assertEqual((logs/'0001.stdout').read_bytes(),b'partial\n')

 def test_fixed_main_get_raw_boundary_closed_cap_and_no_other_endpoint(self):
  import socket
  cid='a'*64;eid='b'*64;expected=('python3',['-B','-u','/fixed/adapter.py','--fixed-success'])
  value=dict(ID=eid,Running=True,ExitCode=None,ProcessConfig=dict(privileged=False,user='0:0',tty=False,entrypoint=expected[0],arguments=expected[1]),OpenStdin=False,OpenStdout=True,OpenStderr=True,CanRemove=False,ContainerID=cid,DetachKeys='',Pid=123)
  class Wire:
   def __init__(self,data):self.data=data;self.sent=[];self.connected=[]
   def __enter__(self):return self
   def __exit__(self,*args):pass
   def settimeout(self,value):self.timeout=value
   def connect(self,path):self.connected.append(path)
   def sendall(self,data):self.sent.append(data)
   def recv(self,cap):r,self.data=self.data[:cap],self.data[cap:];return r
  body=f._canonical(value);good=b'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: '+str(len(body)).encode()+b'\r\n\r\n'+body
  peer=Wire(good)
  with patch.object(socket,'socket',return_value=peer):self.assertEqual(f._actor_get_wire(eid,cid,expected,1),value)
  self.assertEqual(peer.connected,['/var/run/docker.sock']);self.assertEqual(peer.sent,[('GET /exec/'+eid+'/json HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nConnection: close\r\n\r\n').encode()])
  chunked=b'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n'+hex(len(body))[2:].encode()+b'\r\n'+body+b'\r\n0\r\n\r\n'
  with patch.object(socket,'socket',return_value=Wire(chunked)):self.assertEqual(f._actor_get_wire(eid,cid,expected,1),value)
  for bad in (good+b'extra',good.replace(b'200 OK',b'404 Missing'),good.replace(b'Content-Length:',b'Unknown-Length:'),b'x'*8193,chunked[:-2]):
   with patch.object(socket,'socket',return_value=Wire(bad)),self.assertRaises((f.ControlledRejected,ValueError)):f._actor_get_wire(eid,cid,expected,1)

class FixedSubnetSelectionContracts(unittest.TestCase):
 """Pure sealed allocator contracts; no Docker/route input injection/runtime."""
 def test_original_pool_empty_preserves_original_first_two(self):
  self.assertEqual(f._choose_subnets([]),['172.29.0.0/24','172.29.1.0/24'])
 def test_actual_old_whole_primary_pool_uses_only_fixed_fallback(self):
  self.assertEqual(f._choose_subnets(['172.29.0.0/16']),['10.252.0.0/24','10.252.1.0/24'])
 def test_both_fixed_pools_occupied_refuse(self):
  with self.assertRaisesRegex(f.ControlledRejected,'FRESH_SUBNET_CAPACITY'):
   f._choose_subnets(['172.29.0.0/16','10.252.0.0/16'])
 def test_only_one_candidate_is_insufficient_for_two_roles(self):
  occupied=['172.29.0.0/16']+[f'10.252.{i}.0/24' for i in range(255)]
  with self.assertRaisesRegex(f.ControlledRejected,'FRESH_SUBNET_CAPACITY'):f._choose_subnets(occupied)
 def test_routes_exclude_fallback_candidates_without_pool_override(self):
  self.assertEqual(f._choose_subnets(['172.29.0.0/16','10.252.0.0/23']),['10.252.2.0/24','10.252.3.0/24'])

if __name__=='__main__':unittest.main(verbosity=2)
