#[cfg(test)]
use super::dump_observer::{Guard, Observer, Step};
use crate::{BackupError, MigrationRecord};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Read};

#[cfg(test)]
pub(super) fn validate_toc_observed(
    raw: &[u8],
    contract: &SchemaContract,
    observer: Option<&super::dump_observer::Observer>,
) -> Result<(), BackupError> {
    validate_toc_inner(raw, contract, observer)
}
#[cfg(test)]
pub(super) fn validate_owned_observed(
    reader: &mut impl BufRead,
    contract: &SchemaContract,
    observer: Option<&super::dump_observer::Observer>,
) -> Result<(), BackupError> {
    validate_owned_inner(reader, contract, observer)
}
#[cfg(test)]
pub(super) fn validate_sql_observed(
    reader: &mut impl BufRead,
    contract: &SchemaContract,
    observer: Option<&super::dump_observer::Observer>,
) -> Result<Vec<Range>, BackupError> {
    validate_sql_inner(reader, contract, observer)
}

/// An immutable reviewed content contract, with no restore or source authority.
#[derive(Clone)]
pub struct SchemaContract {
    pub(super) template: Vec<u8>,
    pub(super) owned_template: Vec<u8>,
    pub(super) tables: Vec<Table>,
    pub(super) toc: serde_json::Value,
    pub(super) acl: serde_json::Value,
    migrations: Vec<MigrationRecord>,
}

