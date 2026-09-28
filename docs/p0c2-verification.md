# P0-C2 持久后台任务验证记录

**结论：P0_C2_VERIFIED / NOT_PRODUCTION。** 已在全新隔离 Linux 项目中验证真实业务事件、持久任务、租约接管、专用 Worker 凭据边界、取消与撤权；格式、严格 Clippy、四套冻结旧程序升级和全工作区回归通过。此结论仅覆盖 C2，不代表 P0-C 整体完成或生产可用。

## 冻结身份

| 项目 | 已核对值 |
| --- | --- |
| 分支与已验证源码提交 | feat/p0c2-jobs；d286b2e57267996f8e1b6e51ca2ff2c5af580a16 |
| 源码包 | task6-worker-acceptance-v3-candidate.zip，SHA-256 f5f9290b1f39423b95f8eaf4f4094eb3ab43a31c545e21707713bd9c327e1f34；296 个源码文件 |
| 包内清单 | SOURCE_MANIFEST.json，SHA-256 b0fb97892c65d31441b4e319aa702ffc78ab2b24df9d7fc2d141d742e2be43fc |
| 最终隔离项目 | learning-system-p0c2-task6-v3-acceptance-ws6；启动前不存在同名项目、容器、网络或卷 |
| 原始证据 | <local-user-home>/Documents/ubuntu_Seoul/tmp/t6f/p0c2-task6-worker-acceptance-v3-candidate-ws6/result.json；SHA-256 ccc49177f7d249a6d63136a7e4227dc1af6938511add802b2d1413ce04e2ab44 |
| 工具与镜像 | Rust 1.97.0；Rust 测试镜像 sha256:8b4ffdfaa1ec6bb0a12527f00936513238453b83affc7d65faa4a2db04a9b8a1；PostgreSQL 18.6 镜像 sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d |
| Worker 二进制 | 从冻结源码离线编译后导出的单文件，0755，SHA-256 f276cf08d6481b8c27563e7029726b2f6f2ea8d43c6391d362886533171cea6f |

该包上传前后 SHA-256 一致。包内 296 项与本地磁盘的字节、大小、SHA-256 逐项一致；Linux 运行前后的 296 项源码核对均为真。最终 result.json 所列 965/965 份证据文件由主代理与独立审查代理分别重算通过；188 个主阶段退出码均为 0，最终 postcheck_success=true。源码提交的三个 Task 6 新文件与冻结包中对应字节完全一致。本文以及随后批准记录属于验证后的文档收口，不属于上述冻结源码包。

相对已验收 C1 基线，仅追加迁移 0011–0013；迁移 0001–0010、旧 outbox_event 与四套冻结旧程序源码未改。六个数据库在本项目初始化后均为空：原五个测试/升级库和专用 learning_worker_acceptance_test。网络为内部 10.251.42.0/24、无主机端口；PostgreSQL 在主门及补充核验后均正常停止，最终 inspect 为 exited、ExitCode 0、OOMKilled=false。

## 真实故障链和凭据边界

管理夹具通过公开的资产上传、登记、v3 Figure 写入与 job 转投 API 创建任务；没有直接插入业务 job 或 job_outbox。权威资产卷内的 16 字节 PNG 与数据库资产 SHA-256 一致。首个 Worker 在领取后停于私有闸门，数据库显示 running、attempt 1、无结果；管理夹具将数据库当前租约 token 写入宿主 0600 私有文件，仅挂载到管理夹具容器、未挂载到 Worker，只留下文件哈希。首进程由 SIGKILL 真正终止，退出码 137，OOMKilled=false；在 PostgreSQL 时钟确认租约到期后，第二个独立 Worker 领取 attempt 2。

第二个 Worker 仍在闸门内时，管理夹具用同一文件中的旧 token 调用 renew、checkpoint、succeed、fail，四项均返回 false；前后文件 SHA-256 一致，第二租约仍有效。放行后任务为 succeeded、attempt 2、仅一条 asset_integrity_result；重复运行不改变 attempt、摘要或结果数。结果行的 job、outbox、空间、块、修订、资产、SHA-256、16 字节大小与业务输入和卷内原字节逐项匹配。另两个独立任务分别在 Worker 领取后取消、撤回发起者授权，终态为 cancelled、failed，均无派生结果。关键原件见 first-kill.json、capture-token-parsed.json、fence-token-parsed.json、success-payload-parsed.json、replay-snapshot-parsed.json、cancel-final-parsed.json 和 revoke-final-parsed.json。

