# P0-B4 反向链接、证据查询与修订影响施工单

2026-09-23 · P0_B4_VERIFIED / NOT_PRODUCTION · [设计](../specs/2026-09-23-p0b4-impact-query-design.md) · B3 基线 `4252cc812b6c3bb7e7fabbcc309522b005f9df44` · 最终提交 `fc25e91954419bd0c6c272d00577db1b74ce6ce8`

## 执行约束

应用在独立工作树 `隔离工作树`、分支 `feat/p0b4-impact`。原 `learning-system` 和已验收 B3 工作树保持不变。先完成每一项的失败/首个覆盖测试，Linux 真实 PostgreSQL 验证，提交，再由独立子代理审查；审查真实缺陷先补可失败的回归再修复。B4 接口尚未发布，细节可据实际测试收紧，但不可放宽已批准的身份、权限、预算与历史约束。旧迁移 0001–0006、原 105 项及 B3 222 项测试不得删改为虚假通过。

统一的结果语义：固定精确修订与范围；结构、必要引用、语义关系、固定审查选择、谱系分属边族；按固定 ReadingMode/Release manifest 的显示、选择、必要闭包分层判定成员；先授权再统计、排序、分页或扩展，隐藏候选不消耗公开预算；隐藏桥接点截断；`Complete`、`Truncated`、`BudgetExceeded` 不互相冒充。每项最多一个实现子代理和一个独立审查子代理同时修改本项代码；下一项在审查门槛关闭后开始。

## Task 1：查询契约与预算

**Files:** `crates/learning-core/src/impact.rs`、`src/lib.rs`、`tests/impact_contracts.rs`。

- [x] 对 `ImpactStart` 精确起点（B3 `ExactRef` 加组合修订）、`ReadingRef+mode` 或 release ID 范围、边族、深度/返回/工作限制、可见游标、结果状态定义严格 serde 契约；拒绝未知字段、重复边族、零/越界限制、伪造内部对象。
- [x] 消费者身份与 occurrence/placement 位置分开；路径每一步携带精确修订、边族、关系方向及解释，不将 H@1/H@2 混为一体。发布/阅读标明实际固定范围；动态工作态关系注明来源。
- [x] 纯函数测试固定排序、消费者组原子翻页与完整性状态；预算器只对已授权的可见边/节点/路径/字节/步骤计公开限额，隐藏候选不改变可见预算状态。先运行失败测试，后实现并提交 `feat: define bounded impact query contracts`。


执行记录：提交 `2552438`、审查修复 `415d688`；审查 1 Important + 1 Minor 均先行为 RED 再修复并复审关闭。专项 12/12、learning-core 54/54、fmt/check/Clippy 均通过。数据库 Task 2 投影 `ImpactStep` 时必须显式调用 `validate`，因为公共字段可直接构造。

## Task 2：反向来源与直接消费者

**Files:** `migrations/0007_impact_indexes.sql`（仅实测需要的目标优先索引）、`crates/learning-db/src/queries/{mod,scope,consumers}.rs`、`src/projection/mod.rs`、`tests/consumers.rs`、`tests/projection_authorization.rs`。

- [x] 核对 B3 实际表与索引，先写直接消费者测试：同一块出现在两根/重复子组合的不同位置，去重消费者但保留每条完整 occurrence 路径；个人 placement 与未放置项独立。`reference_dependency` 的直接来源不与 release 清单的递归成员混同。
- [x] 范围使用精确阅读视图或历史 release；一个只读 repeatable-read 快照内按 Original/Fused/Personal 实际投影、unplaced、固定选择与必要闭包计算成员资格和全部位置。原文撤权时保留可见个人成员而不加入原文成员；release manifest 的 context 成员不冒充显示/直接引用。以 B3 `Session` 完整授权候选，不把原始索引/JSONB、隐藏数量或过滤序号返回。不可读范围与不存在一致，撤权后重新查询立即过滤。
- [x] 每次反向索引访问使用目标优先索引、有界 keyset 批次与 SQL `LIMIT`；预过滤 grant/overlay，再逐项完整授权。公开预算只计可见图；隐藏密集候选不能单独造成 `BudgetExceeded`。内部扫描超时统一报不可用，不能伪装为完整。测受限角色、隐藏密集候选和真实 PostgreSQL `EXPLAIN (ANALYZE, BUFFERS)`。通过后提交 `feat: query authorized direct consumers`。

执行记录（未验收）：首个冻结包 `b4-task2-green1.zip` 的 SHA-256 为 `9a7072e1e28db3495ee4fbb6e0883bbe158050d13abbad55b9b66ba1550ff3b1`，236 个源码文件逐项哈希通过。独立 Linux PostgreSQL 专项实际运行 4 项，3 通过、1 失败；失败位于 `consumers.rs:140` 的测试夹具创建，错误 `Invalid("unsupported_content_version")`，尚不能据此评价该查询场景。原始日志、退出码 101、角色、资源和源前后校验保存在 `客户端绝对路径`；下一包修正夹具并补齐场景后重测。此前 Windows 本地只记录编译 RED，未声称服务器 RED。

第二冻结包 `b4-task2-green2.zip`（SHA-256 `0619dc85c9f26c48f9dd0ecca78978078f109bb2defb92c4add4e24d34479dfc`）在另一全新库实跑 6 项，5 通过、1 失败；失败为 `consumers.rs:407` 的公开状态断言。核对契约可见单组需起点、消费者、边、路径共 4 个 work，夹具当时设 `work_limit=3`，因此需加无隐藏/隐藏对照与正确边界断言后重跑，不能直接判定为隐藏候选泄漏。原始证据：`客户端绝对路径`，退出码 101，源前后哈希一致。

第三冻结包 `b4-task2-green3.zip`（SHA-256 `52f5d681fd3000d2fb72958ccb6cff6b799510015ed9452b82fdbfb4df6dd4a5`）在全新库专项 6/6 通过，退出码 0；预算边界与 64 个撤权候选前后结果对照均通过。根任务独立复核包内 236 文件和原始日志 SHA，证据位于 `客户端绝对路径`。工作区全量和 `EXPLAIN` 尚待结果，因此 Task 2 尚未验收。

同包首次 `--workspace` 尝试执行到升级测试时因缺 `TEST_B1_UPGRADE_ADMIN_DATABASE_URL` 停止，已执行 83 通过、1 环境前置失败，不能记为全量或产品回归；实际证据目录为 `客户端绝对路径`。已安排按 B3 冻结旧程序路径准备四种夹具及五个独立 B4 数据库，再运行完整工作区。

同包随后按上述五库与四冻结夹具完成 `--workspace`：50 条 suite 结果，实际 240 通过、0 失败、退出码 0；四个 bootstrap 均退出 0，旧数据 manifest 与源码哈希核对通过。根任务用原始全量 stdout 和包独立复核数量、哈希；证据为 `客户端绝对路径`。仍需 SQL `EXPLAIN` 与独立审查，Task 2 不在此标记完成。

EXPLAIN 冻结包 `b4-task2-explain1.zip`（SHA-256 `1fff0d8058a98b2b213963b5a74524676b3b2eed5bf8e1998ab2035778eee99a`，只新增默认忽略的测量测试）在两个新合成库按 scale 1、8 各 1/1 通过；每轮 24 条脱敏计划，分别保存于 `客户端绝对路径` 和 `.../b4-task2-explain1-scale8`。scale 8 首/后页结构查询未用目标索引，首页 128 行约 2876 shared hits；必要依赖后页退回主键，约 6125 hits。后续只在实测改善时保留追加的两个目标＋keyset 索引。独立预审还发现发布清单历史 Context 与组合修订起点两项重要缺口，报告在工作树 scratch 的 `task-2-pre-review.md`，需 RED 回归、修复与复审，故此项仍未验收。

