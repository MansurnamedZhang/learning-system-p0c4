# Task6 正向快照依赖组件

## 当前检查点 · 10-24 03:45:20快照（非实时）· 返回16/16，正式接受0/16

截至 **2026-10-24 03:45:20（北京时间）** 的固定检查点（**非实时状态**），公开源码 **2e5026d78e81e57c3c8cbc2696dee08d188d4d6a** 的同源十六案已全部 **16/16 执行返回**，串行控制器自然退出 0。逐项核对十六行返回记录、既有五阶段审查与 80 份阶段索引引用哈希，记录均为 **test_exit=0、1 passed／0 failed／0 ignored**；本次文档准备未重跑阶段或内层原件。

在同一 **2026-10-24 03:45:20（北京时间）** 快照中，**正式 core／P6 最终原件接受仍为 0/16**。十六案执行返回不替代核心原件、业务语义、P6 完整副本／交付证据与独立审查；**16 项均待正式整链证明，Task6 继续 OPEN**。

截至 **2026-10-24 03:45:20（北京时间）**，当前 source2e 的**有限四门尚未执行，新取消组件尚待执行与独立闭合**，未把未来预期写成实测。历史源码 **592639f** 的 15/16 五阶段返回、唯一 P6 正式接受 1/16，以及取消组件 2/0/0 和既有有限回归，保留原来源与范围，**不转计当前源码**。

本次只更新文档与路线图，**应用源码改动为 0**。已公开的三叶窄修、原 15 秒排水守卫、权限、预算和精确历史断层断言保持；CBE 正文通过但后置读回失败的批次继续保留 HOLD、不重放。此前七项返回快照、启动时 0/16 与更早失败均作为有日期的历史记录保留。

**Task6、P0-C4 整关、Task7–9、独立备份存储、CompleteBackup 与生产仍 OPEN**。Task5 既有结论、D1 分项证据与 Task8 历史失败保持。私有原件、数据库内容、凭据与完整 inspect 不上传 GitHub。

详见[资源保护与整链待核记录](task6-resource-guard-revalidation.md)。

<details><summary>此前检查点（历史范围，原文保留）</summary>

### 2026-10-24 02:25:54 七案执行返回检查点（历史原文）


## 当前检查点 · 10-24 02:25:54快照（非实时）· 返回7/16，正式接受0/16

截至 **2026-10-24 02:25:54（北京时间）** 的固定七案快照（**非实时状态**），公开源码 **2e5026d78e81e57c3c8cbc2696dee08d188d4d6a** 的同源十六案中，本检查点已核对前七项 **7/16 执行返回**。本检查点仅选择 history-gap、derived、effects、leases、receipt-failure、retry-limit、P1 七案；逐案重新核对五阶段原件及收据，均记录 **test_exit=0、1 passed／0 failed／0 ignored**。后续场景即使返回，也不加入这份固定快照。

在同一 **2026-10-24 02:25:54（北京时间）** 快照中，**正式 core／P6 最终原件接受仍为 0/16**。五阶段返回及测试正文回执不替代核心原件、业务语义与 P6 全套副本／交付证据的独立接受。已返回七案待 core 审查，其余九案未纳入本检查点；**Task6 继续 OPEN**。

截至 **2026-10-24 02:25:54（北京时间）**，当前 source2e 的**有限四门与新取消组件仍待执行／独立闭合**，没有将未来预期计作实测。历史源码 **592639f** 的 15/16 五阶段返回、唯一 P6 正式接受 1/16，以及取消组件 2/0/0 和既有有限回归，保留各自原来源与范围，**不转计当前源码**。

本次只更新文档与路线图，**应用源码改动为 0**。现已公开的三叶窄修保持：fixture 在 direct runtime pool 关闭后释放最后句柄，history-gap 的封存角色配置沿用既有严格读取合同及回归；原 15 秒排水守卫、权限、预算和精确历史断层断言不变。此前 CBE 正文通过但封存配置后置读回失败的批次仍保留 HOLD，不重放；启动时 0/16 和更早失败均作为有日期的历史记录保留。

**Task6、P0-C4 整关、Task7–9、独立备份存储、CompleteBackup 与生产仍 OPEN**。Task5 既有结论、D1 分项证据与 Task8 历史失败保持。私有原件、数据库内容、凭据和完整 inspect 不上传 GitHub。

详见[资源保护与当前阻断记录](task6-resource-guard-revalidation.md)。



### 2026-10-24 01:27:30 启动检查点（历史原文）


## 当前候选 · 2e5026d 窄修已审，全新十六案待补验；正式接受 0/16

