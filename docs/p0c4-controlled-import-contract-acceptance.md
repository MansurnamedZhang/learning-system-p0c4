# P0-C4 Task 1：客户端合同与合成工件采集验收

## Task6 最新状态（2026-10-03）：最终回归 3/3 已接受

固定合成 fixture 的累计现场场景 11/11 已接受；当前 17153d8 源码实际 7/11，历史四项保留 5619026 来源并经差异复审携带。新 dda83916 批次集成回归 37 通过 / 2 忽略、文档测试 exit0（0 可执行用例）、工作区严格 Clippy exit0，三门全部接受，not_run=[]。

完整返回、publisher 哈希重建、逐目标名称/源码/二进制摘要和独立精确 Docker 停机/留卷/清理已核对。限定 Task6 技术条件闭合，正常文档提交同步待收尾；完整恢复、P0-C4 整关和 CompleteBackup 未验收。当前状态与证据范围见[Task6 验收汇总](p0c4-controlled-import-task6-final.md)。

以下为历史实施与交接记录；旧交接命令已执行或作废，不再运行，旧失败证据保留。

## Task6 最后三道回归入口已交接（待运行）

受控合成导入累计 **11/11**，当前封存提交17153d8实际 **7/11**，四项历史来源经差异审查携带。最后三道回归入口已独立复审并上传，但现场新增通过数仍 **0/3**；Task6保持未关闭。本轮运行指定集成目标、文档测试和工作区严格Clippy，首错停止，结束精确ID停机留证。运行命令与交接回执见[最终回归交接](../.superpowers/sdd/2026-09-30-p0c4-controlled-import/Task6最终回归交接.md)。C4完整恢复、CompleteBackup和生产尚未验收。


## 当前状态（2026-10-02）

**Task6 固定合成 fixture 的现场场景清单累计已接受 11/11：1 项正例与 10 项负例。整计划最终复审和必要回归仍待收口。** 历史四项来自 `5619026`；当前源码 `17153d8` 实际通过 7/11，不能称最终修订同源 11 项重跑通过，也不能换算整个项目完成百分比。

| 门 | 当前结果 |
| --- | --- |
| 当前 Linux 前置 | `1df95b63`：格式、严格 Clippy、编译、库测试 146 通过／19 忽略、journal 1 通过；不增加现场场景计数。 |
| 历史四项 | success、READY 真 EOF、PRECOMMIT 真 EOF、提交前取消已接受；保留 `5619026` 原收据。 |
| 当前修订七项 | DDL 前重启，以及 SQL 错误、COPY 截断、attempt 同步失败、commit-intent 同步失败、提交结果未知、错误端点均已实际接受。 |
| 最新六项回传 | 六个精确测试及 main 均退出 0；总 `REMAINING_SIX_RUN_EXIT=0`。检查点数量分别为 11／12／12／13／14／13，结果 canonical SHA 与回传一致。 |
| 提交结果未知 | `CommitUnknown`，已尝试 COMMIT，未确认回滚且禁止重试；该负例通过不代表导入成功。 |
| 独立接受 | Spec／Quality Approved，C0／I0／M0；控制者读取完整报告并重验 26 个输入 pin，新增登记 6 项。 |
| 独立停机核对 | 13 个精确 PG 容器 ID 均停止，固定镜像／项目／挂载卷匹配且卷保留；12 个 builder ID 已消失。 |
| 下一道门 | Task6 整计划最终独立复审、历史四项对最终源码的覆盖适用性、必要定向及 package/workspace 回归。专项场景累计 11/11 本身不能关闭计划。 |
| 验收边界 | C4 整关、完整恢复与独立备份目标仍未验收；CompleteBackup 未签发。 |

本轮六项运行单已执行并关闭，禁止重跑其批次或历史失败批次。原封存交接保持创建时的未运行快照；实际执行与接受另存于 [remaining-six-accepted-record.json](../.superpowers/sdd/2026-09-30-p0c4-controlled-import/remaining-six-accepted-record.json)，SHA-256 `4a6ef39da32d49f494efcfb408a533512a4c80a75ff704bff7e9394e9b7c2bd5`。返回审查 seal SHA-256 `ae0098d01e91782955c64dc6e6868c1ddcc454c6e5164aef1521c2b8fd1b7ebb`；控制者只读 Docker 证明 SHA-256 `4ef2d64a259d0ff9d9ea1909a7b422e447081440f75d156a09ffa11f0e44d5c0`。

控制者核对用户完整返回及普通用户 Docker 元数据，没有直接读取 root 私有原始日志、工件、marker 或 fsync 状态；这些由封存并审查的原操作器及严格 runner 承载。未重跑用例、未放宽现场谓词，旧失败结果及未用预约全部保留。

## 历史实施与验证记录

### 2026-10-02：剩余六项交接时的历史状态（六项命令现已执行，禁止重跑）

**Task6 固定合成 fixture 的实际验收累计为 5/11：1 项导入正例和 4 项负例已接受，剩余 6 项未运行。** 这是专项验收进度，不代表整个项目完成百分比。累计前四项来自 `5619026`；当前源码 `17153d8` 已实际验证 `ready-restart` 一项，不能把历史四项写成当前源码重新执行通过。

