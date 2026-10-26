# Task9 B1：最小集成与失败测试准备草案

2026-10-26。**仅供设计审阅：未采纳为新权限合同、未实现或启用新首签／发布权限入口，Task9/C4 仍开放。**

用户确认暂时没有独立目标，继续本地准备。依据仍为[原完成规范 Appendix B/B1](2026-10-04-p0c4-completion-design.md)及[原施工单 Task9](../plans/2026-10-04-p0c4-completion.md)。本稿细化实施义务和测试规格，不替换原规范，不选择认证设施、生成密钥或制造单机独立验收。

产品 Rust 来源为 `679b11c3f280546f8d6c20ab15cf7ef59fae3c1a`；本轮准备发布基线为 `c1f170a5d83351eb8ff1d66568b812b5770684c6`。Linux 业务实现、权限合同、迁移、v2 包／statement／witness／receipt 与 Task8 结论保持原样。本地候选累计追加十项 cfg(test) 拒绝回归及测试专用helper/适配器，并对一个非Linux“不支持”分支作同一 Err 的尾表达式修正；不能说这两个源码文件与旧ZIP逐字相同。[前置状态](../../p0c4-task9-readiness.md)和[机器可读测试规格清单](../../task9/b1-preparation-test-matrix.json)均不产生产品可消费的授权。

## 四个职责与现有接入位置

这些是四项责任，不是四个已新增 API、服务或 capability 类型。

| 职责 | 沿用现有位置与拥有者 | 新集成必须履行的义务 |
|---|---|---|
| S：源端保护原操作 | `ProtectionSnapshot::require_same_operation`、原 held registry lease、锁、scan budget 和 enrolled source roots | 保留原操作和当前源保护；开始捕获及最终发布前重核。另开 lease、复制 hash/快照或换成同 ID 不等于原操作。 |
| D：目的端提交与签名原操作 | `destination::attempt::publish` 的原 `OperationAuthority`、证据根、完整字节重读、`VerifiedDestination::sign_statement` | 十一个证据步骤和 outcome 全部成功后才登记原内存成功状态；私钥留在目的端；签名与撤销保持可证明的串行裁决。 |
| C：经审的跨边界衔接 | 当前尚无被采纳的异机准入适配层 | 将仍活着的 S 与原已成功且未撤销的 D 绑定到同一次请求、backup/attempt/proof/manifest、source/build 和各自根身份。认证通道是必要条件，不能替代成功权限；异机不能共享原 Arc。 |
| P：新收据最终消费 | `publish_complete_backup` 与 `check_publication_proof` 的原验证链 | 明确新发布授权的真正来源、拥有者、撤销及唯一最终消费点；当前 publisher 自取新 lease 的路径继续默认拒绝，不把签名消息当成发布许可。 |

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

- **最终提交前的拒绝、错误、取消、断线、超时或对端退出：** 不再授予新首签／新发布，撤销仍可撤销的原操作权限并保留原件。不能因断线断言远端必未提交、数据库必已回滚或重试安全。
- **本端看不清对端结果：** 保持 UNKNOWN/HOLD；不自动重连后补签、重建 authority、重发同 ID 或刷新基线。只读核对不能变成新写入资格。
- **最终消费竞争：** 当前同进程 `finalize_use` 与撤销共用 mutex。跨进程不能假装仍有同一 mutex；方案必须给出可审的排序和成功见证，不能仅写“双方验证后原子发布”。
- **已真正提交之后的迟到断线／撤销：** 不追溯否定此前真实已完成的 CompleteBackup；只关闭未来新操作。原始证据与目的端 only 重开规则保留。
- **已签名但新 receipt 写入结果不明：** 不凭 unsigned terminal、单张 witness 或可见文件推断正常发布成功。新发布资格与既有历史 receipt 的只读重开要分别裁决；具体通信／持久化故障窗口仍须在实现方案中证明。
- **历史恢复：** 已签完整包不依赖在线 source registry/DB/asset/pin；不比较 captured 历史 source inode 与目的端副本 inode。旧 bare-v2 的低层读取不被扩大为新的 full-recovery 资格。

## 已准备的测试规格

