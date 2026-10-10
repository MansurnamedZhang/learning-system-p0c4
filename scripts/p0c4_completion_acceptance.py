#!/usr/bin/env python3
"""Fixed root-owned Task1 case envelope over the reviewed lifecycle controller.

Usage: run --plan PATH --case CASE; verify --result PATH.
Root must install this exact package and allocate the private plan/input roots.
No input may select a command, credential, product pin or Docker endpoint.
"""
import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import stat
import sys
import time
import uuid
from types import SimpleNamespace
if __package__:
    from . import p0c4_storage_registry as registry
    from . import p0c4_source_lifecycle_gate_acceptance as driver
    from . import p0c4_source_admission_gate_acceptance as admission
    from . import p0c4_source_binding_gate_acceptance as binding
    from . import p0c4_source_isolation as isolation
else:
    import p0c4_storage_registry as registry
    import p0c4_source_lifecycle_gate_acceptance as driver
    import p0c4_source_admission_gate_acceptance as admission
    import p0c4_source_binding_gate_acceptance as binding
    import p0c4_source_isolation as isolation

if __package__:
    from .p0c4_completion import plan as envelope
    from .p0c4_completion import evidence, resources, schema as schema_lane, full_import as full_lane
    from . import p0c4_completion as helper_package
else:
    from p0c4_completion import plan as envelope
    from p0c4_completion import evidence, resources, schema as schema_lane, full_import as full_lane
    import p0c4_completion as helper_package
CASE_TESTS,BUILDER,POSTGRES,PLAN_KEYS,RECEIPT_KEYS = envelope.CASE_TESTS,envelope.BUILDER,envelope.POSTGRES,envelope.PLAN_KEYS,envelope.RECEIPT_KEYS
ALL_CASE_TESTS={**CASE_TESTS,**envelope.SOURCE_CASE_TESTS,**envelope.DESTINATION_CASE_TESTS,**schema_lane.CASE_TESTS,**full_lane.CASE_TESTS}
CompletionError,require,canonical,digest,validate_plan = envelope.CompletionError,envelope.require,envelope.canonical,envelope.digest,envelope.validate_plan
exact_counts,private_read,write_record,file_ref,relative_ref = evidence.exact_counts,evidence.private_read,evidence.write_record,evidence.file_ref,evidence.relative_ref
actor_preflight=resources.actor_preflight
LIFECYCLE_CASE='source-lifecycle-eight'
LIFECYCLE_PASS='SOURCE_LIFECYCLE_EIGHT_CURRENT_PASSED_NOT_COMPLETE'
SOURCE_PASS='SOURCE_ENDPOINT_TASK2_CASE_PASSED_NOT_COMPLETE'
DESTINATION_PASS='DESTINATION_TASK3_CASE_PASSED_NOT_COMPLETE'
FULL_PASS='FULL_IMPORT_CASE_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE'

def validate_source_fault_transitions(case,name):
    rows=case.get('source_fault_transitions');count=6 if name=='source-cancel' else 2
    keys={'backup_id','epoch','nonce','phase','postmaster_start_ticks','observed_state','container_id','external_signal','actual_postmaster_restart'}
    require(type(rows) is list and len(rows)==count,'RESULT_SOURCE_FAULT_TRANSITIONS')
    seen=set();nonces=set()
    for index,row in enumerate(rows):
        require(type(row) is dict and set(row)==keys and row['container_id']==case.get('container_id') and row['actual_postmaster_restart'] is False and registry.integer(row['postmaster_start_ticks'],1,2**64-1),'RESULT_SOURCE_FAULT_TRANSITIONS')
        for key in ('backup_id','epoch','nonce'):
            try:registry.v4(row[key])
            except registry.RegistryError as error:raise CompletionError('RESULT_SOURCE_FAULT_TRANSITIONS') from error
        require(row['nonce'] not in nonces,'RESULT_SOURCE_FAULT_TRANSITIONS');nonces.add(row['nonce'])
        pause=index%2==0
        require(row['phase']==('pause' if pause else 'resume') and row['external_signal']==('SIGSTOP' if pause else 'SIGCONT') and (row['observed_state']=='T' if pause else row['observed_state'] in ('R','S','D','I')),'RESULT_SOURCE_FAULT_TRANSITIONS')
        if pause:
            require(row['backup_id'] not in seen,'RESULT_SOURCE_FAULT_TRANSITIONS');seen.add(row['backup_id'])
        else:
            previous=rows[index-1]
            require(all(row[k]==previous[k] for k in ('backup_id','epoch','postmaster_start_ticks','container_id')) and row['nonce']!=previous['nonce'],'RESULT_SOURCE_FAULT_TRANSITIONS')

def validate_lifecycle_bodies(plan,result):
    envelope.validate_lifecycle_plan(plan)
    schedule=tuple((name,index==0,False) for index,name in enumerate(driver.PG_TESTS))
    driver.scheduled_gates(SimpleNamespace(scope='all'),schedule,result)
    require(all(all(type(row.get('outcome',{}).get(key)) is int and row['outcome'][key]==value
                    for key,value in dict(exit_code=0,passed=1,failed=0,ignored=0).items())
                for row in result['cases']),'SOURCE_REGRESSION_BODY_COUNTS')