当前公开代码候选为 **2e5026d78e81e57c3c8cbc2696dee08d188d4d6a**。相对此前公开的 592，只同步三份指定源码：history-gap fixture 在 direct runtime pool 的 close().await 后、原 quiesce 前增加三行注释与 **drop(runtime)**；history-gap 封存配置读回改用已有角色配置读取合同；现有测试叶补相应回归。其余 **600 份原源码**及 15 份迁移保持；21 个 controller 的集合不变，仅 full_import.py 一份内容调整，不全量覆盖既有公开文档转换。

截至 **2026-10-24 01:27:30（北京时间）**，新同源十六案串行补验已从 history-gap 启动，首项仍在运行，尚无完成回执或正式接受，当前 **0/16**。启动与阶段记录不替代逐案原件接受；源码独审与本地回归也不转作 native 信用。

此前 CBE 批次的首个 history-gap 原收据记录正文 **test_exit=0、1 passed／0 failed／0 ignored**，但 driver 返回 **FAILED_UNUSABLE**，ordinal 043 exec 退出 1，串行 HOLD、completed 为空，后十五案未启动。该失败批次保留、不重放，正文通过不等于整案通过；CBE 正式接受仍为 **0/16**，不转授当前新源码。

后置失败已定位为读取合同冲突：history-gap 分支读取实际已封存的 **0444 角色配置**时，误用只接受 0600 的证据读取器；本次仅为该配置改用常规分支已有的严格配置读取器。proof／inventory 等原 0600 要求和后续摘要、版本、构建、目标身份检查保持。实际模式取证与局部重放支持这一定位；fake OS 重放不是新 Linux／native 补验，窄修仍待新批次验证。

新配置读取修复已有独立本地 Windows 回归 **45 通过、3 项 Linux Root 专项跳过**，不称这些专项已运行。此前 fixture 句柄释放改动保持原 15 秒排水守卫、业务权限、预算和精确历史断层断言；临时 lease 只保留四个自有值，不持有池或连接。

历史源码 **592639f** 的 15/16 五阶段返回、P6 正式接受 1/16、十四项 core 待核，及取消组件 2/0/0、严格 Rust 553 通过／136 ignored、Root Python 322 通过／2 跳过、FS 三项、OS 三项与普通 DAC 一项，均保留原实际来源和范围，不计作当前新源码的新跑或接受。

**Task6、P0-C4 整关、Task7–9、独立备份存储、CompleteBackup 与生产仍 OPEN**。Task5 既有结论、D1 分项证据与 Task8 历史失败记录保持。仅同步已审窄修和准确进度，私有原件、数据库内容、凭据与完整 inspect 不上传 GitHub。

详见[资源保护与当前阻断记录](task6-resource-guard-revalidation.md)。



## 中间进度 · 592639f 返回 15/16，正式独核 1/16；历史断层 HOLD

源码 **592639fc83fa88ebb366e83ee26344109eaabbe8** 的 16 项同源原生计划中，已有 **15 项返回五阶段通过记录**；正式原件接受仍为 **1/16**，仅 P6。其余十四项待核心原件闭合与逐案接受，十五项 core 独立审查尚未接受。执行返回通过与正式验收分别计数，Task6 仍 OPEN。

避开外部整点同步后，十项新场景依次运行到 snapshot-queued-zero 并返回通过。最后的 **snapshot-history-gap 已进入测试正文**，以 INVALID／UNKNOWN 和 EXACT_TEST_STDERR 失败，continuation 保留 HOLD 并停止，不自动重放；这次不是资源保护再次失败，也不是测试尚未开始。粗粒度诊断尚未确定具体拒绝点，不能据此宣布历史断层守卫已验收。

诊断检查点截至 **2026-10-23 22:50:17（北京时间）**：源码 **dfea6c63b61791fc1e3f534ea329ad0f7b84e12c** 的新批次已复现失败。该版本仅增加 24 个固定标记（48 行插入），去除标记逐字恢复 592；SIGNAL、FIRST_JOB、WAIT、DIRECT_POOL、CLAIM、QUIESCE、REFRESH 均有完成标记，最后出现 **PREPARE_RESULT_EXPECTED_BEFORE**，未出现对应 AFTER。当前仅能确定：**捕获准备返回结果与原测试预期不匹配，确切原因待分类**；这不是确认唯一根因，也没有放宽原断言。

