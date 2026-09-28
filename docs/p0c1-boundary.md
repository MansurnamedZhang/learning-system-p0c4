# P0-C1 原件与图片块边界

本文件记录已验收的 C1 契约；隔离 Linux/Compose 与独立审查证据见[验证记录](p0c1-verification.md)。当前状态为 `P0_C1_VERIFIED / NOT_PRODUCTION`。

## 权威对象与读取

- PostgreSQL 保存空间内的 `asset` 身份、精确的 `block_asset_use` 或 `resource_version` 使用关系和上传收据；文件适配器保存按 SHA-256 定稿的不可变原字节。同一字节可以复用物理对象，但不合并资产身份或授权。
- `FsAssetStore::put_from_file(upload_id, source, UploadDeclaration { expected_size_bytes, max_size_bytes })` 强制调用方传入声明字节数和由可信服务策略给出的最大字节数，没有无界上传重载。首次写入 staging 与随后复制到 assets 卷内 `.finalizing` 时，均在每次写入前检查累计大小，结束时要求与声明字节数完全相等；超限或中断不能返回 `VerifiedBlob`，未完成文件只留作保守对账。C1 尚无 HTTP 上传入口，调用方负责在未来入口中选取可信上限，不得让请求者自行决定该上限。
- 就绪登记前，服务重开并校验定稿原件：声明为 PDF 时检查 `%PDF-` 前缀，声明为 PNG 时检查 8 字节签名，声明为 Notebook 时检查 JSON 对象具有 `cells` 数组及正数 `nbformat`。`application/octet-stream` 是允许的通用回退，包括调用方有意把已知格式作为通用字节处理的情况；此声明不触发 PDF/PNG/Notebook 的类型特异性检查。声明的具体类型与原件不符，或声明不支持的媒体类型时，不生成就绪行或收据。此为签名与形状检查，**不是** PDF/PNG/Notebook 的完整解析、文件安全扫描或内容可信性证明。
- `AssetStore::read_for_use(actor, AssetUseRef)` 按块的 **block_id + revision_id** 或资源的 **space_id + resource_id + version_id** 解析使用关系。块读取遵循既有精确引用闭包授权：必要依赖隐藏时，块及其资产不可读。它不追随块或资源的当前 head。不存在与不可访问的使用关系均返回 `None`，不泄露资产摘要、存储键或存在性。
- 应用层只通过 `AssetStore::open_for_use` 打开原件。它先进行同一精确授权，再由文件适配器校验数据库摘要键、文件大小和已打开文件的 SHA-256。文件丢失或损坏返回明确存储错误，不伪装为隐藏或空字节。`FsAssetStore::open_record` 是可信存储适配层入口，不可直接暴露给前端/用户。
- 图片及附件经 v3 正文引用已就绪资产，中央写入事务登记精确使用关系。Reading 中以既有 `InsertExisting` 放置 v3 块，以 `Move` 调整个人位置；移动不重写块修订、资产身份或原字节。历史固定 Reading 仍按原视图修订读取。

## 保守对账

`FsAssetStore::reconcile(protected, older_than)` **只生成报告，不删除文件**。调用方须从权威数据库、进行中的上传及备份保留清单构建保护集合，包含所有已引用/备份的摘要键，并以上传 UUID 或精确 `staging/<uuid>[.incoming-<uuid>]`、`.finalizing/<uuid>.pending-<uuid>` 键标记活跃上传。保护键格式不合法时拒绝扫描；资产卷或暂存卷根目录消失时失败，不返回空报告。

对账只考虑早于阈值、名称符合当前协议、位于真实目录中的摘要对象、暂存文件和 `.finalizing` 文件。只要有任何活跃上传标记，就**暂缓报告所有摘要对象**：原件定稿后摘要路径无法反推出上传 ID，此保守假阴性避免误报活跃原件。没有活跃上传时仍需完整保护引用和备份键。报告不是删除许可；C1 没有自动删除或人工删除 API。

## 后续边界

C1 不含 HTTP/登录、OCR、S3、生产部署或整库导入。C2 才实现通用任务、租约和可重试处理器；C3 才实现按当前授权闭包生成可交换融合快照及导入；C4 才实现完整备份和干净实例恢复。P1 另补学习事实闭包。本文件不得用来推断这些阶段已经实现。