def _installed_full_controller_bytes(relative):
    # Resolve only the fixed closure below, never import to discover a path.
    path=Path(__file__).absolute().parent/relative
    for ancestor in reversed(path.parents):
        meta=os.lstat(ancestor)
        require(stat.S_ISDIR(meta.st_mode) and meta.st_uid==0 and meta.st_mode&0o022==0,'INSTALLED_FULL_ANCESTOR')
    fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC)
    try:
        meta=os.fstat(fd)
        require(stat.S_ISREG(meta.st_mode) and meta.st_uid==0 and meta.st_nlink==1 and meta.st_mode&0o022==0 and 0<meta.st_size<=1024*1024,'INSTALLED_FULL_METADATA')
        raw=bytearray()
        while len(raw)<=1024*1024:
            chunk=os.read(fd,min(65536,1024*1024+1-len(raw)))
            if not chunk:break
            raw.extend(chunk)
        after=os.fstat(fd)
        require(len(raw)==meta.st_size and (meta.st_dev,meta.st_ino,meta.st_size,meta.st_mtime_ns,meta.st_ctime_ns)==(after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns),'INSTALLED_FULL_CHANGED')
        return bytes(raw)
    finally:os.close(fd)


def _verify_full_import_closure(files):
    for relative in ('p0c4_restore_target.py','p0c4_restore_target_birth.py','p0c4_restore_target_pin_prepare.py','p0c4_restore_target_pin.py','p0c4_restore_birth_acceptance.py','p0c4_completion/roles.py','p0c4_completion/full_import.py','p0c4_completion/full_target_fs.py','p0c4_controlled_import_acceptance.py','p0c4_maintenance_gate_acceptance.py'):
        entry='scripts/'+relative
        require(entry in files and _installed_full_controller_bytes(relative)==files[entry],'INSTALLED_FULL_CONTROLLER')


def accepted_source(plan):
    source=plan['source'];raws={}
    for key in ('archive','manifest'):
        ref=source[key];raws[key]=private_read(ref['path'],32*1024**2)
        require(len(raws[key])==ref['size'] and digest(raws[key])==ref['sha256'],'FROZEN_SOURCE_READBACK')
    helper_raw=Path(admission.__file__).read_bytes()
    manifest,files=admission.verify_package(raws['archive'],source['archive']['sha256'],source['manifest']['sha256'],digest(helper_raw),'green')
    require(canonical(manifest)==raws['manifest'] and manifest['base_commit']==source['application_commit'],'FROZEN_SOURCE_MANIFEST')
    # Every imported controller/provisioner is checked against this accepted
    # package. No dynamic module is selected by the plan.
    for module in (admission,binding,driver,isolation,registry,envelope,evidence,resources,helper_package,sys.modules[__name__]):
        entry='scripts/'+Path(module.__file__).resolve().relative_to(Path(__file__).resolve().parent).as_posix()
        require(entry in files and files[entry]==Path(module.__file__).read_bytes(),'INSTALLED_PACKAGE_CONTROLLER')
    if plan['case_name'] in full_lane.CASE_TESTS:
        _verify_full_import_closure(files)
        import p0c4_restore_target, p0c4_restore_target_birth, p0c4_restore_target_pin_prepare, p0c4_restore_target_pin, p0c4_restore_birth_acceptance
        from p0c4_completion import roles as full_roles, full_target_fs
        import p0c4_controlled_import_acceptance, p0c4_maintenance_gate_acceptance
        for module in (full_lane,full_roles,p0c4_restore_target,p0c4_restore_target_birth,p0c4_restore_target_pin_prepare,p0c4_restore_target_pin,p0c4_restore_birth_acceptance,full_target_fs,p0c4_controlled_import_acceptance,p0c4_maintenance_gate_acceptance):
            entry='scripts/'+Path(module.__file__).resolve().relative_to(Path(__file__).resolve().parent).as_posix()
            require(entry in files and files[entry]==Path(module.__file__).read_bytes(),'INSTALLED_FULL_CONTROLLER')
    driver.PUBLIC_PINS={entry:digest(files[entry]) for entry in driver.PUBLIC_PINS}
    driver.PRODUCT_PINS={entry:(digest(files[entry]),len(files[entry])) for entry in (*driver.PRODUCT_PINS,'crates/learning-backup/src/registry.rs','crates/learning-backup/src/protection.rs')}
    driver.BASE_COMMIT=manifest['base_commit'];binding.HELPER_SHA=digest(helper_raw)
    admission.GateError=binding.GateError=driver.GateError
    return manifest,files

