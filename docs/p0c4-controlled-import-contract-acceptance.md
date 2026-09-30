# P0-C4 Task 1：客户端合同与合成工件采集验收

2026-09-30。本说明属于[六任务施工单](superpowers/plans/2026-09-30-p0c4-controlled-import.md)的任务1；规格见[受控小型 dump 导入设计](superpowers/specs/2026-09-30-p0c4-controlled-import-design.md)。

## 当前状态

任务1本地实现 `43cd2df8b10d3eb98385d7c5ddd785152d474e5f` 和修复 `4fad8e681060c8bd4575b033f860e97054cccc2e` 已经独立审查/定向复审。共享停机截止、私有 Rust 接口可见性和 UUID 变体校验问题已关闭。首轮现场执行返回 `CONTRACT_ADMISSION_OR_EVIDENCE_REJECTED_NOT_IMPORT`、退出1；授权包的中文路径清单编码错误已用未改动的接收端在本地复现。封装修复 `305a54f580ba5efc6a42f32fd80dc1b5b00d6707` 已独立复审通过，无新增 Critical/Important 问题；服务器的原始异常栈尚未取得，不能将本地复现冒充现场诊断。任务1未关闭，任务2–6未启动。新 ZIP 和新资源仍待单独授权，旧失败批次不重跑。

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

首轮三个脚本已相邻安装到 root 所有、0700 的 `/var/lib/knowweave-c4/tools/p0c4-controlled-import-task1-045544f/`。本次只修封装器，三份脚本字节不变；新批次拟复用此已安装目录，执行前再次核对模式、所有者、单独授权的 SHA-256 和新 ZIP 的 manifest 成员，不重新上传或覆盖脚本。新 ZIP 安装到 `/var/lib/knowweave-c4/incoming/` 的新文件，模式 0400。不能使用 Windows 工作副本替代 Git 原字节。所有其他依赖仅从封存 ZIP 解出，旧大 runner 的模式和行为不修改。

三个复用脚本 SHA-256 分别为：合同驱动 `913c882301490d6cedced06e1b83a7c4728ed60d2abcf9835082cce5b1de8c4c`，fixture helper `a8fef75b20d6f46539bb458ac71861be8aaf2c1929607bd6729808e1f24d5a48`，archive helper `53b391cca520347803399828e1d0a2fcb6a1563eb1304be4319b00565f0a2daf`。

## 拟用资源，仍待逐批授权

| 用途 | UUIDv4 | 项目 / 子网 |
|---|---|---|
| 控制批次 | `55571d76-02f8-4c47-aeb8-4d54fe29ad46` | `/var/lib/knowweave-c4/controlled-import/batches/55571d76-02f8-4c47-aeb8-4d54fe29ad46/`；无 PG 容器 |
| 合成源 | `2cde52c8-fbbc-4b9c-ae4b-dce60c21acf1` | `learning-system-p0c4-restore-2cde52c8-fbbc-4b9c-ae4b-dce60c21acf1` / `10.251.239.0/24` |
| 合同目标 | `ff1fde0c-506f-4ee8-8aec-f5f98803b2e2` | `learning-system-p0c4-restore-ff1fde0c-506f-4ee8-8aec-f5f98803b2e2` / `10.251.240.0/24` |

每个 PG 项目独占 `<project>_pg` 卷和 `<project>_test` 网络。既有 provisioner 设置每个容器 2 CPU、4 GiB 内存上限，两个项目合计上限为 4 CPU / 8 GiB，这不是预留内存。合同驱动的最低预检是磁盘可用 2 GiB、MemAvailable 1 GiB；本批准备时还按合计容器上限核对服务器容量。已知合成工件合计最多 128 KiB，独立卷、解包文件和证据需要额外空间，磁盘预检不是卷配额。

拟用名称和子网是准备值，不是资源预留。实际执行前再次检查已有项目、网络、卷、全部路由及容量；若占用或前置条件失败，停止并保留证据。每批只执行一次，不复用失败目标或其他项目。

## 结果核对与下一任务

成功状态必须精确为 `CLIENT_CONTRACT_AND_FIXTURE_CAPTURE_PASSED_NOT_IMPORT`，退出码为0。还要核对完整 `result.json` 的封存/批次/资源身份、独立目标基线、原始工件哈希、源码未变、精确停机和留卷；文件摘要本身不能替代这些字段。

`artifacts/fixture.dump` 与 `artifacts/decoded.sql` 是本轮已知合成工件，放在独立工件目录。仅在本批的明确受控导出授权内获取它们；不导出数据库卷、凭据、原始 Docker 环境或其他批次证据。验收失败时先读取脱敏固定原因码，不重跑旧批次。

控制器核对实际工件字节和哈希，再由独立审查核对 TOC、完整 SQL、表/主键/两行及随机 restrict key 的实际输出形态。审查结果通过后才关闭任务1、启动任务2的不可变 golden 封存；不自动把任何捕获输出认作 golden。

任务5首次目标写入、十一项现场用例、完整恢复、独立备份目的地与 `CompleteBackup` 仍有各自的后续门。本轮合同成功不关闭 C4 整关。
