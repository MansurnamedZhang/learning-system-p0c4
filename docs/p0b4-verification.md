# P0-B4 验证记录

状态：`P0_B4_VERIFIED / NOT_PRODUCTION`。本记录区分已运行结果和外部生产边界。所有专项包均由源码白名单封装，Linux 独立新 PostgreSQL 数据库执行，普通 admin/runtime 无 SUPERUSER/BYPASSRLS，PG 限 2 CPU/4 GiB、测试容器 4 CPU/4 GiB，内部网络且无宿主端口。根任务逐项复核包内源文件及原始证据 SHA；证据镜像位于 `<local-user-home>/Documents/ubuntu_Seoul/learning-system-p0b4/evidence/`，服务器原件位于 `<server-user-home>/experiments/learning-system-p0b4/evidence/`。各批次的真实 stdout/stderr 与失败记录须按目录读取，下面的计数不是替代原始日志。

## 源码与阶段身份

| 对象 | 精确身份 |
|---|---|
| B3 固定基线 | `4252cc812b6c3bb7e7fabbcc309522b005f9df44` |
| B4 Task 1–4 当前基线提交 | `2294011479d5bf7eb44eb93f6c81b40af250e3a0` |
| Task 4 最终可执行包 / 251 源文件 | `62c29095857c50d17b1a32c99fc2e2da0abb9514d99605def79d9833249ebd5e` |
| Task 5 Attention 首次局部补覆盖 / 251 文件 | `0eb248c7f74ca1dda6db481e1b87d3daeb1cc681c917f954b0047538ae449c6e` |
| Task 5 Attention 扩展 / 252 文件 | `0cf1c97a5c2daae2bf94020b4fa9dd4ba6f30102edfdd1c152927049770d8e6b` |
| Task 5 阈值行为 RED / 252 文件 | `9ced9d5fb714d2fe1f4e16d816fdfe2b717f5eacc5e05c1ed9ddc9a1e66e8b44` |
| Task 5 阈值修复最小包 / 252 文件 | `e4b631e2fe0857626d429ee5b4bf07131fe90a2005428d74b245ce0f568ec25d` |
| Task 5 增强全量期望候选 / 252 文件 | `6fccd824b14cf64bbb2a774dd1bdce627950c860949e996f3aaf74c9c7577b6a` |
| 同一增强测试的未修复源码对照 / 252 文件 | `8a245766993fe8a39da004f967237ddd2a330993b1d44454bec56bad0c671d50` |
| Task 5 宿主全量候选 / 255 文件 | `daa8e47c282a66ad13209b298d58be6b213064f72b2c0510c6c2ce0d73d12477` |

`8a245...` 与 `6fcc...` 的 252 个清单源文件除 `queries/consumers.rs` 外一致；前者在封包时恢复 Task 4 原消费者实现，封包后工作树自动恢复。它用于验证同一夹具和同一断言在旧实现的 RED，不是可交付产品候选。`0eb...` 名称含 red 但真实测试通过，不能当作失败证据。最终候选若代码或测试再变化，需重新冻结身份并重跑相应检查。

## 已取得的真实结果

