# 知织 · KnowWeave

## Task7 有限前置通过，整链阶段顺序修复中 · 2026-10-24 18:39:40（北京时间，固定检查点）

源码 `e2ce9af96d6fbfad8be851533232bce984984723` 的正式原生接受仍为 **0/6**，Task7 尚未完成。

本候选的 Linux 有限前置已独立核对：格式与严格 Clippy 通过，Rust 560 项实际通过、142 项 ignored 未执行，普通用户 DAC 一项通过；608 个源码 Git blob、22 个控制器 pin 与归档一致。该有限配方不是 Task8 的真实 PostgreSQL／四套升级验收。

首项新隔离整链在正式测试正文前被既有空根检查拒绝：Attention 原件预填充早于独立签发，触发 SNAPSHOT_PREPARED_PRIVATE_EMPTY_ROOT。旧失败与 HOLD 保留，不重放原批次；正在将固定原件创建移至签发通过后，空根检查及其余守卫不放宽。

源库与目标库均已精确停机留卷，仅移除自然结束业务后的已知控制容器。完整运行前 C618／N586／V2078 投影保持；未将准备失败写成业务用例通过。候选实现与有限前置的信用均不转授尚未执行的新修复源码。

Task6 私有单机 16/16 历史结论保持。Task7–9、P0-C4 整关、独立备份存储、CompleteBackup、公开恢复与生产仍 OPEN。当前仅同步候选分支 `feat/task7-attention-recovery`，未合并 main、未部署。

---

<details><summary>此前检查点（保留当时来源与范围）</summary>

## Task7 实施中 · 2026-10-24 15:49:42（北京时间，固定检查点）

Task7 已进入完整 Attention 场景与故障注入实现，正式接受 **0/6**。当前公开的是测试基线检查点 `2ced6366a23fcc472d7b6995cddc474194313d6b`，并非 GREEN 完成版本。

全新 Linux／PG18 基线已执行真实捕获与导入，在完成全部 **64 表**的归一化前校验后，实际发现 H@1/H@2 均为 0，触发完整性守卫。精确测试退出 **101，0 passed／1 failed／0 ignored**；独立审查确认这是缺失 Attention 行为的预期 RED。失败回执与 HOLD 保留，未转为通过。

正在实现：完整历史／当前知识图、原件、Reading／Release／权限语义比较，以及缺失原件、损坏 dump、中断传输、未知效果、根目录替换五项失败注入。固定原件合同为 5 份物理文件、6 条逻辑资产，尚未取得这些 GREEN 用例的实际通过证据。

Task6 在源码 `2e5026d` 上的代码与私有单机 16/16 限定完成记录保留。Task7、Task8、Task9 与 P0-C4 整关仍 OPEN；当前没有独立备份存储，不签发 CompleteBackup 或公开恢复实例，不代表生产部署。后续证据按各自源码、编译产物和全新隔离资源记录。

详见[Task7 验收清单](docs/p0c4-completion-acceptance.md)。

---

## Task6 代码与私有单机补验限定完成 · 10-24 07:39:32（非实时）

截至 **2026-10-24 07:39:32（北京时间）** 的已封存验收记录（**非实时状态**），Task6 仅在**代码与私有单机整链补验范围完成**。公开源码 **2e5026d78e81e57c3c8cbc2696dee08d188d4d6a** 的五项恢复基础场景与十一项快照场景，合计 **16/16 项同源原生用例**经原始正文、core／P6 证据与独立审查接受；每项均为 **1 passed／0 failed／0 ignored**。历史源码的接受记录不转计本轮。

同源 P6 核对 **4,367 份实际导出原件**、8 个日志帧与 CUT、4 个交付文件、原管理事务持有的交付和恢复后 **64 表屏障**；原副本、独立第二副本与第三交付副本按完整字节及实际元数据闭合。此为私有 current-pin 单机验证，未补造生产信任文件、公开恢复资格或独立存储证明。

同源有限回归已核：严格 Rust **553 通过**、**136 项 ignored 未执行**；Root Python **328 通过、2 项跳过**；文件系统三项、OS 三项及普通用户 DAC 一项通过。新取消组件 **2 passed／0 failed／0 ignored**，按本轮实际编译、ELF 测量与资源收尾原件接受；组件结果不增加十六案计数。

