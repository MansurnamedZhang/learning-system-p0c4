# 知织 · KnowWeave

以“块”为最小修订单元的个人学习与知识系统。当前实现是 Rust、PostgreSQL 18 与私有文件存储内核，包含内容修订、组合、个人阅读、关系审查、原件、持久 Worker、快照交换及正在实现的备份恢复。

**最新同步：2026-10-23 · Task6候选代码 64755f6，最终同源native 0/16。Task6、Task7–9与C4仍OPEN；未签发CompleteBackup，未开放生产。**

候选源码检查点：`64755f651e3c0a30fbce1da132a5bec77ed9eb70`；前提交 `2741d3d59986649242c4b6a7a6ed04c662f0cf3f`。这是开发源码身份，公开仓库保持自己的快照提交历史，不把来源 ID 当作公开仓库同名提交。

64755f6 有限前置已独审通过：新执行严格 Rust 542通过、0失败，133项ignored未执行；Root Python 12模块300项（299通过、1项DAC跳过），ordinary Python DAC另1通过。两批新执行均使用64755f6包，自然退出0、精确移除本批容器且旧资源图不变。Root OS三项仍为2741d3d的实际各1/0/0；其完整598项未变源输入（含381项Rust/Cargo/toolchain及SQL/fixture）已逐字比对兼容，不声称64755f6重跑RootOS。这些均非native，accepted仍0/16；不授全774、Task8或生产资格。

旧2741d3d首轮P6在source容器created／准备阶段因PG_BINDS_MODE失败，未进入业务测试；失败与HOLD原件保留。64755f6的R3仅修固定snapshot socket RW挂载的资源规范适配，缺失、null或空Binds均拒绝；不在控制capsule访问未挂载的宿主卷，不签发原FD或业务效果权威。新647 P6在独立消费者构建阶段失败并HOLD；尚未进入精确测试，仍0/16。

另两项formal quiesce取消组件实际2/0/0并独审接受，实际源码与原sealed ELF仍属2741d3d；64755f6仅经完整输入字节比对后兼容保留该组件信用，不是新ELF执行或native场景通过。

正常公开恢复资格仍必须持有真实 CompleteBackup。本轮私有管理流程以同源 SourceLocalPin／不透明效果对象进行单机 rehearsal；不签发 CompleteBackup，不开放生产恢复或服务。缺少有效效果证明仍 Unknown/HOLD，禁止回源猜测、领养或重发。

[Task6 实现、验证与剩余门](docs/p0c4-recovery-normalization.md)。

新增[快照首写与验证记录](docs/p0c4-snapshot-components.md)：C2+D0 已将原 P1 领取接到实际文件写入及 verifier，复用绝对 deadline、累计预算和原收束过程。实际新增六项 C2 PG 用例与九项回归通过；文件事实不代表业务提交、NoEffect 或 AlreadyCommitted。

**历史分源基础与旧S1记录（保持当时范围）：** 旧令牌、三次重试上限、未知快照副作用、非空派生结果/阅读校验及收据同步失败已完成真实隔离测试与独立复核，基础现场进度 5/5（前三项509956e，后两项0df5128）；snapshot 正向证据链和最终同源回归仍待完成，Task6整体保持OPEN。旧失败及 UNKNOWN 原样保留。 新增可选快照代理的写入禁用基础；Linux 5 项普通测试及 6 项 Root 权限测试通过，真实只读部署已有首个客户端以78拒绝执行，整体仍因旧网络保护检查HOLD，未授予部署通过；正向恢复仍待验。详见[快照代理](docs/p0c4-snapshot-broker.md)。

## 进度与文档

- [项目最新进度](docs/项目最新进度.md)：已运行结果、当前补验与未完成边界。
- [实施路线与系统架构](docs/KnowWeave实施路线与系统架构.md) · [离线 HTML 路线图](KnowWeave实施路线与架构.html)。
- [架构设计文档集](docs/architecture/README.md) · [离线 HTML 架构设计](KnowWeave架构设计.html)。
- [P0-C4 Task2、3、5 补验与收尾](docs/P0-C4补验与收尾.md)。
- [系统设计方案](docs/学习系统设计方案.md) · [批准记录与实施路线](docs/设计批准与实施路线.md)。
- [公开同步说明](docs/公开同步说明.md) · 文件与来源清单（私有或未公开参考）。

| 阶段 | 状态 |
| --- | --- |
| P0-A、B1–B4 | 各阶段隔离验收通过；非生产 |
| P0-C1、C2、C3 | 各阶段隔离验收通过；非生产 |
| P0-C4 | Task2/3/5保留原限定范围；Task6候选64755f6，最终同源native0/16（五基础＋十一快照）；旧5/5与D1组件为分源历史；Task6–9/C4/Complete与生产OPEN |
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
