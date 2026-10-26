# Task9 B1：已采纳异机时序与历史准备记录

2026-10-26。**SEMANTICS_ADOPTED_SPEC_FINALIZATION；规范定稿，独审结果见采纳记录。用户已采纳封存提案的时序语义，仅准定稿和独审；NOT_IMPLEMENTED，R2b/安装/配置/启用/源隔离/恢复仍未批准，Task9/C4 仍开放。**

本稿产生于尚未选定独立目标的历史准备阶段；当前目标见[已选目标记录](../../task9/selected-target-preparation.md)。用户答复“采纳方案，仅定稿规范（推荐）”及提案/独审摘要见[采纳记录](../../task9/b1-final-decision-adoption.md)。现行语义以[完成规范 Appendix B/B1.1–B1.6](2026-10-04-p0c4-completion-design.md)的Task9窄异机例外为准；[RFC](../../task9/b1-offhost-admission-rfc.md)与[原提案](../../task9/b1-final-decision-contract-proposal.md)保留各自冻结时点，不回写成已实现或已授权。本文同步现行规则并保留原草案/历史执行层级，不更改37条规格或其未执行状态。

以下产品来源及本地检查段落是历史准备记录，“权限合同保持原样”等措辞指当时检查点，不能覆盖本次仅语义采纳；运行时权限和代码当前仍未改变。

产品 Rust 来源为 `679b11c3f280546f8d6c20ab15cf7ef59fae3c1a`；原Rust19检查点的准备发布基线为 `c1f170a5d83351eb8ff1d66568b812b5770684c6`；本轮离线工具的发布准备基线为 `2c507ea4b0e6134b66a9e06a9970e939f50a7422`。Linux 业务实现、权限合同、迁移、v2 包／statement／witness／receipt 与 Task8 结论保持原样。本地候选累计追加十项 cfg(test) 拒绝回归及测试专用helper/适配器，并对一个非Linux“不支持”分支作同一 Err 的尾表达式修正；不能说这两个源码文件与旧ZIP逐字相同。[前置状态](../../p0c4-task9-readiness.md)和[机器可读测试规格清单](../../task9/b1-preparation-test-matrix.json)均不产生产品可消费的授权。

## 四个职责与现有接入位置

这些是四项责任，不是四个已新增 API、服务或 capability 类型。

| 职责 | 沿用现有位置与拥有者 | 新集成必须履行的义务 |
|---|---|---|
| S：源端保护原操作 | `ProtectionSnapshot::require_same_operation`、原 held registry lease、锁、scan budget 和 enrolled source roots | 原本地要求保持；Task9异机仅在原S最终选择前持原上下文完成精确重核与持久负fence，再派生一次固定最终消息。负fence保持保护连续性，不证明S持续活性；fresh lease/复制字段不等于原操作。 |
| D：目的端提交与签名原操作 | `destination::attempt::publish` 的原 `OperationAuthority`、证据根、完整字节重读、`VerifiedDestination::sign_statement` | 十一个证据步骤和 outcome 全部成功后才登记原内存成功状态；私钥留在目的端；签名与撤销保持可证明的串行裁决。 |
| C：经审的跨边界衔接 | 时序语义已采纳，适配层尚未实现或获R2b施工批准 | 认证确切原manager/构建/活操作及同一会话新挑战，绑定backup/attempt/proof/manifest/source-control、registry代际和各自根；不是SSH身份、自报build或通用signer。异机不共享原Arc，不从旧决定重建权限。 |
| P：新收据最终消费 | 原验证链保持，新路径待另审实现 | 原D十一阶段及outcome自然成功后，同一本地裁决Pending→Consuming（许可点）→Committed（全部自然完成）或Failed，不可回退/重用。旧finalize_use不是one-shot；fresh-lease publisher继续拒绝。 |

现有 `require_same_operation`、新 lease 的空成功表以及最终原操作检查保留。不能通过去掉 `Arc::ptr_eq`、公开 capability 构造／反序列化、增加 `allow_local/force/authorized=true`、给 CLI 接受任意 token，或从磁盘 terminal 补建成功表来接通流程。

可行的实现方案以后应复用既有证据格式、canonical/parser、seam writer、签名消费和全量恢复引擎。即使新入口只在 crate 内可见，只要使原先拒绝的新首签／新发布可达，也必须先独审其 authority 来源，不能称为无权限变化的重构。

