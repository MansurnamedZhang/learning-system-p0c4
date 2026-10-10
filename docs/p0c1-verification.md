# P0-C1 原件与图片块验证记录

**结论：`P0_C1_VERIFIED / NOT_PRODUCTION`。** 隔离 Linux 主门、四套冻结旧程序升级、独立 Compose 实际资产/暂存卷探针及同项目 PostgreSQL 补证均已通过；源码与原始证据经逐项哈希复核，独立源码审查无阻断。此结论只覆盖 C1 契约，不代表生产可用。

## 冻结身份与证据

| 项目 | 已核对值 |
|---|---|
| 分支、提交 | `feat/p0c-assets`，`a962ab8faa6abc2506f2f47d3e23ae99e2ee781c` |
| 源码包 | `p0c1-task6-pending-approval-final.zip`，SHA-256 `79a24f6f158f25f1edc470956e1232c9bc9b7a38f75b7abedd19986ce9ca55ec`；274 个文件 |
| 包内清单 | `SOURCE_MANIFEST.json` SHA-256 `2bcd7e61eae81425b4f11029f68653b1efe58b617e247719c7e570342ce2fcd7` |
| 主门隔离项目 | `learning-system-p0c1-task6-final-ws1`，最初不存在同名容器、卷与网络；五个测试/升级数据库均为空 |
| 工具链与镜像 | rustc/cargo 1.97.0；测试镜像 `sha256:8b4ffdfaa1ec6bb0a12527f00936513238453b83affc7d65faa4a2db04a9b8a1`；PostgreSQL 18.6 镜像 `sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d` |
| 主门原始证据 | `<user-home>/Documents/ubuntu_Seoul/learning-system-p0c1/evidence/p0c1-task6-final-workspace/`，入口 `result.json`，其所列 78/78 份证据文件的 SHA-256 独立核对一致；`source_before`、`source_after` 逐项为真，`source_unchanged=true` |

主门 PostgreSQL 容器为 2 CPU/4 GiB、内部网络且无主机端口；测试容器为 4 CPU/4 GiB、源码只读挂载、无主机端口。`learning_runtime` 在五个数据库均为非 superuser、非 BYPASSRLS、非数据库/公共 schema/表 owner。完整角色、资源、网络与空库快照见该证据目录的 `initial-identities.json`、`preflight-empty-databases.json`、`postgres-resources.json`、各门 `resources.json` 和 `network-preflight.json`。主门网络使用预检无重叠的 `10.251.16.0/24`。

## 已通过的 Linux 主门

以下结果均来自上述冻结提交和原始日志，命令退出码均为 0；项目已执行 `pg-stop` 且退出码为 0，证据保留。

| 门 | 结果与原始记录 |
|---|---|
| 综合 Attention 夹具 | `cargo test --offline --locked -p learning-db --test p0c1_acceptance -- --test-threads=1`：1 通过、0 失败；`acceptance/`。夹具串联 PDF/PNG/Notebook 原字节、Figure/Attachment 精确使用、两份共享 H@1 组合、个人位置移动、E1/E2/X 关系与 Inconclusive 审查、固定 Reading/Release、H@2、拆分及待放置。 |
| 格式 | `cargo fmt --all -- --check`：退出码 0；`fmt/`。 |
| 严格静态检查 | `cargo clippy --offline --locked --workspace --all-targets -- -D warnings`：退出码 0；`clippy/`。 |
| 工作区回归 | `cargo test --offline --locked --workspace -- --test-threads=1`：328 通过、0 失败、3 个既有忽略；`workspace/stdout.log`、`test-result-lines.json`。包括声明限额的两段复制测试、媒体声明及综合夹具。三个忽略项仍属于原有 EXPLAIN 类测试，未在此轮执行。 |
| 冻结旧程序升级 | P0-A、B1、B3-schema、B2 四套 `bootstrap-*.exit` 均为 0；`workspace-fixtures/` 保留原始 stdout/stderr、各 JSON 和 `fixtures.sha256`。四套旧程序分别在空升级库上运行，旧对象/收据/内容摘要及迁移校验和由夹具核对。 |

迁移 0001–0010 的逐文件 SHA-256 见 `migrations-sha256.json`；其中 0009 为 `398d1e42d29f20ee928e5ee608c951397c792934226331d3777b6af5d7d71eae`，0010 为 `7234724187438ecc44956cd59c006e6a0cc2bef663b42018fa3ead47eeef4340`。本轮未改写旧迁移或冻结旧程序源码。独立源码审查对该候选的 spec/quality 结论为 APPROVED、无源码阻断；卷探针及同项目 PG 补证作为独立动态验收证据，亦已完成并在下节列明。

## Compose 实际卷探针

