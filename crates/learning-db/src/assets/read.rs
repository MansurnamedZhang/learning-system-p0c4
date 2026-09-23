use super::{AssetMedia, AssetRecord};
use crate::storage;
use learning_core::{AssetRef, ContentError, Principal};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub(super) struct AssetRow {
    pub space_id: Uuid,
    pub id: Uuid,
    pub sha256: String,
    pub byte_size: i64,
    pub storage_key: String,
    pub media_type: String,
    pub original_file_name: String,
}

impl AssetRow {
    pub(super) fn record(self) -> AssetRecord {
        AssetRecord {
            reference: AssetRef {
                space_id: self.space_id,
                asset_id: self.id,
            },
            sha256: self.sha256,
            byte_size: self.byte_size,
            storage_key: self.storage_key,
            media: AssetMedia {
                media_type: self.media_type,
                original_file_name: self.original_file_name,
            },
        }
    }
}

pub(super) async fn receipt(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    request: Uuid,
) -> Result<AssetRecord, ContentError> {
    let row: Option<AssetRow> = sqlx::query_as(
        "SELECT a.space_id,a.id,a.sha256,a.byte_size,a.storage_key,a.media_type,a.original_file_name \
         FROM public.upload_receipt r JOIN public.asset a ON (a.space_id,a.id)=(r.space_id,r.asset_id) \
         WHERE r.actor_id=$1 AND r.request_id=$2",
    )
    .bind(actor.actor_id)
    .bind(request)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)?;
    row.map(AssetRow::record).ok_or(ContentError::Storage)
}