预审修复 RED：本地 `learning-core::impact_contracts::exact_composition_can_be_a_strict_impact_start` 实跑 0/1、退出码 1，未知 `composition` 变体；Linux 冻结包 `b4-task2-review-red.zip`（SHA-256 `fd6056a630657f74da0034f2f9968aed6fa86b1f35ebdf2bb1627b1691bd6372`）在新库实跑旧锚发布场景 0/1、退出码 101，`consumers.rs:263` 取到 `None`。原始服务器证据 `客户端绝对路径`，包内 237 文件和日志哈希已独立核对。允许实现子代理进入修复。

修复首包 `b4-task2-review-green1.zip`（SHA-256 `6c7ca2dc4a2aa9047c942ea1d86a2c22571fa2db5b29141777c29a47f252e324`）在新库专项 7/7，通过旧锚、组合起点及父路径；证据 `客户端绝对路径`。同包 scale 8 合成库测量显示结构复合索引首页 shared hits 2876→314、约 0.65→0.11 ms；必要复合索引后页 6125→6000、约 1.80→1.86 ms，未证明该样本加速，但使原先走主键的后页改走目标优先索引，满足施工单的有界访问约束。两条索引暂保留，绝不把后者写成提速。对照证据 `客户端绝对路径`；样本不同 UUID/缓存，不能当作生产基准。组合反向查询计划、最终迁移及全量/审查尚待完成。

组合反向 SQL 的合成夹具首轮失败因子组合自身给目标块多一条 occurrence、旧计数断言未更新；失败证据 `.../b4-task2-composition-diag1-scale1` 已保留。修正夹具的 `b4-task2-composition-explain2.zip` 在 scale1/8 两个独立库各 1/1，通过全部 36 条计划/轮；组合首/后页 custom/generic 均使用既有 `occurrence_composition_target`，未见主键回退，故不加第三索引。原始证据为 `客户端绝对路径` 和 `...scale8`。

最终候选 `b4-task2-final-candidate.zip`（SHA-256 `a601bf17976a22e28352dffc5a3ba33af3bd10f5cb174632088a7232abc540ee`）在五个全新库与四种冻结旧程序夹具完成工作区全量：51 条 suite、242 通过、0 失败、1 默认忽略、退出码 0；根任务独立复核 238 文件和原始日志哈希，证据 `客户端绝对路径`。此前同包基线因 `b2_upgrade.rs` 期望 `[1..6]` 而实际有 0007，85/1 停止；最终仅把当前迁移版本期望改 `[1..7]`，旧 checksum/快照/回放断言不变。独立最终审查仍发现两项 Important：固定阅读来源组合缺少 Reading 直接包含边；发布根和阅读来源重复快照可在去重前重复计费。需行为 RED、修复、重测和复审，Task 2 尚未验收或提交。

两项审查回归的冻结包 `b4-task2-final-review-red.zip`（SHA-256 `98b5d0b558791c2793717840e4a59d70d5f5caed43deaee0af1ed8d61084e54d`）在新库真实运行 9 项：7 通过、2 失败、退出码 101；固定阅读边为 0 组而期望 1，重复路径场景未得到 `Complete`。原始证据 `客户端绝对路径`，包内 238 文件及日志哈希已独立复核；已放行同一实现者修复，随后仍需独立复审。

修复包 `b4-task2-final-review-green1.zip`（SHA-256 `accd58fee52a77894229b707da7c8e3e8611e5600eb736faba436deb684ac657`）在新库专项 9/9、退出码 0，两个 RED 均转绿；根任务核对原始日志与 238 文件哈希，证据 `客户端绝对路径`。同一独立审查者复审关闭两项 Important，无新增具体问题；完整报告在工作树 scratch `task-2-final-review.md`。同包五个全新数据库与四种冻结旧程序夹具完成工作区全量：51 条 suite、244 通过、0 失败、1 默认忽略、退出码 0；逐项源码与原始日志哈希复核通过，证据 `客户端绝对路径`。提交 `39aed849049aa712f71750d4284ec05b4423395e`，工作树干净，Task 2 验收完成。必要索引在合成样本中未证实加速，但后页确实改为目标优先计划；不据此声称性能提升。

## Task 3：局部证据与动态工作态关系

**Files:** `crates/learning-core/src/evidence_query.rs`、`tests/evidence_contracts.rs`，`crates/learning-db/src/queries/{scope,evidence}.rs`、`tests/{evidence_query,evidence_explain}.rs`。沿用 B3 的同事务 `references::Session`，不另造重复的关系／审查投影模块。

执行记录（进行中）：首个冻结包 `b4-task3-red1.zip`（SHA-256 `37527b363136d788bc527e50befe11a8a5dfe84b30c721e1e048af6f9ef7cad7`）在全新 B4 数据库编译成功，固定关系方向、条件、理由与所选审查配对用例实跑 0/1，失败于查询返回 `None` 而非编译或夹具前置；退出码 101。根任务复核 241 个源码文件和原始 stdout/stderr 哈希，证据在 `客户端绝对路径`。独立契约预审明确匿名认识审查只按已授权端点聚合一枚、不可用隐藏 ID 排序，发布多阅读对同一关系的所选审查必须逐来源保留；实现与正式审查尚未完成。

首轮实现包 `b4-task3-green1.zip`（SHA-256 `b97e5e43013667b42930faf48dd87da64ce684e29bb9bf78107cbe63911a1214`）在另一全新 B4 库专项 3/3、退出码 0，覆盖固定方向/条件/审查配对、历史关系与当前 head 区分、最新撤回只抑制动态关系；根任务复核 241 文件与原始日志哈希，证据 `客户端绝对路径`。独立预审已发现可见关系审查/固定选择边未完整计入公开图预算；需真实 RED 对照、修复和扩充权限/匿名/分页场景，此包不代表 Task 3 验收。

预算回归包 `b4-task3-budget-red.zip`（SHA-256 `758b7334b5d37b5e3fecf5619809cd2cbbafc8d163f42aa65f90d857e590c833`）在新库编译成功、专项 0/1，失败于有可见关系审查时未得到预期 `BudgetExceeded`；无审查的同关系/同限制对照先通过，退出码 101。根任务复核 241 源文件及原始日志哈希，证据 `客户端绝对路径`。日志未输出实际错误状态，不推断具体变体。独立预审还发现先聚合全部可见关系、后收费的顺序可能让多对象查询先碰到 B3 操作预算；修复需边完整授权边对可见图增量收费，超限原子返回公开 `BudgetExceeded`，并给展示的另一端、选择边与可见审查准确去重计费。

修复包 `b4-task3-budget-green2.zip`（SHA-256 `30e9bb06fd641e5a3861a073878f1caa5b14fb92f0ced7e162af204e0101bec8`）在新库专项 6/6、退出码 0，先前预算 RED 与低工作上限动态多关系对照均通过；根任务核对 241 个源文件及原始 stdout/stderr 哈希，证据 `客户端绝对路径`。独立复审又指出路径数在发现阶段、而不是本页原子组形成时收费，与 Task 1 分页语义不一致；需把发现工作与本页路径上限分开后重测，Task 3 仍未验收。

分页路径回归包 `b4-task3-page-red.zip`（SHA-256 `da5200f68ec661b7a2952b452a854f809e39c6c72551e72fc59556795d83c851`）在新库编译成功，精确用例 0/1，失败于两条可见动态关系、`limit=1/work_limit=8` 时未返回预期 `Truncated`；退出码 101。日志未打印实际状态，不附会具体变体。根任务复核 242 个源码文件及原始日志哈希，证据 `客户端绝对路径`。实现者将路径收费移到游标过滤后的本页完整组，随后需重跑。

