# P0-C4 Task7：完整 Attention 与隔离验收

## Task7 Source36 私有单机六案原件验收限定关闭 · 2026-10-25 15:41:01（北京时间，Root采用快照）

源码 **`36c89f2e94dccdc90652f8b196372bb62fa398a5`** 的 Task7 已由 Root 依据同案独审原件显式采用，**限定关闭6/6**。范围仅为**代码＋规定私有单机夹具的原始字节验收**；同源五阶段来源为**此前3项保留＋本轮新3项返回**，不是六项新跑。Root采用逻辑标识 `TASK7_SOURCE36_PRIVATE_SINGLE_HOST_SIX_CASE_ORIGINALS_ACCEPTED`，SHA256 **`1a2f293cbf3c9ee1512e8bec9995ade5d31b404034e37b7d531acd2e72356db4`**。56,097条文件引用已复读（按引用路径键计数，不表示物理文件数）。此前各独审记录中的formal0／Task7 OPEN保持原历史值，本次采用由相连后验单独记录。

六案 typed 来源、实际 metadata、Core／Proof完整原流及各自报告语义均已闭合。完整正例与缺失原件／传输中断两条good-pin恢复路径的P6，**每案245文件、15个完整journal frame、三项finding＝1 AlreadyCommitted＋2 ConfirmedNoEffect、三份真实live UID65532 worker见证**；实际ReadingCopy四文件共**1172B**，原package、source capture、独立destination copy与held target delivery四处逐字相等，每文件四处inode独立。声明与实际文件字节已分别核对。

所有旧失败UUID、原件及HOLD保留且不重放；53df虽helper自然0、完整流返回，外层保护失败仍为0信用，仅按原c541完整身份与资源守卫精确移除已知exited helper，保留suite及其它完整C/N/V。缺失原件／传输中断两次P6 metadata按各自当时完整637容器基线执行并保持；最后两条bytes捕获在同步资源自然结束、五份只读采集原件观测sync=[]后串行启动。各metadata／bytes均保护其当时完整current C/N/V基线及原守卫，不把不同操作的全局基线伪装成旧636全等；未停止／删除／排除其他项目资源，无失败瞬间raw diff时不推断根因。namespace字段映射与固定image继承OCI标签均为已独审窄修，原守卫与预算保持。

原wrapper未另存完整helper／producer inspect raw及独立外层OS双流／最终elapsed／drain事件收据；当前证据为固定完整guard程序、实际调用摘要、完整C/N/V与映射／image／input／attach／result原件。双EOF／自然退出／期限按该原成功路径和实际原件范围采用，不补造独立OS见证。应用2MiB进程、16MiB文件及helper预算与Root512MiB完整原流通道不变；四份受保护文件未改，任务源码保持36c89f2。

**Task8／Task9、P0-C4整关、独立故障域与外部备份存储、CompleteBackup、RecoveredInstance、正常公开恢复和生产仍OPEN**。Task6私有单机16/16历史范围保持；本次关闭不签发公开恢复资格，不合并main、不部署。

| 固定场景 | 来源 | 本次限定边界（六案均已按其预期采用） |
|---|---|---|
| `attention-full-chain` | 此前来源保留 | 完整正例；Core4458／Proof4464；64表、63表不变＋10任务13列允许归一化，Reading权限与原件核对；P6与阅读副本实际字节闭合。 |
| `attention-missing-asset` | 此前来源保留 | 缺失声明原件在导入前拒绝；实际good-pin回退恢复及P6／交付闭合。failed_new_capture_preserves_previous_pin原值false，不升级该未证明分支。 |
| `attention-corrupt-dump` | 此前来源保留 | DUMP_FREEZE_CUSTOM_HEADER处导入前拒绝；完整before/after与小index重建大legacy字节，目标导入／归一化／交付未宣称成功。 |
| `attention-interrupted-transfer` | 本轮新UUID返回 | 目标复制chunk后中断拒绝；新capture在本地封存前失败仍保留旧pin；previous-pin实际恢复及P6／交付闭合。旧53df导出失败0信用。 |
| `attention-unknown-effect` | 本轮新UUID返回 | 已导入后POST_IMPORT_EXTERNAL_EFFECT_VALIDATION拒绝；UNKNOWN hold3／runnable0／positive Root finding0，unchanged=false，无阅读交付。 |
| `attention-root-replacement` | 本轮新UUID返回 | held与replacement inode不同，导入前拒绝；列举DB catalog/assets/attempt的完整9091268B before/after相等，不声称整个文件系统未变。 |

