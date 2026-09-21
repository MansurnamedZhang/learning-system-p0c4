# P0-B3 验证记录

状态：`P0_B3_VERIFIED / NOT_PRODUCTION`。Task 8 同一可执行候选的宿主 focused12/full222、全新 Compose222 已由controller审计；独立补充审查通过并独立关闭runtime条件。首次 Compose 外部secret权限失败保留，不覆盖成功重试。全部B3测试容器已停止，五个证据/数据卷保留；没有待执行的runtime动作。B4仍未实施。

## 源码与运行绑定

| 对象 | 精确身份 |
|---|---|
| 冻结 B2 基线 | `b60056ba08892d69efdfa4778c976154a421e17d` |
| B2 原始基线包（94 文件） | `d915d39599abc394f191b93858d058ae170bf654fa671c951c4eea6046eb0deb` |
| 冻结源码清单（70 原文件）SHA-256 | `2da005cdf6f2e92ea4fa2f34463b371a71c30e145e6d8927eb75db558b284225` |
| Tasks 1–7 产品 HEAD | `d2dfcd8428e1b69ebce9c6d3ed45645a42049d6f` |
| Task 7 最后 GREEN 包（222 文件） | `dc4d7d1b30b731108270ac7e059621eddc8b9671f877a38079ad7ea306f19fe5` |
| Task 7 focused/full stdout SHA-256 | `c03c29e59a2a8d1d508f136c7163ae8f8745d189e33d860a16314c6d2d23a91b` / `65769f9d21d7803d6451b60b8d3f3d0632fc623acca648898b712e4a0d2190ea` |
| Task 8 第一个可执行候选（227 文件） | `e560181f41ccbc0fb5f18f0dcd8a1b0950dfa2dfd6494ccda20a6b3093c80ead` |

逐文件清单以 controller 封包及运行前后 SHA 为准；源码白名单为 Cargo 清单/锁、crates、migrations、contracts、必要 deploy 文件，包含独立冻结 runner 及其独立 Cargo.lock。不得包含 DSN、密码、私钥、整个 `.runtime` 或课件。本文在测试包冻结后补写，最终验收提交可通过 `git log -- docs/p0b3-verification.md` 追溯；218个非docs输入文件与测试包逐字节一致，已由controller审计。

## 已取得的验证

| 批次 | 实际状态 |
|---|---|
| Tasks 1–7 每关最终 | core35；full132、154、174、185、205、220，详见执行账本原始包与日志 |
| Task 8 Windows 本地 root fmt/check all-targets/clippy `-D warnings` | 全部 exit 0 |
| Task 8 Windows 本地 frozen runner package fmt/clippy | 全部 exit 0；冻结源未改 |
| Task 8 core / DB pure | 42 / 8 通过，0 失败、0 ignored；不冒称运行了本地 DB 集成 |
| Task 8 focused | 实际12通过/0失败/0 ignored，exit0；四 bootstrap exit0 |
| Task 8 宿主全量 | 实际222通过/0失败/0 ignored，exit0；五个全新库、四 bootstrap exit0 |
| Task 8 首次 Compose | build0/up101/stop0；外部 secret 不可读导致 p0a bootstrap101，余下 bootstrap/workspace NOT RUN；保留失败证据 |
| Task 8 全新 Compose 重试 | `learning-system-p0b3-test2` 新卷、相同候选；config/build/up/stop0，四 bootstrap0，workspace222通过/0失败/0 ignored，48份摘要 |
| 独立产品跨模块审查 | `b60056b..d2dfcd8` spec/quality PASS，0 新发现；不覆盖本轮 Task 8 文件和最终运行 |
| 独立 Task 8 补充审查 | spec/quality PASS，0 新发现；确认五旧 store、独立期望、部署、26 ruling 和旧105核算；另独立核对最终12/222/222、四bootstrap、资源与停止状态、227源码与43证据hash，runtime条件已关闭 |
| B3最终停止与保留 | 新test2 PG/test Exited(0)；baseline B3 PG stop0；所有B3容器停止、五卷保留，旧B2/dev不动 |

本地 core stdout SHA-256 `2961412987bbe57fe063cc2aab5d23f568b1a968481d0dd67c58563e4d8bba14`；root Clippy stderr `090faf0c4a8863cbdddd9b4fdd9909b75a94dee9fb482870f4aff8b4c7929790`；fixture Clippy stderr `dfc43c64a538cf95a14c8baa6ca5b9f40b26dc0580dcb063d11481ba257b6f38`。每个命令 stdout/stderr/exit 独立保存在任务 scratch `task-8-local`；本地没有 PostgreSQL DSN，集成测试不能 skip，也没有在本地宣称通过。

本地 DB pure8 stdout SHA-256 `9273346d8b627157de3c961c144cc78f942fe50b985ee7655270d43725e51cc0`。Linux `b3-task8-validation1` 宿主 focused stdout `be729cb9db88cfd7a6c473e09c0f53eafb0cc51b113394aa1c514fb182b039ff`，full stdout `c2c0e01f51581f769f31fb050b746862644397b16113d1b1b1211935725c79f2`；源包为上表227文件的 `e560181f…`，两轮均完成四次真实旧程序 bootstrap。原始证据目录为 `C:/Users/hans/Documents/ubuntu_Seoul/learning-system-p0b3/evidence/b3-task8-validation1/`。

