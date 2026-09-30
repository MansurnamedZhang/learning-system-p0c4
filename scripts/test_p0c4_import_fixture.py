import sys
import unittest
from unittest.mock import patch
import p0c4_import_fixture as fixture

ID = "a" * 64
DB = "learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3"
NONCE = "abababababababababababababababab"


class FixtureTests(unittest.TestCase):
    def test_rejects_alias_conninfo_and_noncanonical_database(self):
        for cid, db in [("pg", DB), ("A" * 64, DB), (ID, "host=remote"),
                        (ID, DB.upper()), (ID, DB.replace("48aa", "18aa"))]:
            with self.subTest(cid=cid, db=db):
                with self.assertRaises(fixture.ImportRejected):
                    fixture.validate_identity(cid, db)

    def test_fixed_argv_has_disabled_passfile_and_no_automatic_commit(self):
        command = fixture.writer_command(ID, DB)
        self.assertEqual(command, ["/usr/bin/docker", "exec", "--interactive",
            "--user", "999:999", ID, "/usr/bin/env", "-i", "LC_ALL=C",
            "PGCONNECT_TIMEOUT=10", "PGPASSFILE=/dev/null/knowweave-c4-passfile-disabled",
            "/usr/lib/postgresql/18/bin/psql", "-X", "-qAt", "-P", "pager=off",
            "--no-password", "--host=/var/run/postgresql", "--port=5432",
            "--username=learning_admin", "--dbname=" + DB, "-v", "ON_ERROR_STOP=1", "-f", "-"])
        self.assertEqual(fixture.decoder_command(ID)[11:],
            ["/usr/lib/postgresql/18/bin/pg_restore", "--file=-", "--no-owner", "--no-acl", "--exit-on-error"])

    def test_receipt_rejects_replay_noncanonical_and_extra_fields(self):
        line = f"KW_C4|{NONCE}|READY|42|1234567|99\n".encode()
        self.assertEqual(fixture.parse_writer_line(line, NONCE), ("READY", (42, 1234567, 99)))
        for bad in [line.replace(b"42|", b"042|"), line + b"\n", line[:-1],
                    line.replace(b"abab", b"aaaa", 1), line.replace(b"99\n", b"0\n"),
                    line.replace(b"42|", b"2147483648|"), line.replace(b"1234567|", b"9223372036854775808|")]:
            with self.subTest(bad=bad), self.assertRaises(fixture.ImportRejected):
                fixture.parse_writer_line(bad, NONCE)

    def test_contract_does_not_close_stdin_to_obtain_ready(self):
        # A real pipe peer only emits READY/PRECOMMIT after complete input lines;
        # closing stdin before either handshake makes the next stage impossible.
        program = """import sys
for phase in ['READY', 'PRECOMMIT']:
    if not sys.stdin.readline(): sys.exit(8)
    sys.stdout.buffer.write(('KW_C4|abababababababababababababababab|' + phase + '|42|1234567|99\\n').encode()); sys.stdout.flush()
if not sys.stdin.readline(): sys.exit(9)
sys.stdout.buffer.write(b'KW_C4|abababababababababababababababab|ROLLED_BACK\\n'); sys.stdout.flush()
if sys.stdin.read(): sys.exit(10)
"""
        facts = fixture.run_handshake([sys.executable, "-u", "-c", program], NONCE)
        self.assertEqual(facts, {"ready_stdin_open": True, "precommit_stdin_open": True,
                                "same_writer_transaction": True, "rollback_observed": True})

    def test_rejects_changed_writer_before_precommit(self):
        program = """import sys
sys.stdin.readline()
sys.stdout.buffer.write(b'KW_C4|abababababababababababababababab|READY|42|1234567|99\\n'); sys.stdout.flush()
sys.stdin.readline()
sys.stdout.buffer.write(b'KW_C4|abababababababababababababababab|PRECOMMIT|42|1234567|100\\n'); sys.stdout.flush()
sys.stdin.read()
"""
        with self.assertRaisesRegex(fixture.ImportRejected, "Protocol"):
            fixture.run_handshake([sys.executable, "-u", "-c", program], NONCE)

    def test_bounds_stdout_and_stderr_during_reading(self):
        for pipe, code in [("stdout", "StdoutLimit"), ("stderr", "StderrLimit")]:
            with self.subTest(pipe=pipe), self.assertRaisesRegex(fixture.ImportRejected, code):
                fixture.run_fixed([sys.executable, "-u", "-c",
                    f"import sys,time; sys.{pipe}.buffer.write(b'x'*9000); sys.{pipe}.flush(); time.sleep(10)"],
                    timeout=2, stdout_limit=8192, stderr_limit=8192)

    def test_deadline_and_stderr_never_escape_as_raw_diagnostics(self):
        with self.assertRaisesRegex(fixture.ImportRejected, "Deadline"):
            fixture.run_fixed([sys.executable, "-c", "import time; time.sleep(10)"], timeout=0.1)
        with self.assertRaises(fixture.ImportRejected) as failure:
            fixture.run_fixed([sys.executable, "-c", "import sys; sys.stderr.write('password=private')"])
        self.assertEqual(str(failure.exception), "Stderr")

    def test_capture_rejects_unverified_id_before_spawning(self):
        with patch.object(fixture, "run_fixed", side_effect=AssertionError("must not spawn")):
            with self.assertRaises(fixture.ImportRejected):
                fixture.capture_fixture("pg", DB)

    def test_input_and_backpressure_are_bounded(self):
        with self.assertRaisesRegex(fixture.ImportRejected, "InputLimit"):
            fixture.run_fixed([sys.executable, "-c", "import sys; sys.stdin.read()"], b'x' * 65537)
        with self.assertRaisesRegex(fixture.ImportRejected, "Deadline"):
            fixture.run_fixed([sys.executable, "-c", "import time; time.sleep(10)"], b'x' * 65536, timeout=0.1)

    def test_unexpected_trailing_output_rejects_handshake(self):
        program = """import sys
for phase in ['READY', 'PRECOMMIT']:
    sys.stdin.readline()
    sys.stdout.buffer.write(('KW_C4|abababababababababababababababab|' + phase + '|42|1234567|99\\n').encode()); sys.stdout.flush()
sys.stdin.readline()
sys.stdout.buffer.write(b'KW_C4|abababababababababababababababab|ROLLED_BACK\\nextra\\n'); sys.stdout.flush()
sys.stdin.read()
"""
        with self.assertRaisesRegex(fixture.ImportRejected, "Protocol"):
            fixture.run_handshake([sys.executable, "-u", "-c", program], NONCE)

    def test_capture_separates_artifacts_and_requires_post_eof_rows(self):
        # PostgreSQL is unavailable in this local gate: substitute only fixed
        # database command I/O; keep capture sequencing and open pipe real.
        program = """import sys,re
line = sys.stdin.readline()
nonce = re.search(r'KW_C4\\|([a-f0-9]{32})\\|READY', line).group(1)
sys.stdout.buffer.write(('KW_C4|' + nonce + '|READY|42|1234567|99\\n').encode()); sys.stdout.flush()
if sys.stdin.read(): sys.exit(9)
"""
        original = fixture.OpenClient
        outputs = [b'', b'1|alpha\n2|beta\n', b'1|alpha\n2|beta\n', b'PGDMPfixture', b'synthetic SQL, not a golden\n']
        def command_io(*args, **kwargs):
            return outputs.pop(0)
        def local_peer(*args, **kwargs):
            return original([sys.executable, '-u', '-c', program], **kwargs)
        with patch.object(fixture, 'run_fixed', side_effect=command_io), patch.object(fixture, 'OpenClient', side_effect=local_peer):
            captured = fixture.capture_fixture(ID, DB)
        self.assertEqual(captured, {'dump': b'PGDMPfixture', 'sql': b'synthetic SQL, not a golden\n',
            'source_eof_insert_rolled_back': True, 'source_rows_verified': True, 'not_golden': True})

    def test_capture_rejects_failed_independent_eof_readback(self):
        program = """import sys,re
line = sys.stdin.readline()
nonce = re.search(r'KW_C4\\|([a-f0-9]{32})\\|READY', line).group(1)
sys.stdout.buffer.write(('KW_C4|' + nonce + '|READY|42|1234567|99\\n').encode()); sys.stdout.flush()
sys.stdin.read()
"""
        original = fixture.OpenClient
        with patch.object(fixture, 'run_fixed', side_effect=[b'', b'1|alpha\n2|beta\n', b'1|alpha\n2|beta\n3|eof-rollback\n']), patch.object(fixture, 'OpenClient', side_effect=lambda *args, **kwargs: original([sys.executable, '-u', '-c', program], **kwargs)):
            with self.assertRaisesRegex(fixture.ImportRejected, 'Fixture'):
                fixture.capture_fixture(ID, DB)


if __name__ == "__main__":
    unittest.main()
