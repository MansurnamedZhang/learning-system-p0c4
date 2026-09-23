# P0-B4 执行记录与验收门槛

状态：`P0_B4_VERIFIED / NOT_PRODUCTION`。设计依据为工作区批准稿 `2026-09-23-p0b4-impact-query-design.md`，实施依据为同日 `2026-09-23-p0b4-impact-query.md` 施工单；这两份批准文档位于工作区外层 `docs/superpowers/`，不随本仓库的源码包打包。工作分支 `feat/p0b4-impact` 位于独立工作树；B3 验收分支、原 `learning-system`、旧数据库/卷与原图实验均不修改。用户已批准逐任务实现与独立审查。Linux 由单独服务器任务串行运行冻结包，源码只读、每轮新库，根任务核对逐文件清单和原始日志。以下仅写已回传并核验的结果；最终可执行源码身份仍以验证记录中的候选为准。

## 分关结果

| 阶段 | 已证事实 | 尚需处理 |
|---|---|---|
| Task 1 查询契约 | 提交 `2552438`、审查修复 `415d688`；精确身份、严格反序列化、可见预算和原子组分页回归已通过 | 无分关阻断 |
| Task 2 反向直接查询 | 提交 `39aed849049aa712f71750d4284ec05b4423395e`；真实 PostgreSQL 结构/必要反向索引、有界 keyset、隐藏候选、旧锚与升级回归通过；最终分关全量 244 通过、0 失败、1 默认忽略，独立复审关闭发现 | Task 5 阈值修复后五库全量已重跑通过；跨模块终审已关闭 |
| Task 3 局部证据 | 提交 `148e79c0df79061645a9104da1171b2aca9987e5`；固定/动态方向、条件、审查配对、匿名与授权恢复通过；最终分关全量 265 通过、0 失败、2 默认忽略，严格 fmt/Clippy 与独立审查通过 | Task 5 五库全量已复核大范围回归；跨模块终审已关闭 |
| Task 4 多族遍历 | 提交 `2294011479d5bf7eb44eb93f6c81b40af250e3a0`；必要菱形、语义环、结构重复位置、固定选择、谱系、历史范围与匿名/稠密场景通过；最终冻结包 `62c29095857c50d17b1a32c99fc2e2da0abb9514d99605def79d9833249ebd5e` 在五库全量 283 通过、0 失败、3 个显式 EXPLAIN ignored，四种旧程序 bootstrap 均退出 0；fmt/严格 Clippy 通过 | Task 5 新项目 Compose 与跨模块终审均已关闭 |

索引取舍基于真实 `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)`：0007 的结构目标优先索引在合成 scale 8 中减少首页 shared hits；必要后页索引改为目标优先访问，但没有证实该样本加速。0008 的谱系目标索引使合成夹具 8/8 计划从无关输入过滤改为 Bitmap Index/Heap Scan，仍有 Sort，耗时有升有降；不声称生产提速或延迟上界。测量在全缓存合成数据、新库随机 UUID 下进行，不能外推生产性能。

## Task 5 Attention 与横向授权阈值

原 B3 合成 Attention 场景的固定 H@1、同源 E1/E2、反例 X、人工关系审查、认识审查 J、结论 C、Reading/Release、H@2、撤权恢复和 Split 被扩成 B4 查询期望。首切片冻结包 `0eb248c7f74ca1dda6db481e1b87d3daeb1cc681c917f954b0047538ae449c6e` 在新库 1/1 通过，名称虽带 `red1`，实际是 **GREEN 补覆盖**，不能记录为行为 RED。扩展包 `0cf1c97a5c2daae2bf94020b4fa9dd4ba6f30102edfdd1c152927049770d8e6b` 在新库 1/1 通过，覆盖 H/C placement、原文 occurrence 的完整组/路径、H@2 固定历史负断言、撤权匿名和恢复、Split 的旧发布边界及新阅读输出。后续更严格的 Semantic/Necessary/ReviewSelection 单族全组等值断言已冻结，Linux 结果单列于验证记录。