| 门 | 当前结果 |
| --- | --- |
| 当前 Linux 前置 `1df95b63` | 源码 `17153d8`：格式、严格 Clippy、编译、库测试 146 通过／19 忽略、journal 1 通过。前置门不计入 11 项实际验收。 |
| 已接受的历史四项 | 导入正例 `94b411de`；READY 真 EOF `5ce4c404`；PRECOMMIT 真 EOF `c679e11d`；提交前取消 `b2c7b953`。原收据保留，不重跑、不改写。 |
| DDL 前重启 `431b411b` | 全新源目标批次实际通过，精确测试及本机 wrapper 退出 0；Identity 拒绝、全部 12 个原检查点、无 COMMIT、attempt／intent 均不存在；旧 guard 拒绝及精确停机门满足。 |
| 原重启失败 `7f2ed357` | 历史失败及诊断证据保留；旧批次禁止重跑。本轮成功不反向改写旧结果，也不单凭本轮成功断言旧失败的全部根因。 |
| 独立返回审查 | Spec／Quality Approved，C0／I0／M0。控制者重新核对报告、seal 及 21 个输入 pin，新增登记 1 项。 |
| 停机与证据 | 原 canonical result SHA 与用户完整返回一致；普通用户 Docker 元数据独立核对源目标精确 ID 均停止、固定镜像／项目／卷匹配、卷保留，两个 builder ID 消失。 |
| 后续六项 | SQL 错误、COPY 截断、attempt 同步失败、commit-intent 同步失败、提交结果未知、错误端点。均为 NOT_RUN；新资源预约已只读预检，适配的 16 项本地检查及独立 runtime/staging 审查通过（C0／I0／M0）；两份封存文件已上传，各完成两次字节／哈希读回。等待用户本机 sudo 运行。最后一项独占第三个克隆项目。 |
| 验收边界 | C4 整关、完整恢复及独立备份目标均未验收；CompleteBackup 未签发。 |

本轮 `READY_RESTART_RUN_EXIT=0`。原先已执行的重启运行命令和十负例命令均禁止再运行；后续六项必须使用新控制 UUID、新源目标项目、新 PG18 卷与新子网，并按首个失败即停规则执行。

登记文件 [ready-restart-accepted-record.json](../.superpowers/sdd/2026-09-30-p0c4-controlled-import/ready-restart-accepted-record.json)，SHA-256 `e99d518b12430b856fd0f1a438b1cdc72cd4edef1c3731eee120453e14f89e32`；B result SHA-256 `6717beb481f8bec5163e98d0819ffa80887448c96c8e6c24d090052b2a140b7f`；A result SHA-256 `3c07639ce1de10186424657b43ad0574c1fa2ebffc9f481b6bb99d99aeb5ca01`。返回审查 seal SHA-256 `65dc16f585252ac8fc029f09a5570560c21831ad8552b849a1d2f6bf652dced7`；控制者只读 Docker 证明 SHA-256 `53ba58309e7a5f0d03b091ec35d492f8a1b86ad9f5f4aaaeb3f693dc030863db`。

控制者核对用户完整返回和普通用户 Docker 元数据，没有直接读取 root 私有原始结果、stdout／stderr、marker、工件或文件系统持久化状态。A 原始严格／列举／库／journal 日志与源码、二进制由已审查的 root 操作器在唯一 B 调用前复验；B 原始精确测试及完整严格解析由封存 runner 执行。控制者和返回审查员仅对返回数据作纯验证，没有重跑实际用例。历史 uploader 首行“三文件”注释问题（实际两文件白名单）仍作为既有 M1 保留到最终分支审查；现场谓词未放宽。

剩余六项可使用 [剩余六项运行单](../.superpowers/sdd/2026-09-30-p0c4-controlled-import/剩余六项运行单.md) 的单行入口。交接 SHA-256 `5d42a9fd2f2a6452a0baa4988466f8562350d3ff07aadb25a25a63cca19b27a3`；runtime review seal SHA-256 `3fec70d6a8cd50d61814e1f144ef0b8e0c755462b44430f34498de0b47b9e4be`。上传本身未增加实际通过数，root 现场门仍未执行。

### 2026-10-02：重启修补交接时的历史状态（当前命令已执行，禁止重跑）

**固定合成 fixture 的实际验收为 4/11：1 项导入正例和 3 项负例通过；DDL 前重启负例正式失败，余下 6 项未运行。** 这是 Task6 单机专项进度，不能换算为整个项目的完成百分比。

| 门 | 当前结果 |
| --- | --- |
| Linux 前置 `86dae3be`（历史） | 源码 `5619026`：格式、严格 Clippy、编译、库测试 140 通过／19 忽略、journal 1 通过；保留既有收据，不作为新源码 `17153d8` 的前置证明。 |
| 新源码与前置 `1df95b63` | 窄修补提交 `17153d8` 已独立审查通过（C0／I0／M0）；本地六项回归和 Windows 库测试 136 通过、格式／严格 Clippy 退出 0。新 Linux 前置与真实重启负例均待运行。 |
| 导入正例 `94b411de` | 已接受。真实 dump 解码、同 writer 准入、attempt／intent 持久屏障、唯一 COMMIT、表／两行／主键／摘要读回通过。 |
| 新负例通过 | READY 真 EOF `5ce4c404`、PRECOMMIT 真 EOF `c679e11d`、提交前取消 `b2c7b953`。各精确测试与原 main 均退出 0，12／14／15 项检查点、无提交／零对象／marker 谓词完整。 |
| DDL 前重启 `7f2ed357` | 正式失败不改写。固定只读诊断确认仅检查点集合不满足：缺少 `RESTART_BEFORE_DDL`、`OLD_GUARD_REJECTED`、`DDL_NOT_SENT`，其余 15 个谓词匹配；诊断退出 1，不增加通过数。 |
| 停机与证据 | 四份 canonical result SHA 与用户返回一致；独立返回审查新增 C0／I0／M0。控制者普通用户只读核对 8 个精确 PG 容器已停止、固定镜像／项目／卷匹配、8 卷保留、8 个 builder ID 消失。 |
| 后续 | 新 Git 原样源码包和新运行单已独立审查、上传并逐文件读回核验 2 次。一次本机 sudo 先运行新 Linux 格式／Clippy／编译／库测试／journal，再在全新源目标项目验证 `ready-restart`；旧失败批次不重跑，后续六项仍 NOT_RUN。见 [READY 重启新批次运行单](../.superpowers/sdd/2026-09-30-p0c4-controlled-import/READY重启新批次运行单.md)。 |
| 验收边界 | C4 整关、完整恢复及独立备份目标均未验收；CompleteBackup 未签发。 |

