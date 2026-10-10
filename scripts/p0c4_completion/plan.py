"""Strict immutable completion plan schema; no operational enrollment."""
import hashlib
import ipaddress
from pathlib import Path
import uuid
if __package__.startswith('scripts.'):
    from .. import p0c4_storage_registry as registry
else:
    import p0c4_storage_registry as registry

CASE_TESTS = { 'registry-'+name.replace('_','-'): 'protection_tests::real_registry_'+name for name in ('reopen','root_replacement','corrupt_stale','concurrent_capture','protection_release','pending_staged_fault','pending_visible_fault','catalog_fault','retained_release_fault','abandon_ready_fault','abandon_terminal_fault') }
SOURCE_CLONE_TEST='source::endpoint_tests::source_same_id_physical_clone_rejected_before_journal_or_acl'
SOURCE_CASE_TESTS={'source-clone':SOURCE_CLONE_TEST}
SOURCE_CASE_TESTS.update({
    'source-bound-dump':'source::endpoint_tests::source_dump_uses_exact_container_socket_and_admitted_backend',
    'source-restart':'source::endpoint_tests::source_restart_or_lock_loss_blocks_release',
    'source-cancel':'source::endpoint_tests::source_dump_timeout_cancel_and_overflow_reap_child_keep_gate_closed',
    'source-dump-failure':'source::endpoint_tests::source_dump_failure_keeps_registry_protection',
})
DESTINATION_CASE_TESTS={
    'destination-negative':'destination::tests::linux_cases::real_destination_negative',
    'transfer-interrupted':'destination::tests::linux_cases::real_transfer_interrupted',
}
BUILDER='sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b'
POSTGRES='sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d'
PLAN_KEYS={'format_version','capability','batch_id','case_id','cache_scope','case_name','source','images','subnet','evidence_root','budget_profile'}
RECEIPT_KEYS={'format_version','capability','batch_id','case_id','case_name','plan_sha256','registry_provisioning_sha256','source_archive_sha256','source_manifest_sha256','application_build_sha256','binary_sha256','test_name','test_exit','passed','failed','ignored','driver_result','raw_index','status'}
class CompletionError(RuntimeError): pass
def require(ok,token):
    if not ok: raise CompletionError(token)
def canonical(value):return registry.canonical(value)
def digest(raw):return hashlib.sha256(raw).hexdigest()
def validate_plan(plan,case):
    if case.startswith(('full-', 'recovery-')):
        from . import full_import
        if case in full_import.CASE_TESTS:return full_import.validate_plan(plan,case)
    if case in DESTINATION_CASE_TESTS:
        require(type(plan) is dict and set(plan)==PLAN_KEYS and plan['capability']=='c4_completion_destination_plan_v1' and plan['case_name']==case and plan['budget_profile']=='destination_task3_v1','DESTINATION_PLAN')
        base=dict(plan,capability='c4_completion_case_plan_v1',case_name='registry-reopen',budget_profile='registry_task1_v1')
        validate_plan(base,'registry-reopen');return plan
    if case=='source-lifecycle-eight':
        return validate_lifecycle_plan(plan)
    if case=='source-clone':
        return validate_source_clone_plan(plan)
    if case in SOURCE_CASE_TESTS:
        require(type(plan) is dict and set(plan)==PLAN_KEYS and plan['capability']=='c4_completion_source_endpoint_plan_v1' and plan['case_name']==case and plan['budget_profile']=='source_endpoint_task2_v1','SOURCE_ENDPOINT_PLAN')
        base=dict(plan,capability='c4_completion_case_plan_v1',case_name='registry-reopen',budget_profile='registry_task1_v1')
        validate_plan(base,'registry-reopen');return plan
    require(type(plan) is dict and set(plan)==PLAN_KEYS,'PLAN_FIELDS')
    require(type(plan['format_version']) is int and plan['format_version']==1 and plan['capability']=='c4_completion_case_plan_v1' and plan['budget_profile']=='registry_task1_v1','PLAN_VERSION')
    require(case in CASE_TESTS and plan['case_name']==case,'PLAN_FIXED_CASE')
    for key in ('batch_id','case_id','cache_scope'):
        try:registry.v4(plan[key])
        except registry.RegistryError as error:raise CompletionError('PLAN_UUID') from error
    require(len({plan[k] for k in ('batch_id','case_id','cache_scope')})==3,'PLAN_DISTINCT_UUIDS')
    require(type(plan['images']) is dict and plan['images']==dict(builder=BUILDER,postgres=POSTGRES),'PLAN_IMAGES')
    source=plan['source'];require(type(source) is dict and set(source)=={'archive','manifest','application_commit','application_build_sha256'},'PLAN_SOURCE')
    require(type(source['application_commit']) is str and registry.HEX40.fullmatch(source['application_commit']) and type(source['application_build_sha256']) is str and registry.HEX64.fullmatch(source['application_build_sha256']),'PLAN_BUILD')
    for key,maximum in (('archive',32*1024**2),('manifest',2*1024**2)):
        ref=source[key];require(type(ref) is dict and set(ref)=={'path','size','sha256'} and registry.integer(ref['size'],1,maximum) and type(ref['sha256']) is str and registry.HEX64.fullmatch(ref['sha256']),'PLAN_SOURCE_FILEREF')
        try:registry.path_text(ref['path'])
        except registry.RegistryError as error:raise CompletionError('PLAN_INPUT_PATH') from error
    # The current reviewed exact-package build derivation embeds archive SHA.
    require(source['application_build_sha256']==source['archive']['sha256'],'PLAN_BUILD_DERIVATION')
    require(Path(source['archive']['path']).parent==Path(source['manifest']['path']).parent,'PLAN_FROZEN_INPUT_ROOT')
    try:
        registry.path_text(plan['evidence_root']);subnet=ipaddress.IPv4Network(plan['subnet'],strict=True)
        require(subnet.prefixlen==24 and any(subnet.subnet_of(ipaddress.IPv4Network(block)) for block in ('10.0.0.0/8','172.16.0.0/12','192.168.0.0/16')) and str(subnet)==plan['subnet'] and not subnet.is_loopback and not subnet.is_link_local,'PLAN_SUBNET')
    except (ValueError,TypeError,registry.RegistryError) as error:raise CompletionError('PLAN_PATH_OR_SUBNET') from error
    return plan

