# 03 Rust 模块与代码边界

## 实际代码结构

当前 workspace 是 Rust 2024 edition、Rust 1.97、锁定 Cargo.lock 的五个 crate；Tokio 提供异步运行，SQLx 访问 PostgreSQL 18，Serde 表达版本化 JSON。没有 Axum、React 或 HTTP 登录模块被当作已经实施的应用。

```text
crates/
  learning-core/     领域契约、校验、精确引用与摘要
  learning-assets/   私有原件、安全文件操作、C3 目录包
  learning-db/       权限、事务、图查询、持久任务与原子导入
  learning-worker/   作业轮询、处理器、租约和进程停止
  learning-backup/   C4 清单、封存、收据、恢复预检与绑定
migrations/         0001–0015 追加模式迁移
deploy/             隔离 Compose、初始化、升级夹具
scripts/            受控验收与管理辅助脚本
docs/               设计、边界、实施及验证记录
```

未来 `learning-api` 与 `apps/web` 尚不存在于当前 workspace。部署测试夹具中的旧源码是升级输入，不是另一套正在维护的产品实现。

![crate 依赖与授权边界](diagrams/knowweave-crates.svg)

## Cargo 依赖

| 模块 | 直接依赖本仓库模块 | 明确不承担 |
| --- | --- | --- |
| learning-core | 无 | 不连接数据库、不处理登录、不持有文件权限 |
| learning-assets | core | 不认证 actor，不独立决定业务读取权限 |
| learning-db | core、assets | 不提供浏览器身份验证，不把 SQL 账号等同于用户 |
| learning-worker | core、assets、db | 不决定学习是否掌握，不承担 API 或高权管理 |
| learning-backup | assets、db | 不是普通用户导出，不是已完成的生产恢复入口 |

表中只列直接 Cargo 依赖；backup 可经 db 使用 core 的能力，不应画成不存在的直接依赖。模块共享契约，但原件 I/O、数据库授权与管理操作不能因为处在同一仓库而混用职责。

## 推荐读代码顺序

1. 从 [Cargo.toml](../../Cargo.toml) 与各 crate 的 `lib.rs` 确认模块和公开入口。
2. 阅读 core 的 content、composition、overlay、reading、references，理解身份、位置和固定版本。
3. 阅读 db 的 write、release、reading、relation、impact，理解事务与当前授权。
4. 阅读 assets 的资产存储及快照目录代码，再看 db 的 asset 和 snapshot 模块。
5. 阅读 job 合同、0011–0014 迁移和 worker 处理器，理解 token fencing。
6. 阅读 backup 的 restore_policy、restore_preflight 与 target_binding；再对照 db 的 snapshot/import，区分 C4 恢复与 C3 范围导入，同时核对各阶段状态和公开/内部边界。

示例和验收程序可说明调用方法，但不能替代生产身份边界。对当前内部接口进行 HTTP 包装前，要明确 trusted caller、actor、请求收据和错误投影的责任。

## 模式迁移地图

| 迁移 | 主要职责 |
| --- | --- |
| 0001 | app_user、space、grant、块修订与 mutation receipt |
| 0002 | 组合、occurrence、发布、request key 与旧 release outbox |
| 0003 | 个人层、阅读视图、位置迁移与确认记录 |
| 0004–0006 | 类型化引用登记、关系审查、阅读证据、发布闭包与谱系 |
| 0007–0008 | 影响查询和谱系索引 |
| 0009–0010 | 原件、资源版本、来源定位及块资产使用一致性 |
| 0011–0013 | job_outbox、job、租约函数与资产完整性结果 |
| 0014 | C3 快照作业、独立状态与完成结果 |
| 0015 | C3 原子导入批次回执 |

C4 当前代码包含文件清单、角色恢复规则和目标绑定，不因此产生“已完成 C4 数据库恢复”的事实。追加迁移和旧版升级须保持已发布迁移字节不变。

## 分层约束

纯合同检查靠 core，涉及可见性与原子提交的检查靠 db，实际文件句柄与字节靠 assets。Worker 协调这些能力；它不是授权规则的第二实现。高权备份与恢复不进入普通 Worker 的凭据范围。

数据库约束与延迟触发器负责持久不变量，Rust 入口负责命令校验和可读错误。参数化运行时 SQL 不能称为已经完成全部 SQL 的编译期校验。

代码事实来自 [workspace](../../Cargo.toml) 和 [内容边界](../content-boundary.md)。每个阶段还有自己的 boundary、execution、verification 三类文档，分别回答“做了什么、怎么落地、怎样验证”。