#[derive(Clone, Deserialize)]
#[serde(try_from = "TableWire")]
pub(super) struct Table {
    pub table: String,
    pub sql_name: String,
    pub columns: Vec<Column>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TableWire {
    table: String,
    sql_name: String,
    columns: Vec<Column>,
}
impl TryFrom<TableWire> for Table {
    type Error = &'static str;
    fn try_from(wire: TableWire) -> Result<Self, Self::Error> {
        let mut names = BTreeSet::new();
        if !native_identifier(&wire.table, &wire.sql_name)
            || wire.columns.is_empty()
            || wire.columns.len() > 1600
            || wire
                .columns
                .iter()
                .any(|c| !native_identifier(&c.name, &c.sql_name) || !names.insert(c.name.as_str()))
        {
            return Err("native table or column spelling");
        }
        Ok(Self {
            table: wire.table,
            sql_name: wire.sql_name,
            columns: wire.columns,
        })
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Column {
    pub name: String,
    pub sql_name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub not_null: bool,
    pub default: Option<String>,
    pub identity: String,
    pub generated: String,
}
impl Table {
    fn header(&self) -> Vec<u8> {
        format!(
            "COPY public.{} ({}) FROM stdin;\n",
            self.sql_name,
            self.columns
                .iter()
                .map(|c| c.sql_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into_bytes()
    }
}
impl SchemaContract {
    pub fn embedded() -> Result<Self, BackupError> {
        Self::embedded_inner(
            #[cfg(test)]
            None,
        )
    }
    #[cfg(test)]
    pub(crate) fn embedded_observed(observer: Option<&Observer>) -> Result<Self, BackupError> {
        dump_observe!(
            observer,
            Step::EmbeddedContract,
            Guard::ContractLoad,
            Self::embedded_inner(observer)
        )
    }
    fn embedded_inner(#[cfg(test)] observer: Option<&Observer>) -> Result<Self, BackupError> {
        // Root-adopted fresh PG18.6 capture from source04d5346. These exact
        // bytes retain their original source/build/case identity; they are a
        // content contract, never a backup or target-write capability.
        load_inner(
            include_bytes!("../../tests/fixtures/c4-full-schema/contract.json"),
            [
                (
                    "pg18-schema.sql.in",
                    include_bytes!("../../tests/fixtures/c4-full-schema/pg18-schema.sql.in"),
                ),
                (
                    "pg18-toc.json",
                    include_bytes!("../../tests/fixtures/c4-full-schema/pg18-toc.json"),
                ),
                (
                    "columns.json",
                    include_bytes!("../../tests/fixtures/c4-full-schema/columns.json"),
                ),
                (
                    "acl.json",
                    include_bytes!("../../tests/fixtures/c4-full-schema/acl.json"),
                ),
            ],
            #[cfg(test)]
            observer,
        )
    }
    pub fn table_names(&self) -> impl Iterator<Item = &str> {
        self.tables.iter().map(|t| t.table.as_str())
    }
    pub fn migrations(&self) -> &[MigrationRecord] {
        &self.migrations
    }
    pub fn validate_migrations(&self, rows: &[MigrationRecord]) -> Result<(), BackupError> {
        if rows != self.migrations {
            return Err(BackupError::Invalid("full migration set"));
        }
        Ok(())
    }
    pub fn acl_catalog(&self) -> &serde_json::Value {
        &self.acl
    }
    pub fn table_columns(&self, table: &str) -> Option<impl Iterator<Item = ColumnDefinition<'_>>> {
        self.tables.iter().find(|t| t.table == table).map(|t| {
            t.columns.iter().map(|c| ColumnDefinition {
                name: &c.name,
                postgres_type: &c.ty,
                not_null: c.not_null,
                default: c.default.as_deref(),
                identity: &c.identity,
                generated: &c.generated,
            })
        })
    }
    pub fn toc_entries(&self) -> impl Iterator<Item = &str> {
        self.toc["entries"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
            .filter_map(|v| v.split_once(' ').map(|(_, name)| name))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ColumnDefinition<'a> {
    pub name: &'a str,
    pub postgres_type: &'a str,
    pub not_null: bool,
    pub default: Option<&'a str>,
    pub identity: &'a str,
    pub generated: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddedManifest {
    format_version: u32,
    capability: String,
    postgres_image: String,
    postgres_version: String,
    producer_sha256: String,
    source: GeneratedSource,
    migrations: Vec<MigrationRecord>,
    migration_fingerprint: String,
    files: BTreeMap<String, EmbeddedFile>,
    migration_copy_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedSource {
    case_id: String,
    container_id: String,
    database: String,
    application_commit: String,
    application_build_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddedFile {
    size: u64,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddedAcl {
    catalog: serde_json::Value,
    owned_schema_template: String,
}

fn canonical_json<T: serde::de::DeserializeOwned>(
    raw: &[u8],
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<T, BackupError> {
    if raw.len() > 4 * 1024 * 1024 {
        return Err(dump_error!(
            observer,
            Step::EmbeddedContract,
            Guard::CanonicalJson,
            BackupError::Capacity("embedded schema metadata")
        ));
    }
    let value: serde_json::Value = dump_observe!(
        observer,
        Step::EmbeddedContract,
        Guard::CanonicalJson,
        serde_json::from_slice(raw).map_err(BackupError::from)
    )?;
    if dump_observe!(
        observer,
        Step::EmbeddedContract,
        Guard::CanonicalJson,
        serde_json::to_vec(&value).map_err(BackupError::from)
    )? != raw
    {
        return Err(dump_error!(
            observer,
            Step::EmbeddedContract,
            Guard::CanonicalJson,
            bad()
        ));
    }
    dump_observe!(
        observer,
        Step::EmbeddedContract,
        Guard::CanonicalJson,
        serde_json::from_value(value).map_err(BackupError::from)
    )
}

// Kept private: callers cannot load an alternative contract from a backup or a
// path. This function is wired ONLY to reviewed checked-in include_bytes.
fn load_inner(
    manifest: &[u8],
    files: [(&str, &[u8]); 4],
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<SchemaContract, BackupError> {
    let manifest: EmbeddedManifest = canonical_json(
        manifest,
        #[cfg(test)]
        observer,
    )?;
    if manifest.format_version != 1
        || manifest.capability != "full_schema_content_v1"
        || manifest.postgres_image
            != "sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
        || manifest.postgres_version != "18.6 (Debian 18.6-1.pgdg12+2)"
        || !crate::valid_digest(&manifest.producer_sha256)
        || !crate::valid_digest(&manifest.migration_copy_sha256)
    {
        return Err(dump_error!(
            observer,
            Step::EmbeddedContract,
            Guard::ContractManifest,
            bad()
        ));
    }
    let source = &manifest.source;
    let case = uuid::Uuid::parse_str(&source.case_id).map_err(|_| {
        dump_error!(
            observer,
            Step::EmbeddedContract,
            Guard::ContractSourceIdentity,
            bad()
        )
    })?;
    if case.to_string() != source.case_id
        || case.get_version_num() != 4
        || case.get_variant() != uuid::Variant::RFC4122
        || source.database != format!("learning_backup_c4_task3_{}", source.case_id)
        || !crate::valid_hex(&source.container_id, 64)
        || !crate::valid_hex(&source.application_commit, 40)
        || !crate::valid_digest(&source.application_build_sha256)
    {
        return Err(dump_error!(
            observer,
            Step::EmbeddedContract,
            Guard::ContractSourceIdentity,
            bad()
        ));
    }
    let expected: Vec<_> = learning_db::MIGRATOR
        .iter()
        .map(|m| MigrationRecord {
            version: m.version as u64,
            checksum_hex: hex::encode(&m.checksum),
        })
        .collect();
    if expected.len() != 15
        || expected != manifest.migrations
        || manifest.migration_fingerprint != crate::digest(&serde_json::to_vec(&expected)?)
    {
        return Err(dump_error!(
            observer,
            Step::EmbeddedContract,
            Guard::ContractMigrations,
            bad()
        ));
    }
    let inputs: BTreeMap<_, _> = files.into_iter().collect();
    if inputs.len() != 4
        || inputs.keys().copied().collect::<BTreeSet<_>>()
            != BTreeSet::from([
                "pg18-schema.sql.in",
                "pg18-toc.json",
                "columns.json",
                "acl.json",
            ])
        || inputs.keys().copied().collect::<BTreeSet<_>>()
            != manifest.files.keys().map(String::as_str).collect()
    {
        return Err(dump_error!(
            observer,
            Step::EmbeddedContract,
            Guard::ContractFileSet,
            bad()
        ));
    }
    for (name, raw) in &inputs {
        let record = &manifest.files[*name];
        if raw.len() > 4 * 1024 * 1024
            || record.size != raw.len() as u64
            || record.sha256 != crate::digest(raw)
        {
            return Err(dump_error!(
                observer,
                Step::EmbeddedContract,
                Guard::ContractFileHash,
                bad()
            ));
        }
    }
    let tables: Vec<Table> = canonical_json(
        inputs["columns.json"],
        #[cfg(test)]
        observer,
    )?;
    let mut names = BTreeSet::new();
    for table in &tables {
        if !identifier(&table.table)
            || !names.insert(table.table.as_str())
            || table.columns.is_empty()
            || table.columns.len() > 1600
        {
            return Err(dump_error!(
                observer,
                Step::EmbeddedContract,
                Guard::ContractTable,
                bad()
            ));
        }
        let mut columns = BTreeSet::new();
        for column in &table.columns {
            if !identifier(&column.name)
                || !columns.insert(column.name.as_str())
                || column.ty.is_empty()
                || !column.identity.is_empty()
                || !column.generated.is_empty()
            {
                return Err(dump_error!(
                    observer,
                    Step::EmbeddedContract,
                    Guard::ContractColumn,
                    bad()
                ));
            }
        }
    }
    if tables.len() != 64 || !names.contains("_sqlx_migrations") {
        return Err(dump_error!(
            observer,
            Step::EmbeddedContract,
            Guard::ContractTableSet,
            bad()
        ));
    }
    let acl: EmbeddedAcl = canonical_json(
        inputs["acl.json"],
        #[cfg(test)]
        observer,
    )?;
    let toc: serde_json::Value = canonical_json(
        inputs["pg18-toc.json"],
        #[cfg(test)]
        observer,
    )?;
    if toc
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect::<BTreeSet<_>>())
        != Some(BTreeSet::from(["entries", "header"]))
        || toc["entries"]
            .as_array()
            .is_none_or(|v| v.len() > 100000 || v.iter().any(|r| r.as_str().is_none()))
    {
        return Err(dump_error!(
            observer,
            Step::EmbeddedContract,
            Guard::ContractToc,
            bad()
        ));
    }
    let contract = SchemaContract {
        template: inputs["pg18-schema.sql.in"].to_vec(),
        owned_template: acl.owned_schema_template.into_bytes(),
        tables,
        toc,
        acl: acl.catalog,
        migrations: expected,
    };
    for sql in [&contract.template, &contract.owned_template] {
        if !sql.starts_with(PREAMBLE)
            || sql.contains(&0)
            || sql.contains(&b'\r')
            || sql.len() > 4 * 1024 * 1024
        {
            return Err(dump_error!(
                observer,
                Step::EmbeddedContract,
                Guard::ContractTemplate,
                bad()
            ));
        }
        expand_key(
            sql,
            b"checked_contract_key",
            #[cfg(test)]
            Step::EmbeddedContract,
            #[cfg(test)]
            observer,
        )?;
    }
    Ok(contract)
}
fn identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= 63
        && matches!(bytes.next(), Some(b'a'..=b'z' | b'_'))
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
fn native_identifier(name: &str, sql_name: &str) -> bool {
    identifier(name) && (sql_name == name || sql_name == format!("\"{name}\""))
}

const KEY: &[u8] = b"{{RESTRICT_KEY}}";
const PREAMBLE: &[u8] = b"--\n-- PostgreSQL database dump\n--\n\n";

fn bad() -> BackupError {
    BackupError::Invalid("full schema contract")
}
fn locate(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|x| x == needle)
}
fn expand_key(
    template: &[u8],
    key: &[u8],
    #[cfg(test)] step: Step,
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<Vec<u8>, BackupError> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut count = 0;
    while let Some(index) = locate(&template[start..], KEY) {
        result.extend_from_slice(&template[start..start + index]);
        result.extend_from_slice(key);
        start += index + KEY.len();
        count += 1;
    }
    result.extend_from_slice(&template[start..]);
    if count != 2 {
        return Err(dump_error!(observer, step, Guard::TemplateKeyCount, bad()));
    }
    Ok(result)
}
fn read_key(
    reader: &mut impl BufRead,
    #[cfg(test)] step: Step,
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<Vec<u8>, BackupError> {
    let mut preamble = vec![0; PREAMBLE.len()];
    dump_observe!(
        observer,
        step,
        Guard::Preamble,
        reader.read_exact(&mut preamble).map_err(BackupError::from)
    )?;
    if preamble != PREAMBLE {
        return Err(dump_error!(observer, step, Guard::Preamble, bad()));
    }
    let line = small_line(
        reader,
        256,
        #[cfg(test)]
        step,
        #[cfg(test)]
        observer,
    )?;
    let key = line
        .strip_prefix(b"\\restrict ")
        .and_then(|l| l.strip_suffix(b"\n"))
        .ok_or_else(|| dump_error!(observer, step, Guard::RestrictKey, bad()))?;
    if key.is_empty() || key.len() > 128 || !key.iter().all(u8::is_ascii_alphanumeric) {
        return Err(dump_error!(observer, step, Guard::RestrictKey, bad()));
    }
    Ok(key.to_vec())
}
fn small_line(
    reader: &mut impl BufRead,
    cap: usize,
    #[cfg(test)] step: Step,
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<Vec<u8>, BackupError> {
    let mut row = Vec::new();
    loop {
        let buffer = dump_observe!(
            observer,
            step,
            Guard::LineRead,
            reader.fill_buf().map_err(BackupError::from)
        )?;
        if buffer.is_empty() {
            return Err(dump_error!(observer, step, Guard::LineRead, bad()));
        }
        let n = buffer
            .iter()
            .position(|&b| b == b'\n')
            .map_or(buffer.len(), |i| i + 1);
        if row.len() + n > cap {
            return Err(dump_error!(
                observer,
                step,
                Guard::LineRead,
                BackupError::Capacity("schema line")
            ));
        }
        row.extend_from_slice(&buffer[..n]);
        reader.consume(n);
        if row.last() == Some(&b'\n') {
            return Ok(row);
        }
    }
}
fn expect(
    reader: &mut impl Read,
    expected: &[u8],
    #[cfg(test)] step: Step,
    #[cfg(test)] guard: Guard,
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<(), BackupError> {
    let mut buffer = [0u8; super::spool::BUFFER];
    let mut offset = 0;
    while offset < expected.len() {
        let n = buffer.len().min(expected.len() - offset);
        dump_observe!(
            observer,
            step,
            guard,
            reader
                .read_exact(&mut buffer[..n])
                .map_err(BackupError::from)
        )?;
        if buffer[..n] != expected[offset..offset + n] {
            return Err(dump_error!(observer, step, guard, bad()));
        }
        offset += n;
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct Range {
    pub start: u64,
    pub len: u64,
    pub table: Option<usize>,
}

/// Validate every fixed byte and every COPY boundary. Whole frames are returned
/// in reviewed contract order; variable data bytes are never transformed.
pub(super) fn validate_sql(
    reader: &mut impl BufRead,
    contract: &SchemaContract,
) -> Result<Vec<Range>, BackupError> {
    validate_sql_inner(
        reader,
        contract,
        #[cfg(test)]
        None,
    )
}
fn validate_sql_inner(
    reader: &mut impl BufRead,
    contract: &SchemaContract,
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<Vec<Range>, BackupError> {
    let key = read_key(
        reader,
        #[cfg(test)]
        Step::SqlValidation,
        #[cfg(test)]
        observer,
    )?;
    let template = expand_key(
        &contract.template,
        &key,
        #[cfg(test)]
        Step::SqlValidation,
        #[cfg(test)]
        observer,
    )?;
    let consumed = PREAMBLE.len() + b"\\restrict ".len() + key.len() + 1;
    let marker = b"--\n-- Data for Name: ";
    let first = locate(&template, marker)
        .ok_or_else(|| dump_error!(observer, Step::SqlValidation, Guard::SqlFixedSchema, bad()))?;
    expect(
        reader,
        &template[consumed..first],
        #[cfg(test)]
        Step::SqlValidation,
        #[cfg(test)]
        Guard::SqlFixedSchema,
        #[cfg(test)]
        observer,
    )?;
    let mut ranges = vec![Range {
        start: 0,
        len: first as u64,
        table: None,
    }];
    let mut frames = BTreeMap::new();
    let mut cursor = first;
    let mut order = Vec::new();
    for _ in 0..contract.tables.len() {
        if !template[cursor..].starts_with(marker) {
            return Err(dump_error!(
                observer,
                Step::SqlValidation,
                Guard::SqlTemplateFrame,
                bad()
            ));
        }
        let token_start = locate(&template[cursor..], b"{{COPY:").ok_or_else(|| {
            dump_error!(
                observer,
                Step::SqlValidation,
                Guard::SqlTemplateFrame,
                bad()
            )
        })? + cursor;
        let token_end = locate(&template[token_start..], b"}}").ok_or_else(|| {
            dump_error!(
                observer,
                Step::SqlValidation,
                Guard::SqlTemplateFrame,
                bad()
            )
        })? + token_start
            + 2;
        let table_name =
            std::str::from_utf8(&template[token_start + 7..token_end - 2]).map_err(|_| {
                dump_error!(
                    observer,
                    Step::SqlValidation,
                    Guard::SqlTemplateFrame,
                    bad()
                )
            })?;
        let table = contract
            .tables
            .iter()
            .position(|t| t.table == table_name)
            .ok_or_else(|| {
                dump_error!(
                    observer,
                    Step::SqlValidation,
                    Guard::SqlTemplateFrame,
                    bad()
                )
            })?;
        let header = &template[cursor..token_start];
        if !header.ends_with(&contract.tables[table].header())
            || !template[token_end..].starts_with(b"\\.\n\n\n")
        {
            return Err(dump_error!(
                observer,
                Step::SqlValidation,
                Guard::SqlTemplateFrame,
                bad()
            ));
        }
        let line_end = locate(&header[3..], b"\n").ok_or_else(|| {
            dump_error!(
                observer,
                Step::SqlValidation,
                Guard::SqlTemplateFrame,
                bad()
            )
        })? + 4;
        let name_line = header[3..line_end].to_vec();
        if frames.insert(name_line, (table, header.to_vec())).is_some() {
            return Err(dump_error!(
                observer,
                Step::SqlValidation,
                Guard::SqlTemplateFrame,
                bad()
            ));
        }
        order.push(table);
        cursor = token_end + 5;
    }
    let mut found = BTreeMap::new();
    let mut position = first as u64;
    for _ in 0..contract.tables.len() {
        let start = position;
        let first_line = small_line(
            reader,
            4,
            #[cfg(test)]
            Step::SqlValidation,
            #[cfg(test)]
            observer,
        )?;
        if first_line != b"--\n" {
            return Err(dump_error!(
                observer,
                Step::SqlValidation,
                Guard::CopyHeader,
                bad()
            ));
        }
        let name_line = small_line(
            reader,
            1024,
            #[cfg(test)]
            Step::SqlValidation,
            #[cfg(test)]
            observer,
        )?;
        let (table, header) = frames
            .get(&name_line)
            .ok_or_else(|| dump_error!(observer, Step::SqlValidation, Guard::CopyHeader, bad()))?;
        if found.contains_key(table) {
            return Err(dump_error!(
                observer,
                Step::SqlValidation,
                Guard::CopyHeader,
                bad()
            ));
        }
        expect(
            reader,
            &header[3 + name_line.len()..],
            #[cfg(test)]
            Step::SqlValidation,
            #[cfg(test)]
            Guard::CopyHeader,
            #[cfg(test)]
            observer,
        )?;
        let (length, rows) = dump_observe!(
            observer,
            Step::CopyValidation,
            Guard::CopyFrame,
            super::copy::scan_payload(
                reader,
                contract.tables[*table].columns.len(),
                contract.tables[*table].table == "_sqlx_migrations",
            )
        )?;
        if contract.tables[*table].table == "_sqlx_migrations" {
            validate_migration_copy_inner(
                &rows,
                contract,
                #[cfg(test)]
                observer,
            )?;
        }
        expect(
            reader,
            b"\n\n",
            #[cfg(test)]
            Step::SqlValidation,
            #[cfg(test)]
            Guard::CopyFrame,
            #[cfg(test)]
            observer,
        )?;
        position += header.len() as u64 + length + 2;
        found.insert(
            *table,
            Range {
                start,
                len: position - start,
                table: Some(*table),
            },
        );
    }
    for table in order {
        ranges.push(
            found.remove(&table).ok_or_else(|| {
                dump_error!(observer, Step::SqlValidation, Guard::CopyHeader, bad())
            })?,
        );
    }
    expect(
        reader,
        &template[cursor..],
        #[cfg(test)]
        Step::SqlValidation,
        #[cfg(test)]
        Guard::SqlPostamble,
        #[cfg(test)]
        observer,
    )?;
    let mut extra = [0; 1];
    if dump_observe!(
        observer,
        Step::SqlValidation,
        Guard::TrailingBytes,
        reader.read(&mut extra).map_err(BackupError::from)
    )? != 0
    {
        return Err(dump_error!(
            observer,
            Step::SqlValidation,
            Guard::TrailingBytes,
            bad()
        ));
    }
    ranges.push(Range {
        start: position,
        len: (template.len() - cursor) as u64,
        table: None,
    });
    Ok(ranges)
}

#[cfg(test)]
fn validate_migration_copy(rows: &[Vec<u8>], contract: &SchemaContract) -> Result<(), BackupError> {
    validate_migration_copy_inner(rows, contract, None)
}
fn validate_migration_copy_inner(
    rows: &[Vec<u8>],
    contract: &SchemaContract,
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<(), BackupError> {
    let bad = || {
        dump_error!(
            observer,
            Step::MigrationValidation,
            Guard::MigrationIdentity,
            bad()
        )
    };
    if rows.len() != contract.migrations.len() {
        return Err(bad());
    }
    let mut seen = BTreeSet::new();
    for row in rows {
        let fields: Vec<_> = row
            .strip_suffix(b"\n")
            .ok_or_else(bad)?
            .split(|&b| b == b'\t')
            .collect();
        if fields.len() != 6 {
            return Err(bad());
        }
        let version = std::str::from_utf8(fields[0])
            .map_err(|_| bad())?
            .parse::<u64>()
            .map_err(|_| bad())?;
        let migration = contract
            .migrations
            .iter()
            .find(|m| m.version == version)
            .ok_or_else(bad)?;
        let source = learning_db::MIGRATOR
            .iter()
            .find(|m| m.version as u64 == version)
            .ok_or_else(bad)?;
        if fields[0] != version.to_string().as_bytes()
            || !seen.insert(version)
            || fields[1] != source.description.as_bytes()
            || fields[2].is_empty()
            || fields[3] != b"t"
            || fields[4] != format!("\\\\x{}", migration.checksum_hex).as_bytes()
        {
            return Err(bad());
        }
        if std::str::from_utf8(fields[5])
            .map_err(|_| bad())?
            .parse::<i64>()
            .map_err(|_| bad())?
            < 0
        {
            return Err(bad());
        }
    }
    Ok(())
}
pub(super) fn validate_owned(
    reader: &mut impl BufRead,
    contract: &SchemaContract,
) -> Result<(), BackupError> {
    validate_owned_inner(
        reader,
        contract,
        #[cfg(test)]
        None,
    )
}
fn validate_owned_inner(
    reader: &mut impl BufRead,
    contract: &SchemaContract,
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<(), BackupError> {
    let key = read_key(
        reader,
        #[cfg(test)]
        Step::OwnedSchemaValidation,
        #[cfg(test)]
        observer,
    )?;
    let template = expand_key(
        &contract.owned_template,
        &key,
        #[cfg(test)]
        Step::OwnedSchemaValidation,
        #[cfg(test)]
        observer,
    )?;
    let consumed = PREAMBLE.len() + b"\\restrict ".len() + key.len() + 1;
    expect(
        reader,
        &template[consumed..],
        #[cfg(test)]
        Step::OwnedSchemaValidation,
        #[cfg(test)]
        Guard::TemplateBytes,
        #[cfg(test)]
        observer,
    )?;
    let mut byte = [0];
    if dump_observe!(
        observer,
        Step::OwnedSchemaValidation,
        Guard::TrailingBytes,
        reader.read(&mut byte).map_err(BackupError::from)
    )? != 0
    {
        return Err(dump_error!(
            observer,
            Step::OwnedSchemaValidation,
            Guard::TrailingBytes,
            bad()
        ));
    }
    Ok(())
}
pub(super) fn validate_toc(raw: &[u8], contract: &SchemaContract) -> Result<(), BackupError> {
    validate_toc_inner(
        raw,
        contract,
        #[cfg(test)]
        None,
    )
}
fn validate_toc_inner(
    raw: &[u8],
    contract: &SchemaContract,
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<(), BackupError> {
    if raw.len() > 4 * 1024 * 1024 {
        return Err(dump_error!(
            observer,
            Step::TocValidation,
            Guard::TocCapacity,
            BackupError::Capacity("full TOC")
        ));
    }
    if !raw.is_ascii() || raw.contains(&b'\r') || !raw.ends_with(b"\n") {
        return Err(dump_error!(
            observer,
            Step::TocValidation,
            Guard::TocFormat,
            bad()
        ));
    }
    let lines: Vec<_> = std::str::from_utf8(raw)
        .map_err(|_| dump_error!(observer, Step::TocValidation, Guard::TocFormat, bad()))?
        .lines()
        .collect();
    if lines.len() < 15 || lines.len() > 100015 || lines[0] != ";" {
        return Err(dump_error!(
            observer,
            Step::TocValidation,
            Guard::TocFormat,
            bad()
        ));
    }
    let date = lines[1]
        .strip_prefix("; Archive created at ")
        .ok_or_else(|| dump_error!(observer, Step::TocValidation, Guard::TocDate, bad()))?;
    if date.len() != 23
        || !date.ends_with(" UTC")
        || date.bytes().enumerate().any(|(i, b)| match i {
            4 | 7 => b != b'-',
            10 | 19 => b != b' ',
            13 | 16 => b != b':',
            20..=22 => false,
            _ => !b.is_ascii_digit(),
        })
    {
        return Err(dump_error!(
            observer,
            Step::TocValidation,
            Guard::TocDate,
            bad()
        ));
    }
    let database = lines[2]
        .strip_prefix(";     dbname: ")
        .ok_or_else(|| dump_error!(observer, Step::TocValidation, Guard::TocDatabase, bad()))?;
    if database.is_empty()
        || database.len() > 63
        || !database
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(dump_error!(
            observer,
            Step::TocValidation,
            Guard::TocDatabase,
            bad()
        ));
    }
    let header = dump_observe!(
        observer,
        Step::TocValidation,
        Guard::TocHeader,
        serde_json::to_value(&lines[3..15]).map_err(BackupError::from)
    )?;
    if contract.toc.get("header") != Some(&header) {
        return Err(dump_error!(
            observer,
            Step::TocValidation,
            Guard::TocHeader,
            bad()
        ));
    }
    let mut ids = BTreeSet::new();
    let mut entries = BTreeSet::new();
    for line in &lines[15..] {
        let bad = || dump_error!(observer, Step::TocValidation, Guard::TocEntry, bad());
        let (id, rest) = line.split_once("; ").ok_or_else(bad)?;
        let mut rest = rest.splitn(3, ' ');
        let catalog = rest.next().ok_or_else(bad)?;
        let oid = rest.next().ok_or_else(bad)?;
        let name = rest.next().ok_or_else(bad)?;
        for num in [id, catalog, oid] {
            let value = num.parse::<u32>().map_err(|_| bad())?;
            if value.to_string() != num {
                return Err(bad());
            }
        }
        if id == "0"
            || !ids.insert(id)
            || name.is_empty()
            || !entries.insert(format!("{catalog} {name}"))
        {
            return Err(bad());
        }
    }
    if contract.toc.get("entries")
        != Some(&dump_observe!(
            observer,
            Step::TocValidation,
            Guard::TocSet,
            serde_json::to_value(entries).map_err(BackupError::from)
        )?)
    {
        return Err(dump_error!(
            observer,
            Step::TocValidation,
            Guard::TocSet,
            bad()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};
    #[test]
    fn genuine_embedded_schema_parses_all_frames_with_variable_test_history() {
        let contract = SchemaContract::embedded().unwrap();
        // The schema is the adopted real PG capture. These variable history
        // values are a grammar unit input, not native replay/restore evidence.
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
        let mut raw = String::from_utf8(contract.template.clone())
            .unwrap()
            .replace("{{RESTRICT_KEY}}", "grammartestkey");
        for table in contract.table_names() {
            raw = raw.replace(
                &format!("{{{{COPY:{table}}}}}"),
                if table == "_sqlx_migrations" {
                    &history
                } else {
                    ""
                },
            );
        }
        let ranges = validate_sql(
            &mut BufReader::with_capacity(257, Cursor::new(raw.as_bytes())),
            &contract,
        )
        .unwrap();
        assert_eq!(ranges.len(), 66);
        assert_eq!(ranges.iter().filter(|r| r.table.is_some()).count(), 64);
        assert_eq!(ranges.iter().map(|r| r.len).sum::<u64>(), raw.len() as u64);
        let changed = raw.replacen("SECURITY DEFINER", "SECURITY INVOKER", 1);
        assert_ne!(changed, raw);
        assert!(validate_sql(&mut Cursor::new(changed.as_bytes()), &contract).is_err());
    }
    #[test]
    fn actual_failed_capture_nine_native_headers_bind_semantic_names() {
        // Failed087 actual decoded SHA08fe6a4d...; diagnostic header examples,
        // never a complete or adopted PG schema fixture.
        let headers = [
            "COPY public.composition_occurrence (composition_revision_id, space_id, composition_id, occurrence_id, \"position\", block_space_id, block_id, block_revision_id, child_space_id, child_composition_id, child_revision_id) FROM stdin;\n",
            "COPY public.lineage_input (operation_id, \"position\", space_id, block_id, revision_id) FROM stdin;\n",
            "COPY public.lineage_output (operation_id, \"position\", space_id, block_id, revision_id) FROM stdin;\n",
            "COPY public.\"overlay\" (id, space_id, owner_id, root_composition_id, head_revision_id) FROM stdin;\n",
            "COPY public.overlay_placement (overlay_id, overlay_revision_id, placement_id, group_id, \"position\", block_space_id, block_id, block_revision_id) FROM stdin;\n",
            "COPY public.reading_epistemic_selection (view_id, view_revision_id, \"position\", stream_id, review_id) FROM stdin;\n",
            "COPY public.reading_receipt_block (actor_id, request_id, \"position\", block_space_id, block_id, revision_id) FROM stdin;\n",
            "COPY public.reading_relation_selection (view_id, view_revision_id, \"position\", relation_id, relation_revision_id, review_id) FROM stdin;\n",
            "COPY public.reference_dependency (source_kind, source_object_id, source_revision_id, \"position\", role, target_kind, target_object_id, target_revision_id) FROM stdin;\n",
        ];
        for native in headers {
            let (table, columns) = native
                .strip_prefix("COPY public.")
                .unwrap()
                .strip_suffix(") FROM stdin;\n")
                .unwrap()
                .split_once(" (")
                .unwrap();
            let metadata = serde_json::json!({"table":table.trim_matches('"'),"sql_name":table,"columns":columns.split(", ").map(|column|serde_json::json!({"name":column.trim_matches('"'),"sql_name":column,"type":"text","not_null":false,"default":null,"identity":"","generated":""})).collect::<Vec<_>>()});
            let table: Table =
                serde_json::from_value(metadata.clone()).expect("native table/column spelling");
            assert_eq!(table.header(), native.as_bytes());
            let name = table.table.clone();
            let frame = format!(
                "--\n-- Data for Name: {name}; Type: TABLE DATA; Schema: public; Owner: -\n--\n\n{native}{{{{COPY:{name}}}}}\\.\n\n\n"
            );
            let mut contract = sample();
            contract.tables = vec![table];
            contract.template=format!("--\n-- PostgreSQL database dump\n--\n\n\\restrict {{{{RESTRICT_KEY}}}}\n\n{frame}\\unrestrict {{{{RESTRICT_KEY}}}}\n\n").into_bytes();
            let raw = String::from_utf8(contract.template.clone())
                .unwrap()
                .replace("{{RESTRICT_KEY}}", "nativetestkey")
                .replace(&format!("{{{{COPY:{name}}}}}"), "");
            let ranges = validate_sql(&mut Cursor::new(raw.as_bytes()), &contract).unwrap();
            assert_eq!(
                ranges.iter().filter_map(|r| r.table).collect::<Vec<_>>(),
                vec![0]
            );
            let first_column = columns.split(", ").next().unwrap();
            let raw_frame = frame.replace(&format!("{{{{COPY:{name}}}}}"), "");
            for changed in [
                raw.replace(
                    native,
                    &native.replace(
                        &format!("({first_column},"),
                        &format!("(\"{first_column}\","),
                    ),
                ),
                raw.replace(native, "COPY public.unknown (id) FROM stdin;\n"),
                raw.replace(
                    native,
                    &native.replace(" FROM stdin;", " FROM PROGRAM 'bad';"),
                ),
                raw.replace("\\.\n\n\n", "\\.\n\\connect other\n\n"),
                raw.replace(&raw_frame, &format!("{raw_frame}{raw_frame}")),
            ] {
                assert!(validate_sql(&mut Cursor::new(changed.as_bytes()), &contract).is_err());
            }
            for injected in [
                "other",
                "\"overlay\"; COMMIT;",
                "\"OVERLAY\"",
                "overlay\n\\connect other",
            ] {
                let mut bad = metadata.clone();
                bad["sql_name"] = serde_json::Value::String(injected.into());
                assert!(serde_json::from_value::<Table>(bad).is_err());
            }
            let mut duplicate = metadata.clone();
            let first = duplicate["columns"][0].clone();
            duplicate["columns"].as_array_mut().unwrap().push(first);
            assert!(serde_json::from_value::<Table>(duplicate).is_err());
        }
    }
    fn sample() -> SchemaContract {
        let table = Table {
            table: "edge".into(),
            sql_name: "edge".into(),
            columns: vec![Column {
                name: "value".into(),
                sql_name: "value".into(),
                ty: "text".into(),
                not_null: false,
                default: None,
                identity: "".into(),
                generated: "".into(),
            }],
        };
        let template=b"--\n-- PostgreSQL database dump\n--\n\n\\restrict {{RESTRICT_KEY}}\n\nCREATE TABLE public.edge (value text);\n\n--\n-- Data for Name: edge; Type: TABLE DATA; Schema: public; Owner: learning_admin\n--\n\nCOPY public.edge (value) FROM stdin;\n{{COPY:edge}}\\.\n\n\nALTER TABLE public.edge ADD CHECK (value <> '');\n\n\\unrestrict {{RESTRICT_KEY}}\n\n".to_vec();
        SchemaContract {
            owned_template: template.clone(),
            template,
            tables: vec![table],
            toc: serde_json::Value::Null,
            acl: serde_json::Value::Null,
            migrations: vec![],
        }
    }
    #[test]
    fn full_dump_rejects_extra_objects_changed_function_acl_or_reconnect() {
        let contract = sample();
        let raw = String::from_utf8(contract.template.clone())
            .unwrap()
            .replace("{{RESTRICT_KEY}}", "abc123")
            .replace("{{COPY:edge}}", "中文\\nline; COMMIT; \\\\connect other\n");
        let mut reader = BufReader::with_capacity(3, Cursor::new(raw.as_bytes()));
        let ranges = validate_sql(&mut reader, &contract).unwrap();
        let joined: Vec<u8> = ranges
            .iter()
            .flat_map(|r| {
                raw.as_bytes()[r.start as usize..(r.start + r.len) as usize]
                    .iter()
                    .copied()
            })
            .collect();
        assert_eq!(joined, raw.as_bytes());
        for bad in [
            raw.replace("CREATE TABLE", "COMMIT; CREATE TABLE"),
            raw.replace("ADD CHECK", "OWNER TO attacker; ADD CHECK"),
            raw.replace("value <> ''", "value <> 'changed'"),
            raw.replace("\\unrestrict abc123", "\\unrestrict different"),
            format!("{raw}\\connect other\n"),
            raw.replace("\\.\n\n\n", "\\.\nCOMMIT;\n\n"),
        ] {
            assert!(validate_sql(&mut Cursor::new(bad.as_bytes()), &contract).is_err());
        }
    }
    #[test]
    fn separate_owned_schema_detects_acl_or_security_definer_change() {
        let mut contract = sample();
        contract.owned_template=b"--\n-- PostgreSQL database dump\n--\n\n\\restrict {{RESTRICT_KEY}}\n\nCREATE FUNCTION public.f() RETURNS void LANGUAGE sql SECURITY DEFINER SET search_path TO 'pg_catalog', 'public', 'pg_temp' AS 'SELECT NULL';\nALTER FUNCTION public.f() OWNER TO learning_auth_lock;\nREVOKE ALL ON FUNCTION public.f() FROM PUBLIC;\nGRANT EXECUTE ON FUNCTION public.f() TO learning_runtime;\n\n\\unrestrict {{RESTRICT_KEY}}\n\n".to_vec();
        let raw = String::from_utf8(contract.owned_template.clone())
            .unwrap()
            .replace("{{RESTRICT_KEY}}", "key");
        validate_owned(&mut Cursor::new(raw.as_bytes()), &contract).unwrap();
        for bad in [
            raw.replace("SECURITY DEFINER", "SECURITY INVOKER"),
            raw.replace("TO learning_runtime", "TO PUBLIC"),
            raw.replace("'pg_catalog', 'public', 'pg_temp'", "'public'"),
            raw.replace("OWNER TO learning_auth_lock", "OWNER TO learning_admin"),
        ] {
            assert!(validate_owned(&mut Cursor::new(bad.as_bytes()), &contract).is_err());
        }
    }
    #[test]
    fn migration_copy_checks_every_row_not_only_max_version() {
        let mut contract = sample();
        contract.migrations = learning_db::MIGRATOR
            .iter()
            .map(|m| MigrationRecord {
                version: m.version as u64,
                checksum_hex: hex::encode(&m.checksum),
            })
            .collect();
        let rows: Vec<Vec<u8>> = learning_db::MIGRATOR
            .iter()
            .map(|m| {
                format!(
                    "{}\t{}\t2026-10-07 01:02:03+00\tt\t\\\\x{}\t42\n",
                    m.version,
                    m.description,
                    hex::encode(&m.checksum)
                )
                .into_bytes()
            })
            .collect();
        validate_migration_copy(&rows, &contract).unwrap();
        assert!(validate_migration_copy(&rows[1..], &contract).is_err());
        let mut altered = rows.clone();
        altered[1] = String::from_utf8(altered[1].clone())
            .unwrap()
            .replace(&contract.migrations[1].checksum_hex, &"0".repeat(96))
            .into_bytes();
        assert!(validate_migration_copy(&altered, &contract).is_err());
        let mut duplicate = rows.clone();
        duplicate[1] = duplicate[0].clone();
        assert!(validate_migration_copy(&duplicate, &contract).is_err());
        let mut unsuccessful = rows.clone();
        unsuccessful[0] = String::from_utf8(unsuccessful[0].clone())
            .unwrap()
            .replace("\tt\t", "\tf\t")
            .into_bytes();
        assert!(validate_migration_copy(&unsuccessful, &contract).is_err());
        let mut renamed = rows.clone();
        renamed[0] = String::from_utf8(renamed[0].clone())
            .unwrap()
            .replace("content core", "renamed")
            .into_bytes();
        assert!(validate_migration_copy(&renamed, &contract).is_err());
    }
    #[test]
    fn toc_rejects_extra_objects_even_when_decoder_strips_acl() {
        let mut contract = sample();
        let header = [
            ";     TOC Entries: 5",
            ";     Compression: gzip",
            ";     Dump Version: 1.16-0",
            ";     Format: CUSTOM",
            ";     Integer: 4 bytes",
            ";     Offset: 8 bytes",
            ";     Dumped from database version: 18.6 (Debian 18.6-1.pgdg12+2)",
            ";     Dumped by pg_dump version: 18.6 (Debian 18.6-1.pgdg12+2)",
            ";",
            ";",
            "; Selected TOC Entries:",
            ";",
        ];
        contract.toc = serde_json::json!({"header":header,"entries":["1259 TABLE public edge learning_admin"]});
        let raw = format!(
            ";\n; Archive created at 2026-10-07 01:02:03 UTC\n;     dbname: database_a\n{}\n1; 1259 555 TABLE public edge learning_admin\n",
            header.join("\n")
        );
        validate_toc(raw.as_bytes(), &contract).unwrap();
        validate_toc(
            raw.replace("1; 1259 555", "9; 1259 777").as_bytes(),
            &contract,
        )
        .unwrap();
        for bad in [
            raw.replace("TABLE public edge", "TABLE public evil"),
            format!("{raw}2; 0 0 ACL public edge learning_admin\n"),
            raw.replace("1; 1259 555", "01; 1259 555"),
            raw.replace("1259 555", "9999 555"),
            raw.replace("18.6 (", "19.0 ("),
        ] {
            assert!(validate_toc(bad.as_bytes(), &contract).is_err());
        }
    }
    #[test]
    fn entire_copy_frames_can_reorder_without_rewriting_field_bytes() {
        let mut contract = sample();
        let first = contract.template.clone();
        let block_start = locate(&first, b"--\n-- Data for Name: ").unwrap();
        let end = locate(&first, b"ALTER TABLE").unwrap();
        let block = String::from_utf8(first[block_start..end].to_vec()).unwrap();
        let other = block.replace("edge", "other");
        contract.template = [&first[..end], other.as_bytes(), &first[end..]].concat();
        let mut second = contract.tables[0].clone();
        second.table = "other".into();
        second.sql_name = "other".into();
        contract.tables.push(second);
        let prefix = String::from_utf8(first[..block_start].to_vec()).unwrap();
        let suffix = String::from_utf8(first[end..].to_vec()).unwrap();
        let raw = format!("{prefix}{other}{block}{suffix}")
            .replace("{{RESTRICT_KEY}}", "key")
            .replace("{{COPY:edge}}", "中文\\t\\\\path\n")
            .replace("{{COPY:other}}", "\\N\n");
        let ranges = validate_sql(&mut Cursor::new(raw.as_bytes()), &contract).unwrap();
        assert_eq!(
            ranges.iter().filter_map(|r| r.table).collect::<Vec<_>>(),
            vec![0, 1]
        );
        let frames: Vec<_> = ranges
            .iter()
            .filter(|r| r.table.is_some())
            .map(|r| &raw.as_bytes()[r.start as usize..(r.start + r.len) as usize])
            .collect();
        assert!(
            frames[0]
                .windows("中文\\t\\\\path\n".len())
                .any(|b| b == "中文\\t\\\\path\n".as_bytes())
        );
        assert!(frames[1].windows(3).any(|b| b == b"\\N\n"));
        let duplicated = raw.replace(
            &other.replace("{{COPY:other}}", "\\N\n"),
            &block.replace("{{COPY:edge}}", "中文\\t\\\\path\n"),
        );
        assert!(validate_sql(&mut Cursor::new(duplicated.as_bytes()), &contract).is_err());
    }
}
