"""Run the synthetic reading preview against a fresh, local PostgreSQL 18 cluster."""
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path, PurePosixPath
import queue
import re
import secrets
import socket
import subprocess
import sys
import threading
import time
import uuid


def child_environment():
    keep = {
        "PATH", "PATHEXT", "SYSTEMROOT", "WINDIR", "PROGRAMFILES", "PROGRAMFILES(X86)",
        "PROGRAMW6432", "USERPROFILE", "HOME", "TEMP", "TMP", "TMPDIR", "LOCALAPPDATA",
        "APPDATA", "PLAYWRIGHT_BROWSERS_PATH",
    }
    return {key: value for key, value in os.environ.items() if key.upper() in keep}


def command(args, payload=None, timeout=30, env=None):
    result = subprocess.run(
        args, input=payload, capture_output=True, encoding="utf-8", timeout=timeout, env=env
    )
    if result.returncode:
        # Keep SQL and any PostgreSQL authentication material out of terminal output.
        raise RuntimeError(f"{PurePosixPath(args[0]).name} exited {result.returncode}")
    return result.stdout.strip()


def line_reader(process):
    lines = queue.Queue()

    def read():
        for line in process.stdout:
            lines.put(line.strip())
        lines.put(None)

    threading.Thread(target=read, daemon=True).start()

    def next_line(timeout):
        line = lines.get(timeout=timeout)
        if line is None:
            raise RuntimeError("preview child closed stdout")
        return line

    return next_line


