"""Contract producer tests; synthetic grammar samples are NOT trusted fixtures."""
import importlib.util
import pathlib
import unittest
import uuid
import copy


class ProducerTests(unittest.TestCase):
    def producer(self):
        path = pathlib.Path(__file__).with_name('p0c4_full_schema_contract.py')
        self.assertTrue(path.exists(), 'fixed full-schema producer is missing')
        spec = importlib.util.spec_from_file_location('full_schema_producer', path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_restrict_normalization_is_exact_and_position_bound(self):
        p = self.producer()
        raw = b'--\n-- PostgreSQL database dump\n--\n\n\\restrict abc123\n\nSELECT 1;\n\n\\unrestrict abc123\n\n'
        expected = raw.replace(b'abc123', b'{{RESTRICT_KEY}}')
        self.assertEqual(p.normalize_restrict(raw), expected)
        for bad in (raw.replace(b'unrestrict abc123', b'unrestrict other'), raw + b'COMMIT;\n', raw.replace(b'SELECT 1;', b'\\restrict abc123'), raw.replace(b'abc123', b'x;bad'), raw.replace(b'\n', b'\r\n')):
            with self.subTest(bad=bad), self.assertRaises(p.ContractError):
                p.normalize_restrict(bad)

    def test_migration_identity_covers_every_version(self):
        p = self.producer()
        rows = p.migration_identity(pathlib.Path(__file__).resolve().parents[1])
        self.assertEqual([r['version'] for r in rows], list(range(1, 16)))
        self.assertEqual(rows[0]['checksum_hex'], 'ec6e4c80c17203e3fb9431f850a2a8aa57411b1d945c4f3f67a3e2a3e0425021bf53b237586e74e74438d8b4319e005e')
        p.require_migrations(rows, rows)
        for bad in (rows[1:], [dict(r, checksum_hex='0'*96) if r['version']==2 else r for r in rows]):
            with self.assertRaises(p.ContractError): p.require_migrations(bad, rows)

    def test_copy_template_keeps_exact_headers_and_data_bytes(self):
        p = self.producer()
        rows = [{'table':'edge','sql_name':'edge','columns':[{'name':'id','sql_name':'id'},{'name':'text','sql_name':'text'}]}]
        payload = '1\t中文\\n\\\\\\t\\N; DROP TABLE x;\\n\\\\connect bad\n'.encode()
        raw = b'prefix\nCOPY public.edge (id, text) FROM stdin;\n'+payload+b'\\.\n\nsuffix\n'
        template, data = p.split_copy(raw, rows)
        self.assertEqual(template, b'prefix\nCOPY public.edge (id, text) FROM stdin;\n{{COPY:edge}}\\.\n\nsuffix\n')
        self.assertEqual(data, {'edge':payload})
        for bad in (raw.replace(b'(id, text)', b'(text, id)'),raw.replace(b'FROM stdin;',b'FROM PROGRAM \'evil\';'),raw.replace(b'\\.\n',b'\\. extra\n'),raw+raw):
            with self.assertRaises(p.ContractError):p.split_copy(bad, rows)

    def test_actual_failed_capture_nine_native_headers_remain_exact(self):
        p=self.producer()
        # Actual observed failed087 decode SHA08fe6a4d...: diagnostic examples,
        # NOT a complete schema golden or successful capture fixture.
        headers=[
            'COPY public.composition_occurrence (composition_revision_id, space_id, composition_id, occurrence_id, "position", block_space_id, block_id, block_revision_id, child_space_id, child_composition_id, child_revision_id) FROM stdin;\n',
            'COPY public.lineage_input (operation_id, "position", space_id, block_id, revision_id) FROM stdin;\n',
            'COPY public.lineage_output (operation_id, "position", space_id, block_id, revision_id) FROM stdin;\n',
            'COPY public."overlay" (id, space_id, owner_id, root_composition_id, head_revision_id) FROM stdin;\n',
            'COPY public.overlay_placement (overlay_id, overlay_revision_id, placement_id, group_id, "position", block_space_id, block_id, block_revision_id) FROM stdin;\n',
            'COPY public.reading_epistemic_selection (view_id, view_revision_id, "position", stream_id, review_id) FROM stdin;\n',
            'COPY public.reading_receipt_block (actor_id, request_id, "position", block_space_id, block_id, revision_id) FROM stdin;\n',
            'COPY public.reading_relation_selection (view_id, view_revision_id, "position", relation_id, relation_revision_id, review_id) FROM stdin;\n',
            'COPY public.reference_dependency (source_kind, source_object_id, source_revision_id, "position", role, target_kind, target_object_id, target_revision_id) FROM stdin;\n',
        ]
        tables=[]
        for header in headers:
            table,columns=header.removeprefix('COPY public.').removesuffix(') FROM stdin;\n').split(' (')
            tables.append(dict(table=table.strip('"'),sql_name=table,columns=[dict(name=column.strip('"'),sql_name=column) for column in columns.split(', ')]))
        raw=b''.join(h.encode()+b'\\.\n\n' for h in headers)
        try:template,data=p.split_copy(raw,tables)
        except p.ContractError as error:self.fail('actual native header rejected: '+str(error))
        self.assertEqual(set(data),{t['table'] for t in tables})
        self.assertTrue(all(value==b'' for value in data.values()))
        for header in headers:self.assertEqual(template.count(header.encode()),1)
        for bad in [raw.replace(b'"position"',b'position',1),raw.replace(b'public."overlay"',b'public.overlay'),raw.replace(b'(id, ',b'("id", '),raw.replace(b'composition_revision_id, space_id',b'space_id, composition_revision_id',1),raw.replace(b'"position"',b'"unknown"',1),raw.replace(b' FROM stdin;',b' FROM PROGRAM \'bad\';',1),raw+headers[0].encode()+b'\\.\n',raw+b'COPY public.unknown (id) FROM stdin;\n\\.\n']:
            with self.assertRaises(p.ContractError):p.split_copy(bad,tables)
        for sql_name in ['"overlay"; COMMIT;', 'other', '"OVERLAY"', '"over""lay"', 'overlay\n\\connect other']:
            changed=copy.deepcopy(tables);changed[3]['sql_name']=sql_name
            with self.assertRaises(p.ContractError):p.split_copy(raw,changed)
        for changed in [tables+[tables[0]], [dict(tables[0],columns=tables[0]['columns']+[tables[0]['columns'][0]])]+tables[1:]]:
            with self.assertRaises(p.ContractError):p.split_copy(raw,changed)

    def test_toc_normalizes_only_numeric_identity_and_archive_metadata(self):
        p=self.producer()
        header = b';\n; Archive created at 2026-10-07 01:02:03 UTC\n;     dbname: learning_backup_c4_task3_01234567-1234-4234-8234-0123456789ab\n;     TOC Entries: 5\n;     Compression: gzip\n;     Dump Version: 1.16-0\n;     Format: CUSTOM\n;     Integer: 4 bytes\n;     Offset: 8 bytes\n;     Dumped from database version: 18.6 (Debian 18.6-1.pgdg12+2)\n;     Dumped by pg_dump version: 18.6 (Debian 18.6-1.pgdg12+2)\n;\n;\n; Selected TOC Entries:\n;\n'
        raw=header+b'1; 1259 555 TABLE public edge learning_admin\n'
        one=p.normalize_toc(raw)
        self.assertEqual(one,p.normalize_toc(raw.replace(b'1; 1259 555',b'9; 1259 777')))
        self.assertNotEqual(one,p.normalize_toc(raw.replace(b'TABLE public edge',b'TABLE public other')))
        self.assertNotEqual(one,p.normalize_toc(raw.replace(b'1259 555',b'9999 555')))
        for bad in (raw.replace(b'Format: CUSTOM',b'Format: TAR'),raw+b'\\connect other\n',raw.replace(b'1; 1259 555',b'01; 1259 555')):
            with self.assertRaises(p.ContractError):p.normalize_toc(bad)

    def test_fixed_commands_reject_arbitrary_targets_and_options(self):
        p=self.producer()
        db='learning_backup_c4_task3_01234567-1234-4234-8234-0123456789ab'
        command=p.fixed_command('a'*64, db, 'dump')
        self.assertIn('--format=custom',command)
        self.assertNotIn('--no-acl',command)
        for container,database,operation in [('pg',db,'dump'),('a'*64,'postgres','dump'),('a'*64,db,'evil')]:
            with self.assertRaises(p.ContractError):p.fixed_command(container,database,operation)

class SchemaLaneTests(unittest.TestCase):
    def lane(self):
        path=pathlib.Path(__file__).with_name('p0c4_completion')/'schema.py'
        self.assertTrue(path.exists(),'fixed schema lane contract missing')
        spec=importlib.util.spec_from_file_location('p0c4_completion.schema',path)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        return module

    def plan(self,case='full-schema-contract'):
        return dict(format_version=1,capability='c4_completion_schema_plan_v1',batch_id=str(uuid.uuid4()),case_id=str(uuid.uuid4()),cache_scope=str(uuid.uuid4()),case_name=case,source=dict(archive=dict(path='/private/source/source.zip',size=1,sha256='a'*64),manifest=dict(path='/private/source/manifest.json',size=1,sha256='b'*64),application_commit='c'*40,application_build_sha256='a'*64),images=dict(builder='sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b',postgres='sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d'),subnet='10.249.90.0/24',evidence_root='/private/schema-case',budget_profile='full_schema_task4_v1')

    def test_schema_plan_accepts_only_three_fixed_content_cases(self):
        lane=self.lane()
        for case in ['full-schema-contract','full-schema-altered','full-copy-edge-data']:
            lane.validate_plan(self.plan(case),case)
        for plan,case in [(self.plan('registry-reopen'),'registry-reopen'),(dict(self.plan(),command='psql'),'full-schema-contract'),(dict(self.plan(),capability='c4_completion_case_plan_v1'),'full-schema-contract')]:
            with self.assertRaises(lane.SchemaError):lane.validate_plan(plan,case)

    def receipt(self,lane):
        plan=self.plan();ref=dict(path='capture/index.json',size=10,sha256='d'*64)
        receipt=dict(format_version=1,capability='c4_schema_content_receipt_v1',batch_id=plan['batch_id'],case_id=plan['case_id'],case_name=plan['case_name'],plan_sha256='e'*64,source_archive_sha256='a'*64,source_manifest_sha256='b'*64,application_build_sha256='a'*64,application_commit='c'*40,postgres_image=plan['images']['postgres'],container_id='1'*64,client_sha256='eba5f8a8361918f873e273b1107dd727c19fc4269e8f98854ed5589555d668c6',migration_fingerprint='2'*64,binary_sha256='3'*64,test_name='full_restore::live_tests::real_full_schema_contract',test_exit=0,passed=1,failed=0,ignored=0,capture_index=ref,raw_index=dict(ref,path='raw/index.json'),resource_receipt=dict(ref,path='resources.json'),status='REAL_PG18_SCHEMA_CONTENT_NOT_RESTORE',reason_code='NONE')
        return plan,receipt

    def test_content_pass_requires_real_body_and_all_evidence_refs(self):
        lane=self.lane();plan,receipt=self.receipt(lane)
        lane.validate_receipt(receipt,plan)
        for key,value in [('passed',0),('ignored',1),('test_exit',101),('capture_index',None),('reason_code','BODY_FAILED'),('binary_sha256','0'*64),('test_name','source::lifecycle')]:
            changed=copy.deepcopy(receipt);changed[key]=value
            with self.subTest(key=key),self.assertRaises(lane.SchemaError):lane.validate_receipt(changed,plan)

    def test_failed_receipts_are_not_promoted_by_readback(self):
        lane=self.lane();plan,receipt=self.receipt(lane)
        receipt.update(status='FAILED_UNUSABLE',reason_code='BODY_FAILED',test_exit=101,passed=0,failed=1)
        self.assertEqual(lane.validate_receipt(receipt,plan)['status'],'FAILED_UNUSABLE')
        for reason in ['full DSN could leak here','NONE','stack trace']:
            changed=dict(receipt,reason_code=reason)
            with self.assertRaises(lane.SchemaError):lane.validate_receipt(changed,plan)

    def test_fixed_body_parser_rejects_wrong_skipped_or_zero_bodies(self):
        lane=self.lane();name='full_restore::live_tests::real_full_schema_contract'
        self.assertTrue(hasattr(lane,'parse_body'),'schema body parser missing')
        raw=('running 1 test\ntest '+name+' ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 20 filtered out; finished in 0.10s\n').encode()
        self.assertEqual(lane.parse_body(0,raw,name),dict(exit_code=0,passed=1,failed=0,ignored=0))
        for code,changed in [(101,raw),(0,raw.replace(name.encode(),b'other::test')),(0,raw.replace(b'1 passed',b'0 passed')),(0,raw.replace(b'0 ignored',b'1 ignored')),(True,raw)]:
            with self.assertRaises(lane.SchemaError):lane.parse_body(code,changed,name)
        failed=raw.replace(b' ... ok',b' ... FAILED').replace(b'ok. 1 passed; 0 failed',b'FAILED. 0 passed; 1 failed')
        self.assertEqual(lane.parse_body(101,failed,name),dict(exit_code=101,passed=0,failed=1,ignored=0))

    def test_original_process_join_requires_same_pair_and_successful_wait(self):
        lane=self.lane();out=b'body';err=b''
        self.assertTrue(hasattr(lane,'join_process'),'original process join missing')
        row=dict(exit_code=0,stdout_sha256=lane.envelope.digest(out),stderr_sha256=lane.envelope.digest(err))
        process=dict(exit_code=0,reason=None,stdout_bytes=4,stderr_bytes=0)
        originals=[(out,err,process)]
        self.assertEqual(lane.join_process(originals,row),out)
        for changed in [dict(process,exit_code=1),dict(process,reason='timeout'),dict(process,stdout_bytes=3),dict(process,exit_code=True)]:
            with self.assertRaises(lane.SchemaError):lane.join_process([(out,err,changed)],row)
        with self.assertRaises(lane.SchemaError):lane.join_process([(out,b'error',process)],row)

    def test_cli_routes_schema_without_source_issuer(self):
        from unittest.mock import patch
        import p0c4_completion_acceptance as adapter
        lane=self.lane()
        self.assertTrue(set(lane.CASE_TESTS)<=set(adapter.ALL_CASE_TESTS),'schema dispatcher absent')
        with patch.object(adapter.schema_lane,'run',return_value={'status':lane.PASS}) as dispatched:
            self.assertEqual(adapter.run(pathlib.Path('/private/plan'),'full-schema-contract'),{'status':lane.PASS})
            dispatched.assert_called_once_with(pathlib.Path('/private/plan'),'full-schema-contract')

    def test_schema_edge_proof_joins_retained_copy_and_decoded_hash(self):
        lane=self.lane();payload=b'id\t\\N\ttext\\n\\t\\\\connect\n';sha=lane.envelope.digest(payload)
        proof=dict(classification=lane.PASS,source_copy_sha256=sha,unchanged_payload_bytes=len(payload),decoded_sha256='a'*64)
        raw={'full-copy-edge-data.json':lane.envelope.canonical(proof),'capture/edge-copy.bin':payload}
        rows={'decoded':dict(path='validation-uuid/full-decoded-uuid/decoded',sha256='a'*64)}
        lane._verify_case_output(raw.__getitem__,rows,{},self.plan('full-copy-edge-data'))
        for changed in [dict(proof,source_copy_sha256='0'*64),dict(proof,unchanged_payload_bytes=0),dict(proof,decoded_sha256='b'*64)]:
            with self.assertRaises(lane.SchemaError):lane._verify_case_output(dict(raw,**{'full-copy-edge-data.json':lane.envelope.canonical(changed)}).__getitem__,rows,{},self.plan('full-copy-edge-data'))

    def test_cleanup_cannot_pass_without_actual_transport_and_exact_stop(self):
        lane=self.lane();case=dict(stopped=True,execs_absent=True,retained_network_empty=True,cleanup_errors=[],retained_volumes_verified=True)
        result=dict(preflight_complete=True,resource_creation_unknown=False,registered_transports_reaped=True,helpers=[dict(removed=True)],cases=[case])
        self.assertTrue(lane.schema_cleanup_verified(result))
        for key in ('stopped','execs_absent','retained_network_empty','retained_volumes_verified'):
            self.assertFalse(lane.schema_cleanup_verified(dict(result,cases=[dict(case,**{key:False})])))
        self.assertFalse(lane.schema_cleanup_verified(dict(result,registered_transports_reaped=False)))

    def test_readback_keeps_original_distinct_setup_database_identity(self):
        lane=self.lane();_,_,driver,_=lane._modules();case=str(uuid.uuid4())
        identity=driver.completion_case_identity(case)
        lane.validate_identity(identity,case)
        lane.validate_identity(identity,case)  # readback never regenerates its UUID
        for changed in [dict(identity,other_database=identity['database']),dict(identity,other_database='postgres'),dict(identity,volume='foreign')]:
            with self.assertRaises(lane.SchemaError):lane.validate_identity(changed,case)

    def failed_resource_fixture(self,lane):
        # In-memory validator inputs only; never native operational evidence.
        h,_,d,_=lane._modules();plan,receipt=self.receipt(lane)
        ident=d.completion_case_identity(plan['case_id']);case=lane._batch(pathlib.PurePosixPath(plan['evidence_root']),plan)/plan['case_id']
        volumes={key:dict(Name=ident[key],Mountpoint='/volumes/'+ident[key],Driver='local',Options=None,Labels={'com.docker.compose.project':ident['project'],'knowweave.source-lifecycle.batch':plan['batch_id']}) for key in ('volume','source_volume','build_volume')}
        row=dict(identity=ident,test=lane.CASE_TESTS[plan['case_name']],evidence_directory=str(case),volumes=volumes,container_id='1'*64,network_id='2'*64,stopped=True,execs_absent=True,retained_network_empty=True,retained_volumes_verified=True,cleanup_errors=[])
        image=dict(Id=plan['images']['postgres'],RepoDigests=['postgres@'+plan['images']['postgres']],Config=dict(Env=[],Entrypoint=['docker-entrypoint.sh']))
        mounts=[dict(Type='volume',Source=volumes[key]['Mountpoint'],Destination=target,RW=rw) for key,target,rw in [('volume','/var/lib/postgresql',True),('source_volume',d.ROOT,True),('build_volume','/target',False)]]
        mounts += [dict(Type='bind',Source=str(case/'initdb.sh'),Destination='/docker-entrypoint-initdb.d/10-lifecycle.sh',RW=False),dict(Type='bind',Source=str(case/'schema'),Destination='/var/lib/knowweave-schema',RW=True)]
        mounts += [dict(Type='bind',Source=str(case/'secrets'/(role+'_password')),Destination='/run/secrets/'+role+'_password',RW=False) for role in ('postgres','admin')]
        env=dict(POSTGRES_USER='postgres',POSTGRES_DB='postgres',POSTGRES_PASSWORD_FILE='/run/secrets/postgres_password',POSTGRES_INITDB_ARGS='--auth-host=scram-sha-256 --auth-local=trust',C4_DATABASE=ident['database'],C4_OTHER_DATABASE=ident['other_database'])
        container=dict(Id=row['container_id'],Name='/'+ident['pg_name'],Image=image['Id'],Config=dict(Image=h.PG_IMAGE,Labels={'com.docker.compose.project':ident['project'],'com.docker.compose.service':'pg','com.docker.compose.project.working_dir':str(case),'com.docker.compose.project.config_files':str(case/'compose.json')},Cmd=['postgres','-c','max_prepared_transactions=16'],Entrypoint=image['Config']['Entrypoint'],Env=[k+'='+v for k,v in env.items()]),HostConfig=dict(Privileged=False,CapAdd=None,CapDrop=None,SecurityOpt=['no-new-privileges'],NanoCpus=2*10**9,Memory=4*1024**3,MemorySwap=4*1024**3,IpcMode='private',NetworkMode=ident['network']),Mounts=mounts,State=dict(Running=False,Pid=0),ExecIDs=[],NetworkSettings=dict(Ports={}))
        for mount in mounts:
            if mount['Type']=='bind':mount.update(Mode='rw' if mount['RW'] else 'ro',Propagation='rslave')
            else:mount.update(Name=next(value['Name'] for value in volumes.values() if value['Mountpoint']==mount['Source']),Mode='z',Propagation='')
        container['HostConfig']['Binds']=[m['Source']+':'+m['Destination']+':'+m['Mode'] for m in mounts if m['Type']=='bind']
        container['HostConfig']['Mounts']=[dict(Type='volume',Source=ident[key],Target=target,ReadOnly=ro,VolumeOptions=dict(NoCopy=True)) for key,target,ro in [('volume','/var/lib/postgresql',False),('source_volume',d.ROOT,False),('build_volume','/target',True)]]
        network=dict(Id=row['network_id'],Name=ident['network'],Internal=True,Driver='bridge',Containers={},IPAM=dict(Config=[dict(Subnet=plan['subnet'],Gateway='10.249.90.1')]),Labels={'com.docker.compose.project':ident['project']})
        return plan,receipt,row,[image,container,network,*volumes.values()]

    def check_failed_readback(self,lane,plan,receipt,row,objects,**changes):
        import json
        from contextlib import ExitStack
        from unittest.mock import patch
        h,b,d,p=lane._modules();root=pathlib.Path(plan['evidence_root']);batch=lane._batch(root,plan)
        archive=b'archive';plan=copy.deepcopy(plan);plan['source']['archive'].update(size=len(archive),sha256=p.sha(archive))
        manifest=dict(base_commit=plan['source']['application_commit']);manifest_raw=lane.envelope.canonical(manifest)
        result=dict(batch_id=plan['batch_id'],case_id=plan['case_id'],base_commit=plan['source']['application_commit'],archive_sha256=p.sha(archive),classification=lane.PASS,cases=[row],cleanup_verified=True,preflight_complete=True,resource_creation_unknown=False,registered_transports_reaped=True,helpers=[],status='FAILED_UNUSABLE',reason_code='BODY_FAILED');result.update(changes)
        # Changes are explicit test observations; neither fixture nor mocks can
        # replace the resource validator or cleanup consistency predicate.
        rawfiles={};indexfiles=[]
        for i,value in enumerate(objects,1):
            out=json.dumps([value],separators=(',',':')).encode();process=dict(exit_code=0,reason=None,stdout_bytes=len(out),stderr_bytes=0)
            for suffix,raw in [('stdout',out),('stderr',b''),('process.json',lane.envelope.canonical(process))]:
                name=f'{i:04d}.'+suffix;rawfiles[str(batch/'evidence/logs'/name)]=raw
                indexfiles.append(dict(path='logs/'+name,size=len(raw),sha256=p.sha(raw),dev=1,ino=len(indexfiles)+1))
        index=dict(format_version=1,capability='c4_source_raw_index_v1',files=indexfiles,scan_bytes=3*sum(x['size'] for x in indexfiles))
        indexraw=lane.envelope.canonical(index);indexref=dict(path=(batch/'evidence/raw-index.json').relative_to(root).as_posix(),size=len(indexraw),sha256=p.sha(indexraw))
        wrapper=lane.envelope.canonical(dict(format_version=1,capability='c4_schema_raw_index_v1',original_index=indexref))
        refs={'resources.json':lane.envelope.canonical(result),'raw/index.json':wrapper,indexref['path']:indexraw}
        rawfiles.update({plan['source']['archive']['path']:archive,plan['source']['manifest']['path']:manifest_raw})
        receipt=copy.deepcopy(receipt);receipt.update(status='FAILED_UNUSABLE',reason_code='BODY_FAILED',capture_index=None)
        before=copy.deepcopy(receipt)
        def logical_stream(root,plan,domain,name,rows,budget):
            # These three tests cover downstream failed-receipt semantics only.
            # No filesystem credit: the dedicated stream tests own real Linux
            # UID/FD/open/identity checks, including actual zero-byte opens.
            self.assertEqual(domain,'raw')
            self.assertRegex(name,r'[0-9]{4}\.(stdout|stderr)\Z')
            raw=rawfiles[str(batch/'evidence/logs'/name)]
            self.assertEqual(rows['logs/'+name]['size'],len(raw))
            self.assertEqual(rows['logs/'+name]['sha256'],p.sha(raw))
            return raw
        with ExitStack() as stack:
            stack.enter_context(patch.object(lane.schema_resources,'_SchemaDirectory'))
            stack.enter_context(patch.object(h,'verify_package',return_value=(manifest,{})))
            stack.enter_context(patch.object(b,'source_digest',return_value='unchanged'))
            stack.enter_context(patch.object(lane.evidence,'private_read',side_effect=lambda path,cap:rawfiles[str(path)]))
            stack.enter_context(patch.object(lane.evidence,'relative_ref',side_effect=lambda root,ref,cap:refs[ref['path']]))
            stack.enter_context(patch.object(d,'raw_log_records',return_value=indexfiles))
            stack.enter_context(patch.object(lane,'_read_stream',side_effect=logical_stream))
            for module,name in [(d,'pg_exec'),(d,'run_helper'),(p,'capture')]:stack.enter_context(patch.object(module,name,side_effect=AssertionError('readonly verifier executed a resource command')))
            self.assertIsNone(lane._verify_contents(root,receipt,plan))
        self.assertEqual(receipt,before)

    def test_failed_receipt_rejects_original_stop_network_and_volume_mismatch(self):
        lane=self.lane();plan,receipt,row,objects=self.failed_resource_fixture(lane)
        self.check_failed_readback(lane,plan,receipt,row,objects)
        for mismatch in ('stop','exec','network','volume'):
            changed=copy.deepcopy(objects)
            if mismatch=='stop':changed[1]['State'].update(Running=True,Pid=99)
            if mismatch=='exec':changed[1]['ExecIDs']=['active']
            if mismatch=='network':changed[2]['Containers']={'foreign':{}}
            if mismatch=='volume':changed[3]['Mountpoint']='/foreign'
            with self.subTest(mismatch=mismatch),self.assertRaises(lane.SchemaError):self.check_failed_readback(lane,plan,receipt,row,changed)

    def test_failed_receipt_rejects_claimed_cleanup_without_observations(self):
        lane=self.lane();plan,receipt,row,objects=self.failed_resource_fixture(lane)
        row.update(stopped=False,execs_absent=False,retained_network_empty=False,retained_volumes_verified=False)
        with self.assertRaises(lane.SchemaError):self.check_failed_readback(lane,plan,receipt,row,objects)

    def test_partial_creation_failure_joins_only_acquired_volume_and_stays_failed(self):
        lane=self.lane();plan,receipt,row,objects=self.failed_resource_fixture(lane)
        row.pop('container_id');row.pop('network_id');row['volumes']={'volume':row['volumes']['volume']}
        row.update(stopped=False,execs_absent=False,retained_network_empty=False,retained_volumes_verified=False)
        kwargs=dict(cleanup_verified=False,resource_creation_unknown=True)
        self.check_failed_readback(lane,plan,receipt,row,[objects[3]],**kwargs)
        for changed in [[],[dict(objects[3],Labels={})]]:
            with self.assertRaises(lane.SchemaError):self.check_failed_readback(lane,plan,receipt,row,changed,**kwargs)
        network_only=dict(row,network_id=objects[2]['Id'],retained_network_empty=True)
        self.check_failed_readback(lane,plan,receipt,network_only,[objects[3],objects[2]],**kwargs)
        with self.assertRaises(lane.SchemaError):self.check_failed_readback(lane,plan,receipt,network_only,[objects[3],dict(objects[2],Containers={'foreign':{}})],**kwargs)
        empty=dict(row,volumes={})
        self.check_failed_readback(lane,plan,receipt,empty,[],**kwargs)

    def schema_resources(self):
        path=pathlib.Path(__file__).with_name('p0c4_completion')/'schema_resources.py'
        self.assertTrue(path.exists(),'fixed schema PG adapter missing')
        spec=importlib.util.spec_from_file_location('p0c4_completion.schema_resources',path)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module

    def test_actual_bind_shape_schema_adapter_preserves_legacy_rejection(self):
        from unittest.mock import patch
        from types import SimpleNamespace
        import json
        module=self.schema_resources();lane=self.lane();plan,receipt,row,objects=self.failed_resource_fixture(lane)
        h,_,d,_=lane._modules();image,facts,network=objects[:3]
        case=pathlib.PurePosixPath(row['evidence_directory'].replace('\\','/'));row['evidence_directory']=str(case)
        for m in facts['Mounts']:m['Source']=m['Source'].replace('\\','/')
        for k in ('com.docker.compose.project.working_dir','com.docker.compose.project.config_files'):facts['Config']['Labels'][k]=facts['Config']['Labels'][k].replace('\\','/')
        facts['HostConfig']['Binds']=[m['Source']+':'+m['Destination']+(':'+('rw' if m['RW'] else 'ro')) for m in facts['Mounts'] if m['Type']=='bind']
        facts['HostConfig']['Mounts']=[dict(Type='volume',Source=row['identity'][key],Target=target,ReadOnly=ro,VolumeOptions=dict(NoCopy=True)) for key,target,ro in [('volume','/var/lib/postgresql',False),('source_volume',d.ROOT,False),('build_volume','/target',True)]]
        expected=dict(id=facts['Id'],name=row['identity']['pg_name'],image=image['Id'],image_ref=h.PG_IMAGE,env=h.env_dict(facts['Config']['Env']),labels=facts['Config']['Labels'],cmd=facts['Config']['Cmd'],entrypoint=facts['Config']['Entrypoint'],mounts={(m['Type'],m['Source'],m['Destination'],m['RW']) for m in facts['Mounts']},network=row['identity']['network'],builder=False,user='')
        with self.assertRaisesRegex(h.GateError,'PG_BINDS_MODE'):h.validate_container(facts,expected)
        before=copy.deepcopy(facts)
        with patch.object(module,'_SchemaDirectory') as directory:
            with module._SchemaPgAdapter(h,case,row['volumes']) as adapter:
                adapter.validate_container(facts,expected)
                owner=dict(intent=dict(expected=dict(expected,id=None),network=dict(name=row['identity']['network'],project=row['identity']['project'],subnet=plan['subnet'])))
                facts['NetworkSettings']['Networks']={row['identity']['network']:dict(NetworkID=network['Id'])}
                admitted=d.admit_created_pg(adapter,owner,facts,network,{})
                self.assertEqual(admitted['id'],facts['Id'])
                # Exercise the actual shared lifecycle consumers with scrubbed
                # recorded Docker shapes; only RPC I/O and host fs are faked.
                class OriginalMaps:
                    namespace=SimpleNamespace(pin=None)
                    def run(self,command,**kwargs):
                        value=facts if command[1]=='container' else network
                        return 0,json.dumps([value]).encode(),b''
                    def observed(self,call):return call()[1],{'original':'unit-input'}
                    def inspect(self,kind,name):return next(v for v in row['volumes'].values() if v['Name']==name)
                    def docker(self,*args):return (facts['Id']+'\n').encode()
                runner=OriginalMaps();row['subnet']=plan['subnet'];facts['RestartCount']=0
                for phase,pid in [('created',0),('running',42),('exited',0)]:
                    facts['State'].update(Status=phase,Running=phase=='running',Pid=pid,StartedAt='0001-01-01T00:00:00Z' if phase=='created' else '2026-10-08T00:00:00Z',Dead=False,OOMKilled=False,Error='')
                    network['Containers']={facts['Id']:{}} if phase=='running' else {}
                    actual,_=d.pg_facts(adapter,runner,row,admitted)
                    self.assertEqual(actual['State']['Status'],phase)
                    self.assertEqual(runner.namespace.pin is not None,phase!='created')
                # Cleanup re-admission and the known-container check use the
                # same adapter even after the primary operation failed.
                cleanup=d.admit_created_pg(adapter,owner,facts,network,{})
                adapter.validate_container(facts,cleanup)
                for change in ('extra_rw','duplicate','wrong_source','wrong_target','readonly','volume','security','label','options'):
                    changed=copy.deepcopy(facts)
                    if change=='extra_rw':changed['HostConfig']['Binds'].append('/foreign:/foreign:rw')
                    if change=='duplicate':changed['HostConfig']['Binds'][0]=changed['HostConfig']['Binds'][-1]
                    if change=='wrong_source':changed['HostConfig']['Binds'][-1]='/foreign:/var/lib/knowweave-schema:rw'
                    if change=='wrong_target':changed['HostConfig']['Binds'][-1]=str(case/'schema')+':/foreign:rw'
                    if change=='readonly':changed['HostConfig']['Binds'][-1]=str(case/'schema')+':/var/lib/knowweave-schema:ro'
                    if change=='volume':changed['HostConfig']['Mounts'][0]['VolumeOptions']['NoCopy']=False
                    if change=='security':changed['HostConfig']['Privileged']=True
                    if change=='label':changed['Config']['Labels']['com.docker.compose.project']='foreign'
                    if change=='options':changed['HostConfig']['Binds'][-1]+=',shared'
                    with self.subTest(change=change),self.assertRaises((module.SchemaResourceError,h.GateError,d.GateError)):d.admit_created_pg(adapter,owner,changed,network,{})
                    with self.subTest(cleanup_change=change),self.assertRaises((module.SchemaResourceError,h.GateError)):adapter.validate_container(changed,cleanup)
            self.assertGreater(directory.return_value.check.call_count,0)
        for key in ('NetworkSettings','State','RestartCount'):before[key]=facts[key]
        self.assertEqual(facts,before)
        with self.assertRaisesRegex(h.GateError,'PG_BINDS_MODE'):h.validate_container(facts,expected)

    def test_fixed_failure_diagnostics_keep_primary_and_cleanup_without_messages(self):
        module=self.schema_resources();lane=self.lane();h,_,_,_=lane._modules()
        result={};module._record_failure(result,'primary','pg-admit',h.GateError('PG_BINDS_MODE'))
        module._record_failure(result,'cleanup','cleanup-pg-admit',h.GateError('PG_BINDS_MODE'))
        self.assertEqual(result['primary_failure'],dict(stage='pg-admit',exception_class='GateError',reason_code='PG_BINDS_MODE'))
        self.assertEqual(result['cleanup_failures'],[dict(stage='cleanup-pg-admit',exception_class='GateError',reason_code='PG_BINDS_MODE')])
        class Unsafe(Exception):
            def __str__(self):raise AssertionError('must not format exception')
        module._record_failure(result,'cleanup','cleanup-pg-stop',Unsafe('postgresql://secret'))
        module._record_failure(result,'cleanup','cleanup-pg-stop',h.GateError('postgresql://secret'))
        self.assertNotIn('secret',repr(result));self.assertEqual(result['primary_failure']['stage'],'pg-admit')
        for _ in range(40):module._record_failure(result,'cleanup','cleanup-pg-stop',Unsafe())
        self.assertEqual(len(result['cleanup_failures']),16)

    def test_schema_directory_rejects_wrong_owner_mode_and_replacement(self):
        from types import SimpleNamespace
        from unittest.mock import patch
        import stat
        module=self.schema_resources();good=dict(st_mode=stat.S_IFDIR|0o700,st_uid=0,st_dev=1,st_ino=2)
        with patch.object(module.registry,'open_directory',return_value=17),patch.object(module.os,'close') as close,patch.object(module.os,'fstat',return_value=SimpleNamespace(**good)),patch.object(module.os,'stat',return_value=SimpleNamespace(**good)) as at_path:
            held=module._SchemaDirectory(pathlib.PurePosixPath('/private/schema'));held.check()
            for change in [dict(good,st_ino=3),dict(good,st_uid=1),dict(good,st_mode=stat.S_IFDIR|0o755),dict(good,st_mode=stat.S_IFLNK|0o700)]:
                at_path.return_value=SimpleNamespace(**change)
                with self.assertRaises(module.SchemaResourceError):held.check()
            held.close();close.assert_called_once_with(17)

if __name__=='__main__':unittest.main()
