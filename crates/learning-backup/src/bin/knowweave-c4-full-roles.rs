//! Fixed read-only installed complete package role observer; no signing.
fn main() {
    if run().is_err() {
        eprintln!("complete role observation refused");
        std::process::exit(1);
    }
}
fn run() -> Result<(), learning_backup::BackupError> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 1 {
        return Err(learning_backup::BackupError::Invalid(
            "one backup UUID required",
        ));
    }
    let id = uuid::Uuid::parse_str(&args[0])
        .map_err(|_| learning_backup::BackupError::Invalid("backup UUID"))?;
    if id.get_version_num() != 4
        || id.get_variant() != uuid::Variant::RFC4122
        || id.to_string() != args[0]
    {
        return Err(learning_backup::BackupError::Invalid(
            "canonical backup UUID required",
        ));
    }
    let bytes = learning_backup::full_restore::verified_complete_role_recipe(id)?;
    use std::io::Write;
    std::io::stdout().lock().write_all(&bytes)?;
    Ok(())
}
