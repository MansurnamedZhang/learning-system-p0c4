//! Fixed installed destination verifier; stdout contains only canonical witness.
fn main() {
    if run().is_err() {
        eprintln!("destination verification refused");
        std::process::exit(1);
    }
}
fn run() -> Result<(), learning_backup::BackupError> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 1 {
        return Err(learning_backup::BackupError::Invalid(
            "one canonical backup UUID required",
        ));
    }
    let id = uuid::Uuid::parse_str(&args[0])
        .map_err(|_| learning_backup::BackupError::Invalid("canonical backup UUID"))?;
    if id.get_version_num() != 4 || id.to_string() != args[0] {
        return Err(learning_backup::BackupError::Invalid(
            "canonical backup UUID",
        ));
    }
    let registry = learning_backup::ManagementRegistry::open_installed()?;
    let lease = registry.try_lock()?;
    let verified = learning_backup::verify_enrolled_destination(&lease, id)?;
    let signer = learning_backup::DestinationSigner::open_installed()?;
    let witness = verified.sign_statement(&signer)?;
    use std::io::Write;
    std::io::stdout().lock().write_all(&witness)?;
    Ok(())
}
