# Task9 C 安装前核验与最小审批

2026-11-03。D 为独立备份目标，S 为开发及源端主机。B 三把密钥均已有各自封存证据，原批次 HOLD 仍单列。C 的工作停在只读核对与配置预览。

## 现场结果

| 对象 | 实测结果 | 后续要核对的条件 |
| --- | --- | --- |
| 专用账号 | D 的默认 NSS 和本地 files 均查不到 `kw-b1-transport` 用户、同名组及候选 UID/GID `2401/2401`。两处身份源均已检查。 | 2401 仍是候选。写入前重查四项冲突，创建后回读真实身份、主组、补充组、口令锁定、home 与登录入口。 |
| 安装路径 | D 的两个待建父目录 `/etc/knowweave-c4-b1` 和 `/usr/local/libexec` 缺失，三个 ELF、四个配置文件、待启用 sudoers、PID 文件和拟用空 home 均未占用。`/etc` 与 `/usr/local` 身份符合已有 pin。D 的产品 registry、policy、verifier 及 session 根仍待按批准范围建立。 | S 已有 `/var/lib/knowweave-c4`，而固定 B1 目录、registry 及 fence 根缺失。现有根需单列审查，任何新旧对象冲突都停止。 |
| 22229 | D 找到一处匹配候选本机 IPv4，22229 本机监听数为 0，UFW 报告 inactive。原 22 路径的来源地址和 D Ed25519 公钥仍只作候选输入。 | 云安全组、其他主机防火墙规则、外部端口映射与 22229 实际来源、握手均待核对。计划只允许获审 S 出口 IPv4 /32 到 D 的 TCP 22229。 |
| 候选入口配置 | 以实测地址和已封存 transport 公钥渲染预览。D 本机 OpenSSH 对内存中的配置用 `-G` 检查，26 项字段匹配。`visudo -c` 核现行 sudoers 及带占位 launcher 摘要的候选规则，均返回 0。安装工具的 3 项本地回归通过，Linux chroot 项本轮跳过。 | 最终配置必须绑定实际 UID/GID 和真实 launcher/receiver ELF。实装后另验 `sshd -t`、连接上下文与生效 sudo 授权。 |
| 管理入口 | D 本机 `sshd -G` 回读当前 22 基础配置。没有发现显式排除该账号的规则或 `Match` 块。管理入口开启口令认证，授权公钥只查相对 home 路径，未配置全局 CA 或外部授权公钥命令。 | 用锁定口令、root 所有的空 home 隔离这一个账号。入口启用前验该账号无法通过管理 22 认证。若证据不足或出现旁路，停止并把准确的管理配置差额另行送审。 |

OpenSSH 9.6 的首次预览因 `-G` 与 `-C` 组合被本机拒绝，记录继续保留。后续仅用 `-G` 通过配置解析，当前结果不含实际连接上下文、PAM 会话、主机私钥加载或真实用户授权。D 的 SSH、sudo 和 PAM 规则来源摘要与此前限定观测一致。 [Ubuntu sshd 手册](https://manpages.ubuntu.com/manpages/noble/man8/sshd.8.html) · [OpenSSH 9.6 实现](https://github.com/openssh/openssh-portable/blob/V_9_6_P1/sshd.c) · [Ubuntu visudo 手册](https://manpages.ubuntu.com/manpages/noble/en/man8/visudo.8.html)

## 最小审批顺序

| 审批对象 | 精确对象与最小权限 | 实施边界和核验 |
| --- | --- | --- |
| **C-identity，下一项决策** | 只在 D 新建一组本地用户及主组 `kw-b1-transport`，候选 UID/GID 各 `2401`。初始 shell 为现有 `/usr/sbin/nologin`，口令锁定且无补充组。单独新建空 home `/var/lib/knowweave-c4-b1-transport-home`，`root:root 0755`。写入限本用户及主组的本机账号记录、系统工具必要的锁与备份、该空 home。 | 沿用一次现有 root 管理权限。执行前冻结命令和路径身份，确认无额外组、subuid/subgid 或初始化文件分配。执行后用 NSS、身份与口令状态元数据回读。信任文件、正式 shell、sudo 授权、网络规则与服务启动均属于之后的审批。 |
| C-prepare 资源与实际构建 | 为 S/D 独立 registry、S fence、D session、S 隔离 PG、真实 control 根等提交各自精确路径、UUID、所有权及资源清单。以实际根身份、PG 标识和 B 公钥 pin 构建角色 ELF。 | S 既有 C4 私有根有历史用途，新增子路径须单独核对。固定 Go 工具链和实际输入必须冻结，当前预览没有最终 launcher 或 receiver ELF。 |
| C-install 文件与信任 | D 新 `/etc/knowweave-c4-b1` 与 `/usr/local/libexec` 为 `root:root 0755`。前者仅安装 `sshd_config` 0600、`authorized_keys` 0644、`sudoers.pending` 0440、`receiver-launch.lock` 0600。后者仅安装 gate 0755、launcher 0500、receiver 0500。D 单一授权公钥限制来源 /32，S 新 `known-hosts` 仅绑定获审 D 公钥及 22229，产品固定私钥从各自 B 暂存根在本机安装。 | 审批须附实际 ELF、配置、policy、anchor、manifest 的 SHA-256、owner/mode 和安装落点。两侧实际 trust、registry/fence/session 等材料仍需各自冻结。文件安装采用独占创建和回读，启用权限另审。 |
| C-enable 权限、网络与监听 | D 将此账号的 shell 从 `nologin` 改为受审的固定 gate，并启用单条 `/etc/sudoers.d/knowweave-c4-b1`。该规则只允许经 SHA-256 固定的 launcher，以 `(root:root)`、空参数运行。仅对该命令关闭 PTY 和协议原始 I/O 记录，保留事件审计。单独批准 S /32 的 TCP 22229 放行和 D 隔离 `sshd` 的精确启动及停止。 | 审前核账号额外匹配规则、管理 22 排除、真实网络与云侧范围。现有管理 listener 和业务服务保持在本批之外。遇到身份、地址、权限、ELF 或通道不符，按准确新增对象留证并停止。 |

## 启动链与验收边界

预期链路为 **S 固定 SSH 客户端 → D 的 22229 独立 sshd → 低权专用账号的固定 gate → 单条 digest sudo → root native launcher → Rust receiver**。Gate 比较实际 UID/GID、固定强制命令与原始命令，只传固定 argv。Launcher 核 root 身份、空参数、真实文件与独占锁，核三个方向正确且 inode 互异的 FIFO，清空子进程环境，保留自身为 receiver 的直接父进程并传递 EOF 和退出码。该链已按现有源码核对，实际 SSH、PAM、sudo、父进程 ELF、FD、错误命令和转发拒绝仍须在获准启用后逐项取证。首次签名和恢复由后续单独批次承担。 [安装工具和 native 入口](../../scripts/task9_h4/install.py) · [native 启动链](../../scripts/task9_h4/native_linux.go) · [双机实施单](installation-and-two-host-acceptance-plan.md)

原始结果 SHA-256。D 首次观察 `49fbfa7de5b2ced72a2460259e1a472a8904d362c9f10969efbd97659e746e63`，D 解析复核 `00d72d67f0850adf8c5920d599267bdddf2bd62fd868acc377bfb9d51880c9e4`，D 管理入口 `1cb7eccd4499e80278613557e931d8d1f1947cf00c67e9989545a5661aeb4f04`，S 路径 `3c79a1563c592ed06138929b61e969902529ac3a342bcbf75907c5a2707ce882`。地址、实际 pin 与父目录身份均留在私有原件。
