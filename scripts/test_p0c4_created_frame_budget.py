"""Portable request-budget tests; no Docker, native owner or PG authority.

The private reconstructed inventory is opt-in and is never copied into the repo.
The transport double replaces Docker only; Broker framing and owner parsing,
protocol order, request/response budgets and context contract checks stay real.
"""
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import queue
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import p0c4_source_lifecycle_gate_acceptance as driver
from p0c4_completion import controlled_fixture as fixture
from p0c4_completion import full_target_fs as fs

SESSION='11111111-1111-4111-8111-111111111111'
LEGACY=1048576
CREATED=2097152


def frame(size,op='created',**extra):
    value=dict(session=SESSION,seq=3,op=op,value={'padding':''},**extra)
    value['value']['padding']='x'*(size-len(driver.canonical(value)))
    raw=driver.canonical(value)
    assert len(raw)==size
    return raw


class Transport:
    """No Docker dispatch: queue a small response and retain exact sent bytes."""
    def __init__(self,profile=None,context=None,response=b'{"accepted":true}\n'):
        self._full_profile=profile
        self.runner=SimpleNamespace(ownership=SimpleNamespace(_controlled_pair=context))
        self.sent=[]
        self.child=SimpleNamespace(p=SimpleNamespace(poll=lambda:None),events=queue.Queue(),
                                   check=lambda:None,send=self.sent.append)
        self.child.events.put((0,response))
    def persistent(self,logs,timeout):return self.child


def controlled_transport():
    # Only the pure fixed helper contract is needed. No install, native runtime,
    # public constructor, plan parsing or current/old dispatch is executed.
    context=object.__new__(fixture.ControlledFixtureContext)
    context._closed=False;context._native_broken=False
    context._install=SimpleNamespace(recheck=lambda:None)
    context._active='target';context._sides={'target':{}}
    context._suite=PurePosixPath('/var/lib/docker/volumes/kwc4c-suite-'+'a'*32+'/_data')
    context._backend=SimpleNamespace(source=context._suite/'source')
    mounts,command=context._helper_contract('controlled-owner')
    profile=fs._HelperProfile(fs._TOKEN,'controlled-owner',mounts,command,True,_controlled=context)
    return Transport(profile,context)


def broker(transport):return driver.BrokerClient(transport,None,driver.CaseBudget())


class Backend:
    """Native FS is external here; parse/order and the complete values stay real."""
    def __init__(self):self.calls=[];self.disconnected_calls=0
    def perform(self,op,value):
        self.calls.append((op,value))
        if op=='setup':return dict(root=dict(path='/owned',dev=1,ino=2,uid=0,gid=0,mode=0o700),lock_held=True)
        if op=='pin':return dict(precreation_sha256='a'*64)
        if op=='files':return dict(compose_sha256='a'*64)
        if op in ('created','failure'):return dict(recorded=True)
        if op=='check':return dict(checked=True)
        if op=='birth':return dict(birth_sha256='a'*64,issuance_sha256='b'*64,birth_path='/birth',destination_root='/destination',control_root='/control',asset_root='/assets')
        if op=='release':return dict(released=True)
        raise AssertionError(op)
    def disconnected(self):self.disconnected_calls+=1


def owner(raw,*,controlled=True,prefix=True,session=SESSION):
    backend=Backend()
    prior=b''.join(driver.canonical(dict(session=session,seq=i,op=op,value={}))+b'\n'
                   for i,op in enumerate(('setup','pin','files'))) if prefix else b''
    stream=io.BytesIO(prior+raw)
    output=io.StringIO()
    with patch.object(fs,'_new_controlled_owner',return_value=backend), \
         patch.object(fs,'_NativeOwner',return_value=backend), \
         patch.object(fs.sys,'stdin',SimpleNamespace(buffer=stream)), \
         patch.object(fs.sys,'stdout',output),patch.object(fs.os,'umask',return_value=0):
        code=fs._owner_main(_recipe=fs._CONTROLLED_RECIPE if controlled else None)
    return code,backend,output.getvalue(),stream.tell()


