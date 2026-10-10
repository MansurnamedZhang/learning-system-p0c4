"""Private seven-mount schema profile; legacy validators remain unchanged."""
import copy
import os
from pathlib import PurePosixPath
import stat
from . import plan
if __package__.startswith('scripts.'):
    from .. import p0c4_storage_registry as registry
else:
    import p0c4_storage_registry as registry

class SchemaResourceError(plan.CompletionError):pass

def _require(ok,reason):
    if not ok:raise SchemaResourceError(reason)

class _SchemaDirectory:
    def __init__(self,path):
        self.path=str(path);self.fd=registry.open_directory(self.path)
        try:
            self.identity=self._identity(os.fstat(self.fd));self.check()
        except BaseException:
            self.close();raise
    @staticmethod
    def _identity(meta):
        _require(stat.S_ISDIR(meta.st_mode) and meta.st_uid==0 and stat.S_IMODE(meta.st_mode)==0o700,'SCHEMA_DIRECTORY_PRIVATE')
        return meta.st_dev,meta.st_ino
    def check(self):
        _require(self.fd is not None and self._identity(os.fstat(self.fd))==self.identity and self._identity(os.stat(self.path,follow_symlinks=False))==self.identity,'SCHEMA_DIRECTORY_REPLACED')
    def close(self):
        if self.fd is not None:os.close(self.fd);self.fd=None

class _SchemaPgAdapter:
    """Only this case's fixed PG uses this view; no shared-module mutation.

    Validate all original bind/mount facts first. Then omit only the validated
    schema bind from the redundant legacy all-RO Binds view. Original Mounts
    and all other fields still pass through the unchanged legacy validator.
    """
    def __init__(self,legacy,case,volumes):
        self.legacy=legacy;self.case=PurePosixPath(case.as_posix())
        registry.path_text(str(self.case));registry.v4(self.case.name)
        _require(self.case.parent.name.startswith('batch-') and self.case.parent.parent.name=='schema-run','SCHEMA_PROFILE_PATH')
        batch=self.case.parent.name.removeprefix('batch-');registry.v4(batch)
        _require(batch!=self.case.name and set(volumes)=={'volume','source_volume','build_volume'},'SCHEMA_PROFILE_VOLUMES')
        project='kwc4c-'+self.case.name.replace('-','');self.project=project
        self.volume_specs={};self.mounts=set()
        for key,suffix,target,rw in [('volume','pg','/var/lib/postgresql',True),('source_volume','source','/var/lib/knowweave-source',True),('build_volume','build','/target',False)]:
            value=volumes[key];name=project+'-'+suffix;registry.path_text(value['Mountpoint'])
            _require(value['Name']==name and value['Driver']=='local' and not value.get('Options') and value['Labels']=={'com.docker.compose.project':project,'knowweave.source-lifecycle.batch':batch},'SCHEMA_PROFILE_VOLUME_IDENTITY')
            self.volume_specs[target]=(name,rw);self.mounts.add(('volume',value['Mountpoint'],target,rw))
        self.binds={(str(self.case/'initdb.sh'),'/docker-entrypoint-initdb.d/10-lifecycle.sh','ro'),(str(self.case/'schema'),'/var/lib/knowweave-schema','rw')}
        self.binds|={(str(self.case/'secrets'/(role+'_password')),'/run/secrets/'+role+'_password','ro') for role in ('postgres','admin')}
        self.mounts|={('bind',source,target,mode=='rw') for source,target,mode in self.binds}
        self.directory=_SchemaDirectory(self.case/'schema')
    def __enter__(self):return self
    def __exit__(self,*args):self.close()
    def close(self):self.directory.close()
    def validate_container(self,facts,expected):
        self.directory.check()
        _require(expected['builder']is False and expected['name']==self.project+'-pg-1' and expected['network']==self.project+'-net' and expected['mounts']==self.mounts,'SCHEMA_PROFILE_EXPECTED')
        _require(facts['Config'].get('User','')==expected.get('user','')=='' and not facts['Config'].get('Healthcheck'),'SCHEMA_PROFILE_USER_HEALTH')
        binds=facts['HostConfig'].get('Binds')
        _require(type(binds)is list and len(binds)==4 and all(type(v)is str for v in binds),'SCHEMA_PROFILE_BINDS')
        parsed=[tuple(v.split(':')) for v in binds]
        _require(all(len(v)==3 for v in parsed) and set(parsed)==self.binds,'SCHEMA_PROFILE_BINDS')
        mounts=facts['Mounts']
        _require(type(mounts)is list and len(mounts)==7 and all(type(v.get('RW'))is bool for v in mounts) and {(v['Type'],v['Source'],v['Destination'],v['RW']) for v in mounts}==self.mounts,'SCHEMA_PROFILE_MOUNTS')
        for value in mounts:
            if value['Type']=='bind':
                _require(value.get('Mode')==('rw' if value['RW'] else 'ro') and value.get('Propagation')=='rslave','SCHEMA_PROFILE_BIND_OPTIONS')
            else:_require(value.get('Name')==self.volume_specs[value['Destination']][0] and value.get('Mode')=='z' and value.get('Propagation')=='','SCHEMA_PROFILE_VOLUME_IDENTITY')
        declared=facts['HostConfig'].get('Mounts')
        _require(type(declared)is list and len(declared)==3,'SCHEMA_PROFILE_DECLARED_VOLUMES')
        seen=set()
        for value in declared:
            target=value.get('Target');spec=self.volume_specs.get(target)
            _require(spec is not None and target not in seen and set(value)<={'Type','Source','Target','ReadOnly','VolumeOptions'} and value['Type']=='volume' and value['Source']==spec[0] and type(value.get('ReadOnly',False))is bool and value.get('ReadOnly',False)==(not spec[1]) and value.get('VolumeOptions')=={'NoCopy':True} and value['VolumeOptions']['NoCopy']is True,'SCHEMA_PROFILE_DECLARED_VOLUMES');seen.add(target)
        view=copy.deepcopy(facts)
        view['HostConfig']['Binds']=[original for original,parts in zip(binds,parsed) if parts[2]=='ro']
        self.legacy.validate_container(view,expected)