进一步分类诊断源码 **8aadcdb21b36bd5ca0d91fb07508b7fce157fb88** 只在同一 cfg(test) 函数的 result.await 后增加八类固定分类，Invalid／Capacity 仅输出静态原因的 SHA-256；原 expect 不变。九项本地 Rust 参数探针通过，不能代替产品正文或 native 运行。新分类批次在本检查点处于 prepare，尚无结论。dfea 与 8aadc 的正式接受均为 **0/16**，不继承 592 的接受或返回记录；业务条件、权限、预算保持。本次仅更新文档，公开源码仍保持 592。

592 的 P6 已核对 4,175 份原件、八帧日志与 CUT、四文件交付及恢复后 64 表屏障；交付沿用原管理事务，未重连运行角色、未访问源端或重复发布。该结论仅限私有单机演练，不签发 CompleteBackup 或生产资格。

同源有限回归保留原范围：严格 Rust **553 通过**，**136 项 ignored 未执行**；Root Python **322 通过、2 项跳过**；文件系统三项、OS 三项及普通用户 DAC 一项通过。新取消组件批次 **2 passed／0 failed／0 ignored**，244 份原件与 228 次 CLI 经 Root 及独立审查通过；仅计组件，不增加 native 接受项。

此前资源保护中止已独立定位为同主机外部整点同步引起的一处既有网络变化。后续只在 UTC 每小时第 5–50 分钟且固定同步任务空闲时启动新场景，有界等待；完整旧资源保护、原测试预算及应用守卫保持不变。旧失败与 HOLD 原件保留，不复用失败批次。

**Task6、P0-C4 整关、Task7–9、独立备份存储、CompleteBackup 与生产仍 OPEN**。Task5 已有结论及 Task8 历史全量回归失败记录保持；诊断标记、有限回归、组件回执和执行返回均不替代整关验收。

详见[资源保护与当前阻断记录](task6-resource-guard-revalidation.md)。原始数据库、凭据、完整 inspect 及私有证据不上传 GitHub。

<details><summary>此前检查点（历史范围，原文保留）</summary>

## 当前进度 · 592639f，同源独核 1/16，Task6 OPEN

源码 **592639fc83fa88ebb366e83ee26344109eaabbe8** 的同源独立验收当前 **1/16**：P6 已按原始证据接受；五项场景已返回通过记录，其余四项仍待核心原件审查。Task6 仍 OPEN。

P6 实际正文 1 passed / 0 failed / 0 ignored，4,175 份原件、8 个日志帧与 CUT、4 个交付文件及恢复后 64 张表已交叉核对；交付沿用已有管理事务，未重连运行角色、未访问源端或重复发布。结论仅限私有单机恢复演练，不授 CompleteBackup 或生产资格。

同源有限回归已独核：严格 Rust 553 通过、136 项 ignored 未执行；Root Python 322 通过、2 项跳过；文件系统三项、OS 三项及普通用户 DAC 一项通过。取消组件的当前版本补验仍待完成。

第六项补验结束时，同主机外部同步容器出现并改变既有网络，完整资源保护检查正确中止。旧失败批次保留、不重放。新调度仅在 UTC 每小时第 5–50 分钟、固定同步任务空闲时启动新场景；等待有界，资源保护条件、原测试预算及应用源码均保持。已核对的五项保留各自原始身份，后十一项使用全新批次串行补验；本地调度检查 21 项通过。

P0-C4 整关、Task7–9、独立备份存储、CompleteBackup 与生产仍 OPEN；Task8 既有全量回归失败记录保留。详见[资源保护排查与补验记录](task6-resource-guard-revalidation.md)。

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

## 历史检查点 · 309bf95（保持当时范围）

Task6 候选代码 309bf95 已冻结；最终同源 native 当前 0/16（五项基础＋十一项快照）。同源有限前置已按本轮实际原件独审通过（非native）；Task6、Task7–9、C4、CompleteBackup 与生产继续 OPEN。

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

候选已接通完整C（C0＋D1）角色／模式合同、不透明源端效果捕获与复制、归一化后全表核对、原持锁副本交付和隔离收尾。代码与定向纯测不替代最终同源16项现场验收。

正常公开恢复资格仍必须持有真实 CompleteBackup。本轮私有管理流程以同源 SourceLocalPin／不透明效果对象进行单机 rehearsal；不签发 CompleteBackup，不开放生产恢复或服务。缺少有效效果证明仍 Unknown/HOLD，禁止回源猜测、领养或重发。

旧基础5/5仍分属509956e三项与0df5128两项；D1 PG24＋旧publisher3保留原分项信用，不能拼接成新候选的同源通过。失败、UNKNOWN与HOLD记录保持原样。

以下此前分源／组件结果按原源码与批次保留，不提升旧证据或生产资格。