def run(plan_path,case):
    if case in schema_lane.CASE_TESTS:return schema_lane.run(plan_path,case)
    actor_preflight();os.umask(0o077)
    full_context=full_lane.read_context(plan_path,case) if case in full_lane.CASE_TESTS else None
    raw=private_read(plan_path,65536);plan=validate_plan(registry.parse_record(raw),case);root=Path(plan['evidence_root'])
    require(Path(plan_path).parent==root,'PLAN_CASE_DIRECTORY')
    fd=registry.open_directory(str(root))
    try:require(registry.names(fd,2)==[Path(plan_path).name],'CASE_ALREADY_ATTEMPTED')
    finally:os.close(fd)
    manifest,files=accepted_source(plan)
    write_record(root/'case-plan.json',raw)
    lifecycle=case==LIFECYCLE_CASE
    deadline=time.monotonic()+7200
    schedule=[(driver.completion_case_identity(row['case_id']),row['subnet'],name,index==0) for index,(row,name) in enumerate(zip(plan['cases'],driver.PG_TESTS))] if lifecycle else [(driver.completion_case_identity(plan['case_id']),plan['subnet'],ALL_CASE_TESTS[case],True)]
    pass_status=FULL_PASS if full_context is not None else LIFECYCLE_PASS if lifecycle else SOURCE_PASS if case in envelope.SOURCE_CASE_TESTS else DESTINATION_PASS if case in envelope.DESTINATION_CASE_TESTS else 'REGISTRY_TASK1_CASE_PASSED_NOT_COMPLETE'
    result=dict(status='FAILED',cleanup_verified=False,cases=[],helpers=[],batch_id=plan['batch_id'],base_commit=plan['source']['application_commit'],archive_sha256=plan['source']['archive']['sha256'],current_source=True,root_trust_scope='fixed_fresh_completion_case')
    raw_budget=[0]
    batch=None;runner=None;images=None;source=None;raw_index=None;status='UNCONFIRMED_UNUSABLE'
    try:
        stage=root/'driver';admission.mkdir_new(stage);batch=binding.create_batch(admission,stage,plan['batch_id']);source=batch/'source';binding.extract_public_source(admission,source,files)
        result['source_sha256_before']=binding.source_digest(admission,source,manifest)
        runner=driver.owned_log_runner(admission,batch,result,len(schedule))
        identities=[row[0] for row in schedule];subnets=[row[1] for row in schedule]
        if case=='source-clone':identities.append(driver.completion_case_identity(plan['clone']['case_id']));subnets.append(plan['clone']['subnet'])
        driver.fresh_resource_preflight(admission,runner,identities,subnets,result,route_observer=lambda:resources.route_observer(batch,result))
        planned=[ident[k] for ident,_,_,_ in schedule for k in ('volume','source_volume','build_volume','registry_volume')]
        if case=='source-clone':planned.append(identities[-1]['volume'])
        driver.admit_resources(planned,runner.docker('volume','ls','--format','{{.Name}}').decode().splitlines())
        # cache_scope only names regenerable state. Task1 conservatively leaves
        # its cache uncreated; every source/build/PG/registry volume is fresh.
        result['compile_cache']=dict(name='kwc4c-cache-'+uuid.UUID(plan['cache_scope']).hex,used=False)
        images={ref:runner.inspect('image',ref) for ref in (admission.BUILDER,admission.PG_IMAGE)}
        require(images[admission.BUILDER]['Id']==BUILDER and any(r.split('@')[-1]==POSTGRES for r in images[admission.PG_IMAGE].get('RepoDigests',[])),'CASE_CACHED_IMAGES')
        require(all(not any(key in admission.env_dict(image['Config']['Env']) for key in (driver.PIN_ENV,'KNOWWEAVE_C4_VERIFIER_KEY_SHA256','PGPASSWORD','DATABASE_URL')) for image in images.values()),'CASE_IMAGE_ENV')
        require(time.monotonic()<deadline,'CASE_DEADLINE')
        require(os.statvfs(batch).f_bavail*os.statvfs(batch).f_frsize>=64*1024**3,'CASE_FREE_SPACE_BUDGET')
        result['preflight_complete']=True
        for ident,subnet,name,first in schedule:
            require(os.statvfs(batch).f_bavail*os.statvfs(batch).f_frsize>=64*1024**3,'CASE_FREE_SPACE_BUDGET')
            driver.fresh_resource_preflight(admission,runner,[ident],[subnet],result,ident['case_id'],route_observer=lambda:resources.route_observer(batch,result))
            if case=='source-clone':
                clone=identities[-1];driver.fresh_resource_preflight(admission,runner,[clone],[plan['clone']['subnet']],result,clone['case_id'],route_observer=lambda:resources.route_observer(batch,result))
            driver.live_case(admission,binding,isolation,runner,batch,source,ident,subnet,name,images,result,manifest,files,plan['source']['archive']['sha256'],first,False,total_deadline=deadline,_full_context=full_context,**({'source_clone':plan['clone']} if case=='source-clone' else {}))
        require(time.monotonic()<deadline,'CASE_DEADLINE')
        driver.final_audits(admission,binding,runner,result,images,source)
        result['source_sha256_after']=binding.source_digest(admission,source,manifest)
        driver.equal_audit(result['source_sha256_before'],result['source_sha256_after'])
        if lifecycle:validate_lifecycle_bodies(plan,result)
        else:require(len(result['cases'])==1 and result['cases'][0]['test']==ALL_CASE_TESTS[case] and all(result['cases'][0]['outcome'][k]==v for k,v in dict(exit_code=0,passed=1,failed=0,ignored=0).items()),'CASE_BODY_COUNTS')
        if full_context is not None:full_lane.validate_result(plan,result)
        result['status']=pass_status
    except BaseException as error:
        result.update(status='FAILED',reason=('FULL_PREPARATION_FAILED' if full_context is not None else str(error) if isinstance(error,(CompletionError,driver.GateError,registry.RegistryError,resources.ResourceError)) else 'CASE_UNEXPECTED_ERROR'))
        if full_context is not None:result['preparation_failure']=full_context._failure_observation()
    finally:
        if batch is not None:
            try:
                if runner is not None:
                    errors=driver.cleanup_known_helpers(runner,result)
                    if errors:result['global_cleanup_errors']=errors
                    result['registered_transports_reaped']=all(c.retired or (c.finished and c.p.poll() is not None and not any(t.is_alive() for t in c.threads)) for c in runner.ownership.children)
                result['cleanup_verified']=driver.cleanup_verified(result)
                if not result['cleanup_verified']:
                    result['status']='FAILED'
                    if full_context is None:result['reason']='CASE_CLEANUP_UNKNOWN'
                    else:result['cleanup_reason']='CASE_CLEANUP_UNKNOWN'
                if runner is not None:raw_index=driver.publish_raw_index(batch,runner,result,budget=raw_budget)
                binding.finalize_result(admission,batch,result)
                require(not (batch/'evidence/result.pending').exists(),'CASE_PENDING_RETAINED')
                final_raw=private_read(batch/'evidence/result.json',32*1024**2);evidence.parse_driver_result(final_raw)
                require(final_raw==canonical(result),'CASE_FINAL_PUBLISHER_READBACK')
                status=result['status'] if result['status']==pass_status else 'FAILED_UNUSABLE'
            except BaseException:status='UNCONFIRMED_UNUSABLE'
    # A complete envelope exists only when its fixed nested artifacts do. Late
    # failures may preserve pending/raw evidence, never publish a pass receipt.
    if batch is None or raw_index is None or not result['cases']:
        raise CompletionError(status)
    if lifecycle:
        receipt=dict(format_version=1,capability='c4_completion_source_regression_receipt_v1',batch_id=plan['batch_id'],case_name=LIFECYCLE_CASE,plan_sha256=digest(raw),driver_result=file_ref(batch/'evidence/result.json',root),raw_index=file_ref(batch/'evidence/raw-index.json',root),status=status)
        _verify_lifecycle_envelope(root,receipt,raw_budget=raw_budget,reserve_recheck=True)
        write_record(root/'case-receipt.json',canonical(receipt));verify(root/'case-receipt.json',raw_budget=raw_budget);return receipt
    record=result['cases'][0];issuer=record.get('issuer',{}).get('registry_provisioning')
    require(issuer is not None,'CASE_ISSUER_UNCONFIRMED')
    issuer_raw=canonical(issuer);write_record(root/'registry-provisioning.json',issuer_raw)
    outcome=record.get('outcome',dict(exit_code=-1,passed=0,failed=0,ignored=0))
    receipt=dict(format_version=1,capability='c4_completion_case_receipt_v1',batch_id=plan['batch_id'],case_id=plan['case_id'],case_name=case,plan_sha256=digest(raw),registry_provisioning_sha256=digest(issuer_raw),source_archive_sha256=plan['source']['archive']['sha256'],source_manifest_sha256=plan['source']['manifest']['sha256'],application_build_sha256=plan['source']['application_build_sha256'],binary_sha256=record.get('compiled',{}).get('lib',{}).get('sha256','0'*64),test_name=ALL_CASE_TESTS[case],test_exit=outcome['exit_code'],passed=outcome['passed'],failed=outcome['failed'],ignored=outcome['ignored'],driver_result=file_ref(batch/'evidence/result.json',root),raw_index=file_ref(batch/'evidence/raw-index.json',root),status=status)
    receipt_raw=canonical(receipt);require(len(receipt_raw)<=16384,'CASE_RECEIPT_BYTES');_verify_envelope(root,receipt,raw_budget=raw_budget,reserve_recheck=True);write_record(root/'case-receipt.json',receipt_raw)
    verify(root/'case-receipt.json',raw_budget=raw_budget);return receipt

