"""Private fixed synthetic-fixture clients. Raw dump/SQL are separate artifacts."""
import re
import secrets
import subprocess
import threading
import time
import uuid

HEX_ID = re.compile(r"[0-9a-f]{64}\Z")
NONCE = re.compile(r"[0-9a-f]{32}\Z")
MAX_INPUT = 65536
MAX_WRITER = 8192

# Strong ownership survives a caught/redacted exception. No background cleanup
# may issue commands; only explicit close() can observe the exact owner again.
_UNSETTLED_CLIENTS = set()


class ImportRejected(RuntimeError):
    def __init__(self, code):
        self.code = code
        super().__init__(code)


def require(value, code):
    if not value:
        raise ImportRejected(code)


def validate_identity(container_id, database):
    require(type(container_id) is str and HEX_ID.fullmatch(container_id), "Identity")
    require(type(database) is str and database.startswith("learning_restore_c4_"), "Identity")
    suffix = database.removeprefix("learning_restore_c4_")
    try:
        parsed = uuid.UUID(suffix)
    except (ValueError, AttributeError):
        raise ImportRejected("Identity") from None
    require(parsed.version == 4 and str(parsed) == suffix, "Identity")


def client_prefix(container_id):
    require(type(container_id) is str and HEX_ID.fullmatch(container_id), "Identity")
    return ["/usr/bin/docker", "exec", "--interactive", "--user", "999:999",
            container_id, "/usr/bin/env", "-i", "LC_ALL=C", "PGCONNECT_TIMEOUT=10",
            "PGPASSFILE=/dev/null/knowweave-c4-passfile-disabled"]


def writer_command(container_id, database):
    validate_identity(container_id, database)
    return client_prefix(container_id) + ["/usr/lib/postgresql/18/bin/psql",
        "-X", "-qAt", "-P", "pager=off", "--no-password", "--host=/var/run/postgresql",
        "--port=5432", "--username=learning_admin", "--dbname=" + database,
        "-v", "ON_ERROR_STOP=1", "-f", "-"]


def decoder_command(container_id):
    return client_prefix(container_id) + ["/usr/lib/postgresql/18/bin/pg_restore",
        "--file=-", "--no-owner", "--no-acl", "--exit-on-error"]


def parse_writer_line(line, nonce):
    require(type(nonce) is str and NONCE.fullmatch(nonce), "Protocol")
    require(type(line) is bytes and len(line) <= 256 and line.endswith(b"\n"), "Protocol")
    try:
        fields = line[:-1].decode("ascii").split("|")
    except UnicodeError:
        raise ImportRejected("Protocol") from None
    require(len(fields) >= 3 and fields[:2] == ["KW_C4", nonce], "Protocol")
    phase = fields[2]
    if phase in ("COMMITTED", "ROLLED_BACK"):
        require(len(fields) == 3, "Protocol")
        return phase, None
    require(phase in ("READY", "PRECOMMIT") and len(fields) == 6, "Protocol")
    values = []
    for value, maximum in zip(fields[3:], (2**31 - 1, 2**63 - 1, 2**64 - 1)):
        require(re.fullmatch(r"[1-9][0-9]{0,19}", value), "Protocol")
        number = int(value)
        require(number <= maximum, "Protocol")
        values.append(number)
    return phase, tuple(values)


