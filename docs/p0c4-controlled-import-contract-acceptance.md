# P0-C4 Task 1：客户端合同与合成工件采集验收

更新：2026-10-01。本说明属于[六任务施工单](superpowers/plans/2026-09-30-p0c4-controlled-import.md)的任务1；规格见[受控小型 dump 导入设计](superpowers/specs/2026-09-30-p0c4-controlled-import-design.md)。

## 当前状态

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
