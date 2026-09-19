# P0-B1 组合与发布边界

这是受信任服务调用的 Rust 内核，不是 HTTP 或浏览器应用。Principal 只能来自认证后的服务入口；运行数据库账号不交给终端用户。本阶段固定复用文本块，个人层、语义关系、资产与恢复按后续阶段交付。

## 身份、引用与顺序

CompositionRevision 固定引用 BlockRef 或 CompositionRef；两种引用都含稳定对象 ID 和精确修订 ID。组合不复制正文，工作头不改变旧组合和发布。kind 只有 document/section，创建后不变。

每个 occurrence 表示一次出现。重排保留身份，复制生成新身份；已有 occurrence 只能延续同一稳定目标，可以选其新修订。从新组合移除出现项不会删除块；已移除的 ID 不由客户端复活。需要恢复历史结构时可以发布旧修订，或在新修订生成新出现项。

嵌套的同一章节在两处出现合法，完整路径逐层包含 occurrence ID。结构环按每条路径的 composition_id 检查，禁止 A@2→B@1→A@1；不会把兄弟分支共享节点误判为环。所有遍历都按固定修订，没有 latest/head 隐式引用。

## 命令与版本

SaveComposition 新建时 composition_id/base_revision_id 都为空；修订时二者都有值，基础版本与当前工作头一致才保存。客户端的新 occurrence_id 为空，服务端分配。作者与时间由服务端生成。

标题最多 300 Unicode 标量，理由为 1–1,000 标量，不含 U+0000。每层最多 512 nodes，空组合合法；发布一次 1–16 个不同根，全部根位于指定发布空间，只读依赖可跨空间。未知 JSON 字段一律拒绝；直接构造 Rust 命令也经过相同校验。

组合摘要为 canonical_json 的 composition-v1 域，包含 kind/title/有序 occurrence 与精确 targets。节点顺序有意义；发布根按 composition_id 排序后计算请求摘要。P0-A 的 content-v1、原摘要和历史收据保持兼容。

## 读取与授权

CompositionStore.read 返回固定根、去重的组合修订和块修订；nodes 保留阅读顺序。对象数组按稳定 ID、修订 ID 排序，其数组位置不表示阅读位置。

本关采用完整闭包授权：根、每个子组合和每个块都必须可读；任一不可见或不存在，read 返回 None。ReleaseStore.read 对所有发布根校验；active 只校验所请求组合的活动根。没有隐藏占位、隐藏计数或原始 JSON 旁路。

读取在只读 REPEATABLE READ 快照中检查 grants、结构和正文。请求开始前已提交的撤权生效；已开始或已返回的数据不能追溯收回。写入先发现已授权依赖、按 UUID 排序锁全部 grants，再复核闭包；目标空间要写权限，依赖空间只需读权限。锁住授权后撤权等待事务完成。

基础版本冲突为 Conflict，发布指针过期为 PublicationConflict；后者只含已经授权的 composition_id。不存在与无权写均为 NotFound；数据库错误为 Storage，不返回 SQL、连接信息或隐藏引用。

ReleaseStore.state 提供新客户端和冲突后的恢复入口：返回该组合的当前工作头、当前发布根和 publication_token。工作头与发布根分别检查完整闭包；同一多根发布中的其它文档不会扩大这次读取范围。任一所需闭包不可读时返回 None。active 仍可单独读取可见的已发布根。

publication_token 是组合专属的 64 位小写十六进制摘要，输入域为 publication-basis-v1、composition_id 和内部 last_release_id。调用方将它视为不透明值，填入 PublishRoot.expected_publication_token；服务端在组合行锁下比较。它不是授权凭据，不暴露多根 release ID；另一组合的令牌不能复用。即便再次发布相同内容，令牌也会改变。

## 事务、重试与发布

(actor_id,request_id) 在块、组合、发布之间统一唯一。新 request_key 登记表回填 P0-A receipt；旧摘要不变。操作共用原 advisory lock 命名空间。同键同命令返回原结果，同键不同操作或命令拒绝。重放仍要求本次权限，重放检查先于基础头过期检查。

保存一次事务提交组合修订、出现项、工作头和收据。发布一次事务锁定所有根，比对预期工作头和发布指针，再写 release/roots、全部发布指针、outbox 事件及收据。尾部失败全部回滚。

保存知识或组合只推进工作头。发布明确选中的组合才改变发布指针。回退追加 release 引用历史组合，工作头保持不变，不删除发布或内容历史。

outbox_event 只追加 composition_released 事件，含 release ID、payload_version=1 和时间，根清单由不可变发布记录解析。尚无消费者；事件入库不表示预览、搜索或导出已完成。

## 有界读取与数据库升级

- 最多 16 层组合，根为第 1 层。
- 每次闭包校验累计最多 4,096 个展开 occurrence；复用的子章节在每条路径上分别计数。
- 每次闭包最多 2,048 个不同修订对象；保存时拟生成的根计入一个对象。
- 去重块正文最多 8 MiB，按 PostgreSQL `octet_length(content::text)` 的 JSONB UTF-8 序列化计数，在读取超出预算的正文前拒绝。它不是 HTTP 包大小或整个响应字节的上限。
- state 需要验证工作头和发布根；相同根只验证一次，不同根分别受上述预算限制，最多两份闭包，避免两个各自合法的文档版本无法读取并发基础。
- 超限返回固定 Invalid code，不返回隐藏对象或精确隐藏计数；合法的小组合可被复用，但嵌套后的整体仍需满足预算。

READ COMMITTED 写入接受查验时已经提交的精确引用；本次请求内尚未创建的临时引用不支持。固定修订与逐路径检查保证结构，不要求全局图锁。

新增 0002，不修改 0001。新表持有组合键 FK，运行角色只有 SELECT/INSERT 和身份表指定指针列 UPDATE；没有修订/收据/事件的 UPDATE、DELETE 或 TRUNCATE。数据库凭据仍属于受信任服务边界，不宣称具备用户级 RLS。

升级测试必须使用专用空数据库：原 0001 → 历史夹具与旧收据 → 完整迁移 → 原值和重放比较。该测试会拒绝已存在迁移表的库；重复完整验收请建新测试批次，保留旧卷和证据。