def stop_child(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


@contextmanager
def postgres(args):
    prefix = ["wsl.exe", "-d", args.wsl, "--exec"] if args.wsl else []
    prefix += ["/usr/bin/env", "-i", "PATH=/usr/bin:/bin", "LANG=C.UTF-8"]
    if args.pg_lib:
        prefix.append(f"LD_LIBRARY_PATH={args.pg_lib}")

    def pg(tool, *options, payload=None, timeout=30):
        return command(
            prefix + [f"{args.pg_bin}/{tool}", *options], payload=payload, timeout=timeout
        )

    version = pg("postgres", "--version")
    if not re.match(r"postgres \(PostgreSQL\) 18\.", version):
        raise RuntimeError("preview requires the PostgreSQL 18 tools")
    user = command(prefix + ["/usr/bin/id", "-un"])
    if not re.fullmatch(r"[a-z_][a-z0-9_-]{0,31}", user) or user == "root":
        raise RuntimeError("run the preview under an ordinary Linux account")
    with socket.socket() as probe:
        # The application connects from this OS. Start with its free port rather
        # than a free WSL port, which may already be reserved on Windows.
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    base = command(prefix + ["/usr/bin/mktemp", "-d", "/tmp/kw-reading-preview.XXXXXXXXXX"])
    if not re.fullmatch(r"/tmp/kw-reading-preview\.[A-Za-z0-9]{10}", base):
        raise RuntimeError("unexpected temporary cluster path")
    data = f"{base}/data"
    try:
        pg("initdb", "-D", data, "--encoding=UTF8", "--locale=C.UTF-8",
           "--auth-local=peer", "--auth-host=scram-sha-256", timeout=45)
        pg("pg_ctl", "-D", data, "-l", f"{base}/postgres.log", "-w", "-t", "15",
           "-o", f"-p {port} -h 127.0.0.1 -k {base} -c jit=off -c log_min_error_statement=panic",
           "start", timeout=20)

        def sql(database, statement):
            return pg("psql", "-XqAtw", "-v", "ON_ERROR_STOP=1", "-h", base,
                      "-p", str(port), "-U", user, "-d", database, payload=statement)

        yield version, port, sql
    finally:
        status = subprocess.run(
            prefix + [f"{args.pg_bin}/pg_ctl", "-D", data, "status"],
            capture_output=True, timeout=10,
        ).returncode
        if status == 0:
            pg("pg_ctl", "-D", data, "-w", "-t", "15", "-m", "fast", "stop", timeout=20)
            status = subprocess.run(
                prefix + [f"{args.pg_bin}/pg_ctl", "-D", data, "status"],
                capture_output=True, timeout=10,
            ).returncode
        if status not in (3, 4):
            raise RuntimeError(f"temporary PostgreSQL stop could not be verified for {base}")
        command(prefix + ["/usr/bin/python3", "-c",
            "from pathlib import Path; import os,re,shutil,sys; p=Path(sys.argv[1]); "
            "assert p.resolve()==p and p.parent==Path('/tmp') and "
            "re.fullmatch(r'kw-reading-preview\\.[A-Za-z0-9]{10}',p.name) and p.stat().st_uid==os.getuid(); "
            "assert not (p/'data'/'postmaster.pid').exists(); shutil.rmtree(p)", base])


def prepare(sql, port, binary):
    marker = uuid.uuid4()
    database = f"kw_preview_{marker.hex}"
    credentials = {role: secrets.token_hex(32) for role in (
        "learning_admin", "learning_runtime", "kw_preview_reader"
    )}
    roles = "\n".join(
        f"CREATE ROLE {role} LOGIN PASSWORD '{value}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;"
        for role, value in credentials.items()
    )
    sql("postgres", f"""
{roles}
CREATE ROLE learning_auth_lock NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
GRANT learning_auth_lock TO learning_admin WITH INHERIT FALSE, SET TRUE;
ALTER ROLE kw_preview_reader SET default_transaction_read_only = on;
CREATE DATABASE {database} OWNER learning_admin;
REVOKE ALL ON DATABASE {database} FROM PUBLIC;
GRANT CONNECT, TEMPORARY ON DATABASE {database} TO learning_admin, learning_runtime;
GRANT CONNECT ON DATABASE {database} TO kw_preview_reader;
""")
    sql(database, f"""
CREATE SCHEMA kw_preview;
CREATE TABLE kw_preview.bootstrap (marker uuid PRIMARY KEY);
INSERT INTO kw_preview.bootstrap VALUES ('{marker}');
GRANT USAGE ON SCHEMA kw_preview TO learning_admin, learning_runtime, kw_preview_reader;
GRANT SELECT ON kw_preview.bootstrap TO learning_admin, learning_runtime, kw_preview_reader;
GRANT USAGE ON SCHEMA public TO kw_preview_reader;
""")
    db = {"port": port, "marker": str(marker)}
    fixture = json.loads(command([str(binary), "seed"], json.dumps({
        "db": db, "admin_password": credentials["learning_admin"],
        "runtime_password": credentials["learning_runtime"],
    }), timeout=60, env=child_environment()))
    config = {
        "db": db, "fixture": fixture, "reader_password": credentials["kw_preview_reader"],
        "owner_code": secrets.token_hex(32), "visitor_code": secrets.token_hex(32),
    }
    return database, config


def verify(args, url, database, config, sql):
    browser = Path(__file__).with_name("browser-check.mjs")
    env = dict(child_environment(), KW_PREVIEW_URL=url, KW_OWNER_CODE=config["owner_code"],
               KW_VISITOR_CODE=config["visitor_code"], KW_PREVIEW_EVIDENCE=args.evidence or "")
    child = subprocess.Popen(["node", str(browser)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             text=True, encoding="utf-8", env=env)
    try:
        next_line = line_reader(child)
        fixture = config["fixture"]
        owner = uuid.UUID(fixture["owner"])
        marker = uuid.UUID(config["db"]["marker"])
        for step, field in [("REVOKE_SOURCE", "source_space"), ("REVOKE_PERSONAL", "personal_space")]:
            if next_line(45) != step:
                raise RuntimeError(f"browser did not reach {step}")
            space = uuid.UUID(fixture[field])
            deleted = sql(database, f"""
DELETE FROM public.space_grant WHERE actor_id = '{owner}' AND space_id = '{space}'
AND EXISTS (SELECT 1 FROM kw_preview.bootstrap WHERE marker = '{marker}')
RETURNING 1;
""")
            if deleted != "1":
                raise RuntimeError("synthetic grant revocation failed")
            child.stdin.write("CONTINUE\n")
            child.stdin.flush()
        result = json.loads(next_line(45))
        if child.wait(timeout=10) != 0 or result["status"] != "B1_BROWSER_CHECKS_PASSED":
            raise RuntimeError("browser checks did not pass")
        return result
    finally:
        stop_child(child)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--pg-bin", required=True, help="absolute Linux path to PostgreSQL 18 bin")
    parser.add_argument("--pg-lib", help="absolute Linux path to private shared libraries, if needed")
    parser.add_argument("--wsl", help="existing WSL distribution, required on Windows")
    parser.add_argument("--verify", action="store_true", help="run the browser and revocation checks")
    parser.add_argument("--evidence", help="existing directory for browser screenshots")
    args = parser.parse_args()
    if (os.name == "nt") != bool(args.wsl):
        parser.error("use --wsl only on Windows, with an existing Linux distribution")
    for value in (args.pg_bin, args.pg_lib):
        if value and not PurePosixPath(value).is_absolute():
            parser.error("PostgreSQL tool paths must be absolute Linux paths")
    binary = args.binary.resolve(strict=True)
    if not binary.is_file():
        parser.error("--binary must be the compiled preview executable")
    if args.evidence and not Path(args.evidence).is_dir():
        parser.error("--evidence must be an existing directory")

    with postgres(args) as (version, port, sql):
        # Allow the existing WSL loopback forwarder time to observe the new listener.
        for attempt in range(20):
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                    break
            except OSError:
                if attempt == 19:
                    raise RuntimeError("temporary PostgreSQL is not reachable over loopback")
                time.sleep(0.15)
        database, config = prepare(sql, port, binary)
        server = subprocess.Popen([str(binary), "serve"], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, text=True, encoding="utf-8",
                                  env=child_environment())
        try:
            server.stdin.write(json.dumps(config))
            server.stdin.close()
            url = line_reader(server)(20)
            if not re.fullmatch(r"http://127\.0\.0\.1:\d{4,5}", url):
                raise RuntimeError("unexpected preview listener")
            if args.verify:
                result = verify(args, url, database, config, sql)
            else:
                print(f"\n打开 {url}\n学习者访问码  {config['owner_code']}\n"
                      f"访客访问码    {config['visitor_code']}\n按 Ctrl+C 结束本次预览。", flush=True)
                server.wait()
                raise RuntimeError("preview server stopped")
        finally:
            stop_child(server)
    result.update(postgresql=version, temporary_cluster_stopped_and_removed=True)
    print(json.dumps(result, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("预览已结束。")
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired, queue.Empty) as error:
        print(f"预览退出：{error}", file=sys.stderr)
        sys.exit(1)
