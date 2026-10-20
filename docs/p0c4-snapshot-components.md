# Task6 正向快照依赖组件

本页记录可选组件的分项实现与验证，Task6 整体保持 OPEN。五项基础恢复场景已独立复核5/5，来源仍为三项509956e和两项0df5128；最终同源整链回归尚未运行。

## 独立权限 profile 与回滚观察组件 C0

C0 使用独立真实 LOGIN、原顶层 READ COMMITTED 事务和固定锁定观察 helper，完整读取13个 job 字段与9个 outbox 字段，共享累计预算；公开事务 API 只有开始、观察和 rollback。它核对原三个 canonical 函数的正文、owner、设置及 EXECUTE 边界，本片不调用 claim/renew/fail，不产生业务历史、lease、写入许可、NoEffect 或 AlreadyCommitted。

权限检查限于当前数据库全 namespace 的直接 broker ACL、有效应用/序列权限，以及 pg_authid/pg_shadow 表与列的有效 SELECT 元数据。PUBLIC MAINTAIN、敏感目录读取和取消后的拒绝复用均已在真实 PG 验证；这不等于全部系统 PUBLIC ACL 或特权 builtin 白名单。源端两次最早预检均拒绝 C0 helper 的任意签名，安装此扩展的数据库仍不是可导出的完整 C schema。

七个源码路径：[可选权限 profile](../extensions/snapshot-claim-v1/install.sql)、[显式 installer](../crates/learning-db/src/snapshot_broker/mod.rs)、[catalog核对](../crates/learning-db/src/snapshot_broker/catalog.rs)、[owned观察事务](../crates/learning-db/src/snapshot_broker/transaction.rs)、[真实PG夹具](../crates/learning-db/tests/snapshot_claim.rs)、[库入口](../crates/learning-db/src/lib.rs)、[源端拒绝](../crates/learning-backup/src/source.rs)。原15迁移、64表和 A1/S2-B 行为保留。

Linux 格式、两包 all-target 类型检查及严格 Clippy、6项纯测试通过。三项真实 PG 与一项源端拒绝用例在四个独立空数据库中各为1 passed / 0 failed / 0 ignored，210份原件已独立复核并由 Root 限定接受。原450容器、449网络、1499卷的观测条目逐值相同；after完整投影另含本批停机PG、保留网络和卷各一项，不能称两个完整文件字节相同。客户端自然退出0后精确移除，PG一次TERM及wait0后停机留卷。

正式七文件提交 `a6e23483f3078954465d08778d6afab05b4ff602`；实际结果摘要 `ba592f54b1a20b49633f597147d1bd1df623a455158b4d56c242462037e44056`；Root限定接受摘要 `48ffb07ec73de17db1a41df698542c5050bce8855760332f7c2bca9592b30859`。Linux源包摘要 `aa2b954a95f04083adb4081c5d26bfeb42fbb97a0e79a9107097367570ee487c`（976a9cf加七路径，569文件，排除A2.1草稿）。Windows既有 backup all-target Clippy 101仍保留；本次Linux通过不改写旧记录。

上述检查属于 C0 组件验证，不增加基础恢复5/5，也不是最终同源整链。中间 Docker inspect 正文及含合成凭据的 SQL 未导出原文，相关断言与脱敏进程记录不等于独立系统调用追踪。下一步为同一事务前后状态与 durable INTENT/OUTCOME 业务历史；Task6仍 OPEN。

## 数据库观察组件 A1

A1 为独立真实 LOGIN 提供受限、可回滚的锁定观察事务。它检查固定函数正文、owner、设置和应用范围权限，读取任务与 outbox 的完整类型化行，并共享有界读取预算。源端两次预检都会拒绝此可选 helper 的任意签名；目前安装了该组件的数据库不能作为完整备份源。

本轮修复了四处关系类型敏感的权限检查：显式 CASE 保证只在对应对象类型上调用权限函数。实际 PostgreSQL 测试包含表、索引、TOAST、分区表、视图、物化视图、序列以及五组逐项 GRANT/拒绝/REVOKE。函数正文与既有权限边界保持。

