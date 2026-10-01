# P0-C4 受控小型 dump 导入 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在全新 Linux/PG18 隔离目标验证固定 fixture 的首次写入、唯一提交、未提交EOF回滚及失败隔离，不开放产品恢复入口。

**Architecture:** 固定 `pg_restore` 从冻结的 custom dump 解码，完整 golden 校验后，由同一个固定 `psql` writer 显式 BEGIN、验证原 SQLx 双锁、等待持久 attempt，再执行 DDL/COPY。单一监督任务持有全部 guards 和 stdin；提交意图持久化且最终复验后才能单独发送 COMMIT，成功必须读回、停机、持久证据全部完成。

**Tech Stack:** Rust 1.97/edition 2024、Tokio、SQLx 0.8.6、Python unittest、固定 PostgreSQL 18.6 Docker 镜像。沿用现有依赖，不新增服务、迁移或公开 feature。

**Spec:** [已获批准的首次写入设计](../specs/2026-09-30-p0c4-controlled-import-design.md)。开始编码前执行者必须同时阅读本计划与规格。

**Status:** 用户于2026-09-30以“开始”批准本施工单。Task1已完成本地实现/修复复审、真实PG18开放stdin合同、合成工件采集/导出/下载哈希核对和独立实产审查，现场状态为 `CLIENT_CONTRACT_AND_FIXTURE_CAPTURE_PASSED_NOT_IMPORT`；旧失败批次全部保留。Task2提交 `b8fb7be80dd65eec5e814770b2fc11d7d375c50b` 已完成dump冻结、来源于实产的完整SQL模板、变长key边界和四种SET LOCAL转换；定向7/7、包内库66/66、格式及严格package Clippy通过，独立规格/质量审查Approved。上述Task2结果仅为Windows纯模型；后续整合必须绑定已审查dump摘要/TOC、实际no-follow输入及固定PG客户端。Task3提交 `f815b6e62bc78abe45b0610876ce4926e5346835` 已完成候选attempt/提交意图、两道sync屏障及同目录/writer/初始SHA绑定；定向4/4、包内库70/70、格式及严格Clippy通过，独立规格/质量Approved。Task3真实Linux no-follow/fsync门尚未编译/执行，留给另授权批次。Task4实现 `1e4dbda`、修复 `986137b` 已完成本地管道/取消状态机及独立审查：最终管道15/15、状态机3/3、相邻监督器5/5、格式/严格Clippy通过；修复前包内库83/83，修复后未重复全包。独立审查发现的过期finish竞态和继承管道清理阻塞两项Important已修复并复审全部ADDRESSED，未发现新增阻断。Task5实现 `fae1e56`、修复 `fbdcd1a`/`cfea8e5` 已完成私有同writer准入、持久屏障、首次提交尝试与清理整合，四项Important经两轮独立复审全部ADDRESSED。本地组合61/61后仅强化两项测试断言，定向2/2；后续交接修复定向4/4及严格Clippy通过，未重复全包。Linux组合尚未编译，真实PG18谓词与可信来源签发T5-PROVENANCE-01仍待Task6。Task6开始静态实现；尚未执行目标导入。既有完整Python回归231/232，M3计时断言原因未证实，保留给最终整分支审查，不能称完整回归通过。执行基线 `dee75734a36b77babe7d049f9819281b1301d645`、设计基线 `cd2dc96046ff4d03fb33cc38c00f138803727bbd`；沿用逐任务子代理实现及独立审查，新服务器文件/资源分别授权。验收边界见[合同与工件采集说明](../../p0c4-controlled-import-contract-acceptance.md)及[验证记录](../../p0c4-verification.md)。

## Global Constraints