修复包 `b4-task3-page-green1.zip`（SHA-256 `40f01008c490c2ff82c031e7ca0e14136fc49e2f2253ea50d08f61292e3a1b46`）在新库 DB 专项 10/10、退出码 0，包含分页回归、H@1/H@2+E1/E2/X 双向、发布双阅读的两份固定审查及外部端点撤权恢复。另以同包独立运行 core 严格输入契约 0/2、退出码 101，均是编译后真实行为 RED：反序列化零限制和直接构造 nil 游标未拒绝。两条命令结果不能混记；根任务复核 242 文件和两份原始日志哈希，证据 `客户端绝对路径`。需修 core 契约，继续根撤权、关系空间撤权、翻页重授权及 EXPLAIN。

授权候选 `b4-task3-authorization-green1.zip`（SHA-256 `cb59f0e214ac1aa7e92e72815c83b746681fed87de5b6645720f672ba8e92758`）在新库 DB 专项 14/14、退出码 0，新增隐藏审查根、第三空间关系撤权、旧游标后重授权和 12 隐藏候选分页。core 契约独立运行 1/2、退出码 101：零限制等已转绿，但 `judgment_incomplete` 单位游标附 `review_id` 仍被反序列化接受（`evidence_contracts.rs:35`）。因此整体未验收；须收紧游标原始形状。根任务复核 242 文件及两份原始日志哈希，证据 `客户端绝对路径`。

严格游标修复包 `b4-task3-cursor-green1.zip`（SHA-256 `6bb4f89e4fd031f1ee6f1daa3c179cb00bdda1cc0fc79cdd2878fc2f71cd0faf`）在新库分别通过 core 契约 3/3 与 DB 专项 14/14，两条命令均退出 0；前述匿名游标 RED 已转绿。根任务复核 243 个源文件和两份原始日志哈希，证据 `客户端绝对路径`。动态反向 SQL 的真实 EXPLAIN、工作区全量及最终独立审查仍未完成，Task 3 继续进行。

同包在新合成库另跑默认忽略的动态 SQL 测量 1/1、退出码 0，保存 8 条原始 `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)`：custom/generic × 撤权前后 × 首/后页。后页 4/4 使用 `reference_dependency_target_page` Index Scan；首页 4/4 在仅 260 条目标关系的小表选择 Seq Scan+Sort，因此**不能声称首页目标索引命中或提速**。暖缓存单样本时间约 0.07–1.04 ms，不是生产性能基准。根任务复核 243 文件与原始日志哈希，证据 `客户端绝对路径`；需增加无关目标规模验证首页目标选择性。独立最终审查另发现同一精确认识审查被多个固定 reading 选中时可能拆成多个页面，需先行为 RED、再按精确审查合组并保留所有来源，故 Task 3 尚未验收。

精确认识审查合组回归包 `b4-task3-judgment-group-red.zip`（SHA-256 `5817f3ec0ebb38891400ba2f21ab87fc8727ebde5c2ad92db982f3ff9caa1bc0`）在新库编译成功、专项 0/1、退出码 101；两个固定 Reading 选中同一 J 且 `limit=1` 时未满足单组 `Complete` 断言。实际状态未打印，不推断其具体变体。根任务复核 243 文件与原始日志哈希，证据 `客户端绝对路径`。独立审查另发现 B3 Session 累计上限在大固定范围与多小候选组合下可能早于 B4 公开预算触发 Storage；需真实 PG 阈值回归和同一事务内逐候选完整授权的修复。

合组修复包 `b4-task3-judgment-group-green1.zip`（正确 SHA-256 `422e60aed8aeae7dd7a1c36fa401b9bdab080f78c7496ba0c956742bbb271f61`）在新库 DB 专项 17/17、core 契约 3/3，分别退出 0；双 Reading 同 J 的单组结果与两个来源均获断言，独立审查关闭该 Important。根任务复核 243 文件及原始日志哈希，证据 `客户端绝对路径`。前次人工抄写了 62 位预期 SHA，Linux 在上传前阻止并记录无建库、无测试；随后复核正确 64 位哈希，用同一源码包运行，未替换包。

Session 阈值首包在本地发现单组合 1900 节点违反 B1 每组合 512 上限，未派发。修成四子组合的 `b4-task3-session-red-v2.zip`（SHA-256 `30f2e2ff60df95dd39001d8f27b0e85805949f4d53beb287548b73bb1bdaffac`）在新库只到编译阶段，两个 E0308：测试夹具把 `TextDraft` 传给需要 `ContentDraft` 的 `seed_content`；0 测试执行，**不是行为 RED**。243 文件和原始日志哈希已核对，证据 `客户端绝对路径`。修夹具、重冻结、再跑真实阈值。

仅修夹具类型的 `b4-task3-session-red-v3.zip`（SHA-256 `e25630b99bd2186bf7e8148337c27fed2d8f7aebff4cd0d8af9931253a6192fc`）在新库编译成功，合法的四子组合／1900 显示块 Reading 与 90 条动态关系已完成创建；精确用例 0/1、退出码 101，失败于最终 `evidence(...).await.unwrap()` 返回通用 `Storage`，测试 3.40 秒无超时，尚未执行 Complete/90 断言。根任务复核 243 文件与原始日志哈希，证据 `客户端绝对路径`。该现象与累计 Session 上限推断一致，但公开日志不含内部错误码，不能仅凭 RED 断定根因；同一只读快照内逐候选完整授权的修复及 GREEN 差分仍待验证。

逐候选新建 B3 授权 `Session`、共用同一只读 repeatable-read 事务的修复包 `b4-task3-session-green1.zip`（SHA-256 `5711b6108864957498a34252e40a2f50077073fbefee0612a86ac1c52b1e9700`）在全新库通过 DB `evidence_query` 18/18 和 core `evidence_contracts` 3/3，两个进程退出码均为 0；命名的 `large_reading_scope_does_not_exhaust_reference_session_across_small_dynamic_closures` 通过。根任务独立核对 243 个包内文件及两份原始 stdout 哈希，证据位于 `客户端绝对路径`。此为真实阈值差分，不替代第二轮查询计划、工作区全量与独立终审，Task 3 暂不验收。

第二轮 SQL 计划包 `b4-task3-explain2.zip`（SHA-256 `43a25775a1ab4acb1d36864eac1b1c7857d39ab4ed5904e6b5ffb067138abcfb`）在新合成库默认忽略夹具 1/1、退出码 0。增加 512 条无关目标关系和 4096 条无关块依赖后，custom/generic × 撤权前后 × 首页/后页共 8 条原始计划全部以 `reference_dependency_target_page` Index Scan 访问依赖，未出现 Seq Scan 或 Sort；关系、授权和 overlay 按各自索引访问。暖缓存单样本 0.059–0.800 ms，仅用于验证该夹具的计划，不推导生产耗时或跨夹具提速。根任务独立复核 243 个包内文件及原始 stdout SHA，证据位于 `客户端绝对路径`。独立代码审查的两项 Important 已关闭；尚待冻结工作区全量与旧夹具兼容回归，Task 3 暂不验收。

首个全量冻结包 `b4-task3-full-regression.zip`（SHA-256 `70c6bcd43190854112bbb338f18b79f821dd392c5c8c9b7b63bbbcf399a7c56f`）在五个独立新库完成一轮 `cargo test --workspace`：54 条 suite 结果、265 通过、0 失败、2 个默认忽略的 EXPLAIN，退出码 0；四份冻结旧程序 bootstrap 均退出 0，旧源码 manifest/fixtures 哈希匹配。`cargo fmt --check` 退出 0；严格 `cargo clippy --workspace --all-targets -- -D warnings` 退出 101，发现 `EvidenceJudgment` 大枚举变体。根任务独立审计 243 个包内文件与 workspace 原始 stdout SHA，证据位于 `客户端绝对路径` 与 `.../b4-task3-full-checks`。实现将 `Available.review` 改为 `Box<EpistemicReview>`，独立静态复审确认 JSON 与实际字节计费未变；修复包 `b4-task3-clippy-fix.zip`（SHA-256 `96e7bb53297f9af70cb00de7f0112d6ed53636c82a94b884b25d28c56f132214`）需重新跑完整验收，Task 3 暂不验收。