def verify(result_path,*,raw_budget=None):
    actor_preflight(False);root=Path(result_path).parent;receipt=registry.parse_record(private_read(result_path,16384))
    if receipt.get('capability')=='c4_schema_content_receipt_v1':return schema_lane.verify(result_path)
    if receipt.get('capability')=='c4_completion_source_regression_receipt_v1':return _verify_lifecycle_envelope(root,receipt,raw_budget=raw_budget)
    return _verify_envelope(root,receipt,raw_budget=raw_budget)

def _validate_lifecycle_case_audits(row,issuer):
    """Read back the existing broker/retained-volume audit, including held roots."""
    sql=row.get('sql_identity');full=row.get('issuer')
    require(type(sql) is dict and set(sql)=={'database','database_oid','system_identifier'} and sql['database']==row['identity']['database'] and registry.integer(sql['database_oid'],1,2**32-1) and type(sql['system_identifier']) is str and re.fullmatch('[1-9][0-9]{0,19}',sql['system_identifier']) and int(sql['system_identifier'])<2**64,'SOURCE_REGRESSION_SQL_IDENTITY')
    require(type(full) is dict and registry.integer(full.get('issuer_euid'),0,0) and type(full.get('binding')) is dict and registry.integer(full['binding'].get('database_oid'),1,2**32-1) and canonical(full.get('sql'))==canonical(sql),'SOURCE_REGRESSION_ISSUER_TYPES')
    driver.validate_issuer(full,sql)
    binding=full['binding'];pin=full['binding_sha256']
    original=dict(binding_sha256=pin,control_dev=binding['control_dev'],control_ino=binding['control_ino'])
    for name in ('pg_root_before','pg_root_after'):
        observed=row.get(name)
        require(type(observed) is dict and set(observed)==set(original) and all(registry.integer(observed[k],1,2**64-1) for k in ('control_dev','control_ino')) and canonical(observed)==canonical(original),'SOURCE_REGRESSION_ORIGINAL_CONTROL')
    require(driver.compile_binding(row)==issuer['source_binding_sha256'],'SOURCE_REGRESSION_PRECOMPILE_BINDING')
    compiled=row.get('compiled');provenance=row.get('artifact_provenance')
    require(type(compiled) is dict and set(compiled)=={'lib','example'} and type(provenance) is dict,'SOURCE_REGRESSION_BINARY_AUDIT')
    binaries={}
    for kind,suffix in (('lib','test'),('example','migrate')):
        value=compiled[kind];proof=provenance.get(kind)
        require(type(value) is dict and type(proof) is dict and type(proof.get('materialization')) is dict,'SOURCE_REGRESSION_BINARY_AUDIT')
        path='/target/retained/'+row['identity']['case_id']+'-'+suffix
        materialized=proof['materialization'];sha=value.get('sha256')
        artifact=value.get('artifact')
        require(type(artifact) is str and (re.fullmatch('/target/build/debug/deps/learning_backup-[0-9a-f]+',artifact) if kind=='lib' else artifact=='/target/build/debug/examples/c4_task3_migrate'),'SOURCE_REGRESSION_BINARY_AUDIT')
        require(value.get('binary')==path and type(sha) is str and registry.HEX64.fullmatch(sha) and registry.integer(value.get('exit_code'),0,0) and materialized.get('sha256')==sha and materialized.get('source_path')==artifact and materialized.get('destination_path')=='/target/materialized/'+row['identity']['case_id']+'-'+kind,'SOURCE_REGRESSION_BINARY_AUDIT')
        binaries[path]=sha
    root_names={'control','pins','assets','asset-staging','proofs','secrets','regressions'}
    issued_names={'source_control':'control','local_pins':'pins','source_assets':'assets','source_staging':'asset-staging'}
    issued={issued_names[r['kind']]:r for r in issuer['roots']}
    audits=[]
    for name in ('audit_before','audit_after','audit_final'):
        audit=row.get(name)
        require(type(audit) is dict and set(audit)=={'binding_sha256','control_dev','control_ino','roots','binaries'} and all(registry.integer(audit[k],1,2**64-1) for k in ('control_dev','control_ino')) and all(audit[k]==original[k] for k in original) and type(audit['roots']) is dict and set(audit['roots'])==root_names and type(audit['binaries']) is dict and canonical(audit['binaries'])==canonical(binaries),'SOURCE_REGRESSION_CASE_AUDIT')
        for root,pointer in audit['roots'].items():
            paths={driver.ROOT+'/'+root}
            if root in ('control','pins') and name!='audit_before':paths.add(driver.ROOT+'/'+root+'-held')
            require(type(pointer) is dict and set(pointer)=={'path','dev','ino'} and type(pointer['path']) is str and pointer['path'] in paths and all(registry.integer(pointer[k],1,2**64-1) for k in ('dev','ino')),'SOURCE_REGRESSION_HELD_ROOT')
            if root in issued:
                require(issued[root]['path']==driver.ROOT+'/'+root and all(pointer[k]==issued[root][k] for k in ('dev','ino')),'SOURCE_REGRESSION_ENROLLED_ROOT')
            if audits:
                require(all(pointer[k]==audits[0]['roots'][root][k] for k in ('dev','ino')),'SOURCE_REGRESSION_HELD_ROOT')
        audits.append(audit)
    require(canonical({k:v for k,v in audits[0].items() if k!='roots'})==canonical({k:v for k,v in audits[1].items() if k!='roots'}) and canonical(audits[1])==canonical(audits[2]),'SOURCE_REGRESSION_AUDIT_JOIN')