首次 Compose 外部执行者创建的 secret 文件为0600、hans所有，PG容器 UID 不能读取，角色密码为空，p0a登录失败。环境故障不算产品 RED，不算缺凭据跳过，更不能计成 workspace通过；build0/up101/stop0及失败卷全部保留。只修正外部权限为 secret 父目录0700、文件0444（Docker挂载保持只读、主机父目录限制其他用户进入），不打印凭据内容。新项目 `learning-system-p0b3-test2` 和新卷重试同一源码；已通过的宿主字节不变，无需机械重复。

### 全新 Compose 的最终证据

原始目录 `C:/Users/hans/Documents/ubuntu_Seoul/learning-system-p0b3/evidence/b3-task8-compose2/`。controller逐项核对227源文件运行前后均一致、43份记录hash全部匹配；与宿主使用相同 `e560181f41ccbc0fb5f18f0dcd8a1b0950dfa2dfd6494ccda20a6b3093c80ead` 源包。config/build/up/stop全部0、四旧程序bootstrap全部0，workspace222通过/0失败/0 ignored，48份摘要。

| 证据 | SHA-256 |
|---|---|
| exported/workspace.stdout.log | `48b05905d2df7067a4bcc6033450039ee43e8381acadec6a9261a44c666b2d68` |
| exported/workspace.stderr.log | `aa4f9a6a2477c25cd6e06214c295bcfef733dbf59c791867fb71420955713b8b` |
| up/stdout.log | `ae76b4fcc5c76a479f59c2d4671cc683080e00dfa702891a85b0f5088c80300d` |
| build/stderr.log | `c7d1e4b38d542b0b4575936ea0e8a38a6af1ab44589483b688f4a6b6521456c4` |
| exported/b2.json | `fb6a05230136163f4420e4ceae0e90f38973f8999ba1fcf5a0bcc19df5fedecb` |
| SOURCE-MANIFEST.json | `2a006675a539c0e23ec7172d43490058dba35551725ddac6fb0f96f3e82d6d37` |

测试镜像 `sha256:f9ee9945628f7e1056e66b0e35082ac0052040a2b9c85f33db0cc7590d662664`；PostgreSQL固定镜像 `postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d` 不变。成功项目PG/test均Exited(0)，PG2CPU/4GiB、test4CPU/4GiB、internal网络和无宿主端口已核验。

最终只额外停止 `learning-system-p0b3-pg-1`（exit0），全部B3容器退出。五卷保留：baseline pgdata、失败项目test_pg/test_evidence、成功test2项目test_pg/test_evidence。最终状态证据在 `C:/Users/hans/Documents/ubuntu_Seoul/learning-system-p0b3/evidence/b3-final-stop/` 的 `containers.json`、`retained-volumes.json`、`evidence-sha256.json` 和stop日志；旧B2/dev未触碰。失败Compose、成功Compose和宿主证据均未删除。

## 升级协议和可复现命令

五个数据库各自独立：普通 current、P0-A(0001)、B1(0001–0002)、B3-schema(0001–0003)、完整 B2(0001–0003)。四个旧库先由冻结 runner 以 SQLx 正常登记旧迁移，严禁预装新表或伪造 checksum。P0-A 继续使用原 schema SQL 种子和冻结 reader；B1 用旧 Content/Composition/Release store；B3-schema 用旧 ContentStore；B2 使用全部五个旧 store。

每个升级库配置独立 ADMIN/runtime DSN 对，变量前缀分别 `TEST_UPGRADE`、`TEST_B1_UPGRADE`、`TEST_B3_SCHEMA_UPGRADE`、`TEST_B2_UPGRADE`。普通库为 `TEST_ADMIN_DATABASE_URL/TEST_DATABASE_URL`。路径变量分别为 `TEST_P0A_FIXTURE_MANIFEST`、`TEST_B1_FIXTURE_MANIFEST`、`TEST_B3_SCHEMA_FIXTURE_MANIFEST`、`TEST_B2_FIXTURE_MANIFEST`；bootstrap 脚本统一导出。缺凭据/夹具直接失败。

```sh
cargo build --offline --locked --manifest-path deploy/fixture-runner/Cargo.toml
# 配好五组独立数据库；目录须全新且使用绝对路径。
export FIXTURE_EVIDENCE_DIR=/absolute/fresh/evidence
. deploy/bootstrap-fixtures.sh
cargo test --offline --locked -p learning-db --test b2_upgrade --test b3_scenario --test relations -- --test-threads=1
```

focused 后完整升级库已经应用新迁移，**全量必须另建五库并重新执行四 bootstrap**，不能在消费过的旧库重复使用夹具。然后运行：