## 目的端原状态机的保留点

现有 `destination/attempt.rs` 的 `STEPS` 固定为以下顺序；本地清单检查会对实际源码逐项对照。

1. Admit
2. Prepare
3. Stage
4. BeforeRename
5. Rename
6. ParentSync
7. FinalReadback
8. TerminalWrite
9. TerminalSync
10. TerminalReadback
11. FinalRecheck

`outcome()` 还必须返回与本次 backup 身份一致的结果。只有这些步骤全部自然成功，并在原操作的串行提交位置再次确认未失效，才能记录成功状态。可见文件、完整 checksum、一次后续 readback/resync 都不能替代这个条件。

每一个 seam 都要有失败规格：保留实际已产生的残留，不授新签名／新发布，本次失败调用不自动重试。Admit 持久准入前拒绝可能没有磁盘残留，不能制造 attempt 或虚构磁盘 ID 已消费；存在持久准入／残留时，同 backup_id 即使换新 attempt_id 也必须拒绝。已有无关备份不变。此处不增加自动清理、repair 或重试器。

## 认证、授权、签名与发布分别证明什么

| 观察 | 可以证明 | 不能单独证明 |
|---|---|---|
| 已认证连接／消息 | 经审通道内的对端身份、消息完整性和当次请求关联 | D 的十一阶段已成功、成功权限仍有效，或 P 获准新发布 |
| 原 D 操作成功状态 | 该原操作在规定成功登记点满足提交条件 | 另一进程、新 lease 或另一个 source 操作继承权限 |
| 有效 v2 witness | 签名覆盖的原 v2 statement 内容 | S 的当前保护仍有效、跨操作准入已完成、P 可写新收据 |
| 已真正完成并验证的 signed CompleteBackup | 既有目的端重开／恢复合同中的历史授权 | 新 same-ID 首签、重发收据、绕过当前源保护或产生新的成功操作 |

跨边界绑定至少要覆盖现有 backup、attempt、proof、manifest、source-control、source/build、registry generation、enrolled roots 和原操作拥有关系。这里列出的是核对义务，不是新的 wire 字段或可序列化 permit；公开 v2 格式不变。

## 故障与最终消费的裁决

- **许可前的本地串行胜负：** 旧本地约束保持。Task9异机仅在原D的Pending→Consuming之前，已观察并获该裁决的拒绝/取消/EOF/错误/超时使Failed并禁止消费；不能靠两端时钟或所谓共享mutex判定先后。S最后消息后死亡而D未观察的窗口，按已采纳窄例外处理，不再声称S活到每次最终物理写入。
- **本端看不清对端结果：** 保持 UNKNOWN/HOLD；不自动重连后补签、重建 authority、重发同 ID 或刷新基线。只读核对不能变成新写入资格。
- **一次消费与物理完成分开：** 原D在成功authority、本次可信原S完整决定/新挑战、保护连续性及预算期限均成立时进入Consuming；撤销/消费同锁或等价单actor排序。迟到远端退出/取消不回夺已获许可；本地错误/期限/unwind仍Failed并关闭新签名/发布，禁止回退或重试。旧finalize_use仅require+串行使用、不删除成功项，新一次消费状态尚待实现与独审。
- **Committed和回复：** 只有内部签名、receipt排他写入/同步/no-replace/父同步及完整验证全部自然成功才Committed并可输出witness/成功对象；签名不能先于最终许可。Committed后的回复丢失仍使S保持HOLD，不追溯否定D已完成结果，不授未来新操作。D原进程丢失不由新进程续做。
- **三账及六窗口：** 原调用自然成功、后来目的端历史只读接受、新首签/新发布资格分别记录。保留现v2 reader可能接受合法完整已签残留的行为；后来open及其sync不能追补原P的sync/verify/reply成功，也不计失败调用的正常首发正例。许可前、签名前、rename前、parent-sync前、最终验证前、回复前六窗口按Appendix B1.5分别举证。
- **历史恢复：** 已签完整包不依赖在线 source registry/DB/asset/pin；不比较 captured 历史 source inode 与目的端副本 inode。旧 bare-v2 的低层读取不被扩大为新的 full-recovery 资格。

