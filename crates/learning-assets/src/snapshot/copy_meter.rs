//! Custody-only cumulative work prepayment. This is not business authority.
use super::{
    ReadingCopy, SNAPSHOT_FORMAT_VERSION, SNAPSHOT_MAX_JSON_BYTES, SNAPSHOT_MAX_JSON_FILE_BYTES,
    SnapshotIoError, SnapshotManifest, StageWriter, WrittenStage, hex_digest, render_html_to,
    render_markdown_to,
};
use crate::root_journal::SharedJournalBudget;
use std::fmt;
use uuid::Uuid;

const VALIDATION: &[u8] = br#"{"status":"validated"}"#;
const ZERO_SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const NAMES: [&str; 4] = [
    "reading.md",
    "reading.html",
    "validation.json",
    "manifest.json",
];

fn capacity() -> SnapshotIoError {
    SnapshotIoError::LimitExceeded
}

// Paid output counter: no rendered String/Vec exists during this first pass.
struct Counter<'a> {
    length: usize,
    budget: &'a SharedJournalBudget,
}
impl fmt::Write for Counter<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let next = self
            .length
            .checked_add(value.len())
            .filter(|n| *n <= SNAPSHOT_MAX_JSON_FILE_BYTES)
            .ok_or(fmt::Error)?;
        self.budget
            .charge(value.len() as u64, 0)
            .map_err(|_| fmt::Error)?;
        self.length = next;
        Ok(())
    }
}
struct Bounded {
    bytes: Vec<u8>,
    length: usize,
}
impl Bounded {
    fn new(length: usize) -> Result<Self, SnapshotIoError> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| capacity())?;
        Ok(Self { bytes, length })
    }
    fn finish(self) -> Result<Vec<u8>, SnapshotIoError> {
        if self.bytes.len() != self.length {
            return Err(capacity());
        }
        Ok(self.bytes)
    }
}
impl fmt::Write for Bounded {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self
            .bytes
            .len()
            .checked_add(value.len())
            .is_none_or(|n| n > self.length)
        {
            return Err(fmt::Error);
        }
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }
}

// Only fixed names, a typed UUID, sizes and internally computed 64-byte hex
// hashes enter this exact canonical wire writer. No caller JSON is accepted.
fn manifest_to(
    out: &mut impl fmt::Write,
    job: Uuid,
    markdown: u64,
    html: u64,
    hashes: [&str; 3],
) -> fmt::Result {
    out.write_str("{\"capability\":\"reading_copy_v1\",\"copy_id\":\"")?;
    write!(out, "{job}")?;
    out.write_str("\",\"files\":[")?;
    for (index, (name, size, hash)) in [
        ("reading.html", html, hashes[0]),
        ("reading.md", markdown, hashes[1]),
        ("validation.json", VALIDATION.len() as u64, hashes[2]),
    ]
    .into_iter()
    .enumerate()
    {
        if index != 0 {
            out.write_char(',')?;
        }
        write!(
            out,
            "{{\"path\":\"{name}\",\"sha256\":\"{hash}\",\"size\":{size}}}"
        )?;
    }
    write!(out, "],\"format_version\":{SNAPSHOT_FORMAT_VERSION}}}")
}

fn reserve_work(
    budget: &SharedJournalBudget,
    markdown: u64,
    html: u64,
    manifest: u64,
) -> Result<(), SnapshotIoError> {
    let text = markdown.checked_add(html).ok_or_else(capacity)?;
    let payload = text
        .checked_add(VALIDATION.len() as u64)
        .ok_or_else(capacity)?;
    let all_files = payload.checked_add(manifest).ok_or_else(capacity)?;
    if [markdown, html, manifest]
        .iter()
        .any(|n| *n > SNAPSHOT_MAX_JSON_FILE_BYTES as u64)
        || all_files > SNAPSHOT_MAX_JSON_BYTES as u64
    {
        return Err(capacity());
    }
    // A conservative, non-refundable WORK PREPAYMENT, not an actual-I/O count:
    // - text materialization; three payload digests; manifest emit + parse + digest;
    // - each of four known buffers: source-read + file-write + writer hash;
    // - two original verifications (finish_existing and the broker): manifest
    //   read/hash/parse/value encode/canonical encode/compare, payload
    //   read/hash/UTF-8-or-validation compare;
    // - 8 bounded directory/name phases: create, record, sync, seal, each
    //   verifier's enumerate + compare. Job/partial admission adds 2 entries.
    // Counting passes above have already charged their actual escaped output.
    let bytes = text
        .checked_add(payload)
        .and_then(|n| n.checked_add(manifest.checked_mul(3)?))
        .and_then(|n| n.checked_add(all_files.checked_mul(3)?))
        .and_then(|n| n.checked_add(manifest.checked_mul(12)?))
        .and_then(|n| n.checked_add(payload.checked_mul(6)?))
        .and_then(|n| n.checked_add(NAMES.iter().map(|s| s.len() as u64).sum::<u64>() * 8))
        .ok_or_else(capacity)?;
    budget
        .charge(bytes, NAMES.len() as u64 * 8 + 2)
        .map_err(|_| capacity())
}