顺序操作器按首个失败即停规则结束，`NEGATIVE_SEQUENCE_RUN_EXIT=1`。原四批源库／目标库、卷及证据保留隔离；既有交接与旧失败记录不改写、不重放。此前十项运行命令已经执行，不能再运行同一命令。未运行的六项不能用编译／列举／静态检查代替真实验收。

新登记记录 `real-import-negative-sequence-accepted-record.json` SHA-256 `9a2ae590f8d9ec88d2226524d42e68cdc9ef00ef07a54506e8bdd5726e508f97`；只读 Docker 证明 SHA-256 `f2b548eccd0b9eb9d6fef04e50970824a11f4054d09eac83c610118ead543a06`；返回独立 seal SHA-256 `efb919945446918de833f56304e95f2c333e277a4de16d81ba79b5bad59472b3`。失败 result SHA-256 `d9e6550f171ec6a2738edf89690fb4a875b7cf2d5d58130e6e7161b0ce912bba`。

控制者核对来自用户终端的四份完整返回和已授权的普通用户 Docker 元数据，没有直接读取 root 私有结果、日志、出生记录、marker 或工件原文。A86 原始证据由现场操作器复验，不能把控制者未读的私有事实写成直接观察。沿用既有 M1 uploader 首行“三文件”注释问题（实际白名单为两文件），留到最终分支审查；没有放宽任何现场验收条件。

### 2026-10-02：十负例交接时的历史状态（尚未运行，命令现已执行，禁止重跑）

**固定 fixture 导入正例已通过，实际验收为 1/11；其余 10 项负例尚未运行。** 候选源码 `5619026` 的真实 Linux 前置门为库测试 **140 通过／19 忽略**、journal 专项 **1 通过**。`SOURCE_PIN_RUN_EXIT=0`。

| 门 | 当前结果 |
| --- | --- |
| Linux 前置 `86dae3be` | 格式、严格 Clippy、编译及真实库／journal 通过；pending 不存在。结果 SHA-256 `68e9cf5e11dfeb46d7ff4c302a0d365d327a1fc00a757d53b8576566cd5ccb8c`。 |
| 实际正例 `94b411de` | 精确 `live_controlled_import_commits_fixture` 退出 0；真 dump 解码、同 writer 准入、attempt／intent 持久屏障、唯一 COMMIT、表／两行／主键／摘要读回均通过。 |
| 证据复核 | 独立规格／质量审查 PASS、0 阻塞／0 重要／0 次要；B 完整返回按真实生产者合同重算 result SHA 吻合。控制者只读核对两精确 PG 容器已停止、卷保留及两 builder ID 不在。 |
| 下一门 | READY／PRECOMMIT 真 EOF、取消、DDL 前重启、SQL／COPY 故障、两项 journal 同步故障、提交未知和实际 writer 错端点，共 10 项；每项全新资源、顺序执行、首个失败即停。 |
| 验收边界 | 仅单机固定合成 fixture 正例；C4 整关、完整恢复、独立备份目标及 CompleteBackup 均未验收。 |

正例 result SHA-256 `94f0411b96732d741567042b8603ede5fc96f62d40d310f5f550069dfd8d0737`，验收记录 `real-import-success-5619026-accepted-record.json` SHA-256 `85536e02e25975e7d2f1c8895af207c1b2e35ee23c11dc55b4c1dcc9f3239d30`。A 只回传摘要，控制者不能据此独立重算 A 私有 result；B 正式操作器在现场重新读取其原始收据、strict／all／ignored／unit／journal 日志和源码／二进制后放行。控制者没有直接读取 root 私有结果、原始日志、marker、dump／TOC／SQL 原文；不把用户终端来源写成直接远端读取。

所有旧失败批次和本轮成功卷均保留隔离，不重跑。沿用相同封存源码和 builder，通过严格原始证据复验后复用 A86 收据；不重复已通过的 A 或 success。新的负例顺序操作器已通过本地 17/17、独立规格／质量与上传器审查，两份新脚本已上传到全新 hans 0700 私有目录，每份 0400／两次稳定哈希读回。十项现场仍为 NOT_RUN；当前实际验收继续为 1/11。独立审查 0 阻塞／0 重要／1 次要；M1 为 uploader 首行“三文件”注释，实际白名单严格只有 root／wrapper 两文件，保留到最终分支审查。

在 Linux 终端执行一次（sudo 密码仅在该终端输入）：

```sh
bash /home/hans/knowweave-c4-negative-sequence-51e6e796-749b-46e0-a60c-7daaa1a93719/real-import-negative-sequence-operator-command.sh
```

