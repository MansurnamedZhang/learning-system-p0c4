# P0-B2 · 知织个人阅读内核边界

Rust 库提供固定原文与个人内容的连续阅读数据、原子编辑和显式迁移。当前没有 HTTP、身份会话、前端、图片资产、语义关系或索引消费者。Principal 必须由未来受信任认证边界提供；runtime 是受信任服务账号，不代表数据库启用了用户级 RLS。

## 使用顺序

1. `ReadingStore.create(actor, personal_space, CreateReading)`：个人空间必须由 actor 拥有，当前可写；根必须是固定 Document 修订。返回 `ReadingSaved` 的层和视图精确引用。
2. `ReadingStore.state(actor, overlay_id)`：取得当前两个预期头。只有完整原文、全部个人块和待放置历史来源均可读时，才返回可提交的 `editable`；否则只返回层/视图身份。
3. `ReadingStore.edit(actor, overlay_id, EditReading)`：InsertNew/InsertExisting 插入；ReviseSelected 只更新明确选择的 placement；AdoptExisting 采用已保存正文；Move/Remove 不写正文；PlaceUnplaced 显式安置待放置组。
4. `ReadingStore.read(actor, ReadingRef, mode)`：Original/Fused/Personal 均固定到给定视图。历史视图不跟随块或文档工作头。
5. `MigrationStore.propose(actor, overlay_id, ProposeMigration)`：比较同一稳定根的两个固定基线，不推进层头。`read(actor, proposal_id)` 返回当前权限投影。
6. `MigrationStore.decide(actor, overlay_id, DecideMigration)`：拒绝只保存决定；采用则原子保存新层、新视图、映射、决定和收据。全部旧组须恰好处置一次，碰撞须显式合并顺序，原组内部顺序必须保留。

命令 JSON 样例在 contracts。输入拒绝未知字段，服务生成作者、时间和新身份。空根/空章节可插入；重复章节通过完整 occurrence 路径区分。左右邻居必须对应固定父组合的真实相邻边界。两种 affinity 在同一间隙可分别存在，展示顺序为 after_left、before_right。

## 保存与冲突

正文修订、层修订、视图修订和收据同一事务提交。插入失败、预算超限、冲突或收据写入失败不会留下部分正文。每批最多 32 个正文；返回 changed_blocks 顺序为创建顺序，修订批次按 block_id 排序。

新请求必须同时匹配层与视图头，正文修订额外匹配块头。ReadingConflict 返回当前两个头，重新调用 state 后形成新请求。未知提交结果先用原 request_id 和完整原命令重试；成功重放优先于过期判断，但仍检查当前权限。全局请求键与 P0-A/B1 共用，同键不同操作或内容冲突。

移动/删除/采用相同版本等无变化操作返回 Invalid(no_change)。正文批次任一项与当前块稿相同则整体拒绝；要将旧位置升级到已存在版本，使用 AdoptExisting。删除只移除出现位置，正文与历史继续存在。

锁顺序：请求 advisory lock → UUID 排序的空间授权 → 层 → 视图 → UUID 排序的正文块。只读 API 使用 READ ONLY REPEATABLE READ；写入 READ COMMITTED，在锁住所需当前授权后复查固定引用。独立事务的固定版本可以在查询前刚提交，不承诺写事务开始时的快照。

## 权限与预算

层只对 owner 且有个人空间当前 grant 的调用方开放。原文固定闭包必须全部可读才显示来源结构；否则 source=unavailable、原文不返回、所有可读个人内容进入 unplaced 且 location=null。此读取降级不改历史，恢复权限后原位置恢复。

个人块不可读则整项省略，不返回隐藏 ID、数量或原序号。部分可见结果不返回原始层摘要。迁移历史分别授权旧来源和目标；分类与原因只在两侧均可读时出现，决定摘要不暴露内部映射和结果基线。历史来源不是当前来源的别名，连续迁移仍保留真正 origin_anchor。

每层最多 512 组、2,048 个个人出现位置，路径最多 15 层；个人正文按去重块修订计算，最多 8 MiB。字节定义为 PostgreSQL `octet_length(content::text)` 的 JSONB 序列化，与 HTTP/紧凑 JSON 大小不同；授权过滤先于正文计数和取值。原文闭包沿用 B1 的独立预算。这里是正确性限额，不是生产性能保证。

## 数据升级与运维

0003 为新增迁移，0001/0002 不变。不可变明细、提案、决定和收据只授予 SELECT/INSERT；身份表只允许指定 head 列 UPDATE。固定引用使用复合外键，身份不随移动或正文变化而改变。

普通库、P0-A 升级空库、B1 升级空库必须分开。新增环境变量：TEST_B1_UPGRADE_ADMIN_DATABASE_URL、TEST_B1_UPGRADE_DATABASE_URL、TEST_B1_MIGRATIONS_DIR（只含原始 0001/0002）。升级测试会拒绝已有迁移登记的库。Compose 初始化提供三个库；每轮全量测试用全新项目/卷，旧证据卷保留。

B2 无后台索引、在线滚动升级、生产性能或备份恢复验收结论；原件与导出属于 P0-C，浏览器编辑属于 P1-A，关系与影响查询在 B3/B4。