## 2026-10-23 · D1 阅读副本发布与持久收束组件

本轮打通同一原操作内的领取、持有 Prepared 值的快照写入、runtime 原事务业务发布、原 COMMIT 成功知识，以及 `BUSINESS_COMMIT_KNOWN` → `WRITE_CLOSED` 的持久收束。没有从路径、返回的 staged 对象或日志内容重建写入能力；旧 stage-only 行为继续保留。

新增只读受限观察函数，由严格 owner installer 首装并在重复调用时只核验；仅新增 runtime 对该函数的 EXECUTE，现有表权限、迁移与旧 broker/C0 接口保持原样。before 核对原 token、expiry、request 后才锁定真实行；after 核对清空的 lease 与 receipt.stage_token，并依赖同一原事务中的 before 证明和持续行锁，不能独立重建旧 expiry。现场分别验证错误租约、签名／源码漂移、runtime WITH GRANT OPTION 的 ACL 漂移，以及非 owner 安装的拒绝。该函数本身不构成写入或 COMMIT 权威。

原绝对 deadline、累计预算、单次签发状态和双 OFD SH 持有贯穿事务、阻塞 verifier 与 append 的实际收束。大段笔记的计划摘要改为受计量的线性输出与 hash，保持 canonical 摘要字节不变；空间集合的读取、比较、搬移也先付原预算。BEGIN 初始化失败的原事务会等待显式 rollback；COMMIT 未知或晚到不会用新 SELECT 补出成功知识。

### 实际验证

| 项目 | 结果 |
| --- | --- |
| Linux | 格式、三 crate all-target 检查及严格 Clippy 通过；36 项纯测试实际通过。ignored 列举与 no-run 不算现场执行。 |
| D1／既有组件 | 六项 D1、三项受限观察接口专项、六项 P1、六项 C2、三项 C0，共 24 个独立空库，每项 1 passed / 0 failed / 0 ignored。 |
| 旧 public publisher | 独立三个空库验证撤权失败外壳、过期 token 与 winner 发布，以及裁剪副本交付；使用实际 snapshot_jobs ELF，三项各 1/0/0。 |

正例独立核对六条有序持久记录、完整 job/outbox/request/key/receipt 前后关系，以及包内每个文件的实际长度和 hash。负例包括真实撤权／计划变化、canonical renew 的到期字段漂移与取消、主动丢弃已执行 COMMIT 的成功知识、实际 pending 前缀拒绝，以及已执行 COMMIT 后和 append 的返回暂停点取消与 deadline。它们不代表自然租约接管、网络丢包或磁盘 fsync 故障均已验证。

源码来源 `1aa20a454bf72e737c40407ada37bb8b8f246feb` 是开发仓库身份；公开仓库有独立连续快照历史。580 个 Git archive 成员正文与受验包逐字相同，未混入 A2 草稿。Linux 结果 `4ec4e3ec1a7cb1284763b9125e89f96cff8785fa5df4576a71bd8631c8b11d66`；24 项 PG 结果 `11ec9883668ef9c3497359789f7d71692ab6beaa7edec02fd736d5418fa2f755`；旧发布回归结果 `dab60e22ae0371069e08baef9e477c6a165d46ae6b15c7503613a2a0e512e6aa`。每轮新资源和证据独立留存，成功 PG 已精确停机留卷；此前失败／HOLD 记录保留。

### 适用边界

这仍是私有阅读副本发布组件，尚未授予正式 `NoEffect`／`AlreadyCommitted` 分类或生产恢复权威。BUSINESS_COMMIT_KNOWN／WRITE_CLOSED 记录不是 `CompleteBackup`；本轮也没有宣称最终同一源码的恢复归一化与完整校验整链通过。基础恢复 5/5、Task2/3/5 的既有限定结论保持原范围，Task6/C4 OPEN。

源码：[publication.rs](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/writer/publication.rs)、[jobs.rs](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot/jobs.rs)、[writer_tests.rs](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/writer_tests.rs)。


以下保留此前分项记录，历史结果与范围不作覆盖。

## C2+D0：有限首写组件，Task6 / C4 保持 OPEN

本片将真实非空 ReadingCopy 的首写接在原 P1 操作内。它只形成私有 `PACKAGE_STAGED` 事实，不代表完整 C2/D、业务发布或生产接入。实现来源为 `c4cf09afeb8e5a8140941353502c1022c4369e5f`（开发仓库来源身份；公开仓库使用独立的连续快照历史）。本次冻结 13 个源码路径；最新 Linux 和 PG 使用该 R5 源码，OS 文件组件采用下述逐字等价的 R4 实测结果。当前 577 个 Git archive 成员正文与 R5 受验包逐字相同。