恢复归一化核对六张任务相关表及全部 13 列：保留尝试次数与终态结果、清除旧租约，三次上限不增加重试机会；未知外部效果保持 UNKNOWN。排队零次、历史断层拒绝、各 P1–P6 场景按各自原件范围记录。原资源保护、权限、预算及排水守卫保持；此前失败与 HOLD 原件不重放、不改写为成功。

**P0-C4 整关、Task7–9、独立备份存储、CompleteBackup、RecoveredInstance 与生产仍 OPEN**。Task2／Task3／Task5 既有结论、D1 分项与 Task8 历史失败保持；正常公开恢复仍必须取得 CompleteBackup。本次仅同步文档与路线图，应用源码改动为 0；私有原件、数据库内容、凭据与完整 inspect 不上传 GitHub。

本轮最终同源收尾索引 SHA-256：`6fff3afa3d6ffea8b6a5faf0e8d5d4705e9f68e93301a0a5b8cc90015b41a165`。

详见[资源保护与整链验收范围](docs/task6-resource-guard-revalidation.md)。

<details><summary>此前检查点（历史范围，原文保留）</summary>

### 2026-10-24 03:45:20 十六案执行返回检查点（历史原文）


## 当前检查点 · 10-24 03:45:20快照（非实时）· 返回16/16，正式接受0/16

截至 **2026-10-24 03:45:20（北京时间）** 的固定检查点（**非实时状态**），公开源码 **2e5026d78e81e57c3c8cbc2696dee08d188d4d6a** 的同源十六案已全部 **16/16 执行返回**，串行控制器自然退出 0。逐项核对十六行返回记录、既有五阶段审查与 80 份阶段索引引用哈希，记录均为 **test_exit=0、1 passed／0 failed／0 ignored**；本次文档准备未重跑阶段或内层原件。

在同一 **2026-10-24 03:45:20（北京时间）** 快照中，**正式 core／P6 最终原件接受仍为 0/16**。十六案执行返回不替代核心原件、业务语义、P6 完整副本／交付证据与独立审查；**16 项均待正式整链证明，Task6 继续 OPEN**。

截至 **2026-10-24 03:45:20（北京时间）**，当前 source2e 的**有限四门尚未执行，新取消组件尚待执行与独立闭合**，未把未来预期写成实测。历史源码 **592639f** 的 15/16 五阶段返回、唯一 P6 正式接受 1/16，以及取消组件 2/0/0 和既有有限回归，保留原来源与范围，**不转计当前源码**。

本次只更新文档与路线图，**应用源码改动为 0**。已公开的三叶窄修、原 15 秒排水守卫、权限、预算和精确历史断层断言保持；CBE 正文通过但后置读回失败的批次继续保留 HOLD、不重放。此前七项返回快照、启动时 0/16 与更早失败均作为有日期的历史记录保留。

**Task6、P0-C4 整关、Task7–9、独立备份存储、CompleteBackup 与生产仍 OPEN**。Task5 既有结论、D1 分项证据与 Task8 历史失败保持。私有原件、数据库内容、凭据与完整 inspect 不上传 GitHub。

详见[资源保护与整链待核记录](docs/task6-resource-guard-revalidation.md)。



### 2026-10-24 02:25:54 七案执行返回检查点（历史原文）


## 当前检查点 · 10-24 02:25:54快照（非实时）· 返回7/16，正式接受0/16

截至 **2026-10-24 02:25:54（北京时间）** 的固定七案快照（**非实时状态**），公开源码 **2e5026d78e81e57c3c8cbc2696dee08d188d4d6a** 的同源十六案中，本检查点已核对前七项 **7/16 执行返回**。本检查点仅选择 history-gap、derived、effects、leases、receipt-failure、retry-limit、P1 七案；逐案重新核对五阶段原件及收据，均记录 **test_exit=0、1 passed／0 failed／0 ignored**。后续场景即使返回，也不加入这份固定快照。

在同一 **2026-10-24 02:25:54（北京时间）** 快照中，**正式 core／P6 最终原件接受仍为 0/16**。五阶段返回及测试正文回执不替代核心原件、业务语义与 P6 全套副本／交付证据的独立接受。已返回七案待 core 审查，其余九案未纳入本检查点；**Task6 继续 OPEN**。

