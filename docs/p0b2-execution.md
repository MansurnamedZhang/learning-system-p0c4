# P0-B2 执行记录与取舍

状态：P0_B2_VERIFIED。早期 pending / in progress 行为历史记录，最终结论位于末尾。

# SDD ledger — plan: <workspace>/docs/superpowers/plans/2026-09-19-p0b2-personal-reading.md

2026-09-19 user explicitly said 推进吧 after plan delivery. BASE 182fa2a. Execute all six tasks inline, final independent review. Current brand 知织 / KnoWeave; no unrelated rename/refactor.
Ruling: reuse existing dedicated feat/rust-learning-core checkout as B1 continuation; clean baseline, no remote/main branch — avoids copying local offline caches, cost no separate worktree.
Ruling: manual plan ledger and briefs because bundled shell bookkeeping previously failed on this Windows host — preserve task identity/evidence and archive on completion; cost manual bookkeeping.
Pre-flight 1→3/4/5: strict commands vs projected output; raw persisted records remain internal. state editable requires historical unplaced anchors to be authorized, not merely current base.
Pre-flight 2→3: extract existing transaction-only insert revision primitive; preserve content-v1 digest and receipts. B2 owns its one request receipt; no nested public store transactions.
Pre-flight 3→5: adoption persists new layer/view in caller transaction, proposal decisions and receipt share commit.
Pre-flight 4→5: hidden prior origins must be authorized individually before returning anchors; migration classification requires actual old origin and new target both visible.
Pre-flight 2→6: new B1 upgrade test uses third independent empty DB; original P0-A upgrade still retained. Server baseline requested using frozen267b88 package.
Ruling: Task2 original68 baseline and schema gates run before B2 business tests exist; later full suite includes stubs only when intentionally establishing RED. No false all-green claims.
Task 1: in progress; strict contract tests and minimal type scaffolding first.
Task 2: pending.
Task 3: pending.
Task 4: pending.
Task 5: pending.
Task 6: pending.
Evidence: original B1 frozen267b88 baseline68 GREEN on server; core contracts4 RED→GREEN (core21 total); anchor2 RED→GREEN local. Schema1 RED missingtable→GREEN and originalPG20 GREEN; reading3 RED Storage all, server c29da118 package70files unchanged, evidence b2-schema-green-reading-red2-20260919/result.json.
Ruling: add placement_manual_decision for explicit PlaceUnplaced mappings outside migration — immutable manual-placement history required by spec; cost one small audit table.
Task 2/3: schema and transaction-only insert primitive ready; reading save/read implemented after3 RED, server verification pending. Extract write primitive into dedicated file before completion.
Evidence: migration classifier2 RED→GREEN local; MigrationStore2 RED Storage→GREEN server, then expanded three fresh databases workspace93/93 GREEN (99e35a8e,86files unchanged), original68 retained. Prior reference lifetime compiled error in tokio::spawn fixed by collecting owned BlockRefs before async call; Clippy clean.
Evidence: historical-location privacy regression RED confirmed 6e1027ae, exact assertion non-Storage; fixed current-source gate; targeted10 GREEN d568c685 including 8MiB/+1, dedup, original budget independence, actual cross-object FK and revocation.
Ruling: integration checkpoints rather than committing interim stubs per task — schema/exports/reading/migration share signatures; every frozen RED/GREEN package preserved — cost larger implementation commit, same independent review range.
Ruling: PostgreSQL JSONB text byte budget matches B1 and measured before body read; not compact JSON or transport bytes — cost callers must not estimate from HTTP sizes.
Task 1: contract6 + anchor2 + total-budget1 implemented; independent Python golden43b9dbbf and strict sample JSON; full core local passes.
Task 2: 0003+runtime/FK tests and extracted block_write; actual B1 store+receipt upgrade and original P0A upgrade passed full93. Extra FK assertions passed targeted10.
Task 3/4/5: reading/edit/migration and permissions passed, including explicit collision and receipt-trigger rollback, immutable old views, unplaced across two migrations and manual placement. Latest shared-body concurrency checks await new full run.
Task 6: ongoing; added third Compose DB. Final full suite/Compose/independent review/evidence audit pending.
Task 1: complete; strict core23 (incl reading6), anchor2, budget1 passed full101; independent Python golden and sample JSON included.
Task 2: complete; immutable schema2, P0A/B1 upgrades1+1, all original68 passed full101; old0001/0002/lockfile unchanged independently audited.
Task 3: complete; reading3 + overlay2 through actual PG, receipts and all-table rollback, selected-only upgrades, old views; full101 GREEN.
Task 4: complete; auth2 + exact budget1 + history privacy regression RED→GREEN, full101 GREEN.
Task 5: complete; classifier2 + migration5, explicit candidate/reject/merge/rollback/unplaced/privacy/replay; full101 GREEN. Final reviewer also checks missing input classes.
Task 6: host101/101 fmt0 Clippy0 verified package18f465cd92files; concurrency6 actual separate connections. Final review and clean Compose pending.
Evidence audit: source package and logs recomputed locally; artifact copy current10batches/22logs. No private DSNs. Status still REVIEW_PENDING, not verified delivery.
Final review: p0b2_review read-only 182fa2a..b213d6e; no Critical,3 Important,1 Minor coverage recommendation; no subreview. Independent review complete.
Final regrade: three Important stand: unplaced order scrambled; explicit unplaced location incorrectly non-null; manual merge lacks target-group mapping. All3 reproduced before fixes: local order RED first/second/third -> second/third/first; local explicit-unplaced location RED; PG manual merge RED source set2 expected/1 actual (a5b4ccab,logce895f31).
Final: Ruling: promote nested-migration coverage recommendation to Important acceptance gap because promised path identity guards protect note placement across repeated/moved sections — add deterministic nonempty-path/repeated-child/lost-parent/cross-parent/empty-child regression, with no behavior change (initial GREEN), cost one extra unit test rather than deferred acceptance uncertainty.
Final fix pass: preserve group-ID order plus saved within-group filtered order; explicit Unplaced never has active location; structure returns all mappings, both merged origins persisted atomically. Extended PG manual merge test checks prior view and actor-scoped receipt failure rollback including manual mappings. Pure8/8 and Clippy0; final PG/full/Compose pending.
Final: Ruling: reviewer excluded HTTP/UI/assets/relations/lineage, production throughput/backup/rolling upgrade — retain approved scope exclusions, not implicit acceptance — cost subsequent phases and production validation remain necessary.
Final: Ruling: final Compose was not judgeable at review time — treat it as mandatory remaining gate, no second reviewer; cost another clean-environment verification after fixes.
Final integration ruling: only local feature branch with no remote/base destination; preserve branch and workspace, no merge/push/PR. Cost integration remains local.
Final: fixed unplaced ordering — deterministic unit RED→GREEN; fixed unplaced active location — independent unit RED→GREEN; fixed missing manual merge target mapping — PG RED1→GREEN1 including failure rollback/history. Final host fresh3DB suite105/105,fmt0,Clippy0,source93 unchanged; package5d90ca9c,workspace logf3f85504. No deferred minors. Compose still pending.
Task 6: complete; reviewed-final host105/105 and clean Compose105/105, all fmt/clippy/config/build/up/stop exits0. Standard BuildKit no proxy fallback/daemon change. Independent audit14batches31logs+10localTDD logs; package93files match, original0001/0002/lockfile/content/digest unchanged. All3B2containers exited0, volumes retained, resource/network/ports checked, existing4services running.
Final: P0_B2_VERIFIED / NOT_PRODUCTION. Implementationb213d6e, reviewed fixesde71766, documentation commit follows. No unresolved Important/Critical or deferred Minor. Execution ledger archived to docs/p0b2-execution.md before deleting only this plan scratch directory. B3/B4/HTTP/UI/assets remain unimplemented and require their phase work.
