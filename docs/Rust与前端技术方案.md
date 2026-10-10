# Rust 与前端技术方案

2026-09-17 · 执行用户“用 rust+前端去做”的技术栈调整

2026-09-30 状态校正：Rust workspace 有 core、assets、db、worker、backup 五个 crate；C3 已隔离验收，C4 实施中。完整分层事实见[项目架构文档入口](KnowWeave架构设计总览.md)。HTTP、登录及前端归 P1-A；Axum、React + TypeScript + Vite 为建议组合，依赖在 P1-A 固化。

## 技术选择

沿用已批准的产品设计、PostgreSQL 权威库、不可变块修订、组合与个人层、关系版本以及原件存储。正式后端采用 Rust；浏览器前端建议采用 React + TypeScript + Vite。原 Python 施工单归档，Python 数据库对比实验仍作为历史证据保存。

| 层 | 选择 | 职责 |
|---|---|---|
| 后端运行时与 HTTP | Rust / Tokio / Axum | API、身份提取、请求校验、后台入口 |
| 数据访问 | SQLx + PostgreSQL 18 | 参数化 SQL、显式事务、迁移、连接池 |
| 领域契约 | Rust 类型、Serde、版本化 JSON | 块、用途、命令、错误与响应结构 |
| 前端 | React + TypeScript + Vite | 连续阅读、块编辑、关系面板和学习工作台 |
| 状态 | 服务端为权威，前端保留未提交草稿 | 只有服务器确认后才显示已保存；冲突不丢草稿 |
| 资产 | 持久化文件卷、可替换存储接口 | 原件和图片；二进制不塞进块 JSON |
| 测试 | Rust 单元/集成测试、TypeScript 检查、前端行为测试 | 真实 PostgreSQL 事务与权限；UI 只验证用户行为 |
| 部署 | Linux / Docker Compose / 同源 Web + API | 前端构建为静态文件；后端和 Worker 共用业务 crate |

Axum 采用 Tokio/Tower 生态，SQLx 提供 PostgreSQL 异步访问，适合当前模块化单体。首版先使用参数化运行时 SQL；不能将这种查询写法称为已完成编译期 SQL 校验。[Axum](https://docs.rs/axum/latest/axum/)、[SQLx](https://docs.rs/sqlx/latest/sqlx/)

React 组件采用 TypeScript；Vite 负责开发与静态构建。浏览器只通过 API 读写数据，不连接数据库。[React TypeScript](https://react.dev/learn/typescript)、[Vite](https://vite.dev/guide/)

具体依赖由 Cargo.lock、package-lock.json 锁定；只使用验证过的稳定版本，不因官方文档出现新版本就自动升级。

## 项目边界

```text
learning-system/
  Cargo.toml / Cargo.lock
  crates/
    learning-core/             # 内容契约、校验、摘要、领域错误
    learning-db/               # PostgreSQL 事务与查询
    learning-assets/           # 已实现：原字节与 staging
    learning-worker/           # 已实现：持久任务运行角色
    learning-backup/           # C4 实施中：清单、封存、预检与只读绑定
    learning-api/              # 待建设：HTTP、身份、静态前端入口
  apps/web/                    # 待建设：建议 React + TypeScript
  migrations/                  # PostgreSQL 迁移
  deploy/                      # 构建与隔离验证配置
  docs/                        # 运行、接口和验收说明
```

crate 是代码边界，不是微服务。当前已有 core、assets、db、worker、backup 五个 crate；API 与 Web 目录属于目标结构。backup 存在不表示实际恢复已验收。程序与原件库继续分开，历史资料规模不作为部署容量保证。

## P1-A 浏览器切片（待建设）

早期“P0-A 提前接入 API”建议已由批准记录中的分期取代。当前先完成 P0-C 的 C3/C4，再在 P1-A 接入受保护 HTTP API 与浏览器内容工作台：创建知识/笔记块、连续阅读、插入图片与猜想、保存修订、查看历史、处理冲突。它复用已验收内核；学习任务、尝试和能力验收归 P1-B。

账号、会话、失效、HTTPS 与 HTTP 授权在 P1-A 统一设计和验收；原稿的开发访问密钥不作为已批准或已实现的登录方案。业务请求不能自行声明可信作者，浏览器不持有数据库凭据。

网络/API 失败时前端保留用户输入；相同请求重试复用幂等键。冲突响应只向有权限的调用者提供新工作头；前端保留当前草稿供对照。旧修订只读，不能通过“查看旧版”静默覆盖当前头。

## 不变的验收要求

- 创建修订、推进工作头和保存幂等收据在同一 PostgreSQL 事务内。
- 同键不同请求拒绝；两个设备从同一基础修订提交时，一个成功、另一个明确冲突。
- 读取正文、历史和列表均使用当前权威授权；权限不足与不存在采用相同外部结果。
- 用户输入以文本渲染，首阶段不执行原始 HTML；后续受控 Markdown/公式渲染仍需验证。
- 图片、关系、位置、结构环、原子发布和恢复不能因为语言切换而省略测试。
- 先前实验的 4 项权限失败保持原记录；引入跨块载荷时必须建立对应的正式回归。

本地已检测到 Rust/Cargo 1.97.0、Node 26.5.0、npm 11.17.0；版本可用不等于应用已编译成功。实际构建与测试结果另记。
