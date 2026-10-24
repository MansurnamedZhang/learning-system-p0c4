//! Test-only Attention coverage. Content observations never grant authority.
use crate::BackupError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HypothesisHistory {
    pub historical: u64,
    pub current: u64,
}

impl HypothesisHistory {
    pub(crate) fn require_complete(self) -> Result<(), BackupError> {
        if self.historical == 0 || self.current == 0 {
            return Err(BackupError::Invalid(
                "attention historical-current fixture is incomplete",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::HypothesisHistory;

    #[test]
    fn attention_history_requires_both_actual_revisions() {
        for (historical, current) in [(0, 0), (1, 0), (0, 1)] {
            assert!(
                HypothesisHistory {
                    historical,
                    current
                }
                .require_complete()
                .is_err()
            );
        }
        assert!(
            HypothesisHistory {
                historical: 1,
                current: 1
            }
            .require_complete()
            .is_ok()
        );
    }
}