依次验证 READY／PRECOMMIT EOF、取消、DDL 前重启、SQL／COPY 故障、两项 journal 同步故障、提交未知、实际 writer 错端点。每项新源库／目标库／卷；末项另建新物理克隆。首个失败即停止后续项，不复用旧批次，不重跑原前置门或正例。运行前实际复验原 A86／B94 证据及资源／子网／容量；结束后精确 ID 停机、留卷留证。贴回终端 JSON 输出及 `NEGATIVE_SEQUENCE_RUN_EXIT`；失败后不要重跑同一批次。

新 runtime 审查 seal SHA-256 `11b0dd445930af2fa1bf730884aeec991c7eaa00fd81df10f8ae895b652fcc8c`；上传 receipt SHA-256 `f0e9ba7a92e52b346d294b923572dc0ac5944cd9107018ae8b7b93bf873e7ac6`。上述上传未执行 root／PG／负例；receipt 中 actual_cases_passed=0 指本次上传没有运行新用例，不覆盖已接受的 1/11。

### 2026-10-02：修复包交接时的历史状态（尚未运行）

源码候选 **`5619026`** 已完成三文件修复、独立规格与质量审查，并封存上传；新 Linux 批次尚未运行，固定 fixture 导入实际验收仍为 **0/11**。

| 项目 | 当前证据 |
| --- | --- |
| 修复 | 夹具采集的源库 guard 原来使用目标库编译时 birth pin；测试构建现在分别选择 source／target pin。目标恢复入口继续固定使用 target pin。 |
| 本地验证 | Windows 库 130/130；格式及严格 all-targets/all-features Clippy 退出 0。整包测试退出 101，集成夹具缺少 `TEST_BACKUP_EMPTY_DATABASE_NAME`；后续整包用例未验证。 |
| 独立审查 | 源码、操作器、wrapper 与普通用户上传器通过；新审查 0 阻塞／0 重要／0 次要。 |
| 上传 | 新源码 ZIP 456 个 Git 文件；ZIP、操作器、wrapper 已在新 hans 私有目录排他写入，每份 0400、两次稳定哈希读取一致。上传没有执行 root 或测试。 |
| 下一门 | 全新 `86dae3be` Linux 格式／严格 Clippy／编译／真实库和 journal 前置门；再次读取其实际收据与日志通过后，只运行一次全新 `94b411de` success。 |
| 验收边界 | 新源码 Linux 计数未知；旧源码 `0902bc34` 的 136/19/1 不能作为新源码证明。C4 整关、完整恢复和 CompleteBackup 仍未验收。 |

上一轮 `282a1dfc` 的精确测试退出 101，固定阶段为 `fresh-fixture-capture`／`Identity`。此次修复依据封存源码中可复现的调用合同错误；该现场内部首个失败谓词未被观察，旧 `91a` 根因仍未知。所有旧失败批次和证据保留，不重跑。

在 Linux 服务器终端运行一次（sudo 密码仅在本机输入）：

```sh
bash /home/hans/knowweave-c4-source-pin-b1a18043-2ae0-4f8b-a8eb-8a9112064a74/real-import-source-pin-operator-command.sh
```

命令末尾回传 A／B 的 JSON 摘要和 `SOURCE_PIN_RUN_EXIT`。失败即停，不在同一批次重跑。新源／目标 UUID 为 `81e27cfc-a51b-4009-81fe-b4158467f3af`／`a9720b7d-ed0c-4112-8554-523d65baa4ed`，拟用子网 `10.253.134.0/24`／`10.253.135.0/24`，运行时再次检查路径、项目和子网未占用。

封存 ZIP SHA-256 `9e73c9665604b348436f39f62117dce02e02629b4a988bacb2f494a70a6c5c82`；新交接记录 `real-import-source-pin-runtime-handoff.json` SHA-256 `40dc74bfbb67dffdf78a850c38adc1e017b295c1aa857c94936f39184dc7f5ff`；可变进度账本为 `.superpowers/sdd/2026-09-30-p0c4-controlled-import/task6-live-gates-status.json`。

以下保留各轮当时的状态、待运行提示和失败证据，以本页“当前状态”为最新口径。

**新源码 Linux 前置门通过，success 定位到夹具采集身份拒绝（2026-10-02）。** 用户终端回传新 `0902bc34` 前置收据：库测试 136 通过／19 忽略，journal 1 通过，pending 不存在，结果 SHA-256 `552c34642d017b829ed9e1312e32344d37a3af9f90acdaff4c0f1cbd08206b91`。统一操作器重新读取该实际收据和原始日志后进入新 `282a1dfc` success；精确测试实际退出 101，固定诊断为 `fresh-fixture-capture`／`Identity`，candidate 为空，结果 SHA-256 `81a2990a3a0071bb6ce6e4913bca6387180e40e0fd125c8ea4cafc514a10b999`，总命令退出 1。构建／列举退出 0 的 executed:false 仅描述列举，不覆盖实际测试失败。控制者独立只读核对两个精确 PG 容器已退出、映像／项目／卷匹配且卷保留，两个构建器 ID 均不存在；未直接读取 root 私有结果或日志。当前实际仍 0/11，内部首个失败谓词尚未观察；正在核对封存源码的 source 身份 pin 调用合同。该失败批次不重跑，不将新诊断倒推为旧 91a 的确定根因。