**Task6 当前状态（2026-10-01）：** 十一项 ignored 源码、固定可信源签发与 phase-import runner 的本地实现及独立任务审查完成；整计划首次审查所提三项 Important 与一项输出问题由单次修复提交 `1d5a7b2baca4789dac4ac5d9ac2cf77b57e010cd` 解决，独立修复复审全部 ADDRESSED、无新增问题。Python 一次共享修改后完整回归258/258，随后窄修正以最终26/26定向覆盖；Windows journal4/4和格式检查通过。Linux no-run 与 11 项 ignored 名称列举已在批次 `a3f5719a` 通过，用户终端退出0。随后无PG前置批次 `b2af0b1a` 已执行并在格式／严格Clippy／编译组合阶段失败，退出1、通用原因码Io，具体步骤及根因待只读日志诊断；默认库测试、journal no-follow/fsync与实际PG18用例尚未运行，实际用例0/11。独立SSH核对两批builder名称及标签无残留；root私有完整记录尚未直接读取。只读诊断的16项解析检查及独立修复复审通过，上传及双次哈希核验完成，等待用户Linux终端sudo认证读取旧日志。原22/22前置脚本模型、封存源码和失败证据保留，不重跑旧批次；本施工单复选项不因编译或静态源码而关闭。早期状态段中的M3待办和计数属于历史记录，当前关闭边界以本段为准。后续自主执行范围、资源预算及逐case证据规则见 [受控导入验收说明](../../p0c4-controlled-import-acceptance.md)。

- 工作目录：`D:/codex/DeepLearning/.worktrees/knowweave-p0c4`；保留现有分支，不新建外部项目，不往C盘安装/生成依赖或测试数据。
- 本地执行者在任务1设置 `CARGO_TARGET_DIR` 为该工作树的 `target/`，`TMP`/`TEMP` 为新建的 `.runtime/p0c4-controlled-import-tests/`；验证后恢复原环境。只读取已有工具/依赖缓存，不安装新运行时。Linux临时文件放本批root私有tmp目录。
- 恢复算法和候选入口仅私有测试代码；实际执行为 `#[cfg(all(test, target_os = "linux"))]`、opt-in ignored。纯模型可 `cfg(test)` 在Windows验证。默认/发布构建无新写入口，`preflight_restore(&CompleteBackup, ...)` 原门槛不变。
- 不构造假CompleteBackup、witness或receipt，不签发构建pin，不恢复资产/任务，不放行服务；任意dump及完整C4/生产不在本计划验收范围。
- 固定 PG 镜像：`postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d`；沿用已核验builder ID `sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b`，执行前复核。
- 固定精确64hex容器ID、`/usr/bin/docker exec --interactive --user 999:999`、`/usr/bin/env -i`、`LC_ALL=C`、`PGCONNECT_TIMEOUT=10`、`PGPASSFILE=/dev/null/knowweave-c4-passfile-disabled`、绝对PG18客户端路径。无shell/TTY/caller flags；目标固定本地socket、5432、learning_admin与出生UUID库。
- dump快照和解码SQL各最多64 KiB；writer stdout/stderr各8 KiB，读取期间执行上限。解码15秒、writer45秒、READY/PRECOMMIT各10秒且不延长总截止；backend消失观察5秒，隔离确认另15秒。
- SQL固定 `SET LOCAL`：statement_timeout=10秒、lock_timeout=5秒、idle_in_transaction_session_timeout=30秒、transaction_timeout=60秒；不改变控制事务/角色/库设置。
- fixture仅 `public.c4_import_probe(id integer NOT NULL,label text NOT NULL)`、固定主键、两行 `(1,'alpha')`/`(2,'beta')`。完整golden核验；只有匹配的随机restrict key占位允许变化，不支持通用SQL清洗。
- 用户于2026-10-01明确要求“无需要我逐项确认，自己去做”，替代此前逐包/逐新资源人工确认要求。批准范围内由执行者自主封存、核验并记录每个源码包/原样runner的哈希和每个新UUID/子网/卷；运行前核对资源未占用，仍使用全新隔离批次。sudo凭据只在用户Linux终端输入；旧批次、卷、证据与生产不碰，失败批次不重跑。
- 每任务RED→GREEN→定向验证→提交→独立规格/质量审查，审查问题解决后才开始下一任务。任务1的现场协议/golden门未过，任务2不得猜测封存golden，任务5不得启动写入。

