# P0-C4 固定 fixture 导入：静态实现与后续验收

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


**首项实际导入的准入受阻，实际仍为0/11。** 全新操作批次 `6390746c` 在 `PREREQUISITE_RECEIPT` 返回 `Identity`／`REAL_IMPORT_SUCCESS_PRE_ATTESTATION_REJECTED_NOT_RUN`，退出1，没有启动真实导入；只读Docker名称盘点未发现该控制、源或目标UUID对应资源。静态诊断已确认读取端要求的四个 `build` 字段不在真实前置结果中，之前的本地模拟结构掩盖了该接口不一致；现场首次失败谓词仍未确认，控制者未读取root私有结果。只修订新的读取操作器，沿用真实严格退出码并核验原始all/ignored日志，不改旧收据、生产源码或验收门槛；旧批次不重跑。

**读取端修复的现场结果：前置收据阶段通过，停在安装输入核验，实际仍0/11。** 真实生产者结构回归复现了旧读取端拒绝；修复后15项定向检查退出0，规格／质量／上传器静态复审PASS，I-RECEIPT-01已处理。新操作批次 `10dc6556` 使用全新源/目标及 `10.253.102/103.0/24`，命令19594字节、SHA-256 `af140f7410d50feb42327ab6aaf50d947dbd31ea81bbc013b09b97233c6c019c`，服务器两次读取核验一致；root终端已执行准入检查，但真实导入未启动，仍0/11。前置收据按真实七字段结构读取，严格退出码及原始all/ignored日志独立核验；旧失败脚本、收据和批次保留。

**最新准入诊断（2026-10-01）：** 操作批次 `10dc6556` 返回 `Identity`／`INSTALLED_INPUT_PINS`，退出1，真实导入未启动。用户在root终端只读核对：已有runner、fixture、birth helper和ZIP的大小、权限及SHA-256均匹配；`p0c4_restore_pin_acceptance.py` 不存在可用的普通非符号链接文件。编译阶段只安装三份脚本，而导入准入要求四份。控制者未直接读取root文件；普通SSH元数据核对未发现该控制/源/目标UUID的Docker资源。已审查的窄修补会从同一封存ZIP提取四份原样Git字节到全新0700工具目录，旧目录/脚本/批次保留，不放宽身份或验收门槛；新候选尚未运行。

**安装输入修补已审查并上传，等待首项实际导入（2026-10-01）：** 新操作批次 `91a34686-ad94-4ad9-9c40-c70021b4e4dc` 使用全新源/目标和 `10.253.130.0/24`、`10.253.131.0/24`。修补后的操作脚本先核验原收据、完整ZIP、源码和二进制，再从同一ZIP向新root私有工具目录排他安装四份原样脚本，旧三份工具目录保留。本地定向检查12通过／0忽略，独立规格、质量及上传器静态审查PASS，I-INPUT-01已处理，控制者核对35份审查输入。命令24995字节、SHA-256 `d5219773880fce7c0d74c221e2480d49bc5b4f255d19cfa6617297179657d7c5`，上传后服务器两次读取核验一致。尚未执行本轮root安装或真实导入，实际仍0/11；运行时再次核验资源未占用，通过后再核对完整结果、停机留卷和哈希，才能计入首项success。本地模拟/结构检查不替代Linux/PG18现场结果，C4整关、CompleteBackup及生产恢复仍未验收。

**2026-10-01 最新现场进度：全新 root 批次 `03d1c8c8` 前置门通过，格式与严格 Clippy 通过，普通库129通过／19忽略，Linux journal no-follow/fsync专项1通过，退出0；实际导入仍为0/11。** 用户终端回传结果 SHA-256 `a67ab567f8bffefcb66d71f58a381f900e32fe34727b738af3ce07a9163f415a`，源码和二进制未变、builder清理确认、pending消失。封存源码提交为 `1cc3aeca3485caa7ac910e9f18411187afba6d8f`。来源为用户终端摘要；控制者未直接读取root私有完整结果或日志，后续实际导入操作先独立核验该收据。接下来在全新源/目标、PG18卷及子网运行11项实际case；C4、CompleteBackup及生产恢复仍未验收。

