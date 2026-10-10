# P0-C4 源备份尝试生命周期

更新：2026-10-04。源端尝试生命周期限定验收、独立文档及整源码复审、路线图静态QA与正常feature分支源码发布均已完成；浏览器file协议被策略拒绝，渲染和交互未验证。父C4 Task3与整关保持开放，本轮后续修改仅作发布记账。

现场为 suite `943f3a4a-16a9-44ab-af07-257151349a7a`，base `e43bb2cef380adda3bf9486005e58a175237d247` 的475文件未提交 working-tree-green，common ZIP SHA-256 `85211a1fead1cd903b5ee269230b02ee6dbfc31ce2808ea31ec04aa5e4d0a466`，manifest `48aba66fef05f6f133aa6a7283651d5b2de0f30e30af9152a15797b2f7b34478`。plan `08195e2220cd4a2c3667ce3e43c348dea11634541e3c1aa88015f7f19bf09f55`；GO `0f9480c59fe4f9bd518efbae2c11fdf363ccb2b6eb24348175a309db00a663ce`；9681-byte aggregate `e61470e9092cf9bca4134e6a872902039a60a0473fd395138647db4acc66f45a`，状态 `INDEPENDENT_COMPLETE_SCOPED_LIFECYCLE_SUITE_ACCEPTED_NOT_COMPLETE_NOT_RESTORE`。后来源码发布不能改变此 execution identity。

## 四个 Linux 管理 Rust 库入口

由 learning-backup re-export；非 Linux Unsupported。没有 HTTP API、登录/前端或生产部署。测试 driver CLI 不等于产品管理 CLI。七阶段 v1 journal 原字段顺序/null/规范字节、公开旧 API及旧迁移保持兼容。

| 入口 | 行为、成功与拒绝 |
| --- | --- |
| `prepare_source_backup(&PgPool, &FsAssetStore, &SourceBackupConfig)` | 新尝试；同 admitted 会话覆盖真实 PG dump、全部 ready index、本地 sealed pin全字节及持久释放。成功返回 SourceLocalPin与Released，非CompleteBackup；其他unresolved authority阻止新尝试。 |
| `force_close_release_ready(&PgPool, &Path, Uuid, &str)` | 仅严格ReleaseReady及原bound root；REVOKE/drain、create-only `<id>.release-recovery/closed.json`。close-only不重验pin、不finish/abandon、不重开；重复调用可能先关闸后因已有目录失败，非通用幂等修复。 |
| `finish_source_backup(&PgPool, &SourceBackupConfig)` | 返回 SourceLocalPin。仅 DumpAndIndexDurable/PinsDurable/ReleaseReady 且真实已 published matching `<id>.sealed`；reclose/drain/full verify 后补合法阶段至 Released。早期 Intent/Closed/Drained 在首个变更前拒绝；缺失、损坏或只有 staging 的 pin 不能完成或重新开放。任何 abandonment authority 禁止 finish。 |
| `abandon_source_backup(&PgPool, &SourceBackupConfig)` | 返回()。合法unresolved Intent..ReleaseReady可显式放弃，部分/坏payload可以，但坏journal authority不可绕过；durable ready后GRANT/核验，再durable abandoned terminal。原journal phase/source/sealed/staging全保留，无fake Released/删除/GC/completion资格。Released禁止abandon。 |

finish/abandon不消费capture-only executable/passfile/host/port，不执行dump、重采集资产、重封存缺字节或promote staging；保持ID/database/project/isolation/control/local-pin/drain timeout核对。DumpAndIndexDurable sealed-rename seam须先sync/full readback，再补PinsDurable；不能凭journal digest/token合成成功。

Released finish只读重验pin；abandoned repeat只读验证authority。二者都须safe open runtime ACL、PUBLIC closed、无unexpected sessions，不REVOKE/re-GRANT、不改terminal字节（可resync保留证据）。closed/unsafe terminal返回明确manual ambiguity。

## 原构建、原绑定和操作前置证明