截至 **2026-10-24 02:25:54（北京时间）**，当前 source2e 的**有限四门与新取消组件仍待执行／独立闭合**，没有将未来预期计作实测。历史源码 **592639f** 的 15/16 五阶段返回、唯一 P6 正式接受 1/16，以及取消组件 2/0/0 和既有有限回归，保留各自原来源与范围，**不转计当前源码**。

本次只更新文档与路线图，**应用源码改动为 0**。现已公开的三叶窄修保持：fixture 在 direct runtime pool 关闭后释放最后句柄，history-gap 的封存角色配置沿用既有严格读取合同及回归；原 15 秒排水守卫、权限、预算和精确历史断层断言不变。此前 CBE 正文通过但封存配置后置读回失败的批次仍保留 HOLD，不重放；启动时 0/16 和更早失败均作为有日期的历史记录保留。

**Task6、P0-C4 整关、Task7–9、独立备份存储、CompleteBackup 与生产仍 OPEN**。Task5 既有结论、D1 分项证据与 Task8 历史失败保持。私有原件、数据库内容、凭据和完整 inspect 不上传 GitHub。

详见[资源保护与当前阻断记录](docs/task6-resource-guard-revalidation.md)。



### 2026-10-24 01:27:30 启动检查点（历史原文）


## 当前候选 · 2e5026d 窄修已审，全新十六案待补验；正式接受 0/16

当前公开代码候选为 **2e5026d78e81e57c3c8cbc2696dee08d188d4d6a**。相对此前公开的 592，只同步三份指定源码：history-gap fixture 在 direct runtime pool 的 close().await 后、原 quiesce 前增加三行注释与 **drop(runtime)**；history-gap 封存配置读回改用已有角色配置读取合同；现有测试叶补相应回归。其余 **600 份原源码**及 15 份迁移保持；21 个 controller 的集合不变，仅 full_import.py 一份内容调整，不全量覆盖既有公开文档转换。

截至 **2026-10-24 01:27:30（北京时间）**，新同源十六案串行补验已从 history-gap 启动，首项仍在运行，尚无完成回执或正式接受，当前 **0/16**。启动与阶段记录不替代逐案原件接受；源码独审与本地回归也不转作 native 信用。

此前 CBE 批次的首个 history-gap 原收据记录正文 **test_exit=0、1 passed／0 failed／0 ignored**，但 driver 返回 **FAILED_UNUSABLE**，ordinal 043 exec 退出 1，串行 HOLD、completed 为空，后十五案未启动。该失败批次保留、不重放，正文通过不等于整案通过；CBE 正式接受仍为 **0/16**，不转授当前新源码。

后置失败已定位为读取合同冲突：history-gap 分支读取实际已封存的 **0444 角色配置**时，误用只接受 0600 的证据读取器；本次仅为该配置改用常规分支已有的严格配置读取器。proof／inventory 等原 0600 要求和后续摘要、版本、构建、目标身份检查保持。实际模式取证与局部重放支持这一定位；fake OS 重放不是新 Linux／native 补验，窄修仍待新批次验证。

新配置读取修复已有独立本地 Windows 回归 **45 通过、3 项 Linux Root 专项跳过**，不称这些专项已运行。此前 fixture 句柄释放改动保持原 15 秒排水守卫、业务权限、预算和精确历史断层断言；临时 lease 只保留四个自有值，不持有池或连接。

历史源码 **592639f** 的 15/16 五阶段返回、P6 正式接受 1/16、十四项 core 待核，及取消组件 2/0/0、严格 Rust 553 通过／136 ignored、Root Python 322 通过／2 跳过、FS 三项、OS 三项与普通 DAC 一项，均保留原实际来源和范围，不计作当前新源码的新跑或接受。

**Task6、P0-C4 整关、Task7–9、独立备份存储、CompleteBackup 与生产仍 OPEN**。Task5 既有结论、D1 分项证据与 Task8 历史失败记录保持。仅同步已审窄修和准确进度，私有原件、数据库内容、凭据与完整 inspect 不上传 GitHub。

详见[资源保护与当前阻断记录](docs/task6-resource-guard-revalidation.md)。



## 中间进度 · 592639f 返回 15/16，正式独核 1/16；历史断层 HOLD

