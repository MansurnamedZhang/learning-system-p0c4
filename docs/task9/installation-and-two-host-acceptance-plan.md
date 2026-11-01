# Task9：真实安装与双机整链验收实施单

日期：2026-11-01。代码基线：`be147b3dbfd0cf4ecfc126dda4d21dfb3fc0d30c`，树 `675b764b27ca4bbe6f7485094e314c0999844e3f`。

**本单是已批准范围内的审查、准备与审批分界，不是安装授权或可直接执行的安装器。** 所有产品功能保持默认关闭。Task9、C4、首次真实 S、D Ready、持久 fence、CompleteBackup 和公开恢复资格均未验收。配套[准备状态表](installation-readiness.json)不能被产品加载为权限。

## 1. 当前已确认什么

- 现有首次 S/M6 默认关闭代码已发布；准确受测源码的 13 道普通 Linux 门通过。Backup 默认配置 450 passed / 106 ignored，offhost 配置 649 passed / 106 ignored；46 个不同新增测试体，跨配置重复执行不相加。DB 在该批仅作为依赖编译。源码索引 `6efcea40845eef4193a48ee62a7774ca3691b183b1784f525d1ad9cd42f9e48f`，结果 `95fd387f5e92a168f74b2ba1b7d29e5b2b3436312de60706e90f3cf9e9fc424d`。
- 用户指定两台独立主机：S 为既有 Linux 开发服务器，D 为用户管理的独立云主机及云盘。Windows 仅整理文本、元数据和 Git，不充当备份目标或产品测试机。具体地址、账号、主机指纹和私有路径留在私有运行单，不公开发布。
- 2026-11-01 01:30:47 UTC，沿用现有公钥登录完成 S 普通用户只读元数据检查：x86_64、24 个逻辑 CPU，内存约 30.7 GiB，home 所在文件系统可用 251,911,761,920B。固定 `/var/lib/knowweave-c4/b1` 返回权限拒绝，状态是 **UNKNOWN**；其他新根或 ELF 的普通用户不可见结果不充当完整安装清单。没有越权读取或修改。
- D 本轮使用现有主机信任、禁密码及禁新增凭证的单次非交互检查返回权限拒绝，没有取得新的现场状态。2026-10-26 的 Ubuntu 24.04/x86_64、2 CPU、约 2 GiB RAM、50 GiB 云盘及当时约 36.7 GiB 可用，仅为历史观察。既有 MySQL/Redis 等服务必须保留。
- 用户已选择 D 的 root 私有目录托管备份签名私钥，由用户保管；这是托管方案，不是已生成、已安装或已验收的事实。运维密码登录与未来受限传输身份分开，不索取或保存密码。

Task8 私有单机收尾及旧失败/HOLD 保持原结论。这里不重复 Task8，也不把普通检查改记为双机验收。

## 2. 安装前的准确阻碍

| 编号 | 当前缺口 | 关闭条件 |
|---|---|---|
| H1 | D 当前可达性、容量、文件系统、既有安装和非冲突路径未重新核实；S 的 root 私有安装状态未知 | 经获准的现场元数据清单取得时间、主机/存储身份、容量、既有文件/模式/摘要及冲突清单；不读取私钥内容，不绕过权限拒绝 |
| H2 | Capture 固定 12 挂载与新 Source mutation policy/fence 读取存在真实布局冲突 | C0 准确拓扑增量合同、代码与普通回归获批并通过；C 再验证真实 mountinfo/权限/held FD，不用父目录广泛 bind 绕过 |
| H3 | 真实托管公钥、ELF、policy、anchor、launch manifest 的现场 pin 尚未形成 | 按第 4 节无环顺序，在 B/C 授权后生成并封存真实清单；任何空值或测试向量禁止进入安装 |
| H4 | 没有已审 B1 policy/key/launcher/受限 forced-command 安装器；旧 source fixture issuer 不能承担该工作 | 准确安装工具及其差额先独审，安装清单逐项给出新增文件、父目录、所有者、模式、摘要、命令与退出/留存规则，再经 C 批准 |
| H5 | 旧 writer 排除、全期 global64、S5、真实 Root/PG/C11 和原 holder 结算缺现场证据 | C 的真实生命周期观察通过；列全产品 FD 与 runtime/SSH 必需 FD，证明旧自动入口/在途写者不存在或被精确隔离 |
| H6 | 尚无真实双机原调用成功、正常完成收据和 RA/RB 恢复证据 | D1/D2 的准确资源单和场景获批后，按第 6 节执行并独核原件 |

