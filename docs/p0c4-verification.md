# P0-C4 完整备份与干净恢复验证记录

**状态：实施中；未验收；非生产。**

## 基线与范围

- C3 已验证基线：`4507f3f55f860a6d3afed2cefe2bce27f7ff6b7c`。
- C4 工作分支：`feat/p0c4-backup-recovery`；工作树：`D:\codex\DeepLearning\.worktrees\knowweave-p0c4`。
- 目标是完整 PostgreSQL 权威数据、所有 ready 原件、角色/模式重建信息和干净实例恢复；C3 融合快照不是整库备份。
- Linux 服务器仍须每份精确源码包单独确认后，在新隔离 Compose 项目验证。没有通过证据前不写 `P0_C4_VERIFIED`。

## 当前门槛

| 门槛 | 结果 | 证据或待办 |
|---|---|---|
| C3 基线源码可编译 | 已通过 | 本地 `cargo test --offline --locked --workspace --no-run`，2026-09-28，退出码 0 |
| Rust 格式基线 | 已通过 | 本地 `cargo fmt --all -- --check`，2026-09-28，退出码 0 |
| C4 Task 1 清单/契约 | Linux PG18 专项通过；C4 全链未验收 | `691fd4d` 静态复审无阻断；ws3 的隔离 runner 返回 `TASK1_PG18_PASSED_NOT_C4_ACCEPTANCE`，按固定测试计数验证 9/2/1、格式、严格 Clippy 与新空库真实 PG18 专项；前两批失败证据保留 |
| C4 Task 2 安全复制/封存 | Linux 专项通过；C4 全链未验收 | `431e002`；ws3 的封存集成 5/5、库内故障 5/5（含 SIGKILL）、格式、严格 Clippy、工作区编译均退出 0，命令身份与源码哈希已核；内核级真实 fsync 错误未注入，使用同步边界故障钩子 |
| C4 Task 3 写闸/dump/保护/完成收据 | Linux 隔离候选门通过；原始结果审计中 | `c344a1e` 的三项目 runner 返回 `TASK3_CANDIDATE_GATES_PASSED_NOT_PRODUCTION`；全链仍未验收 |
| C4 Task 4 干净恢复 | 策略原语静态复审通过；只读预检复审未通过 | `48f493b` 的恢复策略已复审；`4e1cb54` 的 Complete-only 锁定预检仍须补齐空库、实例身份和路径祖先检查；真实恢复、Linux/PG 验收未完成 |
| C4 Task 5 全链与失败注入 | 未开始 | 四套旧版升级、工作区回归、整关验收 |

所有后续结果需记录源码提交与包 SHA-256、项目名、新数据库/卷、工具版本、命令、退出码、日志哈希、失败根因及清理状态。失败批次不可覆盖或改写为通过。

## Task 1 本地证据