源码 **592639fc83fa88ebb366e83ee26344109eaabbe8** 的 16 项同源原生计划中，已有 **15 项返回五阶段通过记录**；正式原件接受仍为 **1/16**，仅 P6。其余十四项待核心原件闭合与逐案接受，十五项 core 独立审查尚未接受。执行返回通过与正式验收分别计数，Task6 仍 OPEN。

避开外部整点同步后，十项新场景依次运行到 snapshot-queued-zero 并返回通过。最后的 **snapshot-history-gap 已进入测试正文**，以 INVALID／UNKNOWN 和 EXACT_TEST_STDERR 失败，continuation 保留 HOLD 并停止，不自动重放；这次不是资源保护再次失败，也不是测试尚未开始。粗粒度诊断尚未确定具体拒绝点，不能据此宣布历史断层守卫已验收。

诊断检查点截至 **2026-10-23 22:50:17（北京时间）**：源码 **dfea6c63b61791fc1e3f534ea329ad0f7b84e12c** 的新批次已复现失败。该版本仅增加 24 个固定标记（48 行插入），去除标记逐字恢复 592；SIGNAL、FIRST_JOB、WAIT、DIRECT_POOL、CLAIM、QUIESCE、REFRESH 均有完成标记，最后出现 **PREPARE_RESULT_EXPECTED_BEFORE**，未出现对应 AFTER。当前仅能确定：**捕获准备返回结果与原测试预期不匹配，确切原因待分类**；这不是确认唯一根因，也没有放宽原断言。

进一步分类诊断源码 **8aadcdb21b36bd5ca0d91fb07508b7fce157fb88** 只在同一 cfg(test) 函数的 result.await 后增加八类固定分类，Invalid／Capacity 仅输出静态原因的 SHA-256；原 expect 不变。九项本地 Rust 参数探针通过，不能代替产品正文或 native 运行。新分类批次在本检查点处于 prepare，尚无结论。dfea 与 8aadc 的正式接受均为 **0/16**，不继承 592 的接受或返回记录；业务条件、权限、预算保持。本次仅更新文档，公开源码仍保持 592。

592 的 P6 已核对 4,175 份原件、八帧日志与 CUT、四文件交付及恢复后 64 表屏障；交付沿用原管理事务，未重连运行角色、未访问源端或重复发布。该结论仅限私有单机演练，不签发 CompleteBackup 或生产资格。

同源有限回归保留原范围：严格 Rust **553 通过**，**136 项 ignored 未执行**；Root Python **322 通过、2 项跳过**；文件系统三项、OS 三项及普通用户 DAC 一项通过。新取消组件批次 **2 passed／0 failed／0 ignored**，244 份原件与 228 次 CLI 经 Root 及独立审查通过；仅计组件，不增加 native 接受项。

此前资源保护中止已独立定位为同主机外部整点同步引起的一处既有网络变化。后续只在 UTC 每小时第 5–50 分钟且固定同步任务空闲时启动新场景，有界等待；完整旧资源保护、原测试预算及应用守卫保持不变。旧失败与 HOLD 原件保留，不复用失败批次。

**Task6、P0-C4 整关、Task7–9、独立备份存储、CompleteBackup 与生产仍 OPEN**。Task5 已有结论及 Task8 历史全量回归失败记录保持；诊断标记、有限回归、组件回执和执行返回均不替代整关验收。

详见[资源保护与当前阻断记录](docs/task6-resource-guard-revalidation.md)。原始数据库、凭据、完整 inspect 及私有证据不上传 GitHub。

<details><summary>此前检查点（历史范围，原文保留）</summary>

## 当前进度 · 592639f，同源独核 1/16，Task6 OPEN

源码 **592639fc83fa88ebb366e83ee26344109eaabbe8** 的同源独立验收当前 **1/16**：P6 已按原始证据接受；五项场景已返回通过记录，其余四项仍待核心原件审查。Task6 仍 OPEN。

P6 实际正文 1 passed / 0 failed / 0 ignored，4,175 份原件、8 个日志帧与 CUT、4 个交付文件及恢复后 64 张表已交叉核对；交付沿用已有管理事务，未重连运行角色、未访问源端或重复发布。结论仅限私有单机恢复演练，不授 CompleteBackup 或生产资格。