历史前置门记录（旧失败批次、普通用户诊断与局部审查保留）：**当前已完成固定 fixture 导入的本地实现与代码审查；Linux 新源码的格式、严格 Clippy 和 no-run 已通过。批次 `9583c1ba` 的普通库测试为125通过／1失败／19忽略，journal尚未运行，实际导入仍为0/11。** 失败定位到 `blocked_stdin_obeys_total_deadline` 的清理返回值断言；历史具体错误值、调度触发和测试退出码未被脱敏诊断记录。后续普通用户 Linux 隔离验证证明清理返回 Deadline 可同时完成精确子进程与 reader 回收，据此修正测试合同并补齐失败路径；最终格式与严格 Clippy退出0、管道26通过、库129通过／19忽略，独立规格与质量复审PASS，I1/I2均ADDRESSED。新源码仍须在全新root批次通过普通库和journal前置门；不能将普通用户诊断当作root验收。默认/发布构建没有新写入口，CompleteBackup、生产恢复和C4整关均未验收。

2026-10-01 本地修复验证：共享 stop/close 修复后，P0-C4 Python 一次完整回归 258/258，候选 journal Windows 模型 4/4，格式检查退出0。其后补齐诊断输出管道失败与 builder 清理截止传播，仅作定向验证：32/32，最后加入成功清理合同覆盖后26/26；未重复全量回归，不能称最终所有源码全套通过。修正 Linux ignored journal 测试的构造函数同名变量，不将 Windows 模型当作 Linux 编译证据。历史 M3 的 37/38 失败及清理资源警告保留；确定性测试证明原关闭分支会跳过 reader 清理，新实现保留未完成的精确子进程、reader 和管道所有权。一次原计时测试诊断未重现历史 PID39808 的调度，不宣称已重建其原因。

隔离仍从最外层 stop 入口起使用同一个绝对15秒截止，包含早期克隆发现与已知源/目标停止。克隆发现超时或归属拒绝会保留失败，期限有余且没有未完成 CLI 时继续尝试已知精确 ID；未知克隆不得替换或猜测。截止或 wait 失败后无法证明回收完成时，批次和命令适配器不可用，禁止新 Docker 操作及正常结果发布，保留文件锁。命令行输出一次固定 `CONTROLLED_IMPORT_UNCONFIRMED_UNUSABLE_OWNING_CLEANUP`，同步持有这些资源，只作零等待的精确回收观察，并等待已有 condition 通知；这段失败所有权生命周期可能无界，不是延长隔离预算。所有子进程、reader 和句柄确证完成后才释放锁并退出1，绝不补发成功。不可服务的内核可能令失败进程及锁持续保留直至外部终止宿主；不能声称15秒内隔离已确认。

builder 从实际 run 起另保存该编译操作的原绝对截止；继承的精确 ID 清理配方中每个 Docker 命令均使用此截止、原 argv/env/输出上限。即使宿主 builder CLI 已回收，超过截止也不得发起新的容器发现/移除，cleanup 保持未确认、适配器不可用且不发布成功。此时容器可能仍在，保留批次证据并报告失败；没有未完成宿主句柄不等于容器清理已确认。

## 已批准的范围与来源

2026-10-01 用户明确要求“无需要我逐项确认，自己去做”，撤销此前逐文件/逐批次人工确认要求。已批准的 C4 固定合成 fixture、单机隔离验收范围内，控制者自主准备并记录每个 case 的 Git 同字节 ZIP/manifest、原样 root runner/helper 摘要及新控制/源/目标 UUID、项目、子网和卷；`wrong-endpoint` 仍需全新物理克隆 UUID/子网/卷。该变更仅减少人工确认，不放宽验证合同、审查、资源隔离或生产边界。失败批次不重跑、不清理、不复用；root/sudo 凭据只由用户在 Linux 终端输入。控制者负责封存、上传、完整可见命令、回传结果及独立 Docker 核验。