class OpenClient:
    """Concurrent bounded reads with deadline-bound sends and open stdin.

    Cumulative counters include consumed receipts. No raw stderr escapes.
    """
    def __init__(self, command, *, timeout=45, stdout_limit=MAX_WRITER,
                 stderr_limit=MAX_WRITER, deadline=None, cleanup_deadline=None,
                 env=None, allow_stderr=False):
        self.deadline = min(time.monotonic() + timeout, deadline) if deadline is not None else time.monotonic() + timeout
        self.cleanup_deadline = cleanup_deadline
        self.allow_stderr = allow_stderr
        self.condition = threading.Condition()
        self.buffers = [bytearray(), bytearray()]
        self.totals = [0, 0]
        self.eof = [False, False]
        self.failure = None
        self.threads = []
        require(not _UNSETTLED_CLIENTS, 'UnconfirmedIsolation')
        require(time.monotonic() < self.deadline, 'Deadline')
        try:
            self.process = subprocess.Popen(command, stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0,
                env=env if env is not None else {"PATH": "/usr/bin:/bin", "LC_ALL": "C"})
        except OSError:
            raise ImportRejected("Io") from None
        for index, stream, limit in [(0, self.process.stdout, stdout_limit),
                                      (1, self.process.stderr, stderr_limit)]:
            thread = threading.Thread(target=self._read, args=(index, stream, limit), daemon=True)
            self.threads.append(thread)
            thread.start()

    def _read(self, index, stream, limit):
        try:
            while True:
                chunk = stream.read(min(1024, limit + 1))
                with self.condition:
                    if not chunk:
                        self.eof[index] = True
                        self.condition.notify_all()
                        return
                    self.totals[index] += len(chunk)
                    if self.totals[index] > limit:
                        self.failure = "StdoutLimit" if index == 0 else "StderrLimit"
                        self.condition.notify_all()
                        return
                    self.buffers[index].extend(chunk)
                    self.condition.notify_all()
        except OSError:
            with self.condition:
                self.failure = self.failure or "Io"
                self.condition.notify_all()

    def _check(self, deadline):
        if self.failure:
            raise ImportRejected(self.failure)
        if time.monotonic() >= min(deadline, self.deadline):
            raise ImportRejected("Deadline")

    def send(self, payload):
        require(type(payload) is bytes and len(payload) <= MAX_INPUT, "InputLimit")
        done = threading.Event()

        def write():
            try:
                view = memoryview(payload)
                while view:
                    count = self.process.stdin.write(view)
                    if not count:
                        raise OSError()
                    view = view[count:]
                self.process.stdin.flush()
            except (OSError, ValueError):
                with self.condition:
                    self.failure = self.failure or "Io"
            finally:
                done.set()
        thread = threading.Thread(target=write, daemon=True)
        self.threads.append(thread)
        thread.start()
        while not done.wait(0.01):
            self._check(self.deadline)
        self._check(self.deadline)

    def line(self, *, timeout=10):
        deadline = min(self.deadline, time.monotonic() + timeout)
        with self.condition:
            while True:
                self._check(deadline)
                require(not self.buffers[1], "Stderr")
                position = self.buffers[0].find(b"\n")
                if position >= 0:
                    line = bytes(self.buffers[0][:position + 1])
                    del self.buffers[0][:position + 1]
                    return line
                require(not self.eof[0], "Protocol")
                self.condition.wait(min(0.05, max(0, deadline - time.monotonic())))

    def still_open(self):
        require(not self.process.stdin.closed and self.process.poll() is None, "Protocol")

    def finish(self):
        self.process.stdin.close()
        while self.process.poll() is None or not all(self.eof):
            self._check(self.deadline)
            with self.condition:
                self.condition.wait(0.01)
        self._check(self.deadline)
        require(self.allow_stderr or not self.buffers[1], "Stderr")
        require(self.process.returncode == 0, "Exit")
        return bytes(self.buffers[0])

    def close(self, *, observe_only=False):
        _UNSETTLED_CLIENTS.add(self)
        if self.cleanup_deadline is None:
            self.cleanup_deadline = time.monotonic() + 5
        failed = False
        try:
            if (not observe_only and self.process.poll() is None and
                    not getattr(self, '_kill_requested', False)):
                self._kill_requested = True
                self.process.kill()
        except OSError:
            failed = True
        try:
            self.process.wait(timeout=0 if observe_only else self._cleanup_remaining(5))
        except (subprocess.TimeoutExpired, OSError):
            failed = True
        for thread in self.threads:
            thread.join(timeout=0 if observe_only else self._cleanup_remaining(1))
        readers_settled = not any(thread.is_alive() for thread in self.threads)
        streams = (self.process.stdin, self.process.stdout, self.process.stderr)
        # Never contend on a pipe lock while a reader/writer still owns it.
        if readers_settled:
            for stream in streams:
                try:
                    stream.close()
                except OSError:
                    failed = True
        try:
            reaped = self.process.poll() is not None
        except OSError:
            reaped = False
        settled = reaped and readers_settled and all(stream.closed for stream in streams)
        if settled:
            _UNSETTLED_CLIENTS.discard(self)
        if failed or not settled:
            error = ImportRejected("UnconfirmedIsolation")
            error.cleanup_owner = self
            raise error

    def _cleanup_remaining(self, maximum):
        return maximum if self.cleanup_deadline is None else min(maximum,
            max(0, self.cleanup_deadline - time.monotonic()))

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def run_fixed(command, payload=b"", *, timeout=15, stdout_limit=MAX_INPUT,
              stderr_limit=MAX_WRITER):
    require(type(payload) is bytes and len(payload) <= MAX_INPUT, "InputLimit")
    with OpenClient(command, timeout=timeout, stdout_limit=stdout_limit,
                    stderr_limit=stderr_limit) as client:
        client.send(payload)
        return client.finish()


