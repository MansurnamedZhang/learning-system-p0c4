"""Private no-replace envelope publishing and exact test-result readback."""
import json
import math
import os
from pathlib import Path
import re
import uuid
from .plan import CompletionError, require, canonical, digest
if __package__.startswith('scripts.'):
    from .. import p0c4_storage_registry as registry
else:
    import p0c4_storage_registry as registry

def parse_driver_result(raw):
    """Read the existing driver JSON domain, which includes finite timings.

    Registry/plan/receipt records keep their separate integer-only parser.
    Consumers must still type-check authoritative counts and compare embedded
    records by canonical bytes, never Python's int/float/bool equality.
    """
    require(type(raw) is bytes and 0 < len(raw) <= 32*1024**2,'RESULT_DRIVER_BYTES')
    def pairs(items):
        value={}
        for key,item in items:
            require(key not in value,'RESULT_DRIVER_DUPLICATE_KEY');value[key]=item
        return value
    def finite(token):
        value=float(token)
        require(math.isfinite(value),'RESULT_DRIVER_FINITE_NUMBER')
        return value
    try:
        value=json.loads(raw.decode('utf-8'),object_pairs_hook=pairs,parse_float=finite,parse_constant=finite)
        require(type(value) is dict and canonical(value)==raw,'RESULT_DRIVER_CANONICAL')
    except (ValueError,UnicodeError,RecursionError) as error:
        raise CompletionError('RESULT_DRIVER_JSON') from error
    return value


def exact_counts(exit_code,raw):
    require(type(exit_code) is int and exit_code==0,'CASE_TEST_EXIT')
    try:lines=raw.decode('utf-8').splitlines()
    except UnicodeError as error:raise CompletionError('CASE_TEST_UTF8') from error
    tests=[line for line in lines if line.startswith('test ') and not line.startswith('test result:')]
    results=[line for line in lines if line.startswith('test result:')]
    require(len(tests)==1 and tests[0].endswith(' ... ok') and len(results)==1 and re.fullmatch(r'test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in [0-9.]+s',results[0]),'CASE_EXACT_ONE_BODY')
    return dict(passed=1,failed=0,ignored=0)
def private_read(path,cap):
    parent=registry.open_directory(str(Path(path).parent))
    try:return registry.read_file(parent,Path(path).name,cap,registry.Budget())
    finally:os.close(parent)
def write_record(path,raw):
    parent=registry.open_directory(str(path.parent));name='.publish-'+str(uuid.uuid4())
    try:
        registry.new_file(parent,name,raw);registry.rename_no_replace(parent,name,parent,path.name)
        require(registry.read_file(parent,path.name,len(raw),registry.Budget())==raw,'ENVELOPE_READBACK')
    finally:os.close(parent)
def file_ref(path,base,cap=32*1024**2):
    raw=private_read(path,cap);return dict(path=path.relative_to(base).as_posix(),size=len(raw),sha256=digest(raw))
def validate_ref(ref,cap):
    require(type(ref) is dict and set(ref)=={'path','size','sha256'} and type(ref['path']) is str and not ref['path'].startswith('/') and all(p not in ('','.','..') for p in ref['path'].split('/')),'RESULT_FILEREF')
    require(registry.integer(ref['size'],1,cap) and type(ref['sha256']) is str and registry.HEX64.fullmatch(ref['sha256']),'RESULT_FILEREF')
    return ref

def relative_ref(root,ref,cap):
    validate_ref(ref,cap)
    raw=private_read(root/ref['path'],cap);require(len(raw)==ref['size'] and digest(raw)==ref['sha256'],'RESULT_HASH');return raw
