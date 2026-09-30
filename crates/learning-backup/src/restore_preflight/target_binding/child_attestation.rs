//! Read-only, scoped child proof. Never restore authority.
use super::*;

mod bounded_process;

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::restore_preflight) enum ChildFailure {
    Session,
    Identity,
    Protocol,
    Version,
    Deadline,
    StdoutLimit,
    StderrLimit,
    Exit,
    Stderr,
    Io,
    Unusable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::restore_preflight) enum Isolation {
    Stopped,
    UnconfirmedUnusable,
}
impl Isolation {
    pub(in crate::restore_preflight) fn status(self) -> &'static str {
        match self {
            Self::Stopped => "STOPPED",
            Self::UnconfirmedUnusable => "UNCONFIRMED_UNUSABLE",
        }
    }
}
#[derive(Debug)]
pub(in crate::restore_preflight) struct ChildError {
    pub reason: ChildFailure,
    pub isolation: Isolation,
}

pub(in crate::restore_preflight) struct ChildReadOnlyAttestation<'a, G, T, L> {
    _guard: &'a BoundTargetGuard<G, T>,
    _challenge: &'a LockChallenge<L>,
}
impl<G, T, L> ChildReadOnlyAttestation<'_, G, T, L> {
    pub(in crate::restore_preflight) fn status(&self) -> &'static str {
        "CHILD_READ_ONLY_ATTESTED_NOT_RESTORE"
    }
}
trait ChildIo<L> {
    async fn session(
        &mut self,
        lease: &mut L,
        deadline: Instant,
    ) -> Result<(i32, u64), ChildFailure>;
    async fn observe(
        &mut self,
        claim: &DockerClaim,
        deadline: Instant,
    ) -> Result<Value, ChildFailure>;
    async fn run(&mut self, args: &[String], deadline: Instant) -> Result<String, ChildFailure>;
    async fn quarantine(&mut self, claim: &DockerClaim) -> Isolation;
}
pub(super) fn socket_sql(keys: ChallengeKeys) -> String {
    let (a, b) = lock_halves(keys.0[0]);
    let (c, d) = lock_halves(keys.0[1]);
    format!(
        "BEGIN READ ONLY; SET LOCAL statement_timeout = '5000ms'; \
        SELECT current_user::text || '|' || pg_catalog.current_database() || '|' || d.oid::text || '|' || pcs.system_identifier::text \
        FROM pg_catalog.pg_database d CROSS JOIN pg_catalog.pg_control_system() pcs WHERE d.datname=pg_catalog.current_database(); \
        SELECT pid, database::bigint, classid::bigint, objid::bigint, objsubid, mode, granted FROM pg_catalog.pg_locks \
        WHERE locktype='advisory' AND (classid::bigint,objid::bigint) IN (({a},{b}),({c},{d})); ROLLBACK;"
    )
}
fn socket_rows<'a>(claim: &DockerClaim, output: &'a str) -> Result<&'a str, ChildFailure> {
    let (identity, rows) = output.split_once('\n').ok_or(ChildFailure::Protocol)?;
    if identity
        != format!(
            "learning_admin|{}|{}|{}",
            claim.database, claim.database_oid, claim.system_identifier
        )
    {
        return Err(ChildFailure::Identity);
    }
    Ok(rows)
}
fn version(output: &str) -> Result<(), ChildFailure> {
    // The birth image is pinned to this exact PG18 build. Do not accept extra
    // lines, arbitrary suffixes, or a different release from an injected path.
    if output != "pg_restore (PostgreSQL) 18.6 (Debian 18.6-1.pgdg12+1)\n" {
        return Err(ChildFailure::Version);
    }
    Ok(())
}
async fn checked_observation<L>(
    guard: &BoundTargetGuard<impl Sized, impl Sized>,
    io: &mut impl ChildIo<L>,
    deadline: Instant,
) -> Result<Value, ChildFailure> {
    let value = io.observe(&guard.claim, deadline).await?;
    same_observation(&guard.observation, &value).map_err(|_| ChildFailure::Identity)?;
    Ok(value)
}
async fn session_proof<G, T, L>(
    guard: &BoundTargetGuard<G, T>,
    challenge: &mut LockChallenge<L>,
    io: &mut impl ChildIo<L>,
    args: &[String],
    deadline: Instant,
) -> Result<(i32, u64), ChildFailure> {
    let (pid, oid) = io.session(&mut challenge.lease, deadline).await?;
    if pid <= 0 || oid != guard.claim.database_oid {
        return Err(ChildFailure::Session);
    }
    let before = checked_observation(guard, io, deadline).await?;
    let output = io.run(args, deadline).await?;
    let rows = socket_rows(&guard.claim, &output)?;
    let after = checked_observation(guard, io, deadline).await?;
    // The same strict validator as verify_sql_session, fed bounded observations.
    guard
        .verify_sql_session_observed(challenge.keys, pid, oid, &before, rows, &after)
        .map_err(|_| ChildFailure::Session)?;
    Ok((pid, oid))
}
async fn attest_with<'a, G, T, L>(
    guard: &'a BoundTargetGuard<G, T>,
    challenge: &'a mut LockChallenge<L>,
    io: &mut impl ChildIo<L>,
) -> Result<ChildReadOnlyAttestation<'a, G, T, L>, ChildError> {
    let target = ExactRestoreChildTarget::bind(guard, challenge).map_err(|_| ChildError {
        reason: ChildFailure::Unusable,
        isolation: Isolation::UnconfirmedUnusable,
    })?;
    let socket = target.socket_probe_argv();
    let version_args = target.version_argv();
    // Sticky across cancellation and all errors. Only the complete proof can
    // re-enable this guard; no second call may rehabilitate a failed batch.
    guard.child_usable.set(false);
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = async {
        let session = session_proof(guard, challenge, io, &socket, deadline).await?;
        checked_observation(guard, io, deadline).await?;
        version(&io.run(&version_args, deadline).await?)?;
        let output = io.run(&socket, deadline).await?;
        validate_challenge_rows(
            challenge.keys,
            session.0,
            session.1,
            socket_rows(&guard.claim, &output)?,
        )
        .map_err(|_| ChildFailure::Session)?;
        checked_observation(guard, io, deadline).await?;
        if session_proof(guard, challenge, io, &socket, deadline).await? != session {
            return Err(ChildFailure::Session);
        }
        Ok(())
    }
    .await;
    if let Err(reason) = result {
        return Err(ChildError {
            reason,
            isolation: io.quarantine(&guard.claim).await,
        });
    }
    guard.child_usable.set(true);
    Ok(ChildReadOnlyAttestation {
        _guard: guard,
        _challenge: challenge,
    })
}

