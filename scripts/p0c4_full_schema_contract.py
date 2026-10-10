#!/usr/bin/env python3
"""Read-only fixed PG18 contract capture. Root provisions a FRESH migrated fixture.

generate --plan PRIVATE_PLAN: captures one exact container/database. Never creates
a database, migrates, loads data, signs, publishes, stops or deletes resources.
Output is an unreviewed candidate; only explicit source review can adopt it.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import selectors
import subprocess
import time
import uuid

PG_IMAGE = 'sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d'
PG_VERSION = '18.6 (Debian 18.6-1.pgdg12+2)'
MAX_METADATA = 4*1024**2
MAX_DUMP = 1024**3
MAX_DECODE = 4*1024**3
KEY = b'{{RESTRICT_KEY}}'
IDENT = re.compile(r'[a-z_][a-z0-9_]*\Z')
HEX64 = re.compile(r'[0-9a-f]{64}\Z')

class ContractError(RuntimeError): pass

def require(ok, token):
    if not ok: raise ContractError(token)

def canonical(value):
    return json.dumps(value,ensure_ascii=False,sort_keys=True,separators=(',',':')).encode()

def sha(raw): return hashlib.sha256(raw).hexdigest()

def normalize_restrict(raw):
    require(b'\r' not in raw and b'\0' not in raw, 'SCHEMA_LINE_ENDINGS')
    start=b'--\n-- PostgreSQL database dump\n--\n\n'
    require(raw.startswith(start), 'SCHEMA_PREAMBLE')
    lines=raw.splitlines(keepends=True)
    controls=[(i,l) for i,l in enumerate(lines) if l.startswith(b'\\')]
    require(len(controls)==2 and controls[0][0]==4, 'RESTRICT_POSITIONS')
    first=re.fullmatch(rb'\\restrict ([A-Za-z0-9]{1,128})\n',controls[0][1])
    require(first is not None, 'RESTRICT_KEY')
    key=first.group(1)
    require(controls[1][1]==b'\\unrestrict '+key+b'\n' and b''.join(lines[controls[1][0]+1:])==b'\n','UNRESTRICT_POSITION')
    lines[4]=b'\\restrict '+KEY+b'\n'
    lines[controls[1][0]]=b'\\unrestrict '+KEY+b'\n'
    return b''.join(lines)

def normalize_toc(raw):
    require(len(raw)<=MAX_METADATA and raw.isascii() and b'\r' not in raw and raw.endswith(b'\n'),'TOC_BYTES')
    lines=raw.decode().splitlines()
    require(len(lines)>=15 and lines[0]==';' and re.fullmatch(r'; Archive created at \d{4}-\d\d-\d\d \d\d:\d\d:\d\d UTC',lines[1]),'TOC_DATE')
    require(re.fullmatch(r';     dbname: [A-Za-z0-9_-]{1,63}',lines[2]),'TOC_DATABASE')
    require(re.fullmatch(r';     TOC Entries: [1-9][0-9]{0,5}',lines[3]) and int(lines[3].split(': ')[1])<=100000,'TOC_COUNT')
    require(lines[4:15]==[';     Compression: gzip',';     Dump Version: 1.16-0',';     Format: CUSTOM',';     Integer: 4 bytes',';     Offset: 8 bytes',';     Dumped from database version: '+PG_VERSION,';     Dumped by pg_dump version: '+PG_VERSION,';',';','; Selected TOC Entries:',';'],'TOC_HEADER')
    rows=[];ids=set()
    for line in lines[15:]:
        m=re.fullmatch(r'([1-9][0-9]*); (0|[1-9][0-9]*) (0|[1-9][0-9]*) ([^\r\n]+)',line)
        require(m is not None and all(int(n)<=2**32-1 for n in m.groups()[:3]),'TOC_ENTRY')
        require(m[1] not in ids,'TOC_DUPLICATE_ID');ids.add(m[1])
        # Catalog OID (e.g. 1259 for pg_class) is stable and MUST remain bound.
        # Only archive dump ID and per-instance object OID are volatile.
        rows.append(m[2]+' '+m[4])
    require(len(rows)<=100000 and len(set(rows))==len(rows),'TOC_ENTRIES')
    return {'header':lines[3:15], 'entries':sorted(rows)}

def migration_identity(source_root):
    rows=[]
    for path in sorted((source_root/'migrations').glob('*.sql')):
        require(re.fullmatch(r'[0-9]{4}_[a-z0-9_]+\.sql',path.name),'MIGRATION_NAME')
        rows.append({'version':int(path.name[:4]),'checksum_hex':hashlib.sha384(path.read_bytes()).hexdigest()})
    require([r['version'] for r in rows]==list(range(1,16)),'ALL_FIFTEEN_MIGRATIONS')
    return rows

def require_migrations(actual, expected): require(actual==expected,'MIGRATION_SET_OR_CHECKSUM')

def native_identifier(name, sql_name):
    # Native quote_ident output is data captured from the fixed reviewed PG
    # fixture. It can encode only the same existing lowercase ASCII name.
    # Import never strips quotes or accepts an alternative header spelling.
    require(type(name)is str and len(name)<=63 and IDENT.fullmatch(name),'COLUMN_IDENTIFIER')
    require(type(sql_name)is str and sql_name in (name,'"'+name+'"'),'COLUMN_SQL_IDENTIFIER')
    return sql_name

def copy_header(table):
    require(type(table)is dict and type(table.get('columns'))is list and 0<len(table['columns'])<=1600 and all(type(c)is dict for c in table['columns']),'COLUMN_ROSTER')
    table_name=native_identifier(table.get('table'),table.get('sql_name'))
    columns=[native_identifier(c.get('name'),c.get('sql_name')) for c in table['columns']]
    require(len({c['name'] for c in table['columns']})==len(columns),'COLUMN_DUPLICATE')
    return ('COPY public.'+table_name+' ('+', '.join(columns)+') FROM stdin;\n').encode()

def split_copy(raw, tables):
    """Capture only enumerated, exactly framed payloads; never rewrite data."""
    headers={copy_header(t):t['table'] for t in tables};seen={};out=[]
    require(len(headers)==len(tables) and len(set(headers.values()))==len(tables),'COPY_TABLE_DUPLICATE')
    pos=0
    while pos<len(raw):
        end=raw.find(b'\n',pos)
        require(end>=0,'COPY_UNTERMINATED_LINE')
        line=raw[pos:end+1];out.append(line);pos=end+1
        if line.startswith(b'COPY '):
            require(line in headers and headers[line] not in seen,'COPY_HEADER')
            table=headers[line];start=pos
            while True:
                end=raw.find(b'\n',pos);require(end>=0,'COPY_TERMINATOR')
                data=raw[pos:end+1];require(len(data)<=64*1024**2,'COPY_ROW_CAP')
                if data==b'\\.\n':break
                require(not data.startswith(b'\\.'),'COPY_BAD_TERMINATOR')
                pos=end+1
            seen[table]=raw[start:pos];out.append(('{{COPY:'+table+'}}').encode());out.append(b'\\.\n');pos=end+1
    require(set(seen)=={t['table'] for t in tables},'COPY_TABLE_SET')
    return b''.join(out),seen

def fixed_command(container, database, operation):
    require(HEX64.fullmatch(container) is not None,'EXACT_CONTAINER_ID')
    prefix='learning_backup_c4_task3_';tail=database.removeprefix(prefix)
    try: parsed=uuid.UUID(tail)
    except (ValueError,AttributeError) as error:raise ContractError('FIXED_DATABASE') from error
    require(database==prefix+str(parsed) and parsed.version==4 and parsed.variant==uuid.RFC_4122,'FIXED_DATABASE')
    args=['/usr/bin/docker','exec','--interactive','--user','999:999',container,'/usr/bin/env','-i','LC_ALL=C','PGCONNECT_TIMEOUT=10','PGPASSFILE=/dev/null/disabled']
    connection=['--no-password','--host=/var/run/postgresql','--port=5432','--username=learning_admin','--dbname='+database]
    commands={
        'dump':['/usr/lib/postgresql/18/bin/pg_dump','--format=custom',*connection],
        'toc':['/usr/lib/postgresql/18/bin/pg_restore','--list'],
        'decode':['/usr/lib/postgresql/18/bin/pg_restore','--file=-','--no-owner','--no-acl','--exit-on-error'],
        'owned-schema':['/usr/lib/postgresql/18/bin/pg_restore','--schema-only','--file=-','--exit-on-error'],
        'catalog':['/usr/lib/postgresql/18/bin/psql','-X','-qAt','-v','ON_ERROR_STOP=1',*connection],
    }
    require(operation in commands,'FIXED_OPERATION');return args+commands[operation]

# These are fixed catalog reads. The captured SQL is evidence, never a caller
# selected statement. pg_dump supplies all executable definitions separately.
COLUMNS_SQL="""SELECT coalesce(json_agg(t ORDER BY t.table),'[]') FROM (
 SELECT c.relname AS table, pg_catalog.quote_ident(c.relname) AS sql_name, (SELECT json_agg(json_build_object('name',a.attname,'sql_name',pg_catalog.quote_ident(a.attname),'type',format_type(a.atttypid,a.atttypmod),'not_null',a.attnotnull,'default',pg_get_expr(d.adbin,d.adrelid),'identity',a.attidentity,'generated',a.attgenerated) ORDER BY a.attnum)
 FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped) AS columns
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind='r')t;"""
MIGRATIONS_SQL="SELECT json_agg(json_build_object('version',version,'checksum_hex',encode(checksum,'hex'),'success',success) ORDER BY version) FROM public._sqlx_migrations;"
ACL_SQL="""SELECT json_build_object(
 'database',(SELECT json_build_object('owner',pg_get_userbyid(datdba),'acl',datacl::text) FROM pg_database WHERE datname=current_database()),
 'schemas',(SELECT json_agg(json_build_object('name',nspname,'owner',pg_get_userbyid(nspowner),'acl',nspacl::text) ORDER BY nspname) FROM pg_namespace WHERE nspname !~ '^pg_' AND nspname<>'information_schema'),
 'relations',(SELECT json_agg(json_build_object('name',c.relname,'kind',c.relkind,'owner',pg_get_userbyid(c.relowner),'acl',c.relacl::text) ORDER BY c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'),
 'column_acl',(SELECT coalesce(json_agg(json_build_object('table',c.relname,'column',a.attname,'acl',a.attacl::text) ORDER BY c.relname,a.attnum),'[]') FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND a.attacl IS NOT NULL),
 'functions',(SELECT json_agg(json_build_object('name',p.proname,'args',pg_get_function_identity_arguments(p.oid),'owner',pg_get_userbyid(p.proowner),'acl',p.proacl::text,'security_definer',p.prosecdef,'config',p.proconfig) ORDER BY p.proname,pg_get_function_identity_arguments(p.oid)) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public'),
 'roles',(SELECT json_agg(json_build_object('name',rolname,'superuser',rolsuper,'inherit',rolinherit,'create_role',rolcreaterole,'create_db',rolcreatedb,'login',rolcanlogin,'replication',rolreplication,'bypass_rls',rolbypassrls,'connection_limit',rolconnlimit) ORDER BY rolname) FROM pg_roles WHERE rolname IN ('learning_admin','learning_runtime','learning_auth_lock')),
 'memberships',(SELECT json_agg(json_build_object('role',r.rolname,'member',m.rolname,'grantor',g.rolname,'admin',a.admin_option,'inherit',a.inherit_option,'set',a.set_option) ORDER BY r.rolname,m.rolname) FROM pg_auth_members a JOIN pg_roles r ON r.oid=a.roleid JOIN pg_roles m ON m.oid=a.member JOIN pg_roles g ON g.oid=a.grantor WHERE r.rolname LIKE 'learning_%' OR m.rolname LIKE 'learning_%'),
 'default_acl',(SELECT coalesce(json_agg(json_build_object('role',pg_get_userbyid(defaclrole),'schema',coalesce(n.nspname,''),'type',defaclobjtype,'acl',defaclacl::text) ORDER BY defaclrole,defaclnamespace,defaclobjtype),'[]') FROM pg_default_acl a LEFT JOIN pg_namespace n ON n.oid=a.defaclnamespace),
 'constraints',(SELECT json_agg(json_build_object('table',c.relname,'name',p.conname,'type',p.contype,'definition',pg_get_constraintdef(p.oid,true)) ORDER BY c.relname,p.conname) FROM pg_constraint p JOIN pg_class c ON c.oid=p.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'),
 'triggers',(SELECT json_agg(json_build_object('table',c.relname,'name',t.tgname,'definition',pg_get_triggerdef(t.oid,true),'enabled',t.tgenabled) ORDER BY c.relname,t.tgname) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND NOT t.tgisinternal));"""

def collect(command, output, cap, *, input_file=None, input_bytes=None):
    """Bound both pipes while running; failure kills AND reaps before returning."""
    require(not (input_file is not None and input_bytes is not None),'INPUT_MODE')
    # Queries are small and input is never a dump-sized Python allocation.
    child=subprocess.Popen(command,stdin=input_file if input_file is not None else subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env={'PATH':'/usr/bin:/bin','LC_ALL':'C'},close_fds=True)
    selector=selectors.DefaultSelector();size=0;errors=bytearray();deadline=time.monotonic()+900
    try:
        if input_file is None:
            if input_bytes:child.stdin.write(input_bytes)
            child.stdin.close()
        selector.register(child.stdout,selectors.EVENT_READ,False);selector.register(child.stderr,selectors.EVENT_READ,True)
        while selector.get_map():
            require(time.monotonic()<deadline,'CAPTURE_TIMEOUT')
            for key,_ in selector.select(timeout=0.2):
                raw=os.read(key.fd,65536)
                if not raw:selector.unregister(key.fileobj);continue
                if key.data:
                    require(len(errors)+len(raw)<=8192,'CAPTURE_STDERR_CAP');errors.extend(raw)
                else:
                    require(size+len(raw)<=cap,'CAPTURE_OUTPUT_CAP');output.write(raw);size+=len(raw)
        require(child.wait(timeout=max(0.01,deadline-time.monotonic()))==0 and not errors,'CAPTURE_EXIT')
        return size
    finally:
        if child.poll() is None:child.kill()
        child.wait();selector.close();child.stdout.close();child.stderr.close()

def capture(plan):
    require(os.name=='posix' and os.geteuid()==0,'ROOT_LINUX_ONLY')
    keys={'format_version','case_id','container_id','database','source_root','application_commit','application_build_sha256','output_directory'}
    require(type(plan)is dict and set(plan)==keys and type(plan['format_version'])is int and plan['format_version']==1,'PLAN_FIELDS')
    case=uuid.UUID(plan['case_id']);require(str(case)==plan['case_id'] and case.version==4,'PLAN_CASE')
    require(plan['database']=='learning_backup_c4_task3_'+str(case),'PLAN_DATABASE')
    require(re.fullmatch('[0-9a-f]{40}',plan['application_commit']) and HEX64.fullmatch(plan['application_build_sha256']),'PLAN_SOURCE')
    command=lambda op:fixed_command(plan['container_id'],plan['database'],op)
    source=Path(plan['source_root']);out=Path(plan['output_directory'])
    require(source.is_absolute() and out.is_absolute() and not out.exists(),'FRESH_OUTPUT_DIRECTORY')
    migrations=migration_identity(source)
    os.umask(0o077);out.mkdir(mode=0o700)
    def inspect(kind, identity):
        buffer=io.BytesIO()
        collect(['/usr/bin/docker',kind,'inspect',identity],buffer,MAX_METADATA)
        rows=json.loads(buffer.getvalue());require(type(rows)is list and len(rows)==1,'INSPECT_ONE')
        return rows[0]
    facts=inspect('container',plan['container_id'])
    require(facts['Id']==plan['container_id'] and facts['State']['Running'] is True,'EXACT_RUNNING_CONTAINER')
    image=inspect('image',facts['Image'])
    require(any(r.endswith('@'+PG_IMAGE) for r in image.get('RepoDigests',[])),'FIXED_POSTGRES_IMAGE')
    require(not facts['HostConfig'].get('Privileged',False) and not facts['HostConfig'].get('PortBindings'),'ISOLATED_CAPTURE_CONTAINER')
    def generate(name,op,cap,*,query=None,archive=None):
        with (out/name).open('xb') as target:
            if archive:
                with (out/archive).open('rb') as inp:collect(command(op),target,cap,input_file=inp)
            else:collect(command(op),target,cap,input_bytes=query.encode() if query else None)
            target.flush();os.fsync(target.fileno())
        return (out/name).read_bytes() if cap<=MAX_METADATA else None
    observed=json.loads(generate('migrations.raw.json','catalog',MAX_METADATA,query=MIGRATIONS_SQL))
    require(all(r.pop('success',None) is True for r in observed),'MIGRATION_SUCCESS');require_migrations(observed,migrations)
    columns=json.loads(generate('columns.raw.json','catalog',MAX_METADATA,query=COLUMNS_SQL))
    require(len(columns)==64 and any(t['table']=='_sqlx_migrations' for t in columns),'FULL_TABLE_ROSTER')
    for table in columns:copy_header(table)
    # Refuse a populated fixture, not merely a fixture with a matching schema.
    query='SELECT json_build_array('+','.join('(SELECT count(*) FROM public.'+t['table']+')' for t in columns)+');'
    counts=json.loads(generate('counts.raw.json','catalog',MAX_METADATA,query=query))
    require(counts==[15 if t['table']=='_sqlx_migrations' else 0 for t in columns],'EMPTY_MIGRATED_FIXTURE')
    acl=json.loads(generate('acl.raw.json','catalog',MAX_METADATA,query=ACL_SQL))
    generate('database.dump','dump',MAX_DUMP)
    toc=normalize_toc(generate('toc.raw.txt','toc',MAX_METADATA,archive='database.dump'))
    generate('decoded.raw.sql','decode',MAX_DECODE,archive='database.dump')
    # A fresh empty schema must fit this metadata cap even though operational
    # full dumps have a distinct 4 GiB streamed data cap.
    require((out/'decoded.raw.sql').stat().st_size<=MAX_METADATA,'EMPTY_SCHEMA_CAP')
    template,data=split_copy((out/'decoded.raw.sql').read_bytes(),columns)
    template=normalize_restrict(template)
    owned=normalize_restrict(generate('owned-schema.raw.sql','owned-schema',MAX_METADATA,archive='database.dump'))
    after=inspect('container',plan['container_id'])
    require(after['Id']==facts['Id'] and after['Image']==facts['Image'] and after['State']['Running'] is True and after['State']['StartedAt']==facts['State']['StartedAt'],'CAPTURE_CONTAINER_CHANGED')
    artifacts={'pg18-schema.sql.in':template,'pg18-toc.json':canonical(toc),'columns.json':canonical(columns),'acl.json':canonical({'catalog':acl,'owned_schema_template':owned.decode()})}
    contract={'format_version':1,'capability':'full_schema_content_v1','postgres_image':PG_IMAGE,'postgres_version':PG_VERSION,'producer_sha256':sha(Path(__file__).read_bytes()),'source':{k:plan[k] for k in ('case_id','container_id','database','application_commit','application_build_sha256')},'migrations':migrations,'migration_fingerprint':sha(json.dumps(migrations,separators=(',',':')).encode()),'files':{name:{'size':len(raw),'sha256':sha(raw)} for name,raw in artifacts.items()},'migration_copy_sha256':sha(data['_sqlx_migrations'])}
    artifacts['contract.json']=canonical(contract)
    for name,raw in artifacts.items():
        with (out/name).open('xb') as stream:stream.write(raw);stream.flush();os.fsync(stream.fileno())
    evidence={p.name:{'size':p.stat().st_size,'sha256':hash_file(p)} for p in sorted(out.iterdir())}
    with (out/'capture.json').open('xb') as stream:stream.write(canonical({'status':'CANDIDATE_REQUIRES_REVIEW_NOT_RESTORE_AUTHORITY','files':evidence}));stream.flush();os.fsync(stream.fileno())
    fd=os.open(out,os.O_RDONLY|os.O_DIRECTORY)
    try:os.fsync(fd)
    finally:os.close(fd)
    return {'status':'CANDIDATE_REQUIRES_REVIEW_NOT_RESTORE_AUTHORITY','directory':str(out),'contract_sha256':sha(artifacts['contract.json'])}

def hash_file(path):
    h=hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda:stream.read(65536),b''):h.update(chunk)
    return h.hexdigest()

def main():
    parser=argparse.ArgumentParser(description=__doc__);sub=parser.add_subparsers(dest='operation',required=True)
    command=sub.add_parser('generate');command.add_argument('--plan',type=Path,required=True)
    args=parser.parse_args();require(args.plan.stat().st_size<=16384,'PLAN_CAP')
    print(canonical(capture(json.loads(args.plan.read_bytes()))).decode())

if __name__=='__main__':main()