def _validate_destination_case(row,issuer):
    require(row.get('test') in driver.DESTINATION_TESTS and row.get('kind')=='REAL_DESTINATION_FILES_SYNTHETIC_HISTORY_NOT_COMPLETE' and
            type(row.get('generations')) is list and row['generations']==[] and not any(k.startswith(('drain_','source_fault_')) for k in row),'DESTINATION_NO_SOURCE_CREDIT')
    _validate_lifecycle_case_audits(row,issuer)
    require(canonical(row['audit_before'])==canonical(row['audit_after'])==canonical(row['audit_final']),'DESTINATION_AUDIT_UNCHANGED')
    require(row.get('destination_acl_before')==row.get('acl_after')=='t','DESTINATION_ACL_UNCHANGED')
    views=[]
    for name in ('destination_source_before','destination_source_after','destination_source_final'):
        state=driver.destination_source_inventory(row.get(name));views.append(state)
        for root in ('control','pins','assets','asset-staging'):
            require(all(state[root][k]==row['audit_before']['roots'][root][k] for k in ('dev','ino')),'DESTINATION_SOURCE_ROOT_JOIN')
        require(state['control/source-binding.json']['sha256']==issuer['source_binding_sha256'],'DESTINATION_SOURCE_BINDING_JOIN')
    require(canonical(views[0])==canonical(views[1])==canonical(views[2]),'DESTINATION_SOURCE_UNCHANGED')


