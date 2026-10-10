# P0-C4 维护闸补充验证

更新：2026-10-03。上层施工单仍处于 **Task 3：维护窗、数据库 dump 与资产保护**。受控小型 dump 的 Task6 已在固定合成夹具范围内关闭；它不能关闭整库恢复或整个 C4。

## 最新状态：live source admission 本切片实际五门已接受 5/5（2026-10-03）

新批次 `06a0882f-5c7a-4fa5-ab86-252ece7732fa` 在基于 `0adab2e340c0980c120364a1214ebb53277e4405` 的未提交 `working-tree-green` 冻结快照上实际通过五个精确 PG18 用例，各 exit 0、1 通过、0 失败/忽略、stderr 0 字节。规范 result SHA-256 为 `98b0883bef44abe10259006e34b5481e322fac38511421b35c37d56afd74cd98`，控制端已读实际结果/精确日志并核对五个本批 PG 停止、留卷留内部空网、构建器移除及 pending 不存在。独立限定返回审查 Spec/Quality/返回接受均 Approved，Critical/Important/Minor 均为 0；只接受本冻结切片的五门 5/5。

私有 owned `SourceAdmission` 以同一认证连接取得固定 two-i32 数据库域 session lock；busy 在 ACL/日志修改前拒绝，guard 持续覆盖源端 SQL/catalog/journal/dump/seal/pin/release/补偿。五门证明不同 root/attempt 的 live 互斥、单连接 catalog/排空、Drop 关闭旧 backend 及 release/补偿持续持有；没有调用 `pg_dump`，release 前序摘要为 SYNTHETIC。机制、精确测试和输入 pins 见[源端维护准入](p0c4-source-admission.md)，构建计数见[C4 验证记录](p0c4-verification.md)。父 Task3、Task4/5、`CompleteBackup`、C4 和生产继续开放。

下列 c9f 专项属于旧提交 `07eaf416` 的历史接受，不自动转移到本候选；原三门未在本候选重跑，不能与新五门相加为新版本 8/8。

## 历史专项：三个真实 PG18 聚焦门已接受 3/3（2026-10-03）

全新批次 `c9f1cdaf-de98-4f69-96a1-0c6a479fc7cb` 的三个精确 PG18 用例各通过 1 项、失败/忽略为 0，main 退出 0，状态为 `MAINTENANCE_FOCUSED_GATES_PASSED_NOT_FULL_BACKUP_NOT_RESTORE`。这是父 Task3 的维护聚焦验收，不关闭父任务，也不推进 Task4/5。

控制器核对用户粘贴的完整 root 结果 JSON，规范序列化 SHA-256 为 `698eebc766e6aa137f7622111c456422f23d451ad8eee48a00f89275e216fc71`，源码提交 `07eaf416af795cbad309bf5436b27c2289e6a06c`、输入 pins 和运行前后源码摘要一致。此为用户回传的独立哈希/身份核对，**不是控制器直接读取 root 私有原始结果**。另一次普通 hans SSH 只读资源观察独立确认三个精确 PG 容器均停止、对应卷保留、内部网络保留且无附着、构建器已移除；资源观察 SHA-256 为 `e9782ac6991a88d6df612ee3b59509b7dc980ed3116b103fc65931ad9af31530`。未重跑用例或操作外来资源。

独立限定返回审查结论为 `PASS_THREE_FOCUSED_PG_GATES_ONLY`，Spec/Quality Approved，Critical/Important/Minor 均为 0；只接受本批三门。三项均没有执行 `pg_dump`；只核验固定容器内工具路径/版本。释放日志失败用例的前序摘要为 **SYNTHETIC 合成摘要**，不能作为真实 dump、源端封存或独立目标收据。以下旧失败和修复阶段的 0/3 均为历史状态，当前本专项为 **已接受 3/3**。

## 本轮修订