保存**原 audited lifecycle-capable management binary**及executable SHA-256、原编译 KNOWWEAVE_SOURCE_COMMIT / KNOWWEAVE_BUILD_ID_SHA256 / KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256、原source-binding bytes/root、manifest/source/migration身份及actual observed build-profile/compiler/materialization provenance。build ID是包输入身份，不是executable SHA。finish对比manifest.source与当前admitted PG18、完整embedded migrations和这些compile值；latest binary或同profile不能自动兼容旧捕获，无runtime hash override/relabel manifest，缺新API旧artifact也无自动升级恢复承诺。

每case独立issuer先预置binding再compile；case-specific二进制/control binding各自核验，不借用首leaf产物。DEV/TEST DEBUG=0、DEBUG_ASSERTIONS=true、OVERFLOW_CHECKS=true、OPT_LEVEL=0、STRIP=none及128MiB cap、独立materialization目标mode0500/nlink1/不同inode必须关联真实receipts；profile本身不是兼容或完整性证明。

SourceAdmission固定同库live lock/owner/DB、独立compiled control binding、严格所有authority、safe角色/prepared-work与fresh driver isolation/runtime-login denial先于变更。held control/pin/source/sealed handles贯穿核验、释放与补偿；不重借pool/重开可变路径。root alias按path+dev/ino判断。显式retry仅新isolation generation，不重签binding绕过unresolved、不换inode/复制root/编辑journal或sidecar。

source精确四文件manifest.json/asset-index.json/database.dump/roles.json、bounded canonical manifest/index、UUID/digests/PGDMP、compiled身份及所有published pin字节/精确树与journal source/pin SHA一致。历史合法但source/pin摘要不等的journal可以解析，finish仍拒绝其真实内容不一致。

## 原子记录和人工歧义

新journal phase在held control的 `<id>.journal-staging`（0700）用唯一 `<phase>-<v4uuid>.tmp`（0600、<=4096 bytes）写/fsync，held no-follow同device parents通过renameat2(RENAME_NOREPLACE)发布，双parent fsync/readback再更新内存。new start先在initial-v4uuid持久Intent后发布完整control，不公开空final。临时残片保留、非权威，旧.control extra-entry规则不放宽。

`<canonical-id>.abandonment`0700；两个正式文件root-owned0600普通单链接非空<=4096bytes，compact serde JSON无额外换行，UUID规范小写非nil、digest小写64hex，deny_unknown/duplicate/noncanonical。

| 文件 | 精确字段顺序 |
| --- | --- |
| ready.json | format_version:1, capability:"source_abandonment_v1", backup_id, action:"abandon", source_binding_sha256, journal_phase, journal_record_sha256, retained_artifacts:"keep_all", state:"release_ready" |
| abandoned.json | format_version:1, capability:"source_abandonment_v1", backup_id, action:"abandon", source_binding_sha256, ready_sha256, state:"abandoned" |

ready关联admitted compile pin和最后严格journal原字节；terminal关联ready原字节。唯一合法 `.tmp-<v4uuid>`允许零/部分字节但同权限/单链接/上限。unknown/orphan/conflict/缺ready/foreign ID-binding或变动journal拒绝；空/临时sidecar仍unresolved，显式retry新temp并保留旧片段。stage-only或legacy partial/empty/gapped final仍fail-closed/manual，不自动修复。

GRANT/inspection/terminal publish-sync-readback失败时同admission补偿REVOKE并核验closed，无法确认报ambiguity。terminal rename可见但补偿closed时不得以重复调用自动GRANT或覆盖。保持clients停止和source隔离，记录exact error/ACL/backend/authority，保留全部evidence交明确operator调查。TaskAbort/Tokio cancellation不证明GRANT未提交，不是SIGKILL/power-loss；Drop只关闭admission，不隐式GRANT/删证据。

## 真实新九叶与计数

命名测试入口 `source::lifecycle_tests::real_capture_all_ready_and_retained_pin` 执行 9 次（1 个 primary + 8 次 prelude）；该计数不统计各测试体内部的 prepare_source_backup/pg_dump 总调用次数。

