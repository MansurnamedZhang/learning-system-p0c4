# Learning System — P0-A

Rust + PostgreSQL 的块修订与权限内核。面向后续可复用学习系统；当前提供内部库，尚无 HTTP API、登录和前端界面。

- `learning-core`：严格文本类型、内容摘要、命令、修订、分页及错误。
- `learning-db`：块创建/修订、当前权限读取、幂等收据和事务。
- `migrations`：约束、普通运行角色权限和受限授权行锁。
- `contracts` / `docs/content-boundary.md`：后续前后端共享的样例与语义。
- `deploy`：独立测试数据库与测试运行器；没有生产部署配置。

## 构建和验证

锁定 Rust 1.97.0 与 Cargo.lock。数据库为 PostgreSQL 18，测试角色和迁移需独立管理。不能给浏览器运行数据库凭据或未经认证的 Principal。

```sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked -p learning-core --test contracts
```

完整测试需要显式提供 `TEST_ADMIN_DATABASE_URL` 与 `TEST_DATABASE_URL`。缺少配置会失败，不会静默跳过。前者负责迁移/夹具，后者为无超级用户、无表所有权、无角色管理权限的 `learning_runtime`。只对独立测试库运行；测试会插入夹具和临时故障触发器。

```sh
cargo test --locked --workspace -- --test-threads=1
```

普通测试迁移账号为 `learning_admin`，独立授权锁函数所有者为 NOLOGIN `learning_auth_lock`，运行账号不能是后者的成员。初始化规则见 `deploy/initdb.sh`。迁移自动在 PostgreSQL 事务中执行；对已经应用的迁移不做原地修改。

## Docker 隔离测试

从本目录创建仅用于本项目的随机测试凭据，已存在的文件保留：

```sh
python -c 'import pathlib,secrets; p=pathlib.Path(".runtime/secrets"); p.mkdir(parents=True,exist_ok=True); [(p/n).write_text(secrets.token_hex(32)) for n in ("postgres_password","admin_password","runtime_password") if not (p/n).exists()]'
docker compose -f deploy/compose.test.yaml --profile test build test
docker compose -f deploy/compose.test.yaml --profile test up --abort-on-container-exit --exit-code-from test
docker compose -f deploy/compose.test.yaml stop
```

数据库使用独立内部网络，无宿主端口，限制 2 CPU / 4 GiB；测试运行器限制 4 CPU / 4 GiB。构建时下载依赖，运行测试时离线。停止后保留卷；不把凭据、课程原件或编译目录打进源码镜像。已有卷不会重跑初始化；不要改密钥文件来假定已轮换数据库密码。

## 当前边界

块身份与不可变修订分离，只推进编辑头。历史读取仍需当前权限。文本是纯数据，未来前端必须安全渲染；当前没有 Markdown 执行或 HTML 渲染器。

组合、个人层、语义关系、图片资产、导出恢复、学习事实和 HTTP/前端分别按后续阶段实现。具体测试证据与限制记录在 `docs/p0a-verification.md`；不能把本内核通过等同于整套系统上线或旧 Neo4j 实验通过。
