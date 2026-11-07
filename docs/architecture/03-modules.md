# 03 Rust 模块与代码边界

审查基线使用 Rust 2024 edition、固定工具链 1.97.0 和 Cargo.lock。五个 workspace crate 通过 Tokio、SQLx 与 PostgreSQL 协作。仓库实际入口包括 Worker、快照效果 broker、C4 管理和角色二进制。工作树里的旧升级夹具是固定历史输入。

![五个 crate 的实际直接依赖与目标接口](../diagrams/knowweave-crates.svg)

| Crate | 直接依赖本仓库 crate | 现有入口和职责 |
| --- | --- | --- |
| `learning-core` | 无 | 纯领域类型、内容版本、引用和摘要；表达组合、阅读、关系、任务及快照合同 |
| `learning-assets` | core | `FsAssetStore` 相关安全文件操作、C3 目录包、备份文件保护及本地 journal |
| `learning-db` | core、assets | 内容、组合、发布、阅读、关系、影响、原件、作业与快照 Store；统一事务及当前授权 |
| `learning-worker` | core、assets、db | `learning-worker`、`snapshot-effect-broker`；协调取租、处理、结果和效果证明 |
| `learning-backup` | **core、assets、db** | 源预检、保留、封存、完成收据、恢复、Task9 受限角色。无 Worker crate 直接边 |

`offhost-b1` 显式启用 `independent-verifier` 与 DB 的 `b1-source-peer`。Source、Capture、Source RootLauncher、Receiver 四个 Rust 入口要求该 feature。构建还需要角色和公钥等实际 pin，现场需固定安装与运行准入。现有管理 CLI、内部库入口和历史读取各有不同权限，不能互换。

受审内核树提供领域 Store。B1 预览另设 `apps/reading-preview` 工作区，使用 Axum 及只读数据库角色复用领域读接口。正式工作台、产品登录和持久部署属于后续阶段。Cargo 的 crate 边、容器进程边与信任边界分别审查。[Cargo 根配置](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/08c3687ef39416411af9f21a743fed09ccf8ef3f/Cargo.toml) · [Backup Cargo 配置](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/08c3687ef39416411af9f21a743fed09ccf8ef3f/crates/learning-backup/Cargo.toml) · [DB Cargo 配置](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/08c3687ef39416411af9f21a743fed09ccf8ef3f/crates/learning-db/Cargo.toml)

## 模式迁移

| 迁移范围 | 主要职责 |
| --- | --- |
| 0001 | actor、space grant、块和不可变修订、幂等回执 |
| 0002–0003 | 组合和固定发布、出现、个人层、阅读及位置迁移 |
| 0004–0008 | 类型化依赖、关系及审查、阅读证据、谱系及查询索引 |
| 0009–0010 | 原件、资源与来源片段、块的资产使用合同 |
| 0011–0013 | 持久 outbox 与 job、租约函数、资产完整性结果 |
| 0014–0015 | C3 快照任务、状态及完成结果、原子导入批次回执 |

正式迁移有 15 份，另有运行扩展 SQL。历史升级依赖实际旧二进制及固定迁移字节；改动追加新迁移。P1 会话和学习模型需要新的单独合同及迁移评审。[迁移目录](https://github.com/MansurnamedZhang/learning-system-p0c4/tree/08c3687ef39416411af9f21a743fed09ccf8ef3f/migrations) · [四套旧版升级证据](../p0c4-task8-current-regression.md)

## 代码阅读顺序

1. 从 core 的 content、composition、overlay、reading 和 references 建立版本、位置与授权闭包概念。
2. 查看 DB 的 `write`、`release/write`、`reading/read`、`reading/write` 和 `queries`，核对普通调用与持久事务。
3. 从 assets 和 DB 的 snapshot 顺着导出、交付、导入走一遍。再读 0011–0015、Worker 主入口与 broker。
4. C4 从 `source`、`complete`、`full_restore` 读正常接口，再看 `restore_preflight` 和 `offhost_b1` 的物理与运行准入。现场资格在对应的受控证据中核对。

[DB 公共接口](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/08c3687ef39416411af9f21a743fed09ccf8ef3f/crates/learning-db/src/lib.rs) · [Backup 公共接口](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/08c3687ef39416411af9f21a743fed09ccf8ef3f/crates/learning-backup/src/lib.rs)