同源有限回归已独核：严格 Rust 553 通过、136 项 ignored 未执行；Root Python 322 通过、2 项跳过；文件系统三项、OS 三项及普通用户 DAC 一项通过。取消组件的当前版本补验仍待完成。

第六项补验结束时，同主机外部同步容器出现并改变既有网络，完整资源保护检查正确中止。旧失败批次保留、不重放。新调度仅在 UTC 每小时第 5–50 分钟、固定同步任务空闲时启动新场景；等待有界，资源保护条件、原测试预算及应用源码均保持。已核对的五项保留各自原始身份，后十一项使用全新批次串行补验；本地调度检查 21 项通过。

P0-C4 整关、Task7–9、独立备份存储、CompleteBackup 与生产仍 OPEN；Task8 既有全量回归失败记录保留。详见[资源保护排查与补验记录](docs/task6-resource-guard-revalidation.md)。

<details><summary>此前检查点（历史范围）</summary>

## 当前进度 · 592639f，源运行角色夹具修复，Task6 OPEN

当前源码 **592639fc83fa88ebb366e83ee26344109eaabbe8**（2026-10-23）。Task6 仍 OPEN，最终同源原件独立验收 **0/16**。上一版 d428b62 已完成四项隔离测试执行，第五项在源库派生结果夹具认证阶段失败；这些记录保留为历史范围，不转计新版本。

本次仅修正源端派生结果夹具的运行角色连接。原代码复制管理连接参数后只替换用户名，将管理密码带入运行角色的 TCP 连接，真实测试返回 SQLSTATE 28P01。修复使用该源 PG18 容器内既有 Unix socket 与独立运行角色连接参数，并核对实际角色及源库身份；不新增运行密码、不修改 HBA、权限或目标库准入。生产恢复逻辑和目标运行角色关闭条件不变。

下一步是同一源码的有限回归、五项基础与十一项快照隔离测试、原始证据独立核对及 Task6 收尾。测试执行通过、组件回归通过和最终独立验收分别记录；未执行项不计通过。旧失败和 HOLD 项目保持原样，不复用旧批次。

Task6、P0-C4 整关、Task7–9、独立备份存储、CompleteBackup 和生产仍 OPEN。单机隔离测试不会替代独立备份设备验收。

<details><summary>此前检查点与验证记录（历史范围）</summary>

## 当前进度 · d428b62，16 项重跑，Task6 OPEN

当前源码 **d428b6298acc8aad5434b44eda47466949d0a083**（2026-10-23）。Task6 仍 OPEN，新版本 16 项同源验收正在重新运行，正式接受 **0/16**；上一版 95930af 的 P6 原件已独核接受 1 项，保留为历史证据，不转计新版本。

上一轮第三项在测试正文启动前，被普通管理协议的 1 MiB 输入限制拒绝。原始报文复现为 1,053,020 字节，超出 4,444 字节，确认未发送到辅助进程。本次修复在同一持锁管理会话保存完整创建前快照，以 SHA-256 关联后续创建请求；完整创建后快照和资源核验仍保留，输入上限、五秒 RPC 时限及受控导入协议均未放宽。定向本地回归 112 项：97 通过、15 项跳过（14 项 Linux Root 条件、1 项历史私有帧未启用），包含当前故障原报文复现；这不是 Linux 原生验收。

新包只改变两份管理脚本并新增一份测试，全部 Rust 源码与 95930af 相同。Linux 将使用新项目、数据库、卷与子网重跑五项基础和十一项快照场景；旧失败、HOLD 和成功原件均保留。新版本的有限回归、原件独立复核及最终 Task6 收尾尚未完成。

Task7–9、P0-C4 整关、独立备份存储、CompleteBackup 和生产仍 OPEN。上一版 P6、管理事务 PG18 回归及 Task8 历史全量结果均保持原来的来源和范围。

<details><summary>此前检查点与原验证记录（历史范围）</summary>

## 当前进度 · 95930af 候选，正式验收 1/16，Task6 OPEN

候选源码 **95930af55e5ea3717c9d215911a23ba5a50f28c4**（2026-10-23）。Task6 仍 OPEN，最终同源验收正式接受 **1/16**；单机验证不能签发 CompleteBackup 或开放生产。

