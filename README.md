# 知织 · KnowWeave

以“块”为最小修订单元的个人学习与知识系统。当前实现是 Rust、PostgreSQL 18 与私有文件存储内核，包含内容修订、组合、个人阅读、关系审查、原件、持久 Worker、快照交换及正在实现的备份恢复。

**最新同步：2026-10-11。进度核对：2026-10-10。源码检查点：`0b6f2a59a25b4a09bf9e30c085d079d4b0d7b399`。本仓库是持续开发快照，P0-C4 尚未整体验收，包含明确标注的 Task6 WIP。没有 HTTP API、登录界面或浏览器前端，生产未部署。**

## 进度与文档

- [项目最新进度](docs/项目最新进度.md)：已运行结果、当前补验与未完成边界。
- [实施路线与系统架构](docs/KnowWeave实施路线与系统架构.md) · [离线 HTML 路线图](KnowWeave实施路线与架构.html)。
- [架构设计文档集](docs/architecture/README.md) · [离线 HTML 架构设计](KnowWeave架构设计.html)。
- [P0-C4 Task2、3、5 补验与收尾](docs/P0-C4补验与收尾.md)。
- [系统设计方案](docs/学习系统设计方案.md) · [批准记录与实施路线](docs/设计批准与实施路线.md)。
- [公开同步说明](docs/公开同步说明.md) · [文件与来源清单](docs/source-sync-manifest.json)。

| 阶段 | 状态 |
| --- | --- |
| P0-A、B1–B4 | 各阶段隔离验收通过；非生产 |
| P0-C1、C2、C3 | 各阶段隔离验收通过；非生产 |
| P0-C4 | 补验实施中，当前 Task5 十一门实际 0/11；整个 C4 未完成 |
| P1-A / P1-B | HTTP、登录、浏览器内容工作台与学习闭环待建设 |
| P2 / P3 | 多领域演进及按需增强待建设 |

## 代码导航

| 目录 | 职责 |
| --- | --- |
| `crates/learning-core` | 内容、关系、审查、阅读、资产与作业契约 |
| `crates/learning-db`、`migrations` | PostgreSQL 持久化、事务、权限与追加迁移 |
| `crates/learning-assets` | 私有原件与安全目录、融合快照包 |
| `crates/learning-worker` | 持久任务与受限执行器 |
| `crates/learning-backup` | 维护窗、备份目录、目标绑定、受控导入、收据及恢复 WIP |
| `deploy`、`scripts` | 隔离验收配置与工具；具体授权与环境边界见各阶段文档 |
| `contracts`、`docs` | 版本化协议、设计、施工单与分项验证记录 |

## 构建与测试

需要 Rust 1.97 与锁定的 `Cargo.lock`。以下是构建命令，不代表本次文档同步重新执行了完整测试：

```sh
cargo fmt --all -- --check
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo test --offline --locked -p learning-core
cargo test --offline --locked -p learning-backup --lib
```

`--offline` 需要已缓存的依赖。数据库及 Linux/Docker 专项要求各自隔离环境、全新库/卷与明确角色，部分现场用例须显式运行；不能把编译、测试列举、Windows 本地通过或历史批次替代现场验收。先阅读对应边界与验证记录，不在已有业务库运行夹具、迁移或故障注入。

## 公开快照范围

本次从固定 Git 源码字节导出并更新文档。源码检查点属于开发仓库历史，仅作来源身份标识；公开仓库保留自己的连续快照提交历史。代码、迁移、协议和二进制资料保持原字节，公开文档中的本机路径、服务器连接及私有证据链接做脱敏处理。

凭据、课程原件、编译缓存、测试包、服务器原始日志与私有验收记录不在本仓库。分项审查与历史结果各有范围，不能称整个 HEAD、恢复能力、C4 或生产已验收。
