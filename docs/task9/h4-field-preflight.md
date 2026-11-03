# Task9 H4 现场前置核验与只读审批单

2026-11-03。此单补充 [H4 安装候选](h4-installation-materials.md)，保留已完成的本地 8 项工具验证和现场审查 `hold_for_evidence`。

## 本次实测

北京时间 18:38，S、D 均沿用已有 SSH 主机信任，以各自 UID 1000 的普通账号执行一次限定读取。

| 对象 | 取得的事实 | 仍需确认 |
| --- | --- | --- |
| D 监听地址 | 现有管理连接的 D 端 IPv4 与 D 本机全局作用域网卡地址对应，可作为 22229 的本地 bind 候选。两个待装父目录 `/etc`、`/usr/local` 已读取 `dev/ino` | 22229 的外部映射、防火墙及对 S 的真实可达性，运行前复查父目录和端口 |
| D 现有主机公钥 | `/etc/ssh/ssh_host_ed25519_key.pub` 的 wire blob SHA-256 与已知信任记录和本次管理握手一致 | 22229 实际握手及 S 使用的端口绑定信任，待 B/C 实例化后验证 |
| S 路由 | S 到 D 的内核路由查询返回 unicast，有网关，选出的 S 本地源地址为非公网地址 | D 实际接收的源地址。路由查询未向 D 发包，没有观察 NAT 后地址 |
| D 管理权限 | 普通账号对当前管理连接条件执行 OpenSSH `-G` 解析，退出 255，未取得可用配置。sudoers 私有冲突保持未知 | 核实实际 daemon 使用的配置与管理侧排除条件，再按真正账号和连接条件核生效规则 |

原始私有观测分别为 D `8593078773189ce45fc937abfef589165acb850eca8880ee54ed8d062700dc8e`、S `d9aa5a23b73f82ed778afcb653903afc6e9a9b9242702360d208b63e15cb21a8`。实际地址、指纹和目录身份留在私有侧。

已将 D 的候选 bind、公开 host pin、父目录身份填入私有请求草稿。`source_ipv4`、专用 `uid`、`gid` 和 `transport_public_key` 仍为空。实际运行 installer 的输入校验确认拒绝这份草稿，因而当前没有渲染、可安装清单或授权摘要。

## 下一步最小只读范围

以下 D 侧 A 范围待单独批准。批准对象是一次本机只读核验，由现有管理路径进入，输出只保留必要的布尔状态、规则来源和摘要。涉及路径或权限不符就停。

1. **管理监听。** 确认 22 的真实进程、服务或 socket 单元以及配置文件与启动覆盖关系。核 `/etc/ssh/sshd_config` 和其实际加载的 `/etc/ssh/sshd_config.d/` 条目。若配置引用其他路径，先列出来源并停在该对象。关注 `AllowUsers`/`DenyUsers`、`Match`、账号组、用户 key 文件、`AuthorizedKeysCommand`、用户 CA、口令和可用转发。若安装版本能以配置解析模式输出规则，可在确认其实际参数语义后取所需字段。`-G` 的本次失败不推断为任何规则结论。会执行密钥健全性测试的 `-T`/`-t` 留到有明确 C 验证范围时。 [Ubuntu sshd 手册](https://manpages.ubuntu.com/manpages/noble/man8/sshd.8.html)
2. **Sudo 与 PAM。** 先核固定候选 `/etc/sudoers.d/knowweave-c4-b1` 是否占用，核 `/etc/sudoers` 与其实际加载的 `/etc/sudoers.d/` 条目。用已确认的 include 闭集检查语法和可能命中的账号、组、通配、默认项及外部策略来源；语法通过仅计语法。只读取 `/etc/pam.d/sshd`、`/etc/pam.d/sudo` 与其在 `/etc/pam.d/` 内的实际 include，确认哪些 account/session 规则适用，检查是否可能写入 protocol stdout。目录外 include、LDAP、外部授权命令或既有宽授权保持 HOLD，等待准确范围。不得凭空置账号证明有效 sudo 权限。 [Ubuntu sudoers 手册](https://manpages.ubuntu.com/manpages/noble/man5/sudoers.5.html)
3. **S 出口和 C 二次核验。** 优先用已有网络/NAT 或 D 接收侧记录，把 S 的目标流与 D 看到的地址对应。缺少记录时，单列一次范围固定的连通探针再批准。取得真正的账号 UID/GID 和公钥后，按该账号、实际源地址、目标地址和端口复核管理监听排除、隔离监听匹配和有效 sudo 权限。候选账号目前尚不存在，A 阶段只能发现冲突或缺口，C 的二次核验不可省略。

本次待批只读范围不含创建账户、读取私钥/密码内容、扩大安全组、写 sudoers、启动 listener 或改 S。审核完成并取得真实 S 出口和材料后，才可提交 [B/C 的具体写入清单](h4-installation-materials.md)与实际 ELF、owner/mode 和摘要。Task9 正式验收继续开放。
