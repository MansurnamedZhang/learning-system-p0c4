# P0-C4 源备份完成与放弃 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: `superpowers:subagent-driven-development`；每任务由实现子代理完成，独立子代理核对 Spec/Quality。步骤使用 `superpowers:test-driven-development`，实际 Linux/PG18 证据由控制端核对，完成前使用 `superpowers:verification-before-completion`。

**Goal:** 在已批准的 C4 Task3 内，让管理者显式完成已真实封存的中断备份，或持久放弃未完成尝试，保留所有旧证据并安全处理连接闸。

**Architecture:** 复用同会话 `SourceAdmission` 与独立构建 pin 的控制根；保持七阶段 v1 journal 字节不变，以 held-dir 原子发布补齐未来写入。finish 只重验已经发布的真实本地 sealed，abandon 使用独立 ready/terminal sidecar。按新批次独立签发绑定、再构建消费者，实际执行八项 Linux/PG18 门；后续源码发布不改变现场快照身份。

**Tech Stack:** Rust 1.97、现有 SQLx 0.8.6/serde/SHA-256/libc、PostgreSQL 18、固定 Docker 测试镜像与离线 Rust builder；不新增依赖或业务迁移。

**Spec:** [已批准 C4 设计](../specs/2026-09-28-p0c4-backup-recovery-design.md)的备份协议 2–5、异常退出与保留约束；[父 Task3](2026-09-28-p0c4-backup-recovery.md)；[已接受控制根绑定](../../p0c4-source-control-binding.md)。本施工单细化已批准范围，执行方式沿用用户的逐任务子代理实施/审查与范围内自主测试授权。

## 当前限定接受与发布状态

2026-10-04 源备份生命周期新 suite `943f3a4a-16a9-44ab-af07-257151349a7a` 的九个独立新批次及离线 aggregate 已通过，独立返回审查 Spec/Quality/ActualAcceptance 均 PASS，P0/P1/P2/P3 均为 0。实际执行 **17 个 PG body（16 lifecycle + 1 legacy）**，覆盖 **8 个唯一生命周期主用例 + 1 个 legacy**；命名测试入口 `source::lifecycle_tests::real_capture_all_ready_and_retained_pin` 执行 9 次（1 个 primary + 8 次 prelude）；该计数不统计各测试体内部的 prepare_source_backup/pg_dump 总调用次数。文件系统 36 次为 4 个唯一测试体重复 9 次，相关回归 171 次为 19 个唯一测试体重复 9 次；普通 Linux Python 468 通过/27 个具名 root 跳过、独立 root 27 通过/0 跳过分别计数。现场输入为 e43 基线的 475 文件 `working-tree-green` ZIP，后续发布不改变实际执行身份。

接受证据必须同时携带 aggregate、独立返回审查（15272 bytes，SHA-256 `7fea115d64204f5da0f453eb6a7f70aad7c9b94249b087e767bf8ef5a04d5a09`）和 legacy 补充收据（1068 bytes，SHA-256 `c3f95f8196a6deda0236c1cf19be558b43c865d606ad9a3a64d0bfe49ec419f7`）及其六份原始文件。补充原始文件仅在私有证据中保留，不上传 Git；它们不在未改写的 aggregate/raw-index 中，不能声称冻结 aggregate validator 已自动检查这份补充。

现场base/ZIP/manifest/GO/plan/aggregate、nineleaf及17case pins见[接受ledger](../../p0c4-verification.md)。Task1实现/限定审查、Task2/2A/2B实际范围已接受；历史f506 workspace --lib 211通过/42忽略（learning-backup 165通过/41忽略）及fmtClippy为前置检查，当前Windows55为49pass/6skip。fullworkspaceDB/四旧升级未运行，不能推称整个workspace通过。Task3限定文档及整源码复审、root路线图static14/14、6SVG、56HTMLlinks，以及首源码commit正常push/独立ref-tree回读已完成；浏览器file协议被策略拒绝，渲染/交互未验证。源码发布收据见[生命周期runbook](../../p0c4-source-attempt-lifecycle.md)，现场ZIP保持不变；当前后续修改仅记账，本段不自引用记账提交。父Task3三框仍开放。