def _verify_lifecycle_envelope(root,receipt,*,raw_budget=None,reserve_recheck=False):
    keys={'format_version','capability','batch_id','case_name','plan_sha256','driver_result','raw_index','status'}
    require(type(receipt) is dict and set(receipt)==keys and type(receipt['format_version']) is int and receipt['format_version']==1 and receipt['capability']=='c4_completion_source_regression_receipt_v1','SOURCE_REGRESSION_RESULT_SCHEMA')
    require(receipt['case_name']==LIFECYCLE_CASE and receipt['status'] in (LIFECYCLE_PASS,'FAILED_UNUSABLE','UNCONFIRMED_UNUSABLE'),'SOURCE_REGRESSION_RESULT_STATUS')
    plan_raw=private_read(root/'case-plan.json',65536);plan=validate_plan(registry.parse_record(plan_raw),LIFECYCLE_CASE)
    require(receipt['plan_sha256']==digest(plan_raw) and receipt['batch_id']==plan['batch_id'] and root.as_posix()==plan['evidence_root'],'SOURCE_REGRESSION_PLAN_LINK')
    source=plan['source'];nested=evidence.parse_driver_result(relative_ref(root,receipt['driver_result'],32*1024**2))
    require(nested.get('batch_id')==plan['batch_id'] and nested.get('archive_sha256')==source['archive']['sha256'] and nested.get('base_commit')==source['application_commit'] and nested.get('current_source') is True,'SOURCE_REGRESSION_SOURCE_LINK')
    if receipt['status']==LIFECYCLE_PASS:
        require(all(type(nested.get(k)) is str and registry.HEX64.fullmatch(nested[k]) for k in ('source_sha256_before','source_sha256_after')) and nested['source_sha256_before']==nested['source_sha256_after'],'SOURCE_REGRESSION_SOURCE_UNCHANGED')
    cases=nested.get('cases');require(type(cases) is list and 1<=len(cases)<=8,'SOURCE_REGRESSION_CASE_COUNT')
    for index,row in enumerate(cases):
        require(type(row) is dict and row.get('test')==driver.PG_TESTS[index] and row.get('subnet')==plan['cases'][index]['subnet'],'SOURCE_REGRESSION_CASE_ORDER')
        ident=row.get('identity');expected=driver.completion_case_identity(plan['cases'][index]['case_id'])
        require(type(ident) is dict and set(ident)==set(expected) and all(ident[k]==v for k,v in expected.items() if k!='other_database'),'SOURCE_REGRESSION_CASE_IDENTITY')
        other=ident['other_database'];prefix='learning_backup_c4_task3_'
        require(type(other) is str and other.startswith(prefix) and other!=ident['database'],'SOURCE_REGRESSION_OTHER_DATABASE')
        registry.v4(other[len(prefix):])
        require(type(row.get('issuer')) is dict,'SOURCE_REGRESSION_ISSUER_LINK')
        issuer=registry.validate_provisioning_receipt(row['issuer'].get('registry_provisioning'))
        require(issuer['batch_id']==plan['batch_id'] and issuer['case_id']==ident['case_id'] and issuer['state']=='READBACK_DURABLE' and issuer['source_package_sha256']==source['archive']['sha256'] and issuer['application_commit']==source['application_commit'] and issuer['application_build_sha256']==source['application_build_sha256'],'SOURCE_REGRESSION_ISSUER_LINK')
        require(row.get('archive_sha256')==source['archive']['sha256'],'SOURCE_REGRESSION_CASE_SOURCE')
        if receipt['status']==LIFECYCLE_PASS:
            _validate_lifecycle_case_audits(row,issuer)
            require(row.get('compile_source')==dict(application_commit=source['application_commit'],application_build_sha256=source['application_build_sha256'],source_binding_sha256=issuer['source_binding_sha256']),'SOURCE_REGRESSION_COMPILE_LINK')
            require(all(row.get(k) is True for k in ('stopped','execs_absent','transports_reaped','retained_network_empty')) and type(row.get('audit_after')) is dict and canonical(row['audit_after'])==canonical(row.get('audit_final')),'SOURCE_REGRESSION_CLEANUP')
            binary=row.get('compiled',{}).get('lib',{}).get('sha256')
            require(type(binary) is str and registry.HEX64.fullmatch(binary) and row.get('artifact_provenance',{}).get('lib',{}).get('materialization',{}).get('sha256')==binary,'SOURCE_REGRESSION_BINARY_PROVENANCE')
    _verify_raw_evidence(root,receipt,nested,raw_budget,reserve_recheck)
    if receipt['status']==LIFECYCLE_PASS:
        validate_lifecycle_bodies(plan,nested)
        require(nested['status']==LIFECYCLE_PASS and nested.get('cleanup_verified') is True and nested.get('registered_transports_reaped') is True,'SOURCE_REGRESSION_NO_FALSE_PASS')
        require(not (root/receipt['driver_result']['path']).parent.joinpath('result.pending').exists(),'RESULT_PENDING')
    return receipt

