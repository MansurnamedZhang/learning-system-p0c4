use learning_backup::{BackupError, RestoreDatabaseImported, RestorePreflight};
use std::path::Path;

#[test]
fn pg_restore_entrypoint_requires_the_opaque_locked_preflight() {
    fn compile_only(
        checked: RestorePreflight,
        executable: &Path,
        private_pgpass: &Path,
    ) -> Result<RestoreDatabaseImported, BackupError> {
        checked.restore_database(executable, private_pgpass)
    }
    let _ = compile_only
        as fn(RestorePreflight, &Path, &Path) -> Result<RestoreDatabaseImported, BackupError>;
}