| 当前阶段 | 状态与边界 |
|---|---|
| Task6 | 历史代码＋规定私有单机16/16限定完成 |
| Task7 | Source36规定私有单机原件验收6/6限定关闭（此前3＋新3） |
| Task8／Task9、P0-C4整关 | OPEN |
| 独立存储、CompleteBackup、RecoveredInstance、公开恢复、生产 | OPEN；main未合并、未部署 |

独审逻辑标识与SHA256：core-full-chain `77b966a6f69bd277d4226c1e7ca93f44e9bf2a4f10b7359a611bab51bb79bc0e`；core-missing `2bc86208f27d16eb5ef7050484dd904a4328e189fdaac743971750af01afbaa2`；core-corrupt `80aa499cdb7e85bf176272f1059811cd3ebd8ed0b6992576fcd446bf86c2ff1a`；core-interrupted `67389b93dd08c3f656a281e6a066696ced535d373f8fd42323d3868ce038a2f1`；core-unknown `9c4f59192c16b45861ee0b908f06fcfa7795e7eda47e2e84adf5f7495ca7e231`；core-root-replacement `bfd345cd1de4db4848041ac2b890c9c3c48742687b152c45d2b75c6961c2edff`；p6-three-good-pin-original-bytes `9e6335a715d2bc3d3b1e74acac81b18eb6072f7855f26cde029e6828fb457bd4`；failed-53df-exact-known-cleanup `f1f1f3fcdef5a425258a1a5b72d9519e456b0d21955d99e2af4835b305cf1b39`。

---

<details><summary>此前检查点（保留当时来源与范围，不代表当前状态）</summary>

## Task7 六案metadata限定通过，首Core／四报告独核闭合 · 2026-10-25 04:57:17（北京时间，固定快照）

当前源码 **`36c89f2e94dccdc90652f8b196372bb62fa398a5`** 的同源五阶段回执保持 **6/6＝此前3项来源保留＋新3项返回**；此前三项没有重跑计数。六案 typed 来源准入及六次 metadata 原件复读均已限定 PASS，metadata 取证不等于业务接受，**正式接受仍0/6，Task7 OPEN**。

首个完整正例的 Core 4,458 份、Proof 4,464 份原件及四报告已独立复读：64张表、10个任务、5个物理原件／601字节（6项logical）连接一致。该结论只覆盖本案 Core／四报告；其余五案 Core／Proof、三个 good-pin 路径的 P6 与实际交付字节仍待导出和独核，三项finding或ReadingCopy文件声明不代替实际原件。

metadata 首次 namespace 四字段到原采样器两字段的映射失败、随后固定image继承OCI标签校验失败，均已定位并经窄修独审；原native采样器、全C/N/V保护与预算保持。所有失败UUID、原件和HOLD原样保留、不重放。仅在现场严格重核后精确移除了唯一已知自建、从未启动的helper；源目标／suite／owner原件留存，不扩展到其他资源清理。

有限前置Rust562通过／142忽略、ordinary DAC1通过、Root Python385通过／1跳过，以及Task6私有单机16/16历史结论保持；旧f63不转授当前源码。应用2MiB进程／16MiB文件与Root原件512MiB完整流预算保持。Task8／9、P0-C4整关、独立故障域与外部备份存储、CompleteBackup、公开恢复和生产仍OPEN；候选分支 `feat/task7-attention-recovery` 未合并main、未部署。

独审逻辑标识与SHA256：metadata-full-chain `60d4d43bf583bc816866808346e6b77087d78307c797de26833d2a749270ae0a`；metadata-other-five `f41d051233dc8a9363d4216d441f112ff7561bc3e7a5edf4c9e6a7a6807ec06e`；full-chain-core-four-reports `77b966a6f69bd277d4226c1e7ca93f44e9bf2a4f10b7359a611bab51bb79bc0e`；known-never-started-helper-cleanup `37ff26963fc575c2e9094f138a060d714f0927bd29d99e0c4688466c457efebc`。

---

</details>

<details><summary>此前检查点（保留当时状态）</summary>

## Task7 同源回执6/6：此前三项来源采用＋新3项返回 · 2026-10-25 03:27:08（北京时间，固定快照）

六案 typed 来源准入已实际重读通过：此前3项＋新3项，30个控制容器均完成精确清理。正式原件接受仍0/6。首个只读取证操作在 sample-before 因命名空间四字段与原采样器两字段的接口映射错误而拒绝，尚未创建helper；失败UUID/HOLD与完整原件保留，正作窄修，不重放、不放宽资源保护。