class BrokerBudgetTests(unittest.TestCase):
    def test_exact_created_limit_is_sent_complete_with_one_lf(self):
        raw=frame(CREATED);transport=controlled_transport()
        self.assertEqual(broker(transport).call(json.loads(raw)),{'accepted':True})
        self.assertEqual(transport.sent,[raw+b'\n'])

    def test_created_limit_plus_one_is_rejected_before_send_and_latches(self):
        transport=controlled_transport();client=broker(transport)
        with self.assertRaisesRegex(driver.GateError,'BROKER_INPUT_BUDGET'):client.call(json.loads(frame(CREATED+1)))
        with self.assertRaisesRegex(driver.GateError,'BROKER_PROTOCOL_LOST'):client.call({'op':'created'})
        self.assertEqual(transport.sent,[])

    def test_other_operations_keep_one_mib_even_with_budget_selector_keys(self):
        for op in ('pin','files','check','birth','release','setup'):
            with self.subTest(op=op):
                transport=controlled_transport();raw=frame(LEGACY+1,op,budget_profile='controlled-owner',input_budget=CREATED)
                with self.assertRaisesRegex(driver.GateError,'BROKER_INPUT_BUDGET'):broker(transport).call(json.loads(raw))
                self.assertEqual(transport.sent,[])

    def test_default_and_other_profiles_keep_original_body_boundary(self):
        for kind in (None,'owner','pg','roots'):
            profile=None if kind is None else fs._HelperProfile(fs._TOKEN,kind,[],['python3','fixed'],kind=='owner')
            for size,accepted in ((LEGACY,True),(LEGACY+1,False)):
                with self.subTest(kind=kind,size=size):
                    transport=Transport(profile);client=broker(transport);raw=frame(size)
                    if accepted:
                        self.assertEqual(client.call(json.loads(raw)),{'accepted':True})
                        self.assertEqual(transport.sent,[raw+b'\n'])
                    else:
                        with self.assertRaisesRegex(driver.GateError,'BROKER_INPUT_BUDGET'):client.call(json.loads(raw))
                        self.assertEqual(transport.sent,[])

    def test_profile_context_kind_and_fixed_contract_cannot_be_spoofed(self):
        for mutation in ('profile','context','pair','kind','command','mount','interactive','closed'):
            with self.subTest(mutation=mutation):
                transport=controlled_transport();profile=transport._full_profile
                if mutation=='profile':transport._full_profile=SimpleNamespace(_kind='controlled-owner',_controlled=profile._controlled)
                elif mutation=='context':profile._controlled=SimpleNamespace()
                elif mutation=='pair':transport.runner.ownership._controlled_pair=object()
                elif mutation=='kind':profile._kind='controlled-native'
                elif mutation=='command':profile._command=('python3','--caller')
                elif mutation=='mount':profile._mounts=()
                elif mutation=='interactive':profile._interactive=False
                else:profile._controlled._closed=True
                with self.assertRaises((driver.GateError,fs.FsRejected,fixture.ControlledRejected)):
                    broker(transport).call(json.loads(frame(LEGACY+1)))
                self.assertEqual(transport.sent,[])

    def test_extra_top_level_budget_keys_cannot_enlarge_request(self):
        transport=controlled_transport()
        with self.assertRaisesRegex(driver.GateError,'BROKER_INPUT_BUDGET'):
            broker(transport).call(json.loads(frame(LEGACY+1,input_budget=CREATED)))
        self.assertEqual(transport.sent,[])

    def test_response_buffer_stays_one_mib(self):
        transport=controlled_transport();transport.child.events=queue.Queue()
        transport.child.events.put((0,b'x'*LEGACY+b'\n'))
        with self.assertRaisesRegex(driver.GateError,'BROKER_OUTPUT_BUDGET'):broker(transport).call(json.loads(frame(LEGACY+1)))
        self.assertEqual(len(transport.sent),1)