**最新现场结果（2026-10-02）：首项 success 的精确测试已执行且失败，实际仍 0/11。** 用户回传已审查的 root 只读诊断，`IMPORT_DIAGNOSTIC_EXIT=0` 只表示诊断成功。批次 `91a34686-ad94-4ad9-9c40-c70021b4e4dc` 的精确测试 `live_controlled_import_commits_fixture` 为 0 通过／1 失败／0 忽略／147 过滤；原测试退出码未保存，仍为 UNKNOWN。失败日志仅包含 `live_tests.rs:149:9` 的固定 `CONTROLLED_IMPORT_CASE_REJECTED`，没有原始 ImportFailure 或阶段；导入根因尚未确定。列举阶段的 `builds.*.executed:false` 不再用于推断实际测试执行状态。源、目标已停机留卷，两构建器已清理；旧失败批次不重跑。

**只读诊断证据已核对，补充范围已确定（2026-10-02）。** 原结果 7365 字节、SHA-256 `b4364530ea2a4e973360620be4cc3ed279c2a412daba9720d1e885b35171a136`；stdout 370 字节、SHA-256 `b3ec415c651fe06475f7341f9d757e87a4694379224d7a337c9c0e406f3bc5a9`；stderr 346 字节、SHA-256 `29257534f4ae842f9df4216869cee0cb6aed0aea503e346cb866a830f59b77fb`。三份均为 0600、READ_OK，结果摘要和身份一致、pending 不存在；来源是用户终端的脱敏诊断，控制者未直接读取 root 私有日志。原驱动在测试返回后因非空 stderr 拒绝，未进入成功 Observation 解析；测试层又将类型错误和 join 错误折叠为固定 panic，尚不能据此修导入算法。本轮补充限定为测试层的固定阶段、类型原因和实际退出码，由子代理实现并独立审查；后续仍须新源码、新前置门和全新隔离 success 批次验证，不放宽原验收、权限、清理或预算。历史只读解析器展示歧义 M-91A-01 保留待办，以 `files.*.read_outcome` 为准。C4 整关、CompleteBackup 和生产恢复仍未验收。

**诊断补充已提交；新源码 Linux 验证待运行（2026-10-02）。** 六文件提交 `c26e14c59f2332fb1ab15dd658619e8b8f5b068a` 已完成子代理实现及独立规格／质量审查，0 阻塞／0 重要／0 次要。本地 Python 57/57、Windows Rust 库 126/126，格式与严格 all-targets/all-features Clippy 退出 0；控制者核对 33 份审查输入及真实日志。新增输出只有封闭阶段、类型原因和布尔状态，保留原 panic、失败判定及成功 payload；不修尚未确定的导入根因。封存 ZIP 包含 455 个 Git 原始 blob，SHA-256 `66d52d6d19c0f188e6290f126da29e5944fec19ed007f0e709b7cce79b8263f9`。本轮安排新 `0902bc34` Linux 前置批次和一次新 `282a1dfc` success 用例；旧 `03d1c8c8` 收据仅证明旧 `1cc3aec` 源码，不作为新源码通过证据。服务器尚未运行本轮，实际仍 0/11。

**统一运行脚本已审查并上传，等待本机 sudo（2026-10-02）。** 新操作器定向 28/28、退出 0；独立规格／质量与普通用户上传器审查通过，0 阻塞／0 重要／1 次要。M1 仅为审查 diff 的末行换行格式，完整运行脚本可独立审查，该 diff 不上传或执行。控制者复核 48 份审查输入及报告；源码 ZIP、root 操作器、短 wrapper 已上传到新私有目录，服务器每份两次稳定哈希／0400 权限核验一致。运行命令为 `bash /home/hans/knowweave-c4-failure-phase-95b52cde-a1bf-4441-bd1e-d6e1a03e2b65/real-import-failure-phase-operator-command.sh`。先完成全新 `0902bc34` 的严格 Linux 前置门及真实库／journal 测试，重新读取实际收据和原始日志核验通过后，只运行一次全新 `282a1dfc` success 用例；失败即停、不重跑。实际 Linux 库存当前未知，本地合成计数不作现场证明。尚未执行本轮 sudo、PG 或真实导入，实际仍 0/11；CompleteBackup 和 C4 整关未验收。


更新：2026-10-01。本说明属于[六任务施工单](superpowers/plans/2026-09-30-p0c4-controlled-import.md)的任务1；规格见[受控小型 dump 导入设计](superpowers/specs/2026-09-30-p0c4-controlled-import-design.md)。

## 当前状态
**首项实际导入的准入受阻，实际仍为0/11。** 全新操作批次 `6390746c` 在 `PREREQUISITE_RECEIPT` 返回 `Identity`／`REAL_IMPORT_SUCCESS_PRE_ATTESTATION_REJECTED_NOT_RUN`，退出1，没有启动真实导入；只读Docker名称盘点未发现该控制、源或目标UUID对应资源。静态诊断已确认读取端要求的四个 `build` 字段不在真实前置结果中，之前的本地模拟结构掩盖了该接口不一致；现场首次失败谓词仍未确认，控制者未读取root私有结果。只修订新的读取操作器，沿用真实严格退出码并核验原始all/ignored日志，不改旧收据、生产源码或验收门槛；旧批次不重跑。

**读取端修复的现场结果：前置收据阶段通过，停在安装输入核验，实际仍0/11。** 真实生产者结构回归复现了旧读取端拒绝；修复后15项定向检查退出0，规格／质量／上传器静态复审PASS，I-RECEIPT-01已处理。新操作批次 `10dc6556` 使用全新源/目标及 `10.253.102/103.0/24`，命令19594字节、SHA-256 `af140f7410d50feb42327ab6aaf50d947dbd31ea81bbc013b09b97233c6c019c`，服务器两次读取核验一致；root终端已执行准入检查，但真实导入未启动，仍0/11。前置收据按真实七字段结构读取，严格退出码及原始all/ignored日志独立核验；旧失败脚本、收据和批次保留。

