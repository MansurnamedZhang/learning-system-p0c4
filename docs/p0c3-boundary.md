# P0-C3 边界与事务语义

C3 提供本地目录快照和受授权的同身份原样迁移。它不创建身份/空间/授权，不是 HTTP 下载接口、UI、跨租户复制、协作同步或生产发布。C4/P1 的产品化接入与运维仍开放。

## 授权和历史

导出在规划、成功发布、每次交付时重核当前授权，完整必要闭包不得夹带隐藏父修订或审查。完成和交付持有必需 grant 锁，撤权先发生则拒绝；交付已经返回的字节/句柄无法远程撤销。阅读副本只含经裁剪的可读内容，不能携带隐藏 ID/路径/数量提示或冒充 exact。

导入 root 空间需要当前 write grant，个人 overlay 必须属于导入 actor。跨空间已存在且逐字相同的行可在 read grant 下重用；缺失行需要其空间 write grant。所有引用空间都要检查。已存在的第三方 epistemic stream 可在当前 read 授权下重用；缺失 stream 只能由其 actor 导入，reviewer/app_user 身份必须预置。因此包含另一 reviewer 历史的包未必能导入全新目标，不能通过自动创建或冒充用户来绕过。

碰撞检查先沿实际目标 parent/overlay owner 做授权，再决定是否可报告 IdentityConflict；隐藏对象的存在与不存在均返回通用错误。支持/反驳判断根据包中选中的历史 immutable RelationReview.state=Reviewed 及方向/证据语义验证，不要求可变当前 review head 仍相同。

## 目标 head 与回执

`PreparedSnapshotImport` 只能由 preflight 构造，保存经验证的 manifest 摘要及 sealed 原件文件句柄。它不是免复核凭证：事务内仍重核当前权限、所有表不变量、全部 immutable 字段、自然键和 `(kind,revision_id)` 的 registry 完整集合。不同对象的额外 registry 不能逃过检查；无关更新 revision 可保留。

新容器用确定性 HeadPlan 初始化七类 head：block/composition/overlay/reading_view/relation/relation_review_head/epistemic_stream。候选是包中父链末端；多个末端按规范 UUID 排序取首项。root Reading、它指定的 overlay 和 base composition 明确覆盖默认候选。没有候选即拒绝。此规则是目标初始化，**不是恢复源可变 head**。已有容器绝不后退 head/published/release。

先持久化并核对 CAS 原件，再按 request lock → actor+manifest lock → 排序 grant locks 开始数据库写入。固定插入顺序、完整冲突比较和 `SET CONSTRAINTS ALL IMMEDIATE` 在提交前执行；任何失败回滚全部权威行与 receipt。失败后可能留下不可引用 CAS 字节，只报告以供保守核对，不自动删除共享 digest。

专用 `snapshot_import_batch` 与权威行同事务提交。同 actor/request/manifest 重放保持绑定；同 request 换包冲突。新 request 导入已完成的同包得到自己的绑定回执，`reused=true`。仅因为业务行原本存在而没有成功的包回执，不算 reused。导入不生成普通 request_key、普通 mutation receipts 或 job/outbox。

## Worker 隔离与保留策略

C3 非终态为 snapshot_queued/snapshot_running/snapshot_retry_wait，旧 C2 的 queued/running/retry_wait 扫描不可见；新 Worker 分开有界扫描并使用 C3 fenced claim/renew/fail/cancel/completion。旧 token 不得完成新尝试。数据库时钟判断租约到期，最多三次；配置 SNAPSHOT_ROOT 才处理 C3。

job/lease-token 私有目录隔离重试，阅读副本逻辑 copy_id 使用稳定 job ID，重试不能改变规范内容摘要。服务重启按 job/保存的 attempt 和 manifest SHA 重新打开验证文件，不保留内存对象作为交付必要条件。

当前没有自动 TTL collector，也没有公开下载 URL。撤权/失效包和失败中间文件可留在非公开私有卷中，未来只能按服务掌握的 job 状态保守核对回收。租约到期不是磁盘文件到期；不能声称撤权会立即擦除字节。任何清理不得影响被其他已提交资产引用的 CAS 内容。Task 7 驱动仅 stop 新项目，保留卷、退出容器和证据，不执行 down -v/prune。
