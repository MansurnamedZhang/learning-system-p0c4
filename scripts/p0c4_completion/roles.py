"""Full restore cluster role recipe. No caller SQL, credentials, or role names.

The privileged provisioner validates package bytes and a fresh target plan before
starting PostgreSQL. This module never runs under learning_admin.
"""
import json


class RoleProvisioningError(RuntimeError):
    pass


def _require(ok, code):
    if not ok:
        raise RoleProvisioningError(code)


def _object(pairs):
    result = {}
    for key, value in pairs:
        _require(key not in result, 'ROLE_DUPLICATE_KEY')
        result[key] = value
    return result


def _validate_recipe(raw):
    _require(type(raw) is bytes and 0 < len(raw) <= 16384, 'ROLE_RECIPE_SIZE')
    try:
        value = json.loads(raw, object_pairs_hook=_object)
    except (ValueError, UnicodeError) as error:
        raise RoleProvisioningError('ROLE_RECIPE_JSON') from error
    _require(type(value) is dict and set(value) == {'format_version', 'roles', 'memberships'}, 'ROLE_RECIPE_KEYS')
    _require(type(value['format_version']) is int and value['format_version'] == 1, 'ROLE_RECIPE_VERSION')
    names = ('learning_admin', 'learning_auth_lock', 'learning_runtime')
    flags = {'login', 'inherit', 'superuser', 'createdb', 'createrole', 'bypassrls', 'replication'}
    _require(type(value['roles']) is list and len(value['roles']) == 3, 'ROLE_RECIPE_COUNT')
    for role, name in zip(value['roles'], names):
        _require(type(role) is dict and set(role) == flags | {'name', 'connection_limit'}, 'ROLE_FIELDS')
        _require(role['name'] == name and all(type(role[key]) is bool for key in flags), 'ROLE_NAME_OR_FLAGS')
        _require(role['login'] == (name != 'learning_auth_lock') and
                 not any(role[key] for key in ('superuser', 'createdb', 'createrole', 'bypassrls', 'replication')), 'ROLE_PRIVILEGES')
        _require(type(role['connection_limit']) is int and -1 <= role['connection_limit'] <= 2147483647, 'ROLE_CONNECTION_LIMIT')
    expected = dict(role='learning_auth_lock', member='learning_admin', inherit=False, set=True, admin=False)
    members = value['memberships']
    _require(type(members) is list and len(members) == 1 and type(members[0]) is dict and
             members[0] == expected and all(type(members[0][key]) is bool for key in ('inherit', 'set', 'admin')), 'ROLE_MEMBERSHIP')
    return value

# Constructors remain private to the validated readers below. Python object
# privacy is not the root trust boundary: installed code and plans are private.
_TOKEN = object()


class VerifiedRoleRecipe:
    __slots__ = ('_raw', '_recipe', '_receipt')

    def __init__(self, token, raw, receipt):
        _require(token is _TOKEN, 'VERIFIED_PACKAGE_READER_REQUIRED')
        self._recipe = _validate_recipe(raw)
        self._raw, self._receipt = raw, receipt


class FreshTargetPlan:
    __slots__ = ('_value', '_path', '_identity')

    def __init__(self, token, value, path, identity):
        _require(token is _TOKEN, 'VERIFIED_TARGET_PLAN_READER_REQUIRED')
        self._value, self._path, self._identity = value, path, identity

    @classmethod
    def read(cls, path):
        import hashlib
        import ipaddress
        import uuid
        from pathlib import Path
        raw, identity = _private_read(path, 16384)
        value = json.loads(raw, object_pairs_hook=_object)
        _require(type(value) is dict and set(value) == {'format_version', 'capability', 'root', 'batch_id', 'subnet', 'initdb', 'backup_id', 'manifest_sha256', 'receipt_sha256'}, 'FULL_TARGET_PLAN_FIELDS')
        _require(type(value['format_version']) is int and value['format_version'] == 1 and value['capability'] == 'full_restore_target_plan_v1', 'FULL_TARGET_PLAN_VERSION')
        for key in ('batch_id', 'backup_id'):
            parsed = uuid.UUID(value[key])
            _require(str(parsed) == value[key] and parsed.version == 4, 'FULL_TARGET_PLAN_UUID')
        _require(value['batch_id'] != value['backup_id'], 'FULL_TARGET_PLAN_DISTINCT')
        for key in ('manifest_sha256', 'receipt_sha256'):
            _require(type(value[key]) is str and len(value[key]) == 64 and all(c in '0123456789abcdef' for c in value[key]), 'FULL_TARGET_PLAN_HASH')
        for key in ('root', 'initdb'):
            text = value[key]
            _require(type(text) is str and text.startswith('/') and not any(c in text for c in ('\\', '\0')) and all(p not in ('', '.', '..') for p in text.split('/')[1:]), 'FULL_TARGET_PLAN_PATH')
        network = ipaddress.IPv4Network(value['subnet'], strict=True)
        _require(str(network) == value['subnet'] and network.prefixlen == 24 and any(network.subnet_of(ipaddress.IPv4Network(n)) for n in ('10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16')), 'FULL_TARGET_PLAN_SUBNET')
        return cls(_TOKEN, value, Path(path), (identity, hashlib.sha256(raw).hexdigest()))