def _verify_envelope(root,receipt,*,raw_budget=None,reserve_recheck=False):
    require(type(receipt) is dict and set(receipt)==RECEIPT_KEYS and type(receipt['format_version']) is int and receipt['format_version']==1 and receipt['capability']=='c4_completion_case_receipt_v1','RESULT_SCHEMA')
    for key in ('batch_id','case_id','case_name','test_name','status'):
        require(type(receipt[key]) is str,'RESULT_FIELD_TYPE')
    require(receipt['case_name'] in ALL_CASE_TESTS and receipt['case_name'] not in schema_lane.CASE_TESTS,'RESULT_FIXED_CASE')
    expected_pass=FULL_PASS if receipt['case_name'] in full_lane.CASE_TESTS else SOURCE_PASS if receipt['case_name'] in envelope.SOURCE_CASE_TESTS else DESTINATION_PASS if receipt['case_name'] in envelope.DESTINATION_CASE_TESTS else 'REGISTRY_TASK1_CASE_PASSED_NOT_COMPLETE'
    require(receipt['status'] in (expected_pass,'FAILED_UNUSABLE','UNCONFIRMED_UNUSABLE'),'RESULT_CASE_STATUS')
    for key in ('batch_id','case_id'):
        try:registry.v4(receipt[key])
        except registry.RegistryError as error:raise CompletionError('RESULT_UUID') from error
    for key in ('plan_sha256','registry_provisioning_sha256','source_archive_sha256','source_manifest_sha256','application_build_sha256','binary_sha256'):
        require(type(receipt[key]) is str and registry.HEX64.fullmatch(receipt[key]),'RESULT_DIGEST')
    require(all(registry.integer(receipt[k],0,100000) for k in ('passed','failed','ignored')) and registry.integer(receipt['test_exit'],-255,255) and receipt['status'] in ('REGISTRY_TASK1_CASE_PASSED_NOT_COMPLETE',SOURCE_PASS,DESTINATION_PASS,FULL_PASS,'FAILED_UNUSABLE','UNCONFIRMED_UNUSABLE'),'RESULT_STATUS')
    evidence.validate_ref(receipt['driver_result'],32*1024**2);evidence.validate_ref(receipt['raw_index'],512*1024)
    plan_raw=private_read(root/'case-plan.json',65536);plan=validate_plan(registry.parse_record(plan_raw),receipt['case_name'])
    require(digest(plan_raw)==receipt['plan_sha256'] and all(receipt[k]==plan[k] for k in ('batch_id','case_id','case_name')) and receipt['test_name']==ALL_CASE_TESTS[receipt['case_name']],'RESULT_PLAN_LINK')
    source=plan['source']
    require(root.as_posix()==plan['evidence_root'] and receipt['source_archive_sha256']==source['archive']['sha256'] and receipt['source_manifest_sha256']==source['manifest']['sha256'] and receipt['application_build_sha256']==source['application_build_sha256'],'RESULT_SOURCE_LINK')
    issuer_raw=private_read(root/'registry-provisioning.json',16384);issuer=registry.validate_provisioning_receipt(registry.parse_record(issuer_raw))
    require(digest(issuer_raw)==receipt['registry_provisioning_sha256'] and issuer['batch_id']==receipt['batch_id'] and issuer['case_id']==receipt['case_id'] and issuer['state']=='READBACK_DURABLE','RESULT_ISSUER_LINK')
    require(issuer['source_package_sha256']==source['archive']['sha256'] and issuer['application_commit']==source['application_commit'] and issuer['application_build_sha256']==source['application_build_sha256'],'RESULT_ISSUER_SOURCE_LINK')
    driver_raw=relative_ref(root,receipt['driver_result'],32*1024**2);nested=evidence.parse_driver_result(driver_raw)
    require(nested.get('batch_id')==plan['batch_id'] and nested.get('archive_sha256')==source['archive']['sha256'] and nested.get('base_commit')==source['application_commit'] and type(nested.get('cases')) is list and len(nested['cases'])==1,'RESULT_NESTED_SOURCE_LINK')
    case=nested['cases'][0];require(type(case) is dict and type(case.get('identity')) is dict,'RESULT_CASE_IDENTITY')
    expected_identity=driver.completion_case_identity(plan['case_id'])
    require(set(case['identity'])==set(expected_identity) and all(case['identity'][k]==v for k,v in expected_identity.items() if k!='other_database') and case.get('archive_sha256')==source['archive']['sha256'] and canonical(case.get('issuer',{}).get('registry_provisioning'))==issuer_raw,'RESULT_CASE_SOURCE_LINK')
    other=case['identity']['other_database'];prefix='learning_backup_c4_task3_'
    require(type(other) is str and other.startswith(prefix) and other!=case['identity']['database'],'RESULT_CASE_IDENTITY')
    try:registry.v4(other[len(prefix):])
    except registry.RegistryError as error:raise CompletionError('RESULT_CASE_IDENTITY') from error
    if receipt['status'] in ('REGISTRY_TASK1_CASE_PASSED_NOT_COMPLETE',SOURCE_PASS,DESTINATION_PASS,FULL_PASS) or 'compile_source' in case:
        require(case.get('compile_source')==dict(application_commit=source['application_commit'],application_build_sha256=source['application_build_sha256'],source_binding_sha256=issuer['source_binding_sha256']),'RESULT_COMPILE_SOURCE_LINK')
    _verify_raw_evidence(root,receipt,nested,raw_budget,reserve_recheck)
    if receipt['status'] in ('REGISTRY_TASK1_CASE_PASSED_NOT_COMPLETE',SOURCE_PASS,DESTINATION_PASS,FULL_PASS):
        require(receipt['test_exit']==0 and (receipt['passed'],receipt['failed'],receipt['ignored'])==(1,0,0) and nested['status']==receipt['status'] and nested.get('current_source') is True and nested.get('cleanup_verified') is True and len(nested['cases'])==1,'RESULT_NO_FALSE_PASS')
        case=nested['cases'][0];require(case['test']==receipt['test_name'] and all(case.get(k) is True for k in ('stopped','execs_absent','transports_reaped','retained_network_empty')) and canonical(case.get('audit_after'))==canonical(case.get('audit_final')) and case['compiled']['lib']['sha256']==receipt['binary_sha256'] and all(type(case['outcome'][k]) is int and case['outcome'][k]==v for k,v in dict(exit_code=0,passed=1,failed=0,ignored=0).items()),'RESULT_NESTED_CASE')
        if case['test'] in driver.REGISTRY_INCOMPLETE_TESTS:
            require(type(case.get('registry_unusable_after')) is dict and type(case.get('registry_unusable_final')) is dict and canonical(case['registry_unusable_after'])==canonical(case['registry_unusable_final']),'RESULT_REGISTRY_UNUSABLE')
            registry.validate_fault_audit_observation(case['registry_unusable_after'],issuer,case['test'])
            require(case.get('acl_after')==('t' if case['test']==driver.REGISTRY_INCOMPLETE_TESTS[0] else 'f'),'RESULT_REGISTRY_UNUSABLE_ACL')
        else:require('registry_unusable_after' not in case and 'registry_unusable_final' not in case,'RESULT_REGISTRY_UNUSABLE_UNEXPECTED')
        require(case.get('artifact_provenance',{}).get('lib',{}).get('materialization',{}).get('sha256')==receipt['binary_sha256'],'RESULT_BINARY_PROVENANCE')
        if receipt['case_name'] in full_lane.CASE_TESTS:full_lane.validate_result(plan,nested)
        if receipt['case_name'] in envelope.DESTINATION_CASE_TESTS:
            _validate_destination_case(case,issuer)
        if receipt['case_name']=='source-restart':
            require(case.get('actual_postmaster_restart') is False and case.get('source_failure_exercise')=='actual_random_session_lock_loss','RESULT_SOURCE_LOCK_LOSS_CLASSIFICATION')
        if receipt['case_name'] in ('source-restart','source-cancel','source-dump-failure'):
            validate_source_fault_transitions(case,receipt['case_name'])
        if receipt['case_name']=='source-clone':
            peers=nested.get('source_clones');require(type(peers) is list and len(peers)==1 and canonical(peers[0])==canonical(case.get('source_clone')),'RESULT_SOURCE_CLONE_PEER')
            peer=peers[0];expected_clone=driver.completion_case_identity(plan['clone']['case_id'])
            require(peer.get('identity',{}).get('case_id')==plan['clone']['case_id'] and peer['identity']['pg_name']==expected_clone['pg_name'] and peer['identity']['volume']==expected_clone['volume'] and peer.get('subnet')==plan['clone']['subnet'] and peer.get('container_id')!=case.get('container_id') and all(peer.get(k) is True for k in ('stopped','execs_absent','retained_network_empty','physical_backup_verified')),'RESULT_SOURCE_CLONE_IDENTITIES')
            observation=case.get('source_clone_observation');require(type(observation) is dict and observation.get('response',{}).get('original_container_id')==case['container_id'] and observation['response'].get('clone_container_id')==peer['container_id'] and observation['original_namespace']['pid_ns']!=observation['clone_namespace']['pid_ns'],'RESULT_SOURCE_CLONE_OBSERVATION')
            require(peer.get('copy',{}).get('exit_code')==0 and peer.get('verification',{}).get('exit_code')==0,'RESULT_SOURCE_CLONE_COPY')
            prewrite=case.get('source_clone_prewrite_result',{})
            require(prewrite.get('capture_returned_pin') is False and all(prewrite.get(key) is True for key in ('exact_namespace_refusal','actual_admitted_clone_observation','runtime_connect_unchanged','control_journal_dump_unchanged','pins_unchanged','registry_unchanged')),'RESULT_SOURCE_CLONE_PREWRITE')
            require(driver.cleanup_verified(nested),'RESULT_SOURCE_CLONE_CLEANUP')
        require(not (root/receipt['driver_result']['path']).parent.joinpath('result.pending').exists(),'RESULT_PENDING')
    return receipt

