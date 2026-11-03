# Task9 H4 固定启动链与安装清单审查稿

2026-11-03。源码核对基线 [`2b18bba7`](https://github.com/MansurnamedZhang/learning-system-p0c4/commit/2b18bba78e2ebd03a80595b9f3234623f576116d)。本稿定义下一轮审查和审批对象。启动链、安装工具、配置与现场参数均待实现或实例化。C0 的 25 阶段普通回归及 20 项新增用例沿用原证据。

## 启动时实际核对的对象

| 对象 | 当前源码约束 | 安装清单必须证明 |
| --- | --- | --- |
| S 发起的 SSH | `/usr/bin/ssh`，空配置、固定 host-key 文件与独立 transport key，单次尝试，唯一远程命令 `/usr/local/libexec/knowweave-c4-b1-receiver` | S 可达的实际 D endpoint，既有可信公钥的取得渠道与指纹，受限账户绑定，实际 D 生效规则。 [S 固定 SSH 调用](../../crates/learning-backup/src/offhost_b1/supervisor.rs) |
| D receiver | 固定路径，argv 只有该路径，root 所有、单硬链接、模式恰 0500。receiver 和直接父进程均须四组 UID/GID 为 0、无 tracer。内核当前父 ELF、PID、启动时间、具名原件与 manifest 摘要须对应，原操作在消费前持续再核 | 固定 native parent 实际生成并等待唯一 receiver，父进程身份在相应阶段可观察。`exec` 覆盖 launcher、经 shell 直接启动及普通用户直接执行均无法提供候选父进程证据。 [D 入口与 native custody](../../crates/learning-backup/src/offhost_b1/destination_actor.rs) |
| D 双向字节通道 | receiver 接管实际 FD 0/1，要求不同 inode 的只读、只写 FIFO，再固定非阻塞、CLOEXEC 和 SIGPIPE 前提 | 本机 `sshd` 和权限桥的实装通道类型、方向和 owner，父进程关掉重复写端，实际 EOF、原 receiver 自然退出与真实退出码传播。OpenSSH 上游无 PTY 分支同时包含 pipe 和 socketpair 路径，实装必须取证。 [原管道约束](../../crates/learning-backup/src/offhost_b1/wire/io.rs) · [OpenSSH 上游实现](https://github.com/openssh/openssh-portable/blob/master/session.c) |
| D signer | 入口先取得原 registry lease、native custody、签名 key/trust 及 session namespace，后进入 B1 认证；签发依赖原会话后续一次消费 | 入口隔离和并发限制需在启动前成立。成功 SSH 只证明 transport 身份；应用层仍须验证独立的 SourceAuth、最终决定及全套发布条件。 [D signer 固定入口](../../crates/learning-backup/src/destination.rs) · [D 会话顺序](../../crates/learning-backup/src/offhost_b1/destination_actor.rs) |

## 单一待审接入方案

D 使用隔离测试 listener 与新专用低权 transport 账户。listener 只绑定批准的接口与端口，仅接纳指定公开 transport key；账户无业务组、Docker 组或普通交互入口。OpenSSH 强制命令通过账户的登录入口执行，生效配置须禁止 TTY、agent、X11、TCP/Unix socket 转发、用户 rc、用户环境、口令和子系统旁路。采用固定原始命令检查；无命令、附带参数、控制字符、子系统或任意命令均须在启动 root launcher 之前拒绝。源端匹配范围按 D 实测的 S 出口地址填写，不能把 S 的内网地址视为云端可见地址。 [OpenSSH 强制命令及转发规则](https://man.openbsd.org/sshd_config.5#ForceCommand) · [authorized_keys 规则](https://man.openbsd.org/sshd.8#AUTHORIZED_KEYS_FILE_FORMAT)

专用登录入口只比较固定字节串，既不解析 shell 表达式，也不解释输入帧。比较通过才请求唯一的权限桥动作。候选桥为专用账户调用一条经 digest 固定、目标 UID/GID 均为 0、明确限制空参数的非交互 sudo 规则，禁止继承调用者环境或 FD。只为该命令核定无 PTY、无原始字节 I/O 记录的设置，保留脱敏事件审计。最终稿须以目标实际版本的 `sshd -t`、`sshd -T -C`、`visudo` 与生效授权核验为准，排除其他匹配规则追加授权。此处的 sudo 只定义拟审批的单一动作，未启用任何规则。 [Ubuntu sudoers 手册](https://manpages.ubuntu.com/manpages/noble/man5/sudoers.5.html)

root native launcher 固定一条内部子进程路径和单元素 argv，清除环境，先核受控账户/进程边界、stdio 类型和 FD 闭集，再派生 receiver。禁止 setuid 自制入口。launcher 在子进程整个等待期保持自身 ELF 和父身份，不复制或转录协议。父进程关闭继承的会话管道端，记录子进程的实际退出结果。错误、信号、EOF 不明或子进程状态无法确认时留下 HOLD，不能返回成功或替换成另一次调用。若目标通道出现 socketpair、PTY、重复写端或身份不符，候选停止；更换桥接方式须重新审查，产品管道判据保持。

## 分批待装对象

下表的产品路径来自代码。新工具候选名用于划定审查范围，最终文件字节、落点与环境清单须在审批前封存。一般根为 root:root 0700，固定 metadata/key 为 root:root 普通单链接 0600，可信祖先逐组件 no-follow、无 group/other 写权限。对 registry、PG、ELF、Catalog 和 Capture release 使用各自更精确的代码合同。

| 阶段 | 主机与对象 | 审批前须确定的最小内容 |
| --- | --- | --- |
| A 只读核实 | S/D 指定新根及其祖先、现有安装占用、D 磁盘/挂载与管理归属、固定可执行文件、候选监听口 | 主机、既有观察身份、明确路径与时点。root 私有检查仅列具体元数据；对私钥仅查存在性/权限，托管者另输出公钥指纹。权限拒绝保持 UNKNOWN，不切换身份绕过。 |
| B 托管 | S `/var/lib/knowweave-c4/b1/{source-auth.pk8,transport-key,known-hosts}`，D `/var/lib/knowweave-c4/verifier/destination.pk8`，双方用于限定构建和信任核验的 public pins | 三种用途分账，S 应用 Ed25519、公钥 transport Ed25519、D P-256；D 用户负责 root 私有托管。单独列已有 host key 的公开指纹来源。完成 B 不安装 authorized_keys、开放监听或启动 launcher。 |
| C-prepare 实例化 | S 隔离 PG/Unix socket 与 source-control binding、S/D 独立 registry 根及锁、S `/var/lib/knowweave-c4-source-fences`、D `/var/lib/knowweave-c4-b1-sessions`；预留 S1/S2、V1/V2、RA/RB | 精确 UUID、路径、卷/网络/进程资源、角色、容量、权限和已有对象冲突表。绑定需真实 PG `database_oid` 与 `system_identifier` 以及 control 根 `dev/ino`，先取得事实再编译，不用占位身份构建运行 ELF。两 namespace 的固定清单为 `anchor.json`、`lock`、`operations`、`staging`；anchor 随真实根身份生成，禁止空 anchor 充当准入。 [源端绑定解析](../../crates/learning-backup/src/source/binding.rs) |
| C-install 产品材料 | S 三角色 ELF、各自 manifest/Catalog、S/D 固定 policy、fence/session anchor、D trust、D receiver、Capture release 内专属 policy 和 own manifest 的两个 RO 文件别名及 fence 根 RO | 每项列真实原字节/大小/SHA-256、来源、owner/mode、父目录、安装和回读方法。C0 保留原 12 项，实核精确 15 项、同 inode 的别名、RO flock、来源可写面排除及完整 Root/PG/C11/S5/FD 生命周期。 [C0 现场合同](capture-mount-amendment-contract.md) |
| C-install 入口材料 | D 新隔离 listener 配置、专用账户及 authorized_keys，候选 `knowweave-c4-b1-ssh-gate` 与 `knowweave-c4-b1-receiver-launcher`，单条 sudo 规则 | 审批冻结实际绝对路径、文件 owner/mode/SHA、监听对象、允许来源、有效规则、原始命令字节、FD/环境闭集、启动与停止方式。新增对象与 D1 原 reader、旧服务分离，配置变化只限本批。不改变全局 SSH/管理账户或原服务访问。 |
| D1/D2 现场验证 | C 准入后执行实际双机首签与 RA；随后按批准资源隔离实验源访问，在全新 RB 仅用 D 工件恢复 | 另列独立批次资源、适用负例、留卷及原件；S2/V2 预准入与 D1 读语境共存，各固定路径要由真实隔离视图分别提供。D2 仅阻断本批源依赖。 |

固定材料依赖顺序为真实根与 public pins → 固定 builder 角色构建和 native launcher ELF → anchor → D policy → S 两份按实际角色区分的 policy → 各自 launch manifest。manifest 引用 policy/anchor/ELF，policy 不反向引用 manifest 摘要。D policy 的实际字节由安装回读、D 固定启动链与管理写面排除连接到 S 的 `destination_policy_sha256`；不能用远端返回的 receiver ELF 摘要充当 D policy 摘要。 [policy 字段与关联校验](../../crates/learning-backup/src/offhost_b1/policy.rs) · [固定 metadata loader](../../crates/learning-backup/src/offhost_b1/policy/installed.rs)

## 安装工具与验收门

1. **冻结输入。** 一份封存清单分配每个新增对象的路径、owner、mode、来源 digest 与命令，关联实际父目录身份、批准的角色资源、容量和权限。明文密钥、SSH 登录凭证与原协议流不进清单或日志。安装工具的实现、构建和审查单列；原单机 fixture issuer 保持原用途。
2. **准备、写入、回读分开。** 安装前先核全表和冲突，对每步使用持有目录 FD、no-follow 和独占创建。发行固定字节时同步文件与父目录，回读核同一对象和摘要。前置检查、文件安装与入口启用分别留证；入口只有在两侧实际材料和规则核对后才能激活。
3. **先验证拒绝面和字节通道。** 用独立受控测试身份覆盖错 key、空命令/额外参数、shell/SFTP、转发、TTY、环境、非空 sudo 参数、非批准目标和错误父进程。对允许通道取 `/proc` 进程/映射/FD 元数据、pipe flags、关闭与退出原件；无签名通道探针仅授对应传输信用。直接验证 `sshd` → 权限桥 → native parent → receiver 时使用获准的精确实装配置。只读状态或测试夹具不得替代该检查。
4. **发生中断即封存。** 区分未触碰、部分安装、已核未启用、启用结果未知。停止本批新增入口前核具体 ID 和影响。保留文件/同步及权限回读证据，权限撤销按具体差额另审；禁止覆盖未知旧对象、自动重新启用、释放 fence 或补发成功凭据。

**当前结论。** H4 已有可审查的候选链路与文件目录，仍缺 H1 的目标现况和端点事实、实际 installer/native 工具及准确配置、独立审查和 B/C 批准。当前材料没有现场证据或新的运行权限。H2/H5 的真实生命周期、D1/D2、Task9 与 C4 正式验收继续开放。