## Global Constraints

- 起点 `e43bb2cef380adda3bf9486005e58a175237d247`，复用现有 `feat/p0c4-backup-recovery` 隔离工作树；不改 C1–C3 API、旧迁移、旧版本身份、complete/restore 实现。
- 管理权限和密钥绝不进入 Worker、runtime 或备份包；所有测试使用新库/卷，保留旧失败、证据和隔离资源；不重放、不清理已有批次。
- 用户已直接要求无需逐项确认；批准范围内自主封存/上传/验证，仍核对精确输入 pins、全新 UUID/路径/卷/未占用子网。sudo 凭据仅在用户自己的终端输入；本轮优先普通 SSH 协调和新 fixture 内的 root，不读取旧 host-root 私有证据。
- 仅独立预置 `source-binding.json` 的构建 pin 提供控制根/数据库身份预期；无运行时 expected hash、自动注册或未绑定回退。
- 七个 `GatePhase`、原 `SourceGateRecord` 字段顺序/null/规范字节/phase 文件名/顺序保持不变。历史结构合法但 source/pin digest 不等的日志仍可解析；finish 另行拒绝其内容不一致。
- 不删除、改写或解封原 journal/source/sealed/staging；abandon 不伪造 `Released`，不让弃用尝试进入 complete。保留副本与日志不是独立故障域证明。
- 单可信 fixture 命名空间；本轮 held local-pin root 与内容摘要不宣称跨重启独立身份注册，同字节替代 pin root 仍可能被接受。资产根身份、GC 自动发现、同身份物理克隆端点、异故障域、完整恢复/CompleteBackup/C4/生产仍单独跟踪。

## Review Focus

1. 真正 sealed rename 后、PinsDurable 写入前中断：只在原控制权威与全量真实字节重验后补阶段；staging-only、坏文件不能冒充成功。
2. ready/terminal 或 Released 发布的 rename/fsync 缝隙：可见记录不等于成功；补偿与明确歧义不能变成重试自动 GRANT。
3. 其他未解决尝试、预备事务、错误绑定/别名根：两入口均在首个变更前拒绝；同连接和根句柄不能中途丢失。
4. 规范字段/重复 JSON/未知文件/链接/部分临时文件/孤儿 sidecar：只有严格 ready+terminal 关联证明可解除扫描阻塞。
5. 取消或进程退出落在已发 GRANT 的窗口：不把错误/取消解释成服务端未提交；保留 blocking ready/原件，显式重关闸或报告 manual ambiguity。Drop 只关闭 admission，不执行隐式 GRANT。

## Frozen Interfaces and Formats

```rust
pub async fn finish_source_backup(
    admin: &PgPool, config: &SourceBackupConfig,
) -> Result<SourceLocalPin, BackupError>;
pub async fn abandon_source_backup(
    admin: &PgPool, config: &SourceBackupConfig,
) -> Result<(), BackupError>;
pub fn rename_entry_to_noreplace_without_sync(
    &self, old: &str, destination: &BackupDir, new: &str,
) -> std::io::Result<()>;
```

两管理入口由 `learning-backup` re-export；非 Linux 显式 Unsupported。恢复不消费 capture-only dump executable/passfile/host/port，不运行命令，不重采集资产或重新封存；仍核对 ID/database/project/isolation/control/local-pin/drain timeout。

跨父 rename 在管理用 `BackupDir` 和私有 `secure_dir` 实现：单 component 名、两个已持有 no-follow parent、同 device、源普通单链接文件或目录、拒绝链接/特殊文件，`renameat2(RENAME_NOREPLACE)`，不覆盖或删除；调用者负责两个 parent fsync。

未来 phase staging 为原控制根的 `<id>.journal-staging`，0700；唯一 `<phase>-<v4uuid>.tmp` 为 0600，<=4096 字节。先写/fsync 文件与 staging，跨父 rename 到 held `.control` 正式 phase，sync 两 parent/readback，最后更新内存状态。新 start 在 staging 的 `initial-<v4uuid>` 内持久 intent 后原子发布整个 `.control`，不公开空 `.control`。临时残片保留、非权威；stage-only 无正式 journal 或旧坏/空/gapped final 留存并 fail-closed/manual，不修补或推断成功。不放宽旧 `.control` extra-entry reader。

