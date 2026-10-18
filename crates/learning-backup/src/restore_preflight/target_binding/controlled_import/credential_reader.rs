//! Private test-only credential protocol; never a public read capability.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
use super::ImportFailure;
use std::{future::Future, time::Instant};

// No interpolated filename, command, credential or environment. The same
// constant is used by the fixed command and by Root's public dummy probe.
pub(super) const PROGRAM: &str = r#"BEGIN {
    $SIG{__WARN__}=sub {};
    $SIG{__DIE__}=sub { print STDERR "KW_C4_CREDENTIAL_REFUSED\n"; CORE::exit(41); };
}
use strict;
use warnings;
use Fcntl qw(O_RDONLY O_NOFOLLOW O_NONBLOCK F_SETFD FD_CLOEXEC);
use Digest::SHA;
use Config;
$SIG{ALRM}=sub { die; };
alarm(5);
die unless $<==999 && $>==999 && $Config{ivsize}==8;
for my $tool (
    ["/usr/bin/perl",3804464,"f01fa7776dc21c9e4b5f60b2d231ca4d96dab958b8d06aff611cb1c16f871574"],
    ["/usr/bin/env",48536,"615c46b39130a04a08da04163542ce7ce1164fa4b35408efb43aac0a8a9f7ae5"]
) {
    sysopen(my $fh,$tool->[0],O_RDONLY|O_NOFOLLOW|O_NONBLOCK) or die;
    my @m=stat($fh);
    die unless @m && (($m[2]&0170000)==0100000) && (($m[2]&07777)==0755)
        && $m[4]==0 && $m[7]==$tool->[1];
    my $hash=Digest::SHA->new(256);$hash->addfile($fh);
    die unless $hash->hexdigest eq $tool->[2];
    close($fh) or die;
}
sysopen(my $fd,"/run/secrets/admin_password",O_RDONLY|O_NOFOLLOW|O_NONBLOCK) or die;
fcntl($fd,F_SETFD,FD_CLOEXEC) or die;
my @before=stat($fd);
die unless @before && (($before[2]&0170000)==0100000) && (($before[2]&07777)==0600)
    && $before[3]==1 && $before[4]==999 && $before[5]==999 && $before[7]==65;
my $data="";
while (length($data)<66) {
    my $part="";my $n=sysread($fd,$part,66-length($data));
    die unless defined($n);last if $n==0;$data.=$part;
}
die unless length($data)==65 && $data=~/\A[0-9a-f]{64}\n\z/;
my @after=stat($fd);
die unless @after && join("|",@before[0,1,2,3,4,5,7,9,10]) eq join("|",@after[0,1,2,3,4,5,7,9,10]);
close($fd) or die;
binmode(STDOUT,":raw") or die;
my $out="KW_C4_ADMIN_FD_V1|".join("|",@before[0,1,4,5,2,3,7])."\n".$data;
my $offset=0;
while ($offset<length($out)) {
    my $n=syswrite(STDOUT,$out,length($out)-$offset,$offset);
    die unless defined($n) && $n>0;$offset+=$n;
}
alarm(0);
"#;