当前源码 **`36c89f2e94dccdc90652f8b196372bb62fa398a5`** 的续跑队列保持无HOLD；**传输中断、未知效果、根替换** 已在全新UUID中返回完整五阶段回执。此前缺失原件、完整正例、损坏dump三项通过显式来源账本重核完整五阶段、15个控制容器及原始SHA，按限定调度独审保留来源；**不是重跑三次，也不是新增三项业务接受**。合计同源回执 **6/6＝此前3＋新3**，正式原始证据接受仍 **0/6，Task7 OPEN**。后续：无待返回场景。

旧队列在prepare触发 `WHOLE_CURRENT_BASELINE_MISMATCH` 的HOLD和失败UUID原样保留，不重放、不抹除。新队列保留原timer／安全窗口、全C/N/V与600/7800秒预算，并在所有状态的临时同步容器完全消失前等待；不停止、删除或排除其他项目资源。

当前为执行回执与来源重核检查点，六案typed准入已重读通过；原始Core／P6／交付字节导出及独核尚未闭合。旧f63实际原件结论仅属历史，不转授当前源码；应用2MiB进程／16MiB文件及Root原件512MiB完整流预算保持。

有限前置仍为Rust562通过／142忽略、ordinary DAC1通过、Root Python385通过／1跳过；Task6私有单机16/16历史保持。进度同步候选分支 `feat/task7-attention-recovery`，main未合并、未部署；**Task7–9／P0-C4整关、独立存储、CompleteBackup、公开恢复与生产仍OPEN**。

---

</details>

<details><summary>此前检查点（保留当时状态）</summary>

## Task7 同源回执5/6：此前三项来源采用＋新2项返回 · 2026-10-25 03:00:40（北京时间，固定快照）

当前源码 **`36c89f2e94dccdc90652f8b196372bb62fa398a5`** 的续跑队列保持无HOLD；**传输中断、未知效果** 已在全新UUID中返回完整五阶段回执。此前缺失原件、完整正例、损坏dump三项通过显式来源账本重核完整五阶段、15个控制容器及原始SHA，按限定调度独审保留来源；**不是重跑三次，也不是新增三项业务接受**。合计同源回执 **5/6＝此前3＋新2**，正式原始证据接受仍 **0/6，Task7 OPEN**。后续：根替换。

旧队列在prepare触发 `WHOLE_CURRENT_BASELINE_MISMATCH` 的HOLD和失败UUID原样保留，不重放、不抹除。新队列保留原timer／安全窗口、全C/N/V与600/7800秒预算，并在所有状态的临时同步容器完全消失前等待；不停止、删除或排除其他项目资源。

当前为执行回执与来源重核检查点，完整六案typed准入、原始Core／P6／交付字节导出及独核尚未闭合。旧f63实际原件结论仅属历史，不转授当前源码；应用2MiB进程／16MiB文件及Root原件512MiB完整流预算保持。

有限前置仍为Rust562通过／142忽略、ordinary DAC1通过、Root Python385通过／1跳过；Task6私有单机16/16历史保持。进度同步候选分支 `feat/task7-attention-recovery`，main未合并、未部署；**Task7–9／P0-C4整关、独立存储、CompleteBackup、公开恢复与生产仍OPEN**。

---

<details><summary>此前检查点（保留当时状态）</summary>

## Task7 同源回执4/6：此前三项来源采用＋新1项返回 · 2026-10-25 02:35:01（北京时间，固定快照）

当前源码 **`36c89f2e94dccdc90652f8b196372bb62fa398a5`** 的续跑队列保持无HOLD；**传输中断** 已在全新UUID中返回完整五阶段回执。此前缺失原件、完整正例、损坏dump三项通过显式来源账本重核完整五阶段、15个控制容器及原始SHA，按限定调度独审保留来源；**不是重跑三次，也不是新增三项业务接受**。合计同源回执 **4/6＝此前3＋新1**，正式原始证据接受仍 **0/6，Task7 OPEN**。后续：未知效果、根替换。

旧队列在prepare触发 `WHOLE_CURRENT_BASELINE_MISMATCH` 的HOLD和失败UUID原样保留，不重放、不抹除。新队列保留原timer／安全窗口、全C/N/V与600/7800秒预算，并在所有状态的临时同步容器完全消失前等待；不停止、删除或排除其他项目资源。

当前为执行回执与来源重核检查点，完整六案typed准入、原始Core／P6／交付字节导出及独核尚未闭合。旧f63实际原件结论仅属历史，不转授当前源码；应用2MiB进程／16MiB文件及Root原件512MiB完整流预算保持。

有限前置仍为Rust562通过／142忽略、ordinary DAC1通过、Root Python385通过／1跳过；Task6私有单机16/16历史保持。进度同步候选分支 `feat/task7-attention-recovery`，main未合并、未部署；**Task7–9／P0-C4整关、独立存储、CompleteBackup、公开恢复与生产仍OPEN**。

