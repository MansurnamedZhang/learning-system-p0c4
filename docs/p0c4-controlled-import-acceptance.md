# P0-C4 固定 fixture 导入：静态实现与后续验收

**当前已完成本地静态实现、整计划首次代码审查及一次修复独立复审；三项 Important 和一项输出待办全部 ADDRESSED，未发现修复引入的新问题。Linux 尚未编译，现场导入用例仍为 0/11。没有本轮导入成功证据。** 修复提交 `1d5a7b2baca4789dac4ac5d9ac2cf77b57e010cd`。默认/发布构建没有新写入口，`CompleteBackup`、生产恢复、资产/任务恢复、构建 pin、runtime/Worker 放行及 C4 整关均未验收。

2026-10-01 本地修复验证：共享 stop/close 修复后，P0-C4 Python 一次完整回归 258/258，候选 journal Windows 模型 4/4，格式检查退出0。其后补齐诊断输出管道失败与 builder 清理截止传播，仅作定向验证：32/32，最后加入成功清理合同覆盖后26/26；未重复全量回归，不能称最终所有源码全套通过。修正 Linux ignored journal 测试的构造函数同名变量，不将 Windows 模型当作 Linux 编译证据。历史 M3 的 37/38 失败及清理资源警告保留；确定性测试证明原关闭分支会跳过 reader 清理，新实现保留未完成的精确子进程、reader 和管道所有权。一次原计时测试诊断未重现历史 PID39808 的调度，不宣称已重建其原因。

隔离仍从最外层 stop 入口起使用同一个绝对15秒截止，包含早期克隆发现与已知源/目标停止。克隆发现超时或归属拒绝会保留失败，期限有余且没有未完成 CLI 时继续尝试已知精确 ID；未知克隆不得替换或猜测。截止或 wait 失败后无法证明回收完成时，批次和命令适配器不可用，禁止新 Docker 操作及正常结果发布，保留文件锁。命令行输出一次固定 `CONTROLLED_IMPORT_UNCONFIRMED_UNUSABLE_OWNING_CLEANUP`，同步持有这些资源，只作零等待的精确回收观察，并等待已有 condition 通知；这段失败所有权生命周期可能无界，不是延长隔离预算。所有子进程、reader 和句柄确证完成后才释放锁并退出1，绝不补发成功。不可服务的内核可能令失败进程及锁持续保留直至外部终止宿主；不能声称15秒内隔离已确认。

builder 从实际 run 起另保存该编译操作的原绝对截止；继承的精确 ID 清理配方中每个 Docker 命令均使用此截止、原 argv/env/输出上限。即使宿主 builder CLI 已回收，超过截止也不得发起新的容器发现/移除，cleanup 保持未确认、适配器不可用且不发布成功。此时容器可能仍在，保留批次证据并报告失败；没有未完成宿主句柄不等于容器清理已确认。

## 必须先具备的授权与来源

每个 case 独立批准 Git 同字节 ZIP/manifest、原样 root runner/helper 的具体摘要，以及新的控制/源/目标 UUID、项目、子网和卷；`wrong-endpoint` 另需全新物理克隆 UUID/子网/卷。尚未批准的资源不能从本文推定获准。失败批次不重跑、不清理、不复用；root/sudo 凭据只由用户在 Linux 终端输入。控制者负责封存、上传、完整可见命令、回传结果及独立 Docker 核验。

前置门1为 `CLIENT_CONTRACT_AND_FIXTURE_CAPTURE_PASSED_NOT_IMPORT`，控制批次 `14b45cf0-e9f3-4beb-a250-ceb6f49d5a14`，结果摘要 `505ab6179e3be1f1c8afb441fa694db8d5033f27e77a0940908de208bad96508`。这是已审查历史合同引用，不是每个新 dump 的预期摘要或导入能力。历史 root 原结果、用户终端摘要、可公开合成工件及独立下载/实查证据必须分别注明来源，不能把未读到的 root 私有文件称为已独立读取。

固定 PostgreSQL 镜像：`postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d`。固定 builder：`sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b`；每次实际执行前复核。

## 编译与资源预检

经封存验证创建新的 root 私有批次后，`ImportBackend._budget()` 仅作容量与镜像预检，`ImportBackend._preflight_import_builder()` 是可分离的编译/列举入口：内部使用源和目标均为 64 个 `0` 的无权占位出生 pin，离线 `cargo test --locked --offline -p learning-backup --lib --no-run --message-format=json`，随后宿主执行测试二进制 `--list --ignored`，核对下表 11 项全部且各一次。它不创建 PG、网络、卷，不读取 DB 凭据，不运行 ignored 测试。控制者可在另授权的新 root 批次只调用此方法；不增加宽泛 CLI phase。此方法本身不会代替另需的 Linux 严格 Clippy、库检查和 Task3 no-follow/fsync 门。

