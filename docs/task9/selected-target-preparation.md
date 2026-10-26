# Task9 已选目标与平台预检

## 当前状态补充：时序语义已采纳，现场事实未更新

2026-10-26 用户选择“采纳方案，仅定稿规范（推荐）”，见[采纳记录](b1-final-decision-adoption.md)与[完成规范](../superpowers/specs/2026-10-04-p0c4-completion-design.md)。原提案 `2448f459…` 继续原字节保留；其中未采纳状态描述审批前时点。

这次只推进规范定稿/独审，没有重新连接S/D或运行测试，不能把此前点时元数据升级为新的现场安装观察。目标平台六项、Linux无权限帧20正文与各自原件/限制原样保留，不相加或新增验收信用。管理账户与自动transport身份分开；key/trust/registry、源身份/权限、fence落地、源隔离与恢复均未获本次批准或执行，Task9/C4未验收。

> 下文“最终决定仍待采纳”属于此前准备检查点；最新语义决定以本节及采纳记录为准。旧失败与HOLD不变。

---

更新：2026-10-26。**状态：目标已选、限定准备已执行；独立域/安装/B1/恢复正式验收未完成。**

## 当前事实与证据层级

用户指定与开发源端分开的腾讯云 Linux 主机作为备份目标，并确认管理该主机和云盘。备份签名私钥的方案为仅保存在目标端 root 私有目录；这不批准生成/安装 key，也不批准新增管理认证凭证、trust、registry、安全权限、源隔离或恢复写入。

目标普通账户实际 SSH 观察于 2026-10-26 08:47:48 UTC（北京时间16:47:48）：Ubuntu 24.04.4 LTS、x86_64、腾讯云 CVM、2 CPU、约2 GiB内存。单块50 GiB虚拟云盘、根ext4，`/var/lib`所在文件系统当时可用39,425,306,624 bytes（约36.7 GiB）。这些是点时元数据，不证明云存储底层故障域、备份空间峰值或硬件断电持久性。

当时 `/var/lib/knowweave-c4` 及其固定 registry/verifier/key/trust 路径不存在；这是此次目标普通身份元数据观察。此前源端私有目录 `UNKNOWN_PERMISSION_DENIED` 的旧观察仍保留，不能把它改写为源端未安装或借目标身份重读它。

目标 Docker 盘点在08:48:29 UTC看到已有MySQL和Redis运行，另有退出的测试数据库容器；本轮未修改任何已有容器、卷、网络或服务。未在目标部署编译工具。目标机只读盘点与后来普通账户新隔离目录中的合成预检分开记账。

公开文档不包含连接密码、私钥、token、实际IP或账号；完整路径/非秘密资源归属保存在私有运行单。原始密码未写入项目脚本、配置、Git或本批证据。

## 普通账户文件系统预检：6/6

新建普通账户私有隔离目录，保留少量合成文件与脚本，不使用sudo。实际通过：

1. 排他新建、完整写入和文件fsync。
2. `renameat2(RENAME_NOREPLACE)` 对已有目标拒绝，保留两边原字节和身份。
3. 对新目标成功重命名、保持inode并同步目录。
4. `O_NOFOLLOW` 拒绝合成symlink。
5. 持有文件描述符的原inode与完整字节回读。
6. 当前隔离根身份保持，根目录/父目录同步。

这是实际目标上的原生Linux平台检查，不是Rust产品、root固定安装根、云后端灾难持久性或37条Task9集成用例验证。`/home`与`/var/lib`当时同device的观察只作位置参考，不能把普通账户目录当作已安装的root管理根。

| 原件 | SHA-256 |
|---|---|
| `probe.py` 实际执行源码 | `337245461fb2f6004613d54bb691cd10826e4ce757b1bf4459afda091d90cdea` |
| `result.json` 六项结果 | `41e9db4abdaf6ab954dc847d31f1b76eade864c898f181e7af88e402524e1fae` |
| `target-readiness.json` 私有准备计划 | `f97b1886d17babb3e15f890f72d0862e04157de71660c0e12f209705fa52d912` |
| `NEXT-STEPS.md` 私有下一阶段单 | `660f15fb138a0b1d6184a87e6fcb39d8ef19a9bbe896e4875c66eda81612d01c` |
| `ROOT-READBACK.json` 原会话自查收据 | `bb12d70fc0c95af635bd4ad2a94a46dc9e0e710273a61b295d45890afb59c90f` |

Root实际回读四份文件，核对摘要/普通文件身份以及写入计划前的七项目录清单。它是同一Root会话的自查，**不是另一观察者对远端原文件的独立审查**。本轮后续B1设计独审也不被回填成当时平台预检的独立现场见证。旧私有计划中“独审代理额度受限”的说明是该时点状态；后续同一代理已接手设计独审，未重写旧原件。

## 已关闭的准备缺口与仍未满足的条件

- 已有选定的异机目标、可用管理连接和用户确认的托管方案；不能继续把“尚未提供目标”写作当前状态。
- root私钥、trust/registry、构建公钥pin和源manager认证身份仍未安装/核实；普通文件权限和SSH账号不单独证明独立托管或原活操作。
- B1具体接入候选见[异机活操作RFC](b1-offhost-admission-rfc.md)。旧 `finalize_use` 不是一次消费；旧publisher的本地根/新lease路径也不能接上远端IP直接使用。
- S原manager的可信独占身份、跨机死亡/撤销线性化及D最终消费仍是阻断。心跳、最后消息或持久负fence都不能未经规范裁决替代原S活性。
- 历史[37条规格](b1-preparation-test-matrix.json)保持原字节与未执行状态；其中旧 `independent_target.available=false` 属于c1时期准备输入，**不是实时目标盘点**。现有离线检查器也不查询当前目标可用性，不因结构通过授Task9资格。
- 下一步按[执行准备单](execution-readiness.md)和RFC分层推进；安装、安全权限、源隔离和恢复仍需各自具体审批。