后续严格 Clippy 依次揭露 `fixed_selections` 参数数、内部分页枚举体积和测试冗余切片；每项均以最小修复、独立静态复审及新包复检处理，失败证据保留。最终冻结包 `b4-task3-clippy-fix3.zip`（SHA-256 `ed171674a23777850b403546d1e041678c407460dbc49922bc69ae4a93ec8fd4`）在 Linux 的 `cargo fmt --check` 与 `cargo clippy --workspace --all-targets -- -D warnings` 均退出 0；同一精确包在五个独立新库的一轮 `cargo test --workspace` 取得 54 条 suite、265 通过、0 失败、2 个默认忽略的 EXPLAIN、退出 0；四份冻结旧程序 bootstrap 均退出 0。根任务独立复核 243 个包内源文件、检查原始日志及全量 stdout SHA `c76733f0e1f185644187800a6a9f33d058d1d93356513807c2cef4fe24d4c1ca`，证据位于 `客户端绝对路径` 和 `.../b4-task3-clippy-fix3-full`。独立终审无开放发现，已提交 `148e79c0df79061645a9104da1171b2aca9987e5`（`feat: project local evidence queries`），工作树干净；未推送、合并或部署。Task 3 验收完成，B4 整体仍在进行。

- [x] 查询精确端点的 supports/opposes/tests 等用户关系、选定关系审查及认识审查；区分阅读视图的固定选择、发布清单和工作态动态发现。从任一精确端点都发现受影响的关系断言，解释保留其保存方向、条件与来源版本；另一端仅在范围成员且当前可读时继续走，不能产生自动真假或传递性结论。
- [x] 隐藏端点、关系本身或必要依据时整项省略；审查依据不完整时只显示无身份的 `Incomplete`，不得借局部查询跳过历史授权。可见过滤先于总数/分页；两个版本同在视图时结果不混合。
- [x] 同源 E1/E2 与反例 X 固定期望、关系撤回、权限恢复、语义环、重复关系和预算边界使用真实库测试。通过后提交 `feat: project local evidence queries`。

## Task 4：解释路径与影响遍历

**Files:** `crates/learning-db/src/queries/{impact,traversal,work}.rs`、`src/projection/path.rs`、`tests/impact.rs`、`tests/projection_authorization.rs`。

Task 4 首个固定期望 `nested_occurrences_keep_two_explanations_without_cross_product` 已在全新隔离 Linux 数据库跑出真实行为 RED：冻结包 `b4-task4-struct-red1.zip`（SHA-256 `ea63bc534fc8f4a3d03437e4edc1e3db4751666e6f304e2d7829537a4a44bcc1`）编译并建成合法的重复子组合与 Release，随后 `traverse` 最小接口返回 `None`，测试 0/1、退出 101；不是 Windows rustup 环境失败或夹具创建失败。根任务独立核对 245 源文件与原始 stdout SHA，证据在 `客户端绝对路径`。独立审查明确：从块 H 起点出发，间接 root 与 Release 消费者的 `locations` 均须汇总 H 在对应范围中的两条完整 `[rootOcc, childOcc]` occurrence UUID 路径；`child→root` 边自身的 `step.location=[rootOcc]`，`root→Release` 边本身无新位置。GREEN 测试按此收紧，Task 4 仍在进行。

结构首切片 `b4-task4-struct-green1.zip`（SHA-256 `5bc8ffb57894d6516202b95915f2ef27bb436fe05ee57eeed0bd4430909324be`）在另一全新 Linux 库通过该精确测试 1/1、退出 0；根任务独立核对 245 个源文件及原始 stdout SHA，证据在 `客户端绝对路径`。这只证明重复子组合结构多跳；独立预审另发现 Reading→Release 固定包含边尚未实现，需独立真实 RED/GREEN。Task 4 不在此验收。

Reading→Release 固定选择场景 `released_personal_block_reaches_release_only_through_fixed_reading` 在冻结包 `b4-task4-reading-red1.zip`（SHA-256 `035a607516b4cfb3571be143549e1411e207f6bc1eb66abca705f1f53b017706`）的新库运行：编译、合法发布夹具与查询成功，`Complete`、`Displayed` 断言已过，消费者数量实际 1、固定期望 2，测试 0/1、退出 101。日志未打印缺失消费者身份，不能仅由该日志具体命名；当前代码的 Reading 邻居为空，与缺失固定 `release_reading` 边相符。根任务独立核对 245 源文件和原始 stdout SHA，证据在 `客户端绝对路径`。需同一 RR 快照重新授权固定阅读后形成 GREEN。

精确 `release_reading` 三元组和同一只读 RR 快照内的 Reading 授权补丁冻结为 `b4-task4-reading-green1.zip`（SHA-256 `8c6632f79e763b65347ae60563aeb2be05ed7632b709985eadd600b16ad76728`），在另一新库 `cargo test -p learning-db --test impact` 两条结构用例 2/2、退出 0。个人 H 的 placement 在 Reading 与间接 Release 消费者中保留，之前的重复 occurrence 用例仍绿。根任务独立复核 245 源文件及原始 stdout SHA，证据在 `客户端绝对路径`。Necessary、语义/谱系、撤权/预算/分页及全量仍待完成，Task 4 不在此验收。

Task 4 `ImpactQuery` 没有 `include_dynamic` 字段；显式列入 `Semantic` family 即为 Reading 范围的工作态语义发现入口，需同时纳入固定选择与当前可见 head，分别标注固定与 `DynamicWorking` 来源。Release 仅取冻结固定选择；`ReviewSelection` 只表示精确固定选择边，不代替动态开关。动态关系至少一端实际显示，另一端需当前可读且在范围内才能续跳。此为批准设计 §15/17 的接口解释，后续以独立固定期望验证。

Necessary 菱形首个 RED 包 `b4-task4-necessary-red1.zip`（SHA-256 `c9ada0b7dec2a6606dc032a59c8bce96d19e3ab9110f3c25108d613dc5526976`）在 Linux 编译成功，但合法发布夹具创建阶段 `publish(...).unwrap()` 收到 `Invalid("unsupported_content_version")`，测试 0/1、退出 101；`traverse` 尚未调用，**不能记为 Necessary 路径行为 RED**。根任务独立核对 245 源文件与原始 stdout SHA，证据在 `客户端绝对路径`。需修合法的版本/范围夹具，保留独立菱形期望，再重新取得真实 RED。

仅把夹具改为支持 V2 的 `publish_evidence`、保留四块菱形和所有精确断言的 `b4-task4-necessary-red2.zip`（SHA-256 `84655a30bfd111860b2016b93382b30e92ec09631d826e15886db8c7a368dcd7`）在另一新库编译、发布及 `traverse` 成功，`Complete` 断言已过；消费者实际 0、固定期望 3，测试 0/1、退出 101。后续 C 两路径和 Basis role/position 断言尚未执行。根任务独立核对 245 源文件与原始 stdout SHA，证据在 `客户端绝对路径`；这是结果层真实 RED，需完整授权的 Necessary 邻居实现转绿。