## Review Focus

1. stdin开放时READY/PRECOMMIT刷出、EOF不自动COMMIT：任务1真实客户端与任务4/6真实管道和取消测试覆盖。
2. 校验后同inode内容变化、header归零超时、restrict被提前解除：任务2完整快照/golden与固定边界拒绝测试覆盖。
3. fsync迟到、取消与COMMIT竞态、部分COMMIT写入：任务3/4/5必须证明先持久提交意图、单一stdin所有者与未知提交状态。
4. SQLx backend与writer不同、PID重用、同PG标识的错误socket端点：任务5前后断言及任务6克隆writer负例覆盖；旧只读通过不能替代。
5. stop后才取证、kill CLI但容器子进程残留、默认.pgpass回退：任务1认证、任务4/5失败监督和任务6证据顺序覆盖，不宽泛忽略stderr。

## 文件与接口约定

父模块 `crates/learning-backup/src/restore_preflight/target_binding.rs` 只加 `#[cfg(test)] mod controlled_import;` 及必要的窄测试可见性。新文件集中在 `crates/learning-backup/src/restore_preflight/target_binding/controlled_import/`，入口为相邻 `controlled_import.rs`。不搬迁现有整套guard或只读执行器。

共用私有错误类型 `ImportFailure` 固定枚举：Identity、Session、Version、Protocol、Fixture、InputLimit、Deadline、StdoutLimit、StderrLimit、Stderr、Exit、Io、Journal、Cancelled、CommitUnknown、UnconfirmedIsolation。输出只序列化固定码，不保存原始异常/SQL/stderr/锁值。

任务1在 `protocol.rs` 定义 `Nonce([u8;16])`、`WriterIdentity { backend_pid:i32, backend_start_micros:i64, transaction_id:u64 }` 与 `WriterExpected { database:String, database_oid:u64, system_identifier:u64, control_pid:i32, keys:ChallengeKeys, nonce:Nonce }`，字段私有。nonce由ring随机产生，编码为32位小写hex；后台时间用固定SQL转epoch微秒，避免时区文本变化。任务5从原guard/challenge构造期望值；既有DockerClaim.system_identifier是String，必须严格解析为正整数并核对canonical decimal，不能原样插入SQL。不接受调用方SQL或连接参数。

以下所有接口仅在私有测试模块可见；它们是任务间合同，不是公开Rust API。新增支持类型在所属任务内定义，禁止跨任务凭空引用未定义类型。

## Task 1 — 固定客户端合同、开放stdin协议与合成工件采集

**Files:** 创建 `controlled_import.rs`、`controlled_import/commands.rs`、`controlled_import/protocol.rs`；创建 `scripts/p0c4_controlled_import_acceptance.py`、`scripts/p0c4_import_fixture.py` 及各自同名 `test_*.py`；修改父模块声明。Python先只支持 `--phase contract`。

**Interfaces:** `FixedImportCommand::decoder(container_id:&str) -> Result<Self,ImportFailure>`、`::writer(container_id:&str,database:&str) -> Result<Self,ImportFailure>`，`argv(&self)->&[String]`；这些只是不可执行的参数模型。`parse_writer_line(line:&[u8],expected_nonce:&Nonce)->Result<WriterEvent,ImportFailure>`，WriterEvent定义为 Ready(WriterIdentity)/Precommit(WriterIdentity)/Committed/RolledBack；COMMIT后不要求旧事务ID继续存在。`run_contract(args)->dict`、`capture_fixture(source_id:str,database:str)->dict` 为root runner内部函数，拒绝未验证ID/库名。

