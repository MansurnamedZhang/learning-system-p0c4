# P0-B1 执行记录与取舍

状态：`P0_B1_VERIFIED`。以下账本按执行时间保留，早期 pending / in progress 是历史状态；末尾记录最终结果。

# SDD ledger — plan: D:/codex/DeepLearning/docs/superpowers/plans/2026-09-19-p0b1-composition-release.md

Start: 2026-09-19; user continued after implementation plan; execute B1 only. BASE e799d6f.

Workspace: dedicated learning-system repo on feat/rust-learning-core, clean at start; no main branch/remote integration required.
Ruling: reuse the existing dedicated feature checkout, as for P0-A — approved plan names this directory and user chose continuation — cost: no parallel implementation checkout.
Ruling: keep the plan-specific ledger manually because the bundled Bash helper previously lacked basename on this Windows host — same identity/progress format preserved — cost: manual bookkeeping.

Pre-flight 1→2/3/4: new core types consumed by stores; JSON enum representation fixed, no conflict.
Pre-flight 2→3/4/5: all operations share existing advisory namespace and request_key registry; old mutation receipts retain FK and digest compatibility.
Pre-flight 3→4/5: exact closure, stable occurrence and current grants; all-or-none B1 projection; no latest traversal.
Pre-flight 2→6: upgrade test needs separate empty DB; ordinary TestRig must not migrate it first.
Pre-flight 4→6: outbox append only; no worker delivered; production UI remains later.

Task 1: in progress; tests/contracts and new strict contract tests.
Task 2: pending.
Task 3: pending.
Task 4: pending.
Task 5: pending.
Task 6: pending.

Task 1: complete; core RED 1 pass / 6 fail, GREEN 16/16 including P0-A 9. Independent Python golden ac50ebc23e7e32af602a65c879111120acc2137c85462b6658aed3b577a77939. Logs .runtime/p0b1-core-{red,green}.log. Task 2 in progress.
Task 2: schema GREEN 2/2 and P0-A PG regression 20/20; upgrade 1/1 GREEN in separate new DB. Task 3: 6/6 GREEN; 3 extra closure budget/rollback tests pending. Task 4: release RED 3/3 failures -> GREEN 3/3. Task 5: auth RED 3 failures -> GREEN 3/3; concurrency 4 pass/2 stub fail -> GREEN 6/6. Full host 57/57 GREEN with fmt/Clippy exit0, package89771a3. Server evidence pointer in docs/p0b1-verification.md.
Ruling: commit tasks 2–4 at one verified integration checkpoint because they share registry/schema/module exports; keep per-task test evidence — cost: larger review range instead of independently buildable stub commits.
Ruling: 8MiB uses PostgreSQL JSONB text serialization bytes, checked before fetching exceeding body — cost: differs from compact JSON/HTTP payload bytes, explicitly documented.
Ruling: references must be committed at lookup, not necessarily transaction-start time under READ COMMITTED — exact immutable refs/cycle validation suffice, no same-request pending refs — cost: no transaction-start snapshot guarantee on writes.
Task 6: added separate Compose upgrade database and runtime 0001-only directory; final isolated Compose pending.
Final review: reviewer p0b1_review inspected immutable9bde package57files and original57test logs. No Critical, one Important public publishing recovery gap; three groups labelled Minor acceptance assertions. One fix pass started.
Final: Ruling: treat the three missing assertion groups as Important verification gaps for claimed acceptance (existing-object FK pairing, cross-space revoke/lock order, complete structural rollback counts) rather than leave claimed gates unverified — cost: extra targeted PG tests, no new product scope.
Final: Ruling: add authorized PublicationState and root-scoped SHA256 publication token in place of unpublished expected_release_id API — enables fresh-caller/conflict recovery without disclosing multi-root release ID — cost: B1 planned command JSON changes before delivery; no P0-A or database migration change.
Final fix RED: publication_state3/3 Storage failures, packageaa1cf54e, server log730065fb; prerequisite comparisons/scaffold compile. Fix GREEN pending package267b88d2, core17pass and full-targetClippy0 locally. New tests include root-token scope, mixed-root visibility, repeat publication on same revision.
Final: Ruling: no production performance/online rolling binary migration claims; runtime credentials/admin compromise outside trusted service boundary; HTTP/UI/overlay/relations/assets/workers/search/export/backup remain their approved phases; 8MiB is block JSONB budget not transport cap — costs: these validations remain required before production release.
Review declined pending final runs and review-after-modification: final fixed behavior will be verified by failing regression tests then full suite, not a second reviewer, per execution skill.
Task 2: complete (integration commit6563a3d; schema3/3, originalPG20/20, upgrade1/1 in final host run).
Task 3: complete (commit6563a3d; composition10/10 including2048objects,8MiB exact boundaries and full structural rollback).
Task 4: complete (commit6563a3d; release3/3 plus publication_state3/3; atomic publish/rollback/refresh behavior).
Task 5: complete (commit6563a3d; assembly_authorization3/3, assembly_concurrency8/8; actual block waits observed).
Final: fixed Important publication-state recovery — publication_state3 RED Storage ->3 GREEN; full suite68/68 host, fmt0/Clippy0. Added required existing-pair/lock-order/structural-count assertions all GREEN. No deferred minor findings.
Final source host proof: package267b88d2,58files; workspace SHA3d01fd1da0427d8e311b95902de21069faa3e269efabe981da6a466947a7dc3d independently recomputed, source manifest matches code/deps/migrations/deploy. Only boundary/report/README docs changed afterwards. Compose still pending.

Task 6: complete. Final GREEN2 host and new Compose each 68/68, 0 failed/ignored; fmt/Clippy/test/config/build/up/stop exit0. Standard BuildKit build succeeded without proxy fallback or daemon changes. Independent evidence audit verified10batches/24logs, all58source hashes, exact test counts and final stopped state. Container limits PG2CPU4GiB/test4CPU4GiB, internal network/no ports; all B1 containers Exited(0), existing business services running. Original volumes and evidence retained.
Final: all tasks complete, P0_B1_VERIFIED / NOT_PRODUCTION. B2–B4 remain planned. Product code6563a3d; final deployment/documentation commit follows. No new dependencies or changes to original0001/content digest/lockfile. See p0b1-verification.md for final package/image/log hashes.
Final integration ruling: repository has only feat/rust-learning-core and no remote or base branch; preserve local branch/workspace, no merge/push/PR action to choose. Cost: integration remains local until a destination is established.