正式Task9五门、C4、正常CompleteBackup/RecoveredInstance与生产保持未验收；Task8不重跑、不重新记账，旧失败/HOLD继续保留。

## 后续：无权限 Rust 帧准备的 Linux 检查

子代理仅新增 `scripts/rust/task9_frame_preparation.rs`，顶部 `#![cfg(test)]`，不被产品crate或Cargo工作区导入。它只处理不可信的固定header、长度/预算、序号和语法流关闭，不接文件/网络/密钥/真实根/认证/活性/首签或发布。Metadata payload未做JSON语义认证，上游真实IO的分配界限也未实现；不能把纯 `finish` 当实际socket EOF。

| 项 | 结果与证明边界 |
|---|---|
| 最终源码 | 19,585B；SHA-256 `4265db3b16386b70132e429786f6729d287f5a490f9a7b6c6de5b3863b8c3bc1` |
| 独立代码审查 | 限定允许无权限framing；不批准B1 authority、安装/启用或Task9 |
| 实际Linux批次 | `902dabe2-4c23-4cd3-a97a-d46ec25cb089`，既有开发源端固定builder，不在Windows执行 |
| 工具链/格式/编译 | Rust1.97.0；本文件rustfmt检查、`rustc --edition=2024 --deny warnings --test`均退出0 |
| 列举与正文 | 列举20；实际20 passed、0 failed、0 ignored；两阶段分别有原始日志与退出码 |
| 实际结果原件 | `d98efdaf4ca23d4549dc63fa3f12ab22bebff4332e38625a46b3bfc1f5804ccf` |
| 隔离/收尾 | 新私有目录；driver检查固定镜像、无网络、只读源码/根FS、UID1000、2CPU/1GiB；精确ID删除builder，源码/日志/二进制保留。完整前后Docker inspect原件未封存，派生身份摘要不能冒充完整原件独证 |

首次尝试在创建批次/上传之前因原实验父目录0775被guard拒绝；只读核实后使用新私有目录，没有放宽guard或改旧目录权限。第一实际批次 `90ad0db8-2821-4db7-9fcd-c0df1afa5908` 的format退出1，编译/列举/正文未运行，源码为18,129B/SHA `5fd2988d72154cbaa58f2443032e95ee091383a79cf10fc1cbdf1019872541c0`；失败结果 `dece26516037ea738f54f55b94572c0bc4b8891aa23c76c52ef0b637987a6ae9` 保留。之后仅应用18处真实Linux rustfmt差异，核心与20正文逻辑不变，使用全新UUID重跑。

20是实际执行的纯解析器测试body，近512MiB预算/溢出测试使用私有白盒计数器，不是512MiB网络传输。没有执行Cargo工作区、Clippy、PostgreSQL、37条B1集成、首签/新发布或恢复；不把20与历史19/38或目标FS6相加。原七份产品源码pin、十份旧工具字节桥和37条历史规格保持。正式门不推进。

另一子代理随后仅以既有SSH身份只读采集上述两个Linux批次，独核34份原件的普通文件/所有者/链接/前后元数据与全量SHA；其中33份非binary作字节副本，7,084,816B测试binary只在128MiB上限内分块hash、未复制或执行。20项列举和正文逐项一致，旧format失败不计通过。独审INDEX SHA-256为 `d896ab257a894daedd14e43ec528dab8a1c22c8d188a1a1099306ad7bd1d5276`，结论仅为20个实际Linux纯parser正文的准备范围。

该独审没有补造旧测试进程/时钟或完整Docker原件；阶段现有exit+双流，不具备独立OS process原件。新SSH采集收据只见证此次只读采集，不能冒充当时运行观察。没有再跑测试，也没有获得Task9、B1、workspace或Clippy信用。

## 后续检查点：最终决定合同仍待语义采纳

[最终决定时序合同建议](b1-final-decision-contract-proposal.md)已封存，SHA-256 `2448f4592a64bc9bc5ec5b2e618f936b8f47412c2ab0c23526ae3623d6b7ae59`；状态仍为 **NOT_ADOPTED / NOT_IMPLEMENTED / NOT_AUTHORIZED**，提案正在审阅，尚未采纳。当前下一步是明确是否接受其中的S死亡窗口、D一次消费、负fence及UNKNOWN/历史只读分账语义，不是重复增加parser。语义采纳也只允许后续正式规范定稿及独审，不授权R2b权限编码或机器配置。

补充的目标观察来自Root工具结构化元数据回报（2026-10-26 11:56:38 UTC），不是另一观察者的独立原始采集或独核：管理身份可访问本机系统Docker Unix socket，daemon info未报告rootless。不能把该管理身份默认当作受限自动transport身份；Docker官方说明docker组具有root-level权限，这是官方权限模型，与上述点时观察分别记账。[Docker官方权限说明](https://docs.docker.com/engine/install/linux-postinstall/#manage-docker-as-a-non-root-user)。未据此实施root容器、密钥读取或恶意管理员防护验证。

D key/trust/registry、负fence namespace、S管理认证和受限传输身份、固定调用权限，以及实际首发、源隔离和恢复仍须具体方案、独审及对应执行审批；管理登录或托管方案选择不覆盖它们。Task9五个正式门、37条历史规格及旧失败/HOLD不变。本次仅追加文档检查点，没有新测试或验收信用。