pub(super) struct Secret(String);
impl Secret {
    pub(super) fn for_connection(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    nlink: u64,
    size: u64,
}
impl FileIdentity {
    fn safe(self) -> bool {
        self.dev > 0
            && self.ino > 0
            && self.uid == 999
            && self.gid == 999
            && self.mode == 0o100600
            && self.nlink == 1
            && self.size == 65
    }
}

fn validate_description(
    bytes: &[u8],
    id: &str,
    image: &str,
    database: &str,
) -> Result<(), ImportFailure> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Description {
        id: String,
        image: String,
        env: Vec<String>,
    }
    if bytes.len() > 4096 {
        return Err(ImportFailure::Identity);
    }
    let value: Description = serde_json::from_slice(bytes).map_err(|_| ImportFailure::Identity)?;
    if value.id != id || value.image != image || value.env.len() != 10 {
        return Err(ImportFailure::Identity);
    }
    let expected = std::collections::BTreeMap::from([
        (
            "PATH",
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/lib/postgresql/18/bin",
        ),
        ("GOSU_VERSION", "1.19"),
        ("LANG", "en_US.utf8"),
        ("PG_MAJOR", "18"),
        ("PG_VERSION", "18.6-1.pgdg12+2"),
        ("PGDATA", "/var/lib/postgresql/18/docker"),
        ("POSTGRES_USER", "postgres"),
        ("POSTGRES_DB", "postgres"),
        ("POSTGRES_PASSWORD_FILE", "/run/secrets/postgres_password"),
        ("C4_TARGET_DATABASE", database),
    ]);
    let mut observed = std::collections::BTreeMap::new();
    for entry in &value.env {
        let (key, val) = entry.split_once('=').ok_or(ImportFailure::Identity)?;
        if observed.insert(key, val).is_some() {
            return Err(ImportFailure::Identity);
        }
    }
    if observed != expected {
        return Err(ImportFailure::Identity);
    }
    Ok(())
}
fn parse_frame(bytes: &[u8], local: FileIdentity) -> Result<Secret, ImportFailure> {
    let invalid = || ImportFailure::Identity;
    if bytes.len() > 512 || !local.safe() {
        return Err(invalid());
    }
    let split = bytes.iter().position(|b| *b == b'\n').ok_or_else(invalid)?;
    let header = std::str::from_utf8(&bytes[..split]).map_err(|_| invalid())?;
    let fields: Vec<_> = header.split('|').collect();
    if fields.len() != 8 || fields[0] != "KW_C4_ADMIN_FD_V1" {
        return Err(invalid());
    }
    let number = |s: &str| -> Result<u64, ImportFailure> {
        let value = s.parse::<u64>().map_err(|_| invalid())?;
        if s != value.to_string() {
            return Err(invalid());
        }
        Ok(value)
    };
    let meta = FileIdentity {
        dev: number(fields[1])?,
        ino: number(fields[2])?,
        uid: number(fields[3])?.try_into().map_err(|_| invalid())?,
        gid: number(fields[4])?.try_into().map_err(|_| invalid())?,
        mode: number(fields[5])?.try_into().map_err(|_| invalid())?,
        nlink: number(fields[6])?,
        size: number(fields[7])?,
    };
    let body = &bytes[split + 1..];
    if meta != local
        || !meta.safe()
        || body.len() != 65
        || body[64] != b'\n'
        || !body[..64]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return Err(invalid());
    }
    Ok(Secret(
        std::str::from_utf8(&body[..64])
            .map_err(|_| invalid())?
            .to_owned(),
    ))
}
fn before_deadline(deadline: Instant) -> Result<(), ImportFailure> {
    if Instant::now() >= deadline {
        Err(ImportFailure::Deadline)
    } else {
        Ok(())
    }
}

// Error-only I/O seam. No constructor, command, path or returned authority.
trait RejectionHolder {
    fn disable(&mut self);
    fn quarantine(&mut self) -> impl Future<Output = bool> + Send;
    fn release(&mut self);
}
async fn reject_reader(holder: &mut impl RejectionHolder, reason: ImportFailure) -> ImportFailure {
    holder.disable();
    let stopped = holder.quarantine().await;
    holder.release();
    if stopped {
        reason
    } else {
        ImportFailure::UnconfirmedIsolation
    }
}

#[cfg(target_os = "linux")]
pub(super) use native::{read, reject_target_reader};