新 P6 精确测试正文已返回 **1 通过、0 失败、0 忽略**，527 项过滤，88.66 秒，stderr 为空；五个调度阶段均退出 0。原始文件已完整取回并经独立复核；4,070 份文件、八帧日志与 CUT、四文件交付、管理事务交付和最终 64 表屏障已核对，P6 正式计入 1/16。其余 15 项尚待全部闭合；前两项已返回完成记录待独核，第三项在签发准备阶段拒绝，队列已停止，正在只读定位。

本轮修复了恢复管理事务误调 runtime 专用授权函数的问题：私有管理路径在原事务中锁定相同授权行，未增加角色权限、重连运行角色或修改迁移。新 PostgreSQL 18 回归 **1/0/0**，实际原件已由 Root 复核；该回归所测两份 DB 源码与 95930af 完全相同。4c08d4 的严格 Linux 记录为 550 项 Rust 通过、136 项 ignored 未执行及普通 DAC 1 项通过；它保留自身来源，不冒充 95930af 的完整最终回归。

后续依次完成：剩余五基础／十项快照同源闭环、当前版本的有限回归与组件复核。旧失败和 HOLD 原件保留。Task7–9、P0-C4 整关、独立备份存储、CompleteBackup 与生产仍 OPEN；Task8 全量回归未通过的历史记录不被本轮覆盖。

<details><summary>此前检查点与原验证记录</summary>

以“块”为最小修订单元的个人学习与知识系统。当前实现是 Rust、PostgreSQL 18 与私有文件存储内核，包含内容修订、组合、个人阅读、关系审查、原件、持久 Worker、快照交换及正在实现的备份恢复。

**历史同步：2026-10-23 · Task6候选代码 309bf95，最终同源native 0/16。Task6、Task7–9与C4仍OPEN；未签发CompleteBackup，未开放生产。**

候选源码检查点：`309bf9580bc5179ed51a7eb8345047b3606214bb`；前提交 `625162207812d5b346792c1a21bb0398621c277f`。这是开发源码身份，公开仓库保持自己的快照提交历史，不把来源 ID 当作公开仓库同名提交。

309bf95有限前置已独审通过：新执行严格Rust 549通过、0失败，133项ignored未执行；Root Python 12模块317项（316通过、1项UID0 DAC跳过），ordinary Python DAC另1通过。两批新执行均使用309bf95包，自然退出0、精确移除本批容器且旧资源图不变。旧Root OS三项及取消组件仍保留原2741d3d源码与sealed ELF；本轮仅按另封239项DB/assets/core闭包保留其输入兼容，不声称完整backup或全381项Rust输入与2741d3d不变，也不声称新ELF或新OS/取消执行。这些均非native通过，accepted仍0/16；不授全774、Task8或生产资格。

309bf95仅修快照效果验证中的issue摘要接缝：WRITE_AUTHORITY_ISSUED按原producer对完整typed outbox求摘要，request-key继续独立核payload摘要；source binding、完整行、时钟、receipt及全部history守卫保持。新增回归以14份合成现场原件核完整8帧、CUT和inventory，并明确两种摘要不同、拒绝payload替代及payload不变的外层字段篡改。实际Linux先观察完整inventory回归RED，再以单表达式修复通过新回归与本轮有限前置；这不能替代native P6或Root持有FD验收。

新309bf95精确P6已进入body，退出101、0通过／1失败／0忽略（另523项过滤），诊断CAPTURE／IO／ERRNO_2，原因待查。本轮已实际绑定V2 source profile并通过原drain屏障；Worker COMPLETED仅为调度状态，不能替代P6通过。源与目标已停止并确认无exec，teardown错误为空；Root run capsule保留HOLD、不复用，同源native仍0/16。

此前6251622仅修full-C source journal观察接缝：初始化显式锁定C0D1 profile与固定contract，迁移安装已知完成后，由同源learning_admin固定查询broker OID并一次绑定；未绑定full-C、重复绑定或普通V1提升均拒绝。观察器严格核V2九字段及base四字段、原序canonical、阶段flags与digest续接，并分别保留outer journal hash和Ready base hash；旧V1与predump drain条件保持。该修复已通过本轮有限前置验证，尚不代表P6或native现场通过。

旧6251622精确P6已进入body，退出101、0通过／1失败／0忽略（另522项过滤），诊断CAPTURE／INVALID／UNKNOWN，原因待查。本轮已实际绑定V2 source profile并通过原drain屏障；Worker COMPLETED仅为调度状态，不能替代P6通过。源与目标已停止并确认无exec，teardown错误为空；Root run capsule保留HOLD、不复用，同源native仍0/16。