Linux 格式、类型检查、严格 Clippy 及两项纯测试通过。真实 PostgreSQL 三项精确测试以及源端拒绝测试均已实际通过；已完成独立原件复核和 Root 限定接受。测试同时覆盖真实 broker 登录、完整行与预算，以及行锁阻止并发 claim、rollback 后释放。它只提供观察事实，不签发 claim、文件写入、NoEffect 或恢复资格。

角色审计仍有范围边界：应用关系与列 ACL 不等于全部系统目录直接授权白名单。后续正向授权组件必须另行固定完整角色与 catalog 合同；本组件未部署到生产。

## Root 日志字节组件 S2-B

三文件字节后端实现仅从全新私有目录创建的追加链：原目录 FD、no-follow 与单链接检查、共享预算、前缀身份和摘要、临时写入、文件同步、原子不覆盖改名、父目录同步和原 FD 读回。错误或未知状态后不重新打开、领养、重放或修复旧链。

Linux 格式、类型检查、严格 Clippy 与三项纯测试通过。8 项 Root 文件专项均实际通过，并完成独立原件复核及 Root 限定接受。它尚不是具有业务含义的 claim 历史，也不能单独提供 NoEffect 或 AlreadyCommitted 结论。

## 下一步

1. 真实 claim/renew/fail 的独立版本权限 profile、同事务前后观察与持久业务历史。
2. 完整观察扩展 schema 的真实 dump/TOC/ACL 夹具与恢复端禁用角色合同；当前有限 catalog audit 不能替代此门。
3. 文件写入许可、捕获排空，以及 NoEffect/AlreadyCommitted 的正向字节证据。
4. 最终冻结同一源码，重跑归一化、全表完整性、阅读及故障整链，再决定 Task6 是否关闭。

旧失败、未知结果和隔离资源保留。独立备份存储、CompleteBackup、C4 整关与生产部署另行验收。

## 本轮验证与来源

| 组件 | 固定源码和实际结果 | 限定结论 |
| --- | --- | --- |
| C0 | 源码a6e2348；Linux严格检查、6项纯测、真实PG三项＋源端一项；210份原件 | rollback观察；非claim/业务历史/NoEffect/导出资格 |
| A1 | 源码9944c02；Linux两项纯测试；真实PG三项、源端拒绝一项；214份闭合原件 | 可选观察组件；非导出/claim/写入资格 |
| S2-B | 源码976a9cf；Linux三项纯测试、八项Root文件测试；161份闭合原件 | 新鲜私有日志字节存储；非业务历史/正向效果证明 |

A1 Linux源码包摘要：`d2fb225778101d81c515898de2c2cbfde25ebe8ac9438253fd24139582313567`；实际PG结果摘要：`84adfa2bd3a651f2022b16c26c56e9579397406c2fb27b5a78d1108db9f23b98`。

S2-B Linux源码包摘要：`40b65a81bed3817513aea58672a94bdc7c8715d4b94adb57077774395bd55007`。两个组件分别按冻结源码测试；当前公开快照还包含此前0df5128单文件测试夹具修订，不能因此称整个公开HEAD已完成同源全回归。

Root文件测试仅在新无网络容器内运行，测试镜像只读、capabilities全禁，私有/run为临时文件系统，只读挂载一份实际测量ELF。八项测试通过同一个密封内存FD执行。执行器对旧容器、网络、卷的完整观测投影进行校验，允许基线已存在且保持同一指纹的旧非健康检查执行进程；不保证旧进程内部静止或观测间无变化。仅自然成功且完成校验后按精确ID删除本次容器；此前失败批次保留。

A1中间Docker inspect正文和含合成凭据的SQL未导出原文；相关状态依据固定驱动的实际断言和已脱敏进程记录，不冒称另做了独立原始SQL或系统调用追踪。