pub(super) struct PreparedCopy {
    markdown: Vec<u8>,
    html: Vec<u8>,
    manifest_bytes: Vec<u8>,
    manifest: SnapshotManifest,
    manifest_sha256: String,
}
pub(super) fn prepare(
    job: Uuid,
    copy: &ReadingCopy,
    budget: &SharedJournalBudget,
) -> Result<PreparedCopy, SnapshotIoError> {
    let mut markdown = Counter { length: 0, budget };
    render_markdown_to(copy, &mut markdown).map_err(|_| capacity())?;
    let mut html = Counter { length: 0, budget };
    render_html_to(copy, &mut html).map_err(|_| capacity())?;
    let md_length = markdown.length;
    let html_length = html.length;
    let mut manifest = Counter { length: 0, budget };
    manifest_to(
        &mut manifest,
        job,
        md_length as u64,
        html_length as u64,
        [ZERO_SHA; 3],
    )
    .map_err(|_| capacity())?;
    let manifest_length = manifest.length;
    // Every buffer, hash and filesystem operation below has been paid before
    // allocation or the caller's first job_directory/mkdir. No failure refunds.
    reserve_work(
        budget,
        md_length as u64,
        html_length as u64,
        manifest_length as u64,
    )?;
    let mut markdown = Bounded::new(md_length)?;
    render_markdown_to(copy, &mut markdown).map_err(|_| capacity())?;
    let markdown = markdown.finish()?;
    let mut html = Bounded::new(html_length)?;
    render_html_to(copy, &mut html).map_err(|_| capacity())?;
    let html = html.finish()?;
    let md_sha = hex_digest(&markdown);
    let html_sha = hex_digest(&html);
    let validation_sha = hex_digest(VALIDATION);
    let mut manifest_bytes = Bounded::new(manifest_length)?;
    manifest_to(
        &mut manifest_bytes,
        job,
        md_length as u64,
        html_length as u64,
        [&html_sha, &md_sha, &validation_sha],
    )
    .map_err(|_| capacity())?;
    let manifest_bytes = manifest_bytes.finish()?;
    let manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| SnapshotIoError::InvalidPackage)?;
    let manifest_sha256 = hex_digest(&manifest_bytes);
    Ok(PreparedCopy {
        markdown,
        html,
        manifest_bytes,
        manifest,
        manifest_sha256,
    })
}
impl PreparedCopy {
    pub(super) fn stage(self, mut writer: StageWriter) -> Result<WrittenStage, SnapshotIoError> {
        let result = (|| {
            writer.write_stream("reading.md", &mut self.markdown.as_slice())?;
            writer.write_stream("reading.html", &mut self.html.as_slice())?;
            let mut validation = VALIDATION;
            writer.write_stream("validation.json", &mut validation)?;
            writer.write_stream("manifest.json", &mut self.manifest_bytes.as_slice())?;
            writer.finish_existing(&self.manifest, self.manifest_sha256)
        })();
        if result.is_err() {
            writer.discard();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use learning_core::CopyItem;

    fn copy() -> ReadingCopy {
        ReadingCopy {
            items: vec![
                CopyItem::Heading {
                    title: "A[]<&\"'".into(),
                },
                CopyItem::Content {
                    intent: "note".into(),
                    title: "T".into(),
                    display_text: "汉 & <>".into(),
                },
            ],
            evidence: vec!["e*".into()],
        }
    }

    #[test]
    fn custody_copy_prepayment_rejects_insufficient_shared_work_before_filesystem() {
        let budget = SharedJournalBudget::default();
        budget.charge(512 * 1024 * 1024 - 100, 0).unwrap();
        assert!(reserve_work(&budget, 20, 30, 500).is_err());
        assert!(reserve_work(&budget.clone(), 0, 0, 0).is_err());
    }

    #[test]
    fn custody_prepaid_rendering_preserves_exact_escaped_bytes_and_manifest() {
        let budget = SharedJournalBudget::default();
        let prepared = prepare(Uuid::new_v4(), &copy(), &budget).unwrap();
        let markdown = "## A\\[\\]\\<&\"'\n\n### T\n\n汉 & \\<\\>\n\n- e\\*\n";
        let html = "<!doctype html><meta charset=\"utf-8\"><main><h2>A[]&lt;&amp;&quot;&#39;</h2><article><h3>T</h3><p>汉 &amp; &lt;&gt;</p></article><p>e*</p></main>";
        assert_eq!(prepared.markdown, markdown.as_bytes());
        assert_eq!(prepared.html, html.as_bytes());
        assert_eq!(super::super::render_markdown(&copy()), markdown);
        assert_eq!(super::super::render_html(&copy()), html);
        assert_eq!(
            learning_core::canonical_json(&serde_json::to_value(&prepared.manifest).unwrap())
                .as_bytes(),
            prepared.manifest_bytes
        );
        // An independent lower bound: both actual verification passes alone
        // must pay read+hash of payload AND manifest, before any stage write.
        let all = prepared.markdown.len()
            + prepared.html.len()
            + VALIDATION.len()
            + prepared.manifest_bytes.len();
        let usage = budget.usage().unwrap();
        assert!(usage.charged_bytes >= 4 * all as u64);
        assert!(usage.charged_entries >= 4 * 2);
        assert_eq!(
            budget.clone().usage().unwrap().charged_bytes,
            usage.charged_bytes
        );
    }

    #[test]
    fn custody_counting_rejects_expanded_file_limits_before_allocating_output() {
        let copy = ReadingCopy {
            items: vec![CopyItem::Heading {
                title: "<".repeat(SNAPSHOT_MAX_JSON_FILE_BYTES / 4),
            }],
            evidence: vec![],
        };
        let budget = SharedJournalBudget::default();
        assert!(prepare(Uuid::new_v4(), &copy, &budget).is_err());
        assert!(
            budget.usage().unwrap().charged_bytes > 0,
            "failed counted work is never refunded"
        );
    }

    #[test]
    fn custody_bounded_second_render_cannot_grow_past_paid_output() {
        let mut bounded = Bounded::new(1).unwrap();
        fmt::Write::write_str(&mut bounded, "x").unwrap();
        assert!(fmt::Write::write_str(&mut bounded, "y").is_err());
        assert_eq!(bounded.finish().unwrap(), b"x");
    }
}