源端维护闸原先使用 `pg_stat_activity` 检查其他会话。PostgreSQL 的预备事务在执行 `PREPARE TRANSACTION` 后脱离原会话，仍可在其他会话中提交。因此，仅“会话数为零”不足以证明事务排空。[PostgreSQL 18 官方说明](https://www.postgresql.org/docs/18/sql-prepare-transaction.html)

真实源端检查现在同时读取 `pg_catalog.pg_prepared_xacts`，明确限定 `database=current_database()`。只有成功解码的零计数才允许继续；非零、异常计数、查询或解码失败均拒绝。预检拒绝发生在写维护日志和修改 CONNECT 权限之前。共享检查同时用于排空、封存前后核对、放行补偿和 `release_ready` 恢复。

公开 `GateInspection` 构造方式保持不变。补丁不要求全服务器关闭预备事务，也不自动处理已有预备事务。[数据库范围字段](https://www.postgresql.org/docs/18/view-pg-prepared-xacts.html)用于避免其他数据库的事务造成误判。

## 验证与范围

| 验证 | 结果与含义 |
|---|---|
| 新增计数边界 | 行为 RED：1 通过、2 失败；修复后 3 通过。包括非零、负数、缺失、错误与零计数。 |
| Windows 可执行库与维护合同 | 库 139、维护合同 4、非 Linux 日志拒绝 1，共 144 项通过。Windows 日志用例不证明 Linux 持久化。 |
| Windows 格式和严格 package Clippy | 通过；不覆盖 Linux 专用 SQL 查询及 PG 用例编译。 |
| 隔离运行器 | 当前维护运行器 20 项定向检查通过；覆盖准入、合法网络/卷名、停止态资源核对、未知凭据/标签不落盘、完整库存及固定镜像标签继承。既有隔离合同 13 项通过；这些本地检查均不启动 Docker 或 PG。 |
| 真实 PG18：当前库预备事务 | 已通过：预备事务在原连接关闭后仍存在；当前库预检在 ACL/维护日志修改前拒绝，ACL、控制根和 pin 根不变。 |
| 真实 PG18：其他库预备事务 | 已通过：另一库持有预备事务不误阻塞为空的当前库；持有事务的另一库仍拒绝。 |
| 真实 PG18：释放日志失败 | 已通过：日志失败后的补偿 REVOKE 和显式重新关闸；前序摘要为 SYNTHETIC，不证明真实备份或完整放行链。 |

Linux 验收使用三个全新隔离 Compose 项目、新 PG18 卷、私有目录及运行前重新核对的子网。显式启用 `max_prepared_transactions=16`，每个 PG 限制 2 CPU/4 GiB；离线构建器限制 4 CPU/8 GiB，无网络、无 PG 凭据、只读源码。执行精确测试并核对退出码和实际计数，结束后按创建的容器 ID 停机留卷留证；不复用旧批次。

执行入口是 `scripts/p0c4_maintenance_gate_acceptance.py`。封存的源码 ZIP、清单、原样脚本、批次和运行单分别核对 SHA-256；具体服务器命令保存在本批私有运行单，sudo 密码仅在用户服务器终端输入。本批实际回传已核对，三个聚焦 PG 门为 3/3；旧批次不重放。

全局 Docker 盘点只读取碰撞检查所需字段；环境、命令和挂载值仅用于内存核验，持久记录只保留计数、身份和状态。停止后再次确认本批网络没有残留或外来附着，不操作外来资源。本批只核验固定 PG18 容器内的 `pg_dump` 路径/版本，没有调用工具；不据此声明宿主可执行 dump 或已有整库 dump 产物。

## 历史：首批预检失败与模板修复

首个用户执行批次在 `preflight` 返回 `OBSERVATION_EXIT`，实际数据库用例为 0/3，源码前后摘要一致。随后通过普通 SSH 的只读命令复现：固定构建镜像省略了可选 `Config.Volumes` 字段，Docker 模板直接访问该字段会退出 1。

修复仅把该字段改为 Go 模板的 `index` 取值；ID、RepoDigests、Env 和后续镜像合同保持原样。PG 仍必须声明唯一的 `/var/lib/postgresql` 卷，构建镜像仍必须没有卷声明。最终模板在两种固定镜像上实际执行均退出 0、stderr 为空，身份、环境字段和卷合同核验通过；18 项既有纯检查和两文件语法检查通过，限定独立复审无新增问题。

上述只读结果证明模板修复，尚未验证离线构建和三个真实 PG 门。旧失败批次及证据保留，后续只用新源码包和全新批次执行。

## 历史：固定构建镜像标签继承修复（2026-10-03）

后续用户回传止于 `BUILDER_IDENTITY`，cases 为空，源码未变；用户粘贴结果不等于独立读取 root 私有结果。普通用户在全新、从未启动的构建容器上完成限定诊断：八个非标签身份谓词匹配，旧“仅批次标签”谓词失败；镜像基线标签为 1 个，容器标签为 2 个，继承基线加本批标签的精确合并匹配。诊断容器按精确 ID 移除，未执行编译或 PG；旧失败批次保留。[Docker 标签继承规则](https://docs.docker.com/engine/manage-resources/labels/)

运行器现在从预检的不可变固定镜像观察中，以可选 Go `index` 读取 Labels；缺失/null 视为空，其他非字典或非字符串键值拒绝。容器标签必须精确等于该内存基线加内部生成的本批标签；批次值覆盖同名镜像标签，额外、缺失、改动标签及外来批次均拒绝。基线不取自容器或外部参数，不硬编码镜像标签。镜像 ID、摘要、环境、卷及构建器权限、网络、挂载、用户和命令限制继续生效；未知标签值与环境/命令值只留内存，持久记录仍为计数、身份和状态。

行为 RED 为 1 项失败，明确复现旧 `BUILDER_IDENTITY`；修复后维护运行器 20/20、既有隔离合同 13/13，以及两文件语法和启动前后基线传递检查通过。独立限定代码复审为 PASS，Critical/Important/Minor 均为 0。

控制器随后经普通 SSH 完成新构建容器的真实 stopped-only GREEN，SSH 退出 0，记录于 `task3-builder-stopped-identity-green-observation.json`：冻结源码中的完整修复 validator 接受实际完整 facts；额外、修改基线、缺失基线和外来批次四种无效标签均拒绝。最终 image-inspect 模板在固定 PG 与 Builder 镜像上通过；新容器从未启动，并按精确新容器 ID 删除。该证明限定于镜像模板和启动前身份/隔离核验，Linux build/discovery 仍未执行，三个真实 PG 门仍为 0/3；不关闭父 Task3。

本地 Python scripts discovery 292 项通过、退出 0；仅为本地 Python 证据，不计为 Linux/PG 或整项目验收，未重复 Rust 检查。

## 后续关闭条件

当前 live 准入切片已提供同库跨 attempt/root 的存活尝试互斥；会话消失后锁释放，固定 source control-root 的独立持久身份 pin 与跨根崩溃恢复身份仍未完成。所有中断阶段的完成/放弃恢复、真实源端封存产物到独立目标的组合验收继续属于父 Task3 的后续工作。

用户已将独立存储故障域安排延后，当前继续代码和单机隔离范围。父 Task3 还须完成可信固定源根的持久身份 pin、广义中断的完成/放弃恢复、真实源端 dump 与全部 ready assets/index/保留保护到独立目标的组合校验和完成收据；三个任务复选框均保持未勾选。Task4 干净实例整库/原件恢复与 Task5 完整 Attention/授权/租约及故障注入仍待后续验收，独立持钥见证与 `CompleteBackup` 正例也未完成；不签发 C4 已验证或生产状态。

## 历史：POSIX 原子写入模式修复（2026-10-03，聚焦门执行前）

首个到达 PG 启动的维护批次 `8c440305-7a02-447c-a5aa-3471669836c1` 止于 `PG_READY_TIMEOUT`，三个实际 PG 门未执行。普通 SSH 对精确停止容器的只读观察确认 initdb 和临时服务启动成功，随后非敏感初始化脚本出现 `Permission denied`；该观察没有直接 stat 旧主机文件模式，也没有读取 root 私有结果。操作包装脚本设置 `umask 077`。控制器以普通 hans 用户在全新目录执行旧真实 writer 的独立 Linux 探针，实际复现请求 `0444` 得到 `0400`、退出 1，私有 `0600` / PG 副本 `0400` 未变。

运行器现在以仅所有者权限创建独占 no-follow 临时文件，写入并 flush 后在同一打开的 POSIX 描述符上设置和验证精确请求模式，再执行原有文件 fsync、独占硬链接发布及父目录 fsync。模式设置异常或核验不符会在发布前拒绝并移除临时文件；全局 umask、PG 身份/网络/时序和秘密准入规则未调整。Windows 纯检查路径跳过 POSIX 描述符模式操作。

最终本地限定 Python 共收集 36 项：维护运行器 20 项通过、3 项 POSIX 行为测试在 Windows 跳过，相邻隔离检查 13 项通过，退出 0。控制器的第二个全新目录 Linux 探针退出 0，真实验证 `0444` / `0600` / `0400`、原目标拒绝替换且原字节保留、临时文件消失；注入 fchmod 异常及无效 no-op 均拒绝且无目标/临时文件。探针只嵌入 AST 提取的真实 `atomic_write` / `sync_dir`，使用固定无敏感字节、普通 uid 1000；未触及 root 私有结果或 Docker 资源。此为独立 Linux 写入行为检查，不是完整 Linux 单元套件。旧失败批次和证据保留；全新 PG 三门仍为 **0/3**，父 Task3、`CompleteBackup`、完整恢复、C4 和生产仍待验收。
