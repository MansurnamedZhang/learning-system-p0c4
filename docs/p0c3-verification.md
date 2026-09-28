# P0-C3 验证台账与开放门

当前：`P0_C3_VERIFIED / NOT_PRODUCTION`。冻结受测源码提交为 `3342c0554c18787749d0f06a772743f867c18720`；Task 7 已在全新隔离 Linux Compose 项目完成完整 workspace、四套旧版升级、真实 Worker SIGKILL/租约接管、融合快照导出导入往返、四组失败注入、格式与严格 Clippy。root 私有原始证据经独立只读脚本逐项核验；C4 全量备份恢复、P1 HTTP/前端及生产部署仍未验收。

## Task 7 整关验收（2026-09-28）

- 授权源码包：`task7-seed-v1-release-reviewed-candidate.zip`，SHA-256 `4754aee86dfb18d09d8d8c07d2ef94e802afa6c3bffdc5014efd03f982efed5a`；341 份源码文件，`SOURCE_MANIFEST.json` SHA-256 `5b88a2bac54c94e79ff54232d3d980eb872f0da291d7f8450fd02964908b0b5e`。full 证据中的 source-before/source-after/current 均与该清单相同；固定镜像的 301 项 COPY 输入逐项一致。
- 先在 `learning-system-p0c3-task7-3342c05-focus-c47d1e68`（`10.251.207.0/24`）运行真实 PostgreSQL V1 发布夹具与 seed 专项，状态 `FOCUSED_SEED_V1_PASSED_NOT_FULL_ACCEPTANCE`，结果 SHA-256 `305eb229a285f1776c0e13f9ef954a80569f89a9e3450f9e03697748e4559667`。full 启动前再次核验这一份 root 私有证据，而非混用旧批次结果。
- 全量项目 `learning-system-p0c3-task7-3342c05-full-8a4c2d91`（`10.251.208.0/24`）返回 `CANDIDATE_GATES_PASSED_NOT_PRODUCTION`，`result.json` SHA-256 `c939cc712ff8471c0bf8e4175d73c5b70e2166652086fc70dcd6244d60a18da8`。root 私有证据位于 `<server-user-home>/experiments/learning-system-p0c3/evidence/learning-system-p0c3-task7-3342c05-full-8a4c2d91`；五个容器均已停止，workspace、PostgreSQL、第二及重复 Worker 退出 0，首只按要求 SIGKILL 退出 137。
- 独立只读核验脚本 `attest_task7_full_3342c05_v2.py` SHA-256 `c22f94853fd8f3775582c23b0a30f8badd32064c4a7cba36c408d117c55c6586` 由用户在 root 下运行，返回 `FULL_EVIDENCE_ATTESTED_NOT_PRODUCTION`：602 份证据文件的清单 SHA-256 `8b45a18704e672848005d8787281dc5153de2cd1e02b3467f36b5989698cec5e`；341 源文件、301 镜像输入、4 套完整旧版升级校验序列、四组失败注入分别 3/5/6/2 项。核验从原始退出码、命令、日志、哈希与容器状态复查，不只相信 `result.json` 的状态字段。
- 首版额外审计脚本因在多批 `sha256sum` 日志间多插入一个空行而误判 `ValueError`；原脚本与失败输出保留，修正版仅更正拼接方式，未修改业务源码或原始证据。此前 Task 7 的失败批次也各自保留，不合并充当本次通过证据。
- 最终全分支独立静态复审未发现阻断候选验收的新增代码问题；静态结论与上述真实 Linux/PostgreSQL 证据分别成立。隔离验收不等于生产上线。

## 已有精确包证据（历史，不等于新候选通过）

Task 1–5 已按逐任务审查和隔离证据分别收口。Task 3 Linux 文件句柄/包边界、Task 4 PG job lifecycle/撤权、Task 5 只读 preflight 与隐私/metadata-only 负例各有自己的冻结包记录。Task 5 `c1d65adc` 包曾有 Linux/PG 86/86；Task 4 专项修复 `d061ce5` 的 snapshot_jobs 18/18、升级1/1、C2 39/39、Worker11、Task3 38 通过。这些不是 Task 7 进程/组合验收。

Task 6 三次真实运行和单独续跑不能合并抹去失败：