真实执行链为：

1. **真实队列与原 P1 claim。** 通过原域 API 创建非空阅读，调用原 `SnapshotStore::enqueue_export` 生成 request/key/outbox，再由原 `JobStore::dispatch_pending` 入队。原 supervisor 在登记、排队、spawn 之前，从同一 fresh Root custody 独立打开两份 SH：一份随真正 writer，另一份由原 `OwnerGuard` 保持至结果或错误实际收束。这些 OS guard 自身不签发业务权限。
2. **保留同一个原操作。** 原 P1 在真实 C LOGIN、同一事务中完成锁读、durable intent、canonical claim、after 核对、原 commit 已知成功及 durable committed 两帧；随后在结果发布前继续为私有 `Claimed`。它不从已发布或已关闭的 slot 续用能力，也不从 `JobLease`、JSON 或磁盘重建能力。原 single-flight、绝对 deadline、HOLD/poison、owner 和累计 budget 均不重置。
3. **由原运行角色准备副本，再锁读全绑定。** 使用真实受限 `learning_runtime` LOGIN，核对原始认证身份、session/current role、配置与实际 endpoint、database OID 和 postmaster，调用原 `SnapshotStore::prepare_export`，取得非空 ReadingCopy。调用者不能自报 copy、actor 或 payload；`include_personal=true` 的 exact 请求在 issue 与文件创建前拒绝。随后由原 C role helper 重新锁读完整 **13 列 job + 9 列 outbox**，用真实 DB clock 核对原 token、attempt、expiry、完整输入与原 claim/owner/slot 的绑定。
4. **sticky issue 先于首次文件写入。** 在原 slot 的短 gate 内，先永久设置 issued，再通过原 S2-B journal 与原共享 budget durable append typed `WRITE_AUTHORITY_ISSUED`。只有原 append 的已知 durable 返回和已收束的 rollback 才能形成私有 permit；锁不跨长 copy。append 失败或不明、取消、空目录、写失败或清理，都不能撤回 issued。
5. **真正 blocking copy，完整等待。** 私有 permit 移入实际 `spawn_blocking`，调用 custody 专用 `stage_copy_metered`。同一个 `SharedJournalBudget` 在首个 job-directory mkdir 前计长、检查文件边界并完成不可退款的保守工作预付款，覆盖有界 render、四个文件、hash、manifest 编码/解析和两轮原 verifier；预付款不是 actual I/O 计数。原 owner 完整等待已经启动的真实 handle；caller Drop、timeout 或取消 drain 都不会把 blocking writer 脱离 supervisor。额外 SH 覆盖 consumed writer 返回错误后的收束窗口。
6. **verifier 后只记录暂存观察。** 实际 `verify_snapshot` 核对包，Root custody 在相关边界重新核验。成功只追加独立 typed `PACKAGE_STAGED`，绑定原 issue/claim/permit nonce、manifest SHA 及 verifier 实际校验的 payload 文件 count/bytes。ReadingCopy 的 payload 为 `reading.md`、`reading.html`、`validation.json` 三个文件，`manifest.json` 另有已核验 SHA。私有 `StagedComponent` 保持 staged/SH，不暴露 guard 或 capture 权限。

这里**不签发 `WRITE_CLOSED(success)`、`BUSINESS_COMMIT_KNOWN`、NoEffect 或 AlreadyCommitted**，也不调用普通 export-failure 池的 autocommit 来补结论。未完成 publish、DB receipt 与 business commit，`PACKAGE_STAGED` 不能被解释为业务成功。失败保留 issued/HOLD，不返回正面结果。

### 实际验证与限定结论

| 项目 | 实际结果与适用范围 |
| --- | --- |
| Linux | 最新 R5 源码的三 crate all-target 类型检查、严格 Clippy 和格式通过；30 项纯测试通过。32 项 ignored、测试列举和 no-run 均不计为现场通过。 |
| Root 文件专项 | 采用已接受的 R4 批次 fdc60fff 的 10 项精确正文结果：三个无网络 actor 自然退出并精确移除，旧资源投影不变。R5 的全部 assets/core 依赖成员和两个实际文件专项 ELF 与 R4 逐字相同，独立核对后仅复用该文件组件证据。 |
| PostgreSQL 18 | 最新 R5 源码的六项 C2 首写用例、六项 P1 回归、三项 C0 回归，共 15 个独立空库，每项 1 passed / 0 failed / 0 ignored。 |