_STAGES=frozenset(('new','preflight','volumes','compile','fixture-config','pg-create','pg-admit','pg-pre-start','pg-start','pg-ready','pg-post-start','migrate','fixed-producer-capture','fixture','body','body-passed','cleanup-source','cleanup-namespace','cleanup-volume','cleanup-pg-reconcile','cleanup-pg-admit','cleanup-pg-validate','cleanup-pg-stop','cleanup-pg-observe','cleanup-helpers','cleanup-raw','cleanup-publish','cleanup-directory'))
_REASONS=frozenset(('PG_BINDS_MODE','PG_BINDS_IDENTITY','PG_COMPOSE_LABELS','PG_CREATED_ID','PG_NETWORK_IDENTITY','PG_NETWORK_ATTACHMENT','PG_READY_TIMEOUT','CONTAINER_IDENTITY','CONTAINER_CONFIG','CONTAINER_SECURITY','CONTAINER_LIMITS','CONTAINER_HOST_ACCESS','CONTAINER_BINDS_SHAPE','CONTAINER_BINDS_FORBIDDEN','CONTAINER_MOUNTS','CONTAINER_NETWORK','CASE_DEADLINE','PROCESS_TIMEOUT','PROCESS_EXIT','PROCESS_LOG_BUDGET','SCHEMA_DIRECTORY_PRIVATE','SCHEMA_DIRECTORY_REPLACED','SCHEMA_PROFILE_PATH','SCHEMA_PROFILE_VOLUMES','SCHEMA_PROFILE_VOLUME_IDENTITY','SCHEMA_PROFILE_EXPECTED','SCHEMA_PROFILE_USER_HEALTH','SCHEMA_PROFILE_BINDS','SCHEMA_PROFILE_MOUNTS','SCHEMA_PROFILE_BIND_OPTIONS','SCHEMA_PROFILE_DECLARED_VOLUMES','EXACT_STOP_NOEXEC_EMPTY_NET'))

def _record_failure(result,phase,stage,error):
    # Never format an exception. Only fixed enum tokens may leave this helper.
    _require(phase in ('primary','cleanup') and stage in _STAGES,'SCHEMA_FAILURE_STAGE')
    name=type(error).__name__;category=name if name in ('GateError','SchemaError','SchemaResourceError','RegistryError') else 'OSError' if isinstance(error,OSError) else 'UnexpectedError'
    args=error.args;reason=args[0] if len(args)==1 and type(args[0])is str and args[0] in _REASONS else 'UNCLASSIFIED'
    value=dict(stage=stage,exception_class=category,reason_code=reason)
    if phase=='primary':result.setdefault('primary_failure',value)
    else:
        rows=result.setdefault('cleanup_failures',[])
        if len(rows)<16:rows.append(value)
        else:result['cleanup_failures_omitted']=min(65535,result.get('cleanup_failures_omitted',0)+1)
    return value