abandonment 位于 held control 的 `<canonical-id>.abandonment`（0700）。两正式文件 root 所有、0600、普通单链接、非空且 <=4096 字节，deny_unknown_fields struct 按下列字段顺序直接 serde_json 紧凑序列化，无额外换行；UUID 小写规范非 nil，digest 小写 64 hex。

| 文件 | 精确字段顺序与值 |
| --- | --- |
| `ready.json` | `format_version:1, capability:"source_abandonment_v1", backup_id:Uuid, action:"abandon", source_binding_sha256:String, journal_phase:GatePhase, journal_record_sha256:String, retained_artifacts:"keep_all", state:"release_ready"` |
| `abandoned.json` | `format_version:1, capability:"source_abandonment_v1", backup_id:Uuid, action:"abandon", source_binding_sha256:String, ready_sha256:String, state:"abandoned"` |

绑定摘要取自已 admitted 的独立 compile pin；journal digest 来自最后严格恢复的旧规范记录；terminal 摘要关联 ready 原字节。唯一合法临时名 `.tmp-<v4uuid>` 可为零/部分字节，仍须普通单链接/0600/<=4096；保留而不作证明。未知条目、孤儿/冲突 sidecar、缺 ready 的 terminal、坏 final、外来 ID/绑定或变动 journal 均拒绝；临时/空 sidecar unresolved，显式 abandon 用新 temp 重试，不删除旧片段。

## Task 1：原子日志与显式生命周期核心

**Files:** 修改 `crates/learning-assets/src/backup_fs.rs`、`secure_dir.rs`，`crates/learning-backup/src/maintenance.rs`、`source.rs`、`source/binding.rs`、`sealed.rs`、`lib.rs`；新建 focused `source/lifecycle.rs`、`source/lifecycle_tests.rs`，必要的纯契约/文件系统测试放各 owning module。保持 C1/C3 普通资产操作与公开封存包装的行为。

**Interfaces:** consumes 既有 `SourceAdmission`/bound control/`BackupDir`；produces 两公开管理入口、跨父 rename、crate-private held-root seal/verify 与严格 abandonment authority/扫描分类，供 Task2 实际验收。

- [x] 先写 RED：七阶段 literal 旧字节（含不等合法历史 digests）、finish phase eligibility、abandon ready/terminal codec/关联/unknown/duplicate/temp/orphan、不同动作不得切换、非 Linux Unsupported。记录初始缺 API/具体协议断言的失败，基础设施失败另列。
- [x] Linux 文件测试固定 atomic file/directory/noreplace/EXDEV或不同 device拒绝、链接/特殊文件/单 component、写/rename/parent sync/readback 缝隙与 held-parent 改名；新 phase 不出现部分 final，旧坏 journal 保持拒绝。测试体的 root/可信 ancestor 条件必须由现场执行证明。
- [x] 实现 held-dir 原子 phase 出版与 journal 内部 root/staging handles；旧读者/字节兼容。seal/verify 私有 helper 消费已持有 pin root，prepare 在第一变更前保留它；不在恢复中重开可变路径。
- [x] 两入口先同会话 owner/DB/live lock/binding、严格目标 journal/全部其他 authority、角色/prepared 安全与 fresh live driver proof；保留控制、pin、source、sealed handles。根 alias 比 path 和 dev/ino；其他 unresolved 阻止 reopen。
- [x] 非终态合法动作显式 REVOKE/drain，再重验。finish source 精确四文件与 bounded canonical manifest/index、UUID/完整源文件 hash/PGDMP、当前 PG18/完整迁移集合及 compiled commit/build 身份；source digest==journal source SHA，整个已发布 pin 逐字节/精确树重验且 manifest/index/source records 一致；pin SHA==source SHA==已有 journal pin SHA。
- [x] finish 仅从 `DumpAndIndexDurable`（真实 published pin 先 resync，再补 PinsDurable）、`PinsDurable`、`ReleaseReady` 续到旧 Released。Intent/Closed/Drained 首变更前拒绝，无重捕获或 staging promotion。Released 只读重验与安全 open ACL，无任何 GRANT/日志改写；closed/unsafe terminal 报 manual/ambiguous。
- [x] abandon 仅 Intent..ReleaseReady，原 journal 不推进；缺坏捕获字节可以放弃，坏 journal 不行。关闭/drain 后 atomic ready、readback、repeat closed inspection/live driver、GRANT runtime only、inspection、atomic terminal/readback。已有 terminal 只读核对 ACL，不能补 GRANT；成功 Released 禁止 abandon，任何 abandonment decision 禁止 finish。
- [x] GRANT/postinspection/terminal publish-sync-readback 失败，在同 admission 内 REVOKE 并完整闭闸核对；无法确认报歧义。terminal rename 已成功但 sync 失败且补偿 closed 时，重试只读拒绝；不回滚文件或自动重开。取消遵守 Review Focus5，只有实际关闭证明才写闭闸结论。
- [x] f506历史 `cargo test --offline --locked --workspace --lib` 为211通过/42忽略，其中learning-backup为165通过/41忽略（assets10/1、db36/0、core与worker各0/0）；Linux格式/严格Clippy前置检查；当前FS4与相关19实际重复九次，Windows driver55为49通过/6跳过；完整workspace DB/四旧升级仍未运行，上述历史workspace `--lib`已实际运行，但后续Python修改不是当前ZIP的完整workspace DB/四旧升级重跑；独立 Spec/Quality 复审 exact diff/pins。现场八门与恢复/进程 fault 实际计数在 Task2，测试钩子不能自称 SIGKILL/掉电。