Linux 结果 `102c31b99d35ebb0b778ea92bf0d63c8f30ac89d49043dd5229dba8e9be6826f`；采用的 R4 文件结果 `bd797f6ec484976faaa8d6ad0ed6c68e5dbaf8a2da04a6c1d4ce65d5c0e2fcaa`；PG 结果 `e7e161bbbe0423f9d0b595f1f7bc4a55a2579860e8cc1b2c4cf05a632adf04d7`。脱敏原件与来源、授权、终止、旧资源投影已独立复核；本批 PG 停机留卷，旧失败/HOLD 资源保留。

R5 文件重跑 eade5627 的 10 个测试正文虽通过，但无关同步容器在运行期间启动，旧资源投影改变，因此该批保持 HOLD，不宣称操作验收通过。以上分项证据复用也不代表全项目最终同源恢复整链通过。

早期 Linux 闭包编译错误、跨库测试授权未收尾、负例直接变更 token 被真实状态机拒绝的失败记录均保留。最终负例使用原运行角色通过 canonical renew 制造实际 expiry 漂移，保留完整拒绝断言；这不是自然过期或 token 接管的现场验证。预算预付款为保守工作预算，不是实际 I/O 计数。

### 当前边界与源码导航

**Task6 / C4 继续 OPEN。** 完整 C schema 导出、production factory、最终同源 capture → 独立复制 → 恢复 → delivery 整链均未完成；本片也未完成 heartbeat/renewal、attempt2/3、revoke、最终 publish/receipt 或正面恢复分类。默认 worker 的业务生产入口没有因此启用。

以下使用绝对 GitHub/main 导航链接，避免文档发布目录中不存在 `crates/` 时的相对路径失效。main 链接仅作源码导航，受验版本仍以上述实际提交和冻结清单为准；发布时将核对 GitHub main 的实际文件正文。

- [原 owner、single-flight、deadline 与 SH 收束](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/history.rs)
- [原 P1 known-commit 与私有 Claimed 继续](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/transition.rs)
- [有限 issue → stage → PACKAGE_STAGED 接线](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/writer.rs)
- [Root OS custody 与真正 writer](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-assets/src/snapshot/custody.rs)
- [同一累计 budget 的首写预付款](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-assets/src/snapshot/copy_meter.rs)


本页记录可选组件的分项实现与验证，Task6 整体保持 OPEN。五项基础恢复场景已独立复核5/5，来源仍为三项509956e和两项0df5128；最终同源整链回归尚未运行。

## C1-P1：原事务中的持久化领取历史

源码 `a8cabaf2f6fb21d12b652456df44c0fbb496196f` 已实现限定分支：`snapshot_queued / attempt0 → snapshot_running / attempt1`。独立真实 C LOGIN 持有原顶层事务与锁，完整观察13个 job 字段和9个 outbox 字段；原日志追加者先持久化类型化 INTENT，再调用一次固定 claim 函数。校对完整前后状态、事务身份及数据库时钟边界，等待原 COMMIT 的已知成功后，由原追加者持久化 OUTCOME；只有原截止时间和发布栅栏仍有效时才返回私有记录。

本轮修复已启动磁盘追加在取消时脱离等待的竞态，并用 post-spawn/pre-join 与真实 blocking-return 双屏障验证。第二遍JSON编码在写入前执行计数上限。取消、超时、原提交知识丢失或结果追加失败时保持HOLD，不通过新查询、重开日志或补签恢复资格；相同请求/任务拒绝重放，共享累计预算不会重置。

Linux格式、db/backup两包all-target类型检查、严格Clippy和11项纯测试通过。六项真实PG18/Root测试与三项C0回归在九个独立空库各为1 passed / 0 failed / 0 ignored；第5项实际跨越10秒原截止时间。322份脱敏原件已核对。两客户端自然退出0后精确移除，PG一次TERM/wait0停机并保留网络/卷。原451容器、450网络、1500卷观测条目逐值不变；after另增本批停机PG/网络/卷各一项，完整before/after文件并不相同。

六个实现文件：[内部模块](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/mod.rs)、[原事务](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/transaction.rs)、[权限执行接口](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/catalog.rs)、[原操作监督与历史](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/history.rs)、[固定状态变更](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/transition.rs)、[真实测试](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/transition_tests.rs)。权限SQL、15迁移、64表和C0公开只读API保持；Linux初次编译失败及后续一行兼容修复均保留记录。

