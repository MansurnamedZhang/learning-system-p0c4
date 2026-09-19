# P0-B1 实施与验收记录

2026-09-19 · `P0_B1_VERIFIED` / `NOT_PRODUCTION`。固定组合与原子发布已验收；P0-B 总阶段仍未完成。

计划：`../../docs/superpowers/plans/2026-09-19-p0b1-composition-release.md`。基线 e799d6f；用户在计划交付后明确要求“继续”。范围仅 B1，B2 及后续界面不在本次执行中。

## 已取得证据

| 阶段 | 结果 |
|---|---|
| P0-A 独立数据库基线 | 29/29 通过 |
| 组合/发布核心契约 RED | 1 通过 / 6 行为失败 |
| 核心契约 GREEN | 16/16，含原 9 项 |
| 组合 schema RED | 2 项因缺表失败，连接及 0001 成功 |
| schema GREEN 与原 PG 回归 | 新 2/2、原 20/20 通过 |
| 组合业务 RED | 6 项 stub 行为失败 |
| 发布/授权 RED | 两组各 3 项 stub 行为失败 |
| 组合 GREEN / 并发中间状态 | 组合 6/6；并发 4 通过、2 项发布 stub 预期失败 |
| 旧数据升级 | 独立空库 0001→夹具→0002，1/1 通过 |
| 第一轮宿主完整 GREEN | 57/57，fmt/Clippy/test 均 exit 0 |
| 补充预算/组合回滚宿主验收 | 60/60 通过；本批未执行 Compose，后续修复包将执行 |
| 独立审查 | 无 Critical；发现 1 项 Important 发布恢复缺口及 3 组验收断言不足 |
| 发布恢复修复 RED | publication_state 的 3 项在可编译 stub 上失败 |
| 修复后宿主全量 | 68/68，0 failed / 0 ignored；fmt、全目标 Clippy、workspace test 均 exit 0 |
| 全新空库 Compose | 68/68，0 failed / 0 ignored；config / 标准 build / up / stop 均 exit 0 |

源码包均在 `.runtime/`，包含逐文件 SOURCE-MANIFEST.json，服务器执行前后验证；原始日志与结果保存在 `C:/Users/hans/Documents/ubuntu_Seoul/learning-system-p0b1/evidence/`。本任务已独立核对 10 批结果、24 份日志的 SHA-256、测试计数、退出码和当前源码一致性；副本与审计结果保留在忽略目录 `artifacts/p0b1/`。最终包内 58 个文件的运行前后哈希一致，之后仅更新交付文档。

## 固定包

| 批次 | SHA-256 |
|---|---|
| schema-red | 0e376711c7e0d83bdd8b4754ac803ac059682df401671bbf7f890cf10cf38671 |
| schema-green-composition-red | f6bc38e7cfdf6f820978e466edc61df14c202176048394ce8720972c1a6aa1f0 |
| all-business-red | ff9e0002922af65abe61f1f1bcdf45dce36e20b05f9a39eb77a1d380777bffd4 |
| composition-green-release-red | f9424d02dbb3afe1f93d64637fe0a4d8b1ca05ffeafd618f00d1c6e31b0f1105 |
| business-green | 89771a3c0ecaf69faeb04cea1cd3b645f8621bdcd66df7d96e41452d3fe530cf |
| acceptance（60 项宿主） | 9bdeeee3ccd399385eaf09cf95985d353870014c1c22563a8767b126823efa67 |
| review-fix-red | aa1cf54ec5ad875fa0ae35b05a684587b079452f3e733adc72baeb15315851b4 |
| review-fix-green2（最终验收） | 267b88d2b6151a4276fa5cbec94e6e99b87447138680431f2ccf974890e83efd |

第一轮57项日志 SHA-256：`33d72a908a214dd88681d9fe325471b3b30a0255e03026834bf987bdf86b9b72`。测试角色仍非 superuser、bypassrls、表所有者；旧 P0-A 与图实验卷未改动。

## 最终运行与交付

代码提交：契约 `b890798`，组合/发布与回归 `6563a3d`。部署脚本和本文随最终文档提交保存；没有推送或生产部署。[执行账本与取舍](p0b1-execution.md)记录一次独立审查及修复过程。

最终 68 项由核心 17 项、原 PostgreSQL 20 项、新增 PostgreSQL 31 项组成。新增数据库测试为 schema 3、composition 10、release 3、authorization 3、concurrency 8、migration_upgrade 1、publication_state 3。原 P0-A 29 项全部保留。