主门使用测试专属 `/tmp` 卷运行文件测试，未覆盖 `deploy/compose.test.yaml` 的 `/assets` 与 `/staging` 真实卷。补充探针在新项目 `learning-system-p0c1-final-volumes2` 使用相同冻结提交与源码包；配置、依赖锁和 A/B 两阶段退出码均为 0。两个不同容器 ID 的 `/assets`、`/staging`、`/evidence` 均指向各自**同一个命名卷及 Source 路径**，且三个卷彼此不同：`learning-system-p0c1-final-volumes2_test_assets`、`learning-system-p0c1-final-volumes2_test_staging`、`learning-system-p0c1-final-volumes2_test_evidence`。容器使用内部网络、无主机端口，测试容器 4 CPU/4 GiB、固定镜像 `sha256:8b4ffdfaa1ec6bb0a12527f00936513238453b83affc7d65faa4a2db04a9b8a1`；探针/应用源码只读挂载。

探针通过公开 `FsAssetStore` 在挂载卷上写入原字节，PDF 51 字节（SHA-256 `904636248025ad20fb9c6bd8b700179a2a42edb5df3636e926c7e09055ee3f75`）、PNG 15 字节（`746f308b1865d235abc32e8d19cf3d7512342d4bb887f905bd05296b124eb0db`）、Notebook 68 字节（`e60b1dd5444bb8b1e8adb9f808058b2b953b57c3a359fae605bae25721687e44`）；存储键为对应 `sha256/<前两位>/<完整摘要>`。B 阶段在另一容器重开，逐字节读回与大小、摘要、键保持一致。以声明 200000 字节、可信上限 100000 字节尝试失败上传，仅留 65536 字节的 `.incoming` 待对账文件，无摘要定稿或 `.finalizing` 文件。该探针仅验证文件适配器在真实卷上的最小原字节持久性与失败残留，**不验证**媒体完整解析、应用层资产授权、PostgreSQL 登记或完整业务流程；这些分别由主门相应用例覆盖或属于后续范围。

原始证据位于 `<user-home>/Documents/ubuntu_Seoul/learning-system-p0c1/evidence/p0c1-final-volume-probe2/`，入口 `result.json` SHA-256 `28509a8a8379f5fc557328df675ef3dbc321832d626e70dfc46c42d2815588f7`；其 39/39 个证据哈希、本地 274/274 源码前后校验均一致，A/B 各退出码 0。首次尝试的控制器误加 `--no-build`，`phase-A` 在创建容器前退出 1；证据保存在同级 `p0c1-final-volume-probe/`，入口 `result.json` SHA-256 `00a85f315b7fb380385f58145b7bd546d6410cc382ea3b6203b08aa248c99867`，16/16 哈希一致，没有探针结果。修正使用另一个全新项目，不覆盖失败现场。

**同项目 PostgreSQL 与四卷补证。** 初始 A/B 探针使用 `--no-deps`，未启动其配置中的 `pg` 服务，因此另在原成功探针项目 `learning-system-p0c1-final-volumes2` 启动 PostgreSQL，核对 `test_pg`、`test_assets`、`test_staging`、`test_evidence` 四个命名卷的完整 inspect 前后 JSON 一致。PostgreSQL 镜像 `sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d`，健康状态 `healthy`；五个测试/升级数据库从空开始，runtime 角色在每库均非 superuser、非 BYPASSRLS、非 DB/schema/table owner。PG 运行时又在新的只读测试容器中执行 B 阶段，`/assets`、`/staging`、`/evidence` 挂载均为 `RW=false`，三份原件及 65536 字节 incoming 再次读回且失败摘要仍不存在；最后 PG 正常停止，四卷保留。补证入口 `<user-home>/Documents/ubuntu_Seoul/learning-system-p0c1/evidence/p0c1-final-volume-pg-readonly-complete/result.json` SHA-256 `2d25118aacb0f91b7a0bdcf2e9c1043da6ac4d747ea100e8d04bb9819ea20b2d`，`success=true`、13/13 阶段退出 0、60/60 证据哈希一致、274/274 源码前后校验为真，既有成功探针证据未改变。

第一次 PG 补证已证明卷、健康、五空库及角色，但控制脚本在创建只读 B 容器前中止，结果为 `success=false`，且归档中没有 `readonly-B` 阶段；具体中止原因未纳入该证据目录。原始结果位于同级 `p0c1-final-volume-pg-supplement/`，SHA-256 `75483d501b388e454df0177590508b012dafe81adaf419967c038016def3944f`，51/51 证据哈希一致。随后修正控制脚本并获得上述 13/13 成功结果，未改冻结应用源码或擦除失败历史。首次 `--no-build` 失败也如前述保留。

## 范围与后续

C1 仅覆盖本地不可变原字节、PostgreSQL 元数据及精确授权使用、v3 Figure/Attachment、固定 Reading/Release 和只读孤儿报告。PDF/PNG/Notebook 是有限签名/JSON 形状检查，非完整文件解析或安全扫描；`application/octet-stream` 是通用回退。C2 持久作业、C3 快照交换、C4 备份恢复、P1 HTTP/前端与生产部署均未由此验收。
