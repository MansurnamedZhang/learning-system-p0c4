use super::ImportFailure;

#[derive(Debug)]
pub(super) struct FixedImportCommand(Vec<String>);
impl FixedImportCommand {
    #[cfg(test)]
    fn credential_read_id(container_id: &str) -> Result<Self, ImportFailure> {
        if !super::super::exact_id(container_id) {
            return Err(ImportFailure::Identity);
        }
        let mut args = [
            "/usr/bin/docker",
            "exec",
            "--user",
            "999:999",
            container_id,
            "/usr/bin/env",
            "-i",
            "LC_ALL=C",
            "PATH=/usr/bin:/bin",
            "/usr/bin/perl",
            "-T",
            "-e",
        ]
        .map(String::from)
        .to_vec();
        args.push(super::credential_reader::PROGRAM.to_owned());
        Ok(Self(args))
    }
    #[cfg(test)]
    fn credential_describe_id(container_id: &str) -> Result<Self, ImportFailure> {
        if !super::super::exact_id(container_id) {
            return Err(ImportFailure::Identity);
        }
        Ok(Self(
            [
                "/usr/bin/docker",
                "container",
                "inspect",
                "--format",
                "{\"id\":{{json .Id}},\"image\":{{json .Image}},\"env\":{{json .Config.Env}}}",
                container_id,
            ]
            .map(String::from)
            .to_vec(),
        ))
    }
    #[cfg(all(test, target_os = "linux"))]
    pub(super) fn credential_read(
        guard: &super::super::BoundTargetGuard<std::fs::File, Option<std::fs::File>>,
    ) -> Result<Self, ImportFailure> {
        guard
            .ensure_child_usable()
            .map_err(|_| ImportFailure::Identity)?;
        Self::credential_read_id(&guard.claim.container_id)
    }
    #[cfg(all(test, target_os = "linux"))]
    pub(super) fn credential_describe(
        guard: &super::super::BoundTargetGuard<std::fs::File, Option<std::fs::File>>,
    ) -> Result<Self, ImportFailure> {
        guard
            .ensure_child_usable()
            .map_err(|_| ImportFailure::Identity)?;
        Self::credential_describe_id(&guard.claim.container_id)
    }
    #[cfg(target_os = "linux")]
    pub(super) fn producer(container_id: &str, database: &str) -> Result<Self, ImportFailure> {
        Self::writer(container_id, database)?;
        let mut args = Self::prefix(container_id)?;
        args.extend(
            [
                "/usr/lib/postgresql/18/bin/pg_dump",
                "--format=custom",
                "--no-owner",
                "--no-acl",
                "--table=public.c4_import_probe",
                "--no-password",
                "--host=/var/run/postgresql",
                "--port=5432",
                "--username=learning_admin",
            ]
            .map(String::from),
        );
        args.push(format!("--dbname={database}"));
        Ok(Self(args))
    }
    #[cfg(target_os = "linux")]
    pub(super) fn toc(container_id: &str) -> Result<Self, ImportFailure> {
        let mut args = Self::prefix(container_id)?;
        args.extend(["/usr/lib/postgresql/18/bin/pg_restore", "--list"].map(String::from));
        Ok(Self(args))
    }
    pub(super) fn decoder(container_id: &str) -> Result<Self, ImportFailure> {
        let mut args = Self::prefix(container_id)?;
        args.extend(
            [
                "/usr/lib/postgresql/18/bin/pg_restore",
                "--file=-",
                "--no-owner",
                "--no-acl",
                "--exit-on-error",
            ]
            .map(String::from),
        );
        Ok(Self(args))
    }
    pub(super) fn writer(container_id: &str, database: &str) -> Result<Self, ImportFailure> {
        let suffix = database
            .strip_prefix("learning_restore_c4_")
            .ok_or(ImportFailure::Identity)?;
        let parsed = uuid::Uuid::parse_str(suffix).map_err(|_| ImportFailure::Identity)?;
        if parsed.get_version_num() != 4
            || parsed.get_variant() != uuid::Variant::RFC4122
            || parsed.to_string() != suffix
        {
            return Err(ImportFailure::Identity);
        }
        let mut args = Self::prefix(container_id)?;
        args.extend(
            [
                "/usr/lib/postgresql/18/bin/psql",
                "-X",
                "-qAt",
                "-P",
                "pager=off",
                "--no-password",
                "--host=/var/run/postgresql",
                "--port=5432",
                "--username=learning_admin",
            ]
            .map(String::from),
        );
        args.push(format!("--dbname={database}"));
        args.extend(["-v", "ON_ERROR_STOP=1", "-f", "-"].map(String::from));
        Ok(Self(args))
    }
    fn prefix(container_id: &str) -> Result<Vec<String>, ImportFailure> {
        if !super::super::exact_id(container_id) {
            return Err(ImportFailure::Identity);
        }
        Ok([
            "/usr/bin/docker",
            "exec",
            "--interactive",
            "--user",
            "999:999",
            container_id,
            "/usr/bin/env",
            "-i",
            "LC_ALL=C",
            "PGCONNECT_TIMEOUT=10",
            "PGPASSFILE=/dev/null/knowweave-c4-passfile-disabled",
        ]
        .map(String::from)
        .to_vec())
    }
    pub(super) fn argv(&self) -> &[String] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DB: &str = "learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3";