源码定位以该基线为准：[Capture/Root 启动合同](../../crates/learning-backup/src/offhost_b1/source_launch.rs)、[DB peer 进程观察](../../crates/learning-db/src/snapshot_broker/b1_peer/process.rs)、[固定安装读取](../../crates/learning-backup/src/offhost_b1/policy/installed.rs)、[source fence](../../crates/learning-backup/src/offhost_b1/source_fence.rs)。本轮不改变这些源码或原批准合同。

### C0 拓扑候选：需要补充审定，尚未实施

原 12 个 mount 的身份、模式和封闭检查保持不变。优先审查的最小候选为额外三个**准确只读** bind：固定 `source-policy.json`、固定 `launch-manifest.json`、固定 `/var/lib/knowweave-c4-source-fences`，形成独立版本的 15-mount 合同。Capture 不需要接收 S 的应用认证私钥、SSH transport 私钥或 known-hosts；这些留在 host publication S。

这需要协调 Root/Capture 与 DB peer 的实际进程观察、C11 合同/确认、no-follow 祖先目录、同 inode 锁及 registry 锁顺序；不能简单把 `12` 改成 `>=12`、跳过检查或绑定整个 `/var/lib/knowweave-c4/b1`。旧 12-mount profile 继续严格拒绝未知布局。准确变更文件与字段应另封补充合同；未获批前只准备方案，不编码或安装。

## 3. 最小审批分包

| 包 | 最小动作与对象 | 影响及退出条件 | 不包含 |
|---|---|---|---|
| A：现有准备范围 | 本地只读审查、普通 S 元数据检查、方案/填报结构与静态核验、已有 feature 分支及路线图同步 | 不改变运行权限；保留全部旧失败/HOLD；产品源码本轮不变 | 新凭证、root 安装、现场首签、恢复 |
| C0：准确补充编码合同 | H2 的精确拓扑/观察/C11 差额，及 H4 安装工具设计所需的准确代码面；先逐项独审再批准 | 仅默认关闭实现与原限定 Linux 检查；任何扩文件、字段或安全行为必须明确列出 | C 现场配置、密钥生成、权限安装、整链运行 |
| B：密钥与信任托管 | 仅封存清单中 S 应用 Ed25519、独立 SSH transport Ed25519、D 备份 P-256 三种用途；其公钥/host-key trust、registry 身份及托管根 | 原件仅在所属主机 root 私有目录；公开只导出公钥指纹/摘要。意外存在或不符即 HOLD，不覆盖/轮换旧材料 | authorized_keys/sshd/sudoers/launcher、PG 授权、首签 |
| C：运行身份与固定安装 | C-prepare 仅准确新 namespace/registry/fence/session 根及身份实例化；C-install 再批准实际 ELF/父 launcher、private Unix PG owner/socket、policy/anchor/manifest、RO mount、旧 writer 排除与受限 SSH/必要精确安全规则 | 两个阶段分别有准确清单及 guard；C-prepare 不启用运行入口。只在新隔离资源逐项 CAS/no-follow/禁止覆盖，接受永久 fence 可用性代价后才可启用 | 任意 sudo/root shell、业务目录迁移、生产变更、源断联、恢复 |
| D1：正常整链与 RA | 固定新 UUID/数据库/卷/子网/预算的真实跨机首发、正常 CompleteBackup、全新 RA；已具名适用负例及 S2 后续中断 | 精确 ID 停机留卷；S1 fence 永久保留。RA 停机核验后再进入 RB；失败不自动重放 | 源整体断网/关机、源数据删除、已有业务恢复覆盖 |
| D2：源不可访问与 RB | 只隔离已批准测试源的 DB/registry/assets/pins 访问面；保留原件，全新 RB 仅使用 D 包/证据/trust | 核实零源端读取，恢复后按相同完整标准验证；按精确 ID 收尾留证 | 整台主机断联、防火墙广改、破坏性恢复、fence 解除 |

**当前只执行 A。** C0/B/C/D1/D2 不是本次“继续准备”自动授予的权限。B 可先批准准确托管根、真实材料与有限 public-pin 导出；C-prepare 可先批准准确新根/身份的有限创建，供实际 root binding 与构建使用。取得实际 ELF/policy/anchor/命令后，再填妥并批准 C-install。任一阶段都不能提前启动下一阶段；不以尚不存在的 hash 一次性批准无限安装。通过后 D1 与 D2 可按已填妥批次分别集中审批，无需逐条重复确认命令。

