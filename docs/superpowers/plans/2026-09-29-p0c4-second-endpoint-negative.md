# P0-C4 Second SQL Endpoint Negative Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In a wholly new isolated batch, prove that an SQLx transaction on another connectable PostgreSQL 18 instance cannot pass the original exact-container lock challenge, even if that instance has the same PostgreSQL system identifier and database OID.

**Architecture:** Issue one new primary C4 target with the existing birth/pin path. Keep it running and use an online physical base backup into a second new volume, then start the copied cluster under a different project/network/container ID. The test acquires two transaction locks on the copy; the original exact-ID observer must reject those locks while its Docker identity remains unchanged. The alternate instance has no birth or restore authority.

**Tech Stack:** Rust 1.97, SQLx 0.8.6, PostgreSQL 18 `pg_basebackup`/`pg_verifybackup`, pinned Docker image, Python root-only acceptance runner.

**Spec:** [P0-C4 backup and recovery](2026-09-28-p0c4-backup-recovery.md), [SQL session binding](2026-09-29-p0c4-restore-sql-session-binding.md), [focused 3a evidence](../../p0c4-verification.md).

## Global constraints

- Use two never-used UUIDv4 project identities, two new PG volumes and nonoverlapping unused RFC1918 subnets. No existing container, volume, network, target, evidence batch, or production service can serve as the negative endpoint. Never restart the primary after birth; its `StartedAt` is part of the bound identity.
- Before creating resources, pin ZIP/manifest/runner bytes and the offline builder; confirm both projects, volumes, target paths and subnets are unused. The old Task 3a batch is evidence and cannot be replayed or used as the source.
- Physical clone requires an explicitly verified replication permission and host authentication contract. Do not modify `pg_hba.conf`, weaken roles, print secrets, pass password values in CLI arguments/environment, or persist them in evidence. A root-private, short-lived passfile mount may be used only in this new batch; its path, permissions and cleanup must be reviewed. No application data, restore attempt, dump import, asset write, `CompleteBackup` or service admission.
- Stop both PG containers by their exact immutable IDs even on failure; confirm stop and retain both volumes/evidence quarantined. No `down -v`, prune, wildcard cleanup or reuse. Keep one failed batch as failed; any fix needs a new package and new batch.
- If online physical clone cannot satisfy the pinned image's replication/authentication contract, fail closed. A separately initialized second PG can be a later, separately labeled weaker endpoint test; it must not be reported as the same-identifier clone gate.

## Task 1 — new-batch clone preparation and evidence contract

Files: `scripts/p0c4_restore_pin_acceptance.py`, its focused tests, and a small Task 3b acceptance document.

- [ ] Write runner RED tests for exact two-project resource admission, failed replication preflight, secret/passfile non-disclosure, clone volume mount restrictions, and exact-ID dual stop/retained-volume failure evidence.
- [ ] Run the focused tests and record RED, then implement a distinct opt-in `--sql-session-clone-negative` mode. Reuse the 3a primary birth and pinned offline builder. Create the second project without starting its PG; run a pinned-image temporary helper in the primary's exact network namespace to `pg_basebackup -F plain -X stream` into the new volume, then `pg_verifybackup` and assert no standby configuration before starting the copy. Preserve the primary Docker observation throughout.
- [ ] Use only a new-batch `postgres` secret via a root-private 0600 passfile file mount; no raw secret in argument strings, Docker inspect environment, output or evidence. Inspect and strictly admit the pinned image, exact volume/network/container IDs, mount topology and second PG18 readiness.
- [ ] Run runner tests and syntax checks; commit. Obtain independent review before Task 2.

## Task 2 — live same-identifier wrong-endpoint assertion

Files: `crates/learning-backup/src/restore_preflight/target_binding.rs`, runner test selection/output gate and focused tests.

- [ ] Write RED tests for matching system identifier/database OID but distinct exact container ID/IP; two locks visible on the copy but absent on the primary; incorrect acceptance or primary identity drift must fail.
- [ ] Add one ignored Linux test that connects SQLx to the copy, proves the copy and primary share PG system identifier and database OID/name, acquires two transaction locks, and confirms the original `BoundTargetGuard::verify_sql_session` rejects that live wrong endpoint. After rollback, verify both locks disappear, the primary remains bound and empty, and neither target has an attempt marker or application data.
- [ ] Give this test a distinct one-test/one-marker result gate and success state `SAME_ID_WRONG_ENDPOINT_REJECTED_READ_ONLY_NOT_RESTORE`; no full C4 or restore success wording. Run local tests, fmt and strict Clippy; commit and obtain independent review.

## Task 3 — separately authorized isolated Linux batch

- [ ] Rebuild a tracked-byte ZIP and exact ZIP/Git-byte runner, verify every member and manifest locally, and request the user's separate approval for these exact files and a new two-project/two-subnet batch before upload.
- [ ] On the server, recheck unused resources, hashes, pinned image and root-only secret handling. Run only the newly authorized batch. Validate raw result and evidence SHA-256, exact two PG IDs stopped, both volumes retained and pending absent. Record any partial/failed stage without replaying the batch.
- [ ] Update [C4 verification](../../p0c4-verification.md) with the precise result and limits. This gate still does not prove the `pg_restore` child endpoint, credentials, asset closure, lease invalidation or service admission.