TIMEOUTS = (b"SET LOCAL statement_timeout = '10000ms';\n"
            b"SET LOCAL lock_timeout = '5000ms';\n"
            b"SET LOCAL idle_in_transaction_session_timeout = '30000ms';\n"
            b"SET LOCAL transaction_timeout = '60000ms';\n")


def receipt_sql(phase, nonce):
    require(NONCE.fullmatch(nonce) and phase in ("READY", "PRECOMMIT"), "Protocol")
    return (f"SELECT 'KW_C4|{nonce}|{phase}|' || pg_catalog.pg_backend_pid()::text || '|' || "
        "((extract(epoch FROM backend_start)*1000000)::bigint)::text || '|' || "
        "pg_catalog.pg_current_xact_id()::text FROM pg_catalog.pg_stat_activity "
        "WHERE pid=pg_catalog.pg_backend_pid();\n").encode("ascii")


def run_handshake(command, nonce):
    with OpenClient(command) as client:
        client.send(b"BEGIN READ ONLY; " + TIMEOUTS.replace(b"\n", b" ") + receipt_sql("READY", nonce))
        phase, ready = parse_writer_line(client.line(), nonce)
        require(phase == "READY", "Protocol")
        client.still_open()
        client.send(receipt_sql("PRECOMMIT", nonce))
        phase, precommit = parse_writer_line(client.line(), nonce)
        require(phase == "PRECOMMIT" and ready == precommit, "Protocol")
        client.still_open()
        client.send(f"ROLLBACK; SELECT 'KW_C4|{nonce}|ROLLED_BACK';\n".encode())
        require(parse_writer_line(client.line(), nonce) == ("ROLLED_BACK", None), "Protocol")
        require(client.finish() == b"", "Protocol")
    return {"ready_stdin_open": True, "precommit_stdin_open": True,
            "same_writer_transaction": True, "rollback_observed": True}


FIXTURE_SQL = (b"CREATE TABLE public.c4_import_probe (id integer NOT NULL,label text NOT NULL);\n"
               b"ALTER TABLE ONLY public.c4_import_probe ADD CONSTRAINT c4_import_probe_pkey PRIMARY KEY (id);\n"
               b"INSERT INTO public.c4_import_probe(id,label) VALUES (1,'alpha'),(2,'beta');\n")
READ_ROWS = b"SELECT id::text || '|' || label FROM public.c4_import_probe ORDER BY id;\n"


def capture_fixture(source_id: str, database: str) -> dict:
    validate_identity(source_id, database)
    writer = writer_command(source_id, database)
    require(run_fixed(writer, FIXTURE_SQL, timeout=45, stdout_limit=MAX_WRITER) == b"", "Fixture")
    require(run_fixed(writer, READ_ROWS, timeout=45, stdout_limit=MAX_WRITER) == b"1|alpha\n2|beta\n", "Fixture")
    nonce = secrets.token_hex(16)
    with OpenClient(writer) as client:
        client.send(b"BEGIN; " + TIMEOUTS.replace(b"\n", b" ") +
                    b"INSERT INTO public.c4_import_probe(id,label) VALUES(3,'eof-rollback'); " +
                    receipt_sql("READY", nonce))
        require(parse_writer_line(client.line(), nonce)[0] == "READY", "Protocol")
        client.still_open()
        require(client.finish() == b"", "Protocol")
    require(run_fixed(writer, READ_ROWS, timeout=45, stdout_limit=MAX_WRITER) == b"1|alpha\n2|beta\n", "Fixture")
    dump_command = client_prefix(source_id) + ["/usr/lib/postgresql/18/bin/pg_dump",
        "--format=custom", "--no-owner", "--no-acl", "--table=public.c4_import_probe",
        "--no-password", "--host=/var/run/postgresql", "--port=5432",
        "--username=learning_admin", "--dbname=" + database]
    dump = run_fixed(dump_command)
    require(0 < len(dump) <= MAX_INPUT and dump.startswith(b"PGDMP"), "Fixture")
    sql = run_fixed(decoder_command(source_id), dump)
    require(0 < len(sql) <= MAX_INPUT, "Fixture")
    return {"dump": dump, "sql": sql, "source_eof_insert_rolled_back": True,
            "source_rows_verified": True, "not_golden": True}
