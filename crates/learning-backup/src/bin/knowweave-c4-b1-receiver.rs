//! Fixed stdio receiver mode only; foundation never reads stdin or a key.
fn main() {
    if std::env::args_os().nth(1).is_some() || run().is_err() {
        eprintln!("offhost B1 receiver refused");
        std::process::exit(1);
    }
}

fn run() -> Result<(), learning_backup::BackupError> {
    #[cfg(target_os = "linux")]
    {
        learning_backup::run_installed_offhost_b1_receiver()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(learning_backup::BackupError::Invalid(
            "offhost B1 requires Linux",
        ))
    }
}
