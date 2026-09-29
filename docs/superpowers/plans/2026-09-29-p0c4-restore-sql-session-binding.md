# P0-C4 Restore SQL Session Binding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove that every SQLx clean-target preflight query uses one physical PostgreSQL session visible through the already pinned Docker container ID, without enabling database restore.

**Architecture:** Keep the existing global and target locks and exact Docker observation. Acquire one SQLx transaction from the supplied pool, take two unpredictable transaction-scoped advisory locks, and inspect those locks through `docker exec` against the exact container ID and its local Unix socket. Carry that transaction with the internal preflight/continuation so query routing cannot silently move to another connection. The pool address and cloned PostgreSQL system identifier alone are never proof of endpoint identity.

**Tech Stack:** Rust 1.97, SQLx 0.8.6, PostgreSQL 18, existing `learning-backup` Docker binding and isolated Compose harness.

**Spec:** [P0-C4 backup and recovery](2026-09-28-p0c4-backup-recovery.md), [bound-target guard](../../p0c4-bound-target-guard.md), [verification boundary](../../p0c4-verification.md).

## Global constraints

- This plan delivers only an internal, read-only endpoint proof. Do not call or expose `restore_database`, execute `pg_restore`, create a restore-attempt marker, admit the service, or treat the result as C4 completion.
- Preserve the existing exact Docker daemon/container/network/image/volume/mount checks, global-then-target lock order, and preflight rejection rules. Keep the old probe and guard acceptance modes intact.
- Fail closed if the SQLx connection changes, either lock is absent, both locks do not have the same backend PID and expected database OID, the target restarts, or Docker observation differs. A system identifier match is insufficient.
- Use a transaction-scoped lock so dropping the transaction releases both locks; never return a connection to the pool while it holds session-level advisory locks. Hold the transaction in the internal preflight state through its continuation. Do not put the random lock values or credentials in durable evidence or logs.
- Use the current pinned Docker executable and exact container ID for the independent `pg_locks` observation. Keep query text fixed except for validated numeric lock keys; no shell, caller-supplied SQL, or mutable container name.
- Windows unit tests are not Linux/PG18 acceptance. A live gate requires a fresh, separately authorized ZIP/runner, project, subnet and PostgreSQL volume; failed or stopped batches are never reused.

## Task 1 — internal lock challenge and strict observation

Files: `crates/learning-backup/src/restore_preflight/target_binding.rs` and focused tests in that module.

- [x] Write failing dependency-injected tests for two matching transaction-lock rows on one PID/database, missing/duplicate/foreign rows, changed Docker observation, and challenge release/failure ordering.
- [x] Run the focused tests and record the expected RED.
- [x] Add the smallest internal interface that verifies two unpredictable `i64` lock keys against a fixed `pg_locks` query through the exact container ID. Require the expected backend PID and database OID, granted exclusive transaction locks, exact output shape, and Docker identity checks before/after.
- [x] Run focused tests, fmt and strict Clippy; commit this isolated primitive. It grants no write authority.
- [x] Obtain independent task review and address findings before Task 2.

## Task 2 — one SQLx transaction for the entire preflight

Files: `crates/learning-backup/src/restore_preflight.rs`, plus Task 1's internal interface if necessary.

- [x] Write failing tests for connection/transaction lifetime, fail-closed mismatched challenge, and use of the same physical connection by all preflight queries. Keep PostgreSQL integration tests ignored until a fresh Linux batch is authorized.
- [x] Run the focused tests and record RED.
- [x] Start one `Transaction<'static, Postgres>` from the pool under the existing bound guard; acquire two transaction advisory locks and read its backend PID/database OID. Verify through Task 1's exact-container observer before any clean-target SQL and once again after all queries. Change `observed_build_and_pg`, `target_facts`, `observed_public_schema` and `verify_target_birth` to use that transaction, not `&PgPool` or a newly acquired connection. Keep the transaction in `RestorePreflight` and transfer it to `RestoreDatabaseImported`; dropping either state releases it.
- [x] Run focused tests, Python runner regressions, fmt, strict Clippy and available non-PG workspace checks. Do not claim a full workspace pass if the dedicated PG test environment is absent. Commit and obtain independent review.

## Task 3 — fresh Linux/PG18 read-only acceptance

Files: opt-in ignored Linux test and acceptance runner, then `docs/p0c4-verification.md` after evidence.

- [x] Add a distinct `--sql-session-binding` acceptance mode for a new isolated PG18 target. Positive evidence shows one SQLx backend PID, two matching live locks and the exact container observer; negative cases cover a dropped connection/transaction, released lock, changed Docker start facts and wrong database. The same-system-identifier clone and wrong-endpoint negative were completed in the separate [second-endpoint plan](2026-09-29-p0c4-second-endpoint-negative.md). Assert no attempt marker, dump import, asset write or user catalog object.
- [x] Observe RED then GREEN locally where possible; independently review the source and runner. Package only tracked Git bytes, validate ZIP/manifest/runner hashes, and obtain the user's separate authorization for those exact files before server upload.
- [x] In fresh user-authorized server batches, verify result and evidence hashes, no pending result, exact-ID stop and retained quarantined volumes. Record the scope as `READ_ONLY / NOT_RESTORE`; retain all failed evidence and never replay a batch.

Task 3a passed on the single target `5c2b0043-eef0-4925-8853-cda7088b77c8` (`result.json` SHA-256 `e5d8cd32404a105d47b7bddd112832a6e79f8a3c54218f237701ca940ac0ed07`). The separate Task 3b live physical-clone gate passed on primary `e5ff5e73-80bf-4887-a723-63afcd7eb410` and clone `b7f1f331-dd84-4dd0-975b-f649ce664f62` (`result.json` SHA-256 `647a669d376ab1294e132ee8f97737b98c69b2fc6e8aee10b65b4c8ad573e54f`). Both are read-only, single-host, quarantined test evidence; see [C4 verification](../../p0c4-verification.md) for exact limits.

## Later boundary, outside this plan

Actual import still needs a separate proof that the `pg_restore` child runs in the exact immutable container against its local socket, with pinned executable and credential handles, before its first write. The current host `PgRestoreSpec` and caller path arguments remain unwired. CompleteBackup issuance, asset closure, lease invalidation and service admission remain separate gates.
