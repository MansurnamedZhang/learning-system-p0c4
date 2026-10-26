# Task9 B1：异机活操作接入实施候选

日期：2026-10-26。基线：`600c6ee4fdf1ec08f4867aa16e429b6a71315256`。

**状态：设计候选，待独立审查及裁决；不是已采纳权限合同，不授权安装、启用或恢复。** 本文不改原 Appendix B/B1、公共 v2 记录、既有权限入口、37 条未执行集成规格或完整恢复引擎。文中的类型、模块、帧和阶段都是拟议名称，不是已经存在的 API。

## 1. 已具备的输入

- 用户指定一台与开发源端分开的腾讯云 Linux 主机作为目标，并确认由其管理主机与云盘。
- 用户选择备份签名私钥仅在目标机 root 私有目录托管。此为方案选择，不是密钥生成、trust/registry 安装或权限变更的执行批准。
- 目标点时盘点为 Ubuntu 24.04.4、2 CPU、约 2 GiB 内存、50 GiB ext4 云盘；当时可用约 36.7 GiB。MySQL/Redis 已在运行。固定安装根当时不存在。
- 普通账户新建隔离目录内，独占文件写入/同步、禁止覆盖重命名碰撞、新名称重命名/目录同步、no-follow 拒绝、持有描述符回读和根/父目录同步共六项平台预检通过。没有实际 B1 用例、root 安装、签名或恢复执行。实际云存储故障域、峰值容量和未来固定根准入仍需核实。

公开文档只使用 S（开发源端）与 D（云端目标）角色名；地址、账号和私有证据路径保留在私有运行单。

## 2. 现有代码的三个接入限制

1. `complete.rs:252–257,327–368` 同时打开源端和目标端的本地 `BackupDir`，并取得新 registry lease。异机 IP、源端映射目录或不同 `st_dev` 不能代替受审的跨机访问和准入。
2. `ProtectionSnapshot::require_same_operation` 约束原 S lease/预算/根；`destination/attempt.rs::publish` 只在原 D 操作自然完成十一阶段及 outcome 后登记成功。二者都不能从复制的 proof/terminal 或新 lease 重建。
3. `destination.rs:696–770,862–918` 的首签与新发布还消费原 D 权限。有效 witness、启用 feature 或装入 key 不使 fresh-lease publisher 合法。

因此保留旧 API 的拒绝条件，新增经过明确准入的异机路径；不把旧函数中的 `ptr_eq`、root 校验、源保护检查或 fresh-operation 拒绝删掉。也不让远端目录冒充本地 `BackupDir`。

## 3. 推荐评审的部署形态

候选为 **S 发起的一次性 SSH 命令通道，D 运行固定接收/验证程序**。标准输入/输出只承载有界协议；不新增公网业务端口、常驻签名服务或通用 shell/SQL/路径接口。D 不需要反连开发内网。

SSH 主机身份继续严格核验，禁止 `accept-new`/禁用校验。首次从 S 建立连接所需的主机信任、登录身份、凭证或固定 root 命令权限若尚不存在，均另列准确变更并审批。现有 Windows 管理端登录成功不代表 S 已具备这些条件。目标当前有 Docker 访问能力也不是批准新增 root 调用权限。

D 的签名 key 只签现有 v2 `DestinationStatement`，不复用它签任意握手、挑战或调用者消息。私钥不导出到 S、Windows、Git 或证据副本。

### 3.1 SSH 登录身份不能替代 S 原操作

能够打开 SSH 的操作者仍可能自行编造协议帧，所以不能只凭 SSH 账号、环境变量、命令行字段或所报 executable hash 构造 `AdmittedRemoteSource`。

拟评审的认证选择是：**独立配置的 S 管理认证身份，只由受审的原 S 操作管理程序使用**。它与 D 备份签名 key 分离。候选文件合同为 S 的 root 私有 `transport/source-auth.pk8`，以及 D 上固定、root 管理且绑定受审 S 构建/部署身份的公钥信任；具体路径、记录格式和权限尚须作为 C 包裁决，不自动扩大现有文件合同。