class ReaderBudgetTests(unittest.TestCase):
    def assert_not_created(self,raw,**kwargs):
        code,backend,output,_=owner(raw,**kwargs)
        self.assertEqual(code,1)
        self.assertNotIn('created',[op for op,_ in backend.calls])
        self.assertEqual(backend.disconnected_calls,1)
        return backend,output

    def test_exact_created_body_plus_lf_reaches_protocol_without_truncation(self):
        raw=frame(CREATED);code,backend,output,_=owner(raw+b'\n')
        self.assertEqual(code,1,'EOF after created retains existing disconnect behavior')
        self.assertEqual(backend.calls[-1],('created',json.loads(raw)['value']))
        self.assertTrue(json.loads(output.splitlines()[-1])['ok'])
        self.assertEqual(backend.disconnected_calls,1)

    def test_over_limit_missing_lf_and_malformed_frames_are_rejected(self):
        for raw in (frame(CREATED+1)+b'\n',frame(CREATED),frame(LEGACY+1),
                    b'{'+b' '*LEGACY+b'\n'):
            with self.subTest(size=len(raw),lf=raw.endswith(b'\n')):self.assert_not_created(raw)

    def test_read_is_bounded_even_without_lf(self):
        raw=frame(CREATED+4096);_,backend,_,consumed=owner(raw,prefix=False)
        self.assertEqual(consumed,CREATED+2)
        self.assertEqual(backend.calls,[])

    def test_noncreated_large_frames_cannot_touch_backend(self):
        for op in ('setup','pin','files','check','birth','release'):
            with self.subTest(op=op):
                backend,_=self.assert_not_created(frame(LEGACY+1,op)+b'\n')
                self.assertEqual([op for op,_ in backend.calls],['setup','pin','files'])

    def test_default_reader_keeps_wire_limit_including_lf(self):
        for size,accepted in ((LEGACY-1,True),(LEGACY,False),(LEGACY+1,False)):
            with self.subTest(body_size=size):
                _,backend,_,_=owner(frame(size)+b'\n',controlled=False)
                self.assertEqual('created' in [op for op,_ in backend.calls],accepted)

    def test_duplicate_extra_keys_wrong_order_nonce_or_seq_do_not_reach_backend(self):
        raw=frame(LEGACY+1)
        variants=(raw[:-1]+b',"input_budget":2097152}\n',
                  b'{"op":"created",'+raw[1:]+b'\n',
                  raw.replace(b'"seq":3',b'"seq":2')+b'\n',
                  raw.replace(SESSION.encode(),b'22222222-2222-4222-8222-222222222222')+b'\n')
        for candidate in variants:
            with self.subTest(size=len(candidate)):self.assert_not_created(candidate)
        backend,_=self.assert_not_created(raw+b'\n',prefix=False)
        self.assertEqual(backend.calls,[])

    def test_protocol_response_cap_does_not_grow_with_request_budget(self):
        backend=Backend();protocol=fs._Protocol(fs._TOKEN,SESSION,backend)
        for seq,op in enumerate(('setup','pin','files','created')):
            self.assertTrue(protocol.handle(dict(session=SESSION,seq=seq,op=op,value={}))['ok'])
        response=backend.perform('birth',{})
        response['birth_path']='/'+('x'*LEGACY)
        backend.perform=lambda op,value:response
        reply=protocol.handle(dict(session=SESSION,seq=4,op='birth',value={}))
        self.assertFalse(reply['ok'])
        self.assertNotIn('value',reply)


@unittest.skipUnless(os.environ.get('P0C4_CREATED_FRAME_PATH'),'private reconstructed frame requires explicit local path')
class ReconstructedFrameTests(unittest.TestCase):
    def test_sealed_actual_frame_is_complete_at_broker_and_controlled_reader_only(self):
        raw=Path(os.environ['P0C4_CREATED_FRAME_PATH']).read_bytes()
        self.assertEqual(len(raw),1061422)
        self.assertEqual(hashlib.sha256(raw).hexdigest(),'346212a3ac3e4760443d588c35b6eaebb8f7a8fbb614ac84dccbacb96ebc1ef0')
        value=json.loads(raw);self.assertEqual(driver.canonical(value),raw)
        transport=controlled_transport();self.assertEqual(broker(transport).call(value),{'accepted':True})
        self.assertEqual(transport.sent,[raw+b'\n'])
        _,backend,output,_=owner(raw+b'\n',session=value['session'])
        self.assertEqual(backend.calls[-1],('created',value['value']))
        self.assertTrue(json.loads(output.splitlines()[-1])['ok'])
        with self.assertRaisesRegex(driver.GateError,'BROKER_INPUT_BUDGET'):broker(Transport()).call(value)
        _,default_backend,_,_=owner(raw+b'\n',controlled=False,session=value['session'])
        self.assertEqual([op for op,_ in default_backend.calls],['setup','pin','files'])


if __name__=='__main__':unittest.main()
