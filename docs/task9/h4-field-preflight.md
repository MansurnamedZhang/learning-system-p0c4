# Task9 H4 现场前置核验与只读审批单

2026-11-03。此单补充 [H4 安装候选](h4-installation-materials.md)，保留已完成的本地 8 项工具验证和现场审查 `hold_for_evidence`。

C 的当前结果和分阶段审批见[安装前核验与最小审批](h4-c-installation-preflight.md)。

## 19:22 单次出口探针结果

用户批准的一次探针已完成。S 对 D 现有 22 端口只建立一次连接并发送一个版本标记，认证尝试为零。D 在 619 毫秒内取得唯一匹配，观察进程已退出。S、D 回执的标记摘要一致。原始私有结果 SHA-256 `0959612cafb9d71e5f536f075c3f7443e86be0a22938a164d41952016d12d087`。

已将 D 所见的 S 地址填入私有候选，只对当次 22 路径有效。22229 的实际来源和连通性须在 C 复核。B 回查已补入传输公钥。该次观察时仍缺实际 UID/GID。后续专用账号已回读，私有请求已通过字段检查。目标端 P-256 签名密钥已在[仅 D 的补齐批次](h4-key-custody-decision.md)中封存，正式安装和 22229 待验。

## 18:43–18:45 观测与单次探针方案

北京时间 18:43，沿用已有管理路径在 D 执行一次非交互 root 只读检查。只取 22 监听元数据和已列范围内的磁盘规则，未运行 SSH 密钥健全性测试、PAM 会话或新账号认证。实际得到：

- 22 可见两个监听条目，`ssh.service` 和 `ssh.socket` 均活动。观察到 `sshd` 及另一 socket 所有者。范围内盘点了两份 SSH 配置和三份 sudoers 配置。候选 `/etc/sudoers.d/knowweave-c4-b1` 当时不存在。
- SSH 和 sudo 的 PAM 规则分别遍历五份和四份范围内配置。SSH 规则含 `pam_env` 与 `pam_motd`，sudo 规则含 `pam_env`。这要求 C 在真实无 PTY 会话中确认字节通道和环境；静态规则的存在不代表一定输出。
- 原件 SHA-256 `4c934a9e7fb1a4c586a6c5c8d105568e0b9711dc9e419037dc179e1c605e0381`。规则盘点不证明未来专用账号被管理 SSH 排除，也不证明该账号实际只取得一条 sudo 权限。

北京时间 18:45，D 普通账号复核了已观察的 bind 对应网卡以及已有 `tcpdump`。用 D 自带 libpcap 的离线句柄编译过滤条件并检查八个合成帧，准确标记通过，错标记、错端口、错目标、截断、分片、额外或缺少负载均拒绝。未打开实时抓包，S 未发送探针。离线原件 SHA-256 `565480366b5e644f06bb50a4dc7f7ceccbe53e483148853b48b235fa67260c0a`。