前置门1为 `CLIENT_CONTRACT_AND_FIXTURE_CAPTURE_PASSED_NOT_IMPORT`，控制批次 `14b45cf0-e9f3-4beb-a250-ceb6f49d5a14`，结果摘要 `505ab6179e3be1f1c8afb441fa694db8d5033f27e77a0940908de208bad96508`。这是已审查历史合同引用，不是每个新 dump 的预期摘要或导入能力。历史 root 原结果、用户终端摘要、可公开合成工件及独立下载/实查证据必须分别注明来源，不能把未读到的 root 私有文件称为已独立读取。

固定 PostgreSQL 镜像：`postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d`。固定 builder：`sha256:fb91f085b6002b8f75570993722a762579ad392e15c390e8161ffb746c858b9b`；每次实际执行前复核。

## 编译与资源预检

经封存验证创建新的 root 私有批次后，`ImportBackend._budget()` 仅作容量与镜像预检，`ImportBackend._preflight_import_builder()` 是可分离的编译/列举入口：内部使用源和目标均为 64 个 `0` 的无权占位出生 pin，离线 `cargo test --locked --offline -p learning-backup --lib --no-run --message-format=json`，随后宿主执行测试二进制 `--list --ignored`，核对下表 11 项全部且各一次。它不创建 PG、网络、卷，不读取 DB 凭据，不运行 ignored 测试。控制者可在批准范围内创建的新 root 隔离批次只调用此方法；不增加宽泛 CLI phase。此方法本身不会代替另需的 Linux 严格 Clippy、库检查和 Task3 no-follow/fsync 门。

2026-10-01 编译专用批次 `a3f5719a-afe4-43e2-a78b-1ab1d9fc049c` 返回 `LINUX_COMPILE_AND_ELEVEN_TESTS_LISTED_NOT_EXECUTED_NOT_IMPORT`，`LINUX_COMPILE_EXIT=0`。用户摘要记录 11 项名称、未执行测试、pending 消失和 builder 清理确认；结果 SHA-256 为 `c1ccabb430aa2e4ec64d1a9538933f66102459cd412eb7a4690bd38312f4fa49`。

随后前置批次 `b2af0b1a-a81b-4c82-8cb3-5e23e1b74846` 的用户终端返回 `LINUX_PREREQUISITES_FAILED_NOT_IMPORT`、`LINUX_PREREQUISITES_EXIT=1`，阶段 `STRICT_LINUX_FORMAT_CLIPPY_COMPILE`，原因码 `Io`，实际用例0、无自动重放。`Io` 是通用异常映射，不能据此判断磁盘故障或哪项命令失败；默认库测试和 journal 专项尚未运行。独立 SSH 核对本批 builder 精确名称与标签均无残留，旧批次和封存源码保留。

用户已在 Linux 终端执行经过审查的只读诊断。有效标记序列为 FORMAT_BEGIN、FORMAT_PASS、CLIPPY_BEGIN，故格式门通过，失败停在严格 Clippy；no-run、默认库测试和 journal 专项尚未开始。诊断列出7处 Rust 源码位置，静态白名单仅确认 `clippy::collapsible_if`，未穷举其余 lint，也未读取完整错误文字；不能把通用 `Io` 或 broad tool-error 标志当成磁盘/依赖故障证据。诊断脚本 SHA-256 `35af222830a64199c1cf8d07cbfd984e0db518792afe5cf78105caec583f8f19`，原 stderr SHA-256 `3ec07ec898e37015a2046d6b1a6aa0ef1829d2fc9b58aa800855fcc2f1488b91`。

