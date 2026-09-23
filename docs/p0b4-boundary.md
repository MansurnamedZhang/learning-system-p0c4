# P0-B4 · 修订影响查询内核边界

状态：`P0_B4_VERIFIED / NOT_PRODUCTION`。B4 的反向查询与固定范围遍历已完成 Tasks 1–4 的分关实现、独立审查和真实 PostgreSQL 验证；Task 5 的 Linux 五库全量、全新隔离 Compose 全量与跨任务终审均已通过。本页描述已验收的内核契约，不表示整套学习系统或服务器已获生产验收。

## 查询对象与固定范围

`QueryStore::direct` 返回当前授权的直接结构和必要依赖消费者；`QueryStore::traverse` 在指定深度内沿结构、必要依赖、语义关系、固定审查选择、系统谱系五种边族遍历；`QueryStore::evidence` 返回某一精确块的局部关系和认识审查。每次请求必须指定一个精确块／组合／关系／审查起点与固定 Reading 视图及模式，或一份历史 Release。块的稳定 ID 与修订 ID 始终同时比较，H@1 不因当前头变为 H@2 而被替换。

固定范围区分 `Displayed`、`Selected`、`Context`。阅读 Original/Fused/Personal 按实际投影及 personal placement 判定；历史发布按不可变根、所选 Reading、必要清单和当前授权判定。清单 Context 只表明必要成员，不自动成为展示、直接引用或可扩展语义桥。新工作态关系只在 Reading 中从当前可见端点发现，明确标记 `DynamicWorking`；历史 Release 不吸收新 head。固定所选关系标记 `FixedReadingSelection` 或 `FixedReleaseManifest`，不与动态记录混写。

消费者以精确对象身份分组，组内保留每条解释路径。结构位置使用从根到叶的 occurrence UUID 序列或 Reading 的 placement UUID；同一子组合重复出现时两条路径都保留。个人未放置项使用 `Unplaced`，不冒充原文 occurrence。必要引用边保留保存时的 role 与 position；语义边保留关系类型、原方向和固定/动态来源；谱系使用独立 `SystemLineageType`，不会被用户关系类型代替。系统 split/merge/derive 不自动修改语义依据，也不把发布后的新输出塞进旧 Release。

## 授权、预算与结果

一个请求在只读 REPEATABLE READ 快照内执行。先用当前 grant/overlay 所有权预过滤候选，再由 B3 `references::Session` 检查每个可见对象的完整必要闭包及独立循环根深度；隐藏桥接点不可继续扩展。固定 Reading 中某关系另一端即使不在范围、但本身可读，也只可作终端解释，不能加入固定成员或继续扩展。可见审查若证据被撤权，沿用不含隐藏身份的 `Incomplete`；不可读范围与不存在范围同形。撤权后的新请求立即重新授权，恢复后历史固定选择仍指向原精确版本。

深度最大 8；单页消费者最大 200；公开可见边最多 8192、不同节点最多 2048、解释路径最多 4096、投影最多 8 MiB、可见工作步骤最多 131072。默认深度 3、单页 50、可见工作 4096。隐藏候选不计入公开预算，不成为游标或返回总数。游标按已授权的完整消费者组排序，翻页不拆分同组路径。`Complete` 表示该范围完整，`Truncated` 带可见游标，`BudgetExceeded` 原子返回空消费者；B3 完整授权预算或内部扫描/超时失败为通用存储错误，绝不伪装为零影响或公开预算超限。

每次反向访问使用目标优先条件、有界 keyset 批次和 SQL `LIMIT`；不先读全图再裁剪。Task 5 的横向回归发现 `direct` 必要依赖候选复用固定范围会话会累计 B3 工作上限，现改为同一快照内每个候选独立完整会话；未放宽 B3 的单候选 2048 对象／131072 工作保护。该修复的专项、Linux 五库全量及全新隔离 Compose 全量 GREEN 已取得。

## 保留与未覆盖

PostgreSQL 的不可变对象、权限和清单仍为唯一权威；0007、0008 仅追加反向访问索引，不引入 Neo4j 常驻同步。旧 0001–0006、v1 摘要、冻结旧程序与原 PostgreSQL/Neo4j 对比实验原样保留；原实验的四项权限失败仍是 `NOT_PASSED`，不能以 B4 正式路径通过改写其历史结论。

此阶段没有 HTTP 身份认证、P0-C 资产原件、P1-A 网页、生产负载性能、备份恢复或在线滚动升级验收。B4 通过后也仍是 `NOT_PRODUCTION`，后续阶段另行验证。
