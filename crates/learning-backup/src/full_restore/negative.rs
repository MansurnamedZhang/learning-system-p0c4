//! Test evidence only. This never admits a target or a backup.
use crate::BackupError;
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Boundary {
    DumpFreeze,
    CleanTarget,
    Roles,
    WriterRoute,
    OriginalSet,
}

#[derive(Default, Debug, Clone, Serialize)]
pub(crate) struct NegativeEvidence {
    audit_failed: bool,
    injected: Option<Boundary>,
    refused: Option<Boundary>,
    before: Option<Value>,
    after: Option<Value>,
}
impl NegativeEvidence {
    pub(crate) fn failed(&mut self) {
        self.audit_failed = true;
    }
    pub(crate) fn injected(&mut self, stage: Boundary) {
        self.injected = Some(stage);
    }
    pub(crate) fn refused(&mut self, stage: Boundary) {
        self.refused = Some(stage);
    }
    pub(crate) fn before(&mut self, snapshot: Value) {
        self.before = Some(snapshot);
    }
    pub(crate) fn after(&mut self, snapshot: Value) {
        self.after = Some(snapshot);
    }
    pub(crate) fn validate(&self, stage: Boundary, overall_error: bool) -> Result<(), BackupError> {
        if self.audit_failed
            || !overall_error
            || self.injected != Some(stage)
            || self.refused != Some(stage)
        {
            return Err(BackupError::Invalid("negative boundary not proved"));
        }
        let before = self
            .before
            .as_ref()
            .ok_or(BackupError::Invalid("negative before absent"))?;
        let after = self
            .after
            .as_ref()
            .ok_or(BackupError::Invalid("negative after absent"))?;
        if before != after
            || !matches!(before,Value::Object(fields) if fields.len()==3 && fields.contains_key("catalog") && fields.contains_key("assets") && fields.contains_key("attempts"))
        {
            return Err(BackupError::Invalid(
                "negative inventory changed or incomplete",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unrelated_earlier_errors_never_satisfy_injected_boundary() {
        for stage in [
            Boundary::DumpFreeze,
            Boundary::CleanTarget,
            Boundary::Roles,
            Boundary::WriterRoute,
            Boundary::OriginalSet,
        ] {
            let mut evidence = NegativeEvidence::default();
            assert!(evidence.validate(stage, true).is_err());
            evidence.injected(stage);
            assert!(evidence.validate(stage, true).is_err());
            evidence.refused(if stage == Boundary::Roles {
                Boundary::DumpFreeze
            } else {
                Boundary::Roles
            });
            assert!(evidence.validate(stage, true).is_err());
        }
    }
    #[test]
    fn exact_refusal_requires_complete_equal_before_after_inventory() {
        let mut evidence = NegativeEvidence::default();
        evidence.injected(Boundary::CleanTarget);
        evidence.refused(Boundary::CleanTarget);
        let baseline = serde_json::json!({"catalog":{"intentional_dirty_table":true},"assets":{},"attempts":{}});
        evidence.before(baseline.clone());
        assert!(evidence.validate(Boundary::CleanTarget, true).is_err());
        evidence.after(baseline.clone());
        assert!(evidence.validate(Boundary::CleanTarget, true).is_ok());
        let mut audit_error = evidence.clone();
        audit_error.failed();
        assert!(audit_error.validate(Boundary::CleanTarget, true).is_err());
        assert!(evidence.validate(Boundary::CleanTarget, false).is_err());
        for key in ["catalog", "assets", "attempts"] {
            let mut changed = baseline.clone();
            changed[key] = serde_json::json!({"unexpected":1});
            evidence.after(changed);
            assert!(
                evidence.validate(Boundary::CleanTarget, true).is_err(),
                "{key}"
            );
        }
    }
}