**最新准入诊断（2026-10-01）：** 操作批次 `10dc6556` 返回 `Identity`／`INSTALLED_INPUT_PINS`，退出1，真实导入未启动。用户在root终端只读核对：已有runner、fixture、birth helper和ZIP的大小、权限及SHA-256均匹配；`p0c4_restore_pin_acceptance.py` 不存在可用的普通非符号链接文件。编译阶段只安装三份脚本，而导入准入要求四份。控制者未直接读取root文件；普通SSH元数据核对未发现该控制/源/目标UUID的Docker资源。已审查的窄修补会从同一封存ZIP提取四份原样Git字节到全新0700工具目录，旧目录/脚本/批次保留，不放宽身份或验收门槛；新候选尚未运行。

**安装输入修补已审查并上传，等待首项实际导入（2026-10-01）：** 新操作批次 `91a34686-ad94-4ad9-9c40-c70021b4e4dc` 使用全新源/目标和 `10.253.130.0/24`、`10.253.131.0/24`。修补后的操作脚本先核验原收据、完整ZIP、源码和二进制，再从同一ZIP向新root私有工具目录排他安装四份原样脚本，旧三份工具目录保留。本地定向检查12通过／0忽略，独立规格、质量及上传器静态审查PASS，I-INPUT-01已处理，控制者核对35份审查输入。命令24995字节、SHA-256 `d5219773880fce7c0d74c221e2480d49bc5b4f255d19cfa6617297179657d7c5`，上传后服务器两次读取核验一致。尚未执行本轮root安装或真实导入，实际仍0/11；运行时再次核验资源未占用，通过后再核对完整结果、停机留卷和哈希，才能计入首项success。本地模拟/结构检查不替代Linux/PG18现场结果，C4整关、CompleteBackup及生产恢复仍未验收。

**2026-10-01 最新现场进度：全新 root 批次 `03d1c8c8` 前置门通过，格式与严格 Clippy 通过，普通库129通过／19忽略，Linux journal no-follow/fsync专项1通过，退出0；实际导入仍为0/11。** 用户终端回传结果 SHA-256 `a67ab567f8bffefcb66d71f58a381f900e32fe34727b738af3ce07a9163f415a`，源码和二进制未变、builder清理确认、pending消失。封存源码提交为 `1cc3aeca3485caa7ac910e9f18411187afba6d8f`。来源为用户终端摘要；控制者未直接读取root私有完整结果或日志，后续实际导入操作先独立核验该收据。接下来在全新源/目标、PG18卷及子网运行11项实际case；C4、CompleteBackup及生产恢复仍未验收。


任务1本地实现 `43cd2df8b10d3eb98385d7c5ddd785152d474e5f` 和修复 `4fad8e681060c8bd4575b033f860e97054cccc2e` 已经独立审查/定向复审。共享停机截止、私有 Rust 接口可见性和 UUID 变体校验问题已关闭。首轮中文路径封装修复 `305a54f` 已独立复审；首轮服务器异常栈未取得，仍保留诊断边界。canonical 包 `a300908` 随后通过准入但止于全量观察 `StdoutLimit`。有界分批修复 `77c24ba6ab802e815f7968e5d66d325ac0740898` 已独立限定复审，新封存包 `5bc482d` 和新批次已获授权并运行：2026-10-01 返回 `CLIENT_CONTRACT_AND_FIXTURE_CAPTURE_PASSED_NOT_IMPORT`、退出0。控制器复算结果/源码摘要并只读核对双容器停机与留卷。两份合成工件已受控导出、下载并独立审查通过，任务1已完成；任务2可据实产封存完整golden，任务3–6待顺序推进。尚未执行目标导入或完成C4整关验收。旧失败批次不重跑。

2026-09-30 的控制批次 `55571d76-02f8-4c47-aeb8-4d54fe29ad46` 返回 `CONTRACT_FAILED_QUARANTINED_NOT_IMPORT`、退出1、固定原因码 `StdoutLimit`。用户终端摘要的 canonical payload 复算匹配 `245d94129c925b1262b32052133cfd8fdd42227c78f38d99ac8d73c83b1e4933`；源码 digest 与本地封存清单复算相符。控制器未独立读取 root 私有结果文件。只读现场测量发现 764 个既有容器的完整 `inspect` stdout 为 9,387,498 字节，超过单次 4 MiB；用同字节授权适配器只读复现相同拒绝。源、目标项目当前均无容器、网络及精确卷；结果的 `resources` 为空，`volumes_retained:true` 不能作为已经建卷或停机的证明。原始全量 Docker 输出未展示或保存。

修复范围仅为合同驱动的完整元数据查询：每批最多64个对象、单次 stdout 4 MiB/stderr 64 KiB保持不变；一次 inspect 总计最多4096个对象和16 MiB，全部分批共用一个30秒绝对截止，并与停机15秒绝对截止取更早者。保留全部 inspect 字段、对象顺序与身份比较；不改固定 writer、64 KiB工件预算、接收端或客户端合同。行为 RED 使用真实私有 snapshot 与子进程管道复现65个完整对象总量超过4 MiB的拒绝；最终新增六项测试6/6通过。唯一一次提交后全 Python 回归为231/232，既有 M3计时断言再次失败，独立审查判定为范围外未解待办；本轮现场合同成功不消除该失败，也不能宣称全套通过。

## 最新现场证据（2026-10-01）

