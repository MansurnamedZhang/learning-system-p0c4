#[cfg(target_os = "linux")]
use learning_backup::GatePhase;
use learning_backup::SourceGateJournal;
use uuid::Uuid;

#[cfg(target_os = "linux")]
#[test]
fn durable_journal_recovers_consecutive_phases_and_rejects_extra_entries() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let root = std::env::temp_dir().join(format!("c4-gate-{}", Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let id = Uuid::new_v4();
    let mut journal = SourceGateJournal::start(&root, id).unwrap();
    journal.advance(GatePhase::Closed, None).unwrap();
    journal.advance(GatePhase::Drained, None).unwrap();
    journal
        .advance(GatePhase::DumpAndIndexDurable, Some(&"a".repeat(64)))
        .unwrap();
    journal
        .advance(GatePhase::PinsDurable, Some(&"b".repeat(64)))
        .unwrap();
    journal.advance(GatePhase::ReleaseReady, None).unwrap();
    let child = root.join(format!("{id}.control"));
    fs::create_dir(child.join("released.json")).unwrap();
    assert!(journal.advance(GatePhase::Released, None).is_err());
    assert_eq!(journal.record().phase(), GatePhase::ReleaseReady);
    fs::remove_dir(child.join("released.json")).unwrap();
    drop(journal);
    let recovered = SourceGateJournal::recover(&root, id).unwrap();
    assert_eq!(recovered.record().phase(), GatePhase::ReleaseReady);
    assert!(SourceGateJournal::start(&root, id).is_err());
    fs::write(root.join(format!("{id}.control")).join("unexpected"), b"x").unwrap();
    assert!(SourceGateJournal::recover(&root, id).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(not(target_os = "linux"))]
#[test]
fn private_gate_journal_refuses_unsupported_filesystems() {
    let id = Uuid::new_v4();
    assert!(SourceGateJournal::start(std::path::Path::new("C:\\no-real-gate"), id).is_err());
}