实际 case 在全新源/目标出生证明签发后另编译 `probe-live-build`，分别嵌入 `KNOWWEAVE_C4_IMPORT_SOURCE_BIRTH_SHA256` 和原 `KNOWWEAVE_C4_TARGET_BIRTH_SHA256`。源出生使用同一完整验证器（state/issuance-success/无 failure、目录 dev/ino、PG 身份/cast/ACL），不构造第二个目标导入 admission，也不接受环境中的期望 hash 充当能力。源 guard 与原 SQLx challenge 持有到生产和签发完成。预检二进制不能执行实际 case；不同源码或编译 pin 不复用二进制。每次列举/执行前后复核只读二进制 SHA，记录两份 pin、源码 SHA、builder ID、列举摘要和 exact 名称。

builder 固定 `--cpus=4 --memory=8g --memory-swap=8g`，无新增 swap 配额，网络 `none`、源只读、仅新 build 目录可写。runner 必须实查运行中精确 ID、名称/label、镜像、CPU/内存限制、无特权与挂载，再保存脱敏投影；不能用 argv 冒充实际限制。之后观察/清理绑定该 ID，名称替换、过早退出、观察失败均拒绝。复用已审查 helper 的唯一 cleanup 流程，OpenClient 保有 CLI/双管道到完成或终止并回收。

容量预算为两份 build 目录各 16 GiB、每个 PG 估算 2 GiB、磁盘余量 2 GiB：普通 case 要求至少 38 GiB 可用磁盘，错误端点 case 40 GiB。这是估算预留，不是磁盘配额。内存预检为 builder 8 GiB + 每个可能并存 PG 4 GiB + 1 GiB 余量，普通 case 至少 17 GiB、错误端点至少 21 GiB MemAvailable；编译专用准备当前沿用该保守 case 门槛。执行前刷新容量/占用/路由，不能沿用历史空闲量；计时只用同一主机单调时钟。

## 私有输入与单 writer

仅 `#[cfg(all(test, target_os = "linux"))]` ignored 路径执行。非敏感环境只给新 root 路径、case 和精确资源身份；凭据仅原 SQLx 控制会话从既有私有文件读取。源生产者固定生成 `public.c4_import_probe(id integer NOT NULL,label text NOT NULL)`、固定主键和 `(1,'alpha')`/`(2,'beta')`。它实际执行固定 `pg_dump`，create-new 写入后通过 no-follow 句柄读入不可变快照，按真正生产字节的长度/摘要验证；实际 TOC 与完整 golden 解码消费同一快照。普通 `freeze_dump` 仍无签发能力。历史 dump SHA 仅审计。

子进程保持固定精确 64hex 容器 ID、`/usr/bin/docker exec --interactive --user 999:999`、`/usr/bin/env -i`、`LC_ALL=C`、`PGCONNECT_TIMEOUT=10`、`PGPASSFILE=/dev/null/knowweave-c4-passfile-disabled`、绝对 PG18 客户端、本地 socket/5432/learning_admin/出生 UUID 库；没有 caller SQL/flags、shell 或 TTY。dump/解码 SQL 各最多 64 KiB；writer stdout/stderr 各 8 KiB，在读取期间限额。解码 15 秒、writer 45 秒、READY/PRECOMMIT 各 10 秒且不延长总截止；writer 消失观察 5 秒，隔离另 15 秒。错误端点双容器共享一个隔离绝对截止。源生产、目标准入、writer 和清理总 harness 另设 360 秒，只是外层失败上限，不延长任何内部截止。

固定 `SET LOCAL` 为 statement_timeout 10秒、lock_timeout 5秒、idle_in_transaction_session_timeout 30秒、transaction_timeout 60秒。完整 golden 仅允许配对 restrict key 变化；不支持任意 SQL 清洗。原 Task5 同 writer 双锁准入、持久 attempt、提交意图、首次 COMMIT 发送边界、非放弃式管道收束及 guard 所有权算法继续使用。测试钩子只允许下表固定故障。

## exact case 白名单

所有测试的前缀为 `restore_preflight::target_binding::controlled_import::live_tests::`。一次调用只选下表一项 `--exact ... --ignored --nocapture --test-threads=1`；真正退出0、1 passed/0 failed/0 ignored、唯一完整结构化检查点 receipt 才能作为该 case 候选证据。模型输出和单独非零退出不构成负例通过。