若选择此方案，新增持久认证 key/信任属于新的安全配置，必须单独审批；本稿不生成 key。也可以提出经过证明的可信窄 broker 替代，但不能以普通 SSH 身份、调用者提交的证明或通用 signer 代替。

S 管理认证函数不接受任意待签字节：它只在原 actor 内部，从仍持有的源准入、原 registry lease、预算、源/构建绑定和冻结保护快照派生消息。调用者不能提供“已成功/已授权”字段、任意 source key、任意 root 或 capability 构造器。加载身份与实际管理程序/命名空间的可信绑定也必须独审；代码注释称“root 管理”本身不是证明。

## 4. 拟议职责与接口分层

| 模块/类型候选 | 拥有的状态 | 明确不授予的资格 |
|---|---|---|
| S `LiveSourceSession` | 原 `SourceAdmission`、原 lease/预算、源根/构建绑定、当前保护、持有的包描述符 | 不从磁盘或 wire 构造；不在 Drop 后保留可重开的活 S 权限 |
| 有界 `SessionFrame` 编解码 | 版本、方向、序号、挑战、绑定摘要、有限数据帧与明确消息类型 | 解析成功不产生 capability、signer、root 或成功状态 |
| D `AdmittedRemoteSource` | 已认证的本次管理程序、挑战/通道、原 S 阶段证明与受限来源绑定 | SSH 认证或单张签名不够；不能跨会话/进程/新 nonce 复用 |
| D `LiveDestinationAttempt` | D 原 lease、真实写入/rename/readback/terminal 十一阶段、原内存成功表 | 不从 terminal、完整 checksum 或重连恢复成功权限 |
| D `PublicationDecision` | 本次 C 的最终源核对与原 D 成功状态共同产生、一次最终消费 | 没有 serde/public constructor；不是可落盘/传递给新进程的 permit |
| D 接收端内部 publisher | 当前 D 根、原 D 成功状态、受审最终决策、现有 v2 receipt writer/verifier | 不调用原 fresh-lease 入口来假装继承；不改公开 v2 与历史恢复资格 |

源文件捕获、D 证据写入和新收据编码尽可能复用现有实现。若需要提取共同 writer/validator，必须分别证明原本地分支字节、错误语义与拒绝覆盖不变。新接口即使 crate-private，只要开启原来拒绝的首签/发布，仍属于权限集成，必须先审。

### 4.1 P 的一次消费合同候选

现 `attempt::finalize_use` 只检查并串行使用 `successful` 项，**不会删除成功项，不能称作 one-shot consume**。新 P 必须额外维护只属于原 D/C 的 `Pending → Consuming → Consumed` 或 `Pending/Consuming → Failed`，没有回退到 Pending 的分支；同 ID 的第二次消费及任何 fresh process 均拒绝。

准确候选位置为：原 D authority 与本次 C 的最终决策均验证后，在同一个 D 串行裁决内，从 Pending 进入 Consuming；其 action 包含内部签名、v2 receipt 的排他写入/同步/禁止覆盖发布/全量最终验证。全部自然成功才置 Consumed。任何错误、取消裁决或 unwind 置 Failed 并失效相关成功状态。是否可以扩展原 operation 状态还是需要同一 actor 下的组合锁，须实现前审查；不能用两个无排序 mutex 制造窗口。

新路径不在完成该消费之前向外提供可用 witness 字节或返回成功对象。签名不得先于最终许可而外泄；部分字节、失败残留和旧签名不能使新进程再消费。该合同仍是待审候选，不修改旧本地 API 的行为。

## 5. 有界通道和不可替换绑定

候选控制帧使用固定版本/类型、严格字段集、拒绝重复键、固定规范编码及长度前缀；数据帧另有固定长度上限和总预算。具体字节上限在 R2 实现前由现有包/metadata 上限和实测峰值裁决，不允许调用者调大或“尽量读到 EOF”。

