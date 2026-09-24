# P0-C3 融合快照交换详细设计

**状态：** 用户已批准 C3 详细规格；2026-09-24 的施工计划审查补充了可验证的旧 Worker 隔离、授权锁、索引闭包与目录 staging 细则，均保持原批准目标。本文细化已批准的 [P0-C 总设计](../../../../../docs/superpowers/specs/2026-09-23-p0c-assets-jobs-portability-design.md) C3 节。基线为 C2 验收提交 `085c37724ae291559fe684ded8e7e29e97e3f93e`，仍为 `NOT_PRODUCTION`。

## 目标与选择

用户要把一个精确阅读版本及其可见知识、个人插入、关系、审查和原件带到另一实例，且可以验证、幂等导入，不泄露当前无权查看的内容。首版以**同一身份在不同实例间迁移**为目标：目标端已由可信管理员创建相同 `app_user.id`、`space.id` 和必要 `space_grant`；包不得创建用户、空间或授权。跨账号/跨空间复用属于后续显式“派生新身份”流程，不伪装成精确导入。C3 不等于 C4 全量备份，不承担整库、凭据、作业、收据或历史操作恢复。

考虑过三个方案：直接导出当前 `read_versioned` 投影，容易做阅读副本，但丢失组/位置身份、未放置锚、修订元数据，无法精确导入；直接打包底层表，容易保真，却会绕过当前授权、带出隐藏 ID 和个人数据；采用**同一只读事务求授权闭包，分别生成原样交换包与安全阅读投影**。选第三种。两个产物在 manifest 中有不可混淆的 `capability`，导入器只接受 `exact_import_v1`，绝不把裁剪投影补成原修订。

## 输入、边界与模式

输入为 `actor_id`、精确 `view_id + view_revision_id`、`ReadingMode`、`include_originals`，可选显式 `resource_version` / `source_segment` 根。`ReadingMode` 只决定阅读副本的展示，不暗中扩大原样包的范围。请求还明确 `include_personal`：只有其为真且 actor 是 overlay owner，才可能产生含该阅读修订的原样包；否则仅生成 `reading_copy_v1`。模板默认不带个人层；模板身份重映射另立设计，C3 不称模板可原样导入。

`exact_import_v1` 的先决条件是所选修订及全部数据库/内容依赖、父/前序修订、证据和必要资产在当前授权下完整可见，且目标身份模型可满足。任一必要节点不可见时，不发布部分原样包。可以单独生成已裁剪阅读副本，使用泛化“内容未包含”标记，不输出隐藏对象的 ID、类型、路径、数量、正文或摘要，也不保留会暗示其原样可导入的原始 revision ID。对调用者返回统一的不可原样导入原因类，不泄露哪个私有节点阻断。

原样包的授权读取必须在**同一个 `REPEATABLE READ READ ONLY` 数据库事务**内完成：锁定精确阅读修订，按当前 actor 授权遍历闭包、记录对象清单及资产清单并计算 manifest。现有 `ReadingStore`/`AssetStore` 每方法各开事务，不能直接拼接冒充一致快照；实现应提取内部 `&mut Transaction` 共用查询。引用遍历使用既有精确引用/组合的对象、深度、边和字节预算，并对整个包共享总预算，不按根重置。超过上限明确失败。资源版本/来源片段不在现有内容精确引用图内，只接受显式根并逐项授权；不得按相同 SHA 扫描或顺带导出别的资源。

## 包内容与确定性

固定版本 `format_version=1`，包根有 `manifest.json`、按类型/身份排序的不可变对象 JSON、可选 `assets/sha256/<前两位>/<摘要>` 原字节、`validation.json`，以及可选 HTML/Markdown 阅读副本。每个 JSON 对象包含完整的数据库不可变字段和其附属行；按确定性规范序列化后对**完整记录**计算 SHA-256，manifest 记录类型、精确身份、相对路径、字节长度和文件 SHA-256。`content_sha256` 仍按其原业务定义验证，但不能代替完整记录哈希；作者、原因、时间、父修订等差异必须能检测。同一输入和同一数据库快照得到相同的权威 JSON/manifest 摘要；生成时间、作业 ID、校验报告等非权威信息与确定性内容分离。

