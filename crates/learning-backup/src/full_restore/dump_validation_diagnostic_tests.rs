//! C1 observation readiness; these are grammar/owner tests, not restore credit.
use super::dump_observer::Observer;
use super::rehearsal_diagnostic::Diagnostic;
use super::*;
use std::io::{Cursor, Write};

fn emitted<T>(diagnostic: &Diagnostic, result: Result<T, BackupError>) -> String {
    let mut bytes = Vec::new();
    assert!(diagnostic.public_result_to(result, &mut bytes).is_err());
    String::from_utf8(bytes).unwrap()
}

#[test]
fn toc_rejection_keeps_original_error_and_actual_format_guard() {
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let contract = SchemaContract::embedded().unwrap();
    let result = schema::validate_toc_observed(b"malformed\n", &contract, Some(&observer));
    assert!(matches!(
        result,
        Err(BackupError::Invalid("full schema contract"))
    ));
    assert_eq!(
        emitted(&diagnostic, result),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=SCHEMA_CONTRACT_REJECTED__TOC_VALIDATION__TOC_FORMAT\n"
    );
}

#[test]
fn owned_rejection_keeps_original_error_and_actual_preamble_guard() {
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let contract = SchemaContract::embedded().unwrap();
    let result =
        schema::validate_owned_observed(&mut Cursor::new([b'X'; 40]), &contract, Some(&observer));
    assert!(matches!(
        result,
        Err(BackupError::Invalid("full schema contract"))
    ));
    assert_eq!(
        emitted(&diagnostic, result),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=SCHEMA_CONTRACT_REJECTED__OWNED_SCHEMA_VALIDATION__PREAMBLE\n"
    );
}

#[test]
fn sql_rejection_keeps_original_error_and_actual_preamble_guard() {
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let contract = SchemaContract::embedded().unwrap();
    let result =
        schema::validate_sql_observed(&mut Cursor::new([b'X'; 40]), &contract, Some(&observer));
    assert!(matches!(
        result,
        Err(BackupError::Invalid("full schema contract"))
    ));
    assert_eq!(
        emitted(&diagnostic, result),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=SCHEMA_CONTRACT_REJECTED__SQL_VALIDATION__PREAMBLE\n"
    );
}

#[tokio::test]
async fn real_decoder_owner_receives_this_case_observer_before_version_rejection() {
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let result = spool::decode_owner(Some(observer), |sender, observer| async move {
        let result = spool::require_decoder_version(b"secret invalid version", observer.as_ref())
            .map(|()| unreachable!("invalid version must reject"));
        (sender, result)
    })
    .await;
    assert!(matches!(
        result,
        Err(BackupError::Invalid("fixed PG18 decoder"))
    ));
    assert_eq!(
        emitted(&diagnostic, result),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=FIXED_DECODER_REJECTED__VERSION_BYTES__VERSION_BYTES\n"
    );
}

