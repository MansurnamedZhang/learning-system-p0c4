# Task9 H4 安装候选与现场清单

2026-11-03。此候选把 [H4 设计](fixed-launch-installation-review.md)细化为可检查的 D 端文件和运行合同。现场安装保持 **HOLD**。产品代码、默认关闭的构建条件和原 Task9 验收标准沿用既有版本。

后续 D 公开 pin 与本机地址候选已经复核。账号、端口、配置解析和最小权限的最新结果见[C 安装前核验](h4-c-installation-preflight.md)。早期来源探针见[现场前置核验](h4-field-preflight.md)。

## D 的只读观测

北京时间 17:42，沿用已有 SSH 主机信任，以 UID 1000 登录 D。观察到 Ubuntu 24.04、x86_64、2 个逻辑 CPU、约 1.92 GiB 内存，`/var/lib` 的 ext4 文件系统当时约有 36.7 GiB 可用。`openssh-server` 为 `1:9.6p1-3ubuntu13.16`，`sudo` 为 `1.9.15p5-3ubuntu5.24.04.1`。候选端口 22229 当时无本机监听，专用账户 `kw-b1-transport` 及 B1 配置、应用和 session 根未观察到。`/usr/local/libexec` 的缺失是多个候选 ELF 路径显示缺失的共同原因。sudoers 私有目录返回权限不足，冲突状态仍未知。原始观测 SHA-256 为 `329dadd6ea3b9eac3265b720341c00aa12399e767a50ed5e71a1ff8ff925ff92`。地址、主机公钥指纹与实际身份资料留在私有记录。

这些是瞬时、普通权限观察。D 的独立运维归属来自用户指定，云盘与共享依赖仍须在正式验收单中说明。端口空闲不证明云安全组、S 的出口地址或连通性。运行前须复查资源，容量估算须计入失败留存和 RA/RB。

## 候选工具与启动配置

