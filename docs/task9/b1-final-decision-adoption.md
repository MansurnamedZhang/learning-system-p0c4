# Task9 B1 最终决定时序：用户语义采纳记录

日期：2026-10-26。状态：**SEMANTICS_ADOPTED_SPEC_FINALIZATION / SPEC_REVIEW_PASSED / NOT_IMPLEMENTED / R2B_NOT_AUTHORIZED**。

用户在本对话选择项中的逐字答复为：**“采纳方案，仅定稿规范（推荐）”**。原问项明确列出源最终核对与持久保护后由原D一次发布、此后S退出/迟到取消不回夺许可、UNKNOWN不自动重试、fence可能跨group长期阻塞且异常解除另审、历史已签字节不补原P成功；仅批准规范定稿与独立审查，不批准R2b、key/trust、源隔离或恢复。此处按Root转交核对的问项/答复范围记录，不读取会话文件，不编造人类答复的精确OS时间。

## 被采纳的封存输入与审查边界

| 输入 | 固定记录 |
|---|---|
| [原提案](b1-final-decision-contract-proposal.md) | SHA-256 `2448f4592a64bc9bc5ec5b2e618f936b8f47412c2ab0c23526ae3623d6b7ae59`，原字节未改；公开历史提案提交 `58402d52e81ce54a6b106a04ee302e0bef5d1517` |
| 提案独立审查INDEX | SHA-256 `7613d34c8ad85a019d60e2f5575ad0c626fbe98ed4856e1b6abdce2d83348aff`；结论 `PASS_LIMITED_REQUEST_USER_ADOPTION_EXPLICIT_SPEC_ADJUSTMENT` |
| 该审查当时的范围 | 仅认为适合请求用户明确采纳；其中 `semantic_contract_adopted=false`、R2b/安装/权限/Task9均false保留当时原记录，不把它回填为当前规范新稿已独审通过 |
| 本次当前动作 | 按用户答复定稿窄异机语义，提交独立规范审查；不实现或启用产品入口，无新测试 |

原提案的NOT_ADOPTED等状态是冻结时点事实，文件不回写；当前用户采纳事实由本记录单独连接。采纳正文仅以[完成规范Appendix B1.1–B1.6](../superpowers/specs/2026-10-04-p0c4-completion-design.md)的Task9窄异机例外定稿为准，[准备规格](../superpowers/specs/2026-10-26-task9-b1-integration-preparation.md)同步现行解释，旧本地约束和全部历史执行记录保留。[Task9施工单](../superpowers/plans/2026-10-04-p0c4-completion.md)只补当前依赖，Task1–8及五条未勾门不改。

## 已采纳语义，尚待实现证明

- 原S在原actor/lease/累计预算/根下，最终核对并独占持久化、同步、持有FD回读负fence，再选择一次本会话/新挑战的固定最终决定；认证必须证明确切原manager，不是SSH账号、自报build或root0600本身。
- 原D十一阶段及outcome自然成功后，原D/C在共同本地串行裁决内Pending→Consuming（许可点）→Committed（完整写入/同步/验证自然完成）或Failed，不回退。旧finalize_use不one-shot；迟到S死亡/取消不回夺许可，本地错误/deadline/unwind仍失败不重试，不从新进程/旧决定恢复权限。
- fence只授负约束，覆盖全部相关writer/旧构建/重新登记及global-generation影响，计入固定预算；UNKNOWN可能长期跨group阻塞，不TTL解除或自动重试。只有未先行失效或进入失败/UNKNOWN/HOLD的有效原S，才可能在正常核验本次成功后追加关闭；迟到ACK、历史读取或后来receipt读回不恢复资格。该窄资格仍需实现证明，新进程异常解除另审；它不是持续活性或首签权限。
- 签名仅在最终许可后内部生成，完整receipt自然写入/同步/验证前不对外提供witness或成功对象。原调用成功、后来历史只读接受、新首签/重发资格三账分开；保留现v2对合法完整已签残留的历史读取，不追补当时P成功或计为正常首发正例。
- 管理与自动transport身份分离；新的源认证key不同于D备份签名key。可信固定启动/二进制/身份使用、受限principal/固定命令及任何权限变更均另审，不能默认把管理凭证或系统Docker访问权交给S。

## 未批准与未完成

此次批准只覆盖规范定稿和独立审查。**不批准R2b编码、机器配置/启用、key/trust/registry/fence/source-auth安装、用户/公钥/forced-command/sudoers/权限变更、源隔离、发布或恢复写入。** 规范独审通过本身也不自动扩大该授权；后续实现及B/C/D执行均需准确范围和另行批准。 证明义务分阶段：R2b编码前先具体化合同、authority源码入口/影响面、固定预算/布局和验证计划，并取得准确编码范围批准；代码完成后独审并取得适用的实际窗口/reader/回归原件；启用、正式部署或Task9接受前须闭合规定证明并获对应执行审批。验证用隔离执行也须具体批准；不要求尚未写出的实现先取得真实运行证明，不降低后续证明义务，本轮R2b仍未批准。

代码仍未实现新权限合同，公共v2、原本地拒绝路径、37条case及其未执行状态、Task9五门、Task8历史与旧失败/HOLD保持。没有新产品/准备测试、远程连接、凭证使用或Task9/C4/生产验收信用。四份冻结新稿的独审结果与后审状态收尾范围见下节；规范通过不批准R2b或现场动作。

## 规范独审与状态收尾

首版规范审稿因证明阶段循环依赖暂不批准，旧稿及独审保留；INDEX SHA-256 `72946984ebdea8eebed4d22716309b2c1067c6946d3eaba4a7d51624f95b0240`。随后仅修正编码前合同/计划、代码后实际验证、启用前证明闭合的顺序，并将design原CRLF规范化为LF，不削弱证明义务。

修正版独审INDEX SHA-256 `95d3bf176a3e4419df9489e009d243051ff2e5ee6cbd83aa0183bd0de9e5d7cd`，结论仅准规范定稿；`approved_for_normative_specification_finalization=true`、`semantic_contract_adopted=true`，R2b、安装、Task9接受仍为false。其冻结输入如下：

| 输入 | 独审时SHA-256 |
|---|---|
| `docs/superpowers/specs/2026-10-04-p0c4-completion-design.md` | `1c57f9b09baee290ec07a1b8dd27d8afa0b8399e78fc600d14be0f58976638a3` |
| `docs/superpowers/specs/2026-10-26-task9-b1-integration-preparation.md` | `209ce1306e45a8c624d1ddd6d4becf0ff399d226feda0a665dec34f3caad9bbb` |
| `docs/superpowers/plans/2026-10-04-p0c4-completion.md` | `6f27a9541135a5e8e93d67668a457d910636f4fbdda330f982a48a33d328d376` |
| `docs/task9/b1-final-decision-adoption.md` | `0dd79f2d450348577abc6ba191587194a4060731796a6a2224b17e52188407b2` |

本页及三份规范/计划随后仅补上述已发生独审的状态文字和本节指针；没有改动已审时序/权限/证明义务。旧审输入副本原样保留，后续发布独审须逐字核对这组status-only变化；不将修后文件冒充旧SHA。没有新产品测试、机器动作或正式验收信用。
