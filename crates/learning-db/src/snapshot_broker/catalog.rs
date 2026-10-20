//! One fixed profile definition is shared with the explicit owner installer.
//! The DO block is SELECT/RAISE/assignment only; canonical prosrc literals are
//! compared as data, never executed. No sensitive catalog data is read.
use super::{INSTALL_SQL, ObservationError};
use sqlx::{Postgres, Transaction};

pub(super) enum Phase {
    InstallBefore,
    InstallAfter,
    Observe,
}
impl Phase {
    fn fixed_value(&self) -> &'static str {
        match self {
            Self::InstallBefore => "install_before",
            Self::InstallAfter => "install_after",
            Self::Observe => "observe",
        }
    }
}

pub(super) fn profile() -> Result<&'static str, ObservationError> {
    let mut prefix = INSTALL_SQL.split("-- C0_PROFILE_BEGIN\n");
    prefix.next();
    let section = prefix.next().ok_or(ObservationError::Invalid(
        "snapshot claim profile source missing",
    ))?;
    if prefix.next().is_some() {
        return Err(ObservationError::Invalid(
            "snapshot claim duplicate profile source",
        ));
    }
    let mut suffix = section.split("-- C0_PROFILE_END\n");
    let body = suffix.next().ok_or(ObservationError::Invalid(
        "snapshot claim profile source missing",
    ))?;
    if suffix.next().is_none() || suffix.next().is_some() {
        return Err(ObservationError::Invalid(
            "snapshot claim profile source differs",
        ));
    }
    Ok(body)
}

pub(super) async fn set_phase(
    tx: &mut Transaction<'_, Postgres>,
    phase: Phase,
) -> Result<(), ObservationError> {
    sqlx::query("SELECT pg_catalog.set_config('knowweave.snapshot_claim_profile',$1,true)")
        .bind(phase.fixed_value())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn verify(
    tx: &mut Transaction<'_, Postgres>,
    phase: Phase,
) -> Result<(), ObservationError> {
    set_phase(tx, phase).await?;
    sqlx::raw_sql(profile()?).execute(&mut **tx).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_profile_has_no_install_ddl_or_secret_data_access() {
        let text = profile().unwrap();
        assert!(text.starts_with("DO $snapshot_claim_profile_v1$"));
        assert!(!text.contains("CREATE FUNCTION"));
        assert!(!text.contains("GRANT EXECUTE"));
        assert!(!text.contains("rolpassword"));
        assert!(!text.contains("FROM pg_catalog.pg_authid"));
    }
}