通道绑定至少包含：双方独立生成的新挑战、方向/序号、协议域分隔、backup/attempt、manifest/proof/source-control、source group/build、双方 registry 代际和各自根的原拥有关系。挑战与新会话不能来自调用者可重放的旧 transcript。

### 5.1 已允许单独实现的准备帧 profile

独审只允许开展不接权限的编解码准备；这不是已采用的运行时 wire 合同。独立文件 `scripts/rust/task9_frame_preparation.rs` 顶部为 `#![cfg(test)]`，不被产品 crate 引用。固定 20-byte header：`KWB1 | version=1 | kind | direction | reserved=0 | sequence:u64be | payload_len:u32be`。

候选 kind 为 Metadata/Data/End/Abort，双方向分别有独立 decoder；序号从 0 连续增长。首帧 Metadata 且仅一次，后续 Data 非空；End 只允许在 Metadata 后，Abort 可终止尚未关闭的语法流。End/Abort、任何错误及最终 EOF 检查后的状态不能重新接收数据。Metadata 上限 16,384B，Data 上限 65,536B，Abort 上限 256B，End 为零；最多 8,192 帧，含 header 的累计 wire 不超过 536,870,912B。上限不可由调用者调大；这些是准备 profile 的拒绝边界，不是产品 package/metadata 预算的批准。

`inspect_header` 可以在复制/分配 payload 前检查长度，不能推进 decoder 或生成权限；`decode` 只借用已提供的 frame 字节。当前不实现 socket/子进程 reader，因而不证明上游 IO 在分配前有界，也不把纯 `finish` 检查称作真实 TCP EOF。Metadata/Abort payload 仍是不可信 opaque 字节，不声称完成 JSON 语义、来源/构建认证、transcript 或应用阶段检查；这些属于以后受审的一层。无 key、文件、网络、真实根或首签/publisher 接入。

跨主机不比较 `st_dev`/inode 是否相等来证明独立性；各自主机只验证其本地持有根。源 dev/inode/path 在 D 的捕获证据中仍是历史值，不能和 D 副本 inode 混淆。身份 UUID 仅作经核实的部署标签，不是物理故障域证据。

S 只从原 actor 的已准入/持有对象生成固定证据与 package 文件流；D 只写独立安装并准入的 destination/evidence 根，不接受远端路径、SQL、命令、任意文件名或 root 注册参数。包内允许的逻辑名字仍由原 manifest/parser 校验，不能借 framing 放宽。

签名认证只证明固定来源/消息归属；阶段状态、原 D 成功、当前源保护和最终消费必须另外成立。即使某帧全部摘要自洽，缺任何一个活上下文也拒绝。

## 6. 最终提交/撤销：必须先裁决的核心

拟议的阶段为：

`SESSION_BOUND → SOURCE_CAPTURE_FROZEN → DESTINATION_BYTES_VERIFIED → DESTINATION_EVIDENCE_COMMITTED → SOURCE_FINAL_RECHECK → FINAL_DECISION → RECEIPT_WRITE_AND_VERIFY → REPLY`。

D 在证据自然成功后保留原成功 authority。最终决策需要原 S actor 在其本地串行点重核原保护，D 在其本地唯一消费点同时核对该决策、本次 C 和原 D authority。D 的撤销/消费必须在同一锁/单一 actor 下排序；不能隔着 SSH 声称仍拥有旧的同进程 mutex。最终 action 不递归锁定/失效同一个 context。

### Q1：原 S 死亡与跨机最终决策

**不能同时未经证明地承诺“任何时刻 S 死亡都立即阻止 D 写入”与“仅凭最后一次消息继续提交”。** 网络无法让 D 即时观察尚未送达的死亡/取消事件；心跳、再检查一次、短 TTL 或多轮 ACK 都不能消除最后一条消息窗口。

