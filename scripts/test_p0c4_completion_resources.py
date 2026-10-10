import struct
import unittest
from scripts.p0c4_completion import resources

def message(kind,payload=b'',seq=7,pid=9,flags=2):
    size=16+len(payload);return struct.pack('=IHHII',size,kind,flags,seq,pid)+payload+b'\0'*((-size)%4)
def route(prefix=24,destination=b'\x0a\xfe\x14\0'):
    payload=struct.pack('=BBBBBBBBI',2,prefix,0,0,254,2,0,1,0)
    return message(24,payload+struct.pack('=HH',8,1)+destination)

class RouteTests(unittest.TestCase):
    def test_kernel_route_and_done(self):
        parser=resources.RouteParser(7,9);parser.feed(route()+message(3,struct.pack('=i',0)))
        self.assertEqual(parser.finish(),[{'dst':'10.254.20.0/24'}])

    def test_truncated_malformed_foreign_error_and_oversized_are_rejected(self):
        for raw in (b'\0',message(2,struct.pack('=i',-1)),message(24,b'\0'),route()[:-1],message(3,b'',seq=8),message(99),message(3,b'',flags=0x10)):
            with self.subTest(raw=raw),self.assertRaises(resources.ResourceError):resources.RouteParser(7,9).feed(raw)
        parser=resources.RouteParser(7,9)
        with self.assertRaises(resources.ResourceError):parser.feed(b'x'*65537)

    def test_no_partial_routes_without_done(self):
        parser=resources.RouteParser(7,9);parser.feed(route())
        with self.assertRaises(resources.ResourceError):parser.finish()

    def test_missing_and_wrong_fixed_host_namespace_reference(self):
        from unittest.mock import patch
        from types import SimpleNamespace
        with patch.object(resources.os,'open',side_effect=FileNotFoundError),self.assertRaisesRegex(resources.ResourceError,'HOST_NETNS_REFERENCE'):resources.require_host_namespace()
        values=[SimpleNamespace(st_dev=1,st_ino=2,st_uid=0),SimpleNamespace(st_dev=1,st_ino=3,st_uid=0)]
        with patch.object(resources.os,'open',side_effect=[8,9]),patch.object(resources.os,'fstat',side_effect=values),patch.object(resources.os,'close'),self.assertRaisesRegex(resources.ResourceError,'HOST_NETNS_MISMATCH'):resources.require_host_namespace()

    def test_native_overlap_still_uses_existing_resource_admission(self):
        from scripts import p0c4_source_admission_gate_acceptance as helper
        class Runner:
            def docker(self,*args):return b''
        ident=dict(project='fresh',pg_name='fresh-pg',network='fresh-net',volume='fresh-volume')
        with self.assertRaisesRegex(helper.GateError,'SUBNET_OCCUPIED'):
            helper.fresh_resources(Runner(),[ident],['10.254.20.0/24'],'',route_observer=lambda:[{'dst':'10.254.20.0/24'}])
