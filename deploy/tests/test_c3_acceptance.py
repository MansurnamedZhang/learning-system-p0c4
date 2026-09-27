"""Pure acceptance-policy checks; no Docker, network or credentials."""
import importlib.util
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("c3", ROOT / "deploy/c3_acceptance.py")
c3 = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(c3)

def worker_inspect():
    return {"Config":{"User":"65532:65532","Env":["SNAPSHOT_ROOT=/snapshots"],"Cmd":["worker"]},
                "HostConfig":{"Privileged":False,"Devices":[],"ReadonlyRootfs":True,"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges:true"],"NanoCpus":2000000000,"Memory":2147483648,"PidsLimit":128,"PortBindings":{}},
                "Mounts":[{"Destination":"/run/secrets/runtime_password","RW":False,"Type":"bind"},{"Destination":"/assets","RW":False,"Type":"volume","Name":"fresh_test_assets"},{"Destination":"/usr/local/bin/learning-worker","RW":False,"Type":"bind"},{"Destination":"/snapshots","RW":True,"Type":"volume","Name":"fresh_c3_snapshots"}],
                "NetworkSettings":{"Networks":{"fresh_test":{}},"Ports":{}}}

class AcceptancePolicy(unittest.TestCase):
    def test_explicit_subnet_rejects_invalid_and_overlapping_ranges(self):
        docker = [{'IPAM':{'Config':[{'Subnet':'172.20.0.0/16'}, {'Subnet':'fd00::/64'}]}},
                  {'IPAM':{'Config':[{'Subnet':'10.251.202.0/23'}]}}]
        routes = [{'dst':'default'}, {'dst':'10.251.201.128/25'}, {'dst':'192.168.8.0/24'}]
        self.assertEqual(str(c3.check_test_subnet('10.251.200.0/24', docker, routes)), '10.251.200.0/24')
        for value in ['', '10.251.200.1/24', '10.251.200.0/25', '8.8.8.0/24',
                      '10.251.200.0/24,10.251.201.0/24', 'fd00::/24', '0.0.0.0/24',
                      '172.20.1.0/24', '10.251.202.0/24', '10.251.201.0/24']:
            with self.subTest(value=value), self.assertRaises(ValueError):
                c3.check_test_subnet(value, docker, routes)

    def test_merged_config_requires_internal_exact_explicit_subnet(self):
        import copy
        good = {'networks':{'test':{'name':'fresh_test','internal':True,
                                    'ipam':{'config':[{'subnet':'10.251.200.0/24'}]}}}}
        c3.check_test_network_config(good, 'fresh', '10.251.200.0/24')
        for change in [{'internal':False}, {'name':'other_test'}, {'ipam':{'config':[]}},
                       {'ipam':{'config':[{'subnet':'10.251.201.0/24'}]}},
                       {'ipam':{'config':[{'subnet':'10.251.200.0/24'}, {'subnet':'10.251.201.0/24'}]}}]:
            bad = copy.deepcopy(good)
            bad['networks']['test'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                c3.check_test_network_config(bad, 'fresh', '10.251.200.0/24')

    def test_rootless_or_remapped_docker_rejected(self):
        c3.check_docker_identity(['name=seccomp,profile=builtin','name=cgroupns'])
        for options in [['name=rootless'],['name=userns'],None]:
            with self.assertRaises(ValueError): c3.check_docker_identity(options)

    def test_pg_only_mounts_its_own_three_copies(self):
        import copy
        config = {'services':{'pg':{'secrets':[{'source':'pg_'+name,'target':'/run/secrets/'+name} for name in c3.SECRET_NAMES]}},'secrets':{'pg_'+name:{'file':str(ROOT / '.runtime/secrets/pg' / name)} for name in c3.SECRET_NAMES}}
        try:
            c3.check_pg_config(config,ROOT)
        except ValueError as error:
            self.fail(f"absolute PG secret targets should be accepted: {error}")
        relative = copy.deepcopy(config)
        relative['services']['pg']['secrets'][0]['target'] = c3.SECRET_NAMES[0]
        with self.assertRaises(ValueError): c3.check_pg_config(relative,ROOT)
        bad = copy.deepcopy(config)
        bad['services']['pg']['secrets'][0]['source'] = 'postgres_password'
        with self.assertRaises(ValueError): c3.check_pg_config(bad,ROOT)
        good = {'Mounts':[{'Destination':'/run/secrets/'+name,'Source':str(ROOT / '.runtime/secrets/pg' / name),'Type':'bind','RW':False} for name in c3.SECRET_NAMES]}
        c3.check_pg_mounts(good,ROOT)
        good['Mounts'][0]['Source'] = str(ROOT / '.runtime/secrets/postgres_password')
        with self.assertRaises(ValueError): c3.check_pg_mounts(good,ROOT)

    def test_evidence_tar_redacted_before_write_and_rejects_escape(self):
        import io, tarfile, tempfile
        secret = b'a'*64
        def archive(name, body):
            buffer = io.BytesIO()
            with tarfile.open(fileobj=buffer,mode='w') as output:
                item = tarfile.TarInfo(name); item.size=len(body)
                output.addfile(item,io.BytesIO(body))
            return buffer.getvalue()
        with tempfile.TemporaryDirectory() as temp:
            directory = pathlib.Path(temp)
            c3.save_evidence_tar(archive('evidence/test.log', b'dsn='+secret),directory,[secret])
            self.assertEqual((directory/'test.log').read_bytes(), b'dsn=[REDACTED]')
            for path in ['evidence/../outside','/outside']:
                with self.assertRaises(ValueError): c3.save_evidence_tar(archive(path,b'bad'),directory,[secret])

    def test_secret_metadata_and_pair_values_fail_closed(self):
        import stat
        from types import SimpleNamespace
        good = dict(st_mode=stat.S_IFREG | 0o600, st_uid=65532, st_gid=65532, st_nlink=1)
        c3.check_secret_metadata(SimpleNamespace(**good), (65532,65532))
        for change in [dict(st_mode=stat.S_IFREG | 0o644), dict(st_uid=1000), dict(st_gid=1000), dict(st_mode=stat.S_IFLNK | 0o600), dict(st_nlink=2)]:
            with self.assertRaises(ValueError): c3.check_secret_metadata(SimpleNamespace(**(good | change)), (65532,65532))
        c3.check_secret_pair(b'a'*64, b'a'*64)
        for value in [b'b'*64, b'a'*63, b'a'*64+b'\n']:
            with self.assertRaises(ValueError): c3.check_secret_pair(b'a'*64, value)

    def test_redaction_covers_config_dsn_and_timeout_bytes(self):
        secrets = [bytes([x])*64 for x in b'abcdef']
        body = b'config: ' + secrets[0] + b' postgres://user:' + secrets[1] + b'@pg/db\n' + b' '.join(secrets[2:])
        clean = c3.redact(body, secrets)
        for secret in secrets: self.assertNotIn(secret, clean)
        self.assertIn(b'postgres://user:[REDACTED]@pg/db', clean)

    def test_image_manifest_rejects_old_missing_and_extra_source(self):
        expected = {'/app/crates/a.rs':'a'*64, '/run-tests.sh':'b'*64}
        c3.check_image_sources(expected, 'a'*64+'  /app/crates/a.rs\n'+'b'*64+'  /run-tests.sh\n')
        for actual in ['c'*64+'  /app/crates/a.rs\n'+'b'*64+'  /run-tests.sh\n', 'a'*64+'  /app/crates/a.rs\n', 'a'*64+'  /app/crates/a.rs\n'+'b'*64+'  /run-tests.sh\n'+'c'*64+'  /extra\n']:
            with self.assertRaises(ValueError): c3.check_image_sources(expected, actual)

    def test_all_copy_inputs_are_in_docker_context(self):
        mapping = c3.image_source_manifest(ROOT)
        for path in ['/app/deploy/c3-env.sh','/app/deploy/c3-databases.json','/c3-manage.sh']:
            self.assertIn(path, mapping)
        self.assertFalse(any('/target/' in path for path in mapping))

    def test_startup_failure_cleanup_continues_after_inspect_and_log_errors(self):
        calls = []
        def command(label, argv, allowed=(0,)):
            calls.append(argv)
            if argv[1] == 'ps': return 'broken\nworker\nforeign\npg\n'
            if argv[1] == 'inspect':
                if argv[-1] == 'broken': raise RuntimeError('inspect failure')
                import json
                return json.dumps([{'Config':{'Labels':{'com.docker.compose.project':'other' if argv[-1]=='foreign' else 'fresh'}},'State':{'Running':False}}])
            if argv[1] == 'logs' and argv[-1] == 'worker': raise RuntimeError('logs failure')
            return ''
        errors = c3.cleanup_owned('fresh', command, ['broken','worker','pg'])
        self.assertTrue(errors)
        self.assertIn(['docker','stop','pg'],calls)
        self.assertIn(['docker','stop','worker'],calls)
        self.assertNotIn(['docker','stop','foreign'],calls)
        self.assertNotIn(['docker','stop','broken'],calls)

    def test_driver_startup_failure_records_original_and_stops_created_pg(self):
        import json, subprocess, tempfile
        from types import SimpleNamespace
        from unittest.mock import patch
        project = 'learning-system-p0c3-task7-unit123'
        calls = []
        def execute(argv, **kwargs):
            calls.append(argv)
            output, code = b'', 0
            if argv[0] == 'ip': output = b'[]'
            elif argv[1] == 'context': output = b'"unix:///var/run/docker.sock"'
            elif argv[1] == 'info': output = b'[]'
            elif argv[1] == 'ps': output = (project+'-pg-1').encode() if any('up' in x for x in calls) else b''
            elif 'id -u postgres; id -g postgres' in argv: output = b'999\n999\n'
            elif 'config' in argv: output = json.dumps({'potential_config_value':'c'*64, 'networks':{'test':{'name':project+'_test','internal':True,'ipam':{'config':[{'subnet':'10.251.200.0/24'}]}}}}).encode()
            elif 'up' in argv: code, output = 1, b'healthcheck failed'
            elif argv[1] == 'logs': code, output = 1, b'postgres://user:' + b'c'*64 + b'@pg/db'
            elif argv[1] == 'inspect': output = json.dumps([{'Config':{'Labels':{'com.docker.compose.project':project}},'State':{'Running':False}}]).encode()
            return subprocess.CompletedProcess(argv, code, output, b'')
        with tempfile.TemporaryDirectory() as temp:
            directory = pathlib.Path(temp)
            binary = directory/'worker'; binary.write_bytes(b'fixture')
            output = directory/'evidence'
            env = {'C3_PROJECT':project,'C3_IMAGE':'sha256:'+'a'*64,'C3_RUNTIME_IMAGE':'sha256:'+'b'*64,'C3_WORKER_BIN':str(binary),'C3_TEST_SUBNET':'10.251.200.0/24'}
            fake_os = SimpleNamespace(name='posix', environ=env)
            secrets = [b'c'*64]
            metadata = {f'.runtime/secrets/pg/{name}':None for name in c3.SECRET_NAMES}
            with patch.dict('sys.modules', {'os':fake_os}), patch.object(c3,'load_secrets',return_value=(secrets,metadata)), patch.object(c3,'source_hashes',return_value={}), patch.object(c3,'check_secret_metadata'), patch.object(c3,'image_source_manifest',return_value={}), patch.object(c3,'check_image_sources'), patch.object(c3,'check_pg_config'), patch('subprocess.run', side_effect=execute):
                with self.assertRaisesRegex(RuntimeError, 'postgres-start exit 1'):
                    c3.run_acceptance(ROOT, output)
            self.assertIn(['docker','stop',project+'-pg-1'],calls)
            result = json.loads((output/'result.json').read_text())
            self.assertEqual(result['status'],'FAILED')
            self.assertIn('postgres-start exit 1',result['primary_failure']['message'])
            self.assertTrue(result['cleanup_errors'])
            for file in output.rglob('*'):
                if file.is_file(): self.assertNotIn(secrets[0],file.read_bytes(),str(file))

    def test_inventory_has_distinct_databases_and_all_frozen_paths(self):
        inventory = c3.inventory(ROOT)
        self.assertEqual(len(set(inventory.values())), len(inventory))
        self.assertEqual(set(inventory), {"TEST", "TEST_UPGRADE", "TEST_B1_UPGRADE", "TEST_B2_UPGRADE", "TEST_B3_SCHEMA_UPGRADE", "TEST_C3_UPGRADE", "TEST_IMPORT_A", "TEST_IMPORT_B", "TEST_IMPORT_UPGRADE", "C3_SOURCE", "C3_TARGET"})
        init = (ROOT / "deploy/initdb.sh").read_text()
        env = (ROOT / "deploy/c3-env.sh").read_text()
        for prefix, db in inventory.items():
            self.assertIn(f"CREATE DATABASE {db} OWNER learning_admin;", init)
            self.assertIn(f"{prefix} {db}", env)
        bootstrap = (ROOT / "deploy/bootstrap-fixtures.sh").read_text()
        for kind in ["p0a", "b1", "b2", "b3-schema"]:
            self.assertIn(kind, bootstrap)

    def test_security_check_rejects_admin_source_socket_and_ephemeral_snapshot(self):
        good = worker_inspect()
        c3.check_worker(good, "fresh_test")
        import copy
        for destination in ["/run/secrets/admin_password", "/app", "/var/run/docker.sock"]:
            bad = copy.deepcopy(good)
            bad["Mounts"].append({"Destination":destination,"RW":False,"Type":"bind"})
            with self.assertRaises(ValueError): c3.check_worker(bad,"fresh_test")
        bad = copy.deepcopy(good)
        bad["Mounts"][-1]["Type"] = "tmpfs"
        with self.assertRaises(ValueError): c3.check_worker(bad,"fresh_test")

    def test_worker_rejects_privileged_override_and_foreign_volume(self):
        good = worker_inspect()
        good["HostConfig"]["Privileged"] = True
        with self.assertRaises(ValueError): c3.check_worker(good,"fresh_test")
        good["HostConfig"]["Privileged"] = False
        good["Mounts"][-1]["Name"] = "old_project_snapshots"
        with self.assertRaises(ValueError): c3.check_worker(good,"fresh_test")

    def test_originals_are_real_png_and_pdf(self):
        import struct, zlib
        directory = ROOT / "crates/learning-worker/examples/c3"
        png = (directory / "attention.png").read_bytes()
        self.assertEqual(png[:8], b"\x89PNG\r\n\x1a\n")
        offset = 8
        kinds = []
        while offset < len(png):
            size = struct.unpack(">I", png[offset:offset+4])[0]
            kind = png[offset+4:offset+8]
            body = png[offset+8:offset+8+size]
            crc = struct.unpack(">I", png[offset+8+size:offset+12+size])[0]
            self.assertEqual(crc, zlib.crc32(kind+body), f"invalid PNG {kind} CRC")
            kinds.append(kind)
            if kind == b"IDAT": self.assertTrue(zlib.decompress(body))
            offset += 12 + size
        self.assertEqual(kinds, [b"IHDR", b"IDAT", b"IEND"])
        pdf = (directory / "attention.pdf").read_bytes()
        start = int(pdf.split(b"startxref\n")[1].splitlines()[0])
        self.assertTrue(pdf[start:].startswith(b"xref\n0 5\n"))
        for i, entry in enumerate(pdf[start:].splitlines()[3:7], 1):
            self.assertTrue(pdf[int(entry[:10]):].startswith(f"{i} 0 obj".encode()))

    def test_failure_groups_require_each_actual_case(self):
        output = "\n".join(f"test {case} ... ok" for cases in c3.FAILURES.values() for case in cases)
        self.assertEqual(set(c3.failure_evidence(output)), set(c3.FAILURES))
        with self.assertRaises(ValueError): c3.failure_evidence(output.replace("interrupted_input_never_creates_ready_package ... ok", "interrupted_input_never_creates_ready_package ... FAILED"))
        with self.assertRaises(ValueError): c3.failure_evidence("compilation succeeded")

    def test_project_validation_is_fail_closed(self):
        for name in ["", "prod", "learning-system-p0c3-", "../old", "learning-system-p0c3-X"]:
            with self.assertRaises(ValueError): c3.validate_project(name)
        c3.validate_project("learning-system-p0c3-task7-abc123")

if __name__ == "__main__": unittest.main()