- [x] 写RED：`commands_reject_alias_and_conninfo`、`commands_fix_environment_and_clients`；断言decoder无连接/-1选项，writer无-1、含pager=off和固定passfile。Python `test_contract_does_not_close_stdin_to_obtain_ready`、`test_contract_rejects_replayed_names`、`test_contract_redacts_failures_and_publishes_after_stop` 覆盖严格握手与资源/证据边界。
- [x] 运行 `cargo test --offline --locked -p learning-backup --lib controlled_import::commands` 和 `python -m unittest discover -s scripts -p 'test_p0c4_*import*.py'`，确认预期RED；命令不存在不是行为RED，记录编译缺接口与行为失败的区别。
- [x] 实现参数/回执模型和contract runner：固定decoder选项照规格，writer精确argv；严格nonce/PID/事务/阶段行。增量并行有界读取管道，不使用无界`.output()`/communicate后才限长。复用既有provisioner/出生/inspection及archive核验的窄helper，不扩展旧大型runner全部模式。
- [x] 本地上述GREEN、格式、Python邻近回归通过，提交并独立审查；再封存Git同字节包与runner，单独申请新源/目标项目授权。contract阶段在新源生成fixture及dump/SQL，目标只做BEGIN READ ONLY、双握手、ROLLBACK与无DDL EOF；源另在显式事务内插入第三行后真EOF，独立读回仍只有两行，用于证明客户端EOF回滚能力，不把目标只读空库当DDL回滚证据。
- [x] 新PG18实测/dev/null字符设备、禁用passfile路径无stderr与local HBA事实；每个握手保持stdin开放且子进程存活。源和目标按精确ID停机留卷。捕获物仅本轮已知合成fixture，原始dump/SQL保存在单列工件目录，不混入诊断日志；授权范围明确其受控导出。controller核对工件hash后独立审查，不自动将输出认作golden。成功只标 `CLIENT_CONTRACT_AND_FIXTURE_CAPTURE_PASSED_NOT_IMPORT`，记完整证据后关闭任务1。

## Task 2 — 冻结dump、完整golden与header/payload边界

**Files:** 创建 `controlled_import/fixture_sql.rs`、`crates/learning-backup/tests/fixtures/c4-controlled-import/pg18-fixture.sql.in` 和 `capture-contract.json`；测试同模块。最后两个文件只能由任务1已核验合成工件形成，不凭文档猜测版本header。

**Interfaces:** `freeze_dump(reader:impl Read,expected_len:u64,expected_sha256:[u8;32])->Result<FrozenDump,ImportFailure>`；FrozenDump私有不可变字节/摘要，无路径重开，`bytes(&self)->&[u8]`、`sha256(&self)->[u8;32]`供内部解码和journal使用。`verify_fixture_sql(decoded:&[u8])->Result<VerifiedFixtureSql,ImportFailure>`；返回私有header/payload、原始及转换后SQL摘要，唯一构造器完成整份匹配后按固定偏移分段。`header(&self)->&[u8]`、`payload(&self)->&[u8]`、`raw_sha256(&self)->[u8;32]`、`transformed_sha256(&self)->[u8;32]` 供任务3/5消费。

- [x] 写RED：`snapshot_stays_fixed_after_same_inode_rewrite`（读后改源不影响快照）、`fixture_budget_accepts_65536_rejects_65537`、`golden_rejects_transaction_reconnect_lo_and_early_unrestrict`、`header_is_ddl_free_and_timeouts_are_local`。断言任何非key字节变化、key不匹配/多次、CRLF或注释变化均拒绝，payload含固定DDL/COPY且无COMMIT。
- [x] 运行 `cargo test --offline --locked -p learning-backup --lib controlled_import::fixture_sql`，记录预期RED。
- [x] 实现magic/长度/SHA/读取中预算和不可变快照。完整模板仅匹配一对1..128位ASCII字母数字restrict key，实产key长度也记入capture合同；只在已审定完整header语句固定替换四种SET LOCAL超时。拒绝未知TOC/输出，不靠关键词黑名单放行任意SQL。
- [x] 重跑定向GREEN与 `cargo fmt --all -- --check`、`cargo clippy --offline --locked -p learning-backup --all-targets -- -D warnings`。Windows只证明纯模型；Linux-only编译另由授权builder核验。
- [x] 提交fixture来源/镜像/捕获与模板SHA，独立审查字节边界和变体拒绝；不可变工件不含密码、业务数据或真实服务配置。T2-LIMIT-01：运行时调用者对dump/TOC来源及真实解码的绑定仍由后续整合/现场门验证。