#[cfg(target_os = "linux")]
mod native {
    use super::super::super as binding;
    use super::super::{
        commands::FixedImportCommand,
        stream::{StreamBudget, spawn_stream},
    };
    use super::*;
    use binding::child_attestation::{Isolation, linux_child};
    use learning_assets::backup_fs::BackupDir;
    use std::{
        fs::{File, Metadata, OpenOptions},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::{MetadataExt, OpenOptionsExt},
        },
        path::{Path, PathBuf},
        time::Duration,
    };

    type Guard = binding::BoundTargetGuard<File, Option<File>>;
    fn identity(meta: &Metadata) -> FileIdentity {
        FileIdentity {
            dev: meta.dev(),
            ino: meta.ino(),
            uid: meta.uid(),
            gid: meta.gid(),
            mode: meta.mode(),
            nlink: meta.nlink(),
            size: meta.size(),
        }
    }
    fn unchanged(a: &Metadata, b: &Metadata) -> bool {
        identity(a) == identity(b)
            && a.mtime() == b.mtime()
            && a.mtime_nsec() == b.mtime_nsec()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
    }
    fn open_entry(dir: &File, name: &std::ffi::CStr) -> Result<File, ImportFailure> {
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(ImportFailure::Identity);
        }
        let file = unsafe { File::from_raw_fd(fd) };
        if !identity(&file.metadata().map_err(|_| ImportFailure::Identity)?).safe() {
            return Err(ImportFailure::Identity);
        }
        Ok(file)
    }
    struct HeldFiles {
        trusted: BackupDir,
        directory: File,
        path: PathBuf,
        admin: File,
        peer: File,
        admin_before: Metadata,
        peer_before: Metadata,
    }
    impl HeldFiles {
        fn open(guard: &Guard) -> Result<Self, ImportFailure> {
            let path = Path::new(&guard.claim.target_path).join("secrets");
            let trusted =
                BackupDir::open_trusted_private_root(&path).map_err(|_| ImportFailure::Identity)?;
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)
                .map_err(|_| ImportFailure::Identity)?;
            let admin = open_entry(&directory, c"admin_password")?;
            let peer = open_entry(&directory, c"postgres_password")?;
            let value = Self {
                admin_before: admin.metadata().map_err(|_| ImportFailure::Identity)?,
                peer_before: peer.metadata().map_err(|_| ImportFailure::Identity)?,
                trusted,
                directory,
                path,
                admin,
                peer,
            };
            value.recheck()?;
            Ok(value)
        }
        fn recheck(&self) -> Result<(), ImportFailure> {
            let err = || ImportFailure::Identity;
            let mut names = self.trusted.list_bounded(3).map_err(|_| err())?;
            names.sort();
            let dir = self.directory.metadata().map_err(|_| err())?;
            let path = std::fs::symlink_metadata(&self.path).map_err(|_| err())?;
            if names != ["admin_password", "postgres_password"]
                || self.trusted.identity().map_err(|_| err())? != (dir.dev(), dir.ino())
                || path.dev() != dir.dev()
                || path.ino() != dir.ino()
                || !path.is_dir()
                || path.uid() != 0
                || path.mode() & 0o7777 != 0o700
            {
                return Err(err());
            }
            for (file, before, name) in [
                (&self.admin, &self.admin_before, c"admin_password"),
                (&self.peer, &self.peer_before, c"postgres_password"),
            ] {
                let current = file.metadata().map_err(|_| err())?;
                let named = open_entry(&self.directory, name)?;
                if !unchanged(before, &current)
                    || !unchanged(before, &named.metadata().map_err(|_| err())?)
                {
                    return Err(err());
                }
            }
            Ok(())
        }
    }
    // Both fixed RPCs are bounded by the same absolute reader deadline. No
    // observation records, stderr samples, environment or credential logging.
    async fn rpc(
        fixed: FixedImportCommand,
        budget: StreamBudget,
        deadline: Instant,
        lines: usize,
    ) -> Result<Vec<u8>, ImportFailure> {
        before_deadline(deadline)?;
        binding::linux::trusted_docker_path().map_err(|_| ImportFailure::Identity)?;
        let mut command = tokio::process::Command::new("/usr/bin/docker");
        command
            .args(&fixed.argv()[1..])
            .env_clear()
            .env("DOCKER_HOST", "unix:///var/run/docker.sock");
        let mut owner =
            spawn_stream(command, budget).map_err(|_| ImportFailure::UnconfirmedIsolation)?;
        let result = async {
            owner.close_input().await?;
            let mut bytes = Vec::new();
            for _ in 0..lines {
                let line = owner
                    .next_line(deadline)
                    .await?
                    .ok_or(ImportFailure::Protocol)?;
                bytes.extend_from_slice(&line);
            }
            if owner.next_line(deadline).await?.is_some() {
                return Err(ImportFailure::Protocol);
            }
            owner.finish().await?;
            before_deadline(deadline)?;
            Ok(bytes)
        }
        .await;
        match result {
            Ok(bytes) => Ok(bytes),
            Err(_) => {
                // Reaping the local CLI is not evidence of remote exec exit.
                let _ = owner.kill_and_wait().await;
                Err(ImportFailure::UnconfirmedIsolation)
            }
        }
    }
    fn guard_recheck(guard: &Guard, deadline: Instant) -> Result<(), ImportFailure> {
        before_deadline(deadline)?;
        guard.recheck().map_err(|_| ImportFailure::Identity)?;
        // Original synchronous guard RPCs are not interruptible by this reader.
        before_deadline(deadline)
    }
    async fn describe(guard: &mut Guard, deadline: Instant) -> Result<(), ImportFailure> {
        let bytes = rpc(
            FixedImportCommand::credential_describe(guard)?,
            StreamBudget::credential_description(deadline),
            deadline,
            1,
        )
        .await?;
        validate_description(
            &bytes,
            &guard.claim.container_id,
            &guard.claim.image_id,
            &guard.claim.database,
        )
    }
    pub(in super::super) async fn read(guard: &mut Guard) -> Result<Secret, ImportFailure> {
        let deadline = Instant::now() + Duration::from_secs(15);
        guard_recheck(guard, deadline)?;
        let held = HeldFiles::open(guard)?;
        describe(guard, deadline).await?;
        let bytes = rpc(
            FixedImportCommand::credential_read(guard)?,
            StreamBudget::credential(deadline),
            deadline,
            2,
        )
        .await?;
        let password = parse_frame(&bytes, identity(&held.admin_before))?;
        held.recheck()?;
        describe(guard, deadline).await?;
        guard_recheck(guard, deadline)?;
        held.recheck()?;
        before_deadline(deadline)?;
        Ok(password)
    }
    struct LiveHolder(Option<Guard>);
    impl RejectionHolder for LiveHolder {
        fn disable(&mut self) {
            self.0
                .as_ref()
                .expect("held reader guard")
                .child_usable
                .set(false);
        }
        async fn quarantine(&mut self) -> bool {
            let claim = self.0.as_ref().expect("held reader guard").claim.clone();
            linux_child::candidate_quarantine(&claim).await == Isolation::Stopped
        }
        fn release(&mut self) {
            drop(self.0.take());
        }
    }
    pub(in super::super) async fn reject_target_reader(
        guard: Guard,
        reason: ImportFailure,
    ) -> ImportFailure {
        reject_reader(&mut LiveHolder(Some(guard)), reason).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const IMAGE: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const DB: &str = "learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3";

    fn description() -> serde_json::Value {
        json!({"id":ID,"image":IMAGE,"env":[
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/lib/postgresql/18/bin",
            "GOSU_VERSION=1.19","LANG=en_US.utf8","PG_MAJOR=18",
            "PG_VERSION=18.6-1.pgdg12+2","PGDATA=/var/lib/postgresql/18/docker",
            "POSTGRES_USER=postgres","POSTGRES_DB=postgres",
            "POSTGRES_PASSWORD_FILE=/run/secrets/postgres_password",
            format!("C4_TARGET_DATABASE={DB}")]})
    }
    fn admit(value: &serde_json::Value) -> Result<(), ImportFailure> {
        validate_description(&serde_json::to_vec(value).unwrap(), ID, IMAGE, DB)
    }
    fn identity() -> FileIdentity {
        FileIdentity {
            dev: 24,
            ino: 12345,
            uid: 999,
            gid: 999,
            mode: 0o100600,
            nlink: 1,
            size: 65,
        }
    }
    fn frame() -> Vec<u8> {
        let mut bytes = b"KW_C4_ADMIN_FD_V1|24|12345|999|999|33152|1|65\n".to_vec();
        bytes.extend_from_slice(&[b'a'; 64]);
        bytes.push(b'\n');
        bytes
    }
    #[test]
    fn exact_description_admits_only_frozen_image_and_provisioner_env() {
        assert_eq!(admit(&description()), Ok(()));
    }
    #[test]
    fn environment_refuses_duplicates_extras_loader_and_interpreter_injection() {
        for extra in [
            "LD_PRELOAD=/private",
            "LD_AUDIT=/private",
            "PERL5OPT=-Mprivate",
            "PERL5LIB=/private",
            "LANG=en_US.utf8",
            "EXTRA=private",
        ] {
            let mut value = description();
            value["env"].as_array_mut().unwrap().push(json!(extra));
            assert_eq!(admit(&value), Err(ImportFailure::Identity));
        }
        for i in 0..10 {
            let mut value = description();
            value["env"][i] = json!("WRONG=private");
            assert_eq!(admit(&value), Err(ImportFailure::Identity));
        }
        // Same cardinality reaches duplicate detection rather than only the cap.
        let mut duplicate = description();
        duplicate["env"][9] = duplicate["env"][2].clone();
        assert_eq!(admit(&duplicate), Err(ImportFailure::Identity));
        for key in ["id", "image"] {
            let mut value = description();
            value[key] = json!("foreign");
            assert_eq!(admit(&value), Err(ImportFailure::Identity));
        }
    }
    #[test]
    fn held_inode_frame_returns_only_exact_private_password() {
        let secret = parse_frame(&frame(), identity()).expect("public synthetic frame rejected");
        assert_eq!(secret.for_connection(), "a".repeat(64));
    }
    #[test]
    fn protocol_never_releases_malformed_or_foreign_inode_secret() {
        let mut wrong = identity();
        wrong.ino += 1;
        assert!(matches!(
            parse_frame(&frame(), wrong),
            Err(ImportFailure::Identity)
        ));
        for bytes in [
            b"private-secret-sentinel".to_vec(),
            [frame(), b"extra\n".to_vec()].concat(),
        ] {
            let error = match parse_frame(&bytes, identity()) {
                Err(error) => error,
                Ok(_) => panic!("unsafe frame accepted"),
            };
            assert_eq!(format!("{error:?}"), "Identity");
        }
        for (from, to) in [
            ("|24|", "|024|"),
            ("|12345|", "|12346|"),
            ("|999|999|", "|0|999|"),
            ("|999|999|", "|999|0|"),
            ("|33152|", "|33188|"),
            ("|1|65", "|2|65"),
            ("|1|65", "|1|66"),
        ] {
            let bytes = String::from_utf8(frame())
                .unwrap()
                .replace(from, to)
                .into_bytes();
            assert!(matches!(
                parse_frame(&bytes, identity()),
                Err(ImportFailure::Identity)
            ));
        }
        let mut uppercase = frame();
        let n = uppercase.len();
        uppercase[n - 2] = b'A';
        let mut missing_lf = frame();
        missing_lf.pop();
        for bytes in [uppercase, missing_lf, vec![b'a'; 513]] {
            assert!(matches!(
                parse_frame(&bytes, identity()),
                Err(ImportFailure::Identity)
            ));
        }
    }
    #[test]
    fn expired_final_guard_deadline_cannot_release_a_password() {
        assert_eq!(
            before_deadline(Instant::now() - Duration::from_secs(1)),
            Err(ImportFailure::Deadline)
        );
    }
    struct TraceHolder {
        held: bool,
        disabled: bool,
        stopped: bool,
        events: Vec<&'static str>,
    }
    impl RejectionHolder for TraceHolder {
        fn disable(&mut self) {
            assert!(self.held);
            self.disabled = true;
            self.events.push("disable");
        }
        async fn quarantine(&mut self) -> bool {
            assert!(self.held && self.disabled);
            self.events.push("quarantine-await");
            tokio::task::yield_now().await;
            assert!(self.held);
            self.events.push("quarantine-settled");
            self.stopped
        }
        fn release(&mut self) {
            assert!(self.held);
            self.held = false;
            self.events.push("drop");
        }
    }
    #[tokio::test]
    async fn target_reader_error_disables_and_awaits_quarantine_before_guard_drop() {
        let mut holder = TraceHolder {
            held: true,
            disabled: false,
            stopped: true,
            events: vec![],
        };
        assert_eq!(
            reject_reader(&mut holder, ImportFailure::Identity).await,
            ImportFailure::Identity
        );
        assert_eq!(
            holder.events,
            ["disable", "quarantine-await", "quarantine-settled", "drop"]
        );
        assert!(!holder.held);
    }
    #[tokio::test]
    async fn target_unknown_read_or_quarantine_never_becomes_known_identity() {
        for (reason, stopped) in [
            (ImportFailure::Identity, false),
            (ImportFailure::UnconfirmedIsolation, true),
        ] {
            let mut holder = TraceHolder {
                held: true,
                disabled: false,
                stopped,
                events: vec![],
            };
            assert_eq!(
                reject_reader(&mut holder, reason).await,
                ImportFailure::UnconfirmedIsolation
            );
        }
    }
}
