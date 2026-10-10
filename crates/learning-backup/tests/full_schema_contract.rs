//! Tests consume only the reviewed, actually generated embedded PG18 contract.
use learning_backup::{MigrationRecord, full_restore::SchemaContract};
use std::collections::BTreeSet;

#[test]
fn full_schema_contract_contains_every_migrated_object_and_table() {
    let contract = SchemaContract::embedded().expect("actual fixed PG18 contract");
    let names: Vec<_> = contract.table_names().collect();
    assert_eq!(names.len(), 64);
    let mut every = BTreeSet::from(["_sqlx_migrations".to_owned()]);
    for migration in learning_db::MIGRATOR.iter() {
        for line in migration.sql.lines() {
            if let Some(rest) = line.trim_start().strip_prefix("CREATE TABLE ") {
                every.insert(
                    rest.split(|c: char| c.is_whitespace() || c == '(')
                        .next()
                        .unwrap()
                        .trim_start_matches("public.")
                        .to_owned(),
                );
            }
        }
    }
    assert_eq!(every, names.iter().map(|s| s.to_string()).collect());
    let entries: Vec<_> = contract.toc_entries().collect();
    for table in &every {
        assert!(
            entries
                .iter()
                .any(|row| row.starts_with(&format!("TABLE public {table} "))),
            "table missing from TOC: {table}"
        );
    }
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.starts_with("FUNCTION public "))
            .count(),
        46
    );
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.starts_with("TRIGGER public "))
            .count(),
        30
    );
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.starts_with("VIEW public "))
            .count(),
        1
    );
    assert!(
        entries
            .iter()
            .filter(|e| e.starts_with("INDEX public "))
            .count()
            >= 26
    );
    assert_eq!(
        contract.acl_catalog()["schemas"][0]["owner"],
        "pg_database_owner"
    );
    assert_eq!(
        contract.acl_catalog()["database"]["owner"],
        "learning_admin"
    );
}

#[test]
fn full_schema_migration_fingerprint_matches_all_fifteen_versions() {
    let contract = SchemaContract::embedded().expect("actual fixed PG18 contract");
    let expected: Vec<_> = learning_db::MIGRATOR
        .iter()
        .map(|m| MigrationRecord {
            version: m.version as u64,
            checksum_hex: hex::encode(&m.checksum),
        })
        .collect();
    assert_eq!(contract.migrations(), expected);
    assert_eq!(
        expected.iter().map(|m| m.version).collect::<Vec<_>>(),
        (1..=15).collect::<Vec<_>>()
    );
    contract.validate_migrations(&expected).unwrap();
    assert!(contract.validate_migrations(&expected[1..]).is_err());
    let mut changed = expected.clone();
    changed[1].checksum_hex = "0".repeat(96);
    assert!(contract.validate_migrations(&changed).is_err());
}
