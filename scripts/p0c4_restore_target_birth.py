#!/usr/bin/env python3
"""Opt-in birth issuance during fresh C4 PG18 creation, under its root lock.

The ordinary provisioner does not issue birth. This issuer does not issue a
CompleteBackup, restore a package, or admit a runtime. Its digest is an input
to a later separately reviewed build pin, never an acceptance decision.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import uuid

import p0c4_restore_target as target_provisioner


AdmissionError = target_provisioner.AdmissionError
require = target_provisioner.require

ISSUER_FAILURE_REASONS = {
    "CREATION_STATE": "CREATION_STATE_REJECTED",
    "DOCKER_REINSPECTION": "DOCKER_REINSPECTION_FAILED",
    "TRUSTED_VOLUME_PATH": "TRUSTED_VOLUME_PATH_FAILED",
    "INITDB_RECHECK": "INITDB_RECHECK_FAILED",
    "PG_SQL_EXECUTION": "PG_SQL_EXECUTION_FAILED",
    "PG_SQL_PARSE": "PG_SQL_PARSE_FAILED",
    "PG_FACTS_VALIDATION": "PG_FACTS_VALIDATION_FAILED",
    "PRIVATE_ROOT_CREATION": "PRIVATE_ROOT_CREATION_FAILED",
    "BIRTH_PUBLICATION": "BIRTH_PUBLICATION_FAILED",
}
SAFE_EXCEPTION_CLASSES = {
    "AdmissionError", "OSError", "PermissionError", "FileNotFoundError",
    "ValueError", "TypeError", "KeyError", "RuntimeError", "JSONDecodeError",
    "TimeoutExpired", "KeyboardInterrupt", "InterruptedError",
}


def _write_failure_diagnostic(target, identity, status, phase, error):
    """Best-effort durable private codes; never serialize the raw exception."""
    if os.path.lexists(target / "failure.json"):
        return
    try:
        target_provisioner._trusted_root(target)
        exception_class = type(error).__name__
        diagnostic = {
            "format_version": 1,
            "state": "BIRTH_ISSUER_DIAGNOSTIC_NOT_ACCEPTANCE",
            "batch_id": status["batch_id"],
            "project": identity["project"],
            "phase": phase,
            "reason_code": ISSUER_FAILURE_REASONS[phase],
            "exception_class": (exception_class if exception_class in
                                SAFE_EXCEPTION_CLASSES else "OtherError"),
        }
        target_provisioner._private_write(
            target / "issuer-diagnostic.json",
            json.dumps(diagnostic, ensure_ascii=False,
                       separators=(",", ":")).encode("utf-8"))
    except BaseException:
        # A diagnostic failure cannot turn a failed birth into acceptance.
        pass

DIRTY_QUERIES = {
    "relations": "SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%'",
    "schemas": "SELECT count(*) FROM pg_catalog.pg_namespace n WHERE n.nspname NOT IN ('pg_catalog','information_schema','public') AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%'",
    "routines": "SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public'",
    "types": "SELECT count(*) FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='public'",
    "extensions": "SELECT count(*) FROM pg_catalog.pg_extension WHERE extname<>'plpgsql'",
    "event_triggers": "SELECT count(*) FROM pg_catalog.pg_event_trigger",
    "publications": "SELECT count(*) FROM pg_catalog.pg_publication",
    "large_objects": "SELECT count(*) FROM pg_catalog.pg_largeobject_metadata",
    "collations": "SELECT count(*) FROM pg_catalog.pg_collation c JOIN pg_catalog.pg_namespace n ON n.oid=c.collnamespace WHERE n.nspname='public'",
    "conversions": "SELECT count(*) FROM pg_catalog.pg_conversion c JOIN pg_catalog.pg_namespace n ON n.oid=c.connamespace WHERE n.nspname='public'",
    "operators": "SELECT count(*) FROM pg_catalog.pg_operator o JOIN pg_catalog.pg_namespace n ON n.oid=o.oprnamespace WHERE n.nspname='public'",
    "operator_classes": "SELECT count(*) FROM pg_catalog.pg_opclass o JOIN pg_catalog.pg_namespace n ON n.oid=o.opcnamespace WHERE n.nspname='public'",
    "operator_families": "SELECT count(*) FROM pg_catalog.pg_opfamily o JOIN pg_catalog.pg_namespace n ON n.oid=o.opfnamespace WHERE n.nspname='public'",
    "text_search_objects": "SELECT (SELECT count(*) FROM pg_catalog.pg_ts_config t JOIN pg_catalog.pg_namespace n ON n.oid=t.cfgnamespace WHERE n.nspname='public') + (SELECT count(*) FROM pg_catalog.pg_ts_dict t JOIN pg_catalog.pg_namespace n ON n.oid=t.dictnamespace WHERE n.nspname='public') + (SELECT count(*) FROM pg_catalog.pg_ts_parser t JOIN pg_catalog.pg_namespace n ON n.oid=t.prsnamespace WHERE n.nspname='public') + (SELECT count(*) FROM pg_catalog.pg_ts_template t JOIN pg_catalog.pg_namespace n ON n.oid=t.tmplnamespace WHERE n.nspname='public')",
    "default_acls": "SELECT count(*) FROM pg_catalog.pg_default_acl",
    "foreign_objects": "SELECT (SELECT count(*) FROM pg_catalog.pg_foreign_data_wrapper) + (SELECT count(*) FROM pg_catalog.pg_foreign_server)",
    "custom_languages": "SELECT count(*) FROM pg_catalog.pg_language WHERE lanname NOT IN ('internal','c','sql','plpgsql')",
    "custom_access_methods": "SELECT count(*) FROM pg_catalog.pg_am WHERE amname NOT IN ('heap','btree','hash','gist','gin','spgist','brin')",
    "global_ddl": "SELECT (SELECT count(*) FROM pg_catalog.pg_transform) + (SELECT count(*) FROM pg_catalog.pg_parameter_acl) + (SELECT count(*) FROM pg_catalog.pg_subscription) + (SELECT count(*) FROM pg_catalog.pg_db_role_setting) + (SELECT count(*) FROM pg_catalog.pg_tablespace WHERE spcname NOT IN ('pg_default','pg_global')) + (SELECT count(*) FROM pg_catalog.pg_database WHERE datname NOT IN ('template0','template1','postgres',pg_catalog.current_database()))",
}


def _pg_sql():
    dirty = ",".join("'%s',(%s)" % (name, query) for name, query in DIRTY_QUERIES.items())
    return f"""SELECT pg_catalog.json_build_object(
      'database_oid',d.oid::bigint,
      'pg_system_identifier',pcs.system_identifier::text,
      'cast_count',(SELECT count(*) FROM pg_catalog.pg_cast),
      'public_schema',pg_catalog.json_build_object(
        'owner_oid',n.nspowner::bigint,
        'owner_name',pg_catalog.pg_get_userbyid(n.nspowner)::text,
        'acl_is_null',n.nspacl IS NULL,
        'acl',(SELECT COALESCE(pg_catalog.json_agg(pg_catalog.json_build_object(
          'grantor_oid',a.grantor::bigint,'grantee_oid',a.grantee::bigint,
          'privilege',a.privilege_type::text,'grantable',a.is_grantable)), '[]'::json)
          FROM pg_catalog.aclexplode(COALESCE(n.nspacl,
               pg_catalog.acldefault('n',n.nspowner))) a)),
      'runtime_can_create_public',pg_catalog.has_schema_privilege(
         'learning_runtime',n.oid,'CREATE'),
      'other_sessions',(SELECT count(*) FROM pg_catalog.pg_stat_activity
         WHERE datname=pg_catalog.current_database() AND pid<>pg_catalog.pg_backend_pid()),
      'other_roles',(SELECT count(*) FROM pg_catalog.pg_roles
         WHERE left(rolname,3)<>'pg_' AND rolname NOT IN
           ('postgres','learning_admin','learning_runtime')),
      'app_role_memberships',(SELECT count(*) FROM pg_catalog.pg_auth_members m
         JOIN pg_catalog.pg_roles r ON r.oid=m.member
         JOIN pg_catalog.pg_roles g ON g.oid=m.roleid
         WHERE r.rolname IN ('learning_admin','learning_runtime')
            OR g.rolname IN ('learning_admin','learning_runtime')),
      'public_database_grants',(SELECT count(*) FROM pg_catalog.aclexplode(
         COALESCE(d.datacl,pg_catalog.acldefault('d',d.datdba))) a
         WHERE a.grantee=0),
      'nonowner_database_grants',(SELECT count(*) FROM pg_catalog.aclexplode(
         COALESCE(d.datacl,pg_catalog.acldefault('d',d.datdba))) a
         WHERE a.grantee<>0 AND a.grantee<>d.datdba),
      'runtime_can_use_database',(
         pg_catalog.has_database_privilege('learning_runtime',d.oid,'CONNECT')
         OR pg_catalog.has_database_privilege('learning_runtime',d.oid,'CREATE')
         OR pg_catalog.has_database_privilege('learning_runtime',d.oid,'TEMP')),
      'role_flags_secure',(
         pg_catalog.pg_get_userbyid(d.datdba)='learning_admin'
         AND EXISTS(SELECT 1 FROM pg_catalog.pg_roles a WHERE
            a.rolname='learning_admin' AND a.rolcanlogin AND NOT a.rolsuper
            AND NOT a.rolcreatedb AND NOT a.rolcreaterole
            AND NOT a.rolbypassrls AND NOT a.rolreplication)
         AND EXISTS(SELECT 1 FROM pg_catalog.pg_roles r WHERE
            r.rolname='learning_runtime' AND NOT r.rolcanlogin
            AND NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole
            AND NOT r.rolbypassrls AND NOT r.rolreplication)),
      'dirty_counts',pg_catalog.json_build_object({dirty}))
    FROM pg_catalog.pg_database d CROSS JOIN pg_catalog.pg_control_system() pcs
    JOIN pg_catalog.pg_namespace n ON n.nspname='public'
    WHERE d.datname=pg_catalog.current_database();"""


def _full_pg_sql():
    # Fixed three-role policy, never caller role SQL. Legacy _pg_sql stays byte
    # for byte and the same catalog/birth validator still requires zero extras.
    query=_pg_sql()
    before="('postgres','learning_admin','learning_runtime')"
    require(query.count(before)==1,"full birth role template")
    query=query.replace(before,"('postgres','learning_admin','learning_runtime','learning_auth_lock')")
    before="WHERE r.rolname IN ('learning_admin','learning_runtime')\n            OR g.rolname IN ('learning_admin','learning_runtime'))"
    after="WHERE (r.rolname IN ('learning_admin','learning_runtime','learning_auth_lock') OR g.rolname IN ('learning_admin','learning_runtime','learning_auth_lock')) AND NOT (r.rolname='learning_admin' AND g.rolname='learning_auth_lock' AND NOT m.inherit_option AND m.set_option AND NOT m.admin_option AND pg_catalog.pg_get_userbyid(m.grantor)='postgres'))"
    require(query.count(before)==1,"full birth membership template")
    query=query.replace(before,after)
    before="r.rolname='learning_runtime' AND NOT r.rolcanlogin"
    require(query.count(before)==1,"full birth runtime template")
    query=query.replace(before,"r.rolname='learning_runtime' AND r.rolcanlogin")
    before="AND NOT r.rolbypassrls AND NOT r.rolreplication)),"
    after="AND NOT r.rolbypassrls AND NOT r.rolreplication) AND EXISTS(SELECT 1 FROM pg_catalog.pg_roles l WHERE l.rolname='learning_auth_lock' AND NOT l.rolcanlogin AND NOT l.rolsuper AND NOT l.rolcreatedb AND NOT l.rolcreaterole AND NOT l.rolbypassrls AND NOT l.rolreplication) AND NOT pg_catalog.has_schema_privilege('learning_auth_lock','public','CREATE')),"
    require(query.count(before)==1,"full birth auth role template")
    return query.replace(before,after)


def probe_pg_facts(identity, container_id, *, _phase=None, _full_profile=None, _controlled_profile=None):
    if _controlled_profile is not None:
        from p0c4_completion.controlled_fixture import ControlledFixtureContext
        require(type(_controlled_profile) is ControlledFixtureContext and _full_profile is None,"fixed two-role controlled context required")
        _controlled_profile._require_current_container(identity,container_id)
    require(bool(target_provisioner.HEX_ID.fullmatch(container_id)),
            "verified PG container required")
    if _phase is not None:
        _phase("PG_SQL_EXECUTION")
    execute=_controlled_profile._docker if _controlled_profile is not None else target_provisioner._docker if _full_profile is None else _full_profile._docker
    output = execute(
        "exec", "--user", "postgres", container_id, "psql", "-XAt",
        "-v", "ON_ERROR_STOP=1", "--dbname", identity["database"],
        "-c", _pg_sql() if _full_profile is None else _full_pg_sql())
    if _phase is not None:
        _phase("PG_SQL_PARSE")
    require(output.endswith("\n") and len(output.splitlines()) == 1,
            "target PG metadata shape changed")
    try:
        facts = json.loads(output)
    except json.JSONDecodeError as error:
        raise AdmissionError("target PG metadata is not JSON") from error
    if _phase is not None:
        _phase("PG_FACTS_VALIDATION")
    validate_pg_facts(facts)
    return facts


def _positive_int(value):
    return type(value) is int and 0 < value <= 2**64 - 1


def validate_pg_facts(facts):
    require(type(facts) is dict and set(facts) == {
        "database_oid", "pg_system_identifier", "cast_count", "public_schema",
        "runtime_can_create_public", "other_sessions", "other_roles",
        "app_role_memberships", "public_database_grants",
        "nonowner_database_grants", "runtime_can_use_database",
        "role_flags_secure", "dirty_counts"},
            "target PG fact shape differs")
    require(_positive_int(facts["database_oid"]) and
            _positive_int(facts["cast_count"]), "target PG identity invalid")
    system_id = facts["pg_system_identifier"]
    require(type(system_id) is str and re.fullmatch(r"[1-9][0-9]*", system_id) and
            int(system_id) <= 2**64 - 1, "target PG system identifier invalid")
    require(facts["runtime_can_create_public"] is False and
            facts["runtime_can_use_database"] is False and
            facts["role_flags_secure"] is True and
            all(type(facts[key]) is int and facts[key] == 0 for key in
                ("other_sessions", "other_roles", "app_role_memberships",
                 "public_database_grants", "nonowner_database_grants")),
            "target PG roles, sessions, or public CREATE differ")
    counts = facts["dirty_counts"]
    require(type(counts) is dict and set(counts) == set(DIRTY_QUERIES) and
            all(type(value) is int and value == 0 for value in counts.values()),
            "target contains non-baseline catalog objects")
    public = facts["public_schema"]
    require(type(public) is dict and set(public) ==
            {"owner_oid", "owner_name", "acl_is_null", "acl"} and
            _positive_int(public["owner_oid"]) and
            public["owner_name"] == "pg_database_owner" and
            public["acl_is_null"] is False and type(public["acl"]) is list,
            "target public schema identity differs")
    owner = public["owner_oid"]
    expected = {(owner, owner, "CREATE", False),
                (owner, owner, "USAGE", False),
                (owner, 0, "USAGE", False)}
    observed = set()
    for entry in public["acl"]:
        require(type(entry) is dict and set(entry) ==
                {"grantor_oid", "grantee_oid", "privilege", "grantable"} and
                _positive_int(entry["grantor_oid"]) and
                type(entry["grantee_oid"]) is int and
                0 <= entry["grantee_oid"] <= 2**64 - 1 and
                type(entry["privilege"]) is str and
                type(entry["grantable"]) is bool, "target schema ACL shape differs")
        observed.add((entry["grantor_oid"], entry["grantee_oid"],
                      entry["privilege"], entry["grantable"]))
    require(len(public["acl"]) == len(expected) and observed == expected,
            "target public schema ACL differs from fresh bootstrap")


def canonical_birth(identity, facts, control_id, asset_id, nonce):
    validate_pg_facts(facts)
    require(identity == target_provisioner.identity_for(
        identity["database"].removeprefix("learning_restore_c4_")),
        "target birth identity differs")
    require(all(_positive_int(item) for item in (*control_id, *asset_id)),
            "private root identity invalid")
    parsed = uuid.UUID(nonce)
    require(parsed.version == 4 and str(parsed) == nonce, "birth nonce invalid")
    public = facts["public_schema"]
    acl = [{"grantor_oid": entry["grantor_oid"],
            "grantee_oid": entry["grantee_oid"],
            "privilege": entry["privilege"], "grantable": entry["grantable"]}
           for entry in sorted(public["acl"], key=lambda row: (
               row["grantor_oid"], row["grantee_oid"], row["privilege"],
               row["grantable"]))]
    birth = {
        "format_version": 1,
        "project_name": identity["project"],
        "pg_volume_name": identity["volume"],
        "database_name": identity["database"],
        "database_oid": facts["database_oid"],
        "pg_system_identifier": facts["pg_system_identifier"],
        "control_dev": control_id[0], "control_ino": control_id[1],
        "asset_dev": asset_id[0], "asset_ino": asset_id[1],
        "creation_nonce": nonce,
        "template_database": "template0",
        "baseline_cast_count": facts["cast_count"],
        "public_schema": {
            "owner_oid": public["owner_oid"],
            "owner_name": public["owner_name"],
            "acl_is_null": public["acl_is_null"],
            "acl": acl,
        },
    }
    payload = json.dumps(birth, ensure_ascii=False,
                         separators=(",", ":")).encode("utf-8")
    require(len(payload) <= 4096, "birth attestation exceeds verifier limit")
    return payload


def verify_birth_docker(identity, subnet, before, after, original_ids,
                        target, initdb, _full_profile=None):
    ids = target_provisioner.verify_created(identity, subnet, before, after)
    require(ids == original_ids, "Docker immutable object changed before birth")
    volume = next(v for v in after["volumes"] if v.get("Name") == identity["volume"])
    require(volume.get("Driver") == "local" and volume.get("Scope") == "local" and
            volume.get("Options") in (None, {}), "PG volume driver or options differ")
    pg = next(c for c in after["containers"] if c.get("Id") == ids["container_id"])
    expected = {
        "/var/lib/postgresql": ("volume", ids["volume_mountpoint"], True),
        "/docker-entrypoint-initdb.d/10-restore.sh": ("bind", str(initdb), False),
        "/run/secrets/postgres_password": (
            "bind", str(target / "secrets" / "postgres_password"), False),
        "/run/secrets/admin_password": (
            "bind", str(target / "secrets" / "admin_password"), False),
    }
    if _full_profile is not None:
        expected.update(_full_profile._extra_target_mounts())
        _full_profile._validate_extra_target_mounts(pg)
    mounts = pg.get("Mounts") or []
    require(len(mounts) == len(expected) and
            {m.get("Destination") for m in mounts} == set(expected),
            "PG container has an unexpected mount")
    for mount in mounts:
        kind, source, writable = expected[mount["Destination"]]
        require((mount.get("Type"), mount.get("Source"), mount.get("RW")) ==
                (kind, source, writable), "PG mount identity differs")
    return ids


def _trusted_volume_mount(path):
    mount = Path(path)
    require(mount.is_absolute(), "PG volume mount path is not absolute")
    for ancestor in reversed((mount, *mount.parents)):
        meta = os.lstat(ancestor)
        require(stat.S_ISDIR(meta.st_mode) and not meta.st_mode & 0o022,
                "PG volume mount has an unsafe ancestor")
        if ancestor != mount:
            require(meta.st_uid == 0, "PG volume ancestor is not root-owned")
    meta = os.lstat(mount)
    require(meta.st_dev > 0 and meta.st_ino > 0, "PG volume identity unavailable")
    return meta.st_dev, meta.st_ino


def _new_private_root(path):
    require(not os.path.lexists(path), "private restore root already exists")
    path.mkdir(mode=0o700)
    target_provisioner._sync_directory(path.parent)
    target_provisioner._trusted_root(path)
    require(not any(path.iterdir()), "new private restore root is not empty")
    meta = os.lstat(path)
    return meta.st_dev, meta.st_ino


def _verify_birth_file(path):
    meta = os.lstat(path)
    require(stat.S_ISREG(meta.st_mode) and meta.st_uid == 0 and
            stat.S_IMODE(meta.st_mode) == 0o600 and meta.st_nlink == 1,
            "published birth permissions differ")


def _read_published_birth(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        meta = os.fstat(descriptor)
        require(stat.S_ISREG(meta.st_mode) and meta.st_uid == 0 and
                stat.S_IMODE(meta.st_mode) == 0o600 and meta.st_nlink == 1,
                "published birth cannot be re-opened safely")
        with os.fdopen(descriptor, "rb", closefd=False) as file:
            return file.read(4097)
    finally:
        os.close(descriptor)


def _publish_birth(control, name, payload, nonce):
    final = control / name
    temp = control / ("." + name + "." + nonce + ".tmp")
    require(not os.path.lexists(final), "target birth already exists")
    target_provisioner._private_write(temp, payload)
    linked = False
    try:
        # Hard link is atomic and fails if final exists. Rust requires nlink=1,
        # so a crash between link and unlink remains inadmissible.
        os.link(temp, final, follow_symlinks=False)
        linked = True
        os.unlink(temp)
        target_provisioner._sync_directory(control)
        _verify_birth_file(final)
    except BaseException:
        if linked:
            try:
                os.unlink(final)
                target_provisioner._sync_directory(control)
            except OSError:
                # Caller quarantines and writes failure.json. Never report a
                # digest on an uncertain durable publication.
                pass
        if os.path.lexists(temp):
            os.unlink(temp)
        raise
    return hashlib.sha256(payload).hexdigest()


def finalize_birth_issuance(target, control, identity, status, ids, image_id,
                            volume_mount_id, payload, nonce):
    """Only the final success seal makes a published birth a candidate."""
    birth_name = identity["database"] + ".birth.json"
    birth_sha256 = _publish_birth(control, birth_name, payload, nonce)
    require(_read_published_birth(control / birth_name) == payload,
            "published birth differs on re-open")
    success = {
        "format_version": 1,
        "state": "BIRTH_ISSUED_NOT_RESTORE_ACCEPTANCE",
        "batch_id": status["batch_id"],
        "project_name": identity["project"],
        "database_name": identity["database"],
        "birth_sha256": birth_sha256,
        "container_id": ids["container_id"],
        "network_id": ids["network_id"],
        "pg_volume_name": ids["volume_name"],
        "image_id": image_id,
        "volume_mountpoint": ids["volume_mountpoint"],
        "volume_mount_dev": volume_mount_id[0],
        "volume_mount_ino": volume_mount_id[1],
    }
    success_bytes = json.dumps(success, ensure_ascii=False,
                               separators=(",", ":")).encode("utf-8")
    success_sha256 = _publish_birth(target, "issuance-success.json",
                                    success_bytes, nonce)
    return birth_sha256, success_sha256


def _verify_creation_state(root, target, identity, status):
    require(status.get("state") == "CREATED_QUARANTINED" and
            status.get("project") == identity["project"] and
            status.get("database") == identity["database"] and
            status.get("volume_name") == identity["volume"] and
            target == root / "targets" / status["batch_id"] and
            not os.path.lexists(target / "failure.json"),
            "target is not a fresh quarantined creation")
    target_provisioner._trusted_root(target)
    state_file = target / "state.json"
    meta = os.lstat(state_file)
    require(stat.S_ISREG(meta.st_mode) and meta.st_uid == 0 and
            stat.S_IMODE(meta.st_mode) == 0o600 and meta.st_nlink == 1 and
            json.loads(state_file.read_bytes()) == status,
            "target creation record differs")


def _full_files_profile(profile):
    if profile is None:return False
    from p0c4_completion.full_import import FullRehearsalContext
    return type(profile) is FullRehearsalContext

def _controlled_files_profile(profile):
    if profile is None:return False
    from p0c4_completion.controlled_fixture import ControlledFixtureContext
    require(type(profile) is ControlledFixtureContext,"fixed two-role controlled context required")
    return True


def _issue_birth_inner(root, target, identity, subnet, before, ids, status,
                       initdb, phase, _full_profile=None, _controlled_profile=None):
    """Called only inside provisioner's creation lock, after fresh creation."""
    phase("CREATION_STATE")
    full_files=_full_files_profile(_full_profile)
    controlled=_controlled_files_profile(_controlled_profile)
    require(not controlled or _full_profile is None,"disjoint fixed role recipes required")
    profile=_controlled_profile if controlled else _full_profile
    owned_files=full_files or controlled
    if owned_files:profile._verify_target_files(status)
    else:_verify_creation_state(root, target, identity, status)
    phase("DOCKER_REINSPECTION")
    roots = [target / name for name in ("destination", "control", "assets")]
    if not owned_files:
        require(all(not os.path.lexists(path) for path in roots),"restore roots must be new and disjoint")
    after = target_provisioner.snapshot() if profile is None else profile._snapshot()
    after["images"] = target_provisioner._inspect("image", [identity["image"]]) if profile is None else profile._inspect("image",[identity["image"]])
    verify_birth_docker(identity, subnet, before, after, ids, target, initdb, _full_profile)
    exact = [item for item in after["containers"] if
             item.get("Id") == ids["container_id"]]
    require(len(exact) == 1 and
            exact[0].get("State", {}).get("Running") is True and
            type(exact[0]["State"].get("StartedAt")) is str and
            re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:"
                         r"[0-9]{2}(?:\.[0-9]{1,9})?Z",
                         exact[0]["State"]["StartedAt"]) and
            not exact[0]["State"]["StartedAt"].startswith("0001-"),
            "verified birth Docker start time unavailable")
    container_started_at = exact[0]["State"]["StartedAt"]
    phase("TRUSTED_VOLUME_PATH")
    mount_dev,mount_ino=(profile._measure_birth_pg(ids) if owned_files else _trusted_volume_mount(ids["volume_mountpoint"]))
    phase("INITDB_RECHECK")
    if profile is None:
        target_provisioner.probe_initdb(identity, ids["container_id"])
    else:
        profile._probe(identity, ids["container_id"])
    facts = probe_pg_facts(identity, ids["container_id"], _phase=phase, _full_profile=_full_profile,_controlled_profile=_controlled_profile)
    phase("PRIVATE_ROOT_CREATION")
    if owned_files:
        return profile._publish_target_birth(status,ids,after['images'][0]['Id'],container_started_at,(mount_dev,mount_ino),facts)
    return _publish_birth_files(root,target,identity,status,ids,after['images'][0]['Id'],container_started_at,(mount_dev,mount_ino),facts,after['daemon_id'],_phase=phase)