class RoleProvisioningReceipt:
    __slots__ = ('_birth', '_recipe_sha256', '_backup_id')

    def __init__(self, token, birth, recipe, backup_id):
        import hashlib
        _require(token is _TOKEN, 'ROLE_RECEIPT_ISSUER_REQUIRED')
        self._birth, self._recipe_sha256, self._backup_id = birth, hashlib.sha256(recipe).hexdigest(), backup_id

    def observation(self):
        return dict(backup_id=self._backup_id, recipe_sha256=self._recipe_sha256, birth_sha256=self._birth.get('birth_sha256'), state='THREE_ROLES_BORN_QUARANTINED_NOT_RESTORE')


def _private_read(path, cap):
    import os
    import stat
    from pathlib import Path
    _require(os.name == 'posix' and os.geteuid() == 0, 'ROOT_LINUX_ONLY')
    path = Path(path)
    _require(path.is_absolute(), 'PRIVATE_ABSOLUTE_PATH')
    for ancestor in reversed(path.parents):
        meta = os.lstat(ancestor)
        _require(stat.S_ISDIR(meta.st_mode) and meta.st_uid == 0 and meta.st_mode & 0o022 == 0, 'PRIVATE_ANCESTOR')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        meta = os.fstat(fd)
        _require(stat.S_ISREG(meta.st_mode) and meta.st_uid == 0 and meta.st_nlink == 1 and stat.S_IMODE(meta.st_mode) in (0o600, 0o400, 0o444) and 0 < meta.st_size <= cap, 'PRIVATE_FILE')
        raw = b''
        while len(raw) <= cap:
            part = os.read(fd, min(65536, cap + 1 - len(raw)))
            if not part:
                break
            raw += part
        _require(len(raw) == meta.st_size and len(raw) <= cap, 'PRIVATE_FILE_SIZE')
        return raw, (meta.st_dev, meta.st_ino)
    finally:
        os.close(fd)


