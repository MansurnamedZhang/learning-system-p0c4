# P0-C2 专用 Worker 容器验收夹具

本夹具只用于全新隔离 Compose 项目。`compose.worker-acceptance.yaml` 与 `compose.test.yaml` 叠加使用，但**只启动 `pg` 与 `worker-acceptance`**；`test` 服务拥有管理凭据，不能作为 Worker 的替身。另建第六个空数据库，使用本批独立卷、内部网络和一次性密码。生产及旧隔离项目不在范围内。

## 构建与接口

先选本批唯一的 `WORKER_ACCEPTANCE_PROJECT`（例如带随机后缀的 `learning-system-p0c2-task6-...`）。**每条** Compose `config/up/run/stop` 命令都显式传 `-p "$WORKER_ACCEPTANCE_PROJECT"`；叠加文件也要求同名环境变量，防止落入基础文件的固定 `name`。启动前检查项目、容器、网络、卷标签均不存在：

```sh
project="$WORKER_ACCEPTANCE_PROJECT"
test -n "$project"
docker compose ls --all --format json | PROJECT_NAME="$project" python3 -c 'import json,os,sys; assert all(x["Name"] != os.environ["PROJECT_NAME"] for x in json.load(sys.stdin))'
test -z "$(docker ps -aq --filter "label=com.docker.compose.project=$project")"
test -z "$(docker network ls -q --filter "label=com.docker.compose.project=$project")"
test -z "$(docker volume ls -q --filter "label=com.docker.compose.project=$project")"
for suffix in test_pg test_evidence test_assets test_staging; do
  if docker volume inspect "${project}_${suffix}" >/dev/null 2>&1; then exit 1; fi
done
if docker network inspect "${project}_test" >/dev/null 2>&1; then exit 1; fi
for name in "${project}-pg-1" "${project}-worker-first"; do
  if docker container inspect "$name" >/dev/null 2>&1; then exit 1; fi
done
```

先只解析**基础 Compose 文件**并启动本批 PostgreSQL。此时尚无任务 ID 或 Worker 二进制，绝不叠加 Worker 文件：

```sh
docker compose -p "$WORKER_ACCEPTANCE_PROJECT" -f deploy/compose.test.yaml config
docker compose -p "$WORKER_ACCEPTANCE_PROJECT" -f deploy/compose.test.yaml up -d pg
```

在这个项目内创建第六个干净数据库，供下文的管理容器迁移和建立业务夹具。

在固定、离线 Rust 1.97 **无管理密钥的构建容器**中从冻结源码只编译、不运行数据库测试：

```sh
cargo test --offline --locked -p learning-worker --example acceptance_fixture --no-run
cargo build --offline --locked -p learning-worker --bin learning-worker --example acceptance_fixture
```

把 build target 中的**单个** `learning-worker` debug 二进制导出到本批宿主私有目录，检查模式为 0755 并记录 SHA-256；不得把源码树或整个 target 挂入 Worker。例如构建容器使用 `CARGO_TARGET_DIR=/target` 时：

```sh
install -d -m 0700 "$host_dir"
test ! -e "$host_dir/learning-worker"
docker cp "$build_container:/target/debug/learning-worker" "$host_dir/learning-worker"
test "$(stat -c '%a' "$host_dir/learning-worker")" = 755
sha256sum "$host_dir/learning-worker"
```

默认 debug 构建必须实际触发 Worker 的 `KNOWWEAVE_WORKER_TEST_GATE_DIR` 闸门；若第一容器没有在 `/gate/claimed` 停住，验收立即失败。`WORKER_ACCEPTANCE_BIN` 指向该二进制绝对路径。`WORKER_ACCEPTANCE_IMAGE` 必须是本批固定离线镜像 digest。`WORKER_ACCEPTANCE_DB` 为第六个空数据库名，`WORKER_ACCEPTANCE_JOB_ID` 在管理夹具 `seed` 完成后从 JSON 取得；它们都不是秘密。Worker 启动时在容器内读取密码文件构造 `DATABASE_URL`，不得把密码或 DSN 作为 Compose 环境值、宿主命令行或日志参数。