Task 4 独立 Necessary 邻居及逐候选新 B3 `Session` 的 `b4-task4-necessary-green1.zip`（SHA-256 `ed07f7707edff02f9c8452b5b3ca2095023bc631bc9e7076767c1c2351f03c01`）在新库 `impact` 专项 3/3、退出 0，菱形的 C 两条解释与每步 Basis role/position 固定断言均通过；两条结构场景继续通过。根任务独立复核 246 源文件和原始 stdout SHA，证据在 `客户端绝对路径`。Task2 已验收 `QueryStore::direct`/`necessary` 函数体与会话策略未改，只开放六个 helper 的模块可见性；其累计 WORK 假设仍在 Task5 单独做真实 RED/GREEN。Task4 语义/谱系、撤权/预算/分页及全量仍未验收。

语义隐藏桥固定期望的 `b4-task4-semantic-red1.zip`（SHA-256 `8c29be1d6ca1ede81099ebbca1eef501ede80cc56eefb0d67a4e998cae4e2a71`）在新 Linux 库编译、夹具创建与 `traverse` 成功，`Complete` 断言已过；撤权前未找到精确选定的关系消费者组，测试 0/1、退出 101。保存方向/type/provenance、H 两跳和撤权后隐藏桥断言均尚未执行，不能把此 RED 当成撤权已验证。根任务独立核对 246 源文件和原始 stdout SHA，证据在 `客户端绝对路径`。下一步补固定/工作态分源和当前关系完整授权，再跑全部断言。

`b4-task4-semantic-green1.zip`（SHA-256 `b2a6b0a126e240373096d361b912c71dd23bb082d391a9442c318e5f62f81fca`）在另一新库的 `impact` 专项 4/4、退出 0，隐藏桥测试的关系方向/type/provenance、H 两跳和关系空间撤权后的零消费者断言均已执行通过；前三条结构/必要场景继续通过。根任务独立复核 247 源文件与原始 stdout SHA，证据在 `客户端绝对路径`。此包尚未覆盖下述 Context-only 资格与外部端点，Task 4 不在此验收。

语义实现预审发现两项仍需独立真 RED/GREEN 的范围边界：① 仅因 Necessary 闭包或 release manifest 成为 Context 的关系、若既未被固定选择也不是 Reading 当前合格 head，不可经 `Semantic` 继续连另一端；Release 尤须只取固定选择，`Stored` provenance 不能替代资格。② 关系另一端当前可读但不属于 scope 时应作为终端外部端点展示，不可继续扩展；不能直接丢弃，也不能伪造范围成员。两项均尚未验收，`b4-task4-semantic-green1.zip` 的首轮测试不能覆盖它们。

Context-only 关系负例 `b4-task4-context-red1.zip`（SHA-256 `7fe30e96392f8531935d88074c717da2e7d9a4881abe7104899aed9c48c7ce93`）首次在新 Linux 库实跑：合法 `publish_evidence(readings=[])` 与 R 起点查询成功，`Context` 成员和 `Complete` 状态已过，但 `relation_only.consumers().is_empty()` 失败，0/1、退出 101。混合 Necessary→Semantic 负例与固定 selected R 正对照尚未执行；不能从日志推断具体消费者身份。根任务独立核对 247 源文件与原始 stdout SHA，证据在 `客户端绝对路径`。需以固定/当前 head 资格门控修复后完整跑完全部断言。

资格门控候选 `b4-task4-context-green1-external-red1.zip`（SHA-256 `a3510daccd5ad95e66a3726bfd660b39b59ebf26ef2bdf931b86c8bb2f39287a`）在两独立新库分开运行。A 库跳过外部端点测试，其余 `impact` 5/5、退出 0，Context-only R 起点零语义消费者、E→R Necessary 但不续 R→H、固定 selected R 起点 `FixedReleaseManifest` 正对照全部执行通过；证据 `客户端绝对路径`。B 库仅跑外部 X 终端测试，编译、夹具、查询及 `Complete` 通过，消费者实际 1、期望 2，0/1、退出 101；具体 X 身份、路径及禁止续跳断言尚未执行，证据 `.../b4-task4-external-red1`。根任务独立核对两库同一 247 文件包与两份原始 stdout SHA。外部 X 仍需 GREEN，Task4 未验收。

外部终端修复及旧固定/新动态新增测试包 `b4-task4-external-green1-oldnew1.zip`（SHA-256 `059b95fe3ec61a0465d6f5b7db28831a02a6750cc8246ae46858c63ddce8117b`）分两新库分别运行两个精确命令，但都在同一测试二进制编译阶段 E0507：`tests/impact.rs:914` 的 `find` 闭包移出非 Copy `BlockRef`。两个进程退出 101、实际执行 0 项测试；**不能计外部 GREEN 或旧/新行为 RED**，原始失败证据分别位于 `客户端绝对路径` 和 `.../b4-task4-oldnew1`。根任务独立核对 247 源文件与两份原始 stderr SHA，第三轮全专项未启动。仅修闭包捕获后需新包重测。

仅修该闭包捕获的 `b4-task4-external-green1-oldnew2.zip`（SHA-256 `ba37e49750d7ee88c5ba1903bcbbae870da7f5a68d4cc1424771fa7565fa4eed`）在三独立新库依序通过：外部 X 终端命名用例 1/1、旧固定 R@1/新动态 R@2 精确版本与方向命名用例 1/1、`impact` 全专项 7/7，三次均退出 0。前者的 X 可读终端、不得续至 R2/H 且 X 不能作为范围起点断言，后者 Reading 固定/动态分源和 Release 只取旧固定修订的断言均执行；根任务独立复核同一 247 文件包与三份原始 stdout SHA。证据分别位于 `客户端绝对路径`、`.../b4-task4-oldnew2`、`.../b4-task4-external-oldnew-full2`。这仍仅为 Task4 当前专项，ReviewSelection/Lineage/预算分页/全量待验。

Lineage 边族需携带系统操作的 Derive/Split/Merge 具体类型。现有 `ImpactStep.relation_type` 只表达用户语义关系，故 Task4 增 `lineage_type: Option<SystemLineageType>`：Lineage 边必填，其他族禁填；对应构造与契约测试同步。B3 `lineage::read::load` 只把可见性升至 crate 内，沿用其同一事务的完整成员授权，不直接从系统谱系表原始行投影。这是已批准“谱系边单独说明类型”的最小契约细化，仍待实现/测试。

Lineage 的范围裁定：Reading 与 Release 均只纳入完整授权、且至少一个精确 output 属于本次 fixed scope 成员的系统操作；只向当前可读且在范围内的 exact outputs 续跳。若所有 outputs 均在范围外，operation 本身也不作为终端消费者返回，避免发布后动态派生混入固定 Release。可读范围外端点作为终端的特例仅适用于设计明确规定的 Semantic 另一端，不推广到系统谱系。需独立固定期望与真实 PostgreSQL 验证。

Lineage 首个冻结测试包 `b4-task4-lineage-red1.zip`（SHA-256 `4eb1d712b99fe423032788ce39c040e511432cc0e28a5016633abbc6106848f8`）在新库编译并建成 Split、Merge、发布后范围外操作三组夹具，但各自首次 `traverse` 都返回 `Invalid("impact_family_not_implemented")`，测试 0/3、退出 101。Split 双输出、Merge 撤权断桥和 Release 零消费者的结果断言尚未执行，**这只是查询族入口 RED**；根任务独立核对 248 源文件与原始 stdout SHA，证据在 `客户端绝对路径`。需放开合法族入口后再取得结果层 RED/GREEN；ignored 稠密 EXPLAIN 尚未运行。

只放开合法 Lineage family、邻居仍空的 `b4-task4-lineage-red2.zip`（SHA-256 `da303d42e64a6b16d9d6d5c4aa0b25941d69e68babfe240f911e210ee76ef814`）在新库编译、三组夹具和 `traverse` 均成功：Split `Complete` 后消费者 0、期望 3；Merge 首次查询找不到 operation；发布后 output 全在范围外的零消费者负对照完整通过，合计 1 通过、2 失败、退出 101。Split 具体路径与 Merge 撤权断桥尚未执行。根任务独立核对 249 源文件和原始 stdout SHA，证据在 `客户端绝对路径`。这是结果层真实 RED，仍须独立谱系邻居实现、GREEN 与稠密 EXPLAIN。