候选决策是把 S 原 actor 的最终授权选择明确区分为“可撤销准备阶段”与“一次性、仅限本次仍活 D 尝试的最终决策阶段”。后者是否可构成 B1 中单独准入的新发布 authority、其准确线性化位置以及 S 后续死亡/取消的语义，**必须由本次规范独审明确接受或否决**。这不是从旧 terminal 重建 D 成功表，也不能成为可重放的签名 permit；但它可能改变此前草案对 S 全程存活的隐含要求，因此不能直接实现后宣称兼容。

若审查要求 S 必须一直存活直到 D 最后物理写入，而又没有可证明的共同裁决设施，则这个 SSH-only 候选不能进入启用；保持入口关闭，另行提出受审协调机制，不能把不可观测事件改成成功、偷偷放宽合同或用测试夹具跳过。

### 6.1 持久负 fence：可继续评审，尚未采纳

另一个保守候选是在原 S 最终选择之前，从原持有上下文独占发布、同步并回读一个持久负 fence。它只能阻止破坏保护观察的后续变更，不能授首签/新发布，不能从其字节、签名、重读或重启重建原 S/D authority。

它需要独立准入的固定 namespace、防替换身份、共同串行化、失败/未知版本/超限时拒绝，以及所有后继写入方（含旧二进制、重新登记/恢复入口）的强制遵守。保护范围须包括所有会改变原 `SnapshotRecord.records`、计数、generation 和根身份的变更，不能只挡 GC/删除/释放。UNKNOWN 不自动释放；达到容量/扫描上限继续拒绝；任何释放机制另审，不按 TTL、重连或“看似成功”修复。

此负 fence 只能帮助保护状态连续性，**不证明 S 进程仍活着，也不构成 C/P 的新权限**。若用它承接原草案的 S 持续存活义务，仍须明确修订/采纳规范。共同可信管理员和所有相关写入方服从约束是必要假设，不声称防御任意 root 修改。新增 namespace、记录和相关扫描/预算均未实施。

### Q2：断线与结果未知

- 最终决策前已观察的错误/撤销/EOF/超时：关闭新首签/新发布，失效原 D/C 权限，保留残留。
- 已进入经审最终消费但结果丢失：调用方返回 UNKNOWN/HOLD；不自动重连、重发、再签或同 ID 重跑。不得断言对端没提交/已回滚。
- 已真实完成并验证的历史 CompleteBackup 保留既有目的端 only 重开资格；迟到取消不追溯否认它，也不授下一次新发布。
- 可见 receipt、完整 witness 或后来的 fsync/readback 不能用来补造当时的成功返回；各阶段原始 syscall/结果与断线观察必须分别留证。

实际实现前须逐项核定：最终许可前；许可后/签名前；签名后/receipt rename 前；rename 后/父同步前；父同步后/最终验证前；最终验证后/回复前。每个窗口分别记“原调用是否返回成功”“真实字节/同步是否观测”“是否仍可新签/发布”“是否满足既有历史重开合同”。不能由单一 `UNKNOWN` 标签抹去这些差别，或在缺原始观测时给历史重开资格补证。

### Q3：谁拥有最终 publisher

推荐由 D 原 receiver 内部完成最终写入/验证，以免让 S 的普通目录抽象冒充远端文件系统。S 最后的源保护核对通过经审 C 输入，D 仍须消费其原成功状态。新 helper 不对外暴露 caller proof/expected pin 参数。原 `publish_complete_backup` 的本地/fresh-lease 拒绝保持。

这是一条新增、待审的异机管理入口；不能仅称为重构。是否能在不改 v2 下共享原 writer、失败残留和完整验证合同，是实现前的明确审查问题。

## 7. 最小施工顺序与审批包