Task 2 的横向真实 PG 回归构造 T、250 个叶 L、共享 K、175 个 Context 来源 S、三个聚合 A 和唯一显示 H。A/H、组合与 Reading 由公开 API 创建，B3 正文与 Reading 读取均先通过；第一未修复包 `9ced9d5fb714d2fe1f4e16d816fdfe2b717f5eacc5e05c1ed9ddc9a1e66e8b44` 只在 `direct(T)` 查询返回 `Storage`，0/1、退出 101。这是**查询行为 RED**，公开错误并不泄露私有计数，不能单凭错误文案声称已证明具体内部计数。修复把每个必要候选交给新的 B3 Session，在同一 RR 事务逐项完整授权，未对 `selected.contains` 快速放行或改旧 B3 上限。同一增强测试在 `6fccd824...` 修复包中 175 个 `traverse`/`direct` 完整结果等值、1/1；仅将消费者实现恢复为 Task 4 原字节的 `8a245766...` 对照包中，B3 读与 `traverse` 175 先通过，`direct` 再报 `Storage`、0/1。两个包的其余 251 个源文件字节相同，因而把差异明确定位在旧 `direct` 实现，仍不把公开错误附会为可见内部 work 数。原始记录见 [验证记录](p0b4-verification.md)。

Task 3 先前已有大型固定 Reading + 90 个外部小动态关系的真实 PG 回归 `large_reading_scope_does_not_exhaust_reference_session_across_small_dynamic_closures`；Task 5 五库全量中该测试再次为 ok，不能以只审 Task 2 代替横向审计。

## 冻结、部署和审查门槛

每轮 ZIP 内 `SOURCE-MANIFEST.json` 用 SHA-256 逐文件核验，Linux 只读挂载同一包，执行前后再校验；每个命令独立保存原始 stdout/stderr、退出码、迁移哈希、资源与普通 runtime/admin 身份。失败证据保留，不重用已污染数据库；五个库分别用于普通与 P0-A/B1/B3-schema/B2 四种冻结旧程序升级。旧行、回执、正文摘要、历史发布和 migration checksum 的前后断言必须继续通过。旧 B2 冻结源码 SHA `2da005cdf6f2e92ea4fa2f34463b371a71c30e145e6d8927eb75db558b284225`。

Task 5 文档候选 `daa8e47c282a66ad13209b298d58be6b213064f72b2c0510c6c2ce0d73d12477` 的 255 个清单文件与 `6fccd824...` 已通过的 252 个文件比较，仅新增三份仓内文档，可执行源码字节不变。该包 Linux 五个新库工作区全量 57 份 suite、284 通过、0 失败、3 个显式 EXPLAIN ignored；四种冻结旧程序 bootstrap、fmt、严格 Clippy 全部 exit0。根任务从 B3 原始全量 stdout 提取 222 个唯一测试名，逐一与本轮实跑日志匹配，222/222 状态仍为 ok、缺失 0；Task 3 大范围回归与 Task 5 两个命名专项也在本轮为 ok。源码清单、原始日志 SHA 已独立核对，精确目录与哈希已补入 [验证记录](p0b4-verification.md)。

同一 `daa8e47c...` 包在新 Compose 项目 `learning-system-p0b4-t5-fc1-compose1` 运行，启动前确认该项目无容器/网络、`test_pg` 和 `test_evidence` 两卷不存在；五库迁移前用户表数均为 0。`config`、`build`、`pg-up`、`up`、`stop` 均退出 0，容器内四旧 bootstrap 与工作区全量退出 0，57 份 suite、284 通过、0 失败、3 个显式 EXPLAIN ignored。PostgreSQL 与测试容器分别限 2/4 CPU、4 GiB，内部网络、无宿主映射端口；镜像摘要、原始日志哈希与资源快照见[验证记录](p0b4-verification.md)。Compose 配置文件默认名仍为 B3，本轮通过明确 `-p` 使用独立项目，没有触碰旧 B3 卷。

独立子代理已终审查询契约、授权与解释路径、Task 2/3 的 B3 Session 横向阈值、索引与原始 `EXPLAIN`、旧 API/迁移、宿主及 Compose 原始证据；无开放 Important/Minor finding。0007 的必要后页合成样本未证加速，0008 在 8/8 合成计划中使用新增目标索引但仍需排序、耗时有升有降；终审接受这一有限结论，不将其外推为生产性能。`P0_B4_VERIFIED` 仅指该内核与此记录的隔离验证通过；生产部署、推送或合并没有执行。