固定审查选择 `b4-task4-review-red1.zip`（SHA-256 `684fe44ed0706fb3ef7c530a68bfc263179c20f1e0153dca2d01d387529e82fd`）在新库编译、夹具与 Relation 起点查询成功，`Selected` 成员和 `Complete` 已过，消费者实际 0、期望 1，命名测试 0/1、退出 101；RelationReview 起点与 Release 后续断言尚未执行。另以同包独立运行 core `lineage_type` 严格契约 1/1、退出 0，验证只有 Lineage 步携带系统类型且现有非 Lineage JSON 不因空字段变化。根任务独立复核 248 源文件与两份原始 stdout SHA，证据在 `客户端绝对路径`。ReviewSelection 需结果层 GREEN，不能从首个 RED 推断后续已满足。

`b4-task4-review-green1-page1.zip`（SHA-256 `3829010fdcc66dbcbbc3760578a7787003520c01f38d39738623535c16ab0c98`）在另一新库分别通过 `impact` 专项 8/8 与 core 契约 14/14，两命令退出 0。固定关系与其审查到 Reading、Release 两步选择链路、Context-only 不冒充选定、Necessary 菱形 `limit=1` 三页且 C 双解释同组、低工作上限 `BudgetExceeded` 空组、旧游标在撤权后重新授权均完成断言；没有撤权后再授予的测试。R 起点双来源当前仅断言两路径 provenance 非 `Stored`，尚须精确枚举固定与动态各一。根任务独立核对 249 源文件与两份原始 stdout SHA，证据在 `客户端绝对路径`。Lineage、深链/环、稠密 EXPLAIN 与全量仍待完成。

Lineage 结果层 GREEN 冻结包 `b4-task4-lineage-green1.zip`（SHA-256 `746d999539e16a883b13ea795182dc51797452ac8a9c334699acad984b326b59`，250 文件）在新隔离 Linux PostgreSQL 库运行 `cargo test -p learning-db --test lineage_impact -- --skip explain_dense_lineage_reverse_target_before_and_after_revoke --nocapture`，3/3 通过、退出 0：Split 的两个精确输出与三个消费者、Merge 隐藏第二输入后的断桥、Release 发布后范围外输出不混入均通过。根任务独立复核源清单和原始 stdout SHA，证据在 `客户端绝对路径`。同一精确包包含 ignored 的稠密首/后页、撤权前后 custom/generic EXPLAIN 测试；须在另一新库执行并据真实计划判断索引，尚不能验收 Task4。

扩展边界包 `b4-task4-impact-extended1.zip`（SHA-256 `a96c81d89175cebe1aa137fda1db32274a721e46957ea1b28ab154a23bfe9a31`，250 文件）在新独立 Linux 库执行 `cargo test -p learning-db --test impact -- --nocapture`，10/10、退出 0；包含十块 Necessary 深链、三关系 Semantic 环、R 双来源精确 provenance 与撤权后再授予的旧 cursor。根任务独立核验源清单及 stdout SHA-256 `ba3a94f62f9b06f4f04445e4256be698aa1c5e806419349b7c57e9f027512b1b`，证据在 `客户端绝对路径`。匿名、稠密必要邻居、各预算以及 EXPLAIN 仍待确认。

谱系稠密计划用同一 GREEN1 精确包在另一新库运行 ignored 测试，1/1、退出 0；8 条撤权前后 × custom/generic × 首/后页的原始 `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` 均已保存。260 个目标 operation 加 512 无关 operation 下，前 6 类计划对 `lineage_input` 顺序扫描并过滤 512 或 640 行，8 条均有 Sort；撤权后的后页改用 Bitmap Heap Scan。各模式前两页行数为 128/128，撤权后为 128/2；均为缓存命中的小夹具，不能外推生产延迟。根任务独立核验 250 文件清单及 stdout SHA-256 `e0d9559c1d4be604ced590d7e5b1ca87d88e07c52992e634e95e2ee7a52c97f3`；证据在 `客户端绝对路径`。目标前缀索引是否必要，待更大的无关候选和索引前后真实对照后决定。

匿名与范围外密集候选包 `b4-task4-anonymous-dense-check1.zip`（SHA-256 `3fb5fd7e09f9d5afa2d41ff555ffe0d3dde7c3b4e8dd80a13681955d081029c1`，250 文件）在新独立库分别通过匿名审查、261 条跨三批 Necessary 候选及 core 边/路径阈值三个命名测试，各 1/1、退出 0；根任务独立复核三份 stdout SHA 和源清单。证据在 `客户端绝对路径`。稠密测试的 260 个候选不在固定范围，虽候选空间获授权、SQL 返回三批且隐藏 basis 空间被撤权，实际在范围过滤处跳过，**仅证明范围外密集候选不消耗公开预算且结果字节不变**；不能当作逐候选完整闭包失败的测试。匿名测试验证 B3 唯一匿名 `Incomplete` 与 Impact 零组/不可作起点，游标不泄 stream/review/basis UUID。

最小迁移 `0008_lineage_target_index.sql` 仅增加 `lineage_input_target_page(block_id,revision_id,operation_id,position)`，不改变谱系权威表/旧迁移。`b4-task4-lineage-index-explain2.zip`（SHA-256 `84f3055cbdfc1b3c7f51d1a5380392ba73ddc83afcccd6293a598f73c111d93f`，251 文件）在又一新库 ignored 测试 1/1、退出 0；与同形基线比，8/8 条计划改走该目标前缀索引的 Bitmap Index/Heap Scan，未再报告 `lineage_input` 的无关行过滤，最终行数保持；8/8 仍有 Sort，单次缓存样本用时有升有降。根任务独立核对包清单及 stdout SHA-256 `ed3661a21a7b54cfa77a2ab59ad76c2d644fe9dd1f5ce6fb99942a8f124c1112`，证据在 `客户端绝对路径`。因此仅确认目标访问形状改善，不宣称排序消除或生产提速。

Task4 的 Structural 直接复用 Task2 `consumers.rs` 的块/组合反查 SQL；Necessary 的 Task4 与 Task2 生产 SQL 正文等同；Semantic 动态反查与 Task3 `evidence.rs` 对应 SQL 的 token 序列一致、仅空白不同，参数及 keyset 相同。独立审查确认可引用 Task2 两规模各 36 条和 Task3 第二轮 8 条原始稠密计划作**单次 SQL 访问路径**证据，不能将其外推成 Task4 多跳 BFS 总耗时/预算证据。Reading→Release 新查询是按 `release_reading` 主键做精确 EXISTS。最终报告须列精确包、SQL/索引版本及规模参数。

最终跨模块静态审发现固定 Reading 来源路径可能串线。独立固定期望冻结包 `b4-task4-reading-source-red1.zip`（SHA-256 `60bae151a0d65b629d9f7168b3775e7276fa5f1ffbc8c373051513c0c35ed523`，251 文件）在新库编译、构造 Release 根 D→C→E 与两份 Reading 并成功 `traverse(Complete)`，随后对 Vc(base C) 的位置断言真实行为 RED：实际多出仅属于 D 的 `[Docc,Eocc]`，期望只有 `[Eocc]`；0/1、退出 101。失败发生在首个 Vc `locations` 断言，后续 Vd 合法路径/解释断言尚未执行，不能预称通过。根任务独立核验 251 文件及 stdout SHA-256 `507a219666036cbf7e93b7a9d78ae0d801d2abfb5288b4d77153bdeda9fc9b6f`，证据在 `客户端绝对路径`。修复须按每个 Reading 自己的已授权 B3 source occurrence 路径限制组合→Reading 续跳，并保留 Vd(base D) 的合法嵌套路径；GREEN/全量待验。