fn file_with(raw: &[u8]) -> File {
    let path = std::env::temp_dir().join(format!("kw-c1-{}.fixture", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.write_all(raw).unwrap();
    file.sync_all().unwrap();
    drop(file);
    File::open(path).unwrap()
}

#[tokio::test]
async fn real_validator_blocking_task_receives_this_case_observer_before_toc_rejection() {
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let decoded = spool::Decoded {
        sql: file_with(b""),
        owned: file_with(b""),
        toc: file_with(b"malformed\n"),
    };
    let result = validate_decoded_task(
        decoded,
        SchemaContract::embedded().unwrap(),
        Instant::now() + Duration::from_secs(30),
        Some(observer),
    )
    .await;
    assert!(matches!(
        result,
        Err(BackupError::Invalid("full schema contract"))
    ));
    assert_eq!(
        emitted(&diagnostic, result),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=SCHEMA_CONTRACT_REJECTED__TOC_VALIDATION__TOC_FORMAT\n"
    );
}

#[test]
fn client_hash_rejection_uses_the_actual_held_file_guard() {
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let result =
        spool::require_client_hash(&mut file_with(b"not the pinned client"), Some(&observer));
    assert!(matches!(
        result,
        Err(BackupError::Invalid("fixed PG18 decoder"))
    ));
    assert_eq!(
        emitted(&diagnostic, result),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=FIXED_DECODER_REJECTED__DECODER_CLIENT__CLIENT_HASH\n"
    );
}

fn exit(code: u32) -> std::process::ExitStatus {
    // Real OS wait statuses for the predicate unit inputs. These fixed shell
    // exits are not PG18 execution or a decoder native-wait receipt.
    let code = match code {
        0 => "0",
        2 => "2",
        9 => "9",
        _ => panic!("fixed unit status only"),
    };
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new(r"C:\Windows\System32\cmd.exe");
        command.args(["/d", "/c", "exit", code]);
        command
    };
    #[cfg(unix)]
    let mut command = {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", &format!("exit {code}")]);
        command
    };
    let output = command.stdin(std::process::Stdio::null()).output().unwrap();
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    output.status
}

#[test]
fn native_nonzero_predicate_survives_a_different_successful_reap() {
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let result = spool::native_predicate(exit(2), false, spool::Operation::Toc, Some(&observer));
    let receipt = spool::native_observation::wait_receipt(123, Some(exit(2)), exit(0), &[]);
    assert_eq!(receipt["predicate_status"]["success"], false);
    assert_eq!(receipt["reap_status"]["success"], true);
    assert!(matches!(
        result,
        Err(BackupError::Invalid("pg_restore decode exit"))
    ));
    assert_eq!(
        emitted(&diagnostic, result),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=DECODER_NATIVE_REJECTED__TOC_RUN__NATIVE_PREDICATE__TOC__WAIT_OBSERVED__NONZERO__STDERR_EMPTY\n"
    );
}

#[test]
fn native_stderr_only_rejection_keeps_successful_predicate_separate_from_reap() {
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let result = spool::native_predicate(exit(0), true, spool::Operation::Version, Some(&observer));
    let receipt = spool::native_observation::wait_receipt(
        123,
        Some(exit(0)),
        exit(9),
        b"secret-like original stderr",
    );
    assert_eq!(receipt["predicate_status"]["success"], true);
    assert_eq!(receipt["reap_status"]["success"], false);
    assert!(matches!(
        result,
        Err(BackupError::Invalid("pg_restore decode exit"))
    ));
    assert_eq!(
        emitted(&diagnostic, result),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=DECODER_NATIVE_REJECTED__VERSION_RUN__NATIVE_PREDICATE__VERSION__WAIT_OBSERVED__SUCCESS__STDERR_NONEMPTY\n"
    );
}

#[test]
fn native_cancel_deadline_and_io_without_predicate_never_manufacture_wait_proof() {
    for (error, prefix) in [
        (
            BackupError::Invalid("decoder cancelled"),
            "DECODER_CANCELLED",
        ),
        (
            BackupError::Invalid("full decode deadline"),
            "DECODE_DEADLINE",
        ),
        (
            BackupError::Io(std::io::Error::from_raw_os_error(21)),
            "ERRNO_21",
        ),
    ] {
        let diagnostic = Diagnostic::new();
        let observer = Observer::new(diagnostic.clone());
        let result: Result<(), BackupError> = spool::retain_native_result::<()>(
            Err(error),
            spool::Operation::Decode,
            None,
            false,
            Some(&observer),
        );
        match &result {
            Err(BackupError::Invalid("decoder cancelled" | "full decode deadline")) => {}
            Err(BackupError::Io(error)) => assert_eq!(error.raw_os_error(), Some(21)),
            _ => panic!("original native error was changed"),
        }
        let output = emitted(&diagnostic, result);
        assert!(output.contains(&format!("code={prefix}__DECODE_RUN__NATIVE_RUN__DECODE__WAIT_NOT_OBSERVED__STATUS_NOT_OBSERVED__STDERR_EMPTY\n")));
        assert!(!output.contains("__SUCCESS"));
        assert!(!output.contains("__WAIT_OBSERVED"));
    }
}

#[test]
fn closed_and_expired_native_prechecks_keep_original_error_and_no_wait_claim() {
    for (closed, expired, guard) in [
        (true, false, "CLOSED_BEFORE_RUN"),
        (false, true, "DEADLINE_BEFORE_RUN"),
        (true, true, "CLOSED_BEFORE_RUN"),
    ] {
        let diagnostic = Diagnostic::new();
        let observer = Observer::new(diagnostic.clone());
        let result =
            spool::require_run_ready(closed, expired, spool::Operation::Version, Some(&observer));
        assert!(matches!(
            result,
            Err(BackupError::Invalid("fixed PG18 decoder"))
        ));
        assert_eq!(
            emitted(&diagnostic, result),
            format!(
                "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=FIXED_DECODER_REJECTED__VERSION_RUN__{guard}\n"
            )
        );
    }
}

#[test]
fn missing_native_status_rejects_without_manufacturing_predicate_proof() {
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let result =
        spool::require_native_status(None, spool::Operation::OwnedSchema, false, Some(&observer));
    assert!(matches!(
        result,
        Err(BackupError::Invalid("fixed PG18 decoder"))
    ));
    assert_eq!(
        emitted(&diagnostic, result),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=FIXED_DECODER_REJECTED__OWNED_SCHEMA_RUN__NATIVE_HANDLE_MISSING__OWNED_SCHEMA__WAIT_NOT_OBSERVED__STATUS_NOT_OBSERVED__STDERR_EMPTY\n"
    );
}

#[test]
fn successful_native_predicate_emits_nothing() {
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let result = spool::native_predicate(exit(0), false, spool::Operation::Decode, Some(&observer));
    let mut output = Vec::new();
    diagnostic.public_result_to(result, &mut output).unwrap();
    assert!(output.is_empty());
}

fn valid_toc(contract: &SchemaContract) -> Vec<u8> {
    let header = contract.toc["header"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let mut raw = format!(
        ";\n; Archive created at 2026-10-09 01:02:03 UTC\n;     dbname: c1_fixture\n{header}\n"
    );
    for (i, entry) in contract.toc["entries"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let (catalog, name) = entry.as_str().unwrap().split_once(' ').unwrap();
        raw.push_str(&format!("{}; {catalog} {} {name}\n", i + 1, i + 1000));
    }
    raw.into_bytes()
}

fn history() -> String {
    learning_db::MIGRATOR
        .iter()
        .map(|m| {
            format!(
                "{}\t{}\t2026-01-01 00:00:00+00\tt\t\\\\x{}\t0\n",
                m.version,
                m.description,
                hex::encode(&m.checksum)
            )
        })
        .collect()
}

fn valid_sql(contract: &SchemaContract) -> String {
    let rows = history();
    let mut sql = String::from_utf8(contract.template.clone())
        .unwrap()
        .replace("{{RESTRICT_KEY}}", "c1fixturekey");
    for table in contract.table_names() {
        sql = sql.replace(
            &format!("{{{{COPY:{table}}}}}"),
            if table == "_sqlx_migrations" {
                &rows
            } else {
                ""
            },
        );
    }
    sql
}

fn valid_owned(contract: &SchemaContract) -> Vec<u8> {
    String::from_utf8(contract.owned_template.clone())
        .unwrap()
        .replace("{{RESTRICT_KEY}}", "c1fixturekey")
        .into_bytes()
}

fn decoded_fixture(contract: &SchemaContract) -> spool::Decoded {
    spool::Decoded {
        sql: file_with(valid_sql(contract).as_bytes()),
        owned: file_with(&valid_owned(contract)),
        toc: file_with(&valid_toc(contract)),
    }
}

#[test]
fn embedded_contract_shared_loader_succeeds_without_observation_output() {
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let contract = SchemaContract::embedded_observed(Some(&observer)).unwrap();
    assert_eq!(contract.table_names().count(), 64);
    assert_eq!(contract.migrations().len(), 15);
    let mut output = Vec::new();
    diagnostic.public_result_to(Ok(()), &mut output).unwrap();
    assert!(output.is_empty());
}

#[test]
fn toc_date_database_header_entry_and_set_guards_come_from_actual_branches() {
    let contract = SchemaContract::embedded().unwrap();
    let raw = String::from_utf8(valid_toc(&contract)).unwrap();
    schema::validate_toc(raw.as_bytes(), &contract).unwrap();
    for (changed, guard) in [
        (raw.replacen("2026-10-09", "2026-XX-09", 1), "TOC_DATE"),
        (
            raw.replacen("dbname: c1_fixture", "dbname: bad name", 1),
            "TOC_DATABASE",
        ),
        (
            raw.replacen("Compression:", "CompressionX", 1),
            "TOC_HEADER",
        ),
        (raw.replacen("\n1; ", "\n01; ", 1), "TOC_ENTRY"),
        (
            format!("{raw}99999; 0 0 COMMENT public c1_extra learning_admin\n"),
            "TOC_SET",
        ),
    ] {
        assert_ne!(changed, raw);
        let diagnostic = Diagnostic::new();
        let observer = Observer::new(diagnostic.clone());
        let result = schema::validate_toc_observed(changed.as_bytes(), &contract, Some(&observer));
        assert!(matches!(
            result,
            Err(BackupError::Invalid("full schema contract"))
        ));
        assert_eq!(
            emitted(&diagnostic, result),
            format!(
                "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=SCHEMA_CONTRACT_REJECTED__TOC_VALIDATION__{guard}\n"
            )
        );
    }
}

#[test]
fn owned_key_fixed_template_and_trailing_guards_come_from_actual_branches() {
    let contract = SchemaContract::embedded().unwrap();
    let raw = String::from_utf8(valid_owned(&contract)).unwrap();
    schema::validate_owned(&mut Cursor::new(raw.as_bytes()), &contract).unwrap();
    for (changed, guard) in [
        (
            raw.replacen("\\restrict c1fixturekey", "\\restrict c1_fixturekey", 1),
            "RESTRICT_KEY",
        ),
        (
            raw.replacen(
                "SET statement_timeout = 0;",
                "SET statement_timeout = 1;",
                1,
            ),
            "TEMPLATE_BYTES",
        ),
        (format!("{raw}x"), "TRAILING_BYTES"),
    ] {
        assert_ne!(changed, raw);
        let diagnostic = Diagnostic::new();
        let observer = Observer::new(diagnostic.clone());
        let result = schema::validate_owned_observed(
            &mut Cursor::new(changed.as_bytes()),
            &contract,
            Some(&observer),
        );
        assert!(matches!(
            result,
            Err(BackupError::Invalid("full schema contract"))
        ));
        assert_eq!(
            emitted(&diagnostic, result),
            format!(
                "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=SCHEMA_CONTRACT_REJECTED__OWNED_SCHEMA_VALIDATION__{guard}\n"
            )
        );
    }
}

#[test]
fn sql_fixed_copy_header_postamble_and_trailing_guards_come_from_actual_branches() {
    let contract = SchemaContract::embedded().unwrap();
    let raw = valid_sql(&contract);
    for (changed, guard) in [
        (
            raw.replacen("SECURITY DEFINER", "SECURITY INVOKER", 1),
            "SQL_FIXED_SCHEMA",
        ),
        (
            raw.replacen(" FROM stdin;", " FROM PROGRAM 'bad';", 1),
            "COPY_HEADER",
        ),
        (
            raw.replacen("\\unrestrict c1fixturekey", "\\unrestrict d1fixturekey", 1),
            "SQL_POSTAMBLE",
        ),
        (format!("{raw}x"), "TRAILING_BYTES"),
    ] {
        assert_ne!(changed, raw);
        let diagnostic = Diagnostic::new();
        let observer = Observer::new(diagnostic.clone());
        let result = schema::validate_sql_observed(
            &mut Cursor::new(changed.as_bytes()),
            &contract,
            Some(&observer),
        );
        assert!(matches!(
            result,
            Err(BackupError::Invalid("full schema contract"))
        ));
        assert_eq!(
            emitted(&diagnostic, result),
            format!(
                "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=SCHEMA_CONTRACT_REJECTED__SQL_VALIDATION__{guard}\n"
            )
        );
    }
}

#[test]
fn actual_copy_frame_utf8_and_migration_count_failures_keep_original_reasons() {
    let contract = SchemaContract::embedded().unwrap();
    let raw = valid_sql(&contract);
    let rows = history();
    let mut invalid_utf8 = raw.as_bytes().to_vec();
    invalid_utf8[raw.find(&rows).unwrap()] = 0xff;
    let extra_rows = format!("{rows}{}\n", rows.lines().next().unwrap());
    for (changed, reason, code) in [
        (
            raw.replacen(&rows, "wrong field count\n", 1).into_bytes(),
            "COPY text frame",
            "COPY_FRAME_REJECTED",
        ),
        (invalid_utf8, "COPY UTF-8", "COPY_UTF8_REJECTED"),
        (
            raw.replacen(&rows, &extra_rows, 1).into_bytes(),
            "migration row count",
            "MIGRATION_ROW_COUNT_REJECTED",
        ),
    ] {
        let diagnostic = Diagnostic::new();
        let observer = Observer::new(diagnostic.clone());
        let result =
            schema::validate_sql_observed(&mut Cursor::new(changed), &contract, Some(&observer));
        assert!(matches!(&result, Err(BackupError::Invalid(original)) if *original == reason));
        assert_eq!(
            emitted(&diagnostic, result),
            format!(
                "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code={code}__COPY_VALIDATION__COPY_FRAME\n"
            )
        );
    }
}

#[test]
fn actual_migration_identity_guard_does_not_become_generic_sql_guard() {
    let contract = SchemaContract::embedded().unwrap();
    let raw = valid_sql(&contract).replacen("\\\\x", "\\\\y", 1);
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let result = schema::validate_sql_observed(&mut Cursor::new(raw), &contract, Some(&observer));
    assert!(matches!(
        result,
        Err(BackupError::Invalid("full schema contract"))
    ));
    assert_eq!(
        emitted(&diagnostic, result),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=SCHEMA_CONTRACT_REJECTED__MIGRATION_VALIDATION__MIGRATION_IDENTITY\n"
    );
}

#[tokio::test]
async fn actual_complete_validator_success_is_quiet_and_keeps_checked_ranges() {
    let contract = SchemaContract::embedded().unwrap();
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let result = validate_decoded_task(
        decoded_fixture(&contract),
        contract,
        Instant::now() + Duration::from_secs(30),
        Some(observer),
    )
    .await;
    let mut output = Vec::new();
    let verified = diagnostic.public_result_to(result, &mut output).unwrap();
    assert_eq!(verified.segments().count(), 66);
    assert_eq!(
        verified
            .segments()
            .filter(|s| matches!(s.kind(), SegmentKind::Copy { .. }))
            .count(),
        64
    );
    assert!(output.is_empty());
}

#[tokio::test]
async fn concurrent_owner_and_blocking_cases_keep_their_own_observers() {
    let first = Diagnostic::new();
    let second = Diagnostic::new();
    let owner = spool::decode_owner(
        Some(Observer::new(first.clone())),
        |sender, observer| async move {
            tokio::task::yield_now().await;
            let result =
                spool::require_decoder_version(b"wrong version for first case", observer.as_ref())
                    .map(|()| unreachable!());
            (sender, result)
        },
    );
    let decoded = spool::Decoded {
        sql: file_with(b""),
        owned: file_with(b""),
        toc: file_with(b"wrong toc\n"),
    };
    let validator = validate_decoded_task(
        decoded,
        SchemaContract::embedded().unwrap(),
        Instant::now() + Duration::from_secs(30),
        Some(Observer::new(second.clone())),
    );
    let (a, b) = tokio::join!(owner, validator);
    assert_eq!(
        emitted(&first, a),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=FIXED_DECODER_REJECTED__VERSION_BYTES__VERSION_BYTES\n"
    );
    assert_eq!(
        emitted(&second, b),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=SCHEMA_CONTRACT_REJECTED__TOC_VALIDATION__TOC_FORMAT\n"
    );
}

#[tokio::test]
async fn normal_owner_and_validator_calls_have_no_observer() {
    let diagnostic = Diagnostic::new();
    let result = spool::decode_owner(None::<Observer>, |sender, observer| async move {
        let result = spool::require_decoder_version(b"not a valid version", observer.as_ref())
            .map(|()| unreachable!());
        (sender, result)
    })
    .await;
    assert!(matches!(
        result,
        Err(BackupError::Invalid("fixed PG18 decoder"))
    ));
    let contract = SchemaContract::embedded().unwrap();
    validate_decoded_task(
        decoded_fixture(&contract),
        contract,
        Instant::now() + Duration::from_secs(30),
        None,
    )
    .await
    .unwrap();
    let mut output = Vec::new();
    diagnostic.public_result_to(Ok(()), &mut output).unwrap();
    assert!(output.is_empty());
}

#[test]
fn native_primary_is_retained_before_missing_observation_and_isolation_secondary() {
    use super::rehearsal_diagnostic::Stage;
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let original = spool::native_predicate(exit(2), false, spool::Operation::Toc, Some(&observer));
    let result = diagnostic.observe::<(), ()>(original, None, true);
    assert!(matches!(
        result,
        Err(BackupError::Invalid("pg_restore decode exit"))
    ));
    let isolation = diagnostic.settle(Stage::Stop, || {
        Err::<(), _>(BackupError::Invalid("full target isolation unconfirmed"))
    });
    assert!(matches!(
        isolation,
        Err(BackupError::Invalid("full target isolation unconfirmed"))
    ));
    assert_eq!(
        emitted(&diagnostic, isolation),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=DECODER_NATIVE_REJECTED__TOC_RUN__NATIVE_PREDICATE__TOC__WAIT_OBSERVED__NONZERO__STDERR_EMPTY\nFULL_REHEARSAL_FAILURE kind=SECONDARY stage=OBSERVATION class=INVALID code=OBSERVATION_MISSING\nFULL_REHEARSAL_FAILURE kind=SECONDARY stage=STOP class=INVALID code=ISOLATION_UNCONFIRMED\n"
    );
}

#[test]
fn accepted_negative_with_actual_schema_refusal_remains_quiet() {
    use super::rehearsal_diagnostic::Stage;
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let original = schema::validate_toc_observed(
        b"malformed\n",
        &SchemaContract::embedded().unwrap(),
        Some(&observer),
    );
    let (result, observed) = diagnostic.observe(original, Some(7), true).unwrap();
    assert!(matches!(
        result,
        Err(BackupError::Invalid("full schema contract"))
    ));
    assert_eq!(observed, Some(7));
    diagnostic.settle(Stage::NegativeAudit, || Ok(())).unwrap();
    diagnostic.require_settlement().unwrap();
    let mut output = Vec::new();
    diagnostic.public_result_to(Ok(()), &mut output).unwrap();
    assert!(output.is_empty());
}

#[test]
fn only_the_twelve_approved_new_reasons_are_added_to_the_safe_allowlist() {
    use super::rehearsal_diagnostic::Stage;
    for (reason, code) in [
        ("full schema contract", "SCHEMA_CONTRACT_REJECTED"),
        ("fixed PG18 decoder", "FIXED_DECODER_REJECTED"),
        ("decoder supervisor lost", "DECODER_OWNER_LOST"),
        ("decoder cancelled", "DECODER_CANCELLED"),
        ("pg_restore decode exit", "DECODER_NATIVE_REJECTED"),
        ("full decode deadline", "DECODE_DEADLINE"),
        ("spool identity", "SPOOL_IDENTITY_REJECTED"),
        ("full validator task lost", "VALIDATOR_TASK_LOST"),
        ("COPY text frame", "COPY_FRAME_REJECTED"),
        ("COPY UTF-8", "COPY_UTF8_REJECTED"),
        ("migration row count", "MIGRATION_ROW_COUNT_REJECTED"),
        ("checked range coverage", "RANGE_COVERAGE_REJECTED"),
    ] {
        let diagnostic = Diagnostic::new();
        let result = diagnostic.run(Stage::DumpValidation, || {
            Err::<(), _>(BackupError::Invalid(reason))
        });
        assert_eq!(
            emitted(&diagnostic, result),
            format!(
                "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code={code}\n"
            )
        );
    }
}

#[test]
fn unknown_secret_like_messages_stay_unknown_even_with_actual_observer_context() {
    use super::dump_observer::{Guard, Step};
    for (error, class) in [
        (
            BackupError::Invalid("SELECT secret; postgres://user:password@host/private/path"),
            "INVALID",
        ),
        (
            BackupError::Capacity("role=secret stdout dump bytes"),
            "CAPACITY",
        ),
    ] {
        let diagnostic = Diagnostic::new();
        let observer = Observer::new(diagnostic.clone());
        observer.failed(Step::SqlValidation, Guard::SqlFixedSchema, &error, None);
        let output = emitted::<()>(&diagnostic, Err(error));
        assert_eq!(
            output,
            format!(
                "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class={class} code=UNKNOWN\n"
            )
        );
    }
}

#[test]
fn all_finite_code_combinations_fit_the_four_field_line_without_truncation() {
    use super::dump_observer::{Guard, Native, Step};
    let mut longest = 0;
    let success = exit(0);
    let nonzero = exit(2);
    for &step in Step::ALL {
        for &guard in Guard::ALL {
            for operation in [
                spool::Operation::Version,
                spool::Operation::Toc,
                spool::Operation::OwnedSchema,
                spool::Operation::Decode,
            ] {
                for status in [None, Some(success), Some(nonzero)] {
                    for stderr in [false, true] {
                        let diagnostic = Diagnostic::new();
                        let observer = Observer::new(diagnostic.clone());
                        let error = BackupError::Invalid("migration row count");
                        observer.failed(
                            step,
                            guard,
                            &error,
                            Some(Native::new(operation, status, stderr)),
                        );
                        let output = emitted::<()>(&diagnostic, Err(error));
                        assert_eq!(output.split_whitespace().count(), 5);
                        assert!(output.len() <= 256);
                        let code = output
                            .split_once(" code=")
                            .unwrap()
                            .1
                            .trim_end_matches('\n');
                        assert!(code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'));
                        assert!(output.ends_with(if stderr {
                            "__STDERR_NONEMPTY\n"
                        } else {
                            "__STDERR_EMPTY\n"
                        }));
                        longest = longest.max(output.len());
                    }
                }
            }
        }
    }
    assert_eq!(longest, 229);
    let diagnostic = Diagnostic::new();
    let error = BackupError::Invalid("migration row count");
    Observer::new(diagnostic.clone()).failed(
        Step::OwnedSchemaValidation,
        Guard::ClientComponentMetadata,
        &error,
        Some(Native::new(spool::Operation::OwnedSchema, None, true)),
    );
    assert_eq!(
        emitted::<()>(&diagnostic, Err(error)),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=MIGRATION_ROW_COUNT_REJECTED__OWNED_SCHEMA_VALIDATION__CLIENT_COMPONENT_METADATA__OWNED_SCHEMA__WAIT_NOT_OBSERVED__STATUS_NOT_OBSERVED__STDERR_NONEMPTY\n"
    );
}

#[test]
fn detailed_primary_and_settlement_failures_still_emit_at_most_five_complete_lines() {
    use super::rehearsal_diagnostic::Stage;
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let original = spool::native_predicate(
        exit(2),
        true,
        spool::Operation::OwnedSchema,
        Some(&observer),
    );
    for stage in [
        Stage::Stop,
        Stage::Observation,
        Stage::Report,
        Stage::Proof,
        Stage::StopInspection,
        Stage::DurableInventory,
    ] {
        let _ = diagnostic.settle(stage, || {
            Err::<(), _>(BackupError::Invalid("full target isolation unconfirmed"))
        });
    }
    let output = emitted(&diagnostic, original);
    assert_eq!(output.lines().count(), 5);
    assert!(output.starts_with("FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=DECODER_NATIVE_REJECTED__OWNED_SCHEMA_RUN__NATIVE_PREDICATE__OWNED_SCHEMA__WAIT_OBSERVED__NONZERO__STDERR_NONEMPTY\n"));
    for line in output.lines() {
        assert_eq!(line.split_whitespace().count(), 5);
        assert!(line.len() < 256);
    }
    assert!(!output.contains("stage=STOP_INSPECTION"));
}

#[cfg(target_os = "linux")]
fn linux_root() -> BackupDir {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!("kw-c1-native-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    BackupDir::open_private_root(&path).unwrap()
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn linux_actual_decode_entry_carries_observer_to_frozen_metadata_rejection() {
    let wait = spool::native_observation::FirstVersion::arm().await;
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let dump = FrozenFullDump {
        file: file_with(b"PGDMP"),
        size: 5,
        sha256: "invalid".into(),
    };
    let result = validate_full_dump_observed(
        dump,
        &SchemaContract::embedded().unwrap(),
        &linux_root(),
        observer,
    )
    .await;
    assert!(wait.drain().is_none());
    assert!(matches!(
        result,
        Err(BackupError::Invalid("fixed PG18 decoder"))
    ));
    assert_eq!(
        emitted(&diagnostic, result),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=FIXED_DECODER_REJECTED__FROZEN_METADATA__FROZEN_METADATA\n"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn linux_actual_version_and_toc_run_retain_the_native_rejection_predicate() {
    let wait = spool::native_observation::FirstVersion::arm().await;
    let raw = b"PGDMPnot-a-valid-pg18-archive";
    let diagnostic = Diagnostic::new();
    let observer = Observer::new(diagnostic.clone());
    let dump = FrozenFullDump {
        file: file_with(raw),
        size: raw.len() as u64,
        sha256: crate::digest(raw),
    };
    let result = validate_full_dump_observed(
        dump,
        &SchemaContract::embedded().unwrap(),
        &linux_root(),
        observer,
    )
    .await;
    let (receipt, stderr) = wait.drain().expect("actual first Version wait required");
    assert_eq!(receipt["operation"], "Version");
    assert_eq!(receipt["wait_completed"], true);
    assert_eq!(receipt["predicate_status"]["success"], true);
    assert_eq!(receipt["reap_status"]["success"], true);
    assert!(stderr.is_empty());
    assert!(matches!(
        result,
        Err(BackupError::Invalid("pg_restore decode exit"))
    ));
    assert_eq!(
        emitted(&diagnostic, result),
        "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=DUMP_VALIDATION class=INVALID code=DECODER_NATIVE_REJECTED__TOC_RUN__NATIVE_PREDICATE__TOC__WAIT_OBSERVED__NONZERO__STDERR_NONEMPTY\n"
    );
}
