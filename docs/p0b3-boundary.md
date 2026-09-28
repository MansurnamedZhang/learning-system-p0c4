# P0-B3 · 知织关系与认识审查内核边界

当前状态：`P0_B3_VERIFIED / NOT_PRODUCTION`。Tasks 1–7 完成分关实现与审查；Task 8 宿主与全新 Compose 均222通过，同一227文件候选、四个冻结 bootstrap 均通过；独立补充审查及runtime证据条件已关闭。测试容器已停止、证据卷保留，首次 Compose 外部凭据可读性失败完整归档。B4 影响遍历未实施，本文不是生产上线批准。

## 版本、引用与授权

正文修订、语义关系修订、关系人工审查和认识审查分别留史。引用同时固定稳定对象 ID 与修订 ID；审查引用还固定审查记录。任何历史对象的读取仍受当前权限约束，历史发布不是绕过撤权的副本。

旧 `ContentStore`、v1 `TextDraft`、规范字节及摘要保持；v1 继续拒绝未知字段。`VersionedContentStore` 显式接受 `ContentDraft::V1/V2`；命令使用 `request_id/contract_version/draft/reason`，修订另带 `base_revision_id`。v2 正文为 text、reference 或 relation_view，依据、必要上下文、来源运行和已选择审查均使用类型明确的精确引用。作者、审查者、时间、新身份由服务生成。

所有正文、组合、阅读、关系、审查和发布路径经统一必要依赖闭包授权。隐藏任一必要依赖时，整体正文不可用；单项与批量省略策略一致。可读 v2 进入旧 API 时返回 `Invalid(unsupported_content_version)`，先执行授权，不能用版本错误探测隐藏内容。可见审查但必要证据不完整时只返回不含身份的 `Incomplete`；不可见对象与不存在对象一致。可选审计前序并非必要证据，但输出前序标识仍须授权。

原 PostgreSQL/Neo4j 对比实验保持 **NOT_PASSED：四项权限失败未被改写**。两库各复现“结论 basis_refs 指向后来隐藏的证据”和“个人事实 target 指向隐藏目标”，隐藏标识与正文 helper 都曾泄漏。B3 在正式 Rust/PostgreSQL 路径中补足回归，不追溯宣称原实验已修复。

## 关系与人工判断

- 用户关系类型限定为 annotates、questions、answers、inspired_by、supports、opposes、tests、related_to。同范围、类型和稳定端点只保留一个关系身份；related_to 同时规范化端点 ID 与对应修订，不能只交换 ID。
- `RelationStore::save/read/read_selected/review/read_review` 固定关系端点与选定审查；请求重放返回原固定结果并重新检查当前权限。关系审查为 unreviewed、reviewed、needs_recheck、withdrawn，追加留史，不覆写端点正文。
- `ReviewStore::append/read` 保存条件化认识判断：untested、testing、inconclusive、supported_within_scope、refuted_within_scope、superseded。流按范围、精确目标和审查者隔离。新 supported/refuted 判断须有方向正确、证据列明、选定记录及当前审查头都为 Reviewed 的对应依据。历史读取及原请求重放不会被后来的撤回改写。
- `group_sources` 只按显式 `source_run: BlockRef` 分组；无来源的每项独立显示，不表示统计独立性，不按数量推断真假、置信度或实验结论。

关系理由最多 1,000 Unicode 标量；条件与说明各最多 10,000 UTF-8 字节，均拒绝 NUL。Reviewed 的 supports/opposes 要求理由、条件、说明 trim 后非空，保存原字节。认识 supported/refuted 同样要求非空条件与说明。Space 范围可重用当前可读跨空间证据；PersonalOverlay 限 owner，直接阅读选择必须属于该层。底层依赖仍逐项授权。

## 阅读与发布

`ReadingStore::select_relations` 只生成视图修订，不修改正文或层修订；两个视图可以共享一个 overlay revision，必须用完整 `ReadingRef` 读取。`read_versioned` 返回授权后的关系、可选关系审查和认识审查投影。旧 `ReadingProjection` wire 不添加 evidence 字段；旧客户端读取 v2 视图明确报不支持。