    #[test]
    fn credential_commands_are_fixed_noninteractive_owner_read_and_exact_description() {
        let read = FixedImportCommand::credential_read_id(ID).unwrap();
        let prefix = [
            "/usr/bin/docker",
            "exec",
            "--user",
            "999:999",
            ID,
            "/usr/bin/env",
            "-i",
            "LC_ALL=C",
            "PATH=/usr/bin:/bin",
            "/usr/bin/perl",
            "-T",
            "-e",
        ];
        assert_eq!(&read.argv()[..prefix.len().min(read.argv().len())], prefix);
        assert_eq!(read.argv().len(), 13);
        assert!(read.argv()[12].contains("/run/secrets/admin_password"));
        let description = FixedImportCommand::credential_describe_id(ID).unwrap();
        assert_eq!(
            description.argv(),
            [
                "/usr/bin/docker",
                "container",
                "inspect",
                "--format",
                "{\"id\":{{json .Id}},\"image\":{{json .Image}},\"env\":{{json .Config.Env}}}",
                ID
            ]
        );
    }

    #[test]
    fn commands_reject_non_rfc_uuid_variant_with_v4_nibble() {
        for db in [
            "learning_restore_c4_2b8a1252-54d5-48aa-0176-a9586a86bea3",
            "learning_restore_c4_2b8a1252-54d5-48aa-c176-a9586a86bea3",
            "learning_restore_c4_2b8a1252-54d5-48aa-e176-a9586a86bea3",
        ] {
            assert_eq!(
                FixedImportCommand::writer(ID, db).unwrap_err(),
                ImportFailure::Identity
            );
        }
        assert!(FixedImportCommand::writer(ID, DB).is_ok());
    }

    #[test]
    fn commands_reject_alias_and_conninfo() {
        for id in [
            "pg",
            "aaaaaaaaaaaa",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "",
        ] {
            assert_eq!(
                FixedImportCommand::decoder(id).unwrap_err(),
                ImportFailure::Identity
            );
            assert_eq!(
                FixedImportCommand::writer(id, DB).unwrap_err(),
                ImportFailure::Identity
            );
        }
        for db in [
            "postgres",
            "postgresql:///postgres",
            "host=remote dbname=x",
            "learning_restore_c4_2b8a1252-54d5-18aa-b176-a9586a86bea3",
            "learning_restore_c4_2B8A1252-54d5-48aa-b176-a9586a86bea3",
        ] {
            assert_eq!(
                FixedImportCommand::writer(ID, db).unwrap_err(),
                ImportFailure::Identity
            );
        }
    }

    #[test]
    fn commands_fix_environment_and_clients() {
        let prefix = [
            "/usr/bin/docker",
            "exec",
            "--interactive",
            "--user",
            "999:999",
            ID,
            "/usr/bin/env",
            "-i",
            "LC_ALL=C",
            "PGCONNECT_TIMEOUT=10",
            "PGPASSFILE=/dev/null/knowweave-c4-passfile-disabled",
        ];
        let decoder = FixedImportCommand::decoder(ID).unwrap();
        let mut want = prefix.to_vec();
        want.extend([
            "/usr/lib/postgresql/18/bin/pg_restore",
            "--file=-",
            "--no-owner",
            "--no-acl",
            "--exit-on-error",
        ]);
        assert_eq!(decoder.argv(), want);
        let writer = FixedImportCommand::writer(ID, DB).unwrap();
        let mut want = prefix.to_vec();
        want.extend([
            "/usr/lib/postgresql/18/bin/psql",
            "-X",
            "-qAt",
            "-P",
            "pager=off",
            "--no-password",
            "--host=/var/run/postgresql",
            "--port=5432",
            "--username=learning_admin",
            "--dbname=learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3",
            "-v",
            "ON_ERROR_STOP=1",
            "-f",
            "-",
        ]);
        assert_eq!(writer.argv(), want);
        assert!(
            !writer
                .argv()
                .iter()
                .any(|arg| arg == "-1" || arg == "--single-transaction")
        );
        assert!(
            !decoder
                .argv()
                .iter()
                .any(|arg| arg.starts_with("--dbname") || arg == "-1")
        );
    }
}