---

<details><summary>此前检查点（保留当时状态）</summary>

## Task7 准备阶段保护暂停，前三项回执保留 · 2026-10-25 01:18:40（北京时间，固定快照）

当前源码 **`36c89f2e94dccdc90652f8b196372bb62fa398a5`** 的缺失原件、完整正例、损坏dump三项此前已返回完整五阶段回执；**正式原件接受仍0/6，Task7 OPEN**。第四项传输中断在 prepare 的全量资源基线比较处拒绝（`WHOLE_CURRENT_BASELINE_MISMATCH`）：实际进程退出1、未超时、尚无case_id及已知capsule，未进入测试正文。原串行队列已HOLD，未知效果和根替换尚未运行。

两次静默快照检查记录的投影哈希不同。后续只读检查发现外部临时容器消失及其网络摘要变化；未保存失败瞬间完整投影，因此不补造具体差异或声称精确根因已定。原全C/N/V保护和600/7800秒预算保持，不停止、删除或排除其他项目资源。

后续准备在同步临时容器运行时等待，用全新UUID续跑剩余三项。前三项若复用，必须显式保留原始来源、完整回执与HOLD历史，经过同源原件重核及独立审查；不能复制成一次新执行，也不能直接绕过旧队列HOLD。原始Core／P6／交付完整字节独核仍未完成。

同源有限前置保持：Rust562通过／142忽略、ordinary DAC1通过、Root Python385通过／1跳过。历史f63原件结论仅属历史，不转授当前源码。代码与此检查点同步候选分支 `feat/task7-attention-recovery`，未合并main、未部署；**Task7–9／P0-C4整关、独立存储、CompleteBackup、公开恢复与生产仍OPEN**。

---

<details><summary>此前检查点（保留当时状态）</summary>

## Task7 同源隔离回执已返回3/6，原件独核待闭合 · 2026-10-25 00:34:48（北京时间，固定快照）

当前源码 **`36c89f2e94dccdc90652f8b196372bb62fa398a5`** 已有 **3/6** 个场景返回完整五阶段回执：**缺失原件、完整正例、损坏dump**。每项均核对同源、独立新UUID及五个控制容器已精确清理；回执原始哈希已记录。后续场景：传输中断、未知效果、根替换，按原安全窗口和保护约束串行推进。

**正式接受仍0/6，Task7 OPEN。** 返回回执未代替原始Core／P6／交付字节独核；六案全部稳定后才统一导出和独核，异常保留HOLD、不重放失败UUID。应用原2MiB进程、16MiB文件及helper上限保持；Root私有原件导出沿既有独立512MiB完整流通路，未放宽应用预算。

同源有限前置保持：Rust562通过／142忽略、ordinary DAC1通过；Root Python385通过／1跳过。历史f63正例原始P6已由另一代理独核并由主流程重核245原件／21项引用，实际三项效果与ReadingCopy1172字节闭合；仅为历史结论，不转授当前源码。

代码与进度同步候选分支 `feat/task7-attention-recovery`，main未合并、未部署。Task6私有单机16/16历史保持；**Task7–9／P0-C4整关、独立存储、CompleteBackup、公开恢复与生产仍OPEN**。此为固定时间快照，不是实时完成承诺。

---

<details><summary>此前检查点（仅保留当时状态）</summary>

## Task7 同源隔离回执已返回2/6，原件独核待闭合 · 2026-10-24 23:56:04（北京时间，固定快照）

当前源码 **`36c89f2e94dccdc90652f8b196372bb62fa398a5`** 已有 **2/6** 个场景返回完整五阶段回执：**缺失原件、完整正例**。每项均核对同源、独立新UUID及五个控制容器已精确清理；回执原始哈希已记录。后续场景：损坏dump、传输中断、未知效果、根替换，按原安全窗口和保护约束串行推进。

**正式接受仍0/6，Task7 OPEN。** 返回回执未代替原始Core／P6／交付字节独核；六案全部稳定后才统一导出和独核，异常保留HOLD、不重放失败UUID。应用原2MiB进程、16MiB文件及helper上限保持；Root私有原件导出沿既有独立512MiB完整流通路，未放宽应用预算。

同源有限前置保持：Rust562通过／142忽略、ordinary DAC1通过；Root Python385通过／1跳过。历史f63正例原始P6已由另一代理独核并由主流程重核245原件／21项引用，实际三项效果与ReadingCopy1172字节闭合；仅为历史结论，不转授当前源码。

代码与进度同步候选分支 `feat/task7-attention-recovery`，main未合并、未部署。Task6私有单机16/16历史保持；**Task7–9／P0-C4整关、独立存储、CompleteBackup、公开恢复与生产仍OPEN**。此为固定时间快照，不是实时完成承诺。