- [安装工具](../../scripts/task9_h4/install.py)的 `prepare` 用准确请求和三个 ELF 生成单一文件清单及私有 bundle。字段闭集包括 D 本机 IPv4、D 实际看到的 S 出口 IPv4、专用 UID/GID、单一 Ed25519 transport 公钥、已有 D host 公钥 SHA-256 和两个安装父目录的 `dev/ino`。公钥摘要以 OpenSSH 公钥的二进制 wire blob 为输入。拒绝通配绑定、CIDR 或多行注入、非规范字段和 key 用途混同。字段没有实际值时保持 HOLD。候选固定端口是 22229。
- 同一工具的 `stage-inactive` 需要批准的清单 SHA-256、root 私有 bundle 和 Linux root。重渲染配置，核原字节、固定路径、root 父目录身份和已有 host 公钥 pin；只检查 host 私钥文件元数据。目录及文件独占创建，拒绝覆盖和 symlink，逐文件及父目录同步并回读。摘要约束输入完整性，执行权限须另有实际授权。
- [Linux native 入口](../../scripts/task9_h4/native_linux.go)按明确的构建输入生成两个 ELF。`gate` 要求专用 UID/GID、固定登录参数和 `SSH_ORIGINAL_COMMAND`，仅请求 `/usr/bin/sudo -n -u root -g root -- /usr/local/libexec/knowweave-c4-b1-receiver-launcher`。`launcher` 核 root UID/GID、sudo 原账户、三条不同方向与 inode 的 FIFO、额外继承 FD、独占空锁及实际接收器字节摘要，清空环境后以单元素 argv 启动原 Rust receiver。父进程保持存活等待，关闭自身管道副本并传递实际退出码，信号返回非零。C 仍须排除并发 root writer、验证 launcher 等待期和真实 FD 总量。
- 渲染的 `sshd_config` 供独立 listener 使用，只有专用账户和精确 S 地址获准。单公钥、禁口令及交互认证、root 登录、PTY、转发、用户 rc 和用户环境。固定命令标记由 native 登录入口比较。sudo 规则使用单一 launcher ELF 摘要、`""` 空参数、`(root:root)`、`NOSETENV` 和 `fdexec=always`，仅针对该命令关闭 PTY 和原始 I/O 记录。保留事件日志。 [Ubuntu OpenSSH 配置手册](https://manpages.ubuntu.com/manpages/noble/man5/sshd_config.5.html) · [Ubuntu sudoers 手册](https://manpages.ubuntu.com/manpages/noble/man5/sudoers.5.html)

私有构建的顺序是冻结实际 receiver ELF，再生成 launcher 并记录其 ELF 摘要，最后生成 sudo 规则和产品 launch manifest。`go build -trimpath` 使用获审的 Linux/amd64、`CGO_ENABLED=0` 和固定 Go 工具链，两个角色的 `-ldflags` 分别绑定 `main.role=gate` 或 `main.role=launcher`、`main.transportUID`、`main.transportGID`。仅 launcher 另绑定 `main.receiverSHA256`，取实际 receiver 的 SHA-256。缺少角色、ID 或正确摘要时入口拒绝。测试 ELF 和 Go 预发布工具链只用于本地候选验证。

## 最小权限与实施顺序

| 批次 | 确切写入对象或动作 | 进入条件与停止点 |
| --- | --- | --- |
| A 补核 | 精确读取 D sudoers 相关生效规则、专用账户冲突、既有管理 SSH 认证面、候选 bind 地址和 S 实际出口；核 `/etc` 与 `/usr/local` 原身份 | 仅在指定身份下取所需元数据和公开 pin。任何意外的既有 include、同名账户或配置关联保持 HOLD。 |
| B 托管与 C-prepare | [原清单](installation-and-two-host-acceptance-plan.md)规定的 S 应用认证、独立 SSH transport 和 D 签名托管；预留精确 UID/GID、隔离 PG/registry/fence/session 与测试资源 | 不以管理登录凭证充当 transport。选定现有 D host Ed25519 公钥，并绑定新端口的 S `known-hosts`。外部实际批准逐项覆盖对象、owner、mode 与影响。 |
| C 文件阶段 | 新 `/etc/knowweave-c4-b1`、`/usr/local/libexec` 均为 root:root 0755。前者仅有 `sshd_config` 0600、`authorized_keys` 0644、`sudoers.pending` 0440、`receiver-launch.lock` 0600。后者仅有 `knowweave-c4-b1-ssh-gate` 0755、`knowweave-c4-b1-receiver-launcher` 0500、`knowweave-c4-b1-receiver` 0500。文件均 root:root、普通单链接 | 单独批准实际 bundle SHA-256 及两个父目录身份。专用账户文件、公钥授权、全局 SSH 和 sudoers 的有效状态须先核为无冲突。若路径已存在或安装中断，保留准确已写对象并停；禁止自动重跑和覆盖。 |
| C 启动阶段 | 单独批准专用账户与 native 登录入口、将冻结的 sudo 规则发布到 `/etc/sudoers.d/knowweave-c4-b1`、仅绑定获审地址的 22229 listener 和所需最窄网络放行。候选启动 argv 为 `/usr/sbin/sshd -D -e -f /etc/knowweave-c4-b1/sshd_config` | 先在 D 执行实际 `sshd -t`、`sshd -T -C`、`visudo` 和有效 sudo 授权核验。核专用账户没有额外组、口令或可写 SSH 授权路径，并证明现有管理 listener 不接纳该 transport 账户。真实 UID/GID、父 ELF、实际 FIFO、无注入 banner、EOF、退出码及拒绝面都须有原件。任何失败按精确 ID 停本批入口并留证。 |
| D1 / D2 | 完成 C 现场准入后另批首次独立签名与 RA，再验证实验源不可访问并在全新 RB 恢复 | 使用实际产品与独立目标证据。目标普通观察、工具测试和单机夹具均没有正式验收信用。 |

工具只负责 C 文件阶段的待启用部分。运行时 policy、registry、fence、session anchor、D trust、Capture 挂载和 S 侧真实构建材料仍按原 B/C 分工冻结，不能由 installer 从磁盘残留自动重建。最终发布前保存有效 SSH、PAM、sudo、进程与资源边界证据，覆盖空命令、附加参数、非批准来源、错 key、shell/SFTP、转发、TTY、环境及非批准 sudo 动作。终止和撤销精确作用于本批新对象，任何父进程异常或结果未知继续 HOLD。

## 检查与审查结论

在无网络用户命名空间中，4 项 Python 检查通过，覆盖固定配置、注入拒绝、无覆盖和 chroot 内真实文件写入/同步/回读。该 chroot 使用合成文件，未加载 receiver。4 项 Linux native 检查通过，覆盖固定请求、管道方向和重复 inode、父进程关闭管道后的实际 EOF，以及子进程退出码和信号。两个工具角色以合成输入构建并生成 fixture bundle；本地 C 版 `visudo` 解析通过。解析使用了不同版本，用户命名空间内有 `/etc/sudo.conf` owner 映射提示，不能替代 D 生效规则核验。

审查结论为 **`hold_for_evidence`**。冻结工具补丁 SHA-256 `2c9b977c51041e56eff358a5412edfe34d38216327a505b86972dea69e0aa5ac`，结构化审查 SHA-256 `8166017947e91214d9621d216f710ccb2af70b0e35b63c5f35ad5a7e3c57c564`。候选代码和本地证据可以保留在 feature 分支供审查。安装前仍缺三类原件：D 生效 SSH/PAM/sudo 与既有规则排除；实装对象和公钥/ELF/父目录身份清单；目标上的 UID/父 ELF/FIFO/EOF 及完整拒绝矩阵。H4 和 Task9 正式验收保持开放。