- 源码 HEAD：`691fd4d`（后续文档提交不改变 Task 1 代码）；独立复审已核对三次 Task 1 代码提交，最后一轮静态结论无阻断。
- 实现者观察到两轮编译 RED：缺少契约/管理入口类型，后续缺少隔离预检/迁移指纹/容量函数；修复后纯契约 9/9、空库与角色前置条件 2/2、库单测 1/1 通过。
- 根任务复核 `cargo test --offline --locked -p learning-backup --test contract`：9/9，退出码 0；`cargo test --offline --locked -p learning-backup --test catalog_pg preflight`：2/2，退出码 0；`cargo test --offline --locked -p learning-backup --lib`：1/1，退出码 0。
- 实现者运行 `cargo fmt --all -- --check`、`cargo clippy --offline --locked --workspace --all-targets -- -D warnings`、`cargo test --offline --locked --workspace --no-run`，均退出码 0；根任务曾独立复核格式和严格 Clippy，均退出码 0。
- Windows 本机没有 PostgreSQL 命令或隔离 DSN；当时 `catalog_pg` 的真实数据库测试只编译未运行。后续已在全新 PG18/新数据库/新 Compose 项目中执行，核验 `system_user`、角色属性、复合类型探针及全部 ready 资产。
- 授权 Task 1 ZIP SHA-256 `ce62007afa82ae9b66c84d72251520669ef08ec72916b494082a11dbde652365`、882256 字节；Linux 隔离项目 `learning-system-p0c4-task1-static-ce62007a-ws1` 在启动 PG 前停止于 `fmt`（退出码 1）。只读诊断结果 SHA-256 `1b315ae497f300d6c92c2bdc8bbffd533fd96b7691abdc3b42af60e3aab0ab9b`、证据清单 SHA-256 `a3baf696a6d670276a8d48f0d2f3efc46b8c56e9e91cb75f0405dc22d50a0167`：源码前后与授权包相符；前三项分别退出 0。根任务只读审阅固定 runner，确认三项通过 `--entrypoint` 指定固定测试二进制且断言 9/2/1 测试计数；因此 Task 2 ws1 的继承入口问题不适用于 Task 1。根任务又在只读、无网络、自动删除的同一镜像容器中复现 `cargo fmt --all -- --check` 退出 1，明确错误是镜像缺少 `deploy/fixture-runner/src/legacy_snapshot.rs`，使 `b2_upgrade.rs:2` 的模块无法解析；不是源码格式差异。需用同一授权 ZIP 补齐镜像输入，在全新项目重跑格式、严格 Clippy 与真实 PG18；该失败不能算 Task 1 PG 验收。

- 同一授权 Task 1 ZIP 补齐测试镜像输入后，ws2 的格式、严格 Clippy 与 Compose 配置预检均通过；用户执行 root-only runner 得到 `failure_label=create-dedicated-db`。根任务静态定位到 runner 用含连字符的 UUID 生成专用库名，却在 `CREATE DATABASE`、`REVOKE`、`GRANT` 三处未将库名作为 SQL 标识符引用。ws2 失败证据保留且不在原项目重跑。修正后的 ws3 使用新源码目录、新 Compose 项目 `learning-system-p0c4-task1-static-ce62007a-ws3` 和 `10.251.214.0/24`；上传的三个驱动文件哈希逐一匹配，重新提取的 349 个源码文件清单 SHA-256 仍为 `76f88e0ee44e5191ebcb9e9595d0d3a8bb98560b08db19c1abd914148636f76c`，Compose 配置预检退出 0。独立子代理对 ws3 驱动与 Compose 静态复审未见阻断，随后再执行真实 PG18 门。
- 用户在服务器本机执行 ws3 root-only runner，返回 `TASK1_PG18_PASSED_NOT_C4_ACCEPTANCE`；专用库 `learning_backup_c4_task1_2b8a1252-54d5-48aa-b176-a9586a86bea3`，证据目录 `/home/hans/experiments/learning-system-p0c4/evidence/learning-system-p0c4-task1-static-ce62007a-ws3`，`result.json` SHA-256 `c3c8a35f87e5a457b850d6b61d4dbe2a9abe44dcef033c7d882537e451687497`，证据清单 SHA-256 `ff169cb20ad9a35132c0d08fb034bb0aea83880d6c7212ce58dcdc9d7ae7a712`。runner SHA-256 `852eb7b84f2e9c92cf31470c87145daa7046c95fcde8c6e692466d834ef2d631`，根任务在运行前核对远端字节哈希，静态确认它固定调用 9/2/1 专项、格式、严格 Clippy 和 PG18 测试并检查通过计数；运行后以只读 Docker 查询确认该项目的测试容器和 PG 容器均 `Exited (0)`。证据目录由 root 保管，根任务未独立读取私有日志；结果哈希与成功摘要来自用户本机 runner 输出。此结果只认定 Task 1 专项，不等于 C4 完整备份/恢复验收。

## Task 2 本地证据