实际结果摘要 `261c9d039c7d442f3494c439d8e3fea700b6ddc546342b89247825505a9106fb`；Root限定接受摘要 `10305b1e35c1bdfd38e0122d6017ac12a9eda21ce6cc888e9732409039baeb1a`。编译包 `cab89f03e27bf1248eed28d014f41b65fdd6b222f23a0bbe3f833ceb8fd616eb`，572文件。提交的Git archive全部成员正文与测试包一致，六个源码Git blob与测试正文逐字一致。公开快照仍显式保留先前0df测试夹具覆盖，不能称最终同源整链。

本片没有生产准入工厂。INTENT来源字段为 `UNENROLLED_COMPONENT_EXTERNAL_RUN_ONLY`，执行绑定摘要仅是外部引用，不能转换为生产构建、租约、写入许可、NoEffect或AlreadyCommitted。私有tmpfs上的真实同步/读回不证明断电持久性或独立备份；前缀拒绝、私有阶段屏障与主动丢失提交知识也不等同网络COMMIT确认丢失或所有fsync故障。中间Docker inspect和含合成凭据SQL未导出原文，断言/脱敏RPC记录不等同独立系统调用追踪。

C1 阶段后的首写与文件观察进展见上方 C2+D0 记录。完整业务发布、形式化效果分类、完整 C schema/角色导出合同和最终同源恢复归一化仍待完成；Task6 OPEN，基础恢复 5/5 与 Task2/3/5 的限定结果不变。

## 独立权限 profile 与回滚观察组件 C0

C0 使用独立真实 LOGIN、原顶层 READ COMMITTED 事务和固定锁定观察 helper，完整读取13个 job 字段与9个 outbox 字段，共享累计预算；公开事务 API 只有开始、观察和 rollback。它核对原三个 canonical 函数的正文、owner、设置及 EXECUTE 边界，本片不调用 claim/renew/fail，不产生业务历史、lease、写入许可、NoEffect 或 AlreadyCommitted。

权限检查限于当前数据库全 namespace 的直接 broker ACL、有效应用/序列权限，以及 pg_authid/pg_shadow 表与列的有效 SELECT 元数据。PUBLIC MAINTAIN、敏感目录读取和取消后的拒绝复用均已在真实 PG 验证；这不等于全部系统 PUBLIC ACL 或特权 builtin 白名单。源端两次最早预检均拒绝 C0 helper 的任意签名，安装此扩展的数据库仍不是可导出的完整 C schema。

七个源码路径：[可选权限 profile](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/extensions/snapshot-claim-v1/install.sql)、[显式 installer](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/mod.rs)、[catalog核对](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/catalog.rs)、[owned观察事务](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/snapshot_broker/transaction.rs)、[真实PG夹具](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/tests/snapshot_claim.rs)、[库入口](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-db/src/lib.rs)、[源端拒绝](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/main/crates/learning-backup/src/source.rs)。原15迁移、64表和 A1/S2-B 行为保留。

Linux 格式、两包 all-target 类型检查及严格 Clippy、6项纯测试通过。三项真实 PG 与一项源端拒绝用例在四个独立空数据库中各为1 passed / 0 failed / 0 ignored，210份原件已独立复核并由 Root 限定接受。原450容器、449网络、1499卷的观测条目逐值相同；after完整投影另含本批停机PG、保留网络和卷各一项，不能称两个完整文件字节相同。客户端自然退出0后精确移除，PG一次TERM及wait0后停机留卷。

正式七文件提交 `a6e23483f3078954465d08778d6afab05b4ff602`；实际结果摘要 `ba592f54b1a20b49633f597147d1bd1df623a455158b4d56c242462037e44056`；Root限定接受摘要 `48ffb07ec73de17db1a41df698542c5050bce8855760332f7c2bca9592b30859`。Linux源包摘要 `aa2b954a95f04083adb4081c5d26bfeb42fbb97a0e79a9107097367570ee487c`（976a9cf加七路径，569文件，排除A2.1草稿）。Windows既有 backup all-target Clippy 101仍保留；本次Linux通过不改写旧记录。

上述检查属于 C0 组件验证，不增加基础恢复5/5，也不是最终同源整链。中间 Docker inspect 正文及含合成凭据的 SQL 未导出原文，相关断言与脱敏进程记录不等于独立系统调用追踪。该阶段之后的C1-P1验证见前文；Task6仍 OPEN。

## 数据库观察组件 A1

A1 为独立真实 LOGIN 提供受限、可回滚的锁定观察事务。它检查固定函数正文、owner、设置和应用范围权限，读取任务与 outbox 的完整类型化行，并共享有界读取预算。源端两次预检都会拒绝此可选 helper 的任意签名；目前安装了该组件的数据库不能作为完整备份源。