同一475文件ZIP贯穿九个fresh leaves；capture在每leaf首位保留first=True全部真实denial/drain/FS4/current19/index/retained-byte语义。每leaf一次启动，独立leaf核验后顺序推进；offline aggregate重验exact nine闭包与global identities/occurrences，旧failed suites的成功片段不贡献覆盖。

| 实际执行层 | 通过/覆盖 |
| --- | --- |
| PG body executions | 17；16 lifecycle+1 legacy，各exact body为1pass/0fail/0ignored |
| Unique lifecycle primaries | 8，另1 unique legacy primary |
| 命名 real_capture_all_ready_and_retained_pin entry 执行 | 9 = 1 primary + 8 preludes；不是内部 prepare/pg_dump 总调用计数 |
| FS | 36 = 4 unique bodies重复9次 |
| 当前相关回归 | 171 = 19 unique bodies重复9次 |
| ordinary Linux Python | 468pass+27 named-root skips（九次52+3） |
| root Python lane | 27pass/0skip（九次3） |
| f506历史Rust prerequisite | workspace --lib 211 pass/42 ignored，其中 learning-backup 165 pass/41 ignored；fmt/严格Clippy分列，不称当前ZIP重跑 |
| Windows当前source driver | 55discovered/49pass/6existing skips，独立平台结果，不计PG |

八个unique lifecycle名为：real_capture_all_ready_and_retained_pin、finish_real_pin_after_sealed_rename、finish_real_pin_from_pins_durable、finish_real_release_ready_before_and_after_grant、abandon_early_and_late_attempts、abandon_crash_retry_and_terminal_ambiguity、lifecycle_release_failure_compensates_same_session、lifecycle_admission_and_held_roots（均source::lifecycle_tests::）。legacy为source::binding_tests::matching_bound_recovery_recloses_without_finishing。

percase tools_before/migration_setup/migration_readback/tools_after实际RPC与samecase/container/stdout/stderr/process/顺序核验，不以equal digest借用另case。fresh UID999 sampler/root anchor/container pre/post guards及actual denial/generation/heldroots证明限定single trusted fixture namespace。规范membership `GRANT learning_auth_lock TO learning_admin WITH INHERIT FALSE, SET TRUE;`；direct owned PG18 psql的PID/starttime/exe/envkeys/backend/transport与真实会话关联。

legacy UID999 readonly/proc metadata observer关联实际consumer+holder+PG backend；PG process-title覆盖后的environment area按opaque bytes SHA表示。它不是environment keys/allowlist/policy或跨采样稳定性声明；不记录/发布原始环境area。sameholder COMMIT、exit和backend absence有实际证明，legacy仍为close-only/SYNTHETIC prior phases/NO_DUMP，其leaf的capture prelude是真捕获但不能反向赋予legacy恢复门。

固定stop--timeout与严格sealed辅助SIGKILL startup parser为fixture/controller细节，auxiliary startup0pass/0ignored，不增加regression coverage。生命周期hooks/TaskAbort与既有sealed真实child SIGKILL分列，不称本slice覆盖掉电或全部processfaults。真实capture包含full ready index/assets；nonready INSERT23514约束拒绝不等于持久nonready行排除，TOC pg_restore--list不是restore。

17 个 case 的清理、最终 binary/binding/source/root 证明及九份 raw-index/result/readback/verification hashes 见[C4 验证记录](p0c4-verification.md)。已核对 17 个 PG 停止、17 个内部空网络与 51 个卷保留、226 个 producer helpers 和 9 个 root-unit helpers 移除、pending 不存在；这些是现场回传时的观察，不承诺未来远端状态。caps8192 files/2MiB文件及result/1MiB ledger/512MiB tree/64file2MiB reads/256case512batch audits/16generations、cleanup180s、shared78000s、offlineaggregate180s不扩大；不保证未来运行一定容纳。

## 接受边界与发布