管理夹具 `target/debug/examples/acceptance_fixture` 在**另一容器**执行，使用本批 `learning_admin` 与 `learning_runtime` 的 `ADMIN_DATABASE_URL`、`DATABASE_URL`。`cargo test --offline --locked -p learning-worker --example acceptance_fixture -- --test-threads=1` 也必须在这个持有本批 admin/runtime secret、连接第六库的隔离管理容器运行，不能在无密钥构建容器运行。测试自身会建一个独立待处理任务，因此后续断言按精确 `job_id/business_key` 过滤，不假定全库只有一行任务。仅管理容器可见 admin secret；`seed` 还需要可写的 `ASSET_ROOT`（同一权威资产卷）及独立可写 `STAGING_ROOT`。旧 token 使用本批宿主私有目录（0700）挂到**管理容器**的 `/private-token`：捕获时目录可写、`OLD_LEASE_TOKEN_FILE=/private-token/old-token` 预先不存在，`capture-token` 用 `create_new` 创建 0600 文件；fence 时在另一管理容器只读挂同一目录，Worker 始终不挂它。管理容器须在内部构造 DSN，不把密码写入 `docker inspect`、宿主 argv 或原始日志。每次调用 stdout 恰有一行 JSON；非零退出码 78 表示验收失败，stderr 不含 DSN/token。

完成管理容器的真实测试与 `seed` 后，确认 `WORKER_ACCEPTANCE_BIN` 是已导出的单个可执行文件、`WORKER_ACCEPTANCE_JOB_ID` 来自 `seed.job_id`，再**首次**叠加 Worker 文件运行 `config`，逐项核对仅有 `runtime_password` 一个 secret、只读 `test_assets`、只读二进制、私有 `/staging`、`/gate`、`/tmp` tmpfs、内部 `test` 网络且没有宿主端口。每次用不同容器名运行同一服务：

```sh
test -f "$WORKER_ACCEPTANCE_BIN"
test -n "$WORKER_ACCEPTANCE_JOB_ID"
docker compose -p "$WORKER_ACCEPTANCE_PROJECT" -f deploy/compose.test.yaml -f deploy/compose.worker-acceptance.yaml config
docker compose -p "$WORKER_ACCEPTANCE_PROJECT" -f deploy/compose.test.yaml -f deploy/compose.worker-acceptance.yaml run -d --no-deps --name "$WORKER_ACCEPTANCE_PROJECT-worker-first" worker-acceptance
```

| 命令 | 输入 | 成功 JSON 与作用 |
| --- | --- | --- |
| `seed` | 管理和运行 DSN、两个资产路径 | `job_id/actor_id/space_id/block_id/revision_id/asset_id/asset_sha256/business_key`；通过 `put_from_file → register_verified → VersionedContentStore::create(v3 Figure) → JobStore::dispatch_pending` 建立一个 `queued` job，初始 `attempt_count=0`。仅 actor、space、grant 初设使用管理 SQL。 |
| `snapshot JOB_ID` | 运行 DSN | `status/attempt_count/output_digest/lease_expired_by_db_clock/result_count`；只读，空摘要为 JSON `null`。 |
| `capture-token JOB_ID` | 管理和运行 DSN、绝对路径 `OLD_LEASE_TOKEN_FILE` | 仅在首次尝试 `running`、数据库租约仍有效、无输出/结果时，以 `create_new` 建 0600 普通文件并写当前 token；写后复核首次租约仍有效。只输出 `captured/attempt_count/lease_current/token_file_sha256`，不输出 token。 |
| `fence JOB_ID` | 管理和运行 DSN、同一个 `OLD_LEASE_TOKEN_FILE` | 要求已有 0600 普通文件；先以只读管理查询确认第二次尝试正 `running`、租约按数据库时钟仍有效、当前 token 非空且不同于文件中的旧 token，再通过运行角色 `JobStore` 检查 renew/checkpoint/succeed/fail 都返回 `false`，最后复核第二租约仍有效且未易主。输出四个布尔值与 `token_file_sha256`；须和捕获时 hash 一致。旧 token 文件只挂管理容器，内容不得进入 argv、环境值或日志。 |
| `cancel JOB_ID` | 运行 DSN | `{"cancelled":true}`；调用 `JobStore::cancel`。 |
| `revoke ACTOR_ID SPACE_ID` | 管理和运行 DSN | `{"revoked":true}`；管理 SQL 删除恰好一条 grant。 |

