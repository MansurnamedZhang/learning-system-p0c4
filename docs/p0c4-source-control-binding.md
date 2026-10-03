# P0-C4 源控制根持久绑定

更新：2026-10-04。新批次 `27bdfd1d-3372-47d7-949b-e6c9a3bce162` 实际通过 **4/4** 个 Linux/PG18 绑定门，另有 **2 个独立 live admission 回归**各通过一次。控制端已读取实际结果、日志与资源证明；独立限定返回审查的 Spec/Quality/Acceptance 均 PASS，P0/P1/P2/P3 均为 0；仅接受本四门与分别标识的回归证据。独立文档复审及限定修补复审已通过；源码已正常发布至 `feat/p0c4-backup-recovery`，GitHub ref/commit/tree 已独立核验：[源码提交 `e3afb0545463ea8924df30ee340e586d9edf9cc3`](https://github.com/MansurnamedZhang/learning-system-p0c4/commit/e3afb0545463ea8924df30ee340e586d9edf9cc3)，tree `fe3dc0c57b23c65f3fe2d8f0b3c3cc9a1f1d5342`。父 Task3 未完成。

发布收据：上述源码提交的唯一 parent 为 `a56ab16dfd8f8f8c7f1c83b1f3ca40f2506ff934`；实际执行仍是该 base 的 469 文件未提交 working-tree-green ZIP（SHA-256 `95e6c48702dd3557ce7beaa1e3427779e5944315922820ec5071d48ceae8ceae`），不反向写成现场执行 e3 提交。本次后续修改仅作发布回执与复选框的文档记账，不扩充已验收范围，也不改变现场执行身份。

## 身份预期与信任边界

原 live admission 在同一数据库上以固定 session advisory lock 排斥遵守协议的并发维护尝试；原 backend 消失后锁会释放。持久绑定进一步要求捕获和 close-only 恢复核对独立预置的 `source-binding.json`，使复制绑定到另一个控制根或在原路径替换 inode 都不能绕过旧未完成日志。

独立可信签发者先观察真实控制目录和 PostgreSQL 身份，生成规范 JSON，再将其 SHA-256 通过 **构建时** `KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256` 固定进离线二进制。运行时只消费记录，没有 runtime expected pin、配置摘要替代、自动注册或自动 enrollment。未嵌入 pin 的构建可以编译，但相关支持入口必须拒绝。

| 规范 v1 字段 | 约束与来源 |
| --- | --- |
| `format_version` / `capability` | 固定 `1` / `source_control_binding_v1`。 |
| `binding_id` | 独立签发的规范 UUIDv4。 |
| `control_path` | 规范绝对 Linux 路径，不含空段、`.`、`..` 或尾部 `/`。 |
| `control_dev` / `control_ino` | 实际打开控制根的正数 device/inode。 |
| `database` / `database_oid` | 规范 C4 库名与实际数据库 OID；OID 为正 u32 范围。 |
| `system_identifier` | PostgreSQL 集群正 u64 system identifier 的规范十进制字符串。 |

JSON 必须按键排序、紧凑序列化且无额外换行；未知/重复字段、非规范字节、错误版本或身份均拒绝。记录最多 4096 字节，必须是 root 所有的 0600 普通文件、单硬链接；控制根和全部祖先核对 root 所有、祖先无 group/other 写权限，逐段 no-follow 打开。缺失、链接、宽权限、过大或摘要不符的记录不获得维护能力。

**本切片限定一个可信容器挂载命名空间和每部署的独立构建 pin。** 预置、编译及消费采用一致的控制路径/挂载身份。目录 inode 或数据库身份替换后必须重新独立签发并重建；不允许自动接受新身份。独立签发和重建有部署成本，尚未成为宿主生产安装流程。root 祖先与句柄实测证明的是本容器 fixture；特权 root/Docker 管理者任意换挂载、修改记录和重建可信二进制的威胁不由此切片解决。

数据库名、OID 和 system identifier 不能区分保留这些身份的物理克隆端点；**同身份克隆的端点排除仍是独立端点绑定协议的职责**，不把本切片写成克隆端点证明。资产根和 local pin 的身份绑定也须分别设计/验收；它们的可信路径预检不等于这里的源控制根持久绑定。

## 支持入口与恢复语义

`prepare_source_backup` 和 `force_close_release_ready` 保持公开签名，内部先取得同数据库 `SourceAdmission`，在该 admitted 会话核对数据库身份，打开并保留可信控制根句柄。日志扫描、start、recover 和恢复观察均相对这个句柄；核验后路径被换名也不会改到替代目录写日志。旧公开 journal 路径包装与 v1 序列化字节保持兼容。

捕获先核对绑定和未完成日志，再做同会话只读角色/预备事务检查，之后保留原隔离/端点证明及紧邻第一次维护变更前的角色/预备事务复核。绑定、未完成日志和真实 prepared-work 的权威拒绝不能被缺失 attestation 掩盖；尚未核验时不得改 ACL、创建尝试日志或运行 dump。

匹配原根的恢复仅重新 REVOKE runtime CONNECT、记录 close observation，并保留 `ReleaseReady` 和原日志。它不自动完成、放弃或重启捕获。显式广义 finish/abandon、全量捕获/封存恢复仍是父 Task3 下一项。

## 实际 RED 与当前四门

旧 `41f658ea-78a1-44b1-bcd8-3c4a226ce72f` 首批严格 Linux Clippy 因 `option_env_unwrap` 退出 101，PG 行为测试未执行；这是基础设施失败，不能算 RED。原失败与隔离证据保留，不重放。

新 `b2a63c86-2979-4f1d-a54a-4599b3945b21` 在格式/严格 Clippy 通过后实际触发 ACL 保存断言失败：精确测试 exit 101、0 通过/1 失败/0 忽略；`original_backend_absent=true lock_free=true recovery_ok=true connect_before=true connect_after=false`。基线确实在旧会话消失后恢复替代根并撤销 CONNECT，因而是行为 RED。其 canonical result SHA-256 为 `6f1557ddc242d52d3011c2f244d43296a666a91668ba56659c937370a92bdd5b`；raw stdout SHA-256 为 `5c754d1e1532398e6a2620414ced4c7ffdee752fefa8aada0821650e0a1089b2`。

当前 GREEN 的四个精确名称均位于 `source::binding_tests`。每项用新 PG18 项目、PG/source-control 卷、内部 /24 网络和独立预置/编译 pin；**各 exit 0、1 通过/0 失败/0 忽略**，不是列举或默认 ignored 跳过。

| 精确测试 | 本批实际验证 |
| --- | --- |
| `alternate_root_after_session_loss_preserves_acl_and_journal` | 原 backend 已消失、锁已空闲且 CONNECT 曾重开后，复制相同绑定字节的替代根仍拒绝；ACL 和 A/B 日志目录/字节不变。 |
| `original_bound_root_keeps_unfinished_journal_authoritative` | 原根未完成日志仍拒绝新尝试，替代根捕获因绑定拒绝。本门还实际创建 UUID 所有的 prepared transaction，证明 origin 消失、拒绝前后 count=1、公共捕获返回精确 prepared 拒绝且 ACL/目录不变；仅 rollback 此夹具 GID 后确认零 prepared。它属于本门，不另增计数。 |
| `matching_bound_recovery_recloses_without_finishing` | 匹配绑定的原根实际重新关 CONNECT，原绑定与日志不变、仍为 ReleaseReady，另留 close observation。 |
| `another_database_is_rejected_before_source_mutation` | 同服务器另一真实数据库身份不符时拒绝，ACL 和源根/local-pin inventory 不变。 |

以上四门均 **NO_DUMP**；日志前序 dump/pin phase digests 为 **SYNTHETIC**，实际绑定字节、SQL 身份、ACL 和 backend 观察不是合成摘要。

两项 `source::admission_tests` 在本候选另行真实执行，**各 exit 0、1 通过/0 失败/0 忽略**：`same_database_attempts_share_admission_and_other_database_is_independent` 与 `single_connection_catalog_and_drain_share_admitted_backend`。它们证明 live 同库锁/另一库独立准入，以及同 PID catalog/排空、真实额外事务被拒绝；不等于完整 public 捕获、runtime-login 拒绝、dump 或 pin 发布链。

## 当前冻结字节与运行收据

执行输入是基于 `a56ab16dfd8f8f8c7f1c83b1f3ca40f2506ff934` 的未提交 **working-tree-green** 快照，共 469 个 public source 文件；base 只是包基线，后续提交不反向成为现场执行时的源码身份。ZIP 内创建时的 plan/文档是历史快照，本轮后续文档修改不改原 ZIP。

| 输入或输出 | SHA-256 |
| --- | --- |
| ZIP | `95e6c48702dd3557ce7beaa1e3427779e5944315922820ec5071d48ceae8ceae` |
| manifest | `a48dc4ccf4455e69dd92398cc480a9528a9568927fc0be5a3f3a93c0f4b9f0f5` |
| binding driver | `9f5dea29b6d80ef28567c1953f6b60e2e861ca4765acba86c7b963b51659d603` |
| 固定旧 admission helper | `86c59ad4f29eb35ce2cbf6af3fec50fabe981a56a27bfbf157279ba389fe2920` |
| canonical result | `28a1f798b49e4dbc80cce9f92e46374653a3b1fe9b45d0ad720083f3b4e4156f` |
| source aggregate，执行前后一致 | `5b13c9a9ce7891aa5e6bec8a75cbfda8b71e7a0378ab4d05b46d7869b3588203` |

冻结 Rust 原始字节与独立 fix1 review pins 一致：

| 相对源码路径 | 字节数 | SHA-256 |
| --- | ---: | --- |
| `crates/learning-backup/src/maintenance.rs` | 13821 | `397349faaa46b4fc57a085933dee22f65e4b7e85b5caed28508bf43ba79d625f` |
| `crates/learning-backup/src/source.rs` | 55176 | `f32ff8e84e56c06e77dfa0cdb9f0358ef9727c4d2ead9eabfad308439d2fdf95` |
| `crates/learning-backup/src/source/binding.rs` | 21484 | `dac3b490a6ab06d726cea1b8f5a90979143691b477d564b13e9a79448d51f7c7` |
| `crates/learning-backup/src/source/binding_tests.rs` | 25997 | `bf19486af8e5446ea870371dd02e42bef9ceb02555e67c74554b52bb9bcc9d18` |
| `crates/learning-backup/tests/maintenance_pg.rs` | 14427 | `b6c5ac6e1c3e404add5339b5c3dd67a0c39516c0ab31144d1ef49630eb1547ee` |

| 门 | 实际 binding SHA-256 | 实际 binary SHA-256 |
| --- | --- | --- |
| 1 | `e8db189b00be20ef58dec2af5ec24398e80cc3d7a2bc506bc43751872fff9029` | `f8d4d7cf5ff378d815193f524b48002470621eb29cb643194227a309c0fc1130` |
| 2 | `be99042088c4a2a88dfa773ee4de6b569c63dc87699bc93152b77f0e5ac0aace` | `7a0aaece5f033bf6eed318e2a8e3fc49bc3098dceb0041fdea3faefa93fb5d6c` |
| 3 | `b476f9625f19ffcf24869f6641ed3450df2182446927a12d48346de72f10d2b9` | `66e9d1f3b5f59e2148c2fbee78109abd1377b5165be7a71719d977fe2c571168` |
| 4 | `af5de46b57d54423ab9aae69568df615358c73bcfab4106d56acf740450519d0` | `6b3025d560ec7703d25b26e1c98d5ea4169d019bfa7325f11e40660f246a2217` |

三个 audit 时点确认每门 binding/binary 一致，source aggregate 前后不变。控制端证明四个精确 PG ID 均停止、命名卷与内部空网络保留、临时 helpers 移除、pending 不存在；本页作者只读本地回传和控制端 attestation，没有再次远端执行或重放旧批次。独立源/驱动修补复审均记录原 P2/P1 已解决、新发现 0；那是静态代码结论；实际返回另经独立限定审查 Spec/Quality/Acceptance PASS、新发现 0。审查员重新只读 SSH 核对 canonical result、469 个源码、六份精确三行实际 stdout/空 stderr、root `0:0` 调用、38 helpers 不存在与四 PG 停止/留卷/空网/pending 缺失，并复现 byte-identical attestation。限定返回审查报告 SHA-256 为 `674b0a33cd4ae66144d784d7884b5032dc2e7881b6796c320d1bf597661607ad`。

## 分开记录的本地与 Linux 检查

本批 Linux 格式和严格 package all-targets Clippy 均 exit 0；`learning-backup --lib` **158 通过/0 失败/30 忽略**，maintenance contract **4 通过**、journal **1 通过**。四个文件系统测试体实际运行：缺失/链接/宽权限/过大记录拒绝；祖先与路径别名拒绝；复制记录/原路径 inode 替换拒绝；换名后保留 scan/start/recover authority。控制端核对 root 用户、可写可信 `/target` helper 元数据与实际 unit log（SHA-256 `d3c69d0d334cf35c0bcde088ceefa59d1b5dbe445b8cebb2690ab02a66fd84cb`），并非离开 fixture 后的 early-return 假通过。这些不增加 PG 四门计数。

Windows `cargo test --locked --offline --workspace --lib` **185 通过**（assets 5、backup 144、core 36，另外两库 0），选定免环境集成合同 **28 通过**，格式/严格 Clippy 通过。早先完整 `cargo test -p learning-backup --locked --offline` 在 144 个库测试后，于既有 `catalog_pg` 因专用数据库名环境缺失（`NotPresent`）失败，shell exit 1；该失败保留，不写成完整 package 或完整数据库测试通过。Windows 不证明 Linux no-follow；workspace `--lib` 不等于全工作区 DB/升级回归。

Python 全 scripts 回归经进程内精确 worktree Git trust 后 **331 总数、324 通过、7 平台跳过、exit 0**，没有改全局 Git 配置。此前 328 总数中的既有裸 Git ownership setup error 仍保留。Python/POSIX driver 检查与真实 PG 门分别计数，不提供数据库或恢复证明。

## 历史与下一门

2026-10-03 live admission 的五门 **5/5** 是旧 `0adab2e` working-tree-green 快照、随后 `a56ab16` 公开源基线的历史证据；不是当前候选五门重跑。旧 `c9f1cdaf` 维护三门 **3/3** 属于 `07eaf416`，不与四个新绑定门或两项回归合并。历史原始哈希、失败及封存记录完整保留，见[源端维护准入](p0c4-source-admission.md)和[验证台账](p0c4-verification.md)。

保留的 legacy full-capture/drain/release/prepared-driver fixture 已改为消费独立签发/编译 pin；旧 driver/build 管线须更新后重新独立验收，**明确延后**，本候选不宣称这些管线执行通过。当前 prepared 安全拒绝只由上述第二门提供实际支持。

父 Task3 三项复选框、Task4/5、完整备份/恢复、`CompleteBackup`、C4 与生产继续开放。下一项是显式 finish/abandon 和真实 `pg_dump`、全 ready assets/index、保留保护与完成收据的组合链；独立存储故障域按用户安排延后。没有新的 full dump、restore、CompleteBackup 或独立存储验收。本页对应[绑定子计划](superpowers/plans/2026-10-04-p0c4-source-control-binding.md)，父任务见[五任务计划](superpowers/plans/2026-09-28-p0c4-backup-recovery.md)。