- 实现提交 `4d8749b`、复审修复提交 `431e0022d583c62f5c0bb7b2d9131f424516aa46`；独立复审最后一轮无静态阻断。`Dir::rename` 原有调用语义保持；Task 2 仅产出 `.sealed`，没有完成收据。
- 根任务复核 `cargo test -p learning-backup --test sealed --offline`：Windows 拒绝门 1/1、退出码 0；`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --offline -- -D warnings` 均退出码 0。Windows 不编译 Linux-only 文件系统路径，不能代替 Linux 验收。
- 新源码包 `task2-static-reviewed-candidate.zip` SHA-256 `8f32ffbae3553739d36f85a1d4a0d64ccecc52326f69fb18181e0b2acb741170`、891075 字节、352 个 Git 跟踪文件；用户单独授权后上传，远端 SHA-256 再核完全一致。
- 最终 Linux 隔离项目 `learning-system-p0c4-task2-static-8f32ffba-ws3`（`10.251.212.0/24`）：镜像 `sha256:7a578b6119d90b1f24e5977a4dda56d455ca4402371287281a3aa6b2e9970f01`，镜像内 319 个源码/fixture 输入逐文件匹配授权包。`cargo fmt --all -- --check`、Linux 集成测试 5/5、库内故障测试 5/5、严格 Clippy、全工作区编译的退出码均为 0；日志核对了封存与 SIGKILL 测试名，源码树前后 SHA-256 同为 `1621a11aed86ffa42a063a9642dd2285903e815946e3409ed9d5746ab18e8d1e`，遗留项目容器 0。证据目录 `/home/hans/experiments/learning-system-p0c4/preparations/task2-8f32ffba/evidence-learning-system-p0c4-task2-static-8f32ffba-ws3/`；根任务只读核对 `result.json` SHA-256 `9d2d270a455772409f88168029e6ee6b74ffe33b1ed1bacb6a950cbdc2dc848c`、`inventory.json` SHA-256 `aca25db8a19b07c30a562b57e71590ee1e678a1a8b19cf214a527d1f8b539c71`，并直接阅读 `result.json`，状态 `PASS`。
- ws1 因继承旧 ENTRYPOINT 未执行目标测试；ws2 的 Linux 专项通过，但测试镜像漏拷固定 deploy fixture，格式/全工作区门失败。两轮均保留失败证据，最终 ws3 使用显式入口与完整输入重新验证。同步故障由测试钩子在父目录 fsync 边界注入；这验证错误处理协议，不等同真实磁盘掉电或内核 fsync 故障。C4 的独立故障域、数据库 dump 和恢复全链仍待后续任务。

## Task 3 本地证据与 Linux 待验收