class _FullRoleProfile:
    __slots__ = ('_recipe', '_plan')

    def __init__(self, token, recipe, plan):
        _require(token is _TOKEN and type(recipe) is VerifiedRoleRecipe and type(plan) is FreshTargetPlan, 'FULL_ROLE_PROFILE_AUTHORITY')
        self._recipe, self._plan = recipe, plan

    def _validate_target(self, root, batch_id, subnet, initdb):
        import hashlib
        from pathlib import Path
        value = self._plan._value
        _require((str(root), batch_id, subnet, str(initdb)) == tuple(value[k] for k in ('root', 'batch_id', 'subnet', 'initdb')), 'FULL_ROLE_TARGET_BINDING')
        raw, identity = _private_read(self._plan._path, 16384)
        _require((identity, hashlib.sha256(raw).hexdigest()) == self._plan._identity, 'FULL_ROLE_PLAN_CHANGED')
        current, _ = _private_read(initdb, 65536)
        embedded = Path(__file__).resolve().parents[2] / 'deploy' / 'p0c4_full_restore_initdb.sh'
        expected, _ = _private_read(embedded, 65536)
        _require(current == expected, 'FULL_ROLE_INITDB_DIFFERS')
        receipt = self._recipe._receipt
        _require(all(receipt[k] == value[k] for k in ('backup_id', 'manifest_sha256', 'receipt_sha256')), 'FULL_ROLE_PACKAGE_BINDING')

    def _docker(self,*args):
        import p0c4_restore_target as target
        return target._docker(*args)

    def _inspect(self,kind,ids):
        import p0c4_restore_target as target
        return target._inspect(kind,ids)

    def _snapshot(self):
        import p0c4_restore_target as target
        return target.snapshot()

    def _postgres_uid(self):
        import p0c4_restore_target as target
        return target._postgres_uid()

    def _extend_target_document(self, document):
        pass

    def _extra_target_mounts(self):
        return {}

    def _validate_extra_target_mounts(self, facts):
        pass

    def _configure(self, identity, container_id):
        import p0c4_restore_target as target
        _require(target.HEX_ID.fullmatch(container_id) is not None, 'FULL_ROLE_CONTAINER_ID')
        # Only safe source flags are variable; every identifier is fixed.
        statements = []
        for role in self._recipe._recipe['roles']:
            inherit = 'INHERIT' if role['inherit'] else 'NOINHERIT'
            statements.append('ALTER ROLE ' + role['name'] + ' ' + inherit + ' CONNECTION LIMIT ' + str(role['connection_limit']) + ';')
        output = target._docker('exec', '--user', 'postgres', container_id, 'psql', '-XqAt', '-v', 'ON_ERROR_STOP=1', '--dbname', identity['database'], '-c', 'BEGIN; ' + ' '.join(statements) + ' COMMIT;')
        _require(output == '', 'FULL_ROLE_CONFIGURE_OUTPUT')

    def _probe(self, identity, container_id):
        import p0c4_restore_target as target
        _require(target.HEX_ID.fullmatch(container_id) is not None, 'FULL_ROLE_CONTAINER_ID')
        output = target._docker('exec', '--user', 'postgres', container_id, 'psql', '-XqAt', '-v', 'ON_ERROR_STOP=1', '--dbname', identity['database'], '-c', _ROLE_PROBE)
        observed = json.loads(output, object_pairs_hook=_object)
        _require(set(observed) == {'recipe', 'database_acl', 'database_owner', 'schema_owner', 'runtime_create', 'runtime_control_system', 'admin_control_system'}, 'FULL_ROLE_PROBE_FIELDS')
        _require(_validate_recipe(json.dumps(observed['recipe']).encode()) == self._recipe._recipe, 'FULL_ROLE_PROBE_RECIPE')
        _require(observed['database_acl'] == '{learning_admin=CTc/learning_admin}' and observed['database_owner'] == 'learning_admin' and observed['schema_owner'] == 'pg_database_owner' and observed['runtime_create'] is False and observed['runtime_control_system'] is False and observed['admin_control_system'] is True, 'FULL_ROLE_PROBE_PRIVILEGES')


def provision_full_restore_roles(verified_recipe, target_plan):
    _require(type(verified_recipe) is VerifiedRoleRecipe and type(target_plan) is FreshTargetPlan, 'FULL_ROLE_PROVISION_AUTHORITY')
    import p0c4_restore_target as target
    from pathlib import Path
    profile = _FullRoleProfile(_TOKEN, verified_recipe, target_plan)
    value = target_plan._value
    result = target._provision(Path(value['root']), value['batch_id'], value['subnet'], Path(value['initdb']), _full_profile=profile)
    return RoleProvisioningReceipt(_TOKEN, result, verified_recipe._raw, value['backup_id'])

_ROLE_PROBE = "SELECT json_build_object('recipe',(SELECT jsonb_build_object('format_version',1,'roles',(SELECT jsonb_agg(jsonb_build_object('name',rolname::text,'login',rolcanlogin,'inherit',rolinherit,'superuser',rolsuper,'createdb',rolcreatedb,'createrole',rolcreaterole,'bypassrls',rolbypassrls,'replication',rolreplication,'connection_limit',rolconnlimit) ORDER BY rolname) FROM pg_catalog.pg_roles WHERE rolname IN ('learning_admin','learning_auth_lock','learning_runtime')),'memberships',(SELECT jsonb_agg(jsonb_build_object('role',r.rolname::text,'member',m.rolname::text,'inherit',a.inherit_option,'set',a.set_option,'admin',a.admin_option) ORDER BY r.rolname,m.rolname) FROM pg_catalog.pg_auth_members a JOIN pg_catalog.pg_roles r ON r.oid=a.roleid JOIN pg_catalog.pg_roles m ON m.oid=a.member WHERE r.rolname IN ('learning_admin','learning_auth_lock','learning_runtime') OR m.rolname IN ('learning_admin','learning_auth_lock','learning_runtime')))),'database_acl',(SELECT datacl::text FROM pg_database WHERE datname=current_database()),'database_owner',(SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database()),'schema_owner',(SELECT pg_get_userbyid(nspowner) FROM pg_namespace WHERE nspname='public'),'runtime_create',has_schema_privilege('learning_runtime','public','CREATE'),'runtime_control_system',has_function_privilege('learning_runtime','pg_catalog.pg_control_system()','EXECUTE'),'admin_control_system',has_function_privilege('learning_admin','pg_catalog.pg_control_system()','EXECUTE'));"