## Task 2：独立签发构建与真实 Linux/PG18 八门

**Files:** 新 `scripts/p0c4_source_lifecycle_gate_acceptance.py`、对应 `test_*.py`；Task1 的 live test module；复用已公开 immutable admission helper 与经过 hash 核验的 binding fixture flow，不覆盖旧 driver/payload/batch。控制器 packaging/upload/readback orchestration 仅在本计划忽略目录。

**Interfaces:** consumes Task1 实际入口与 test names、固定 PG/builder、source-binding 独立 issuer→consumer build；produces exact canonical result/源码+binary+binding audit/精确资源停止与保留证据。

- [x] 先写 driver RED：payload/hash/路径/原样 runner、首次资源名与 subnet overlap、created/started identity、timeout/输出上限、compiler JSON 的唯一 artifact、exact test stdout/exit/count、no silent skip、source unchanged、cleanup ambiguity、独立绑定先签发后 compile、fresh isolation driver proof。Windows/WSL 平台 skips 分列。
- [x] 新 /24 内部网络、UUID 项目/库、PG/control/pin/asset 卷，每 PG <=2CPU/4GiB；离线 builder <=4CPU/8GiB、无网络和只读源码。ordinary SSH 可协调，fixture root 只能处理本批新卷；不读取旧 host-root 文件。固定 PG18 与 builder pins 与容量在执行前重查。
- [x] 初始化角色/库/root 后独立观察并预置 source binding，随后独立 consumer compile；所有 live binaries 的 bind/source/build/CLI artifact hash 在 before/after/final 核对。测试 config 无自签 pin。新恢复 attestation、真实 runtime login refusal probe 与 driver liveness/inspect 权威在调用期间保持。
- [x] 实际执行下列八个 `source::lifecycle_tests::` ignored entry，逐新 case `--exact --test-threads=1`；完整 libtest 行、exit0、1 pass/0 fail/0 ignored 和原始日志；任何未运行/列举/infra错误不算通过，失败 case 不自动重放。