此前cb1bcc8仅修真实BudgetRunner接入Worker时的日志目录借用：MonitoredChild从原owned base runner取得logs，原预算、所有权和实际profile守卫保持。新增回归使用真实BudgetRunner与完整Worker构造核对原日志目录，并保留created／running profile和固定START顺序；本地有限78项通过，已并入本轮Root Python 309项实测。这些验证不能替代native现场通过。

旧cb1bcc8精确P6已启动并观察到所选test，但未取得主测试完成回执，退出码与通过／失败计数未知；driver记录actual-libtest／DRAIN_PREDUMP_BARRIER。真实Worker客户端退出0并返回COMPLETED调度状态，不代表P6完成；只读独审确认旧V1观察器拒绝V2 C0D1 journal wrapper，该修复当时尚未计入cb1bcc8。源与目标已停止且无exec，teardown错误为空；Root run capsule保留HOLD、不复用，同源native仍0/16。

此前05c7dd2仅修source companion名册比较：原有界读取、持有FD、预算、no-follow与权限守卫保持；实际名册先核长度，再sort_unstable后精确比较，不去重。新增六项测试覆盖六项名册全部720种排列、两类双项名册及缺件、额外项、重复和伪名拒绝；已并入本轮548项严格Rust实际结果，不能替代native现场通过。

旧05c7dd2精确P6已启动并观察到所选test，但未取得完成回执，退出码与通过／失败计数未知；driver记录actual-libtest／UNEXPECTED_ERROR。源与目标已停止且无exec，teardown错误为空；Root run capsule保留HOLD、不复用，同源native仍0/16。

此前90dcc39在已审的fresh full-C源库TEMP权限准备之上，仅补齐/var/lib/knowweave-c4与/run/kwc4两处全新容器父目录：先核目录、非链接及Root所有权，再固定0700并复核；不改mounted sfx、Rust守卫、迁移或C0/D1 profile。此次两叶修复已独审77项相关纯测及直接入口18项，不能替代native现场通过。

旧90dcc39精确P6已进入body，退出101、0通过／1失败／0忽略（另516项过滤），诊断CAPTURE／INVALID／UNKNOWN，原因待查。源与目标已停止并确认无exec，teardown错误为空；Root run capsule仍保留HOLD、不复用，同源native仍0/16。

此前02885c4真实迁移及C0/D1安装退出0并返回C4_TASK3_MIGRATED；其精确P6已进入body，退出101、0通过／1失败／0忽略（另516项过滤），诊断CAPTURE／IO／UNKNOWN并保留HOLD。后续只读原件显示/run/kwc4为Root 0755，与既有精确0700守卫不符；未据此伪造系统调用errno或给新源转授成功。

旧2741d3d首轮P6在source容器created／准备阶段因PG_BINDS_MODE失败，未进入业务测试；失败与HOLD原件保留。64755f6的R3仅修固定snapshot socket RW挂载的资源规范适配，缺失、null或空Binds均拒绝；不在控制capsule访问未挂载的宿主卷，不签发原FD或业务效果权威。旧64755f6 P6在独立消费者构建准备阶段失败并HOLD；后续原件明确其迁移安装被C0有效权限检查拒绝，未进入精确测试。

另两项formal quiesce取消组件实际2/0/0及Root OS三项继续保留原2741d3d源码、批次和sealed ELF。本轮309bf95只按独立239项DB/assets/core输入闭包保留有限兼容；不把修改过的backup验证代码或整套381项Rust输入标为与2741d3d相同，不借旧组件授新ELF执行、native场景或生产信用。

正常公开恢复资格仍必须持有真实 CompleteBackup。本轮私有管理流程以同源 SourceLocalPin／不透明效果对象进行单机 rehearsal；不签发 CompleteBackup，不开放生产恢复或服务。缺少有效效果证明仍 Unknown/HOLD，禁止回源猜测、领养或重发。

[Task6 实现、验证与剩余门](docs/p0c4-recovery-normalization.md)。

