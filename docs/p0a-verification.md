# P0-A 实施与验收记录

日期：2026-09-19。用户明确要求“开始实施落地”，执行范围为 P0-A 内容内核。当前状态：宿主 PostgreSQL 验证已通过，Compose 验证及独立审查进行中。

## 来源与范围

计划：`../../docs/superpowers/plans/2026-09-17-p0a-block-revision-core.md`。
代码仓库分支：`feat/rust-learning-core`；试作基线：`239a3ba`，保留历史但不视为阶段验收。

本阶段交付两个 Rust crate、独立迁移、契约样例、真实 PG 测试和隔离测试部署配置。后续 API、前端、登录、组合、个人层、语义关系、资产和恢复尚未交付。

## 验证链

| 步骤 | 输入与执行 | 实际结果 |
|---|---|---|
| 本地试作基线 | Rust/Cargo 1.97.0，原 contracts | 5/5 通过；数据库业务为占位 |
| 契约 RED | 新增长度反序列化、ASCII、NUL、版本摘要 | 5 通过 / 4 行为失败 |
| 契约 GREEN | 修正校验与版本化摘要 | 9/9 通过 |
| schema RED | 新库 p0a_schema_red，原试作迁移 | 2 通过 / 2 行为失败：空头、缺少授权锁函数 |
| schema GREEN | 新库 p0a_schema_green，修正迁移 | 4/4 通过；函数与角色 ACL 单独核对 |
| 业务 RED | 保存、读取仍为占位 | revisions 4、authorization 5、concurrency 5、atomicity 2 全部行为失败 |
| 本地最终静态验证 | fmt、全 workspace/all-targets Clippy、全目标编译 | 已通过 |
| 宿主业务 GREEN | Rust 1.97.0 + PostgreSQL 18.6，全 workspace | 29/29 通过；fmt/Clippy/test 退出码均为 0，原始日志已审阅 |
| Compose 独立验证 | 交付配置、空数据库、内部网络 | config 通过；首次 build 因 Docker Hub DNS 失败，正在处理 |
| 独立审查 | 只读检查同一固定源码与全部宿主日志 | 无 Critical / Important / Minor 发现；代码可进入 P0-A 集成，交付仍等待 Compose |

数据库命令使用真实非表所有者的 learning_runtime 连接；learning_admin 只用于迁移、夹具和验证。两者无 superuser、bypassrls、createdb、createrole；运行角色拥有 public 表数量为 0。lock_space_grant 归 NOLOGIN learning_auth_lock 所有，runtime 非其成员；无 PUBLIC 执行权限，临时 schema CREATE 已撤销。

并发验证使用独立数据库连接；撤权竞争通过 pg_blocking_pids 观察实际阻塞后推进。失败注入使用只针对该夹具主体的 PostgreSQL 触发器；先移除触发器，再断言旧头/修订/收据完整回滚和同键重试成功。缺少数据库配置会失败，不跳过。

## 固定源码与原始证据

以下包均含逐文件 SOURCE-MANIFEST.json，服务器在运行前后校验源码不变。

| 包 | ZIP SHA-256 |
|---|---|
| p0a-schema-red-20260919.zip | 7d9f204dadf15125016ae22845e27b4414edea2203dc0fe5f01af9b7fd70321b |
| p0a-schema-green-business-red-20260919.zip | 12e43df226ccc66de0a4472a4f6890305f21e49608f684eb697d171527f67784 |
| p0a-business-green-20260919.zip | 9924a97c1ed1c5273bfe58701294719ecc98d1312606396080bfc3dbb0480437 |

本地包位于 `.runtime/`（不入 Git）。服务器回传原始日志和 JSON 位于 `C:/Users/hans/Documents/ubuntu_Seoul/learning-system-p0/evidence/`。报告区分测试进程真实退出码与外层证据脚本退出码；RED 测试为 101，即使归档脚本为 0 也不记成通过。

## 实施判断与限制

1. 沿用计划指定目录的独立 feature 仓库，先提交试作基线；没有移动课程资料或建立生产服务。若以后需额外 worktree，可以从已保存提交创建。
2. 当前 Windows 环境的技能脚本缺少 basename，改为同格式的计划专属执行账本；没有把环境脚本失败算成业务 RED。
3. U+0000 无法存入 PostgreSQL text/JSONB，因此契约入口明确拒绝；其它空白、换行、Unicode 字符原样保存。未来二进制文本格式需要独立编码契约。
4. 本地下载 chrono 时沙箱 Schannel 报 SEC_E_NO_CREDENTIALS；同一 Cargo 命令经允许在沙箱外完成下载。未关闭 TLS 校验。
5. 本阶段角色权限防止历史 UPDATE/DELETE/TRUNCATE，但终端用户权限由受信任应用边界执行，不把运行数据库账号当作用户级 RLS。历史存在不表示永久可读。
6. P0-A 通过不改变原 PostgreSQL/Neo4j 实验 NOT_PASSED 状态，也不代表整个学习系统、HTTP 授权或生产部署验收通过。

## 独立审查范围的最终判断

- HTTP 身份认证、UI 渲染、传输层断网，以及组合/关系/资产/发布/恢复属于后续阶段；本阶段验证内部库的已提交结果重放，不声称进行了真实网络丢包注入。代价是上线前仍需这些阶段的独立验证。
- 被攻陷的运行数据库凭据可绕过应用读取权限，因此不能授予终端用户。用户级 RLS 不在本阶段范围；代价是必须维护受信任服务边界。
- 完整交换格式 1.0、未来契约版本升级和已有生产数据迁移不在本阶段交付；本次试作不是已验收生产基线。代价是未来格式演进需追加迁移和兼容性测试。
- Compose 可复现性必须实测，不能由静态审查或宿主通过替代；该项保持待完成。
