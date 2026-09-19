# Learning System — 内容、组合与发布内核

Rust + PostgreSQL 的块修订、固定组合与原子发布内核。面向后续可复用学习系统；当前提供内部库，尚无 HTTP API、登录和前端界面。P0-A 与 B1 已验收，B1 在 Linux 宿主和全新 Compose 中各通过 68 项测试，见 [B1 验收记录](docs/p0b1-verification.md)。

- `learning-core`：严格文本类型、内容摘要、命令、修订、分页及错误。
- `learning-db`：块创建/修订、固定组合、原子发布、当前权限读取、跨操作幂等和事务。
- `migrations`：约束、普通运行角色权限和受限授权行锁。
- `contracts` / `docs/content-boundary.md`：后续前后端共享的样例与语义。
- `deploy`：独立测试数据库与测试运行器；没有生产部署配置。

## 构建和验证

锁定 Rust 1.97.0 与 Cargo.lock。数据库为 PostgreSQL 18，测试角色和迁移需独立管理。不能给浏览器运行数据库凭据或未经认证的 Principal。

```sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked -p learning-core
```

完整测试需要显式提供 `TEST_ADMIN_DATABASE_URL` 与 `TEST_DATABASE_URL`。缺少配置会失败，不会静默跳过。前者负责迁移/夹具，后者为无超级用户、无表所有权、无角色管理权限的 `learning_runtime`。只对独立测试库运行；测试会插入夹具和临时故障触发器。

B1 升级测试还需要 `TEST_UPGRADE_ADMIN_DATABASE_URL`、`TEST_UPGRADE_DATABASE_URL` 指向另一专用空库，以及 `TEST_P0A_MIGRATIONS_DIR` 指向只含原 `0001_content_core.sql` 的目录。升级库不能预跑完整迁移；测试会拒绝重用已迁移库。每次全量验收建新批次，保留以前的库与证据。Compose 初始化和运行脚本已安排这两个库。

```sh
cargo test --locked --workspace -- --test-threads=1
```

普通测试迁移账号为 `learning_admin`，独立授权锁函数所有者为 NOLOGIN `learning_auth_lock`，运行账号不能是后者的成员。初始化规则见 `deploy/initdb.sh`。迁移自动在 PostgreSQL 事务中执行；对已经应用的迁移不做原地修改。

## Docker 隔离测试

从本目录创建仅用于本项目的随机测试凭据，已存在的文件保留：

```sh
python -c 'import pathlib,secrets; p=pathlib.Path(".runtime/secrets"); p.mkdir(mode=0o700,parents=True,exist_ok=True); p.chmod(0o700); names=("postgres_password","admin_password","runtime_password"); [(p/n).write_text(secrets.token_hex(32)) for n in names if not (p/n).exists()]; [(p/n).chmod(0o644) for n in names]'
docker compose -p learning-system-p0b1 -f deploy/compose.test.yaml --profile test build test
docker compose -p learning-system-p0b1 -f deploy/compose.test.yaml --profile test up --abort-on-container-exit --exit-code-from test
docker compose -p learning-system-p0b1 -f deploy/compose.test.yaml stop
```

数据库使用独立内部网络，无宿主端口，限制 2 CPU / 4 GiB；测试运行器限制 4 CPU / 4 GiB。Docker 基础镜像锁定为本次已验证的官方 Linux/amd64 manifest；其它架构需选择对应官方 manifest 并重新验证。构建时下载依赖，运行测试时离线。Linux 上凭据父目录为 0700，文件为 0644：宿主其他用户不能遍历父目录，容器内 PostgreSQL 可读单文件只读挂载。停止后保留卷；不把凭据、课程原件或编译目录打进源码镜像。已有卷不会重跑初始化；不要改密钥文件来假定已轮换数据库密码。

最终 B1 验收使用标准 Compose build 成功，无代理回退或共享 Docker 服务变更。P0-A 历史验收曾因 Docker Hub DNS 问题使用命令级代理恢复，详情仅适用于其历史记录。重复全量验收须改用全新项目名，确保升级测试从空库开始。

## 当前边界

块身份与不可变修订分离，只推进编辑头。历史读取仍需当前权限。文本是纯数据，未来前端必须安全渲染；当前没有 Markdown 执行或 HTML 渲染器。

组合保存产生固定修订，发布显式选择采用范围；编辑头与发布头分开，多根发布和 outbox 追加属于同一事务。历史读取、发布清单和幂等重放仍按当前权限检查。详见 `docs/p0b1-boundary.md`。

发布前调用 `ReleaseStore.state` 取得工作头及文档专属 publication_token；把令牌填入 `PublishRoot.expected_publication_token`。发生 PublicationConflict 后重新读取状态再形成新请求，不猜测 release ID；对不确定是否提交的原请求先原样重放。

个人层、语义关系、图片资产、导出恢复、学习事实和 HTTP/前端分别按后续阶段实现。测试证据见 `docs/p0a-verification.md` 与 `docs/p0b1-verification.md`；不能把本内核通过等同于整套系统上线或旧 Neo4j 实验通过。