| 批次 | 命令与结果 | 解释边界 |
|---|---|
| Task 4 完整候选 | `cargo fmt --check` exit0；`cargo clippy --locked --workspace --all-targets -- -D warnings` exit0；五库 `cargo test --offline --locked --workspace -- --test-threads=1` 共 283 通过、0 失败、3 显式 EXPLAIN ignored；四冻结 bootstrap 全 exit0 | 仅证明 Task 4 当时的冻结可执行源码；不能替代 Task 5 代码变更后的全量 |
| Task 5 Attention 首切片 | 新库命名 `b3_scenario` 1/1、exit0 | E1/E2/X 固定关系和到 C 的至少一条路径；补覆盖 GREEN |
| Task 5 Attention 扩展 | 新库命名 `b3_scenario` 1/1、exit0 | H/C placement、原文 occurrence、H@2、撤权恢复、Split 的固定/新范围完整断言 |
| Task 5 Session 首个行为 RED | 新库 `impact_session_threshold` 0/1、exit101；公开聚合 A/H、组合/Reading 建立与 B3 `content.read`/`read_versioned` 先成功，仅 `direct(T)` 报 `Storage` | 查询阶段行为 RED；此版本还没有 traverse/direct 同夹具差分，不把私有 B3 计数写成已观测值 |
| Task 5 增强候选 `6fcc...` | 新库 `impact_session_threshold` 1/1 + `b3_scenario` 1/1，两个命令分别 exit0，原始日志已核验 | 第一项先用同范围 `traverse(T)` 和 `direct(T)` 均得 175 个完整消费者并全结果等值；第二项增加 Semantic/Necessary/ReviewSelection 单族固定期望 |
| Task 5 原实现同测试对照 `8a245...` | 新库 `impact_session_threshold` 0/1、exit101；B3 读和同范围 `traverse(T)` 175 个 `Complete` 先通过，首次失败仅 `direct(T)` 返回 `Storage`；根任务复核包清单及原始日志 SHA | 与 `6fcc...` 同测试、同夹具，252 文件仅消费者实现字节不同，强差分定位旧 `direct`；公开错误仍不能单独证明私有 work 计数 |
| Task 5 五库最终全量 `daa8...` | 255 文件清单逐项/运行前后、原始日志 SHA 由根任务复核；fmt、严格 Clippy、四冻结 bootstrap 均 exit0；五库 `cargo test --offline --locked --workspace -- --test-threads=1` 57 份 suite、284 通过、0 失败、3 个显式 EXPLAIN ignored | 根从 B3 原始全量 stdout 逐名提取的旧 222 项与本轮真实运行日志比对：222/222 状态 ok，缺失 0；Task 3 大范围及 Task 5 两命名专项均 ok |
| Task 5 全新 Compose `daa8...` | 项目 `learning-system-p0b4-t5-fc1-compose1` 启动前无旧容器/网络/两卷；五库均空；config/build/pg-up/stop 均 exit0，up 按 test 容器退出码为 0；容器内 57 份 suite、284 通过、0 失败、3 个显式 EXPLAIN ignored，四旧 bootstrap exit0 | 同一 255 文件可执行包的隔离部署验收；全新卷与原 B3 卷分离，镜像与资源/日志见下；独立终审已关闭 |

Task 3 大范围回归已在 `evidence_query.rs` 定义：四个各 475 块的合法子组合组成 1900 固定范围节点，另有 90 个小动态关系候选；断言 90 条可见关系、`Complete`。该测试在 Task 3 及 Task 5 五库全量中均为 ok。Task 4 的四种旧程序 bootstrap 对应 P0-A、B1、B3-schema、B2，旧冻结源码 SHA-256 `2da005cdf6f2e92ea4fa2f34463b371a71c30e145e6d8927eb75db558b284225`；0001–0006 checksum、旧行、收据、正文摘要及历史发布断言在 Task 4 与 Task 5 全量内通过。新 `0008_lineage_target_index.sql` SHA-256 `26f38c8194c9e6db7fa051128efba9dfad641d930fe8e5084a31b4ff57199085`。

只读静态测试身份核算以 B3 基线 `4252cc8` 对比当前 `crates/**/*.rs` 的路径和 `#[test]/#[tokio::test]` 函数名：原 222 个全部仍在，0 删除/改名，当前共有 287 个、净增 65 个。此为名称辅证；本轮另以宿主与 Compose 真实全量 stdout 逐名核对了 222/222 均为 ok。独立终审已复核断言含义，不能只凭名称或计数认定质量。

## 最终候选的运行命令与原始证据

下列宿主与 Compose 命令已在 `daa8e47c...` 冻结包执行通过。宿主五库/四 bootstrap 全量使用 `p0b4_t5_fc1_full_0` 至 `_4` 五个独立数据库，`preflight-empty-databases.json` 记录每库迁移前用户表数为 0；四份旧程序夹具的 `started_empty` 均为 true。冻结旧程序脚本先校验源码，再做旧版本 bootstrap。具体 DSN 和凭据只留服务器秘密挂载，不写在仓库或证据清单中。

```sh
cargo fmt --all -- --check
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
# 五个新数据库、四个冻结旧程序 bootstrap 完成后：
cargo test --offline --locked --workspace -- --test-threads=1
# 本轮 Compose 使用的项目名：
docker compose -p learning-system-p0b4-t5-fc1-compose1 -f deploy/compose.test.yaml --profile test config
docker compose -p learning-system-p0b4-t5-fc1-compose1 -f deploy/compose.test.yaml --profile test build
docker compose -p learning-system-p0b4-t5-fc1-compose1 -f deploy/compose.test.yaml --profile test up -d --wait --wait-timeout 90 pg
docker compose -p learning-system-p0b4-t5-fc1-compose1 -f deploy/compose.test.yaml --profile test up --abort-on-container-exit --exit-code-from test
docker compose -p learning-system-p0b4-t5-fc1-compose1 -f deploy/compose.test.yaml --profile test stop
```