按每个 Reading 的 B3 已授权 source 快照保存 root-relative occurrence 路径、在组合→Reading 续跳前筛选的修复包 `b4-task4-reading-source-green1.zip`（SHA-256 `62c29095857c50d17b1a32c99fc2e2da0abb9514d99605def79d9833249ebd5e`，251 文件）在新库命名回归 1/1，再在另一新库通过 `impact` 全专项 13/13、`lineage_impact` 普通专项 3/3，三命令均退出 0；Vc 只保留自身 `[Eocc]`，Vd 合法 `[Docc,Eocc]` 与解释正对照已完整执行。根任务独立审 251 清单及三份原始 stdout SHA；证据分别在 `客户端绝对路径` 和 `.../b4-task4-reading-source-green1-suites`。

同一字节 `b4-task4-reading-source-green1.zip` 随后在五个全新隔离库完成一轮 `cargo test --offline --locked --workspace -- --test-threads=1`：56 条 suite、283 通过、0 失败、3 个默认忽略的原始 EXPLAIN 测量，退出 0；P0-A/B1/B3-schema/B2 四种冻结旧程序 bootstrap 均退出 0，旧源码 manifest、fixtures SHA 和旧 checksum/数据快照不变。`cargo fmt --check` 与严格 `cargo clippy --locked --workspace --all-targets -- -D warnings` 均退出 0。根任务独立核对 251 源文件清单、workspace stdout SHA-256 `f07cacd1f6694b5a44d9ef227315f082f363d5087fb51ea78a8a000787538cdb` 及两项检查原始日志；证据在 `客户端绝对路径` 与 `.../b4-task4-final-checks1`。独立终审无开放具体 Important/Minor（`.superpowers/sdd/2026-09-23-p0b4/task-4-review.md`），已提交 `2294011479d5bf7eb44eb93f6c81b40af250e3a0`（`feat: traverse authorized revision impacts`），工作树干净；未推送、合并或部署。Task4 验收完成，B4 整体仍待 Task5。

- [x] 从起点沿允许边族在选定范围中逐步展开，扩展前授权每个节点与完整必要闭包；隐藏桥接点不能连接两侧。路径内已访问节点不无限循环，但保留到同一消费者的不同可见解释路径和所有 occurrence 位置。同一消费者所有路径/位置作为一个分页原子组；若单组超过路径或字节上限，整次查询为 BudgetExceeded。
- [x] 结构边显示“引用此旧版，采用新版需明确生成新组合”；语义边显示“建议复查”，带类型与方向；谱系边单独说明派生来源。状态 `Complete/Truncated/BudgetExceeded` 的差异由独立固定期望验证；超工作预算不返回部分路径或假零。
- [x] 针对菱形、语义环、深链、稠密邻居、撤权后缓存/下一页、旧/新修订同视图跑 PG 固定期望；公开工作与组分页在 PG 验证，其余节点/边/路径/字节硬上限由 core 契约验证，避免 B3 范围构造上限先触发而误判 B4。稠密查询复核 Task2/3 同形 SQL 的原始 `EXPLAIN (ANALYZE, BUFFERS)`，新增谱系查询独立取得索引前后 8 条原始计划与时间；仅断言目标访问命中，不声称提速。已提交 `feat: traverse authorized revision impacts`。

## Task 5：Attention 全链、升级、Compose 与审查

**实际文件：** `crates/learning-db/tests/b3_scenario.rs`、`tests/impact_session_threshold.rs`、`src/queries/consumers.rs`、`docs/p0b4-{boundary,execution,verification}.md`。旧升级验证复用现有测试和四份冻结 bootstrap；没有追加部署配置。

- [x] 固定 Attention 原文、H@1/H@2、同源 E1/E2、X、人工审查、C、阅读、发布和拆分，逐条断言关键可见消费者、各边族完整解释与位置，覆盖撤权和恢复；预期取自写入事实而非查询结果自比较。
- [x] 使用四种冻结旧程序路径生成旧数据并跑新迁移升级；旧行、收据、正文摘要和历史发布不改。五个隔离升级/普通数据库及新 B4 空库 Compose 通过，原有 B3 卷/证据未删除；源码包、逐文件 hash、真实命令/退出码/日志/镜像与资源均入证据。
- [x] 独立终审核契约、授权/路径、SQL 计划与新索引、旧 API、升库及原始运行证据，无开放 Important/Minor。宿主与全新 Compose 各 284 通过、0 失败、3 个显式忽略的测量，B3 原 222 项逐一仍为 `ok`；标记 `P0_B4_VERIFIED / NOT_PRODUCTION` 与 P0-B 内核完成。生产、P0-C 与 P1-A 仍另立验收。已提交 `fc25e91954419bd0c6c272d00577db1b74ce6ce8`（`docs: record verified impact query milestone`）。
- [x] 横向审计 Task 2 `QueryStore::direct` 与 Task 3 的 B3 `references::Session` 2048 对象／131072 工作上限；Task 2 合法共享闭包夹具在旧实现查询层 RED、逐候选完整授权同一只读快照下 GREEN，增强同测试单变量差分及五库/Compose 回归通过；Task 3 大固定范围和动态候选回归仍通过。

横向审计的早期只读推断（历史假设，不能代替测试）：Task 3 动态候选曾有把范围外关系／端点累计进 `Session.selected` 的风险，已在 Task 3 用逐候选 Session 和真实 PG 阈值回归修复；Task 2 Necessary 候选多在固定范围必要闭包中，不能类比为对象数增长，它可能因多源重复验证共享闭包，使同一 Session 的 `work` 超 131072。Task 2 须用合法 PG 夹具确认失败层；B3 对循环根的独立深度验证不能用 `selected.contains` 短路替代。

Task5 首批 Attention 固定期望包 `b4-task5-attention-red1.zip`（SHA-256 `0eb248c7f74ca1dda6db481e1b87d3daeb1cc681c917f954b0047538ae449c6e`，251 文件）虽命名 red1，实际在新独立 PG 库 **1/1 通过、退出 0**；三条 E1/E2/X 固定关系与其 E→关系→H→C 路径只证明指定解释存在，未排除多余消费者/位置，也未覆盖 H@2、权限恢复及拆分对 B4 的影响。根任务独立核对源清单与 stdout SHA-256 `01a22572199e46feeed7094c597ddbfdcd1aa8cacd146708e1db23f12ed0201d`，证据在 `客户端绝对路径`。它只是中间专项，不是 Attention 全链验收。

Task2 `direct` 授权 Session 累计阈值包 `b4-task5-session-red1.zip`（SHA-256 `9ced9d5fb714d2fe1f4e16d816fdfe2b717f5eacc5e05c1ed9ddc9a1e66e8b44`，252 文件）在新库编译、合法草稿、公开 API 创建 H/A/组合/Reading、B3 `content.read(H)` 与 `reading.read_versioned` 均通过；只在 `QueryStore::direct(T)` 首次得到通用 `Storage`，命名测试 0/1、退出 101，175 个 Context 消费者断言尚未执行。测试整体 5.75 秒，不是 15 秒只读语句超时；公开错误本身不揭示内部 B3 work 计数，暂只定性为查询行为 RED，待逐候选新 Session 的差分 GREEN 与审查确认根因。根任务独立核对 252 源清单和 stdout SHA-256 `cf52727413bdb84ba7807def3fa5786a5d1b359b794b9d53523dfb908dd33ec7`，证据在 `客户端绝对路径`。