现场元数据需由现有运维登录在该主机完成。若需要 root 私有元数据，仅在相应精确只读清单获准后读取模式/所有者/大小/摘要；不从聊天、环境、命令参数或临时文件索取密码。缺少既有非交互认证时保持待现场核实，不新建登录密钥绕过。

## 4. 固定材料与无环安装顺序

下列是运行时**固定目标路径**，不是允许调用者任意指定路径的安装参数；实际宿主路径、namespace 和 mounts 仍须 C 实例化。单个 policy/key/trust 上限、目录身份、单 link、canonical JSON 与 no-follow 检查继续原合同。

| 位置/材料 | 用途与最低边界 |
|---|---|
| S `.../b1/source-auth.pk8` | 严格 PKCS#8 v2 应用 Ed25519；仅内部 SourceAuth/FinalDecision 签名，不提供任意字节签名入口 |
| S `.../b1/transport-key`、`known-hosts` | 独立受限 SSH 凭证及已核主机 pin；禁 PTY/agent/转发、禁任意命令与环境覆盖 |
| S `/var/lib/knowweave-c4/b1/source-policy.json` | 固定 group/registry/source-control/build/PG/transport 及 D policy 绑定；host/storage 由固定 verifier trust 绑定，不冒称 policy 有这些字段 |
| S `/var/lib/knowweave-c4-source-fences` | 独立固定 anchor/lock/operations/staging；v1 无 close/TTL/GC；损坏、部分、额外叶或同步不明均阻断后续相关变更 |
| D `.../b1/destination-policy.json`、`/var/lib/knowweave-c4/verifier/destination.pk8` 及 `trust.json` | D 独立 root custody；现 v2 备份 P-256 key（65B未压缩SEC1公钥）与认证/transport 用途分开；不把 key 内容导回 S |
| D `/var/lib/knowweave-c4-b1-sessions` | 固定 anchor/lock/operations/staging；admission 残留只禁止重放，不从磁盘恢复发布权限 |
| 各侧 `.../b1/launch-manifest.json` 与固定 ELF | 记录实际 ELF/父 launcher、policy/anchor 等摘要；D receiver 零参数、固定路径、root 全 UID/GID 与真实父 ELF 链同时成立 |
| S 三角色实际路径 | PublicationSource `/opt/knowweave-c4/bin/knowweave-c4-b1-source`，Capture `/opt/knowweave-c4/b1-capture/knowweave-c4-b1-capture`，Source RootLauncher `/opt/knowweave-c4/bin/knowweave-c4-b1-source-launcher`；不是 D 父 launcher 的同义词 |
| D receiver 与 S SSH ELF | `/usr/local/libexec/knowweave-c4-b1-receiver` 为 root:root、单 link、恰 0500；`/usr/bin/ssh` 为 root:root、单 link、恰 0755。真实 ELF/文件 stamp 与原 FD 全程核对，不能只对同名路径作一次 stat |
| Source 原 12 件证据、registry/pins、D package/evidence/trust | 保持旧严格 roster/公共 v2 格式；新 session/fence 不塞入旧 roster，也不污染 D-only reader |

无环顺序：

1. 完成 H1/C0/安装工具审查；封妥 B 和 C-prepare 的准确对象与最小动作并分别获批。此时只预留 S1/S2、V1/V2 目的端 view、RA/RB 的互不复用 ID/资源，不以未来 ELF/hash 未定为由提前批准 C-install。
2. 由所属主机 root 在新私有目录生成/加载获准托管 key，只导出 public pin。S auth 与 transport 实际材料必须不同；D 备份 key 不充当握手 oracle。
3. 在已获批 C-prepare 的范围内取得真实 root/registry/fence/session 身份；只在 S 既有固定 builder 编译 role/public-pin/source-control 固定的 S/D ELF，D 不编译。封存源码、feature、工具链、build-input 和 ELF/父 launcher 全字节摘要；测试向量或占位 pin 不能替代真实构建输入。真实 public-pin 构建须有相应限定回归/独审，既有 ordinary13 不自动给新 ELF 信用。
4. 准备含真实 registry/root identities 与 launch_id 的 anchor，再准备 D policy、包含 D policy 摘要的 S policy；最后准备引用真实 ELF/policy/anchor 的 launch manifest。policy 不反向引用包含自身的 manifest hash。先在私有封存工件中审定这些待装字节，C-install 批准前不安装或启用。
5. 准确 C-install 清单独审、批准后，以已审安装工具独占安装新文件并回读。准确 owner/mode 与 no-follow 祖先规则逐文件取自实际代码合同；不靠 `chmod -R`、目录重用或覆盖实现。无 C0 合同不得添加挂载。Source 的 `destination_policy_sha256` 没有作为 wire 字段回传；C 必须以 D 实装原字节回读、固定启动链与管理写面排除，闭合 S 期望 pin 和 D 实际使用 policy，不把 HelloD 的 receiver ELF hash 冒认成 policy hash。
6. 独立核验受限 transport → 受审 D 端固定启动链 → receiver 的实际链；D 内核父 ELF 及全部 UID/GID 必须和 D launch manifest 相符。另行核验 S RootLauncher/PG/C11 链，两者不混用。普通管理 shell、临时 sudo 或 authorized_keys 的 `command=` 单项不等于合格链；转发/TTY/agent/环境也须实际拒绝。
7. 完成 C 的 PG/Root/C11、mount、原 holder 生命周期和全期 FD 检查，再允许 D1。任何未确定字段、权限不符、额外写者、旧 ELF 或残留状态都 HOLD，不降级 legacy 或自动再安装。

