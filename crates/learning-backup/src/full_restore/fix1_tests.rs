use super::*;
use std::io::{Cursor, Write};

#[test]
fn auth_lock_acl_requires_current_owner_before_transfer() {
    let recipe = trusted_ownership().unwrap();
    let mut owner = "learning_admin";
    let mut public_execute = true;
    let mut runtime_execute = false;
    let mut auth_create = false;
    let mut transferred = false;
    for sql in recipe.lines() {
        if sql == "GRANT CREATE ON SCHEMA public TO learning_auth_lock;" {
            auth_create = true;
        } else if sql == "REVOKE CREATE ON SCHEMA public FROM learning_auth_lock;" {
            auth_create = false;
        } else if sql.contains("FUNCTION public.lock_space_grant(") {
            // The fixed writer is admin; membership has INHERIT FALSE. ACL
            // statements need the current owner until the explicit transfer.
            assert_eq!(
                owner, "learning_admin",
                "writer lacks grant authority: {sql}"
            );
            if sql.starts_with("REVOKE ") {
                public_execute = false;
            } else if sql.starts_with("GRANT ") {
                runtime_execute = true;
            } else if sql.starts_with("ALTER ") {
                assert!(!public_execute && runtime_execute && auth_create);
                owner = "learning_auth_lock";
                transferred = true;
            }
        }
    }
    assert!(transferred && !auth_create && !public_execute && runtime_execute);
}

#[test]
fn permuted_copy_frames_stream_in_contract_order_with_exact_field_bytes() {
    let contract = SchemaContract::embedded().unwrap();
    let history = learning_db::MIGRATOR
        .iter()
        .map(|m| {
            format!(
                "{}\t{}\t2026-01-01 00:00:00+00\tt\t\\\\x{}\t0\n",
                m.version,
                m.description,
                hex::encode(&m.checksum)
            )
        })
        .collect::<String>();
    let mut canonical = String::from_utf8(contract.template.clone())
        .unwrap()
        .replace("{{RESTRICT_KEY}}", "rangeorderkey");
    for table in contract.table_names() {
        canonical = canonical.replace(
            &format!("{{{{COPY:{table}}}}}"),
            if table == "_sqlx_migrations" {
                &history
            } else {
                ""
            },
        );
    }
    let ranges = schema::validate_sql(&mut Cursor::new(canonical.as_bytes()), &contract).unwrap();
    let mut permuted = Vec::new();
    let frame =
        |r: &schema::Range| &canonical.as_bytes()[r.start as usize..(r.start + r.len) as usize];
    permuted.extend_from_slice(frame(&ranges[0]));
    for r in ranges[1..ranges.len() - 1].iter().rev() {
        permuted.extend_from_slice(frame(r));
    }
    permuted.extend_from_slice(frame(ranges.last().unwrap()));
    assert_ne!(permuted, canonical.as_bytes());
    let checked = schema::validate_sql(&mut Cursor::new(&permuted), &contract).unwrap();
    let path = std::env::temp_dir().join(format!("kw-full-range-{}.sql", uuid::Uuid::new_v4()));
    File::create(&path).unwrap().write_all(&permuted).unwrap();
    let verified = VerifiedFullImport {
        file: File::open(path).unwrap(),
        ranges: checked,
        tables: contract.table_names().map(str::to_owned).collect(),
        decoded_sha256: String::new(),
    };
    // Both actual payload hashing and transmission call this same reader.
    for offset in [0, 1, ranges[0].len + 17, canonical.len() as u64] {
        let mut reader = verified.checked_reader_from(offset).unwrap();
        let mut out = Vec::new();
        let mut buffer = [0; 127];
        loop {
            let n = reader.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buffer[..n]);
        }
        assert!(
            out == canonical.as_bytes()[offset as usize..],
            "physical order leaked at logical offset {offset}"
        );
    }
    assert!(
        verified
            .checked_reader_from(canonical.len() as u64 + 1)
            .is_err()
    );
}