| 步骤 | 可实施的具体产物 | 进入条件/审批 |
|---|---|---|
| R0 | 目标事实、六项平台预检、root 自查回读、托管方案 | 已完成限定准备；不是正式域/安装验收 |
| R1 | 本 RFC 的 Q1/Q2/Q3 裁决、认证来源与字节访问方案独审 | 当前进行中；结论必须书面明确，不能把“可继续设计”当实现批准 |
| R2a | 无权限 framing/header/预算/语法顺序和纯拒绝测试的独立文件 | 本轮独审已限定允许；不接产品 crate、IO、key、安装状态、原 authority 或业务根，不算37条集成通过 |
| R2b | 受审绑定/认证/时序、原 actor/receiver 的最小权限差异 | 只有 R1 明确采纳 Q1/Q2/Q3、身份根和权限来源后才编码；不以 R2a 通过替代 |
| B0 | D 私有目录与 P-256 PKCS#8 key 现场生成，仅回传公钥 | 安装脚本及准确 hash、碰撞/失败保留策略独审并获 root 执行批准；尚未准备/批准/执行 |
| B1-INSTALL | 真实根、registry、trust、构建公钥 pin | 对真实源/目标/证据根、预算和身份作核对后，按准确字节/路径另审批，不预填假部署记录 |
| C | S 管理认证身份/信任及固定 receiver 调用权限、精确二进制 | 所有新增凭证、信任、sudoers/进程权限/必要服务均明确列清后批准；不以 B0 或 SSH 登录批准覆盖 |
| D1 | 新源/新目标正常整链与首次干净恢复 | 当前代码审查/回归及整链执行范围获批，资源预算已核实 |
| D2 | 源端不可访问后的第二次全新恢复 | 另批源隔离及恢复写入，保留源原件；目标端历史重开不重新首签 |
| R3 | 适用拒绝、旧版回归、原件独审与正式五门裁决 | 真实独立验收后才关闭 C4，不合并 main、不部署生产 |

目标只有约 2 GiB RAM；本轮不在其上套用既有 8 GiB 编译配置或并发重型恢复。构建与恢复资源另规划，容量须含包、原件、staging、失败残留、证据及恢复峰值，不按当前空闲空间直接判足够。

## 8. 证明义务及测试映射

沿用 37 条未执行矩阵的原 ID/含义/状态，不把六项平台预检加入该计数。实际实施应另外映射：认证连接但伪造 S actor、旧源 key/错误 build/新 lease、挑战/序号/方向/消息替换、十一处 D seam、S 最终选择与 D 撤销竞争、源/目标进程退出、部分帧/EOF/期限、最终回复丢失、同 ID/新 nonce 重放以及已真 complete 历史重开。

正向必须使用真实独立 D、真实托管 key 和经审新权限路径；不能用 mock signer、普通 SSH 登录、单机模型、测试专用正向 constructor 或纯状态机测试代替。测试应区分编译/列举/纯单测/平台预检/实际 syscall/跨机集成/恢复验收。

## 9. 本次允许和禁止的结论

本文可以关闭“没有具体设计审查输入”的准备缺口。当前独审结论只允许继续设计及 R2a；只有另行明确接纳 Q1/Q2/Q3、认证与权限来源，才可进入 R2b，不代表可以配置或启用。初次独审原稿 SHA-256 为 `a68586b5ace0fe7dcd17ee2f517e244b9153e9fdd36ef3e7ed670f8ac3c11e98`；后续文档精化不冒充该旧稿已获全量接纳。

到本文基线为止：未生成新 key；未安装 trust/registry/管理认证身份；未改变 SSH、sudoers 或服务权限；未开放新首签/发布；未产生正常 CompleteBackup/RecoveredInstance；Task9 五门、C4 和生产均未验收。原 Task8 结论、旧失败/HOLD、公共 v2 与现有拒绝路径保持。

后续已完成R2a限定准备：独立文件的格式/编译/20正文在Linux通过，源码及实际日志由不同子代理独核，原件摘要和层级限制见[当前检查点](selected-target-preparation.md)。这不关闭R1的Q1/Q2/Q3，不批准R2b或安装/启用；代码未导入产品，原七份源码pin与37条规格不变。
