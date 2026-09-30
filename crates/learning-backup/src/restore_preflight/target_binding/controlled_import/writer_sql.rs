//! Fixed SQL only; golden bytes and validated internal identity are the inputs.
use super::{
    ImportFailure,
    fixture_sql::VerifiedFixtureSql,
    protocol::{Nonce, WriterExpected, WriterIdentity},
};

pub(super) fn prelude(
    expected: &WriterExpected,
    sql: &VerifiedFixtureSql,
) -> Result<Vec<u8>, ImportFailure> {
    let mut bytes = b"BEGIN;\n".to_vec();
    bytes.extend_from_slice(sql.header());
    bytes.extend_from_slice(assertion(&identity_predicate(expected), "IDENTITY").as_bytes());
    bytes.extend_from_slice(assertion(&timeout_predicate(), "SESSION").as_bytes());
    bytes.extend_from_slice(receipt(expected, "READY").as_bytes());
    Ok(bytes)
}
pub(super) fn postcheck(expected: &WriterExpected, writer: &WriterIdentity) -> Vec<u8> {
    let (pid, start, xid) = writer.parts();
    let same_writer = format!(
        "{} AND pg_catalog.pg_backend_pid() = {pid} AND (SELECT (EXTRACT(EPOCH FROM a.backend_start)*1000000)::bigint FROM pg_catalog.pg_stat_activity a WHERE a.pid=pg_catalog.pg_backend_pid()) = {start} AND pg_catalog.pg_current_xact_id()::text = '{xid}'",
        identity_predicate(expected)
    );
    format!(
        "{}{}{}{}",
        assertion(&same_writer, "IDENTITY"),
        assertion(&timeout_predicate(), "SESSION"),
        assertion(content_predicate(), "FIXTURE"),
        receipt(expected, "PRECOMMIT")
    )
    .into_bytes()
}
pub(super) fn commit_confirmation(nonce: &Nonce) -> Vec<u8> {
    format!("SELECT 'KW_C4|{}|COMMITTED';\n", nonce.hex()).into_bytes()
}

fn assertion(predicate: &str, reason: &str) -> String {
    format!(
        "DO $kw$ BEGIN IF NOT COALESCE(({predicate}), false) THEN RAISE EXCEPTION 'KW_C4_{reason}'; END IF; END $kw$;\n"
    )
}

// Also used by a fresh, independently bound read-only observer.
pub(super) fn identity_predicate(expected: &WriterExpected) -> String {
    let (db, oid, sid, pid, keys, _) = expected.parts();
    let [first, second] = keys.values();
    let (a, b) = super::super::lock_halves(first);
    let (c, d) = super::super::lock_halves(second);
    format!(
        "current_user::text = 'learning_admin' AND session_user::text = 'learning_admin' AND pg_catalog.current_database() = '{db}' AND (SELECT d.oid::bigint FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database()) = {oid} AND (SELECT pcs.system_identifier::text FROM pg_catalog.pg_control_system() pcs) = '{sid}' AND pg_catalog.pg_backend_pid() <> {pid} AND (SELECT pg_catalog.count(*) = 2 AND pg_catalog.bool_and(l.pid = {pid} AND l.database::bigint = {oid} AND l.objsubid = 1 AND l.mode = 'ExclusiveLock' AND l.granted) FROM pg_catalog.pg_locks l WHERE l.locktype='advisory' AND (l.classid::bigint,l.objid::bigint) IN (({a},{b}),({c},{d})))"
    )
}

fn timeout_predicate() -> String {
    [
        ("statement_timeout", "10s"),
        ("lock_timeout", "5s"),
        ("idle_in_transaction_session_timeout", "30s"),
        ("transaction_timeout", "1min"),
    ]
    .into_iter()
    .map(|(name, value)| format!("pg_catalog.current_setting('{name}') = '{value}'"))
    .collect::<Vec<_>>()
    .join(" AND ")
}

pub(super) fn content_predicate() -> &'static str {
    "(SELECT pg_catalog.count(*) = 2 AND pg_catalog.bool_and((id=1 AND label='alpha') OR (id=2 AND label='beta')) AND pg_catalog.count(DISTINCT id)=2 FROM public.c4_import_probe) AND (SELECT pg_catalog.count(*)=2 AND pg_catalog.bool_and((a.attnum=1 AND a.attname='id' AND a.atttypid='pg_catalog.int4'::pg_catalog.regtype AND a.attnotnull) OR (a.attnum=2 AND a.attname='label' AND a.atttypid='pg_catalog.text'::pg_catalog.regtype AND a.attnotnull)) FROM pg_catalog.pg_attribute a WHERE a.attrelid='public.c4_import_probe'::pg_catalog.regclass AND a.attnum>0 AND NOT a.attisdropped) AND (SELECT pg_catalog.count(*)=1 AND pg_catalog.bool_and(c.contype='p' AND c.conname='c4_import_probe_pkey' AND c.conkey=ARRAY[1]::smallint[] AND c.convalidated) FROM pg_catalog.pg_constraint c WHERE c.conrelid='public.c4_import_probe'::pg_catalog.regclass)"
}