| 精确 entry | 真实验证 |
| --- | --- |
| `real_capture_all_ready_and_retained_pin` | genuine prepare→固定 PG18 real pg_dump、全部 ready index/每原件；多空间/不同 digest/重复 digest/已引用与未引用 ready，独立 DB 行比对；当前真实迁移只允许 ready，非 ready INSERT 须以 23514 拒绝且行数不变，不能宣称持久非 ready 行排除已测试；Released；仅移除本夹具原件后整个 pin 仍可验。TOC 仅 `pg_restore --list`，不恢复。 |
| `finish_real_pin_after_sealed_rename` | 真捕获 sealed rename seam，原 backend 消失；fresh proof，DumpAndIndexDurable 重验并到 Released。 |
| `finish_real_pin_from_pins_durable` | 真 PinsDurable 中断；缺/坏/staging-only pin 不重开，证据保留；有效原 pin 完成。 |
| `finish_real_release_ready_before_and_after_grant` | 顺序独立尝试分别 before/after grant 中断；实际原 backend 消失、reclose/full verify；终态重复只读且没有额外 dump。 |
| `abandon_early_and_late_attempts` | 合法阶段表、真实早期失败和晚期 pin；保持原 hashes；有效 terminal 后才允许新捕获；动作冲突与 Released-abandon 拒绝。 |
| `abandon_crash_retry_and_terminal_ambiguity` | ready/grant/terminal rename-sync 钩子；中间阻塞、显式 fresh retry；closed terminal 无 GRANT、畸形/孤儿拒绝。钩子范围与真实退出分别记录。 |
| `lifecycle_release_failure_compensates_same_session` | 真晚期 pin；grant/inspection/terminal 文件与同步失败；同 backend/live lock 持有到补偿；closed 或明确 ambiguity；取消不隐式重开/删证据。 |
| `lifecycle_admission_and_held_roots` | 两 API busy/wrong root/其他 unresolved/prepared-work 拒绝；失败前后 ACL/树不变；两个 held roots 改名仍走原 handle；独立旧 admission/binding 回归分列。 |

- [x] 相关纯 Linux library/文件系统体、格式/严格 Clippy、selected contract/journal 和公开旧 admission/binding 相关回归；full workspace DB/四旧升级若未运行明确列待，历史版本计数不转移。
- [x] 精确 helper IDs 清理、PG IDs 停机、卷/内部空网络与全部 evidence/源码保留，pending absent；控制端读取/独立复算 canonical result与原始精确输出，并单独独立返回审查。不要使用泛化 `down --volumes`、prune 或 unknown-name cleanup。

## Task 2A：实际 PG 命名空间的只读采样依赖

2026-10-04 的实际验收暴露了前置假设缺陷：UID0 读取 PID1 命名空间返回空行，同 UID999 最小复现可见；未证明底层内核机制。当时五轮静态修正通过、Task2真实验收未接受；该历史失败保留，当前完整九叶与补充review已限定接受，见上文。控制端按 breaker 对必要依赖作以下裁决，不能将失败停机或已完成迁移当作八门通过。

**Files:** 仅 Task2 两份 Python 产品文件；新忽略私有执行桥用于独立读取验证。Rust/core8、公共 helper3、迁移15、旧包/记录均保持冻结。

- [x] 保留 UID0 锚点的 PID/starttime/UID/FD9/锁/命名空间严格相等；其原始采样改为七行。固定 UID999 元数据命令只读实际 PG PID1，核对实际采样器与 PID1 UIDs、exe/starttime、非空命名空间、CapEff/CapPrm/CapAmb=0 和 NoNewPrivs=1；不要求 PG 默认 CapBnd=0，不增加/删除 PG capability，不用 self/预期值替代 PG 观察。
- [x] 每次 audit 都获得新采样，前后精确容器/image/host PID/StartedAt/restart 状态与初始 running pin 一致；原 anchor10 与 broker/proof 权威格式保持兼容。记录实际 RPC/process/原始输出引用，独立桥重建 anchor 和当前 generation/archive 关联，不能只接受声明字符串。
- [x] 不扩大 5 秒 RPC、20/30 秒刷新/新鲜度、phase 时钟、8192 日志文件、读取组64文件/2MiB、result2MiB、ledger1MiB、end-tree512MiB 限制。动态 audit 计数只保证 fail-closed，不保证九案例容量；实际 PG 可见性、耗时与容量均须现场验证。
- [x] 每次资源预检独立批量读取网络 IPAM，保留完整真实前/后列表与碰撞、子网核验；当前 generation 显式关联实际 container/network 观察。预留精确停机和证据容量；不得因日志上限阻断停机。超时后已发出的 broker 请求可能留下 ack，只能保留为歧义证据，不能计为成功或自动继续。
- [x] 两处持久 owned psql 客户端直接使用固定 PG18 真实二进制，保持 env-i 的三项环境、原 UID/角色/数据库/锁/期限；不将系统包装脚本注入的额外环境加入 cohort 允许集合。
- [x] 针对新边界先 RED/GREEN，保持既有55名称/root3分区；独立实现/复审后，新精确输入与九叶十七个新隔离案例验证实际 FS4/PG8/legacy1/相关19与全部原始采样关系。失败停止后续、停机留卷留证，不重放、不自动放宽。Task3 发布仍须真实 Task2 接受结果。