| 精确包/来源 | 原始结果 | 诊断及边界 |
| --- | --- | --- |
| `283d11f` 源码 / ws1 | snapshot_import_atomic 1/2；result SHA `cc4b834d3947d18da34375f0fe7f67f6db7f2895ac5dcd4e8f60ce27a8415c6d` | 跨角色 pg_stat_activity.query 不可见，锁观测夹具超时；后续门停 |
| observer ZIP `c558a77bb4fd166a4334064d69c952a77489836b9fba4c18e39a7336102cc909` / ws2 | 首门2/3；result SHA `cf11bbea48db1d977caa84183e36d7022f649200dafa548eacf6df25831f4814` | observer和升级通过；未提交读取断言把公开 Ok(None) 契约错写为 Err(NotFound)，前一行零权威行数通过；后续门停 |
| `223d29af` / ZIP `83d5a24fa8d221c427d1a5dc30aadc1ad1a5197df5b116d8832b652a7044bdca` / ws3 | Task6 3/3、Task5/C1/相关回归通过；随后 relations_schema exit101 | runner 漏 frozen b3-schema producer manifest，未走产品 SQL 断言；原101保留 |
| ws3 独立续跑 | relations_schema 15/15，Task3/4、fmt、严格Clippy通过 | controller确认 scoped Task6 accepted；并非 C3 整关或生产验收 |

历史证据由 controller 保存并审计。此 Task7 文档没有下载服务器文件，也没有重新复核历史包；最终报告须连接本批 raw artifacts 的真实 hash，不得用本页叙述替代证据。

## 四组具体失败注入

这些测试由 **完整 workspace** 实际执行一次，host 驱动按准确名字与 `... ok` 行建索引；任何被忽略/未运行的项都令门失败。单个原子测试内部仍保留全部细分断言，不能把一个 generic error 当所有故障通过。

| 组 | 必须出现的具体测试名 |
| --- | --- |
| 中断 staging | `interrupted_input_never_creates_ready_package`、`process_exit_before_seal_never_publishes`、`rejects_premature_eof_against_declared_manifest_length` |
| 缺失/损坏/篡改包 | `rejects_extra_files_corrupt_content_and_reparse_points`、`missing_and_corrupt_asset_bytes_never_publish`、`malformed_closure_authors_parent_and_body_leave_all_counts_unchanged`、`no_originals_requires_same_space_and_asset_id_and_real_bytes`、`revoked_and_read_only_root_grants_cannot_preflight_and_copy_is_rejected` |
| 撤权/租约丢失 | `integration::revocation_after_plan_or_copy_prevents_result_and_delivery`、`integration::old_token_cannot_publish_and_winning_package_survives_restart_until_revocation`、`private_overlay_revision_id_never_reports_an_identity_collision`；另有新容器 SIGKILL/fence/revoke 命令门 |
| 碰撞/事务回滚 | `preflight_reuses_exact_rows_read_only_and_rejects_identity_metadata_changes`、`attention_exact_atomic_roundtrip_two_fresh_databases_and_failure_boundaries`：包含同 ID 审计字节变化、自然键碰撞、额外 registry、late SQL/FK 注入、元数据原件被删除/损坏、授权变化、并发 receipts 和零半可见状态 |

错误 actor 的交付/preflight 另由新 Attention 管理例程明确执行，要求 NotFound 和零目标行；reading-copy、隐藏碰撞、metadata-only 同 SHA 错空间/ID 保持独立命名断言。

## 最终必须审计的原始证据

- clean source commit/ZIP/逐文件 manifest，0001–0013 和 frozen producer 源码 hash，source-before/source-after 完全一致。
- 11 个不同数据库的初始空库证明；四 producer manifests + SHA + exit；四旧程序升级 test pass，四库完整 migration checksum 与旧正文/receipt 比较。
- 完整 `cargo test --offline --locked --workspace -- --test-threads=1` 原始 stdout/stderr/exit；Linux非root运行、fmt与严格all-target Clippy。本地Windows Linux-gated的0 tests不能计为通过。
- 每只独立 Worker config/inspect、PID/信号/exit137且非OOM、DB-clock过期、attempt1→2、私有旧token hash一致但无token明文、零旧结果/恰一新结果、重复进程不改变结果。
- 管理进程重启后交付的 manifest hash与job/result一致，目标sealed intake、全行/投影/真实PDF PNG字节摘要、当前 unplaced 身份/原锚/顺序、receipt绑定/排除行数；撤权拒交付。
- 四组的具体 test/exit/result 索引；没有输出的SQLSTATE标为未输出，不推测错误码。完整evidence相对路径SHA清单，失败保留而非重写。只停止本批项目，旧证据/生产无变化。

Task 7 新包真实 Linux/PG 执行、独立静态复审与 raw hash 审计均已完成，因此仅将 C3 标记 `P0_C3_VERIFIED / NOT_PRODUCTION`。C4/P1/HTTP/UI/生产不随之完成。
