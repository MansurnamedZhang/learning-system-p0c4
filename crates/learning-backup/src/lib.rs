//! Management-only full-backup planning. A plan is never a verified backup.

mod catalog;
pub use catalog::AdminAssetCatalog;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use uuid::Uuid;

pub const BACKUP_CAPABILITY: &str = "backup_full_v1";
pub const BACKUP_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("invalid full-backup contract: {0}")]
    Invalid(&'static str),
    #[error("full-backup count or byte size overflow")]
    Overflow,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupState {
    Staging,
    Sealed,
    Complete,
}

/// One logical `asset` row, whether or not any current reading references it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRow {
    pub space_id: Uuid,
    pub id: Uuid,
    pub sha256: String,
    pub byte_size: i64,
    pub storage_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRecord {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    pub application_build_sha256: String,
    pub postgres_major: u32,
    pub migration_version: u64,
    pub migration_fingerprint: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetIndexV1 {
    format_version: u32,
    assets: Vec<AssetRow>,
}

/// A deterministic database catalog plan. It contains no checked asset bytes,
/// dump, role recipe, target receipt, or authority to restore.
#[derive(Debug, Clone)]
pub struct BackupPlan {
    assets: Vec<AssetRow>,
    logical_asset_count: u64,
    asset_index_bytes: Vec<u8>,
    asset_index_file: FileRecord,
    asset_files: Vec<FileRecord>,
    unique_asset_bytes: u64,
}

impl BackupPlan {
    pub fn from_rows(mut assets: Vec<AssetRow>) -> Result<Self, BackupError> {
        assets.sort_by_key(|a| (a.space_id, a.id));
        let mut ids = BTreeSet::new();
        let mut digests = BTreeMap::<String, (i64, String)>::new();
        let mut unique_asset_bytes = 0_u64;
        for asset in &assets {
            if !ids.insert((asset.space_id, asset.id)) {
                return Err(BackupError::Invalid("duplicate asset identity"));
            }
            if !valid_digest(&asset.sha256)
                || asset.byte_size < 0
                || asset.storage_key != asset_key(&asset.sha256)
            {
                return Err(BackupError::Invalid("asset digest, size, or key"));
            }
            match digests.get(&asset.sha256) {
                Some((size, key)) if *size != asset.byte_size || *key != asset.storage_key => {
                    return Err(BackupError::Invalid("conflicting digest metadata"));
                }
                Some(_) => {}
                None => {
                    unique_asset_bytes = unique_asset_bytes
                        .checked_add(
                            u64::try_from(asset.byte_size).map_err(|_| BackupError::Overflow)?,
                        )
                        .ok_or(BackupError::Overflow)?;
                    digests.insert(
                        asset.sha256.clone(),
                        (asset.byte_size, asset.storage_key.clone()),
                    );
                }
            }
        }
        let logical_asset_count = u64::try_from(assets.len()).map_err(|_| BackupError::Overflow)?;
        let asset_index_bytes = serde_json::to_vec(&AssetIndexV1 {
            format_version: BACKUP_FORMAT_VERSION,
            assets: assets.clone(),
        })?;
        let asset_index_file = FileRecord {
            path: "asset-index.json".into(),
            size: u64::try_from(asset_index_bytes.len()).map_err(|_| BackupError::Overflow)?,
            sha256: digest(&asset_index_bytes),
        };
        let asset_files = digests
            .into_iter()
            .map(|(sha256, (size, _))| {
                Ok(FileRecord {
                    path: format!("assets/{}", asset_key(&sha256)),
                    size: u64::try_from(size).map_err(|_| BackupError::Overflow)?,
                    sha256,
                })
            })
            .collect::<Result<Vec<_>, BackupError>>()?;
        Ok(Self {
            assets,
            logical_asset_count,
            asset_index_bytes,
            asset_index_file,
            asset_files,
            unique_asset_bytes,
        })
    }

    pub fn state(&self) -> BackupState {
        BackupState::Staging
    }
    pub fn logical_asset_count(&self) -> u64 {
        self.logical_asset_count
    }
    pub fn unique_asset_bytes(&self) -> u64 {
        self.unique_asset_bytes
    }
    pub fn assets(&self) -> &[AssetRow] {
        &self.assets
    }
    pub fn asset_index_bytes(&self) -> &[u8] {
        &self.asset_index_bytes
    }
    pub fn asset_index_file(&self) -> &FileRecord {
        &self.asset_index_file
    }
    pub fn asset_files(&self) -> &[FileRecord] {
        &self.asset_files
    }
}

/// Describes a proposed full package; this value is not a completion receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifestV1 {
    pub capability: String,
    pub format_version: u32,
    pub backup_id: Uuid,
    pub source: SourceIdentity,
    pub logical_asset_count: u64,
    pub unique_asset_bytes: u64,
    pub files: Vec<FileRecord>,
}

impl BackupManifestV1 {
    pub fn from_plan(
        backup_id: Uuid,
        source: SourceIdentity,
        plan: &BackupPlan,
        database_dump: FileRecord,
        role_recipe: FileRecord,
    ) -> Result<Self, BackupError> {
        if database_dump.path != "database.dump" || role_recipe.path != "roles.json" {
            return Err(BackupError::Invalid("required file path"));
        }
        let mut files = vec![plan.asset_index_file.clone(), database_dump, role_recipe];
        files.extend(plan.asset_files.iter().cloned());
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let manifest = Self {
            capability: BACKUP_CAPABILITY.into(),
            format_version: BACKUP_FORMAT_VERSION,
            backup_id,
            source,
            logical_asset_count: plan.logical_asset_count(),
            unique_asset_bytes: plan.unique_asset_bytes,
            files,
        };
        manifest.validate_with_index(plan.asset_index_bytes())?;
        Ok(manifest)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, BackupError> {
        Ok(serde_json::to_vec(self)?)
    }
    pub fn canonical_sha256(&self) -> Result<String, BackupError> {
        Ok(digest(&self.canonical_bytes()?))
    }

    /// Pure inventory check. Task 2 must verify directory entries, file bytes,
    /// target receipt, and independent fault-domain publication separately.
    pub fn validate_with_index(&self, index_bytes: &[u8]) -> Result<(), BackupError> {
        if self.capability != BACKUP_CAPABILITY || self.format_version != BACKUP_FORMAT_VERSION {
            return Err(BackupError::Invalid("unsupported format"));
        }
        if self.backup_id.is_nil()
            || !valid_digest(&self.source.application_build_sha256)
            || !valid_digest(&self.source.migration_fingerprint)
            || self.source.postgres_major == 0
            || self.source.migration_version == 0
        {
            return Err(BackupError::Invalid("source identity"));
        }
        let index: AssetIndexV1 = serde_json::from_slice(index_bytes)?;
        if index.format_version != BACKUP_FORMAT_VERSION {
            return Err(BackupError::Invalid("asset index format"));
        }
        let plan = BackupPlan::from_rows(index.assets)?;
        if plan.asset_index_bytes() != index_bytes
            || self.logical_asset_count != plan.logical_asset_count()
            || self.unique_asset_bytes != plan.unique_asset_bytes()
        {
            return Err(BackupError::Invalid("noncanonical asset index or counts"));
        }
        let mut expected = vec![plan.asset_index_file.clone()];
        expected.extend(plan.asset_files.iter().cloned());
        let mut dump = None;
        let mut roles = None;
        for file in &self.files {
            if !valid_digest(&file.sha256) {
                return Err(BackupError::Invalid("file digest"));
            }
            match file.path.as_str() {
                "database.dump" => dump = Some(file.clone()),
                "roles.json" => roles = Some(file.clone()),
                _ => {}
            }
        }
        let dump = dump.ok_or(BackupError::Invalid("missing dump"))?;
        let roles = roles.ok_or(BackupError::Invalid("missing role recipe"))?;
        if dump.size == 0 || roles.size == 0 {
            return Err(BackupError::Invalid("empty required file"));
        }
        expected.push(dump);
        expected.push(roles);
        expected.sort_by(|a, b| a.path.cmp(&b.path));
        if self.files != expected {
            return Err(BackupError::Invalid(
                "file inventory differs from asset index",
            ));
        }
        Ok(())
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn asset_key(sha256: &str) -> String {
    format!("sha256/{}/{}", &sha256[..2], sha256)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