## 必过的真实容器断言

1. `seed` 的 JSON 对应一条真实 `job_outbox`、一条 `job`、一条 `block_asset_use`；源 PNG 是与既有测试相同的 16 字节。对实际权威卷中的 `sha256/<前两位>/<完整摘要>` 文件做 size=16 和 SHA-256 复核，等于 `seed.asset_sha256`；管理 SQL 不手造业务事件或任务。
2. 第一只独立 Worker 容器在闸门写入 `/gate/claimed` 后，执行 `capture-token`；其前后数据库快照均须显示 `running/attempt 1`、租约按数据库时钟有效且没有结果。保存 0600 文件的 SHA-256；在租约仍有效时对该容器发真正 SIGKILL，保存 PID、信号与退出码。等 `lease_expires_at<=clock_timestamp()` 为真；此时结果仍为零。
3. 第二只独立 Worker 容器在它自己的闸门停住，显示 `running/attempt 2` 且新 token 不同。执行 `fence`，四项均为 `false`、token 文件 hash 与首次捕获一致、结果仍为零。释放第二闸门，要求退出 0、`succeeded/attempt 2`、64 位输出摘要和恰好一条 `asset_integrity_result`。逐项核对 job、outbox、space、block、revision、asset、SHA、size、结果 `output_digest` 与 seed JSON 和实际卷字节一致；再次运行同一 job 的 Worker，要求状态、attempt、摘要和结果行都不变。
4. 另为取消与撤权各 `seed` 一个独立任务。各自容器在首次领取闸门停住后分别执行 `cancel`、`revoke`，放行并等待退出；取消应为 `cancelled`，撤权应为 `failed`，两者输出摘要为空、结果数为零。
5. 对每只 Worker 容器独立保存 `docker inspect` 中的 Image、Mounts、Config.Env、Config.Cmd、Network、PortBindings、CPU/内存与退出状态；在容器内验证 admin/postgres secret 不存在或不可读，`/assets` 只读，源码、target、证据卷及 Docker socket 均未挂载。管理容器与 Worker 的凭据边界不得混淆。

成功结果使用管理容器内的参数化只读查询逐列核对，不仅凭 `status=succeeded` 判定：`job.id/outbox_id/idempotency_key/output_digest`，`job_outbox.actor_id/payload` 中的 space、block、revision，`asset_integrity_result` 中的 space、block、revision、asset、sha256、byte_size、output_digest，以及 `asset.storage_key`。这些值须与 seed JSON、`block_asset_use`、权威卷中 `storage_key` 指向文件的实际 16 字节与 SHA-256 相符；`asset_integrity_result.output_digest=job.output_digest`，结果数为 1。参数只传 job UUID，不能用字符串拼 SQL，也不能将 token/密码写入查询日志。

原始 stdout/stderr/退出码、SQL 快照、Compose config、卷/网络/容器 inspect、PG 停止状态和源码前后 SHA 均需入证据清单并逐文件复核。真实 PostgreSQL 与容器验收未跑之前，此文件只定义可执行协议，不代表 P0-C2 已验收。

所有场景证据保存后，仅用基础文件停止本批 PostgreSQL，不需再解析带变量门槛的 Worker 文件：

```sh
docker compose -p "$WORKER_ACCEPTANCE_PROJECT" -f deploy/compose.test.yaml stop pg
```