def _publish_birth_files(root,target,identity,status,ids,image_id,container_started_at,volume_id,facts,daemon_id,*,_phase=None):
    _verify_creation_state(root,target,identity,status)
    roots=[target/name for name in ('destination','control','assets')]
    require(all(not os.path.lexists(path) for path in roots),'restore roots must be new and disjoint')
    mount_dev,mount_ino=volume_id
    root_ids = [_new_private_root(path) for path in roots]
    destination, control, assets = roots
    require(not any(destination.iterdir()) and not any(control.iterdir()) and
            not any(assets.iterdir()), "new restore roots are not empty")
    if _phase is not None:_phase("BIRTH_PUBLICATION")
    nonce = str(uuid.uuid4())
    payload = canonical_birth(identity, facts, root_ids[1], root_ids[2], nonce)
    evidence = {
        "state": "BIRTH_SOURCE_REINSPECTED_NOT_RESTORE_ACCEPTANCE",
        "project": identity["project"], "batch_id": status["batch_id"],
        "docker_daemon_id": daemon_id,
        "container_id": ids["container_id"],
        "container_started_at": container_started_at,
        "network_id": ids["network_id"],
        "volume_name": ids["volume_name"],
        "volume_mountpoint": ids["volume_mountpoint"],
        "volume_mount_dev": mount_dev, "volume_mount_ino": mount_ino,
        "image_id": image_id,
        "destination_dev": root_ids[0][0], "destination_ino": root_ids[0][1],
        "control_dev": root_ids[1][0], "control_ino": root_ids[1][1],
        "asset_dev": root_ids[2][0], "asset_ino": root_ids[2][1],
        "birth_sha256": hashlib.sha256(payload).hexdigest(),
    }
    target_provisioner._private_write(
        target / "birth-evidence.json",
        json.dumps(evidence, sort_keys=True, separators=(",", ":")).encode())
    digest, success_digest = finalize_birth_issuance(
        target, control, identity, status, ids, image_id,
        (mount_dev, mount_ino), payload, nonce)
    name = identity["database"] + ".birth.json"
    return {"birth_sha256": digest, "issuance_sha256": success_digest,
            "birth_path": str(control / name),
            "destination_root": str(destination), "control_root": str(control),
            "asset_root": str(assets)}