## 5. 运行单必须填完的值

审批前将[状态表](installation-readiness.json)中的 null 与 UNKNOWN 填为**独立核对的实际资料**，生成新版本并封存摘要；不得把本准备表直接改成运行权限：

- 两侧地址/账号/运维归属、主机标识、磁盘/文件系统及独立故障域资料；同一机房/供电/云账号风险如实说明，不把“异机”夸大成地域或账号灾备。
- S1、S2、V1/V2 隔离 view、RA、RB 的 UUID、精确根路径/inode、registry/group/build/generation、PG 数据库/owner/socket、容器和网络不可变 ID、未占用子网。
- 每个待装文件与父目录的用途、来源、字节数、摘要、公钥 pin、owner/mode、祖先/ACL/单 link、允许的唯一命令/argv、环境与 mountinfo；与现有安装冲突则停止。
- 固定父 ELF/真实 exec 链，旧 writer/自动入口/在途持有者清单及无写入面证明；fence/session 注册身份和所有相关管理入口检查。
- 单 case 资源、总磁盘留存估算、敏感证据位置、原始日志限额、故障注入点、自然退出/取消/结算及精确 ID 停机方案。D 历史 2 GiB 容量不作为现在足够的保证。

原 profile 上限保持：package 256 MiB，证据传输 96 MiB，双向 wire 512 MiB/8192 帧，512 文件/一个活动流，产品 FD ≤64；必要 runtime/SSH FD 另列实数，不把产品 64 误报为全进程 64。原累计扫描 512 MiB/100000 entries；handshake 60s、idle 30s、原 S 总上限 1800s 且不延长更早 Root/capture deadline，settle 30s；不能确认结算就 HOLD。fence 256 operations；D session 256 operations/1536 entries/32 MiB metadata。不得自动抬高 cap。

D 候选 manager 512 MiB/1 CPU、单个 RA 或 RB PG 384 MiB/1 CPU、另预留 256 MiB 余量须 B0 重核，属于待批资源配置，非已测 RSS。RA/RB 顺序运行；磁盘预算需计 S2、失败卷和原件长期留存，不因不足清理旧失败或停止 MySQL/Redis。不能满足时停止，不扩大机器资源或影响别的服务。

## 6. 双机验收顺序与通过证据

以下使用真实独立 S/D 与产品正常链路，业务数据可为准确、完整、可核对的实验数据；单机/临时替代 signer、测试 authority、磁盘残留或固定返回值不能代替真实安装与原操作。

| 顺序 | 场景 | 必须取得的原始证据与判据 |
|---|---|---|
| 1 | C 准入检查 | 实际安装回读、独占 key/trust 使用权、真实父 ELF/UID、RO mountinfo、旧 writer 排除、global64/S5 与 Root/PG/C11 的同原件全期观察；尚不首签 |
| 2 | D1 正常首发 | fresh capture → 原 Source 会话/锁/lease/累计预算/cut → auth/wire → 原 D 11 步及副证明 → Ready → S 最终再核及持久 fence → 原 D Pending→Consuming→签名/同步/禁止覆盖发布/全验→Committed → Ack/End/真实 EOF/自然 exit0；完整原 receipt 与调用结果绑定 |
| 3 | RA 正常恢复 | 用 D1 正常 CompleteBackup，在全新 RA 走原恢复引擎；完整表/资产/引用/角色/作业归一化、旧租约失效、快照效果、ReadingCopy 实际字节均核验；结束精确 ID 停机留卷 |
| 4 | 规定负例与后续中断 | 六窗口、错误 key/trust/目的/故障域/根身份、缺失原件、损坏 dump 等按原分层清单；已消费后的迟到 S 取消不撤回，D 本地失败仍终止。以独立 S2 和预准入 V2 目的端 view 测后续传输中断，前后逐字验证 D1 完整 package/proof/receipt 与原 reader 仍可读 |
| 5 | D2 源不可访问 + RB | 仅让已批准实验 S1 的 DB/registry/assets/pins 对恢复进程不可访问，原件保持；fresh RB 只能读取 D 的包/证据/trust。证明源端访问为零，不能读取 S fence 或调用 S 参与历史校验；执行与 RA 同等完整校验 |
| 6 | 收尾与独审 | 每案原始 argv/环境 allowlist、退出码、stdout/stderr、完整资源身份、源/包/receipt 字节、停机留存及 pending 状态独核；先审定最终改动的适用性，再具名列出需补验的完整回归、四旧版升级与代码检查；未受影响 Task8 历史不无差别重跑或重计，最后回到 Task9 五门逐项签结论 |