## 已采纳语义的保护与部署边界（未实现）

原S在原锁、原累计预算和对象下先独占发布/同步/持有FD回读负fence并精确重核，再选择SOURCE_FINAL_DECISION。fence只能阻止破坏snapshot records/counts/generation/root身份的后继管理变更，不能授首签/新发布或重建原操作。所有writer、旧构建、重新登记/升级/恢复入口都须受门控；全局registry generation可能导致跨group长期阻塞，仅挡GC不足。新增namespace/记录/读取/保留须有固定上限并累计计费，UNKNOWN不按TTL自动解除或重试；管理员及所有相关写入方可信是假设，不声称抵抗任意root改写。

仅仍活原S在原lease及持有fence对象下收到本次完整可信D成功结果、且所需结果/关闭证据已独核时，才可能追加独占同步回读的关闭记录；保留历史，不删除fence。该窄资格仅限仍有效、从未先行失效或进入失败/UNKNOWN/HOLD的原S正常核验本次成功，不能靠迟到ACK、历史读取或后来receipt读回恢复；普通期限内等待原回复不等于复活资格。该条件和实际协议仍待实现证明。关闭失败/未知继续保留约束；新进程不能从磁盘或历史receipt重建关闭资格，异常解除留给未来另审管理流程。

管理身份与自动transport身份分离；系统Docker socket管理权不等于受限传输权。原manager的root管理固定启动/二进制/实例/key使用约束须独审，root0600或自报build不足。新的源认证key不同于D备份签名key；受限非交互principal、固定命令、用户/公钥/forced-command/sudoers及key/trust/registry/fence安装均需具体审批，不由语义采纳批准。

下列测试、工具和收尾段落保持历史原件层级，既有“本轮”等时点用语不改写成当前执行；37条规格保持原内容与未执行状态。

## 已准备的测试规格

JSON 清单包括十一 seam、六种原操作边界、八种绑定替换、八种结果／竞争以及四组真实独立正向链。**37条集成规格全部标记 NOT_EXECUTED_AS_TASK9_INTEGRATION_CASE；未执行或模拟独立故障域链。** 第一检查点原有测试只作覆盖索引；最新检查点已在Windows重跑整个attempt模块。第一检查点另复用现有内存 Store 夹具执行五项新增 Windows 纯拒绝单测，覆盖 Admit/Prepare/Stage/Rename/TerminalWrite 的 pre-action 错误；这是单测层，不是真实文件系统 seam、Linux/PG 或 Task9 集成验收。

本地结构检查应验证：case ID 唯一；十一个 seam 无缺项；现有覆盖引用的函数确在所指源码；源码摘要对应本稿基线；真实首签／新发布／正常恢复／源不可访问四个正向组必须要求真实独立目标。原 every_seam 函数只精确引用六个故障位置，不能把一个函数的存在当成十一处覆盖。五个新增测试与六个历史位置分层记账；结构检查退出 0 只表示清单和映射一致，不能记为三十七项测试通过。

历史首轮pre-action检查点（当时5项；本轮以随后module19原件为准）：这五个单测复用现有内部 Store，未新增模型、独立 key、签名器或权限入口。当时Windows Rust1.97 offline/locked 选中五项全部通过，0 failed/ignored；单文件 rustfmt 与包级 lib/tests 严格 Clippy 通过，均不算 Linux 验收。旧 Windows Clippy 的既有 needless_return 失败原件保留，后续只修非Linux等价尾表达式后重新检查。

未来权限集成的拒绝／撤销回归仍需对应最小实现审查后复用现有内部 fixture；不得用 stub 返回首签成功、伪造独立 key 或新 constructor 让真实正向组变绿。适用真实回归另用全新资源、源码与 binary 证据。

## 最终结果边界的本地回归（后续准备检查点）

新增五个cfg(test) body：四种outcome拒绝／撤销／unwind，加一个八字段自洽identity替换组。沿用原Store和原publish/finalize_use路径；outcome适配器仅属于测试，不给生产入口增加分支、公共构造器或传入callback。八个字段变体不等于八个独立测试；仅证明现有CommitIdentity绑定，不声称覆盖尚未实现的source/build、registry代际或跨域认证。

