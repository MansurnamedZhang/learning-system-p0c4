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

本协议只支持 **rootful Docker、没有 userns remap**，宿主 driver 必须受控 sudo/root 执行（启动前检查 euid=0 和 Docker SecurityOptions）。普通 Docker-group 用户不能读取分属两个 UID 的 0600 文件；rootless/userns 环境必须另行设计，不能放宽成 0644。Docker daemon 必须运行在本机，使 bind source 与本地校验指向同一文件。

准备两套共六个 file-backed secret：`.runtime/secrets/{postgres_password,admin_password,runtime_password}` 属主 `65532:65532`，供非 root 客户端；`.runtime/secrets/pg/` 下三个同名文件属主为固定 PG 镜像内实际 postgres UID/GID。**同名副本内容相同，各角色之间不同**；六文件均 0600，三层私有目录 root-owned 0700，无 symlink/hardlink。PG UID 不硬编码。Worker 只挂客户端 runtime 文件。PG overlay 以相同 target 覆盖 base secret source；driver 核对 merged config 三项和实际 PG bind mounts。

在本批全新源码目录中，可用以下一次性准备命令；已有 `.runtime` 会拒绝，绝不覆盖。镜像必须已按本批授权准备好。命令没有密码 argv/stdout，也不输出 DSN：

```sh
sudo python3 - <<'PY'
import json, os, pathlib, re, secrets, subprocess
assert os.geteuid() == 0
root = pathlib.Path.cwd()
options = json.loads(subprocess.check_output(['docker','info','--format','{{json .SecurityOptions}}']))
assert not any('rootless' in item or 'userns' in item for item in options)
compose = (root/'deploy/compose.test.yaml').read_text()
image = re.search(r'(?m)^    image: (postgres:[^\s]+@sha256:[0-9a-f]{64})$', compose)[1]
subprocess.run(['docker','image','inspect',image],check=True,stdout=subprocess.DEVNULL)
identity = subprocess.check_output(['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges:true','--entrypoint','sh',image,'-eu','-c','id -u postgres; id -g postgres']).splitlines()
assert len(identity) == 2 and all(item.isdigit() for item in identity)
owner = tuple(map(int,identity))
for path in [root/'.runtime',root/'.runtime/secrets',root/'.runtime/secrets/pg']:
    path.mkdir(mode=0o700,exist_ok=False)
for name in ['postgres_password','admin_password','runtime_password']:
    value = secrets.token_hex(32).encode('ascii')
    for prefix, ids in [('',(65532,65532)),('pg',owner)]:
        path = root/'.runtime/secrets'/prefix/name
        fd = os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
        with os.fdopen(fd,'wb') as output:
            os.fchown(output.fileno(),*ids)
            output.write(value)
print('Six private secret files prepared; no values printed.')
PY
```

driver 在任何命令证据落盘前，把六文件读入内存并核对格式、权限与同名副本字节；固定 PG 镜像的无网络只读身份 probe 后核对 PG 属主，再以各实际 UID 在无网络只读容器中检查 mode/uid/gid/可读长度。以上 probe 不运行业务；业务容器启动前全部必须通过。禁止把密码放入 argv/Compose environment，禁止 shell `set -x`。

`deploy/c3-databases.json` 是固定清单，`initdb.sh` 创建 11 个独立库：主测试、p0a/b1/b2/b3-schema 四旧库、C3 0013升级、import A/B、0014导入升级、C3 process source/target。run-tests 的第一步通过所有 admin DSN 检查实际数据库名互异且 public schema 无业务对象，再运行任何 producer/migration。它绝不 drop/reset 旧库。

构建**这份已冻结源码**的 test image，固定其不可变 image ID；构建和接受环境需由该批授权准备，不在脚本里自动联网安装。Dockerfile.test 同时构建 debug Worker 和管理例程，将 /app 与 Cargo cache 交给非 root 测试用户。把镜像中 `/app/target/debug/learning-worker` 导出为单个普通 0755 文件；只读挂它进 Worker，不能挂整个 target/源码。驱动首先从 Dockerfile 的全部 COPY 输入生成逐文件 SHA 清单（包含 Dockerfile/.dockerignore 自身），用无网络只读镜像 probe 对照镜像内字节；缺失、旧字节或清单差异均阻断业务容器。随后仍复核宿主二进制 SHA 与管理镜像构建输出相等。此证据绑定已复制源码与构建输出，不是外部可重现构建证明。

```sh
export C3_PROJECT=learning-system-p0c3-task7-<本批唯一随机后缀>
export C3_IMAGE=sha256:<本批test镜像64位ID>
export C3_RUNTIME_IMAGE=rust:1.97.0-bookworm@sha256:b5a086f64ffecaa4e283063184770107915756739598173e1f5712d6b34b84d0
export C3_WORKER_BIN=/本批私有目录/learning-worker
sudo --preserve-env=C3_PROJECT,C3_IMAGE,C3_RUNTIME_IMAGE,C3_WORKER_BIN python3 deploy/c3_acceptance.py --evidence /全新且在源码目录外的证据目录
```