def validate_source_clone_plan(plan):
    require(type(plan) is dict and set(plan)==PLAN_KEYS|{'clone'},'SOURCE_CLONE_PLAN_FIELDS')
    require(plan['capability']=='c4_completion_source_endpoint_plan_v1' and plan['case_name']=='source-clone' and plan['budget_profile']=='source_clone_two_pg_v1','SOURCE_CLONE_PROFILE')
    clone=plan['clone'];require(type(clone) is dict and set(clone)=={'case_id','subnet'},'SOURCE_CLONE_IDENTITY')
    base={key:value for key,value in plan.items() if key!='clone'}
    base.update(capability='c4_completion_case_plan_v1',case_name='registry-reopen',budget_profile='registry_task1_v1')
    validate_plan(base,'registry-reopen')
    other=dict(base,**clone);validate_plan(other,'registry-reopen')
    require(clone['case_id']!=plan['case_id'] and clone['subnet']!=plan['subnet'],'SOURCE_CLONE_DISTINCT')
    return plan

def validate_lifecycle_plan(plan):
    """One fixed eight-body schedule; leaves cannot select code or authority."""
    keys=(PLAN_KEYS-{'case_id','subnet'})|{'cases'}
    require(type(plan) is dict and set(plan)==keys,'SOURCE_REGRESSION_PLAN_FIELDS')
    require(type(plan['format_version']) is int and plan['format_version']==1 and
            plan['capability']=='c4_completion_source_regression_plan_v1' and
            plan['case_name']=='source-lifecycle-eight' and
            plan['budget_profile']=='source_lifecycle_eight_v1','SOURCE_REGRESSION_PLAN_VERSION')
    rows=plan['cases'];require(type(rows) is list and len(rows)==8,'SOURCE_REGRESSION_EIGHT_CASES')
    identifiers={plan['batch_id'],plan['cache_scope']};subnets=set()
    require(len(identifiers)==2,'SOURCE_REGRESSION_DISTINCT_UUIDS')
    for row in rows:
        require(type(row) is dict and set(row)=={'case_id','subnet'},'SOURCE_REGRESSION_CASE_FIELDS')
        # Reuse the reviewed source/image/path/UUID/network validators without
        # manufacturing a legacy prerequisite or accepting a per-case source.
        leaf={key:value for key,value in plan.items() if key!='cases'}
        leaf.update(row,capability='c4_completion_case_plan_v1',case_name='registry-reopen',budget_profile='registry_task1_v1')
        validate_plan(leaf,'registry-reopen')
        require(row['case_id'] not in identifiers and row['subnet'] not in subnets,'SOURCE_REGRESSION_FRESH_CASES')
        identifiers.add(row['case_id']);subnets.add(row['subnet'])
    return plan