实际Windows模块19/19：既有14项重跑＋新5项，0失败／忽略；新旧任务前缀共10个unique body。原六位置seam循环这次也在Windows执行，但不改写原Linux历史，也不产生物理FS执行信用。原14个测试和运行时前缀逐字不变。outcome失败后的“可解析terminal”与“原成功权限”仍分开；同ID重试与最终消费拒绝、旧原件保留分别断言。格式／包级lib-tests严格Clippy通过；没有独立域、签名、网络或正常恢复用例执行。

37条集成规格的ID、内容和NOT_EXECUTED状态全部保留；新增结果记录在清单的local_outcome_and_binding_units中，产品不读取它。

## 可复用准备工具（结构层，不是权限接口）

新增标准库离线检查器及38项Windows纯Python工具回归，运行方法与边界见[准备状态](../../p0c4-task9-readiness.md)。它固定读取公开检出中的matrix和七个源码文件，检查ID／类型／摘要、当前11步骤文本与引用、37条未执行状态及历史计数范围。严格拒绝重复键、任何小数／指数与非finite数、超过20位整数、未知嵌套字段、伪fresh-domain或执行信用；有效输入每件上限128KiB，每读最多额外一字节探测溢出。检查器没有网络、子进程、任意路径覆盖、环境配置、key访问或写文件；38项工具回归另使用自动清理的私有临时合成文件，不修改产品数据。

这是可审的结构一致性工具，不执行Rust语义证明，不验证独立故障域、现有安装、独立可信基线或历史执行原件；不属于正式Task9验收或生产admission。报告固定无产品消费、无执行信用、无Task9接受。本轮38只计工具测试，历史Windows Rust19不重跑不累计。37条集成case内容／状态、原权限和v2均不变；只修正清单中旧“五项”overlay标签为累计十项。初版及其已发现缺口与封存失败保持原件，最终38项新原件另列。

## 当前本地检查的收尾记录

新[本地准备收尾表](../../task9/local-preparation-closeout.md)只关闭所列准备项。实际从27fc规范Git对象导出655文件、核654MF，原有38工具body重跑及两cwd／拒参数CLI通过，前后完整文件集合和字节不变；不新增用例，不授Task9资格。后续仅文档同步保持十个工具相关输入字节，不能把此次执行重标为未来提交。Rust、工具、37集成case与原权限／v2均未修改；四项实施输入仍待真实目标事实。

## 语义采纳后仍须具体化的四项实施输入

1. **目标与字节访问方式：** 独立运行方和故障域、目标根命名、源／目的端如何读取数据、现有 `BackupDir` 所需目录／同步／no-replace／身份语义。NAS 或对象存储不能未经核实直接当作等价 POSIX 根。
2. **托管与管理通道：** 谁持有独立私钥、现有安装 trust/registry、构建公钥 pin、双方实际调用身份及允许的窄操作。此稿不选定新认证机制，也不配置持久凭证或服务。
3. **原身份与准入实现：** 已采纳最终决定时序，但原manager可信独占身份、D原成功与新消费状态、全writer负fence和六窗口的真实实现/来源证明仍待具体化，不能用旧Arc跨机传递或已解析帧替代。
4. **结果原件与历史读取：** 按已采纳三账和六窗口设计实际观测；现v2合法完整已签残留可历史读取，不要求全部失败残留永拒，也不能靠later resync/readback补造原调用自然成功。

这四项是后续具体设计和实现审批的输入。用户此次仅准语义的规范定稿和独审，不批准R2b施工；规范新稿独审也不自动授权实现。R2b需另行具体批准，key/trust/registry/fence/source-auth/权限安装及源隔离/恢复各需对应B/C/D执行审批。

## 当前交付与未完成项

当前交付新增用户语义采纳记录和Appendix B/B1窄异机规则的规范定稿，尚待独审；原草案/历史源码摘要、37条未执行规格、结构检查及分层Windows纯单测记录保留。没有新增运行时authority、signer、服务、key、公共构造器或单机跨机替身，没有改变Rust权限或重跑Task8；历史两个源码叶的测试/风格差异不冒充旧Source679字节。此次无新测试，不改37条case，Task9五门、C4、正常CompleteBackup/RecoveredInstance和生产保持开放。