---

<details><summary>此前检查点（仅保留当时状态）</summary>

## Task7 新隔离队列已启动，历史正例原件独核闭合 · 2026-10-24 23:00:43（北京时间，固定快照）

当前候选源码 **`36c89f2e94dccdc90652f8b196372bb62fa398a5`** 的 Linux 有限前置已独核：Rust **562 passed／142 ignored**、ordinary DAC **1 passed**；Root Python **385 passed／1 skipped**。修复与前置记录已同步到 GitHub 候选分支，上一同步提交 `21edc8e`；main 未合并、未部署。

新六案串行队列已完成现场 Windows SSH 只读检查和调度绑定，正在等待既有资源的安全启动窗口，先执行缺失原件场景。本快照时 **新源码尚无返回场景，正式接受0/6**；每案保留原进程／文件／helper预算、完整C/N/V保护、自然退出与精确清理，异常即HOLD、不重放失败UUID。原只读传输失败与已取消的本地IDLE记录保留；旧LF经独立执行已证正确，环境修复不宣称修好了未复现的换行错误。

**历史 f63 完整正例的原始P6补验已独核闭合，仅限历史源码。** 9,283,670字节完整流重建245份原件；完整15帧Root日志、实际三项恢复效果、注册根／当前pin／归一化交易闭合，四份ReadingCopy实际1172字节及四处完整副本逐字一致。远端执行、只读挂载、停机清理及全C/N/V保护原件也已重核。独审INDEX：`a4ddfa8ffb27ffb9be61c6f5fe08fc3a018893e2ec54da953f6e0c3ca311b009`。旧源码结论不转授当前源码，当前六案仍需实际返回与原件独核。

Task6私有单机16/16历史范围保持；**Task7／Task8／Task9／P0-C4整关、独立存储、CompleteBackup、公开恢复与生产仍OPEN**。这是已执行工作和后台队列的时间快照，不是实时完成承诺。

---

<details><summary>此前检查点（保留当时状态，不代表当前进度）</summary>

## Task7 读取修复及同源有限前置已独核，六案 native 待验 · 2026-10-24 22:08:02（北京时间，候选草稿）

当前候选源码 **`36c89f2e94dccdc90652f8b196372bb62fa398a5`**（父提交 `f63c48ecd9ed62304434291ed5140bc82ec751f8`）已纳入六份经双向独立源码审查的修复。新 source 的 Linux 有限检查与 Root Python **已按原始回执独立核对，仅接受为有限前置**，审查 INDEX：`3e44f27d6c5b7a1fef3f84d32b6e8621e3c23bc631336ae0ff88be4a374ff497`。六项同源 native 尚未开始，**正式接受 0/6，Task7 OPEN**；后续六案原件独核仍 pending。

读取修复仅在固定 proof 超过原单进程预算时新增 **1 个 proofs 只读 helper**，保持原 **2MiB 单进程输出、16MiB proof 文件、2 个 helper 同时存活及 82 个累计 helper** 上限。固定持有 FD 的完整分块读取、身份核对、全量哈希、自然退出及精确清理保持闭合；当前小报告正例保留原 cat 顺序与零新增 helper。损坏 dump 使用固定小 index 与两份完整 before/after 重建原 legacy 字节及哈希，继续连接原 full-import negative_evidence；旧 generic producer、15 份迁移及既有守卫保持。

新 source 的同源有限前置独核结果：Rust **562 passed／0 failed／142 ignored（未执行）**，新增两项 corrupt_split Rust 单测均实际 `ok`，ordinary DAC **1 passed**；Root Python 原始日志为 **Ran 386，OK (skipped=1)**，即 **385 passed／1 skipped**。16 份原始过程文件、611 源文件及 22 个 controller pin 已核；两只 builder 自然 exit0、精确移除，C624／N592／V2107 完整保护投影保持，未新增 PG、network 或 volume。有限结果 SHA：`91a4c17139af68a090c33b35cbbef30a15a19f8765e8948fe1f73ac08308ae21`；Root Python 结果 SHA：`454a449004cd843c29d47e31dcd6235309ea646c4f9f043ffb9cbc5b5218de53`。

**六项 Attention native 为 6 ignored＋6 listed、实际 0 executed**，列举和编译不计执行信用。本有限配方不包含完整 workspace 的真实 PostgreSQL／四套旧版升级验收，不授予 Task8 完成信用。

此前 Root 私有组合 57 项、独立合并 76 项属于 **portable**，范围有重叠、不相加、不代替上述 Linux 返回或 native 验收。reader 源码独审 INDEX：`481eb4e2f203f33f633d53ee2ad2c298025a3265aaea7258f8b6657070650a0c`；split 源码独审 INDEX：`0158518da952b4816b6fc09f34c2d3b370510a9433bf8edfbbf111a9ee0c16ad`。