最小修复只在 Task2 `direct::necessary` 的固定范围成员过滤之后，给每个候选新建 B3 `Session` 并在原同一只读 RR 事务里完整 `authorize`，不短路循环根深度、不改 SQL、公开预算或其他边族。冻结包 `b4-task5-session-green1.zip`（SHA-256 `e4b631e2fe0857626d429ee5b4bf07131fe90a2005428d74b245ce0f568ec25d`，252 文件）在另一新库精确测试 1/1、退出 0，`Context/Complete`、175 个消费者及每条 `Basis` role/position/reason 全部执行；RED/GREEN 两包该测试文件 SHA 完全相同，仅消费者实现及并行 Attention 测试变化。根任务独立核对包清单与 stdout SHA-256 `c9687e841248450af80a9d40b308243fd6d9d30ae4bf0ea47e538660b10cdf9c`；证据在 `客户端绝对路径`。此差分强烈支持累计会话问题，但公开 Storage 不单独证明内部计数；最终全量仍待。

Attention 扩展包 `b4-task5-attention-full1.zip`（SHA-256 `0cf1c97a5c2daae2bf94020b4fa9dd4ba6f30102edfdd1c152927049770d8e6b`，252 文件）在新库命名测试 1/1、退出 0：H@1/H@2 精确版本、Reading/Release 的位置与结构解释、撤权恢复、Split 后旧发布不延伸/新 Reading 谱系均执行；但 E1/E2/X 跨族路径仍以 `.any()` 证明存在。根任务独立核对包清单与 stdout SHA-256 `bce218cce163759eef6bb54d71f91c3fa51f83c84d6158495d01e0a57c6eca38`，证据在 `客户端绝对路径`。

更严格的同包 `b4-task5-attention-threshold2.zip`（SHA-256 `6fccd824b14cf64bbb2a774dd1bdce627950c860949e996f3aaf74c9c7577b6a`，252 文件）在另一新库依序通过阈值控制与 Attention 命名测试，各 1/1、退出 0。前者用 `traverse` 与 `direct` 在相同固定范围取得 175 个 Context 消费者和完整 `ImpactResult` 等值；后者新增 E1/E2/X 的 Semantic 单族全组/路径等值、H@1 Necessary 单族五组角色位置等值，以及七个固定选择→Reading 的完整一步解释；原扩展断言保留。根任务独立核对包清单及两份 stdout SHA-256 `d131e4052c11f5c4e855aa82f2ce7a333bf4838d5f6740057df4dea73ae5670c`、`8a7d5347f4f5aa9c68f387a9a4a32480a179b94ca813dd7f29535cc8253c4953`，证据在 `客户端绝对路径`。全量/Compose/终审仍待。

增强归因对照包 `b4-task5-session-red2-control.zip`（SHA-256 `8a245766993fe8a39da004f967237ddd2a330993b1d44454bec56bad0c671d50`，252 文件）与上述 6fcc GREEN 包逐文件比较，**唯一源码差异**是 `queries/consumers.rs` 恢复 Task4 `2294011` 的旧字节，增强阈值测试完全同字节。在新库 B3 创建/读取及 `traverse(T)` 的 175 组 `Complete` 控制均通过，首次失败仅在旧版 `direct(T)` 返回通用 `Storage`，0/1、退出 101；后续 direct 结果断言未执行。根任务独立核对包清单、唯一差异与 stdout SHA-256 `41aa0eb7779e7a27f4c90892e8e03f3117658ff5a1fa2ea1f09ada7c6f59aa35`，证据在 `客户端绝对路径`。这与 GREEN 构成同测试强差分，定位旧 `direct` 会话复用是触发条件；公开错误仍不暴露内部精确计数。

新增 `docs/p0b4-{boundary,execution,verification}.md` 的候选 `b4-task5-final-candidate1.zip`（SHA-256 `daa8e47c282a66ad13209b298d58be6b213064f72b2c0510c6c2ce0d73d12477`，255 文件）与已通过 6fcc 包的原 252 个源码/测试/迁移文件逐字节相同。Linux 五个全新隔离库一轮 `cargo test --offline --locked --workspace -- --test-threads=1`：57 条 suite、284 通过、0 失败、3 个显式忽略的 EXPLAIN，退出 0；四份冻结旧程序 bootstrap 及旧快照/迁移 checksum 断言均通过，严格 fmt/Clippy 均退出 0。根任务独立审 255 文件清单、workspace stdout SHA-256 `befa5c4f3fcc68c18a1929a5c07b2b5b950aa0ecf1b4e2171baccd42e4e56c5f`；并从 B3 原始全量 stdout 提取 222 个唯一通过测试名，与本次 stdout 逐名对照 222/222 仍 `ok`、缺失 0。Task3 的 1900 固定范围/90 动态候选，以及 Task5 的 Attention/Session 命名测试本轮均 `ok`。证据在 `客户端绝对路径` 与 `.../b4-task5-final-candidate1-checks`。该批次后继续做空库 Compose 与最终独立审查。

同一 daa8 精确包在新 Compose 项目 `learning-system-p0b4-t5-fc1-compose1`（显式 `-p` 覆盖配置内 B3 默认名）完成独立空库验收：启动前项目容器/卷/网络不存在，启动后五库非系统对象均为 0；`config/build/pg-up/stop` 及测试 `up` 均退出 0，容器内 57 条 suite、284 通过、0 失败、3 忽略，四冻结旧程序 bootstrap 均退出 0。PG 2 CPU/4 GiB、test 4 CPU/4 GiB、内部网络无宿主端口，admin/runtime 均无 SUPER/BYPASSRLS；实际测试镜像 repo digest `sha256:8b4ffdfaa1ec6bb0a12527f00936513238453b83affc7d65faa4a2db04a9b8a1`。仅执行 stop，项目及新 `test_pg/test_evidence` 卷保留，旧卷未动。根任务独立核对 255 文件清单、原始导出 workspace stdout SHA-256 `59eb94347d6afbe7a3bedaee084dd63b24aaf9ff1c9895594d09419752250640` 与 57 条 suite 计数、空项目预检及资源记录；证据在 `客户端绝对路径`。其后完成终审与最终文档固定。

最终独立审查记录 `.superpowers/sdd/2026-09-23-p0b4/task-5-review.md` 复核宿主证据 38+10 项、Compose 50 项文件哈希、255 源文件前后不变、B3 旧 222 测试在宿主与 Compose 原始日志逐一仍 `ok`，以及 0007/0008 索引计划的有限结论；无开放 Important/Minor。终审后的文档包 `b4-task5-final-docs3.zip`（SHA-256 `dbfea71448baa1029123f0cd598d8e5bd2d9d32bb836965f4f6a2994abb9fc49`，255 文件）与 Linux 实测 daa8 包仅三份 `docs/p0b4-{boundary,execution,verification}.md` 字节变化，其余 252 文件逐字节相同；根任务和独立 reviewer 均复核清单与文案。已提交 `fc25e91954419bd0c6c272d00577db1b74ce6ce8`，工作树干净，未推送/合并/生产部署。B4 内核状态为 `P0_B4_VERIFIED / NOT_PRODUCTION`；见[仓内验收记录](../../p0b4-verification.md)。

## 全程证据与停线

每个候选包白名单只含源码/锁文件/迁移/必要 deploy，不含 DSN、密码、原件与 `.runtime`。实际测试数来自原始日志，RED 编译失败不冒称行为失败，新覆盖初次 GREEN 不伪称修复。可见图超过公开工作预算时返回 `BudgetExceeded`；内部/隐藏候选扫描超时或防护失败时返回通用存储不可用并保存查询计划。两者都不把未遍历区域写成无影响，也不附内部计数。历史 PostgreSQL/Neo4j 实验 `NOT_PASSED` 四项权限失败原记录不改。无 remote，不创建虚构 PR、不推送或合并到原开发分支。
