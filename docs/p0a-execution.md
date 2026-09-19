# SDD ledger — plan: D:/codex/DeepLearning/docs/superpowers/plans/2026-09-17-p0a-block-revision-core.md

2026-09-19: User explicitly requested “开始实施落地”. Scope P0-A; no API/frontend/production deployment.
Baseline: 239a3ba; retained trial scaffold, core 5/5 tests passed; DB methods are placeholders and not accepted.

Ruling: Work in the existing dedicated learning-system repository on feat/rust-learning-core, at the directory specified by the plan — preserves the uncommitted trial as baseline 239a3ba without mixing course assets — cost if wrong: relocate the isolated application checkout.
Ruling: Skill Bash helper lacks basename in this Windows installation; keep equivalent plan-scoped ledger and read task sections directly — avoids environment setup unrelated to product — cost if wrong: manual progress bookkeeping must remain complete.

Pre-flight shared interfaces:
- Tasks 1→2/3/4: Principal.actor_id, strict TextDraft, audit fields, versioned digest; trial uses Principal.id and unversioned digest. Align with plan.
- Tasks 2→3: runtime can only read grants; restricted lock_space_grant function supplies row locking without granting permission edits.
- Tasks 3→4/5: reads include audit fields and cursors; write tests first assert using admin SQL, not incomplete read methods.
- Tasks 2/3/4/5→6: real PG tests with distinct admin/runtime logins; source snapshots pinned and transferred by existing server task.

Task 1: in progress, BASE 239a3ba. Expand existing five tests before changing behavior.
Task 1: RED 5 passed / 4 failed (.runtime/task1-red.log); GREEN 9/9 (.runtime/task1-green.log). Independent Python digest equals reviewed golden value.
Ruling: Reject U+0000 in text and reason at the contract boundary — PostgreSQL text/JSONB cannot preserve it — cost if wrong: a future binary text format needs a separate encoding contract.
Build issue: sandbox Schannel SEC_E_NO_CREDENTIALS; same Cargo command outside sandbox downloaded chrono and passed. No TLS validation was disabled.
Task 2: RED independently read server cargo-test.log: 2 passed / 2 behavioral failures (nullable head, missing restricted lock). Source ZIP 7d9f204dadf15125016ae22845e27b4414edea2203dc0fe5f01af9b7fd70321b verified before/after server execution. Migration now fixes both.
Tasks 3/4/5: prepared behavior tests while awaiting schema execution; all targets compile locally. Save and read methods remain placeholders for business RED.
Task 1: complete — core contracts 9/9 and core Clippy passed with Rust 1.97.0; fmt applied. Task 2: in progress — schema tests written against unchanged trial migration.
Task 2: complete — server schema 4/4, ACL audit confirms NOLOGIN owner, no runtime role membership, no residual schema CREATE. Tasks 3/4/5 RED: revisions 4, authorization 5, concurrency 5, atomicity 2 all behavioral failures, each exit101; original logs read. Implementation now compiles all targets; replaced SQLx raw_sql transaction setup with individual query statements after spawn Send-bound compiler reproduction. GREEN awaiting real PostgreSQL.
Task 3: complete — revisions 4/4 GREEN, whole workspace29/29 on real PostgreSQL18.6/Rust1.97.0; tests.log and result.json independently read.
Task 4: complete — authorization5/5 GREEN in same run, current grants/filtering/pagination/replay verified.
Task 5: complete — concurrency5/5 and atomicity2/2 GREEN in same run; distinct connections, observed lock waits and scoped real PG trigger failures.
Task 6: in progress — host fmt/Clippy/workspace all exit0; source31/31 unchanged. Compose config0, initial image authentication DNS failure before build, server continuing official image download via isolated proxy path. Independent read-only reviewer active.
Final review: fresh gpt-6-astra reviewer completed read-only assessment. No Critical/Important/Minor findings. Independently checked fixed ZIP hash and complete host GREEN logs.
Final: Ruling: HTTP/UI/transport-loss injection and composition/relation/asset/publish/recovery behavior remain later-phase requirements — P0-A is an internal content library — cost if wrong: later integration must supply explicit tests before release.
Final: Ruling: compromised runtime credentials are outside user authorization guarantee; runtime remains trusted and not exposed — no RLS claim — cost if wrong: a future user-level DB boundary needs RLS or separate credentials.
Final: Ruling: exchange1.0/future version migration/production migration from trial are deferred — no accepted production schema exists — cost if wrong: future format evolution requires explicit migrations and compatibility tests.
Final: Ruling: Compose remains required despite clean static review and host tests — server is resolving image-download environment — cost if wrong: no deployment reproducibility claim until actual run.
Task 6: complete — host29/29 and two independent empty-volume Compose runs29/29. Final pinned ZIP f46fb20a2167628e745122bdde9c795f18da3c01f4fb731130a1b43d1d3902d5; all commands exit0;32 source files unchanged. Root independently audited20 log hashes, official index/platform manifests, runtime limits and stopped test containers. Initial DNS and import-format failures preserved. No TLS relaxation or shared-daemon change.
Final review: no code fixes required, no deferred minors. Snapshot deployment refs subsequently pinned to the exact tested official manifests and rerun from an empty database.
Final integration: preserve local feat/rust-learning-core; this fresh repository has no separate base branch or configured publish target. No merge/push/production release requested.
Final record: implementation commit26108fd; final deployment/evidence documentation committed separately. This ledger is archived as docs/p0a-execution.md before removing its own scratch directory.