## Task 3 — 单次attempt与提交意图持久屏障

**Files:** 创建 `controlled_import/candidate_attempt.rs`。复用父层 `restore_attempt_name` 与任意既存条目拒绝规则；不修改production receipt语义。

**Interfaces:** `CandidateAttemptContext { batch_id:Uuid, database:String, birth_sha256:[u8;32], inspection_sha256:[u8;32], dump_sha256:[u8;32], raw_sql_sha256:[u8;32], transformed_sql_sha256:[u8;32], fixture_version:u32, writer:WriterIdentity }`，字段私有且fixture_version固定1；`persist_attempt(dir:&BackupDir,context:&CandidateAttemptContext)->Result<DurableAttempt,ImportFailure>`、`persist_commit_intent(dir:&BackupDir,attempt:&DurableAttempt,writer:&WriterIdentity)->Result<DurableCommitIntent,ImportFailure>`。两个返回类型非Clone、字段私有；构造只在文件和父目录sync成功后返回。初始文件 `<出生库>.restore.attempt`，提交意图 `<出生库>.restore.commit-attempt`，后者绑定前者文件SHA。

- [x] 写RED：`journal_rejects_existing_regular_symlink_and_partial_entry`、`file_sync_failure_returns_no_durable_permit`、`directory_sync_failure_preserves_entry`、`commit_intent_binds_attempt_and_writer`。断言失败不删除、不覆盖，不出现伪receipt_sha256，首次持久permit前不得发送DDL。
- [x] 运行 `cargo test --offline --locked -p learning-backup --lib controlled_import::candidate_attempt`，记录预期RED。
- [x] 实现私有候选版本/type、root私有create_new/no-follow写入、文件及父目录fsync。真实Linux路径沿用BackupDir，不以普通Path重新打开已核验目录；可注入仅测试的sync故障。late完成只产生持久记录，不自行发送payload/COMMIT。
- [x] 定向GREEN、格式/严格package Clippy通过；Linux ignored `live_candidate_journal_no_follow_and_fsync` 已添加，Windows未编译/未运行；真实权限/同步/保留仍待后续另授权新无网络私有临时目录，不碰已有target。本勾选仅表示本地实现与检查完成。
- [x] 提交并独立审查。任意文件创建/同步不确定均保留并阻止目标重用；DurableCommitIntent仅是尝试证据，不代表已提交。

## Task 4 — 流式监督器、单一stdin所有权与取消

**Files:** 创建 `controlled_import/stream.rs`、`controlled_import/state.rs`；现有只读 `child_attestation/bounded_process.rs` 保持原语义，仅确有重复且可不变复用时做窄提取。

**Interfaces:** `ImportPhase` 对应规格单向状态；`ImportMachine::accept(&mut self,event:ImportEvent)->Result<ImportAction,ImportFailure>` 只管理允许SendHeader/SendPayload/AttemptCommit/Abort/Finish动作。`spawn_stream(command:tokio::process::Command,budget:StreamBudget)->Result<StreamOwner,ImportFailure>`；StreamOwner独占Child/stdin、增量stdout/stderr与wait，`send(&mut self,bytes:&[u8])`、`next_line(&mut self,deadline:Instant)`、`close_input(&mut self)`、`kill_and_wait(&mut self)` 均异步返回Result。它不自行写COMMIT，也不暴露stdin句柄或Clone。

StreamBudget在本任务定义为固定的deadline/stdout_cap/stderr_cap；decoder与writer按Global Constraints构造，不能传入宽松用户预算。上述send/close/kill返回 `Result<(),ImportFailure>`，next_line返回 `Result<Option<Vec<u8>>,ImportFailure>`。spawn_stream仅创建宿主进程和返回受控所有者，不能启动另一个可独立发送stdin的后台任务；任务5监督任务是唯一发送者。下一阶段回执不提前通过，未知空行只允许任务2golden注明的header位置/次数。