| 当前绑定 | 值 |
|---|---|
| SourceCommit | `36c89f2e94dccdc90652f8b196372bb62fa398a5` |
| ParentSourceCommit | `f63c48ecd9ed62304434291ed5140bc82ec751f8` |
| ArchivePIN / SHA-256 | `99288c6b8074b8ff2009ee06724518eeee056864f8917b7a8aee60e9e88b1096` |
| SourceManifestPIN / SHA-256 | `c5ab0f0b371ff62db0fbbc0ca18a671cd6d5c339375538deed03de9cc42b3960` |
| 源码包 | 611 个 Git 源文件、22 个 controller pin |
| Linux 有限前置独核 | **Rust 562 passed／142 ignored；DAC 1 passed** |
| Root Python 独核 | **386 项：385 passed／1 skipped** |
| 两项新 Rust 单测 | **已实际 ok，纳入本轮有限独核** |
| 有限独审 INDEX | `3e44f27d6c5b7a1fef3f84d32b6e8621e3c23bc631336ae0ff88be4a374ff497` |
| 六案 native / P6 与交付原件闭合 | **pending：新源码全部重新执行；正式接受 0/6** |

`historical_f63`：旧 f63 的有限 Rust 560 通过／142 ignored、ordinary DAC 1 通过、Root Python 363 通过／1 跳过，以及正例五阶段返回、P6／交付待核、missing-asset 后置 `PROCESS_LOG_BUDGET`／`FAILED_UNUSABLE` 和旧 timer 保护差异，均保留在下方历史区及 manifest 的 historical_f63 内。旧源码信用不转授本源码；原 `2ced636` RED 与失败／HOLD 记录保持。

Task6 私有单机 16/16 历史范围保持。**Task7／Task8／Task9／P0-C4 整关、独立存储、CompleteBackup、公开恢复与生产全部 OPEN**；候选分支 `feat/task7-attention-recovery` 不表示 main 合并或部署。本 prep 仅准备草稿，未推送或修改公开 checkout。

---

<details id="historical_f63"><summary>historical_f63 与此前封存检查点（仅历史，不转授当前源码）</summary>

## Task7 大报告读取修复中，剩余隔离验证尚未接受 · 2026-10-24 20:57:44（北京时间，固定检查点）

当前源码 `f63c48ecd9ed62304434291ed5140bc82ec751f8`：完整 Attention 正例已返回五阶段通过，原始1 passed／0 failed／0 ignored；64源表、63表不变与六任务表／10个任务／13列归一化、48函数及两项明确目录差异已完成报告层独核。4,416份核心与追加原件已导回，P6／Root findings及实际阅读交付文件仍待完整原件闭合。**正式接受仍0/6，Task7保持OPEN**。

缺失资产场景的 Rust 正文同样实际1 passed／0 failed／0 ignored、exit0，但驱动后置证据读取触发 `PROCESS_LOG_BUDGET`，最终回执为 `FAILED_UNUSABLE`，不计通过。真实前、后完整审计各 **9,091,268字节**，在原16MiB文件上限内，却超过单进程2MiB输出上限。正在实现保留两个上限、完整字节与身份验证的固定分段读取；不截断报告或放宽全量比较。

本轮失败UUID及原始回执保留。源库与目标库停机留卷，已确认空闲的新控制父容器精确移除。后续收尾核对还观察到一只旧控制容器在原run退出后达到86400秒并自行exit0；原保护差异单独保留，不改写为通过、不操作该旧容器。损坏dump、中断传输、未知效果、根替换四案尚未执行。新读取代码审查通过后会按新源码和全新UUID重组同源六案，旧f63执行信用不转授。

同源有限前置Rust560实际通过／142忽略、ordinary DAC1通过及Root Python363通过／1跳过仍是f63历史范围。Task6私有单机16/16历史保持。Task7–9／P0-C4整关、独立存储、CompleteBackup、公开恢复与生产仍OPEN；只同步候选分支和路线图，未合并main或部署。

---

<details><summary>此前检查点（保留原始状态）</summary>

## Task7 首项整链返回通过，原件独核与五项反例推进中 · 2026-10-24 20:17:21（北京时间，固定检查点）