- 封存提交 `5bc482d7117549050e26f37510e695c8dbcd8dd6`；ZIP `08979f05e5b05c4b83dbbbbf065d6a0c451addbe6044b8d53d90dcf6db24a07a`，manifest `d381a255558938acc4c8993fac7fc58055dbe25202cfebc4bba6ea0f297c3a0a`，runner `9d78b2c38ec7a0e395107d7a56a7a84f2be85a697a5f468775bc9b03476af4e1`。结果路径 `/var/lib/knowweave-c4/controlled-import/batches/14b45cf0-e9f3-4beb-a250-ceb6f49d5a14/evidence/result.json`。
- 用户root终端返回完整成功JSON及退出0；控制器规范序列化复算结果SHA-256 `505ab6179e3be1f1c8afb441fa694db8d5033f27e77a0940908de208bad96508`，源码 digest `996342cf730713bb7e0f7f585c900d869106ca6dcb65047563d97601bbd21fb3` 与封存清单复算一致。回传来源明确标注，不冒充控制器独立读取root私有文件。
- 源、目标开放stdin READY/PRECOMMIT、同writer事务、ROLLBACK、无DDL EOF、`/dev/null`字符设备、disabled passfile空stderr、本地trust HBA均报告通过；源第三行EOF回滚与原两行读回通过，目标基线未变。现场成功仅证明本轮固定客户端和合成夹具。
- 控制器另行只读核对源容器 `7e9e059ddc85a206c1f16b99b8fd7a058938227040931281de1275176c113d56`、目标 `c2337121f90ed34d5e3d8423e017c49c49a42645728e0b37a85999d6d69ccf35` 均exited/退出0，固定镜像和项目/子网匹配，精确卷仍保留。
- 用户在原授权范围内完成两文件导出，返回 `ARTIFACT_EXPORT_EXIT=0`，核对root结果哈希、`result.pending`消失及双容器停机。控制器独立核对新hans目录0700、恰好两份0400文件、实际字节和哈希后下载：dump1,980字节/SHA-256 `56b12180a18e84b998ead3f1c7d552b922c68c765de2b5725d93112e5dbb5136`；SQL1,308字节/SHA-256 `e2c252bfa5d44c5ed9133dcdb7fd8344e0a2471489d0ee410edb6c09baabd44a`。
- 已授权的离线 `pg_restore --list` helper验证身份后执行，固定PG18、无网络/凭据/磁盘卷、只读dump、1CPU/128MiB；用有界tmpfs覆盖镜像声明的PG volume，避免匿名磁盘卷。输出625字节/SHA-256 `edcd6b7eb8b23a1b91113b0fde92da041c08a01235706bb481d1f23dd0d161fb`，退出0/空stderr，按精确ID清理。选中目录仅TABLE、TABLE DATA与CONSTRAINT三项，归档总TOC条目数7不等于七个业务对象。
- 独立子代理直接重算并审查dump、完整SQL和TOC，规格/产物质量均通过，无阻断。实产SQL为LF、59行、63位成对ASCII字母数字key；原始header `[0,670)`、payload `[670,1308)`，固定表/主键与alpha/beta两行，无额外对象、重连、LO、内层事务控制或提前unrestrict。四项timeout仅在整份匹配后改为固定SET LOCAL；Task2只允许匹配的1..128位key变化，不能沿用变长key之后的绝对尾部偏移。

本次封装修复新增真实 Git 字节/接收端回归，RED 为三项中两项预期失败，GREEN 为三项全部通过。全 Python 回归在提交后首次为 225/226：既有隔离计时测试的一处进程退出状态断言失败；同一测试单独运行以及下一次全套均通过，最后全套为 226/226。失败日志保留，原因尚未证实，不能称全套稳定通过。上一代码修订的 Python 专项 30/30（控制器另用 Git 原字节运行）、Rust 全工作区库测试 100/100、格式与严格 all-targets Clippy 通过；本次无 Rust 改动，未重复无关检查。完整 Cargo 集成测试曾止于 `catalog_pg` 缺专用测试库环境，不能声称全 workspace 通过。首轮执行者的原始 RED 日志未回收，只保留其报告；两轮修复另有实际观察的 RED→GREEN 记录。

新驱动给私有 provisioner 安装增量有界命令适配器：观察 stdout 上限 4 MiB、stderr 上限 64 KiB；固定 writer 的各 8 KiB 限制和 stderr 拒绝不变。源、目标停机共享一个 15 秒绝对截止，包含当前 CLI 的终止/回收和读取线程收尾，不通过超时后继续执行动作的后台线程实现。

## 本轮验证什么

| 对象 | 允许活动 | 必须观察到的结果 |
|---|---|---|
| 全新源 PG18 | 创建固定 `public.c4_import_probe` 表及 `(1,'alpha')`、`(2,'beta')`；在另一个显式事务中插入第三行后关闭 stdin | 独立连接读回仍只有原两行，建立实际 EOF 回滚证据 |
| 全新目标 PG18 | 固定客户端的 `BEGIN READ ONLY`、保持 stdin 开放的 READY/PRECOMMIT 握手、ROLLBACK；另做无 DDL 的 EOF 探针 | 无额外目标关系、对象或数据；握手期间同一子进程保持存活 |
| 固定客户端 | 精确容器 ID、999:999、本地 socket、绝对 PG18 客户端、干净环境、disabled passfile | 无密码交互或认证诊断；实测 `/dev/null` 字符设备及所用 local HBA |
| 合成工件 | 源库 custom dump 与离线解码 SQL，各最多 64 KiB | 分开保存、核验字节数和 SHA-256，不向目标执行捕获 SQL |
| 最终证据 | 源/目标精确身份、代码封存身份、停止与留卷事实 | 停机处理、源码复算与持久化完成后才发布最终结果；停机未确认不能成功 |