- 实现提交 `6cd2315`、审查修订 `239c766`、三项目 Linux 验收脚本 `c344a1e`；独立复审核对写闸、Docker 状态/凭据探针、失败前证据捕获、失败卷保留，以及成功场景的 `pg_restore` 独立恢复和 ready 资产索引逐字段比对，未发现静态阻断。静态结论不代替 Linux/PG18 运行。
- 根任务本地复核隔离驱动 13 项测试、验收脚本 5 项测试、`cargo fmt --all -- --check`、`cargo clippy --offline --locked --workspace --all-targets -- -D warnings` 与 Task 3 Rust 契约测试，均退出 0；Linux 专项尚未运行。
- 用户单独授权的精确源码包 `task3-reviewed-candidate.zip`：SHA-256 `892adcedb12b9c9b310c9c0d9814c4289a6b406d100074ed35105d51169d0ca6`，3,401,873 字节，364 个跟踪文件；内嵌清单 SHA-256 `36ee82f1db74652e3e0d7102666e5dc7ef749b1fce10460ce1512f4ad11843b7`，引导脚本 SHA-256 `385d02c59b67d5abf8ca550090b496742555ee74e517b3ebd856b146438a6cce`。本地逐文件验证 ZIP 与清单；服务器 `/home/hans/experiments/learning-system-p0c4/incoming/task3-reviewed-candidate.zip` 上传前后 SHA-256 与授权值一致。
- 计划使用新 root 专用目录 `/var/lib/knowweave-c4`，固定批次 UUID `9b0ba3d9-adc3-4cc0-881c-51c0ee9f7160`，三套全新 Compose 项目与 `10.251.215/216/217.0/24` 子网；启动前必须再次检查宿主路由、Docker 网络、项目/卷/容器均未占用。此测试信任宿主和 Docker 管理员，不能证明抵御其恶意篡改。
- Task 3 成功门只证实源端闸、dump、资产索引及受控恢复比对。当前 `learning-backup` 只暴露 `SealedBackup`/`SourceLocalPin`，尚无独立目标校验后的 `CompleteBackup` 能力或完成收据发布器；`.sealed` 不可作为可恢复备份。Task 4 的干净实例完整恢复和 Task 5 全链验收仍分别待做，不提前写入 `P0_C4_VERIFIED`。
- 用户在服务器本机执行 root-only 验收后，runner 返回 `TASK3_CANDIDATE_GATES_PASSED_NOT_PRODUCTION`，`result.json` 位于 `/var/lib/knowweave-c4/batches/892adcedb12b-9b0ba3d9-adc3-4cc0-881c-51c0ee9f7160/result.json`，报告 SHA-256 `d10e46d7f562925aa05099d55812b4665229a674d766a153d5628a68aef3108d`。根任务通过只读 Docker 查询确认三个 Task 3 项目没有遗留容器、网络或卷；root 私有 `result.json` 内容尚待单独只读审计，故此处只记录候选门摘要，不扩大为完整 C4 验收。
- 服务器只读 `lsblk` 显示仅一块承载根文件系统的 `nvme0n1`；同机两个 Docker 卷不构成实际异故障域。用户选择先完成代码与单机隔离测试，暂无独立目标存储；最终备份可恢复性验收待另一物理磁盘或独立主机/NAS 的实测证据，期间不写 `P0_C4_VERIFIED`。
- 目标端转移与完成收据候选提交 `23c7672`，签名阻断修复提交 `f877b1b`；独立静态复审最终通过。`transfer_sealed_backup` 将源包逐文件重读、复制到新目标 staging 并重新验证后才封存；默认构建的 `CompleteBackup` 发布/打开保持关闭，独立见证开关本身也不足以启用，仍需构建时固定外部见证公钥指纹。该协议不证明物理故障域独立。
- 旧 Ed25519 版本曾在全零弱公钥/签名下出现非确定验签通过：根任务重复 30 次有 4 次失败，独立审查者重复 12 次有 5 次失败，旧提交保留作失败证据。修复改为 v2 P-256 固定格式并拒绝 v1；根任务重跑 3 项契约测试、弱键用例连续 30 次、开启功能开关的契约测试、格式和严格 Clippy 均通过，独立审查者也验证默认与 `--all-features` 测试通过。Linux 专属传输、目录同步、默认拒绝及真实异机持钥仍未实测。
- Linux 单机传输专项脚本提交 `40121b0`、容器清理隔离修订 `8a99735`，独立静态复审通过。用户逐包授权并上传源码包 `task3-transfer-default-8a99735.zip`；服务端 SHA-256 与授权值 `8457be083689f3d3a947348e461f416d4b3300d688673a638275b35d841fc52c` 一致。该包 3,493,704 字节，含 371 个 Git 文件，内嵌清单 SHA-256 `3a0f14d45ac51a55b1e0fd7b5a833dbe064a2e7b801da6a53e473da08d7ce03f`。独立审查者从 Git blob 重新打包与候选 ZIP 逐字节相同；根任务本地核对 ZIP 内新引导脚本哈希 `51a47bc1e793423c104026e31183ff9b352cce559fd74b5e6ff424b2dd32934b`、runner 哈希 `84c84d5a285ecbf469f864b510031fb51794b7100f8d7cf28d5d821c38633299`，新旧脚本测试分别 6/6 与 5/5。设计为无网络/无 PG 凭据的独立容器，默认构建不得产生 `CompleteBackup`。
- 服务器单机隔离批次 `transfer-8457be083689-6bce5634-67cd-42ec-a2b2-de74be4e28b4` 返回 `FAILED`，结果 SHA-256 `a06afe48106f5e39c7d790868a4bfb91c4ed7e7d2e79119e937d5bd21b38892e`；`result.json` 显示格式、严格 Clippy、工作区编译及四组专项测试全部退出 0（7+1+2+3 项），`failure=null`、`source_unchanged=true`，但七个容器清理 inspect 均被记为 `inspect-failed`。根任务只读复现 Docker 对已由 `--rm` 清除的容器输出小写 `error: no such object: ...`，而 runner 只匹配大写 `No such`；只读 `docker ps` 查询确认该项目标签无遗留容器。此轮仍按 `FAILED` 保留，清理误判修复及新批次完整重测待办，不把专项测试通过写成候选门通过。
- `4bbf16f` 以精确容器名识别 Docker 的大小写不同“不存在”错误，其他 inspect 故障继续失败；新增回归先复现旧误报，修复后 7 项脚本测试通过，独立静态复审通过。旧 `8457be08…` 包仍对应旧代码，不可在同批次重跑；须从新提交制备源码包、逐包授权并使用全新隔离批次复测。