宿主运行者需要受控 sudo/root 与本机 Docker 权限，**任何容器不挂 Docker socket**。驱动调用的每条 Compose 命令均显式 `-p`，要求带随机后缀的新 task7 项目名，并检查已有容器/网络/卷标签及固定卷名；发现旧对象立即拒绝。它不负责上传、生产部署或修改主机服务。

## 明确阶段与硬断言

1. 记录 source-before、0001–0013 校验和、单个 Worker hash、脱敏后的 Compose config、镜像源码清单、PG 身份和六文件可读 probe。oneshot init 只接新空卷且无网络/密钥，设置快照/控制/目标/资产/证据目录 0700 uid65532。
2. 启动新 PG（内部网络、无端口映射），用非 root test 容器运行 run-tests。四套 frozen producer 各需空库并生成独立只读 manifest，验证其冻结源码 hash；接着原样运行 `cargo test --offline --locked --workspace -- --test-threads=1`，不是专项替代。最终硬核对四库全部 15 项 migration checksum；保存 producer stdout/stderr/exit、manifest SHA、workspace 输出/exit 和 post-upgrade checksum。旧字节/receipt 检查由四个真实 upgrade tests 执行，host 驱动要求逐个 test ... ok。
3. 只有整套 workspace exit0 才逐例解析四组负例的真实 pass 行；missing/ignored/FAILED 或“只编译成功”均不能通过。`failure-groups.json` 关联 test、结果、workspace exit 与原始日志。任何失败立即停止后续业务门。
4. 管理例程 seed：通过正式写 API 构造有真实 PNG/PDF、两次使用同一 PNG、relation/review/epistemic 前序链与**当前**未放置双 note 的 Attention；断言 group ID、origin anchor、placement IDs 和顺序不变。根遍历覆盖全部 29 白名单表。只预置 actor/space/grant 使用管理 SQL，资源元数据沿现有夹具固定字段插入；业务导出经 enqueue/dispatch。
5. 第一只受限 Worker 容器停在 debug claim gate，捕获私有 0600 旧 token，复核 attempt1/有效租约/零结果。执行真正 Docker SIGKILL，要求 exit137 且非 OOM；按 PostgreSQL clock_timestamp 等待过期，不改 DB 时钟/租约列。第二只独立容器 attempt2 且 token 改变；renew/checkpoint/succeed/fail/旧 lease process 均不得成功或产出结果。放行第二只，要求 exit0、succeeded、attempt2、恰一条 result。第三次运行不得改状态/结果。
6. 对 Worker inspect 硬核对 uid65532、只读 rootfs、cap_drop ALL、no-new-privileges、2CPU/2GiB/128pids、唯一内部网络、无端口、只有 runtime secret、只读 assets+单个binary、**命名持久**snapshot卷及私有 tmpfs；检查没有 admin/postgres、源码、/app、证据、socket。快照卷在容器内 stat 必须 700:65532。管理容器不是 Worker 的替身。
7. 在所有 Worker 已退出后新管理进程 deliver_export，验证仍能从持久卷交付；sealed 文件经目标 stage_incoming/preflight/import。新目标只预置同身份空间授权。比较完成 manifest SHA、全部 included immutable 行和逐表行数、source 仍相同、投影与两个当前 unplaced placement、真实原件字节/hash、恰一条 bound receipt，排除 release/lineage/普通receipt/jobs。随机错误 actor 的交付与 preflight 必须通用 NotFound 且目标仍零业务行。
8. 撤 source grant，再次交付必须通用 NotFound。只 stop 本批项目，保留已退出容器和私有卷；记录 source-after 相同、每条命令/标准输出/标准错误/exit、完整 evidence SHA。启动命令尚未成功时也已武装清理；按标签发现本批对象并逐个核验后 best-effort stop，某项 inspect/logs 失败不会跳过其余对象或 PG。result.json 分别保留原始失败和 cleanup_errors；清理失败不能覆盖原始业务错误。不得将先前 RED 日志覆盖成 GREEN。

当前脚本结果 `CANDIDATE_GATES_PASSED_NOT_PRODUCTION` 仍需独立证据审计和全分支复审；脚本不能自行给项目授予最终验收状态。

## 证据与密钥

所有 command stdout/stderr（含 timeout）写入宿主证据前替换六份密码值，因此嵌入 DSN/config 的原文也会脱敏。`docker cp` 使用 stdout tar，仅在内存读取，拒绝链接/越界条目；逐文件脱敏后才写证据目录，没有暂存明文再改写的窗口。秘密值限定为无换行 ASCII 十六进制，因此不会有 URL/YAML 转义差异。不得自行执行并保存未脱敏的 `docker compose config`。官方 [Compose trust model](https://docs.docker.com/compose/trust-model/) 说明配置解析可能涉及文件内容；本修复属于预防措施，并没有观察到本批已泄漏。

容器生成的旧 evidence.sha256 是其原始字节摘要；宿主脱敏可能改变日志字节，以宿主最终 `evidence.sha256.json` 为交付摘要，两者不伪称相同。密码永不写入摘要清单或错误消息。secret target 的合并规则见 [Compose merge](https://docs.docker.com/reference/compose-file/merge/)；实际合并与容器可读性仍须 Linux 首门验证。