固定镜像为 `postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d`；执行前还核对既有 builder ID `sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b`。本轮不编译现场 Rust writer。

## 封存与安装形态

首次部署必须逐文件确认源码 ZIP 和三个 ZIP/Git 原样 Python 文件：

1. `p0c4_controlled_import_acceptance.py`：新合同驱动，安装模式 0500。
2. `p0c4_restore_birth_acceptance.py`：窄复用的封存/私有目录/证据辅助模块，安装模式 0400；文件保持原实现。
3. `p0c4_import_fixture.py`：固定客户端和合成工件辅助模块，安装模式 0400。

首轮三个脚本已相邻安装到 root 所有、0700 的 `/var/lib/knowweave-c4/tools/p0c4-controlled-import-task1-045544f/`。canonical 失败批次复用的是该同字节目录。第三轮改变合同驱动，下一批须封存并单独确认新 ZIP 与新 runner，安装到另一个新0700私有工具目录；fixture/archive helper字节不变，获明确复用授权并重新核验后才原样复制到新目录（0400）。旧工具目录、文件与失败证据不覆盖。新 ZIP 安装到 `/var/lib/knowweave-c4/incoming/` 的新文件（0400）。所有文件来自 Git 原字节，其他依赖仅从封存 ZIP 解出；不能用Windows换行后的工作副本替代。

第三轮 Git 原样合同驱动 SHA-256为 `9d78b2c38ec7a0e395107d7a56a7a84f2be85a697a5f468775bc9b03476af4e1`（24,135字节）；未变 fixture helper为 `a8fef75b20d6f46539bb458ac71861be8aaf2c1929607bd6729808e1f24d5a48`，archive helper为 `53b391cca520347803399828e1d0a2fcb6a1563eb1304be4319b00565f0a2daf`。旧驱动 `913c8823…` 保留为历史字节，不能用于第三轮新包。

## canonical 失败批次资源（历史记录，禁止重跑）

| 用途 | UUIDv4 | 项目 / 子网 |
|---|---|---|
| 控制批次 | `55571d76-02f8-4c47-aeb8-4d54fe29ad46` | `/var/lib/knowweave-c4/controlled-import/batches/55571d76-02f8-4c47-aeb8-4d54fe29ad46/`；无 PG 容器 |
| 合成源 | `2cde52c8-fbbc-4b9c-ae4b-dce60c21acf1` | `learning-system-p0c4-restore-2cde52c8-fbbc-4b9c-ae4b-dce60c21acf1` / `10.251.239.0/24` |
| 合同目标 | `ff1fde0c-506f-4ee8-8aec-f5f98803b2e2` | `learning-system-p0c4-restore-ff1fde0c-506f-4ee8-8aec-f5f98803b2e2` / `10.251.240.0/24` |

每个 PG 项目独占 `<project>_pg` 卷和 `<project>_test` 网络。既有 provisioner 设置每个容器 2 CPU、4 GiB 内存上限，两个项目合计上限为 4 CPU / 8 GiB，这不是预留内存。合同驱动的最低预检是磁盘可用 2 GiB、MemAvailable 1 GiB；本批准备时还按合计容器上限核对服务器容量。已知合成工件合计最多 128 KiB，独立卷、解包文件和证据需要额外空间，磁盘预检不是卷配额。

这些资源已经获授权并执行，但本轮止于全量观察，当前源/目标资源均不存在；名称和失败控制目录仍不复用。后续修复包必须选全新批次，实际执行前再次检查已有项目、网络、卷、全部路由及容量；若占用或前置条件失败，停止并保留证据。每批只执行一次，不复用失败目标或其他项目。

## 结果核对与下一任务

本轮已授权并运行：控制 `14b45cf0-e9f3-4beb-a250-ceb6f49d5a14`，源 `ed8d1bb5-6694-4108-894d-2db815841477` / `10.251.241.0/24`，目标 `38bbfe39-bd18-4e51-8697-e762a14e353e` / `10.251.242.0/24`。容器停机留卷，不复用本批目标；两份合成工件已在原授权范围内导出、下载并独立审查通过。

成功状态必须精确为 `CLIENT_CONTRACT_AND_FIXTURE_CAPTURE_PASSED_NOT_IMPORT`，退出码为0。还要核对完整 `result.json` 的封存/批次/资源身份、独立目标基线、原始工件哈希、源码未变、精确停机和留卷；文件摘要本身不能替代这些字段。

`artifacts/fixture.dump` 与 `artifacts/decoded.sql` 是本轮已知合成工件，放在独立工件目录。仅在本批的明确受控导出授权内获取它们；不导出数据库卷、凭据、原始 Docker 环境或其他批次证据。验收失败时先读取脱敏固定原因码，不重跑旧批次。

控制器核对实际工件字节和哈希，再由独立审查核对 TOC、完整 SQL、表/主键/两行及随机 restrict key 的实际输出形态。本轮审查已通过，任务1完成，启动任务2的不可变golden封存与严格变体拒绝；它仍有自己的实现、测试和独立审查门，不自动接受未来捕获输出。

任务5首次目标写入、十一项现场用例、完整恢复、独立备份目的地与 `CompleteBackup` 仍有各自的后续门。本轮合同成功不关闭 C4 整关。