源码 `f63c48ecd9ed62304434291ed5140bc82ec751f8` 的完整 Attention 正例已在新 Linux／PG18 隔离批次中完成 prepare／install／plan／run／verify 五阶段，返回精确测试 **1 passed／0 failed／0 ignored，exit 0**。源库、目标库均停机留卷。首项的 **4,410份核心原件与日志**已限界只读导回；导出 helper 自然结束并精确移除，完整既有 C/N/V 投影一致，正在独立核对 Attention、资产和 P6 证明。**返回通过1/6，正式独立接受仍0/6**。

| 场景 | 当前检查点 | 正式接受 |
|---|---|---|
|完整 Attention／原件／阅读整链|五阶段返回通过，原件独核中|待定|
|缺失资产|全新隔离批次执行中|待定|
|损坏 dump|待串行执行|待定|
|中断传输|待串行执行|待定|
|未知外部效果|待串行执行|待定|
|根目录替换|待串行执行|待定|

同源有限前置维持：Rust实际560通过／142 ignored未执行，ordinary DAC 1通过；Root Python363通过／1跳过。代码、15份迁移和四份受保护草稿未因本次文档同步而修改；失败UUID不重放。后续场景每次新建独立资源，收尾与原始证据按实际来源核对。

Task7保持OPEN；Task6私有单机16/16历史保持。Task8／Task9／P0-C4整关、独立备份存储、CompleteBackup、公开恢复和生产仍OPEN。本次只同步候选分支与实施路线图，未合并main或部署。

---

<details><summary>此前检查点（保留来源与范围）</summary>

## Task7 同源 Linux 前置通过，六案隔离验证中 · 2026-10-24 19:43:19（北京时间，固定检查点）

当前源码 `f63c48ecd9ed62304434291ed5140bc82ec751f8` 已完成有限前置原件独核：格式、严格Clippy与Rust **560项实际通过／142项ignored未执行**，ordinary DAC **1项通过**；同源Root Python **363项通过／1项跳过**（共364项）。源码包609个Git文件、22控制器固定绑定，builder自然退出并清理，既有完整资源投影保持。上述均不等同Task8真实PG／四套旧版升级验收。

Task7六项实际PostgreSQL整链已安排串行新批次，正式独立接受仍 **0/6**，Task7未完成。任一阶段失败即停止，不重放失败UUID；通过返回与原件独立接受分别计数。

本次在 `b51da4` 上仅修正一份回归测试的场景分域，生产文件0变化：保留旧nine=9和逐案FULL_PASS，明确六Attention固定名称及正反非整关状态，四域9/5/11/6互斥闭合。便携8方法／68子测试通过，31恶意名称拒绝、7错映射反例拒绝；旧361项RootPython中的15!=9失败保留。此前已审资产预填充阶段修复和真实snapshot审计分派保持。

旧e2ce签发空根前置失败与b51 RootPython失败均停留在各自来源，不转为通过或转授信用。Task6私有单机16/16历史保持；Task7–9、C4、独立存储、CompleteBackup、公开恢复与生产仍OPEN。仅同步候选分支，未合并main、未部署。

---

<details><summary>此前检查点（保留来源与范围）</summary>

## Task7 阶段顺序与审计分派修复候选 · 2026-10-24 19:11:30（北京时间，固定检查点）

源码 `b51da4cd69d0e4b566c56997e2c454c2f3f9defd` 已按两组独立复审冻结。原生正式接受仍为 **0/6**；新源码的 Linux／PostgreSQL 验证尚未接受，Task7 保持 OPEN。

固定原件创建改到独立 issuer 空根验收、落盘证明和源挂载核对之后，再由仅挂载 assets 的已登记 root helper 创建；旧空根检查、Task6 零资产门、Source PG 只读挂载、预算和精确清理均保持。便携阶段回归54项通过，另104项扩展回归中84通过／20个Windows-POSIX跳过、helper类独立probe4通过；这些结果不是Linux或真实PG接受。

共享审计回归已分清14个旧full案例与6个Attention案例。旧96B夹具及每条旧断言保留，新六案通过真实snapshot分派核对5份物理／6条逻辑／601B夹具及严格反事实拒绝；完整类9方法通过，不增加native计数。

此前源码 `e2ce9af` 的Linux有限560通过／142忽略、普通DAC1通过，以及原件预填充早于issuer而触发的前置失败均保留为历史；旧失败停机留卷、不重放、不把旧信用转授新包。新包只含609个Git源码文件及固定22控制器，15份迁移未变。

Task6私有单机16/16历史结论保持。Task7–9、P0-C4整关、独立备份存储、CompleteBackup、公开恢复与生产仍OPEN；当前仅同步候选分支，未合并main或部署。

---

<details><summary>此前已封存检查点（保留当时范围）</summary>

## Task7 有限前置通过，整链阶段顺序修复中 · 2026-10-24 18:39:40（北京时间，固定检查点）