五只实际 Worker 容器的 inspect 均显示：仅挂 runtime_password、只读 /assets 权威卷与只读单文件二进制；没有 admin/postgres secret、源码、target、证据卷或 Docker socket；创建配置的环境和命令未含明文密码或展开后的 DSN。容器以 65532:65532 运行，根文件系统只读，丢弃全部 capability，私有 /staging、/gate、/tmp tmpfs，限制为 2 CPU、2 GiB、128 进程，无主机端口。容器内边界检查确认高权 secret 不存在或不可读、runtime secret 可读、资产卷只读。运行中的 Worker 仍需在自身进程环境中持有 runtime DSN，因此宿主 root 或 Docker daemon 仍可读取；这里验证的是服务文件与容器创建边界，不是对宿主特权主体的保密。

## 质量门

| 门 | 原始结果 |
| --- | --- |
| 管理夹具真实 PostgreSQL 测试 | 2 通过、0 失败，管理容器与 Worker 凭据分离 |
| 格式与严格静态检查 | cargo fmt --all -- --check、cargo clippy --offline --locked --workspace --all-targets -- -D warnings，均退出 0 |
| 四套冻结旧程序升级 | P0-A、B1、B3-schema、B2，各自从空库开始，bootstrap 退出码均为 0；见 workspace-fixtures |
| 全工作区 | cargo test --offline --locked --workspace -- --test-threads=1：388 通过、0 失败、3 个既有忽略；78 个测试结果段从原始 workspace/stdout.log 汇总 |
| 补充只读核验 | 仅持 runtime secret 的独立管理容器以参数化 UUID 执行逐列 SQL，结果与主门相同；网络、所有本项目卷与 PostgreSQL 停止状态另存 inspect；postcheck2/runtime-query-check.json 为通过 |

## 失败历史与范围

ws1–ws5 均为验收运行器的前置或清理错误，不能作为通过证据：PostgreSQL inspect 假设、Compose profile、secret target 规范化、宿主无权读取管理容器 0600 文件、参数化 SQL 缺少标准输入及清理器误杀本批 PG。每次使用新隔离项目并在服务器各自独立批次目录保留 result.json/失败日志；最终本地归档仅包含 ws6 和其补充审计，冻结源码包未为这些运行器问题改动。ws6 主门通过后，首次补充审计误把 PostgreSQL 客户端镜像自动产生的匿名卷判为额外挂载；postcheck/failure.txt 保留，修正为只核允许的挂载后 postcheck2 通过，原主门 result.json 和证据保持可追溯。

独立全分支静态审查未发现新的 P0/P1。仍有两个可信 runtime 边界：共享 runtime 凭据可绕过 Rust 字节复核直接调用数据库完成函数；持有当前 token 的内部代码还可调用通用 succeed 函数，把资产任务标为成功而不写 asset_integrity_result。现有 Worker 正常路径只调用原子结果函数，隔离故障链验证了该路径；未来应收紧通用完成权限或按任务类型加数据库约束。首个处理器只覆盖 v3 Figure/Attachment 的精确块资产使用，不自动处理仅登记或仅关联资源版本的资产，也不做完整媒体解析、恶意文件扫描或预览生成。

服务器时钟另有生产前门槛：只读补证 <local-user-home>/Documents/ubuntu_Seoul/tmp/t6f/task6-clock-skew-readonly.json（SHA-256 24a70b7a5c948f29f5fac7bacf00fda4204b38a6c27581f8b011dbc2bc6c6a60）显示服务器 UTC 比本机快约 8 小时 11 分 32.626 秒，NTP=yes 但 NTPSynchronized=no。此次租约判断全由同一隔离 PostgreSQL 的 clock_timestamp() 完成，功能测试结论不因此撤销；生产部署前必须修复时间同步并另验绝对时间依赖。

C3 融合快照交换、C4 完整备份恢复、P1 HTTP/前端、生产部署仍未实施或验收；P0-C 整体尚未完成。此分支保留在隔离工作树，未推送、合并或部署生产。