## Task 2B：保持容量上限的独立小批次与完整覆盖核验

2026-10-04 实际单批运行在六个真实 PG body 通过后因 `NAMESPACE_LOG_CAPACITY` 停止：2668 个 RPC 三文件共 8004，另 93 个 child/ledger 文件，总计 8097；三个尚未写入的 ledger 加下一案例 93 槽要求 8193，超过 8192。失败批次保留，不转为成功分片。当时Task2尚未接受；当前采用下述九叶实际接受，原失败不投影为成功分片。此为 Task2A 五轮后确认的必要编排依赖，不是第六轮修补。

**Design:** 每个固定主用例为一个独立 leaf，保留 `real_capture_all_ready_and_retained_pin` 在首位及原 `first=True` 的全部语义；legacy 为独立 capture+legacy leaf。9 leaves，每 leaf 最多 2 个新案例，共 17 个真实 PG body：8 个唯一主用例、8 次额外捕获 prelude、1 个 legacy。重复 prelude、9×FS4、9×相关19及9×Linux/root检查分别计数，不能扩大唯一用例覆盖。

**Files:** Task2 两 Python 产品文件；本计划忽略目录的新 `execution-suite/controller.py` 与 `suite-validator-checks.py`。Rust/core8、公共 helper3、迁移15、旧控制器/批次/原始证据保持冻结。

**Interfaces:** 新 `--scope` 默认 `all`，只接受8个主用例 suffix、`legacy-close`、`all`；非all须 `--suite-id` 规范 UUIDv4，all禁止该参数。capture scope 执行 capture 一次、一个 `--subnet`；其他主 scope 执行 capture+指定主、两个 `--subnet`；legacy scope 一个 `--subnet` 加必需 `--legacy-close-subnet`；其余组合在创建 batch/resources 前拒绝。all保持原八案例和可选legacy调用语义。每 leaf 的 `suite_leaf` 必须与固定调度及实际案例顺序相等，元数据不能代替证明。

- [x] 先 RED/GREEN 固定调度、非法/重复scope、suite UUID、子网/legacy组合、失败prelude不得进入主用例；保持55个名称/root3内容，独立任务复审。first只用于首位capture，不拆掉实际drain、FS/回归或独立index审查。ledger与preflight数量按真实一/两案例调整，全部final审查保留。
- [x] 一次封存同一源码ZIP/manifest，独立不可变parent plan预定9个scope、stage/batch UUID和互不重叠的17个子网；执行前每leaf重新核对实际容量、路径、项目、网络与镜像。源码/工具pins跨leaf一致，每个新case的control binding和binary按自己的真实构建及最终审查验证，不能跨case复用。
- [x] 新私有控制器对每个leaf完整独立复算既有RPC顺序、namespace、实时proof/generation、denial/drain、构建/原件/迁移、原始libtest、当前FS4/相关19、精确停机/卷/网络/transport及源文件不变。leaf成功状态明确为部分suite；最终独立离线核验要求exact9、8唯一主+legacy，无遗漏/重复/旧失败投影/伪造prelude计数/改字节或重用身份。
- [x] 8192文件、2MiB文件/result、1MiB ledger、512MiB endtree、64文件/2MiB读取组、256case/512batch audit、16generation及全部时钟/清理期限保持不变。父级九次launch共享原78000秒runtime额度，失败或歧义停止后续；离线aggregate限180秒、逐leaf有界读取，不合并无界原始日志。不保证单leaf必然容纳；容量保护触发仍拒绝接受。
- [x] 新独立复审后全新suite顺序运行，所有leaf及最终aggregate实际通过才接受Task2并进入Task3。不复用本次六项通过片段，不自动重放失败suite，不提高上限、不删旧卷/网络/证据。51个新保留卷和17个网络的成本如实记录；完整恢复、CompleteBackup、生产及父阶段保持开放。