成功后 S1 fence 仍永久保留，可能阻止该源后续备份及相关管理变更，并可能影响同 registry 的其他组。**不能用 S1 registry 重做 PRIOR_COMPLETE 第二个 ID。** S2 必须有预先获准的独立 namespace/registry/源根/profile；V2 是接纳 S2 的独立目的端 view/实例，不能临时 caller 覆盖或覆写 D1 trust。S1 fence、D1 完整材料和原 reader 语境都保留。S2/V2 没有前置准入就不运行该案。

每个窗口和负例都使用事先具名批准的 fresh 原操作、ID、registry/源根及目的端 view；出现 fence 或 D admission 残留即留存，禁止重放窗口、复用成功 S1 registry 或临时更改 policy。V1/V2 只是目的端隔离 view 名称，不表示审批包 D2（源不可访问/RB）已获准。

六个窗口逐案分账：BEFORE_PERMISSION、BEFORE_SIGNATURE、BEFORE_RENAME、BEFORE_PARENT_SYNC、BEFORE_VERIFY、BEFORE_REPLY。前三者不产生正常完成收据；后几窗可能留下完整可读的签字节，但“后来 reader 接受”“原发布自然成功”“可再首签”是三件事。未知/答复丢失留 HOLD，不自动重试，不从磁盘恢复签发 authority，不为便于重跑释放 fence。

旧 [37 项准备矩阵](b1-preparation-test-matrix.json)和 [53 项分层义务](r2b-validation-plan.json)保持原字节及历史状态。现场运行单逐项映射到精确 case/原件/判据，不能把 53 称为 53 个已执行正文，也不能用普通 46 新测试抵扣 C/D。原矩阵 SHA-256 `4eea0c2b2d4fd6a101db1b117a7553fb05f61f3b00af66f555606f4c168e4bcd`，分层计划 SHA-256 `b934d2ef262a8c5e1c8762af515e944fc5c244a2c0c5600a8428ea0cc88111b6`。

## 7. 停止、留存与回退边界

安装前冲突即停，既有文件不覆盖。新建资源按精确 ID 停止，保留 root 私有源码、日志、卷、残留及已装但不可继续的材料；不执行 prune、广删、自动回放或“恢复到可签状态”。key/trust/安全规则的撤销也须针对准确新增项另审，不通过恢复旧同名文件掩盖安装结果。

任何 fence 写入或 D Consuming 后，不能承诺无副作用回滚。原操作结束不明保持 HOLD，旧 signer/入口不得被重新放行。单次有效历史读取不能补证原调用成功；异常解除 fence 或已装权限属于独立未来审批。source 隔离只针对实验资源的访问面，整个 S 主机、业务 PG、已有服务、C3、旧批次与生产均不动。

只有 Task9 五门真实闭合并独审、且 Task8 历史规定范围仍成立，才可关闭 C4 父里程碑并记录限定 `P0_C4_VERIFIED/NOT_PRODUCTION`。本轮 feature 分支发布与路线图更新不关闭 Task9，不合并 main，不部署生产。

规范来源：[Task9 五门](../superpowers/plans/2026-10-04-p0c4-completion.md)、[Appendix B1](../superpowers/specs/2026-10-04-p0c4-completion-design.md)、[已批 R2b 计划](../superpowers/plans/2026-10-26-task9-r2b-implementation.md)、[wire/安装形状](r2b-wire-profile.json)、[当前代码与普通检查记录](r2b-implementation-progress.md)。本单不修改上述规范语义或批准范围。