修复提交 `d70e6eb3ec7febb197e85bce5e0246abf30311fc` 仅调整4个 Rust 文件中的测试辅助代码编译范围、一个可变绑定及两处条件写法；独立规格/代码质量审查均 PASS，未改变导入权限、固定夹具、哈希、截止时间或清理语义。Windows 包内库119/119、格式退出0、严格 package all-targets Clippy退出0；格式仅有退出码文件和报告称空输出，没有保留的输出日志。Linux普通库及测试构建尚待全新隔离前置门，实际导入仍0/11；旧失败批次不重跑。

新批次 `9583c1ba-2d08-4d84-b21d-fe8937c09610` 的用户回传只读诊断记录六项FORMAT／CLIPPY／NO_RUN标记按序各出现一次，全部完成；唯一失败测试位于 `stream.rs:674:37` 的 `kill_and_wait().await.unwrap()`。库stdout20448字节，SHA-256 `cf2b34d1873352580262a7cfe21e6dde75986bf3d99e8aac3ad608a177555fea`，stderr为空。诊断仍明确 `test_exit_code=null`、summary不一致标志及 `root_cause_established=false`；控制者未直接读取root私有完整日志，保留这些限制，不把确定性复现当成历史错误值证明。

后续窄修补只在测试模块内调整清理断言及其资源收束用例。阻塞stdin后仍必须返回Deadline；清理只允许Ok或Deadline，并核对精确子进程ECHILD、句柄消耗及reader完成。新增失败路径先保存拒绝结果，收束本用例资源后再断言；辅助进程首次确认超时必须保留失败和退出通知，确认完成前不得删除通知。独立审查所提I1/I2经两轮修复复审全部ADDRESSED，无新增问题。最终源文件LF SHA-256 `5712042c3de003e994cf7551be799dd5475f77516b11f0aa787f237d47d4b478`；最终Linux隔离日志为格式0、严格all-targets/all-features Clippy0、管道26／库129通过、19忽略。延迟确认用例实际观察通知在恢复期间保留、辅助进程见到通知、首次超时仍拒绝以及最终本用例标记清理。此证据不承诺任意进程树生命周期、永久调度/文件系统故障恢复，成功done仍只是辅助进程协议确认。

新canonical前置门保留原截止、资源所有权、持久发布和严格通过条件，只使用全新批次。批次03d1c8c8已按用户终端摘要通过库129／19和journal1项，11项实际导入仍未执行。旧失败批次、源码和日志保留且不重跑。本轮不扩大导入权限、生产算法或恢复范围。

实际 case 在全新源/目标出生证明签发后另编译 `probe-live-build`，分别嵌入 `KNOWWEAVE_C4_IMPORT_SOURCE_BIRTH_SHA256` 和原 `KNOWWEAVE_C4_TARGET_BIRTH_SHA256`。源出生使用同一完整验证器（state/issuance-success/无 failure、目录 dev/ino、PG 身份/cast/ACL），不构造第二个目标导入 admission，也不接受环境中的期望 hash 充当能力。源 guard 与原 SQLx challenge 持有到生产和签发完成。预检二进制不能执行实际 case；不同源码或编译 pin 不复用二进制。每次列举/执行前后复核只读二进制 SHA，记录两份 pin、源码 SHA、builder ID、列举摘要和 exact 名称。

builder 固定 `--cpus=4 --memory=8g --memory-swap=8g`，无新增 swap 配额，网络 `none`、源只读、仅新 build 目录可写。runner 必须实查运行中精确 ID、名称/label、镜像、CPU/内存限制、无特权与挂载，再保存脱敏投影；不能用 argv 冒充实际限制。之后观察/清理绑定该 ID，名称替换、过早退出、观察失败均拒绝。复用已审查 helper 的唯一 cleanup 流程，OpenClient 保有 CLI/双管道到完成或终止并回收。