#[cfg(any(target_os = "linux", test))]
mod linux_child;
#[cfg(target_os = "linux")]
pub(in crate::restore_preflight) use linux_child::attest;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::VecDeque;

    const ROWS: &str = "42|16385|0|7|1|ExclusiveLock|t\n42|16385|0|9|1|ExclusiveLock|t\n";
    fn socket() -> String {
        format!(
            "learning_admin|{}|16385|{}\n{ROWS}",
            super::super::tests::claim().database,
            super::super::tests::claim().system_identifier
        )
    }
    struct Fake {
        outputs: VecDeque<Result<String, ChildFailure>>,
        observations: usize,
        drift_at: Option<usize>,
        sessions: usize,
        dead_session: Option<usize>,
        block_session: bool,
        changed_pid: bool,
        quarantined: Vec<String>,
        isolation: Isolation,
    }
    impl Fake {
        fn good() -> Self {
            Self {
                outputs: [
                    Ok(socket()),
                    Ok("pg_restore (PostgreSQL) 18.6 (Debian 18.6-1.pgdg12+1)\n".into()),
                    Ok(socket()),
                    Ok(socket()),
                ]
                .into(),
                observations: 0,
                drift_at: None,
                sessions: 0,
                dead_session: None,
                block_session: false,
                changed_pid: false,
                quarantined: vec![],
                isolation: Isolation::Stopped,
            }
        }
    }
    impl ChildIo<()> for Fake {
        async fn session(
            &mut self,
            _: &mut (),
            _: std::time::Instant,
        ) -> Result<(i32, u64), ChildFailure> {
            self.sessions += 1;
            if self.block_session {
                std::future::pending::<()>().await;
            }
            if self.changed_pid && self.sessions == 2 {
                return Ok((43, 16385));
            }
            if self.dead_session == Some(self.sessions) {
                Err(ChildFailure::Session)
            } else {
                Ok((42, 16385))
            }
        }
        async fn observe(
            &mut self,
            _: &DockerClaim,
            _: std::time::Instant,
        ) -> Result<Value, ChildFailure> {
            self.observations += 1;
            Ok(
                json!({"StartedAt": if self.drift_at == Some(self.observations) { "changed" } else { "same" }}),
            )
        }
        async fn run(
            &mut self,
            args: &[String],
            _: std::time::Instant,
        ) -> Result<String, ChildFailure> {
            assert_eq!(&args[4], &super::super::tests::claim().container_id);
            assert!(!args.iter().any(|a| a.contains("restore.attempt")));
            self.outputs.pop_front().unwrap()
        }
        async fn quarantine(&mut self, claim: &DockerClaim) -> Isolation {
            self.quarantined.push(claim.container_id.clone());
            self.isolation
        }
    }
    fn guard() -> BoundTargetGuard<(), ()> {
        acquire_bound_guard(
            || Ok(()),
            || Ok(()),
            || Ok((super::super::tests::claim(), json!({"StartedAt":"same"}))),
        )
        .unwrap()
    }
    #[tokio::test]
    async fn child_proof_requires_before_and_after_live_session_and_scope() {
        let guard = guard();
        let mut lease = LockChallenge::new(ChallengeKeys::for_test(7, 9), ());
        let mut io = Fake::good();
        let proof = attest_with(&guard, &mut lease, &mut io).await.unwrap();
        assert_eq!(proof.status(), "CHILD_READ_ONLY_ATTESTED_NOT_RESTORE");
        assert_eq!(io.sessions, 2);
        assert_eq!(io.observations, 6);
        assert!(io.outputs.is_empty());
        assert!(io.quarantined.is_empty());
    }
    #[tokio::test]
    async fn child_proof_rejects_versions_roles_databases_clone_rows_and_stderr_errors() {
        for (index, bad) in [
            (1, "pg_restore (PostgreSQL) 17.6\n".to_owned()),
            (1, "pg_restore (PostgreSQL) 18.6\nextra\n".to_owned()),
            (2, socket().replace("learning_admin", "postgres")),
            (2, socket().replace("learning_restore_c4_", "wrong_")),
            (2, socket().replace(ROWS, "")),
            (2, socket().replace("42|", "43|")),
            (2, socket().replace("16385|0|9", "16386|0|9")),
            (2, format!("{}password: secret\n", socket())),
            (3, socket().replace(ROWS, "")),
        ] {
            let guard = guard();
            let mut lease = LockChallenge::new(ChallengeKeys::for_test(7, 9), ());
            let mut io = Fake::good();
            io.outputs[index] = Ok(bad);
            let error = attest_with(&guard, &mut lease, &mut io)
                .await
                .err()
                .unwrap();
            assert!(!format!("{error:?}").contains("secret"));
            assert!(ExactRestoreChildTarget::bind(&guard, &lease).is_err());
        }
    }
    #[tokio::test]
    async fn child_proof_rejects_docker_or_postmaster_drift() {
        for step in 1..=6 {
            let guard = guard();
            let mut lease = LockChallenge::new(ChallengeKeys::for_test(7, 9), ());
            let mut io = Fake::good();
            io.drift_at = Some(step);
            assert!(attest_with(&guard, &mut lease, &mut io).await.is_err());
        }
        for step in 1..=2 {
            let guard = guard();
            let mut lease = LockChallenge::new(ChallengeKeys::for_test(7, 9), ());
            let mut io = Fake::good();
            io.dead_session = Some(step);
            assert!(attest_with(&guard, &mut lease, &mut io).await.is_err());
        }
    }
    #[tokio::test]
    async fn child_proof_timeout_overflow_and_exit_fail_closed_with_exact_quarantine() {
        for failure in [
            ChildFailure::Deadline,
            ChildFailure::StdoutLimit,
            ChildFailure::StderrLimit,
            ChildFailure::Exit,
            ChildFailure::Stderr,
        ] {
            for isolation in [Isolation::Stopped, Isolation::UnconfirmedUnusable] {
                let guard = guard();
                let mut lease = LockChallenge::new(ChallengeKeys::for_test(7, 9), ());
                let mut io = Fake::good();
                io.outputs[2] = Err(failure);
                io.isolation = isolation;
                let error = attest_with(&guard, &mut lease, &mut io)
                    .await
                    .err()
                    .unwrap();
                assert_eq!(error.reason, failure);
                assert_eq!(error.isolation, isolation);
                assert_eq!(
                    io.quarantined,
                    std::slice::from_ref(&guard.claim.container_id)
                );
                assert!(attest_with(&guard, &mut lease, &mut io).await.is_err());
            }
        }
    }
    #[tokio::test]
    async fn failed_child_cannot_reuse_older_guard_entrypoints() {
        let guard = guard();
        let mut lease = LockChallenge::new(ChallengeKeys::for_test(7, 9), ());
        let mut io = Fake::good();
        io.outputs[1] = Ok("pg_restore (PostgreSQL) 17.6\n".into());
        assert!(attest_with(&guard, &mut lease, &mut io).await.is_err());
        assert!(
            guard
                .recheck_with(
                    |_| Ok(json!({"StartedAt":"same"})),
                    |_| Ok("16385|7361082129910479001\n".into())
                )
                .is_err()
        );
    }
    #[tokio::test]
    async fn child_cancellation_invalidates_guard_and_live_pid_must_stay_identical() {
        let guard = guard();
        let mut lease = LockChallenge::new(ChallengeKeys::for_test(7, 9), ());
        let mut io = Fake::good();
        io.block_session = true;
        assert!(
            tokio::time::timeout(
                Duration::from_millis(5),
                attest_with(&guard, &mut lease, &mut io)
            )
            .await
            .is_err()
        );
        assert!(ExactRestoreChildTarget::bind(&guard, &lease).is_err());
        assert!(attest_with(&guard, &mut lease, &mut io).await.is_err());
        let guard = self::guard();
        let mut io = Fake::good();
        io.changed_pid = true;
        io.outputs[3] = Ok(socket().replace("42|", "43|"));
        assert!(attest_with(&guard, &mut lease, &mut io).await.is_err());
        assert!(!guard.child_usable.get());
    }
    #[test]
    fn child_sql_enforces_readonly_timeout_and_exact_lock_observation() {
        let guard = guard();
        let lease = LockChallenge::new(ChallengeKeys::for_test(7, 9), ());
        let target = ExactRestoreChildTarget::bind(&guard, &lease).unwrap();
        let args = target.socket_probe_argv();
        let sql = args.last().unwrap();
        assert!(args.iter().any(|a| a == "ON_ERROR_STOP=1"));
        assert!(sql.contains("BEGIN READ ONLY"));
        assert!(sql.contains("SET LOCAL statement_timeout = '5000ms'"));
        assert!(sql.contains("pg_catalog.pg_locks"));
        assert!(sql.contains("((0,7),(0,9))"));
        assert!(!sql.contains("pg_advisory_lock"));
    }
}