JSON 清单包括十一 seam、六种原操作边界、八种绑定替换、八种结果／竞争以及四组真实独立正向链。**37条集成规格全部标记 NOT_EXECUTED_AS_TASK9_INTEGRATION_CASE；未执行或模拟独立故障域链。** 第一检查点原有测试只作覆盖索引；最新检查点已在Windows重跑整个attempt模块。第一检查点另复用现有内存 Store 夹具执行五项新增 Windows 纯拒绝单测，覆盖 Admit/Prepare/Stage/Rename/TerminalWrite 的 pre-action 错误；这是单测层，不是真实文件系统 seam、Linux/PG 或 Task9 集成验收。

本地结构检查应验证：case ID 唯一；十一个 seam 无缺项；现有覆盖引用的函数确在所指源码；源码摘要对应本稿基线；真实首签／新发布／正常恢复／源不可访问四个正向组必须要求真实独立目标。原 every_seam 函数只精确引用六个故障位置，不能把一个函数的存在当成十一处覆盖。五个新增测试与六个历史位置分层记账；结构检查退出 0 只表示清单和映射一致，不能记为三十七项测试通过。

历史首轮pre-action检查点（当时5项；本轮以随后module19原件为准）：这五个单测复用现有内部 Store，未新增模型、独立 key、签名器或权限入口。当时Windows Rust1.97 offline/locked 选中五项全部通过，0 failed/ignored；单文件 rustfmt 与包级 lib/tests 严格 Clippy 通过，均不算 Linux 验收。旧 Windows Clippy 的既有 needless_return 失败原件保留，后续只修非Linux等价尾表达式后重新检查。

未来权限集成的拒绝／撤销回归仍需对应最小实现审查后复用现有内部 fixture；不得用 stub 返回首签成功、伪造独立 key 或新 constructor 让真实正向组变绿。适用真实回归另用全新资源、源码与 binary 证据。

## 最终结果边界的本地回归（后续准备检查点）

新增五个cfg(test) body：四种outcome拒绝／撤销／unwind，加一个八字段自洽identity替换组。沿用原Store和原publish/finalize_use路径；outcome适配器仅属于测试，不给生产入口增加分支、公共构造器或传入callback。八个字段变体不等于八个独立测试；仅证明现有CommitIdentity绑定，不声称覆盖尚未实现的source/build、registry代际或跨域认证。

实际Windows模块19/19：既有14项重跑＋新5项，0失败／忽略；新旧任务前缀共10个unique body。原六位置seam循环这次也在Windows执行，但不改写原Linux历史，也不产生物理FS执行信用。原14个测试和运行时前缀逐字不变。outcome失败后的“可解析terminal”与“原成功权限”仍分开；同ID重试与最终消费拒绝、旧原件保留分别断言。格式／包级lib-tests严格Clippy通过；没有独立域、签名、网络或正常恢复用例执行。

37条集成规格的ID、内容和NOT_EXECUTED状态全部保留；新增结果记录在清单的local_outcome_and_binding_units中，产品不读取它。

## 实施前仍须明确的四项决定

1. **目标与字节访问方式：** 独立运行方和故障域、目标根命名、源／目的端如何读取数据、现有 `BackupDir` 所需目录／同步／no-replace／身份语义。NAS 或对象存储不能未经核实直接当作等价 POSIX 根。
2. **托管与管理通道：** 谁持有独立私钥、现有安装 trust/registry、构建公钥 pin、双方实际调用身份及允许的窄操作。此稿不选定新认证机制，也不配置持久凭证或服务。
3. **活操作准入与撤销排序：** 认证请求如何证明仍对应原成功 D 和仍有效 S；P 的新发布 authority 从何而来、何时消费；取消、崩溃和消息重复如何收敛。
4. **结果已提交与结果未知的证据边界：** 发布成功点、观察丢失和历史只读重开的裁决如何证明；不能靠后来的 resync/readback 补造过去的成功操作。

这四项是进入权限接口实现审查的输入，不是以缺少硬件为由预先开放入口。先完成具体方案独审，再实现对应 Task3 最小差异及测试；之后任何凭证／信任／权限安装、源端断联或目标恢复写入仍需精确执行审批。

## 当前交付与未完成项

本地交付是本草案、带源码摘要的37条未执行规格、结构检查记录与分层Windows纯单测原件。没有新增 authority、signer、服务、key、token 格式、公共构造器或可运行的单机跨机替身；没有改变原 Rust 权限或重跑 Task8；两个源码叶的有限测试／风格差异及 Windows 检查单列，不冒充旧 Source679 ZIP 已包含新测试。Task9 五个正式验收框、C4、正常 CompleteBackup/RecoveredInstance 和生产保持开放。