宿主新批次使用 `p0b1_review_green2` 和专用升级库 `p0b1_upgrade_review_green2`；执行 `cargo fmt --all -- --check`、`cargo clippy --offline --locked --workspace --all-targets -- -D warnings`、`cargo test --offline --locked --workspace -- --test-threads=1`。升级测试实际运行原 0001 → 旧夹具 → 完整迁移，比较旧修订、头、收据、摘要与重放结果；0001、Cargo.lock 和 content-v1 实现与 P0-A 包逐字节一致。

Compose 项目为 `learning-system-p0b1-reviewed-final`，使用全新主库与升级库。标准 BuildKit build 首次成功，约 198.6 秒；本批没有代理回退，没有修改共享 Docker daemon。数据库限制 2 CPU / 4 GiB，测试运行器 4 CPU / 4 GiB，internal 网络，无宿主发布端口。测试结束后本阶段全部三个专用容器均 Exited(0)，数据卷、文件和证据保留；已有业务服务继续运行。

| 最终证据 | SHA-256 |
|---|---|
| 宿主 workspace 日志 | 3d01fd1da0427d8e311b95902de21069faa3e269efabe981da6a466947a7dc3d |
| Compose 标准 build 日志 | 023226278c5308ea1fb779b951dbd936c97f659fed3be77f0bc182f336f23543 |
| Compose up 日志 | 8ea2372d8fe2fb87800c19011131185a82d926188d82b663a7fac9dc98cc87bb |
| 官方 Rust 1.97.0 bookworm amd64 | b5a086f64ffecaa4e283063184770107915756739598173e1f5712d6b34b84d0 |
| 官方 PostgreSQL 18.6 bookworm amd64 | 9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d |
| 测试镜像索引 | bc7bde71f4ec934dcfdc879feb1266b00831f004a7fd91a5a48f527e861793fe |
| 测试镜像 amd64 manifest | df273c358abbb1c80fabfdbc5715e6cb578c72901ebf7d02acded841360e3e67 |

宿主证据位于 `b1-review-fix-green2-20260919`，Compose 证据位于 `compose-reviewed-final-20260919`；后者的 `final-state.json` 包含资源、端口、内部网络、停止状态和镜像无代理环境变量断言。凭据未进入证据或仓库。

## 实施取舍

1. 沿用现有专用 feature 工作区，账本按本计划单独保存；没有创建并行实现任务。
2. 多对象共享请求/授权抽取和组合/发布模块在一个集成 checkpoint 提交，避免留下只能连接半套 schema 的中间提交；各任务 RED/GREEN 证据分别保留。
3. 8 MiB 指块正文 JSONB 的数据库序列化字节，明确作为读取预算；不是整个 API 响应字节上限。
4. 固定引用在查验时必须已提交；不承诺早于整个 READ COMMITTED 事务起点。未提交临时互引不支持。
5. 独立审查指出：active 只返回修订引用，fresh caller 无法取得新的 expected_release_id。现增加 PublicationState 并将未对外发布的 B1 输入改为组合专属不透明 token。代价是 B1 计划 JSON 调整；不改 P0-A 契约或数据库迁移。
6. 审查将三组测试不足标为 Minor；执行方依据已承诺的安全/完整性验收将其提升为必须补齐的验证项，增加真实对象交叉配对、跨空间撤权先提交、乱序输入锁序观察和全部结构行回滚计数。它们是验证缺口，不宣称发现了额外产品漏洞。

这些取舍不改变旧摘要、位置身份、固定发布、权限或回滚规则。

## 独立审查范围的判断

- 审查核对固定源码包及已有57项日志；没有自行取得数据库凭据重跑测试。新增预算测试、修复后的结果及最终 Compose 由本任务独立核验服务器证据。
- 发布恢复缺口按三个真实场景做 RED→GREEN：fresh caller、冲突刷新重试、不可读兄弟根仍允许更新可读根；根专属 token 防止跨组合复用。执行技能要求单次修复后全量回归，不进行重复独立审查。
- 生产容量、延迟和总事务耗时、滚动部署中新旧二进制同时写库不属于本次通过声明；本次验证隔离迁移和结构预算。上线前仍需对应验证。
- 受信任数据库凭据被攻破、管理员篡改和用户级 RLS 不在威胁边界；运行凭据不能交给终端用户。
- HTTP/登录与 Principal 来源、前端、个人层、语义关系、资产、outbox 消费、搜索、导出、备份恢复在后续已批准阶段；本次不声明这些功能可用。
- 8 MiB 是数据库块正文预算，不是整体响应或传输字节上限。