独立实际返回审查 Spec/Quality/ActualAcceptance 均 PASS，残留阻塞发现 0，报告 SHA-256 `7fea115d64204f5da0f453eb6a7f70aad7c9b94249b087e767bf8ef5a04d5a09`。生命周期源码已正常发布至 `feat/p0c4-backup-recovery`：[源码提交 `e51ff31c96b446f9b25c854d43c769182b663fe1`](https://github.com/MansurnamedZhang/learning-system-p0c4/commit/e51ff31c96b446f9b25c854d43c769182b663fe1)，tree `c98b730e51926596fe5ac21ac2fffbb00cea92c0`、唯一 parent `e43bb2cef380adda3bf9486005e58a175237d247`。独立 GitHub ref/commit/tree 核验回执 `publication-source-remote-verification.json` 为479 bytes、SHA-256 `e6f3cd2326073010c0c65dc8b30d4ce41aa887404608deaf3823360f667c02e3`；`refs/heads/feat/p0c4-backup-recovery` 与该 commit 一致，477个blob的路径/mode/OID与本地匹配，truncated=false。现场执行仍为e43基线的475文件working-tree-green ZIP `85211a1fead1cd903b5ee269230b02ee6dbfc31ce2808ea31ec04aa5e4d0a466`，不反向写成该发布commit运行。当前后续修改仅为该源码发布的记账，不增加验收范围，不改变原证据，不关闭父Task3/C4；本段只记录上述已核验的源码发布，不自引用本段所在的记账提交。

独立限定文档修补复审 Spec/Quality PASS、P0/P1/P2/P3均0（4028 bytes，SHA-256 `01a9b0d7b892cad05941610186d08512779c27fad9fc2896e8b50dc7fde437b2`）；最终整源码复审 Spec/Quality PASS、Critical/Important/Minor均0（10584 bytes，SHA-256 `2357d8e2b279d8dbc7273e14a9507d71b968b8c8d073e20725d3099bfa5b21fc`）。其后的实际源码push与远端readback使用上列独立回执，不把审查readiness当作发布证明。

root最终路线图静态QA已完成：14/14输出两次hash一致、精确6 SVG及56 HTML链接核对通过，回执 `roadmap-static-qa-source-publication.json` 为2120 bytes、SHA-256 `d319a0da6b979141430a6d543991b65f463530a9544dc502c27c4d1345c0d397`。浏览器file协议被策略拒绝，浏览器渲染和交互**未验证**；该项只有static完成与browser结论分列，不代表browser PASS。

接受证据必须同时携带 aggregate、独立返回审查（15272 bytes，SHA-256 `7fea115d64204f5da0f453eb6a7f70aad7c9b94249b087e767bf8ef5a04d5a09`）和 legacy 补充收据（1068 bytes，SHA-256 `c3f95f8196a6deda0236c1cf19be558b43c865d606ad9a3a64d0bfe49ec419f7`）及其六份原始文件。补充原始文件仅在私有证据中保留，不上传 Git；它们不在未改写的 aggregate/raw-index 中，不能声称冻结 aggregate validator 已自动检查这份补充。旧e1容量失败、旧3fa LEGACY_PROCESS_JOIN、旧d9aa LEGACY_BACKEND_IDENTITY及早期stop/parser/infra失败保留，不重放、不跨source拼接。

source same-identity physical-clone endpoint仍待验，**不同于此前已接受的restore-target clone门**。runtime-held pin根/内容证明非跨重启persistent enrollment，samebyte替代pinroot仍可能接受。asset/localpin持久身份/GC protection discovery、broaderfaults/offhost独立域、CompleteBackup/fullrestore、父C4Task3三框、Task4/5、C4/production、HTTP/login/frontend继续开放；fullbroaderDB/四旧升级未运行。保留副本非独立故障域，pin非restore许可。


相关记录：[源控制根绑定](p0c4-source-control-binding.md)、[历史 live admission](p0c4-source-admission.md)、[父计划](superpowers/plans/2026-09-28-p0c4-backup-recovery.md)、[生命周期子计划](superpowers/plans/2026-10-04-p0c4-source-attempt-lifecycle.md)。