## Task 4 本地策略切片

- 实现提交 `0fd4806`，两轮独立复审修订 `77f8e8d`、`48f493b`；最终独立静态复审无确定性阻断。先前发现的快照重试/终态外部副作用漏判，以及从备份来源自证目标环境身份的问题已修正。
- 恢复策略可检查固定 `pg_restore` 参数、新空库/私有资产根、独立观察到的构建/PG/迁移身份、非敏感角色配方、全量 ready 资产行与原件字节哈希，并保守分类恢复后的租约和外部效果。根任务独立重跑 8 项 `restore_policy` 测试与格式检查，均退出 0；实现者另报告严格工作区 Clippy 通过。
- 这些函数接收由未来可信执行器采集的事实，不自行证明目标权限、数据库会话或身份；当前没有 `CompleteBackup`、独立目标核验及签发、实际 `pg_restore`/资产导入、租约失效事务、派生物重建或服务放行。完整 crate 的真 PG 测试需要专用空库，Windows 本机没有该配置，未借用已有数据库。
- `4e1cb54` 加入以不透明 `CompleteBackup` 为唯一入口的只读恢复预检：持有 root 私有锁，重新核验收据与包字节，并在目标写入前检查空库、资产根和环境身份。实现者报告本地格式、严格 Clippy、工作区编译及专项通过；根任务独立重跑 2 项 `restore_preflight` 测试通过。独立静态复审未通过：`public` 下的函数/类型/扩展未纳入空库判定、PG 新实例身份未由独立可信记录固定、私有根目录的可写祖先可被换名。修复与 Linux/真实 PG18 负例待办；默认构建仍不能获得真实 `CompleteBackup`，该预检不执行 `pg_restore` 或资产写入。

## 设计裁定

- Ruling: C4 走独立整库备份路径，不复用 C3 授权阅读包 — C3 排除了用户、空间、授权、作业等权威行 — 若误用会产生缺失恢复点。
- Ruling: 本轮隔离维护窗先停止 runtime/Worker、拒绝 runtime 新连接并排空旧事务；在线只停写能力留给 P1 接入后另验 — 现有写入口分散，单入口闸无法证明完整 — 代价是隔离备份期间阅读暂停。
- Ruling: Task 2 只交付 `sealed` 复制原语，Task 3 源端一致性/保留保护及目标校验后才可产生 `complete` — 先封存字节并不代表数据库与资产属于同一恢复点 — 代价是任务之间必须保持状态类型隔离。
- Ruling: SHA-256 检测损坏，不证明有包写权限者未伪造；依赖私有管理目录和独立运行证据，恢复仍全量重验 — 若目标目录不可信需以后另加认证签名/MAC — 当前不宣称抗恶意目标管理员篡改。
- Ruling: 新实例恢复使所有旧实例运行中租约失效，按重试上限和外部副作用记录分类 — 仅按过期时间处理会让旧进程令牌仍有效 — 代价是部分作业恢复后需人工核对。