容量预算为两份 build 目录各 16 GiB、每个 PG 估算 2 GiB、磁盘余量 2 GiB：普通 case 要求至少 38 GiB 可用磁盘，错误端点 case 40 GiB。这是估算预留，不是磁盘配额。内存预检为 builder 8 GiB + 每个可能并存 PG 4 GiB + 1 GiB 余量，普通 case 至少 17 GiB、错误端点至少 21 GiB MemAvailable；编译专用准备当前沿用该保守 case 门槛。执行前刷新容量/占用/路由，不能沿用历史空闲量；计时只用同一主机单调时钟。

## 私有输入与单 writer

仅 `#[cfg(all(test, target_os = "linux"))]` ignored 路径执行。非敏感环境只给新 root 路径、case 和精确资源身份；凭据仅原 SQLx 控制会话从既有私有文件读取。源生产者固定生成 `public.c4_import_probe(id integer NOT NULL,label text NOT NULL)`、固定主键和 `(1,'alpha')`/`(2,'beta')`。它实际执行固定 `pg_dump`，create-new 写入后通过 no-follow 句柄读入不可变快照，按真正生产字节的长度/摘要验证；实际 TOC 与完整 golden 解码消费同一快照。普通 `freeze_dump` 仍无签发能力。历史 dump SHA 仅审计。

子进程保持固定精确 64hex 容器 ID、`/usr/bin/docker exec --interactive --user 999:999`、`/usr/bin/env -i`、`LC_ALL=C`、`PGCONNECT_TIMEOUT=10`、`PGPASSFILE=/dev/null/knowweave-c4-passfile-disabled`、绝对 PG18 客户端、本地 socket/5432/learning_admin/出生 UUID 库；没有 caller SQL/flags、shell 或 TTY。dump/解码 SQL 各最多 64 KiB；writer stdout/stderr 各 8 KiB，在读取期间限额。解码 15 秒、writer 45 秒、READY/PRECOMMIT 各 10 秒且不延长总截止；writer 消失观察 5 秒，隔离另 15 秒。错误端点双容器共享一个隔离绝对截止。源生产、目标准入、writer 和清理总 harness 另设 360 秒，只是外层失败上限，不延长任何内部截止。

固定 `SET LOCAL` 为 statement_timeout 10秒、lock_timeout 5秒、idle_in_transaction_session_timeout 30秒、transaction_timeout 60秒。完整 golden 仅允许配对 restrict key 变化；不支持任意 SQL 清洗。原 Task5 同 writer 双锁准入、持久 attempt、提交意图、首次 COMMIT 发送边界、非放弃式管道收束及 guard 所有权算法继续使用。测试钩子只允许下表固定故障。

## exact case 白名单

所有测试的前缀为 `restore_preflight::target_binding::controlled_import::live_tests::`。一次调用只选下表一项 `--exact ... --ignored --nocapture --test-threads=1`；真正退出0、1 passed/0 failed/0 ignored、唯一完整结构化检查点 receipt 才能作为该 case 候选证据。模型输出和单独非零退出不构成负例通过。