新增[快照首写与验证记录](docs/p0c4-snapshot-components.md)：C2+D0 已将原 P1 领取接到实际文件写入及 verifier，复用绝对 deadline、累计预算和原收束过程。实际新增六项 C2 PG 用例与九项回归通过；文件事实不代表业务提交、NoEffect 或 AlreadyCommitted。

**历史分源基础与旧S1记录（保持当时范围）：** 旧令牌、三次重试上限、未知快照副作用、非空派生结果/阅读校验及收据同步失败已完成真实隔离测试与独立复核，基础现场进度 5/5（前三项509956e，后两项0df5128）；snapshot 正向证据链和最终同源回归仍待完成，Task6整体保持OPEN。旧失败及 UNKNOWN 原样保留。 新增可选快照代理的写入禁用基础；Linux 5 项普通测试及 6 项 Root 权限测试通过，真实只读部署已有首个客户端以78拒绝执行，整体仍因旧网络保护检查HOLD，未授予部署通过；正向恢复仍待验。详见[快照代理](docs/p0c4-snapshot-broker.md)。

## 进度与文档

- [项目最新进度](docs/项目最新进度.md)：已运行结果、当前补验与未完成边界。
- [实施路线与系统架构](docs/KnowWeave实施路线与系统架构.md) · [离线 HTML 路线图](KnowWeave实施路线与架构.html)。
- [架构设计文档集](docs/architecture/README.md) · [离线 HTML 架构设计](KnowWeave架构设计.html)。
- [P0-C4 Task2、3、5 补验与收尾](docs/P0-C4补验与收尾.md)。
- [系统设计方案](docs/学习系统设计方案.md) · [批准记录与实施路线](docs/设计批准与实施路线.md)。
- [公开同步说明](docs/公开同步说明.md) · 文件与来源清单（私有或未公开参考）。

| 阶段 | 状态 |
| --- | --- |
| P0-A、B1–B4 | 各阶段隔离验收通过；非生产 |
| P0-C1、C2、C3 | 各阶段隔离验收通过；非生产 |
| P0-C4 | Task2/3/5保留原限定范围；Task6候选309bf95，最终同源native0/16（五基础＋十一快照）；旧5/5与D1组件为分源历史；Task6–9/C4/Complete与生产OPEN |
| P1-A / P1-B | HTTP、登录、浏览器内容工作台与学习闭环待建设 |
| P2 / P3 | 多领域演进及按需增强待建设 |

## 代码导航

| 目录 | 职责 |
| --- | --- |
| `crates/learning-core` | 内容、关系、审查、阅读、资产与作业契约 |
| `crates/learning-db`、`migrations` | PostgreSQL 持久化、事务、权限与追加迁移 |
| `crates/learning-assets` | 私有原件与安全目录、融合快照包 |
| `crates/learning-worker` | 持久任务与受限执行器 |
| `crates/learning-backup` | 维护窗、备份目录、目标绑定、受控导入、收据及恢复 WIP |
| `deploy`、`scripts` | 隔离验收配置与工具；具体授权与环境边界见各阶段文档 |
| `contracts`、`docs` | 版本化协议、设计、施工单与分项验证记录 |

## 构建与测试

需要 Rust 1.97 与锁定的 `Cargo.lock`。以下是构建命令，不代表本次文档同步重新执行了完整测试：

```sh
cargo fmt --all -- --check
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo test --offline --locked -p learning-core
cargo test --offline --locked -p learning-backup --lib
```

`--offline` 需要已缓存的依赖。数据库及 Linux/Docker 专项要求各自隔离环境、全新库/卷与明确角色，部分现场用例须显式运行；不能把编译、测试列举、Windows 本地通过或历史批次替代现场验收。先阅读对应边界与验证记录，不在已有业务库运行夹具、迁移或故障注入。

## 公开快照范围

本次从固定 Git 源码字节导出并更新文档。源码检查点属于开发仓库历史，仅作来源身份标识；公开仓库保留自己的连续快照提交历史。代码、迁移、协议和二进制资料保持原字节，公开文档中的本机路径、服务器连接及私有证据链接做脱敏处理。

凭据、课程原件、编译缓存、测试包、服务器原始日志与私有验收记录不在本仓库。分项审查与历史结果各有范围，不能称整个 HEAD、恢复能力、C4 或生产已验收。

</details>

</details>

</details>

</details>

</details>

</details>

</details>