宿主原始证据目录为 `b4-task5-final-candidate1-full/` 与 `b4-task5-final-candidate1-checks/`，均位于上述服务器和本地证据镜像根下。全量 `workspace/stdout.log` SHA-256 为 `befa5c4f3fcc68c18a1929a5c07b2b5b950aa0ecf1b4e2171baccd42e4e56c5f`，`workspace/stderr.log` 为 `8046b54f14eb80549617ce7078ea8ba0829ab3902b8c4972fdee7c384f837250`。四份旧程序日志分别为 `workspace-fixtures/bootstrap-{p0a,b1,b3-schema,b2}.{stdout,stderr}.log`，退出码各存同名前缀 `.exit`；旧程序 manifest 在 `workspace-fixtures/{p0a,b1,b3-schema,b2}.json`，四者 `source_manifest_sha256` 同为 `2da005cdf6f2e92ea4fa2f34463b371a71c30e145e6d8927eb75db558b284225`。`result.json` 保存全部原始文件 SHA、各阶段退出码和运行前后源码清单核验；`workspace-databases.json`、`preflight-empty-databases.json`、`workspace-identities.json` 保存五库身份和空库预检。检查批次 `fmt` 和严格 `clippy` 均 exit0，其原始日志与哈希见对应 `result.json`。

Compose 原始证据目录为 `b4-task5-final-candidate1-compose1/`。`preflight-project.json` 记录项目容器/网络为空，且 `learning-system-p0b4-t5-fc1-compose1_test_pg` 与 `_test_evidence` 均不存在；`preflight-empty-databases.json` 记录新卷内五个数据库迁移前用户表数均为 0。`result.json` 记录相同 ZIP SHA、`source_unchanged=true`、六阶段均 exit0；`exported/workspace.exit` 和四份 `exported/bootstrap-*.exit` 均为 0。容器内 `exported/workspace.stdout.log` SHA-256 为 `59eb94347d6afbe7a3bedaee084dd63b24aaf9ff1c9895594d09419752250640`，`exported/workspace.stderr.log` 为 `b627cda3474743adbcaf488f2d7c02f6655fa90039189b2171cc250643b97753`；Compose `up/stdout.log` 为 `b40296ab2fe177f25821320b7a1c88421092fd4548ac4ca41872d626b6f15eca`，`up/stderr.log` 为 `9e6ef450e4188e3600dc328016a1328651de1f04339e02077b080a44b56831d1`。`resources.json` 记录 PostgreSQL 镜像 digest `sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d`、测试镜像 digest `sha256:8b4ffdfaa1ec6bb0a12527f00936513238453b83affc7d65faa4a2db04a9b8a1`，分别限 2/4 CPU 和 4 GiB；两容器 `ports={}`，`network.json` 的唯一网络为 `internal=true`，`stop` 后两容器退出码均为 0。`identities.json` 中五库的 admin/runtime 均非 SUPERUSER/BYPASSRLS，`SOURCE-MANIFEST.json` 有 255 项。

独立终审已复核契约、授权/解释路径、Task 2/3 阈值、SQL 计划与索引、旧 API、升级、宿主/Compose 原始证据及 222 项旧测试的真实日志，**无开放 Important/Minor finding**。新增谱系索引在合成 scale 8 的 8/8 原始计划中被使用，但 Sort 仍存在、耗时有升有降；0007 必要后页索引也未在该样本证实加速。终审只接受目标访问形状及授权正确性，不推断生产延迟上界。宿主与 Compose 运行前后源码清单及三组证据文件 SHA 已复核；本地镜像的 `result.json.evidence_sha256` 分别为 full 38/38、checks 10/10、Compose 50/50 匹配。Compose 容器内 stdout 再与 B3 原始 222 个测试名逐一比对，222/222 为 ok。`deploy/compose.test.yaml` 当前默认项目名为 B3；本轮显式 `-p` 和新卷空库核验避免误用旧 B3 卷。旧 B3/失败 B4 卷及证据均保留，不能为了重试删除。

上述隔离验证与独立终审门槛已关闭，因此本内核标记 `P0_B4_VERIFIED / NOT_PRODUCTION`。验证完成不等于生产部署、HTTP 身份认证、网页、资产原件或原 PostgreSQL/Neo4j 实验已通过。