| case | exact 测试后缀 | 必须实际观察 |
|---|---|---|
| success | live_controlled_import_commits_fixture | 同 writer、attempt/intent 先同步、唯一 COMMIT、提交确认、writer 消失、独立精确两行/主键/PG18两项 NOT NULL 约束及行摘要 |
| ready-eof | live_controlled_import_ready_eof | READY 后真 EOF，未发 ROLLBACK/COMMIT；无 attempt；停机前 writer 消失、独立零对象 |
| precommit-eof | live_controlled_import_precommit_eof | 已发送 DDL/COPY、PRECOMMIT 后真 EOF；attempt 留存、无 intent、独立零对象 |
| precommit-cancel | live_controlled_import_cancel_before_commit | guard/原事务仍在，取消先接受，未提交、无 intent、独立零对象 |
| ready-restart | live_controlled_import_restart_before_ddl | READY 后 DDL 前精确新重启，旧 guard/lease 拒绝，无 attempt/DDL，不重启取证 |
| sql-error | live_controlled_import_sql_error | 固定除零 SQL 实际报错，attempt 留存，无 COMMIT/intent |
| copy-truncated | live_controlled_import_copy_truncated | 固定 COPY 行截断实际报错，无盲 ROLLBACK/COMMIT，attempt 留存；不强称零提交已读回 |
| attempt-sync-failure | live_controlled_import_attempt_sync_failure | 真实 BackupDir 创建/写入后的固定 fsync 边界故障，无 DDL/COMMIT，已产生条目保留；不是内核 fsync 故障实测 |
| commit-intent-sync-failure | live_controlled_import_commit_intent_sync_failure | 同上提交意图同步边界故障，初始/部分意图条目保留，未发送 COMMIT |
| commit-unknown | live_controlled_import_commit_confirmation_lost | intent 先持久，实际发送 COMMIT 后不发送确认查询、收束真实连接；结果未知，无回滚或重试声明 |
| wrong-endpoint | live_controlled_import_same_id_wrong_endpoint | 新物理克隆同 ID/OID，原控制事务双锁复验且克隆无匹配锁，实际 writer 固定合并身份断言报错；双端零对象、无 attempt/DDL、双容器精确停机 |

错误端点仅从有界 stderr 识别完整固定主 ERROR 行 `psql:<stdin>:<正整数行号>: ERROR:  KW_C4_IDENTITY` 后的 LF；context 回显、前后缀、截断及超限均不构成证据。仅输出布尔观察，不保存原始 stderr。此标签本身表示组合身份谓词拒绝，必须和独立克隆/双锁事实合并判断，不能单凭标签断言具体哪一项条件失败。该格式仍需固定 PG18 实际 case 证实，未观察则失败。

## 结果与保留证据

所有正常/负例保留源/目标/克隆卷，停止精确容器，保留 candidate attempt/intent、不允许重试。runner 独立读取有界私有 marker，核对数据库、类型/版本、dump/SQL/出生摘要、同 writer 和初始 attempt 摘要；记录 marker 实际阶段与摘要，不将意图等同已提交。源 `decoded.sql` 的随机 restrict key 可与目标独立解码不同，两者摘要各自注明。

根批次保留源码/build SHA、列举/实际测试摘要、编译前资源观察、marker、合成 dump/TOC/源解码工件；编译/测试原始 stdout/stderr 只存 `*.private.log`。结果只序列化固定非敏感码与审核过的摘要/身份。最终 JSON 通过私有 pending、fsync、不可替换发布、父目录 fsync 和删除 pending 完成；返回摘要再核对最终字节并实际检查 `pending_absent`。持久文件成功字段单独不代表验收，必须有该外部发布事实。

控制者还需用户 root 回传完整脱敏 JSON、result/inspection 摘要、`PENDING_ABSENT`，并独立只读核对 Docker 精确 ID 停机/留卷。成功唯一正例标记为 `CONTROLLED_FIXTURE_IMPORT_PASSED_SINGLE_HOST_QUARANTINED_NOT_FULL_RESTORE`；固定负例用独立 expected-rejection 状态。提交未知永不报告 rollback，未确认停机永不报告通过。本地审查与 M1/M2/M3 修复审查已关闭；新 Linux 全部门和逐 case 现场证据仍未关闭，本施工单不能据静态审查完成。

Task3 的 `restore_preflight::target_binding::controlled_import::candidate_attempt::tests::live_candidate_journal_no_follow_and_fsync` 已在全新root批次03d1c8c8返回1项通过，用户终端摘要与root结果哈希已登记；控制者未直接读私有原日志。Windows模型不替代该门。本地旧 M3 已以确定性关闭分支、真实待取消子进程及命令行持有测试完成修复和独立复审；原失败证据保留，PID39808 的历史调度仍未知，实际 Linux 行为继续单独验证。