def issue_birth(root, target, identity, subnet, before, ids, status, initdb):
    return _issue_birth(root,target,identity,subnet,before,ids,status,initdb)


def _issue_full_birth(root,target,identity,subnet,before,ids,status,initdb,profile):
    from p0c4_completion.roles import _FullRoleProfile
    from p0c4_completion.full_import import FullRehearsalContext
    require(type(profile) in (_FullRoleProfile,FullRehearsalContext), "fixed full restore profile required")
    profile._validate_target(root,status["batch_id"],subnet,initdb)
    return _issue_birth(root,target,identity,subnet,before,ids,status,initdb,profile)

def _issue_controlled_birth(root,target,identity,subnet,before,ids,status,initdb,profile):
    require(_controlled_files_profile(profile),"fixed controlled fixture context required")
    profile._validate_target(root,status['batch_id'],subnet,initdb)
    profile._require_current_container(identity,ids['container_id'])
    return _issue_birth(root,target,identity,subnet,before,ids,status,initdb,_controlled_profile=profile)


def _issue_birth(root, target, identity, subnet, before, ids, status, initdb, _full_profile=None, *, _controlled_profile=None):
    """Failed issuer work emits only bounded diagnostic codes, never raw data."""
    current_phase = "CREATION_STATE"

    def set_phase(next_phase):
        nonlocal current_phase
        current_phase = next_phase

    try:
        return _issue_birth_inner(root, target, identity, subnet, before, ids,
                                  status, initdb, set_phase, _full_profile,_controlled_profile)
    except BaseException as error:
        if not _full_files_profile(_full_profile) and not _controlled_files_profile(_controlled_profile):
            _write_failure_diagnostic(target, identity, status, current_phase,
                                      error)
        # The full owner retains fixed phase evidence and writes its private
        # failure record only after the parent has attempted isolation.
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--batch-id", required=True)
    parser.add_argument("--subnet", required=True)
    parser.add_argument("--initdb", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = target_provisioner.provision(
            args.root, args.batch_id, args.subnet, args.initdb,
            _birth_issuer=issue_birth)
    except BaseException:
        print("BIRTH_NOT_ISSUED_TARGET_QUARANTINED_OR_ADMISSION_REJECTED", flush=True)
        return 1
    print(json.dumps({"state": "BIRTH_ISSUED_NOT_RESTORE_ACCEPTANCE",
                      "batch_id": result["batch_id"],
                      "birth_sha256": result["birth_sha256"],
                      "issuance_sha256": result["issuance_sha256"],
                      "birth_path": result["birth_path"],
                      "destination_root": result["destination_root"],
                      "control_root": result["control_root"],
                      "asset_root": result["asset_root"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