| case | exact 测试后缀 | 必须实际观察 |
|---|---|---|
| success | live_controlled_import_commits_fixture | 同 writer、attempt/intent 先同步、唯一 COMMIT、提交确认、writer 消失、独立精确两行/主键/PG18两项 NOT NULL 约束及行摘要 |
| ready-eof | live_controlled_import_ready_eof | READY 后真 EOF，未发 ROLLBACK/COMMIT；无 attempt；停机前 writer 消失、独立零对象 |
| precommit-eof | live_controlled_import_precommit_eof | 已发送 DDL/COPY、PRECOMMIT 后真 EOF；attempt 留存、无 intent、独立零对象 |
| precommit-cancel | live_controlled_import_cancel_before_commit | guard/原事务仍在，取消先接受，未提交、无 intent、独立零对象 |
| ready-restart | live_controlled_import_restart_before_ddl | READY 后 DDL 前精确新重启，旧 guard/lease 拒绝，无 attempt/DDL，不重启取证 |
| sql-error | live_controlled_import_sql_error | 固定除零 SQL 实际报错，attempt 留存，无 COMMIT/intent |
| copy-truncated | live_controlled_import_copy_truncated | 固定 COPY 行截断实际报错，无盲 ROLLBACK/COMMIT，attempt 留存；不强称零提交已读回 |
| attempt-sync-failure | live_controlled_import_attempt_sync_failure | 真实 BackupDir 创建/写入后的固定 fsync 边界故障，无 DDL/COMMIT，已产生条目保留；不是内核 fsync 故障实测 |
| commit-intent-sync-failure | live_controlled_import_commit_intent_sync_failure | 同上提交意图同步边界故障，初始/部分意图条目保留，未发送 COMMIT |
| commit-unknown | live_controlled_import_commit_confirmation_lost | intent 先持久，实际发送 COMMIT 后不发送确认查询、收束真实连接；结果未知，无回滚或重试声明 |
| wrong-endpoint | live_controlled_import_same_id_wrong_endpoint | 新物理克隆同 ID/OID，原控制事务双锁复验且克隆无匹配锁，实际 writer 固定合并身份断言报错；双端零对象、无 attempt/DDL、双容器精确停机 |

错误端点仅从有界 stderr 识别完整固定主 ERROR 行 `psql:<stdin>:<正整数行号>: ERROR:  KW_C4_IDENTITY` 后的 LF；context 回显、前后缀、截断及超限均不构成证据。仅输出布尔观察，不保存原始 stderr。此标签本身表示组合身份谓词拒绝，必须和独立克隆/双锁事实合并判断，不能单凭标签断言具体哪一项条件失败。该格式仍需固定 PG18 实际 case 证实，未观察则失败。

## 结果与保留证据

所有正常/负例保留源/目标/克隆卷，停止精确容器，保留 candidate attempt/intent、不允许重试。runner 独立读取有界私有 marker，核对数据库、类型/版本、dump/SQL/出生摘要、同 writer 和初始 attempt 摘要；记录 marker 实际阶段与摘要，不将意图等同已提交。源 `decoded.sql` 的随机 restrict key 可与目标独立解码不同，两者摘要各自注明。

根批次保留源码/build SHA、列举/实际测试摘要、编译前资源观察、marker、合成 dump/TOC/源解码工件；编译/测试原始 stdout/stderr 只存 `*.private.log`。结果只序列化固定非敏感码与审核过的摘要/身份。最终 JSON 通过私有 pending、fsync、不可替换发布、父目录 fsync 和删除 pending 完成；返回摘要再核对最终字节并实际检查 `pending_absent`。持久文件成功字段单独不代表验收，必须有该外部发布事实。

控制者还需用户 root 回传完整脱敏 JSON、result/inspection 摘要、`PENDING_ABSENT`，并独立只读核对 Docker 精确 ID 停机/留卷。成功唯一正例标记为 `CONTROLLED_FIXTURE_IMPORT_PASSED_SINGLE_HOST_QUARANTINED_NOT_FULL_RESTORE`；固定负例用独立 expected-rejection 状态。提交未知永不报告 rollback，未确认停机永不报告通过。本地审查与 M1/M2/M3 修复审查已关闭；新 Linux 全部门和逐 case 现场证据仍未关闭，本施工单不能据静态审查完成。

Task3 的 `restore_preflight::target_binding::controlled_import::candidate_attempt::tests::live_candidate_journal_no_follow_and_fsync` 另需全新 root 私有 `KNOWWEAVE_C4_JOURNAL_TEST_PARENT` 与单独授权；Windows 模型不能替代该门。本地旧 M3 已以确定性关闭分支、真实待取消子进程及命令行持有测试完成修复和独立复审；原失败证据保留，PID39808 的历史调度仍未知，实际 Linux 行为继续单独验证。