本轮修复了四处关系类型敏感的权限检查：显式 CASE 保证只在对应对象类型上调用权限函数。实际 PostgreSQL 测试包含表、索引、TOAST、分区表、视图、物化视图、序列以及五组逐项 GRANT/拒绝/REVOKE。函数正文与既有权限边界保持。

Linux 格式、类型检查、严格 Clippy 及两项纯测试通过。真实 PostgreSQL 三项精确测试以及源端拒绝测试均已实际通过；已完成独立原件复核和 Root 限定接受。测试同时覆盖真实 broker 登录、完整行与预算，以及行锁阻止并发 claim、rollback 后释放。它只提供观察事实，不签发 claim、文件写入、NoEffect 或恢复资格。

角色审计仍有范围边界：应用关系与列 ACL 不等于全部系统目录直接授权白名单。后续正向授权组件必须另行固定完整角色与 catalog 合同；本组件未部署到生产。

## Root 日志字节组件 S2-B

三文件字节后端实现仅从全新私有目录创建的追加链：原目录 FD、no-follow 与单链接检查、共享预算、前缀身份和摘要、临时写入、文件同步、原子不覆盖改名、父目录同步和原 FD 读回。错误或未知状态后不重新打开、领养、重放或修复旧链。

Linux 格式、类型检查、严格 Clippy 与三项纯测试通过。8 项 Root 文件专项均实际通过，并完成独立原件复核及 Root 限定接受。它尚不是具有业务含义的 claim 历史，也不能单独提供 NoEffect 或 AlreadyCommitted 结论。

## 当前下一门（候选，native 0/16）

1. 保持309bf95同源包与实际ELF绑定；已闭有限前置不授native信用。
2. 在固定完整C profile运行五基础＋十一快照场景，逐case自然1/0/0、原件、隔离停机与独审闭合，未知保持HOLD。
3. Root限定接受后仅更新Task6代码＋规定单机结论；Task7/8/9、C4、CompleteBackup与生产仍OPEN。

正常公开恢复资格仍必须持有真实 CompleteBackup。本轮私有管理流程以同源 SourceLocalPin／不透明效果对象进行单机 rehearsal，不能借 Pin、组件回执或测试通过生成 CompleteBackup，也不能开放生产恢复或服务。缺少有效效果证明时仍 Unknown/HOLD，禁止回源猜测、领养或重发。

## 本轮验证与来源

| 组件 | 固定源码和实际结果 | 限定结论 |
| --- | --- | --- |
| C1-P1 | 源码a8cabaf；Linux严格检查、11项纯测、六项真实PG/Root＋三项C0回归；322份原件 | 私有fresh领取与持久历史；无生产准入/效果分类/最终整链资格 |
| C0 | 源码a6e2348；Linux严格检查、6项纯测、真实PG三项＋源端一项；210份原件 | rollback观察；非claim/业务历史/NoEffect/导出资格 |
| A1 | 源码9944c02；Linux两项纯测试；真实PG三项、源端拒绝一项；214份闭合原件 | 可选观察组件；非导出/claim/写入资格 |
| S2-B | 源码976a9cf；Linux三项纯测试、八项Root文件测试；161份闭合原件 | 新鲜私有日志字节存储；非业务历史/正向效果证明 |

A1 Linux源码包摘要：`d2fb225778101d81c515898de2c2cbfde25ebe8ac9438253fd24139582313567`；实际PG结果摘要：`84adfa2bd3a651f2022b16c26c56e9579397406c2fb27b5a78d1108db9f23b98`。

S2-B Linux源码包摘要：`40b65a81bed3817513aea58672a94bdc7c8715d4b94adb57077774395bd55007`。两个组件分别按冻结源码测试；当前公开快照还包含此前0df5128单文件测试夹具修订，不能因此称整个公开HEAD已完成同源全回归。

Root文件测试仅在新无网络容器内运行，测试镜像只读、capabilities全禁，私有/run为临时文件系统，只读挂载一份实际测量ELF。八项测试通过同一个密封内存FD执行。执行器对旧容器、网络、卷的完整观测投影进行校验，允许基线已存在且保持同一指纹的旧非健康检查执行进程；不保证旧进程内部静止或观测间无变化。仅自然成功且完成校验后按精确ID删除本次容器；此前失败批次保留。

A1中间Docker inspect正文和含合成凭据的SQL未导出原文；相关状态依据固定驱动的实际断言和已脱敏进程记录，不冒称另做了独立原始SQL或系统调用追踪。

</details>

</details>

</details>

</details>

</details>

</details>