def _verify_raw_evidence(root,receipt,nested,raw_budget,reserve_recheck):
    batch=(root/receipt['driver_result']['path']).parent.parent
    for ref in nested.get('host_route_observations',[]):
        observation=registry.parse_record(relative_ref(batch,ref,512*1024))
        require(observation.get('capability')=='c4_host_routes_v1' and registry.integer(observation.get('route_count'),0,4096) and len(observation.get('routes',[]))==observation['route_count'],'RESULT_NATIVE_ROUTE_SCHEMA')
        raw_routes=relative_ref(batch,observation['raw_file'],1024*1024)
        require(digest(raw_routes)==observation['raw_sha256'] and len(raw_routes)==observation['scan_bytes'],'RESULT_NATIVE_ROUTE_HASH')
    index=registry.parse_record(relative_ref(root,receipt['raw_index'],512*1024));require(set(index)=={'format_version','capability','files','scan_bytes'} and type(index['format_version']) is int and index['format_version']==1 and index['capability']=='c4_source_raw_index_v1' and type(index['files']) is list and len(index['files'])<=4096,'RESULT_RAW_INDEX_SCHEMA')
    require(all(type(r) is dict and set(r)=={'path','size','sha256','dev','ino'} and type(r['path']) is str and r['path'].startswith('logs/') and len(r['path'].split('/'))==2 and registry.integer(r['size'],0,driver.LIMIT) and type(r['sha256']) is str and registry.HEX64.fullmatch(r['sha256']) and registry.integer(r['dev'],1) and registry.integer(r['ino'],1) for r in index['files']),'RESULT_RAW_INDEX_ENTRY')
    logs=(root/receipt['raw_index']['path']).parent/'logs';expected={Path(r['path']).name for r in index['files']}
    require(registry.integer(index['scan_bytes'],0,536870912) and index['scan_bytes']==3*sum(r['size'] for r in index['files']),'RESULT_RAW_INDEX_SCAN_BYTES')
    require(len(expected)==len(index['files']) and driver.raw_log_records(logs,expected,raw_budget if raw_budget is not None else [0])==index['files'],'RESULT_RAW_LOG_HASHES')
    # Refuse before publishing a pass if its mandatory post-publication scan
    # cannot fit. The closed producer logs must retain the verified sizes.
    if reserve_recheck:
        require(raw_budget is not None and raw_budget[0]+sum(r['size'] for r in index['files'])<=536870912,'RESULT_RAW_RECHECK_CAPACITY')

def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__);commands=parser.add_subparsers(dest='operation',required=True)
    command=commands.add_parser('run');command.add_argument('--plan',type=Path,required=True);command.add_argument('--case',choices=(*ALL_CASE_TESTS,LIFECYCLE_CASE),required=True)
    command=commands.add_parser('verify');command.add_argument('--result',type=Path,required=True)
    args=parser.parse_args(argv)
    try:
        receipt=run(args.plan,args.case) if args.operation=='run' else verify(args.result)
        print(canonical(receipt).decode());return 0 if receipt['status'] in ('REGISTRY_TASK1_CASE_PASSED_NOT_COMPLETE',LIFECYCLE_PASS,SOURCE_PASS,DESTINATION_PASS,FULL_PASS,schema_lane.PASS) else 1
    except BaseException as error:
        print(canonical(dict(status='UNCONFIRMED_UNUSABLE',reason=str(error) if isinstance(error,(CompletionError,registry.RegistryError,driver.GateError,resources.ResourceError)) else 'UNEXPECTED_ERROR')).decode());return 1
if __name__=='__main__':sys.exit(main())