源码 `e2ce9af96d6fbfad8be851533232bce984984723` 的正式原生接受仍为 **0/6**，Task7 尚未完成。

本候选的 Linux 有限前置已独立核对：格式与严格 Clippy 通过，Rust 560 项实际通过、142 项 ignored 未执行，普通用户 DAC 一项通过；608 个源码 Git blob、22 个控制器 pin 与归档一致。该有限配方不是 Task8 的真实 PostgreSQL／四套升级验收。

首项新隔离整链在正式测试正文前被既有空根检查拒绝：Attention 原件预填充早于独立签发，触发 SNAPSHOT_PREPARED_PRIVATE_EMPTY_ROOT。旧失败与 HOLD 保留，不重放原批次；正在将固定原件创建移至签发通过后，空根检查及其余守卫不放宽。

源库与目标库均已精确停机留卷，仅移除自然结束业务后的已知控制容器。完整运行前 C618／N586／V2078 投影保持；未将准备失败写成业务用例通过。候选实现与有限前置的信用均不转授尚未执行的新修复源码。

Task6 私有单机 16/16 历史结论保持。Task7–9、P0-C4 整关、独立备份存储、CompleteBackup、公开恢复与生产仍 OPEN。当前仅同步候选分支 `feat/task7-attention-recovery`，未合并 main、未部署。

---

### 原始基线与待验清单

## Task7 实施中 · 2026-10-24 15:49:42（北京时间，固定检查点）

Task7 已进入完整 Attention 场景与故障注入实现，正式接受 **0/6**。当前公开的是测试基线检查点 `2ced6366a23fcc472d7b6995cddc474194313d6b`，并非 GREEN 完成版本。

全新 Linux／PG18 基线已执行真实捕获与导入，在完成全部 **64 表**的归一化前校验后，实际发现 H@1/H@2 均为 0，触发完整性守卫。精确测试退出 **101，0 passed／1 failed／0 ignored**；独立审查确认这是缺失 Attention 行为的预期 RED。失败回执与 HOLD 保留，未转为通过。

正在实现：完整历史／当前知识图、原件、Reading／Release／权限语义比较，以及缺失原件、损坏 dump、中断传输、未知效果、根目录替换五项失败注入。固定原件合同为 5 份物理文件、6 条逻辑资产，尚未取得这些 GREEN 用例的实际通过证据。

Task6 在源码 `2e5026d` 上的代码与私有单机 16/16 限定完成记录保留。Task7、Task8、Task9 与 P0-C4 整关仍 OPEN；当前没有独立备份存储，不签发 CompleteBackup 或公开恢复实例，不代表生产部署。后续证据按各自源码、编译产物和全新隔离资源记录。

详见[Task7 验收清单](../KnowWeave实施路线与架构.html#task7)。

---

| 固定场景 | 当前状态 | 通过条件 |
|---|---|---|
| `attention-full-chain` | 基线 RED 已独核；GREEN 待验 | 全量行、完整函数/ACL、Reading 与权限、原件和任务允许差异 |
| `attention-missing-asset` | 实现中，实际 GREEN 待验 | 原件缺失在目标准入前拒绝 |
| `attention-corrupt-dump` | 实现中，实际 GREEN 待验 | 损坏 dump 在目标准入前拒绝 |
| `attention-interrupted-transfer` | 实现中，实际 GREEN 待验 | 真实封存字节传输中断，保留旧 pin |
| `attention-unknown-effect` | 实现中，实际 GREEN 待验 | 导入后未知效果阻止恢复可用性 |
| `attention-root-replacement` | 实现中，实际 GREEN 待验 | 同路径新 inode 被真实根身份复核拒绝 |

基线观察摘要 SHA-256：`33ec4fbb9e87158e4595f0aaebaf254e08526d9b713a20f751a2c73bb7f7f2d4`。它证明旧空夹具缺少 Attention 历史，不替代完整夹具的验收。资源收尾核对旧 616 容器、584 网络、2067 卷的完整保护投影未变；只删除已自然结束业务子进程的本批控制容器，源／目标卷与失败原件保留。私有原件、凭据、完整 inspect 和数据库内容不发布。

当前源码包 SHA-256：`ea4c39e58f23a69e1de446ae0e66e7d487bc1532fa825863e3a5d2beb1551964`；manifest SHA-256：`4bce09c4be31e04db5b6a66a07e49f6ab9acbbad7c5db6e667a5f37ab65f8e94`；608 个源码文件、22 个控制器模块固定绑定，15 份迁移与既有源码逐字相同。静态复审与本地纯测试不构成六案现场接受。

</details>

</details>

</details>

</details>

</details>

</details>

</details>

</details>

</details>

</details>

</details>

</details>