包需要的对象闭包包括：块与 `block_revision` 的完整祖先链和引用依赖、精确 `block_asset_use`；组合与其修订祖先链、所有 occurrence；overlay 与修订祖先链、group/placement identity、placed/unplaced 组、锚和 placement；reading view 与修订祖先链、evidence 及选择索引；所选关系及其修订祖先链、`relation_review_head` 身份、relation review 前序链、epistemic stream/review 前序链；显式资源根对应的 resource/version/segment；所引用资产的元数据。`reference_object` 与 `reference_dependency` 是受数据库触发器和延迟约束维护的精确索引：包必须包含期望的 registry/依赖清单，导入时由触发器生成 registry、显式插入依赖并在事务末验证精确一致。应保留未放置组的原锚与顺序，不把它投影为普通正文。所引用审查或关系自身有引用依赖时继续闭包和授权。只按精确修订闭包，不自动追随当前 head，也不枚举同空间其他对象。

不纳入 C3 原样包：`release` 及发布索引、lineage operation/receipt、placement migration proposal/decision、request/mutation/upload/reading/relation 等收据、`job`/outbox/result、用户/空间/授权行。这些历史/操作状态由 C4 全备份处理。新对象容器在导入时以所选精确修订为初始 head；已有对象的 head 和 published 状态不被快照倒退或覆盖。manifest 列出该排除边界，不能声称是源实例完整历史。

`include_originals=true` 时将闭包所需资产的原字节按 SHA 路径收入包，并读取时校验哈希和字节数；任何损坏/缺失导致原样导出失败。`include_originals=false` 时仅含资产元数据，manifest 标明 `requires_destination_assets=true`；导入前目标端须有同一 `space_id + asset_id` 的元数据和真实原字节，且逐字节校验。目标端不能因清单声明或同 SHA 的其他未授权资产就生成 `ready` 行或推定可用。阅读副本中的附件链接仅引用本包可公开的文件或显示不可用提示，不附服务器私有路径。

所有路径均由程序从固定 ASCII 类型、UUID 和 SHA 构造；导入器拒绝绝对路径、`..`、重复/大小写碰撞、链接、设备文件、额外文件、超限压缩率、超限总大小和不匹配的 manifest。首版只接受普通目录包，压缩输入全部拒绝。外部目录不得在校验时直接按路径反复打开：先由服务把受控文件流复制到只有服务身份可写的私有 staging，之后校验并保持其只读不变；Linux 输入遍历必须使用相对目录句柄与 no-follow 规则，防止检查后被换成链接。包大小、对象数、边数、层数、单文件大小均有明确上限，不能无限扩展；上限沿用现有闭包预算并在实现计划冻结具体数值。未知主版本、未知对象种类或非规范 JSON 拒绝，不做隐式兼容转换。

## 导出作业与撤权

C2 的 `JobInput`、0011 payload CHECK 和 outbox 目前只允许资产完整性事件；C3 必须以追加迁移扩展类型与 SQL 校验，不改 0011–0013 已应用字节，且不破坏旧 Worker。追加迁移要使旧 `p0c2_claim_job` 只能领取旧种类；新 Worker 使用按种类/版本约束的领取接口。导出由 actor 发起，作业载荷含精确阅读修订和选项，业务幂等键含 actor、精确根和选项摘要。Worker 只用 runtime 权限。读取 DB 清单后复制资产到隔离暂存并校验。完成发布时在**同一个事务内**重新检查根、闭包和资产的当前授权，锁定全部必需 grant，并以当前租约 token 条件提交结果；撤权先获锁则拒绝发布，发布先获锁则撤权等其提交。下载/读取临时包时再次授权并核对仍是同一范围；撤权后服务器拒绝交付并清理/过期暂存，不提供绕过检查的静态 URL。已离开服务器的旧副本无法被撤回，文档明确此边界。