本节勾选表示Windows本地实现/验证与独立审查完成；Linux Docker/PG、真实提交和容器隔离仍属于Task5/6现场门。为兑现成功退出/排空要求，StreamOwner增加窄私有`finish`，沿用同一Child、截止和stdin所有权。

- [x] 写RED：`ready_arrives_while_input_is_open`、`stdout_and_stderr_limits_apply_while_running`、`blocked_stdin_obeys_total_deadline`；用受控自测试进程验证真实IO。模型断言：

```rust
// cancel被接受后，迟到的sync成功事件不能放行任何输入。
assert_eq!(cancelled.accept(ImportEvent::AttemptSynced), Err(ImportFailure::Cancelled));
// 不持久提交意图，不能发出AttemptCommit动作。
assert!(precommit.accept(ImportEvent::CommitPermitRequested).is_err());
```

- [x] 运行 `cargo test --offline --locked -p learning-backup --lib controlled_import::stream` 和同目录 `controlled_import::state`，记录预期RED。
- [x] 实现读取中预算、背压/分阶段截止、严格增量协议；事件类型定义AttemptSynced、CommitIntentSynced、CommitPermitRequested、Cancelled及其必要身份/回执事件。单owner串行裁决取消/提交，任何COMMIT字节发送前必须处于持久意图后的状态；发送部分失败转CommitUnknown，不能解释成零提交。保证kill/wait后才释放宿主句柄，Drop只是请求取消，外层监督任务持续拥有StreamOwner。
- [x] GREEN须覆盖取消先赢/提交先赢、缺LF/CRLF/伪nonce/额外行、未知空行、stderr非空、部分COMMIT、deadline与宿主进程已回收。定向通过后跑原bounded_process回归、格式/严格package Clippy。
- [x] 提交并独立审查真实背压、输出限制及没有隐藏第二个stdin发送任务；测试只使用自进程，不建立产品DB连接。

## Task 5 — 同writer准入、显式导入和失败隔离整合

**Files:** 创建 `controlled_import/linux.rs`、`controlled_import/writer_sql.rs`；修改 `controlled_import.rs` 组合模块。其他生产恢复路径保持关闭。

**Interfaces:** Linux私有 `async fn admit_candidate_target(config:RestorePreflightConfig,admin:PgPool)->Result<OwnedCandidateAdmission,ImportFailure>`；OwnedCandidateAdmission拥有 `BoundTargetGuard<File,Option<File>>`、`LockChallenge<Transaction<'static,Postgres>>`、原PID/OID/config及已核验birth/inspection，non-Clone。复用 `acquire_for_restore`、`begin_sql_session`、`verify_sql_session` 和现有干净目标事实校验，不复制宽松guard。`start_candidate_import(admission:OwnedCandidateAdmission,dump:FrozenDump)->CandidateImportHandle` 把guards/原事务/管道交给一个监督任务；handle只等待或请求取消，Drop不释放任务持有guards。内部 `async fn run_import(admission:OwnedCandidateAdmission,dump:FrozenDump)->CandidateImportReport` 从固定decoder得到并校验VerifiedFixtureSql，不接受调用方SQL。

本任务定义 `CandidateImportReport { phase:ImportPhase, failure:Option<ImportFailure>, stop_confirmed:bool, content_verified:bool, commit_attempted:bool }`，无原始payload/秘密；handle的 `async fn wait(self)->CandidateImportReport` 在监督与隔离完成后返回，`fn cancel(&self)` 只发取消事件。成功、预期负例与未知结果由任务6按固定case谓词判断，不能仅凭test退出0称真实导入成功。