_READER = '/var/lib/knowweave-c4/tools/knowweave-c4-full-roles'
_INSTALLATION = '/var/lib/knowweave-c4/tools/full-restore-installation.json'


def read_complete_role_recipe(backup_id):
    """Only a verified signed package with destination-owned proof can enter.

    Missing fixed installation metadata or executable fails closed. It never
    falls back to a caller recipe, historical pin, or destination signing CLI.
    """
    import hashlib
    import os
    import selectors
    import stat
    import subprocess
    import time
    import uuid
    from pathlib import Path
    parsed = uuid.UUID(backup_id)
    _require(str(parsed) == backup_id and parsed.version == 4, 'ROLE_BACKUP_UUID')
    installation, _ = _private_read(_INSTALLATION, 4096)
    installed = json.loads(installation, object_pairs_hook=_object)
    _require(type(installed) is dict and set(installed) == {'format_version', 'reader_sha256', 'application_build_sha256'} and type(installed['format_version']) is int and installed['format_version'] == 1, 'ROLE_INSTALLATION')
    for key in ('reader_sha256', 'application_build_sha256'):
        _require(type(installed[key]) is str and len(installed[key]) == 64 and all(c in '0123456789abcdef' for c in installed[key]), 'ROLE_INSTALLATION_HASH')
    for ancestor in reversed(Path(_READER).parents):
        meta = os.lstat(ancestor)
        _require(stat.S_ISDIR(meta.st_mode) and meta.st_uid == 0 and not meta.st_mode & 0o022, 'ROLE_READER_ANCESTOR')
    fd = os.open(_READER, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    child = None
    try:
        meta = os.fstat(fd)
        _require(stat.S_ISREG(meta.st_mode) and meta.st_uid == 0 and meta.st_nlink == 1 and stat.S_IMODE(meta.st_mode) in (0o500, 0o555, 0o755) and 0 < meta.st_size <= 256 * 1024 * 1024, 'ROLE_READER_FILE')
        digest = hashlib.sha256()
        size = 0
        while True:
            chunk = os.read(fd, 65536)
            if not chunk:
                break
            size += len(chunk)
            _require(size <= meta.st_size, 'ROLE_READER_CHANGED')
            digest.update(chunk)
        _require(size == meta.st_size and digest.hexdigest() == installed['reader_sha256'], 'ROLE_READER_HASH')
        os.lseek(fd, 0, os.SEEK_SET)
        child = subprocess.Popen(['/proc/self/fd/' + str(fd), backup_id], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, pass_fds=(fd,), env={'LC_ALL': 'C'}, close_fds=True)
        deadline = time.monotonic() + 900
        outputs = [bytearray(), bytearray()]
        with selectors.DefaultSelector() as selector:
            selector.register(child.stdout, selectors.EVENT_READ, 0)
            selector.register(child.stderr, selectors.EVENT_READ, 1)
            while selector.get_map():
                _require(time.monotonic() < deadline, 'ROLE_READER_TIMEOUT')
                for key, _ in selector.select(timeout=min(1, max(0, deadline-time.monotonic()))):
                    chunk = os.read(key.fd, 4096)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    outputs[key.data].extend(chunk)
                    _require(len(outputs[key.data]) <= (16384 if key.data == 0 else 8192), 'ROLE_READER_OUTPUT_CAP')
        _require(child.wait(timeout=max(0.01, deadline-time.monotonic())) == 0 and not outputs[1], 'ROLE_READER_REFUSED')
        record = json.loads(bytes(outputs[0]), object_pairs_hook=_object)
        _require(type(record) is dict and set(record) == {'format_version', 'capability', 'backup_id', 'manifest_sha256', 'receipt_sha256', 'proof_manifest_sha256', 'application_build_sha256', 'roles'}, 'ROLE_READER_FIELDS')
        _require(type(record['format_version']) is int and record['format_version'] == 1 and record['capability'] == 'complete_role_recipe_v1' and record['backup_id'] == backup_id and record['proof_manifest_sha256'] == record['manifest_sha256'] and record['application_build_sha256'] == installed['application_build_sha256'], 'ROLE_READER_BINDING')
        for key in ('manifest_sha256', 'receipt_sha256'):
            _require(type(record[key]) is str and len(record[key]) == 64 and all(c in '0123456789abcdef' for c in record[key]), 'ROLE_READER_DIGEST')
        raw = json.dumps(record['roles'], separators=(',', ':')).encode()
        return VerifiedRoleRecipe(_TOKEN, raw, record)
    finally:
        if child is not None:
            if child.poll() is None:
                child.kill()
            child.wait()
            child.stdout.close()
            child.stderr.close()
        os.close(fd)
