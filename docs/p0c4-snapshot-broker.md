# Task6 快照副作用证据：受限代理基础

更新：2026-10-20。代码检查点 `1521cfeac821072f95fa46d3637c97a3f553343b`，仅完成 S1 写入禁用基础，Task6 未完成。

恢复时仅凭任务状态不能确认导出字节是否已经发布。后续正向证据链将由 Root 托管实际操作与持久化记录，目的端独立校验；缺少证明时继续 Unknown/HOLD。

S1 新增可选 `snapshot-effect-broker --serve-disabled` 与显式 `learning-worker --snapshot-broker-client`。它们使用 Unix socket；客户端只能发固定信号，不能传入 actor、job、token、文件路径或文件描述符。Root 保留私有锁和 epoch 目录，实际内核 UID/GID 必须匹配；消息大小、字段、重复键、额外帧和 SCM_RIGHTS 均受限。Root 单实例锁保持整个生命周期，目录或 socket 身份被替换即拒绝。配置错误不会回退到旧 worker。旧默认 worker 入口保留。

当前代理不读取数据库配置、不 claim、不写数据库、不处理导出，不生成 NoEffect、AlreadyCommitted 或可用恢复资格。正确通信只得到 writer-disabled / Unknown，CLI 以 78 退出；这是明确不可用状态。

验证分开记录：Windows 2 项纯测试通过。固定 Linux 格式、类型检查和 all-targets 严格 Clippy 通过；5 项普通测试覆盖真实 socket pair、peer credentials 和三处 SCM_RIGHTS 拒绝，6 项显式 Root fixture 覆盖真实 UID/GID、锁、epoch 和目录替换。两组分别为 5/0/0 与 6/0/0；不相加为 PostgreSQL 恢复验收。fixture 的 cwd 引导不是实际 RO bind mount，真实只读 epoch-leaf 挂载与普通客户端部署门仍待执行。

下一步需接入真实 claim 与完整历史、Root 私有写入凭据及持锁 producer、源端捕获和目的端独立字节验证，再支持 NoEffect / AlreadyCommitted。原 runtime 权限不放宽，正常恢复仍需 CompleteBackup；S2–S4、Task6 与生产激活均开放。

代码：[代理核心](../crates/learning-worker/src/snapshot_broker/mod.rs)、[Linux custody](../crates/learning-worker/src/snapshot_broker/linux.rs)、[协议](../crates/learning-worker/src/snapshot_broker/wire.rs)、[Root fixture](../crates/learning-worker/tests/snapshot_broker_root.rs)。