## Task 3：交付记录、父任务边界与正常发布

**Files:** 新 `docs/p0c4-source-attempt-lifecycle.md`；README、source binding/admission、C4 verification/父计划与本计划；工作区 root 路线图 source/data/generated artifacts。

**Interfaces:** consumes 真正接受的 Task1/2 source/test/binary/result pins；produces 可审查 runbook/当前进度/正常 feature-branch commit与独立远端 ref/tree 核验。

- [x] 准确记录实际 RED、八门/回归分别计数、输入/输出 pins、root namespace 与 runtime-held pin scope、terminal ambiguity/取消和 retained evidence、真实捕获与钩子的区别；修正父计划上轮绑定“待发布”旧文字并保留历史来源。
- [x] 独立任务/整体源码与实际返回/文档复审，精确 public allowlist commit；沿用已授权 GitHub `feat/p0c4-backup-recovery` 正常 push、独立 ref/commit/tree readback，不 force/merge main。私有证据、包、credentials/caches 不上传 Git。
- [x] 路线图 deterministic14outputs、6SVG/本地链接静态 QA；浏览器验收分列。父 Task3 三框、Task4/5、独立存储/CompleteBackup/完整恢复/C4/生产保持开放；资产/local-pin 持久身份和独立保护 discovery 仍是后续整链条件。


发布进度：

生命周期源码已正常发布至 `feat/p0c4-backup-recovery`：[源码提交 `e51ff31c96b446f9b25c854d43c769182b663fe1`](https://github.com/MansurnamedZhang/learning-system-p0c4/commit/e51ff31c96b446f9b25c854d43c769182b663fe1)，tree `c98b730e51926596fe5ac21ac2fffbb00cea92c0`、唯一 parent `e43bb2cef380adda3bf9486005e58a175237d247`。独立 GitHub ref/commit/tree 核验回执 `publication-source-remote-verification.json` 为479 bytes、SHA-256 `e6f3cd2326073010c0c65dc8b30d4ce41aa887404608deaf3823360f667c02e3`；`refs/heads/feat/p0c4-backup-recovery` 与该 commit 一致，477个blob的路径/mode/OID与本地匹配，truncated=false。现场执行仍为e43基线的475文件working-tree-green ZIP `85211a1fead1cd903b5ee269230b02ee6dbfc31ce2808ea31ec04aa5e4d0a466`，不反向写成该发布commit运行。当前后续修改仅为该源码发布的记账，不增加验收范围，不改变原证据，不关闭父Task3/C4；本段只记录上述已核验的源码发布，不自引用本段所在的记账提交。

独立限定文档修补复审 Spec/Quality PASS、P0/P1/P2/P3均0（4028 bytes，SHA-256 `01a9b0d7b892cad05941610186d08512779c27fad9fc2896e8b50dc7fde437b2`）；最终整源码复审 Spec/Quality PASS、Critical/Important/Minor均0（10584 bytes，SHA-256 `2357d8e2b279d8dbc7273e14a9507d71b968b8c8d073e20725d3099bfa5b21fc`）。其后的实际源码push与远端readback使用上列独立回执，不把审查readiness当作发布证明。

root最终路线图静态QA已完成：14/14输出两次hash一致、精确6 SVG及56 HTML链接核对通过，回执 `roadmap-static-qa-source-publication.json` 为2120 bytes、SHA-256 `d319a0da6b979141430a6d543991b65f463530a9544dc502c27c4d1345c0d397`。浏览器file协议被策略拒绝，浏览器渲染和交互**未验证**；该项只有static完成与browser结论分列，不代表browser PASS。

上述child发布和路线图框已勾选；route框仅表示static QA完成且browser未验证已分列。本次不把legacy补充原始私有字节上传Git，完整接受handoff仍须携带aggregate+实际返回review+supplement receipt及六原始文件。父Task3三框、Task4/5与C4继续开放。