编辑、迁移提案及回执保存精确视图，携带选择也须授权。历史选择固定旧关系和旧审查，不自动跟随关系头、正文头或判断头。H@1 改到 H@2 后，旧关系、旧判断、C@1 与历史阅读均仍引用 H@1；是否对新目标作判断由人显式决定。

`ReleaseStore::publish_evidence/read_evidence` 固定根组合和阅读视图，保存全部必要依赖清单及 `release-manifest-v2` 规范摘要。数据库延迟约束也验证清单完整性，不能靠原生 SQL 绕过应用检查。旧发布只能包含全 v1 内容；嵌套 v2 也必须拒绝。撤权后发布不可读，恢复授权后恢复同一固定清单，不重新生成历史。

## 派生、拆分与合并

`LineageStore::apply/read` 原子建立新正文、新稳定身份、操作、输入输出及回执。derive/split/merge 生成独立的 derived_from/split_from/merged_from 系统投影，不能由用户 `RelationType` 冒充。输入保留，输出顺序固定；失败不留半套输出。谱系是操作来源，不自动继承语义依据、关系判断或替换阅读位置。

## 事务、限制与错误

写入按请求键锁 → UUID 排序空间授权 → 具体对象锁执行。先发现依赖空间，锁后重新验证；不得在对象锁之后反向补空间锁。新请求的过期头只有完整授权后才可出现在 Conflict；并发发现集合变化返回无身份的 `reference_authorization_changed`。重放只授权原固定结果，而非强迫读取后来的头。

只读事务为 READ ONLY REPEATABLE READ；写入为 READ COMMITTED，加授权行锁。撤权提交后的新读取立即遵守新权限，不承诺取消已经合法开始的读取。runtime 是受信任服务账号；Principal 必须来自未来受信任认证边界，当前不声称启用了用户级 RLS。

| 限制 | 上限与含义 |
|---|---|
| 单个 v2 正文直接依赖 | 256；重复精确引用拒绝 |
| 单次必要闭包 | 2,048 去重对象、4,096 去重边、最长简单路径 32 层、8 MiB 去重载荷 |
| 简单路径分析 | 最多 131,072 路径/边步骤，含重访；保守失败防指数复杂度 |
| 内嵌显示 | 两层；显示限制不替代完整授权 |
| 批量正文 / 关系选择 / 判断证据 | 分别 200 / 256 / 256 输入 |
| B1、B2 旧限制 | 组合 2,048 对象、4,096 occurrence、16 层、8 MiB；个人 512 组、2,048 placement、32 正文批次、8 MiB 保留 |

block 载荷按 PostgreSQL `octet_length(content::text)`；关系和审查按不可变实际行 JSONB 序列化字节计量。新依赖闭包预算合并所有根；纯 v1 原路径保持旧预算，不通过逐根检查放大总量。无权/缺失为 NotFound；重复请求不同内容为 IdempotencyConflict；超限为 `Invalid(reference_budget_exceeded)`，不附隐藏计数、ID 或部分成功。

## 升级和阶段范围

0001–0003 与根锁文件不改；0004–0006 分别新增引用/关系/审查、固定阅读证据/发布、系统谱系。registry 与 dependency 是派生索引，由实际不可变对象注册并经延迟约束核对，不能独立伪造。runtime 新不可变表只授 SELECT/INSERT，身份表只开放指定 head 列 UPDATE。

验收使用五个独立数据库及四个冻结程序引导：普通、P0-A、B1、B3-schema、完整 B2。B2 夹具真正运行冻结五个旧 store；P0-A 保留原 schema SQL 造数及冻结读取，不冒称使用 P0-A 旧 create 程序。具体命令和证据见 [验证记录](p0b3-verification.md)。

尚未包括 B4 影响遍历/索引消费者、P0-C 资产与原件、P1-A 网页与登录、HTTP 身份认证、生产性能、在线滚动升级和备份恢复。首版继续以 PostgreSQL 为权威库，不增加常驻 Neo4j。
