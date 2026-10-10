# P0-C4 `pg_restore` 子进程端点绑定设计（已确认实施）


实施确认：用户于 2026-09-30 在收到本设计与计划后指示“继续”，按逐任务实现与独立审查推进。服务器上传及新批次执行仍沿用逐文件授权。

## 目标与现有证据

P0-C4 的 Task 3a 已在新 PG18 目标证明 SQLx 预检使用同一事务、双锁与精确 Docker 容器；Task 3b 已在另一全新批次证明同 PostgreSQL system identifier 和数据库 OID 的物理克隆仍会被原容器观察器拒绝。证据见 [C4 验证记录](../../p0c4-verification.md)。这些证据不覆盖 `pg_restore` 子进程。

当前 `PgRestoreSpec` 接受宿主 host/port，以宿主路径启动 `pg_restore`，并以宿主密码文件路径设置 `PGPASSFILE`。路径检查与随后打开之间存在替换窗口；`RestorePreflight::restore_database_linux` 在子进程端点得到证明前写入持久的恢复尝试标记。这条 crate 内路径虽然没有调用者，仍不能直接接通。

本设计只交付**只读子进程准入证明**：在精确容器 ID 中证明固定 PostgreSQL 18 客户端可运行，`learning_admin` 能经该容器的 Unix socket 对出生证明绑定的数据库作固定只读查询，且该查询能看到 SQLx 事务持有的两把随机锁、同一 backend PID 与数据库 OID。探针前后还要重验事务与容器身份；数据库 system identifier/OID 或 Docker `StartedAt` 单独相等都不足以证明同一正在运行的 PostgreSQL 实例。本轮不读取待恢复 dump、不生成尝试标记、不执行 `pg_restore` 导入，不签发 `CompleteBackup`，不放行服务。

## 方案选择

| 方案 | 端点依据 | 决定 |
| --- | --- | --- |
| 宿主 `pg_restore --host/--port` | DNS、端口和宿主密码路径，不能绑定已证明的容器 | 不采用 |
| 精确容器 ID 内的固定本地 socket 命令 | 已固定的 Docker daemon、容器/镜像/挂载/启动事实和容器内 Unix socket；不需要新增密码挂载 | **本轮推荐的只读准入门** |
| 新增专用 `.pgpass` 挂载及只读 rootfs | 可独立定义密码认证，但改变出生证明、Compose 和 guard 的挂载契约 | 留给实际导入设计，须另验 |

PostgreSQL 18 的 `--host` 参数以 `/` 开头时表示 Unix socket 目录；[官方 `pg_restore` 文档](https://www.postgresql.org/docs/18/app-pgrestore.html)说明此行为。Docker `exec` 默认继承容器创建时的环境，因此宿主 `env_clear` 不足以清除 `PG*`；[Docker 文档](https://docs.docker.com/reference/cli/docker/container/exec/)说明了继承行为。本轮命令在容器内使用固定的 `/usr/bin/env -i`，后接绝对路径的 PostgreSQL 客户端。Docker CLI 使用已校验的 `/usr/bin/docker`，只接受 64 位十六进制容器 ID，不接受名称。

## 信任边界与命令契约

- 宿主 root、Docker daemon 和固定摘要的 PG18 镜像是可信管理边界；本门不声称抵抗拥有 root/Docker 管理权限的攻击者。普通 runtime、Worker、调用方参数与备用 PG 端点不可信。
- 仅持有 `BoundTargetGuard` **和仍存活的 `LockChallenge<Transaction<'static, Postgres>>`** 的内部代码能取得短生命周期、不透明的 `ExactRestoreChildTarget`。创建前及只读探针前后重查 daemon、精确容器 ID、镜像摘要、网络、卷挂载、`StartedAt`、两把事务锁及同一 backend PID/数据库 OID；任何漂移或事务失效都失败关闭。固定 socket 查询通过现有严格双锁行解析，不把锁值写入证据。
- 固定进程形状为 `/usr/bin/docker exec --interactive --user 999:999 <exact-id> /usr/bin/env -i LC_ALL=C PGCONNECT_TIMEOUT=10 <absolute-client> ...`。只读探针分别运行固定的 `/usr/lib/postgresql/18/bin/pg_restore --version` 和 `psql -XAt --no-password --host=/var/run/postgresql --port=5432 --username=learning_admin --dbname=<birth-database> -c <fixed-read-only-SQL>`。调用方不能提供 host、port、用户名、参数、可执行文件或密钥路径。不得使用 shell、TTY、`docker exec --privileged` 或带密码的环境变量。
- 本轮不创建 `.pgpass`：`--no-password` 的本地 socket 连接若需要密码就失败关闭。现有 `/run/secrets/admin_password` 是原始口令字节，绝不能当作 `.pgpass` 使用。探针必须在固定 socket 上观察同一事务的双锁；容器不重启但 postmaster 重启会使事务/锁消失，必须拒绝。探针成功只证明当前只读本地连接能力，不固定未来实际导入所需的认证策略。
- 新子进程执行器必须在**读取期间**对 stdout/stderr 各施加字节上限和总截止时间，固定只读 SQL 还须设置服务端 `statement_timeout`。超限或超时就按已创建的宿主 Docker CLI 子进程句柄终止、`wait` 回收并拒绝；由于杀宿主 CLI 不必然杀死容器内 `psql`，失败批次还须在核对精确容器 ID 后停机隔离并确认无进程继续运行。若身份或停机无法确认，记录 `UNCONFIRMED_UNUSABLE`，不得复用目标或宣称通过。不能复用先 `.output()` 无界缓冲、结束后才检查大小的现有 Docker helper。原始 stderr 不进入持久证据。
- 固定输出仅记录成功/失败、容器和镜像摘要、数据库身份及非敏感命令版本；不记录 `PG*` 值、原始 Docker/psql stderr、密码、完整 DSN 或随机锁值。保持旧的只读 guard/3a/3b 门和既有失败证据不变。

## 首次写入前的强制边界

旧宿主 `PgRestoreSpec::run_from_open_file` 与 `RestorePreflight::restore_database_linux` 在新子进程契约完成前不得成为可调用的写路径。只读探针必须在任何 `restore.attempt` 标记之前完成。即便本设计的只读门通过，也**不能**推断真实导入时第一笔写入一定走同一端点；后续须在全新隔离目标、完整包与经独立验证的子进程执行器上另做小型 dump 写入验收，且失败目标保持隔离。实际资产闭包、源租约失效、`CompleteBackup` 与服务放行仍是独立门。

## 验收边界

本地单测覆盖容器名/TCP/任意 `PG*`/替换容器/错误数据库、密码认证要求、socket 指向同 ID/OID 克隆、容器或 postmaster 启动漂移、进程输出超限/超时及隔离失败的拒绝。标准 PG 容器中 postmaster 独立重启且 Docker `StartedAt` 不变的情形使用依赖注入单测；现场只要求容器重启负例，除非另有经过出生/guard 契约验证的监督进程夹具。错误数据库和密码认证要求采用依赖注入或**另建出生前即固定认证策略的新目标**测试；绝不修改已签发出生证明/已通过 Pin 的库或 HBA。Linux/PG18 验收只能使用用户对精确源码包和 runner 单独授权的新项目、子网和卷；以同一 socket 的双锁/PID/OID 只读挑战、精确停机、留卷、哈希与无 pending 文件为通过条件。若加入同 ID/OID 克隆负例，须同批另外创建全新隔离克隆项目；未经现场验收时只称本地候选。
