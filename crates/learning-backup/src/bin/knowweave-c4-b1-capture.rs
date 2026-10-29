//! Fixed compiled Capture role. Normal success returns through every Drop.
fn main() {
    if std::env::args_os().nth(1).is_some() || run().is_err() {
        eprintln!("offhost B1 capture refused");
        std::process::exit(1);
    }
}

fn run() -> Result<(), learning_backup::BackupError> {
    #[cfg(target_os = "linux")]
    {
        learning_backup::run_installed_offhost_b1_source()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(learning_backup::BackupError::Invalid(
            "offhost B1 requires Linux",
        ))
    }
}