**该次探针的批准范围。** 先检查当前 D 网卡和本机地址仍与只读证据一致。D 在该网卡以非混杂、仅入站方式过滤到本机 IPv4 的 TCP 22，过滤器固定完整的新鲜随机 SSH 版本标记及长度。D 观察进程最长 15 秒，只接受首个准确匹配，输出仅保留该连接的来源元数据。就绪后 S 以普通账号只建立一次到 D 现有 22 的连接，发送无秘密的 SSH 版本标记并在密钥交换和认证前关闭。一次无匹配、权限拒绝或目标变化就停止，无自动重跑。不会生成 pcap、输出其他流量或调整监听、防火墙与信任。 [Ubuntu tcpdump 手册](https://manpages.ubuntu.com/manpages/noble/man8/tcpdump.8.html) · [libpcap 过滤语法](https://manpages.ubuntu.com/manpages/noble/man7/pcap-filter.7.html)

只有这次实际匹配才能填入 D 所见的 S 来源候选。结果仅对应当前 S→D 的 22 路径，22229 的外部映射、实际来源及真实运行链仍在 C 复核。此决定只覆盖上述一次探针。账户、sudoers、监听配置、密钥和正式验收均是后续范围。

## 18:38 普通账号实测

北京时间 18:38，S、D 均沿用已有 SSH 主机信任，以各自 UID 1000 的普通账号执行一次限定读取。

| 对象 | 取得的事实 | 仍需确认 |
| --- | --- | --- |
| D 监听地址 | 现有管理连接的 D 端 IPv4 与 D 本机全局作用域网卡地址对应，可作为 22229 的本地 bind 候选。两个待装父目录 `/etc`、`/usr/local` 已读取 `dev/ino` | 22229 的外部映射、防火墙及对 S 的真实可达性，运行前复查父目录和端口 |
| D 现有主机公钥 | `/etc/ssh/ssh_host_ed25519_key.pub` 的 wire blob SHA-256 与已知信任记录和本次管理握手一致 | 22229 实际握手及 S 使用的端口绑定信任，待 B/C 实例化后验证 |
| S 路由 | S 到 D 的内核路由查询返回 unicast，有网关，选出的 S 本地源地址为非公网地址 | D 实际接收的源地址。路由查询未向 D 发包，没有观察 NAT 后地址 |
| D 管理权限 | 普通账号对当前管理连接条件执行 OpenSSH `-G` 解析，退出 255，未取得可用配置。sudoers 私有冲突保持未知 | 核实实际 daemon 使用的配置与管理侧排除条件，再按真正账号和连接条件核生效规则 |

原始私有观测分别为 D `8593078773189ce45fc937abfef589165acb850eca8880ee54ed8d062700dc8e`、S `d9aa5a23b73f82ed778afcb653903afc6e9a9b9242702360d208b63e15cb21a8`。实际地址、指纹和目录身份留在私有侧。

已将 D 的候选 bind、公开 host pin、父目录身份填入私有请求草稿。`source_ipv4`、专用 `uid`、`gid` 和 `transport_public_key` 仍为空。实际运行 installer 的输入校验确认拒绝这份草稿，因而当前没有渲染、可安装清单或授权摘要。

## 18:38 时的只读范围记录

以下保留当时提出的 A 范围。已取得的磁盘规则和监听事实见上方，尚未执行的生效权限核验继续留在准确 C 范围。

1. **管理监听。** 确认 22 的真实进程、服务或 socket 单元以及配置文件与启动覆盖关系。核 `/etc/ssh/sshd_config` 和其实际加载的 `/etc/ssh/sshd_config.d/` 条目。若配置引用其他路径，先列出来源并停在该对象。关注 `AllowUsers`/`DenyUsers`、`Match`、账号组、用户 key 文件、`AuthorizedKeysCommand`、用户 CA、口令和可用转发。若安装版本能以配置解析模式输出规则，可在确认其实际参数语义后取所需字段。`-G` 的本次失败不推断为任何规则结论。会执行密钥健全性测试的 `-T`/`-t` 留到有明确 C 验证范围时。 [Ubuntu sshd 手册](https://manpages.ubuntu.com/manpages/noble/man8/sshd.8.html)
2. **Sudo 与 PAM。** 先核固定候选 `/etc/sudoers.d/knowweave-c4-b1` 是否占用，核 `/etc/sudoers` 与其实际加载的 `/etc/sudoers.d/` 条目。用已确认的 include 闭集检查语法和可能命中的账号、组、通配、默认项及外部策略来源；语法通过仅计语法。只读取 `/etc/pam.d/sshd`、`/etc/pam.d/sudo` 与其在 `/etc/pam.d/` 内的实际 include，确认哪些 account/session 规则适用，检查是否可能写入 protocol stdout。目录外 include、LDAP、外部授权命令或既有宽授权保持 HOLD，等待准确范围。不得凭空置账号证明有效 sudo 权限。 [Ubuntu sudoers 手册](https://manpages.ubuntu.com/manpages/noble/man5/sudoers.5.html)
3. **S 出口和 C 二次核验。** 优先用已有网络/NAT 或 D 接收侧记录，把 S 的目标流与 D 看到的地址对应。缺少记录时，单列一次范围固定的连通探针再批准。取得真正的账号 UID/GID 和公钥后，按该账号、实际源地址、目标地址和端口复核管理监听排除、隔离监听匹配和有效 sudo 权限。候选账号目前尚不存在，A 阶段只能发现冲突或缺口，C 的二次核验不可省略。

该只读范围不含创建账户、读取私钥/密码内容、扩大安全组、写 sudoers、启动 listener 或改 S。取得真实 S 出口和材料后，才可提交 [B/C 的具体写入清单](h4-installation-materials.md)与实际 ELF、owner/mode 和摘要。Task9 正式验收继续开放。
