SELECT pg_catalog.jsonb_build_object(
 'helpers',(SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
  'schema',n.nspname,'name',p.proname,'owner',pg_catalog.pg_get_userbyid(p.proowner),
  'kind',p.prokind,'language',(SELECT l.lanname FROM pg_catalog.pg_language l WHERE l.oid=p.prolang),
  'definition',CASE WHEN p.prokind IN ('f','p') THEN pg_catalog.pg_get_functiondef(p.oid) ELSE NULL END,
  'body',p.prosrc,'input_types',p.proargtypes::pg_catalog.text,'all_types',p.proallargtypes::pg_catalog.text,
  'arg_names',p.proargnames,'arg_modes',p.proargmodes,'security_definer',p.prosecdef,
  'strict',p.proisstrict,'leakproof',p.proleakproof,'volatility',p.provolatile,
  'parallel',p.proparallel,'config',p.proconfig,'acl',p.proacl::pg_catalog.text,
  'nargs',p.pronargs,'defaults',p.pronargdefaults,'variadic',p.provariadic::pg_catalog.int8,
  'support',p.prosupport::pg_catalog.int8,'cost',p.procost,'rows',p.prorows,
  'bin_null',p.probin IS NULL,'sqlbody_null',p.prosqlbody IS NULL,
  'argdefaults_null',p.proargdefaults IS NULL,'trftypes_null',p.protrftypes IS NULL,
  'returns_set',p.proretset,'return_type',p.prorettype::pg_catalog.int8,
  'extension_member',EXISTS(SELECT FROM pg_catalog.pg_depend d WHERE d.classid='pg_catalog.pg_proc'::pg_catalog.regclass AND d.objid=p.oid AND d.deptype='e'),
  'comment',pg_catalog.obj_description(p.oid,'pg_proc')) ORDER BY n.nspname,p.proname,p.proargtypes)
  FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
  WHERE p.proname IN ('p0c4_snapshot_claim_locked_observation','p0c4_snapshot_publication_observe_v1','p0c4_snapshot_locked_observation')
   AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_'),
 'broker',(SELECT pg_catalog.jsonb_build_object('name',r.rolname,'login',r.rolcanlogin,
  'inherit',r.rolinherit,'superuser',r.rolsuper,'createdb',r.rolcreatedb,'createrole',r.rolcreaterole,
  'replication',r.rolreplication,'bypassrls',r.rolbypassrls,'connection_limit',r.rolconnlimit,
  'valid_until_null',r.rolvaliduntil IS NULL,
  'connect',pg_catalog.has_database_privilege(r.oid,pg_catalog.current_database(),'CONNECT'),
  'create',pg_catalog.has_database_privilege(r.oid,pg_catalog.current_database(),'CREATE'),
  'temp',pg_catalog.has_database_privilege(r.oid,pg_catalog.current_database(),'TEMP'),
  'memberships',(SELECT pg_catalog.count(*) FROM pg_catalog.pg_auth_members m WHERE m.member=r.oid OR m.roleid=r.oid),
  'settings',(SELECT pg_catalog.count(*) FROM pg_catalog.pg_db_role_setting s WHERE s.setrole=r.oid),
  'ownerships',(SELECT pg_catalog.count(*) FROM pg_catalog.pg_shdepend d WHERE d.refclassid='pg_catalog.pg_authid'::pg_catalog.regclass AND d.refobjid=r.oid AND d.deptype='o'),
  'direct_relation_acls',(SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c,LATERAL pg_catalog.aclexplode(c.relacl) a WHERE a.grantee=r.oid),
  'direct_column_acls',(SELECT pg_catalog.count(*) FROM pg_catalog.pg_attribute c,LATERAL pg_catalog.aclexplode(c.attacl) a WHERE a.grantee=r.oid),
  'default_acls',(SELECT pg_catalog.count(*) FROM pg_catalog.pg_default_acl d,LATERAL pg_catalog.aclexplode(d.defaclacl) a WHERE a.grantee=r.oid),
  'schema_acls',(SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object('schema',n.nspname,'grantor_is_owner',a.grantor=n.nspowner,'privilege',a.privilege_type,'grantable',a.is_grantable) ORDER BY n.nspname,a.privilege_type) FROM pg_catalog.pg_namespace n,LATERAL pg_catalog.aclexplode(n.nspacl) a WHERE a.grantee=r.oid),
  'function_acls',(SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object('schema',n.nspname,'name',p.proname,'types',p.proargtypes::pg_catalog.text,'owner',pg_catalog.pg_get_userbyid(p.proowner),'grantor',pg_catalog.pg_get_userbyid(a.grantor),'privilege',a.privilege_type,'grantable',a.is_grantable) ORDER BY n.nspname,p.proname,p.proargtypes,a.privilege_type) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace,LATERAL pg_catalog.aclexplode(p.proacl) a WHERE a.grantee=r.oid),
  'database_acls',(SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object('current_database',d.datname=pg_catalog.current_database(),'grantor',pg_catalog.pg_get_userbyid(a.grantor),'privilege',a.privilege_type,'grantable',a.is_grantable) ORDER BY d.datname,a.privilege_type) FROM pg_catalog.pg_database d,LATERAL pg_catalog.aclexplode(d.datacl) a WHERE a.grantee=r.oid),
  'extra_schema_access',EXISTS(SELECT FROM pg_catalog.pg_namespace n WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_' AND (pg_catalog.has_schema_privilege(r.oid,n.oid,'CREATE') OR (n.nspname<>'public' AND pg_catalog.has_schema_privilege(r.oid,n.oid,'USAGE')))),
  'extra_definer_execute',EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_' AND p.prosecdef AND pg_catalog.has_function_privilege(r.oid,p.oid,'EXECUTE') AND p.oid NOT IN ('public.p0c3_claim_snapshot_job(uuid,bigint)'::pg_catalog.regprocedure,'public.p0c3_renew_snapshot_job(uuid,uuid,bigint)'::pg_catalog.regprocedure,'public.p0c3_fail_snapshot_job(uuid,uuid,text,boolean)'::pg_catalog.regprocedure,'public.p0c4_snapshot_claim_locked_observation(uuid,boolean)'::pg_catalog.regprocedure)))
  FROM pg_catalog.pg_roles r WHERE r.rolname='learning_snapshot_claim_broker'));
