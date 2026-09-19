# P0-A 内容边界

当前实现为内部 Rust 库；没有登录、HTTP API 或前端。Principal 只能由经过身份验证的受信任调用方构造。运行数据库账号不是终端用户账号，不可交给浏览器。

文本契约 content-v1：kind=text，format=markdown，intent 支持 knowledge/note/question/idea/conjecture/observation/evidence/conclusion。未知字段和其它形式拒绝；不存在可绕过授权的自由 target 或 basis_refs。

标题限 300 个 Unicode 标量，正文限 200,000 UTF-8 字节；language 为 1–35 ASCII 字符，不宣称已经验证 BCP 47。修改理由为 1–1,000 Unicode 标量。空正文允许；不裁剪空白、不改变换行、不执行 Unicode 归一化。U+0000 在入口拒绝，因为 PostgreSQL text/JSONB 无法保存它。

内容摘要：给 TextDraft 加入 contract_version=1，递归按对象键排序，数组保持顺序，使用无多余空格的 UTF-8 JSON，再做 SHA-256。摘要不含块 ID、修订 ID、作者、时间或理由；这不是任意 JSON 数值的跨语言规范，而是本阶段受限文本结构的字节契约。固定样例见 contracts/content-draft.example.json，摘要为 35db28db30d4e6d1b4f81c32a365ba9d2b34757bbb6694e09ab308baab1037ec。

block_id/revision_id 由服务生成；request_id 在首次调用前生成并保存，重试必须保留全部命令。幂等范围为 (actor_id,request_id)，请求摘要包含操作、目标、基础修订、正文契约版本和理由。重试先检查当前写权限；已撤销或降为只读时拒绝重复写入请求，即使收据已经存在。相同内容的新请求仍可创建独立块或新修订。

Revision 返回正文、父修订、作者、理由和 UTC 时间。UUID 序列化为字符串，时间为 RFC 3339，枚举为小写。JSON 示例属于内部契约，不是完整系统交换包 1.0。

read 对不存在与不可见都返回 None；read_many 最多 200 个输入，按首次出现顺序去重，只返回可见项。list/history 每页最多 100 项，使用降序 (created_at,id) 键集游标；list 的时间和 ID 属于稳定块，history 属于修订。next_cursor 只根据可见行计算，末页为 null；没有隐藏计数。游标不是发布快照，跨页期间的修改/撤权按新查询生效。

写入事务锁定幂等请求、授权行和目标块。READ COMMITTED 下读取权限与正文在同一语句快照判断；已完成响应不能收回。保存先锁住授权行时撤权等待保存结束，撤权先提交时保存拒绝。读取与写入权限分别验证。

错误：Invalid=契约无效；NotFound=写入目标不存在或无权；Conflict=基础修订过期，携带当前可见修订；IdempotencyConflict=同键不同请求；Storage=数据库或连接错误，不返回 SQL/凭据。前端后续保留草稿与不确定请求，收到服务确认才标记保存，冲突不自动覆盖。