```sh
cargo fmt --all -- --check
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo test --offline --locked --workspace -- --test-threads=1
docker compose -p learning-system-p0b3-test -f deploy/compose.test.yaml --profile test build
docker compose -p learning-system-p0b3-test -f deploy/compose.test.yaml --profile test up --abort-on-container-exit --exit-code-from test
docker compose -p learning-system-p0b3-test -f deploy/compose.test.yaml stop
```

Compose 使用新 project/new volumes。上述命令展示首次项目名；保留失败卷后本轮重试将 `-p` 改为 `learning-system-p0b3-test2`。PG 2 CPU/4 GiB，test 4 CPU/4 GiB，internal 网络，无宿主端口；证据卷保存四个旧 manifest、bootstrap 原始日志/退出码、workspace stdout/stderr/退出码及 SHA 文件。测试结束停止新容器但保留数据库卷和证据卷；旧 B2/dev 不动。证据导出后另核对运行前后文件、镜像 digest、容器终态、CPU/内存/网络与实际测试计数。

B2 验收检查旧 DTO **原始紧凑字节**而非仅 parsed JSON；原始字段、时间、ID、所有旧行和命令 request_sha256/receipt 在升级及重放前后不变。独立固定期望为 R0 `[K,M]`、R1 `[K,N,M]`、R2 `[K2,N,M]`；Original/Personal 模式、occurrence、placement、旧/新 gap、历史 D1/D2 与精确视图都分别核验。新 backfill 表允许增加，不混入旧业务行不变断言。

## 原有 105 项覆盖核算

基线 105 = core23 + DB pure8 + 原 P0-A/B1 集成51 + B2 集成23。逐项扫描基线与当前测试函数，105 个相同路径/函数名全部保留，0 删除/改名。这个名称清单只是账目，不代替断言审阅或运行。宿主与全新Compose实际均为222 = 原105 + B3新增117，0失败、0 ignored。

原有 core23 及绝大多数旧集成测试字节不变。以下旧测试所在实现/夹具经过适配，逐项记录等效覆盖，不能靠重命名隐藏回归：

| 原函数（相同路径/名称保留） | 适配与保留的断言 |
|---|---|
| overlay/anchor `insertion_requires_actual_neighbors` | fixture 改 VersionedCompositionSnapshot；真实相邻左右邻居与拒绝错误位置断言不变 |
| overlay/anchor `gap_resolves_exact_parent_occurrence_path` | fixture 改版本化 snapshot；精确父 occurrence 路径断言不变 |
| overlay/persist `resulting_group_and_placement_limits_are_inclusive` | 产品持久化增加 view 选择，旧测试本身断言不变，保留 512/2048 inclusive 限制 |
| placement_migration/classify `fixed_neighbors_classify_without_text_similarity` | fixture 改版本化 snapshot；固定邻居分類、无文本相似推断断言不变 |
| placement_migration/classify `empty_boundaries_and_unplaced_do_not_guess` | fixture 改版本化 snapshot；空边界/未放置不猜测断言不变 |
| placement_migration/classify `nested_migration_keeps_exact_occurrence_path_and_rejects_lost_parent` | fixture 改版本化 snapshot；嵌套路径身份、丢失父路径拒绝不变 |
| reading/projection `explicitly_unplaced_item_has_no_active_location` | ContentRevision::V1 fixture 和版本化 snapshot；明确未放置必须 location=null 不变 |
| reading/projection `unplaced_projection_preserves_visible_saved_order` | V1 enum 解包代替旧字段访问；故意反序 UUID、完整/过滤后的保存顺序断言不变 |
| b1_upgrade `actual_b1_stores_and_receipts_survive_b2_upgrade` | 旧造数移到独立冻结程序，仍使用真实旧三 store；保留旧 checksum、create/save/publish 重放、snapshot、收据/发布/outbox 计数不变 |
| migration_upgrade `real_p0a_data_and_idempotent_receipt_survive_additive_upgrade` | 原 schema SQL 种子移到冻结 runner；保留原 v1 checksum、历史读/create 重放、content_v1 请求摘要及跨操作冲突 |

## 验收边界与审查收尾

Task 4 Minor 的独立 normalized pair/digest 断言第一轮真实运行 GREEN，补充独立审查确认，已作为补覆盖关闭；没有编造 RED。Task 8 如发现真实产品缺陷，先提交能复现的测试并取得实际失败，再窄修复并重新绑定源码/运行证据。

Tasks 1–7 每次审查发现、修复轮数、实际失败和日志 hash 在 [执行账本](p0b3-execution.md) 完整保留。原数据库实验 **NOT_PASSED 和四项权限失败**保持原结论；新 Rust 回归覆盖 basis_refs、target、正文、精确标识、关系/审查/列表及发布授权，不等于原实验原型修复或生产上线。

宿主与全新Compose原始证据已审计，独立审查runtime条件已关闭；controller确认218个非文档可执行输入文件与e560包逐字节相同。227文件包还含9份原有文档；本次最终新增三份B3交付文档不改变已验收程序。状态为P0_B3_VERIFIED / NOT_PRODUCTION，不批准生产，不把B4标记为已实施。
