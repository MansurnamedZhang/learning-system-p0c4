# P0-C3 Task 7 执行手册

这是候选验收协议，不表示已执行。仅在精确 commit/源码 ZIP SHA/逐文件 manifest 获单独授权后，在全新 Linux 隔离目录运行。不能继承之前 C2/C3 包的上传授权。禁止生产、旧实验项目、旧数据库、旧证据、已有卷复用及全局 prune。

## 本地冻结前

```powershell
python -m unittest discover -s deploy/tests -v
cargo +stable test --offline --locked -p learning-core -p learning-assets
cargo +stable test --offline --locked -p learning-db --lib
cargo +stable test --offline --locked -p learning-worker --bin learning-worker
cargo +stable test --offline --locked --workspace --no-run
cargo +stable fmt --all -- --check
cargo +stable clippy --offline --locked --workspace --all-targets -- -D warnings
git diff --check
```

没有显式隔离 DSN 时只编译 PG 集成目标，不执行、不跳过后计为通过。确认 0001–0013 对 `deploy/c3-migrations-0001-0013.sha256` 完全一致；该基线来自 C3 开工前 `2dfb5ae`。只打包已跟踪源文件，排除 target/.git/.runtime/.superpowers/Python cache。记录 clean commit、ZIP SHA、文件数和 manifest SHA，再请求该份包的独立上传授权。

## 新 Linux 项目预备

使用固定 Rust 1.97 镜像与当前 Cargo.lock 的缓存。Dockerfile.test 在构建阶段取得依赖；有完整缓存时所有验收命令均 offline/locked。冻结 fixture-runner、旧 b2 源码和旧 Cargo.lock 不可修改。worker 的新增 chrono 仅 dev-dependency，沿用 workspace 已有版本；不能仅凭本机缓存声称固定 Linux 镜像已可编译。

准备新的 `.runtime/secrets/{postgres_password,admin_password,runtime_password}`：各自独立随机十六进制密码，目录 0700、文件 0600，不打印密码。这些文件不进入源码包。禁止把 DSN/password 放到宿主 argv 或 Compose environment。管理/测试容器内从 secret 文件构造 DSN，Worker 只有 runtime secret。

`deploy/c3-databases.json` 是固定清单，`initdb.sh` 创建 11 个独立库：主测试、p0a/b1/b2/b3-schema 四旧库、C3 0013升级、import A/B、0014导入升级、C3 process source/target。run-tests 的第一步通过所有 admin DSN 检查实际数据库名互异且 public schema 无业务对象，再运行任何 producer/migration。它绝不 drop/reset 旧库。

构建**这份已冻结源码**的 test image，固定其不可变 image ID；构建和接受环境需由该批授权准备，不在脚本里自动联网安装。Dockerfile.test 同时构建 debug Worker 和管理例程，将 /app 与 Cargo cache 交给非 root 测试用户。把镜像中 `/app/target/debug/learning-worker` 导出为单个普通 0755 文件；只读挂它进 Worker，不能挂整个 target/源码。驱动会复核宿主二进制 SHA 与管理镜像内同一构建输出相等。

```sh
export C3_PROJECT=learning-system-p0c3-task7-<本批唯一随机后缀>
export C3_IMAGE=sha256:<本批test镜像64位ID>
export C3_RUNTIME_IMAGE=rust:1.97.0-bookworm@sha256:b5a086f64ffecaa4e283063184770107915756739598173e1f5712d6b34b84d0
export C3_WORKER_BIN=/本批私有目录/learning-worker
python3 deploy/c3_acceptance.py --evidence /全新且在源码目录外的证据目录
```

运行者需要 Docker 权限，**任何容器不挂 Docker socket**。驱动调用的每条 Compose 命令均显式 `-p`，要求带随机后缀的新 task7 项目名，并检查已有容器/网络/卷标签及固定卷名；发现旧对象立即拒绝。它不负责上传、生产部署或修改主机服务。

## 明确阶段与硬断言

1. 记录 source-before、0001–0013 校验和、单个 Worker hash、完整 Compose config。oneshot init 只接新空卷且无网络/密钥，设置快照/控制/目标/资产/证据目录 0700 uid65532。
2. 启动新 PG（内部网络、无端口映射），用非 root test 容器运行 run-tests。四套 frozen producer 各需空库并生成独立只读 manifest，验证其冻结源码 hash；接着原样运行 `cargo test --offline --locked --workspace -- --test-threads=1`，不是专项替代。最终硬核对四库全部 15 项 migration checksum；保存 producer stdout/stderr/exit、manifest SHA、workspace 输出/exit 和 post-upgrade checksum。旧字节/receipt 检查由四个真实 upgrade tests 执行，host 驱动要求逐个 test ... ok。
3. 只有整套 workspace exit0 才逐例解析四组负例的真实 pass 行；missing/ignored/FAILED 或“只编译成功”均不能通过。`failure-groups.json` 关联 test、结果、workspace exit 与原始日志。任何失败立即停止后续业务门。
4. 管理例程 seed：通过正式写 API 构造有真实 PNG/PDF、两次使用同一 PNG、relation/review/epistemic 前序链与**当前**未放置双 note 的 Attention；断言 group ID、origin anchor、placement IDs 和顺序不变。根遍历覆盖全部 29 白名单表。只预置 actor/space/grant 使用管理 SQL，资源元数据沿现有夹具固定字段插入；业务导出经 enqueue/dispatch。
5. 第一只受限 Worker 容器停在 debug claim gate，捕获私有 0600 旧 token，复核 attempt1/有效租约/零结果。执行真正 Docker SIGKILL，要求 exit137 且非 OOM；按 PostgreSQL clock_timestamp 等待过期，不改 DB 时钟/租约列。第二只独立容器 attempt2 且 token 改变；renew/checkpoint/succeed/fail/旧 lease process 均不得成功或产出结果。放行第二只，要求 exit0、succeeded、attempt2、恰一条 result。第三次运行不得改状态/结果。
6. 对 Worker inspect 硬核对 uid65532、只读 rootfs、cap_drop ALL、no-new-privileges、2CPU/2GiB/128pids、唯一内部网络、无端口、只有 runtime secret、只读 assets+单个binary、**命名持久**snapshot卷及私有 tmpfs；检查没有 admin/postgres、源码、/app、证据、socket。快照卷在容器内 stat 必须 700:65532。管理容器不是 Worker 的替身。
7. 在所有 Worker 已退出后新管理进程 deliver_export，验证仍能从持久卷交付；sealed 文件经目标 stage_incoming/preflight/import。新目标只预置同身份空间授权。比较完成 manifest SHA、全部 included immutable 行和逐表行数、source 仍相同、投影与两个当前 unplaced placement、真实原件字节/hash、恰一条 bound receipt，排除 release/lineage/普通receipt/jobs。随机错误 actor 的交付与 preflight 必须通用 NotFound 且目标仍零业务行。
8. 撤 source grant，再次交付必须通用 NotFound。只 stop 本批项目，保留已退出容器和私有卷；记录 source-after 相同、每条命令/标准输出/标准错误/exit、完整 evidence SHA。失败也保留原始结果。不得将先前 RED 日志覆盖成 GREEN。

当前脚本结果 `CANDIDATE_GATES_PASSED_NOT_PRODUCTION` 仍需独立证据审计和全分支复审；脚本不能自行给项目授予最终验收状态。