重试必须收敛到同一权威包内容；临时文件名不可当授权凭据。崩溃留下的暂存可按作业 ID 安全回收，不作为成功结果。C3 可先提供 Rust 服务接口和受限 Worker 文件交付接口；HTTP/登录/网页下载属于 P1-A。

## 导入协议与冲突

导入器先在受限 staging 解析并验证格式、规范 JSON、每文件 SHA/长度、完整记录哈希、业务 `content_sha256`、精确依赖和父链、资产字节、路径、身份与归属。没有可信目标预置 `app_user`、`space`、授权，或 actor 不是个人 overlay/epistemic stream 的合法 owner/reviewer，则整个包拒绝。包自身的作者/审查者 ID 均必须在目标可信身份集合中存在，不能根据包创建用户或 grant。跨空间引用逐个按现有授权规则验证，不能用包把目标实例其他空间的私有对象“补齐”。

校验通过后先将原件原子持久化至内容寻址文件存储，随后在**单个数据库事务**内插入全部缺失的不可变行和索引行。事务内逐条比对已存在的同 ID 不可变记录：完整规范记录相同则复用，任何字段不同即 `IdentityConflict`；精确 revision ID 属于别的对象也冲突。容器按各表不可变身份字段比较：`block` 为 id/space/created_at，`composition` 加 kind，`overlay` 为 id/space/owner/root，`reading_view` 为 id/overlay，`relation` 为 id/space/overlay/type/origin/from/to/created_at，`relation_review_head` 为 relation+revision，`epistemic_stream` 为 id/space/overlay/target/actor；`head_revision_id`、`head_review_id`、`published_revision_id`、`last_release_id` 是可变状态，不参与同 ID 内容冲突比较。其他附属行所有列按原值比对。新容器可设所选修订为初始 head；已有容器保持当前 head/published/release 状态，永不回退。任何 FK、约束、授权、并发冲突回滚全部 DB 行，因此不存在半可见 reading。文件先持久化而事务失败时可留下不可引用的内容寻址字节，记录为待保守清理；不得误删仍被其他行使用的相同 SHA 文件。重复导入同包返回同一批次状态且不增加对象或移动 head；不同字节同 ID 拒绝。

现有公开写 API 会生成新 ID、作者、时间、收据且逐次提交，不适合导入；应新增专用 `SnapshotImportStore::import_exact` 或等价单事务内部 API，不修改正常写路径。DB deferrable 约束在 commit 前强制完成。导入端还应逐项验证目标数据库已有的引用对象，而非仅信任包内键。`reading_copy_v1`、模板或任何被裁剪产物从入口即拒绝导入。

## 验收证据与未纳入范围

在全新隔离 PostgreSQL 和原件卷中，以固定 Attention 样例验证：精确阅读与未放置组、图片/附件原字节、关系/审查、同一资产的两个使用点往返；同包二次导入不变；同 ID 改作者/父修订/正文拒绝；缺祖先、缺资产、坏 SHA、压缩路径、跨账号或撤权均拒绝且无半可见状态；导出中途撤权与交付前撤权都不能拿到包。再跑格式、严格 Clippy、完整工作区测试和四套冻结旧程序升级。Linux 验收仅在新的隔离项目，源码包和精确目标另行确认；不得触碰已有服务或生产。

交付物为版本化包契约、导出与导入 Rust API、追加迁移/Worker 扩展、真实 PostgreSQL 和文件存储测试、格式说明及验收记录。HTML/Markdown 是可选阅读副本。P1-A 的登录、HTTP 和 Web UI，跨用户模板派生，C4 整库备份，生产部署均不在本阶段。