`writer_sql::prelude(expected:&WriterExpected,sql:&VerifiedFixtureSql)->Result<Vec<u8>,ImportFailure>`、`postcheck(expected:&WriterExpected,writer:&WriterIdentity)->Vec<u8>`、`commit_confirmation(nonce:&Nonce)->Vec<u8>`。prelude为BEGIN+完整已审定header+前置断言/READY；postcheck验同writer/同事务、原双锁、四timeout、表两行/主键并输出PRECOMMIT，无COMMIT。COMMIT仅独立固定字节 `b"COMMIT;\n"`，先发送成功后再发送确认SQL；最早发送尝试即提交未知边界。

- [x] 写RED：`writer_rejects_identity_before_marker`、`writer_requires_original_control_pid_locks_and_distinct_writer`、`no_payload_before_attempt_sync`、`no_commit_before_intent_and_final_recheck`、`cancel_keeps_guards_until_quarantine`；使用依赖注入事件/目标IO核对顺序和固定原因，不能把mock通过当现场通过。
- [x] 运行 `cargo test --offline --locked -p learning-backup --lib controlled_import`，记录新增预期RED及旧门结果。
- [x] 实现原事务观察、root候选准入、同writer PID/backend_start/xid与schema-qualified断言；默认学习角色不能指定任意SQL。已完整匹配后再启动writer；READY精确空行协议与nonce通过、Docker/SQLx复验、attempt持久化后才发送payload。PRECOMMIT复验、intent持久化、最终复验/剩余预算、单owner提交放行按序。
- [x] 实现独立读回/失败状态：observer用新的已绑定只读连接验证原双锁，避免长事务统计缓存；只在writer消失且目标可信仍运行时停机前读回。未知COPY不能发ROLLBACK；COMMIT确认不可靠归CommitUnknown，最终停机不可靠归UnconfirmedIsolation。已取消状态不能被late IO重新放行；旧guard无二次消费。
- [x] GREEN、旧只读/3a/3b模型、格式/严格package Clippy；用 `cargo check --offline --locked -p learning-backup --lib` 及同命令加 `--all-features` 检查正常库构建，确认没有候选公开方法/feature。提交并独立规格/质量审查；整合通过仍不运行实际导入，需任务6授权现场门。

**本地关闭边界（2026-10-01）：** Task5本地实现与独立审查完成，不能据此称Linux编译或实际导入通过。真实可信来源、no-follow/TOC签发、PG18谓词求值、原目录sync与每个写入/负例现场门由Task6在已批准范围内自主封存并使用全新隔离资源验证。前述61/61、后续2/2及4/4分别对应各次修订覆盖，不是最终全包或工作区验收。

## Task 6 — 新项目真实写入、错误端点与证据关闭

**Files:** 创建 `controlled_import/live_tests.rs`；扩展新 `scripts/p0c4_controlled_import_acceptance.py` 的 `--phase import`，配套unittest；创建 `docs/p0c4-controlled-import-acceptance.md`；更新 `docs/p0c4-verification.md` 和本施工单。复用已审查clone helper，不改旧96KB runner模式。

**Interfaces:** runner内部 `run_import_case(args,case:str)->dict`，只接固定case白名单；每次调用必须带另审定的新源/目标UUID、子网、Git/ZIP/runner摘要与固定镜像。ignored测试用结构化非敏感env定位root批次，凭据仅原SQLx控制会话读取，writer仍无passfile。exact测试列表/检查点/源码与二进制SHA、marker及commit-intent阶段、停机留卷均入脱敏结果。

ignored测试的完整前缀固定为 `restore_preflight::target_binding::controlled_import::live_tests::`，runner只接受case对应一项exact测试、实际一项passed/零ignored/退出0及全部检查点。先做磁盘/内存只读预检，申报新源/目标/克隆卷与编译预算；同一封存源码/builder生成的测试二进制可只读共享并逐次复核SHA，PG资源和数据不共享。不同源码/二进制不得混用旧编译结果。

