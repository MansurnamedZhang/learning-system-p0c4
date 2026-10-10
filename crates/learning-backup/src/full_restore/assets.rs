use crate::{BackupError, FileRecord};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

pub(crate) fn copy_original(
    input: &mut impl Read,
    output: &mut impl Write,
    record: &FileRecord,
) -> Result<(), BackupError> {
    if !crate::valid_digest(&record.sha256) {
        return Err(BackupError::Invalid("original digest"));
    }
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count = count.checked_add(n as u64).ok_or(BackupError::Overflow)?;
        if count > record.size {
            return Err(BackupError::Invalid("original longer than manifest"));
        }
        hash.update(&buffer[..n]);
        output.write_all(&buffer[..n])?;
    }
    if count != record.size || hex::encode(hash.finalize()) != record.sha256 {
        return Err(BackupError::Invalid("original differs from manifest"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_assets_copy_preserves_empty_and_binary_originals() {
        for mut bytes in [b"".as_slice(), b"\0\\.\nSELECT 1;\xff".as_slice()] {
            let record = FileRecord {
                path: "assets/sha256/aa/original".into(),
                size: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(bytes)),
            };
            let expected = bytes;
            let mut output = Vec::new();
            copy_original(&mut bytes, &mut output, &record).unwrap();
            assert_eq!(output, expected);
        }
    }
    #[test]
    fn full_assets_corruption_blocks_recovery_receipt() {
        let record = FileRecord {
            path: "assets/sha256/aa/original".into(),
            size: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
        };
        for mut bytes in [b"ab".as_slice(), b"abcd", b"abd"] {
            assert!(copy_original(&mut bytes, &mut Vec::new(), &record).is_err());
        }
        let mut exact = Vec::new();
        copy_original(&mut b"abc".as_slice(), &mut exact, &record).unwrap();
        assert_eq!(exact, b"abc");
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn open_relative(
    root: &learning_assets::backup_fs::BackupDir,
    path: &str,
) -> Result<std::fs::File, BackupError> {
    let parts: Vec<_> = path.split('/').collect();
    if parts.is_empty()
        || parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains('\\'))
    {
        return Err(BackupError::Invalid("package relative file"));
    }
    let mut dir = root.try_clone()?;
    for part in &parts[..parts.len() - 1] {
        dir = dir.open_dir(part)?;
    }
    Ok(dir.open_file(parts[parts.len() - 1])?)
}
#[cfg(target_os = "linux")]
pub(crate) fn restore_all(
    package: &learning_assets::backup_fs::BackupDir,
    target: &learning_assets::backup_fs::BackupDir,
    plan: &crate::BackupPlan,
) -> Result<(), BackupError> {
    if !target.list_bounded(1)?.is_empty() {
        return Err(BackupError::Invalid("original target is not empty"));
    }
    if !plan.asset_files().is_empty() {
        let sha = target.create_dir("sha256")?;
        for record in plan.asset_files() {
            let shard = &record.sha256[..2];
            let dir = match sha.kind(shard) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => sha.create_dir(shard)?,
                _ => sha.open_dir(shard)?,
            };
            let mut source = open_relative(package, &record.path)?;
            let mut dest = dir.create_file(&record.sha256)?;
            copy_original(&mut source, &mut dest, record)?;
            dest.sync_all()?;
            dir.sync()?;
        }
        sha.sync()?;
    }
    target.sync()?;
    verify_all(target, plan)
}
#[cfg(target_os = "linux")]
pub(crate) fn verify_all(
    target: &learning_assets::backup_fs::BackupDir,
    plan: &crate::BackupPlan,
) -> Result<(), BackupError> {
    use std::collections::{BTreeMap, BTreeSet};
    let expected: BTreeSet<_> = plan
        .asset_files()
        .iter()
        .map(|r| r.sha256.clone())
        .collect();
    let records: BTreeMap<_, _> = plan
        .asset_files()
        .iter()
        .map(|r| (r.sha256.as_str(), r))
        .collect();
    let mut actual = BTreeSet::new();
    for root in target.list_bounded(1)? {
        if root != "sha256" {
            return Err(BackupError::Invalid("extra original root"));
        }
        let sha = target.open_dir(&root)?;
        for shard in sha.list_bounded(256)? {
            if shard.len() != 2
                || !shard
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            {
                return Err(BackupError::Invalid("extra original shard"));
            }
            let dir = sha.open_dir(&shard)?;
            let names = dir.list_bounded(crate::MAX_LOGICAL_ASSETS)?;
            if names.is_empty() {
                return Err(BackupError::Invalid("extra empty original shard"));
            }
            for digest in names {
                if !expected.contains(&digest)
                    || !digest.starts_with(&shard)
                    || !actual.insert(digest.clone())
                {
                    return Err(BackupError::Invalid("extra original bytes"));
                }
                let record = records
                    .get(digest.as_str())
                    .ok_or(BackupError::Invalid("original record"))?;
                copy_original(&mut dir.open_file(&digest)?, &mut std::io::sink(), record)?;
            }
        }
    }
    if actual != expected {
        return Err(BackupError::Invalid("original byte set differs"));
    }
    Ok(())
}