fn receipt(expected: &WriterExpected, stage: &str) -> String {
    format!(
        "SELECT 'KW_C4|{}|{stage}|' || pg_catalog.pg_backend_pid()::text || '|' || (EXTRACT(EPOCH FROM a.backend_start)*1000000)::bigint::text || '|' || pg_catalog.pg_current_xact_id()::text FROM pg_catalog.pg_stat_activity a WHERE a.pid=pg_catalog.pg_backend_pid();\n",
        expected.parts().5.hex()
    )
}

#[cfg(test)]
mod tests {
    use super::super::{
        super::ChallengeKeys,
        fixture_sql::verify_fixture_sql,
        protocol::{WriterEvent, parse_writer_line},
    };
    use super::*;
    const DB: &str = "learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3";
    pub(in super::super) fn sql() -> VerifiedFixtureSql {
        let bytes =
            include_str!("../../../../tests/fixtures/c4-controlled-import/pg18-fixture.sql.in")
                .replace("{{RESTRICT_KEY}}", "TestKey99");
        verify_fixture_sql(bytes.as_bytes()).unwrap()
    }
    fn expected(db: &str, sid: &str, pid: i32) -> Result<WriterExpected, ImportFailure> {
        WriterExpected::new(
            db.into(),
            16384,
            sid,
            pid,
            ChallengeKeys::for_test(1, -1),
            Nonce::random().unwrap(),
        )
    }
    #[test]
    fn writer_rejects_identity_before_marker() {
        for (db, sid, pid) in [
            ("host=evil", "1234", 10),
            (DB, "01234", 10),
            (DB, "0", 10),
            (DB, "+1234", 10),
            (DB, "1234", 0),
        ] {
            assert!(
                matches!(expected(db, sid, pid), Err(ImportFailure::Identity)),
                "invalid identity admitted"
            );
        }
        let expected = expected(DB, "1234", 10).unwrap();
        let bytes = prelude(&expected, &sql()).unwrap();
        let out = String::from_utf8(bytes).unwrap();
        assert!(out.starts_with("BEGIN;\n"));
        assert!(
            out.find("RAISE EXCEPTION 'KW_C4_IDENTITY'").unwrap() < out.find("|READY|").unwrap()
        );
        assert!(!out.contains("CREATE TABLE"));
        assert!(!out.contains("COMMIT;"));
    }
    #[test]
    fn writer_requires_original_control_pid_locks_and_distinct_writer() {
        let expected = expected(DB, "1234", 10).unwrap();
        let prelude = String::from_utf8(prelude(&expected, &sql()).unwrap()).unwrap();
        for required in [
            "pg_catalog.pg_backend_pid() <> 10",
            "l.pid = 10",
            "l.database::bigint = 16384",
            "l.objsubid = 1",
            "l.mode = 'ExclusiveLock'",
            "l.granted",
            "(0,1)",
            "(4294967295,4294967295)",
            "pg_catalog.current_setting('transaction_timeout')",
        ] {
            assert!(
                prelude.contains(required),
                "missing writer assertion: {required}"
            );
        }
        let nonce = expected.parts().5;
        let WriterEvent::Ready(writer) = parse_writer_line(
            format!("KW_C4|{}|READY|42|1234567|99\n", nonce.hex()).as_bytes(),
            &nonce,
        )
        .unwrap() else {
            panic!("ready")
        };
        let post = String::from_utf8(postcheck(&expected, &writer)).unwrap();
        for required in [
            "pg_catalog.pg_backend_pid() = 42",
            "1234567",
            "pg_catalog.pg_current_xact_id()::text = '99'",
            "public.c4_import_probe",
            "c4_import_probe_pkey",
            "|PRECOMMIT|",
        ] {
            assert!(
                post.contains(required),
                "missing postcheck assertion: {required}"
            );
        }
        assert!(!post.contains("COMMIT;"));
        let confirmation = String::from_utf8(commit_confirmation(&nonce)).unwrap();
        assert_eq!(
            confirmation,
            format!("SELECT 'KW_C4|{}|COMMITTED';\n", nonce.hex())
        );
    }
}