- [ ] 写RED：runner `test_missing_gate1_attestation_prevents_creation`、`test_selects_one_exact_live_test`、`test_negative_is_not_success_without_expected_evidence`、`test_final_result_requires_stop_and_no_pending`、`test_commit_unknown_is_never_reported_as_rollback`。所有正常/负例必须新目标，不接受已有资源；先编译/列举所有Linux ignored测试，不把未运行记通过。
- [ ] 新增以下真实ignored门；每一场景使用独立新批次，取消逐项人工确认也不得复用目标：

| case / 测试名 | 必须观察的结果 |
|---|---|
| `success` / `live_controlled_import_commits_fixture` | 真dump解码、同writer准入、attempt与intent先持久、唯一COMMIT、独立两行/主键/摘要、停机留卷 |
| `ready-eof` / `live_controlled_import_ready_eof` | 真EOF且未先发ROLLBACK/COMMIT；无对象/attempt，writer消失后停机前独立读回 |
| `precommit-eof` / `live_controlled_import_precommit_eof` | 已有未提交DDL/COPY，真EOF后独立零对象、attempt留存、无提交intent |
| `precommit-cancel` / `live_controlled_import_cancel_before_commit` | 接受取消先于提交放行、guards仍在、零对象读回、停机确认 |
| `ready-restart` / `live_controlled_import_restart_before_ddl` | DDL前新重启、固定拒绝、无attempt/DDL、旧guard不可复用；不重启取证 |
| `sql-error` / `live_controlled_import_sql_error` | 仅测试能力中固定SQL错误；禁止COMMIT、失败停机留marker |
| `copy-truncated` / `live_controlled_import_copy_truncated` | 固定COPY截断，禁止COMMIT/盲ROLLBACK；失败停机留marker，读回能完成才附加零提交证据 |
| `attempt-sync-failure` / `live_controlled_import_attempt_sync_failure` | 初始attempt同步故障不发送DDL，保留部分条目/不可用证据 |
| `commit-intent-sync-failure` / `live_controlled_import_commit_intent_sync_failure` | 提交意图同步故障不尝试COMMIT，保留初始/部分意图条目及隔离证据 |
| `commit-unknown` / `live_controlled_import_commit_confirmation_lost` | intent先持久，确认丢失或部分发送判未知，不能宣称回滚/允许重试 |
| `wrong-endpoint` / `live_controlled_import_same_id_wrong_endpoint` | 新主/物理克隆同ID/OID，但实际writer双锁断言拒绝；无对象/attempt，双容器精确停机留卷 |

- [ ] 每个case先跑本地runner断言与真实编译检查，再单独封存Git字节ZIP/manifest/runner并独立核对；由执行者按当前自主授权记录封存哈希，仅运行固定case和新隔离资源。门失败即保留，不在原项目重跑；修订从新包/新UUID开始。
- [ ] controller核对用户root回传完整脱敏JSON、result/inspection摘要和PENDING_ABSENT，并独立只读核对Docker精确ID停机/留卷。合成fixture可公开的工件与root私有日志分别标记来源，不能称已独立读取无法访问的root文件。
- [ ] 整计划独立复审无阻断，定向回归与必要工作区编译/格式/严格Clippy通过；新Linux各必过门证据完整才关闭计划。成功只标 `CONTROLLED_FIXTURE_IMPORT_PASSED_SINGLE_HOST_QUARANTINED_NOT_FULL_RESTORE`；更新路标但C4整关、CompleteBackup与生产仍未验收。提交文档与代码并正常同步既有GitHub分支，不强推/main/合并。

## 计划自检与执行交接

规格覆盖：输入/认证与客户端→任务1/2；marker→任务3；IO/取消所有权→任务4；原事务/实际writer/提交→任务5；真实成功/EOF/重启/克隆/未知与证据→任务6。Review Focus五项均有指定任务测试；接口名字和消费方向逐一对照。

本施工单审阅后，按已选定的子代理方法从任务1本地RED开始。任务1服务器合同采集最初按具体包/runner/新项目单独授权；2026-10-01起在批准范围内自主推进；其门通过并封存实际golden前，不能跳到任务5/6。任务状态以实际代码、审查、测试与证据分别登记，不把“计划批准”记成实现或现场成功。
