# P0-C4 源端维护准入

更新：2026-10-03。父 Task3 的 live source admission 切片已在全新批次实际通过并限定接受 **5/5** 个 PG18 门。它约束遵守源端协议的并发尝试；父 Task3、完整备份/恢复、`CompleteBackup`、C4 和生产仍未验收。

## 问题与实现范围

此前，另一维护尝试可能在发现其他会话之前先 REVOKE runtime CONNECT；按 attempt 或控制根隔离的日志也不能统一排斥同一数据库上的并发尝试。源端 SQL 和 catalog 若重新借用连接池，还可能让 `max=1` 池等待自己占用的连接，或把另一个管理 backend 算入排空检查。

源端现在用私有、不可 Clone 的 `SourceAdmission` 拥有一个 `PoolConnection<Postgres>`。公开 `prepare_source_backup`、`force_close_release_ready` 和 `AdminAssetCatalog::plan_assets` 的签名保持不变；新增的借用 helper 仅供内部组合。

| 约束 | 当前行为 |
| --- | --- |
| 连接所有权 | acquire 后立即设置 `close_on_drop()`，再执行任何可失败的 SQL/await；正常完成可消费持有者调用 `close().await`。丢弃、错误或取消导致持有者释放时关闭连接，不把带锁会话归还池，不在 Drop 中 GRANT 或删除日志。 |
| 同会话认证 | 在这一个连接上核对 current/session/authenticated learning_admin、数据库 owner，以及当前数据库与预期规范 C4 库名一致；通过后才接受准入。 |
| 固定数据库域锁 | 非阻塞 `pg_catalog.pg_try_advisory_lock` 使用两个固定正 i32：`0x4b57_4334`、`0x5352_4345`，为当前数据库的 session advisory lock。attempt UUID、控制根、摘要和进程不参与锁键。 |
| 拒绝顺序 | 两个源端入口均在扫描恢复日志或修改 ACL 前取得同一准入；busy/认证错误发生在 journal/source/pin/recovery 创建及 ACL 变更之前。 |
| 持有范围 | 同一个 guard 覆盖源端 SQL、catalog、journal/fsync、dump、seal/pin、ReleaseReady/GRANT/Released 与失败补偿。敏感释放和补偿 helper 必须借用 `&mut SourceAdmission`；全流程不包成一个事务，REVOKE 仍按 autocommit 提交。 |
| catalog 与排空 | catalog 在已准入连接上执行短 repeatable-read/read-only 事务，保留 ready 全行、确定排序、limit+1 超限拒绝与 commit。短事务提交不释放 session lock；排空只排除本查询 backend，其他真实会话/事务、当前库预备事务和角色谓词保持约束。 |

这是**会话存活期间、同一数据库内、遵守协议的尝试互斥**：换 attempt 或控制根不能绕过存活锁，其他数据库可独立准入。会话消失后锁会释放；它没有建立固定 source control-root 的持久身份 pin，也不能证明崩溃后换根仍被阻断。

## 实际 RED 与 GREEN

旧实现的全新 RED 批次 `da07fe4e-8731-4708-b8cc-7a4bdf701b1e` 实际执行 busy recovery 门，退出 101、0 通过/1 失败/0 忽略；失败定位于 runtime CONNECT ACL 保存断言，无 fixture-query failure。旧结果 SHA-256 为 `0a6319ae6715c4cfc35ff458232c71aff08f729e3ddd14cb096b0e7b6da5cebf`，不是编译或测试列举 RED。

GREEN 批次 `06a0882f-5c7a-4fa5-ab86-252ece7732fa` 使用基于下列 HEAD 的 **未提交 `working-tree-green` 冻结快照**。该次测试运行时尚未产生本修订提交，base 仅表示测试包基线；本次接受绑定 ZIP 和五个 Rust 原始字节 pins。控制端已读实际规范结果、五份精确测试日志并独立核对资源；独立限定返回审查的 SpecCompliance、Quality 与返回接受均为 Approved，Critical/Important/Minor 均为 0，只接受本冻结 live-admission 切片五门 5/5。返回审查基于控制端的新观察和本地证据，独立重建规范 result，并核对 ZIP 的 463 个源文件、manifest、source aggregate 与当前 Rust pins。

| 运行输入或输出 | SHA-256 / 身份 |
| --- | --- |
| 测试包 base HEAD | `0adab2e340c0980c120364a1214ebb53277e4405` |
| ZIP | `90086c9413d2d4d46bba689267ad6ad3a6aa67a5ea5e2c334e18115f656c543f` |
| manifest | `e7f642b7717e8c1227d639bd0e4d2b48876f0def0522c7a3055a98b5a8c65926` |
| driver | `86c59ad4f29eb35ce2cbf6af3fec50fabe981a56a27bfbf157279ba389fe2920` |
| 规范 result | `98b0883bef44abe10259006e34b5481e322fac38511421b35c37d56afd74cd98` |
| 限定返回审查报告 | `a917823c88cb7a265f247da92c3b4146ad314fb7f8b0254e2587f031d531671e` |
| 实际测试二进制 | `7c6cff0b68a3db71acb7e76c302027a424c46952edc43c11c565a4f731e3bed3` |
| 运行输入源码摘要，前后相同 | `b87289f3be941373ba6626a04f16879debd1766209eb0dcfb697baa5488656cc` |

以下五项每项使用一个全新隔离 PG18 项目、卷和私有测试根，**各 exit 0、1 通过、0 失败、0 忽略、stderr 0 字节**。运行身份为普通 hans，访问的是自己拥有的新夹具，不是直接读取宿主 root 私有旧证据，也不证明生产源根可信。

| 精确测试（均在 `source::admission_tests`） | 实际行为与边界 |
| --- | --- |
| `busy_source_recovery_preserves_acl_and_journal` | 独立连接持锁时，公共 recovery 拒绝，ACL 和全部日志字节/目录项保持不变，无新恢复文件。 |
| `same_database_attempts_share_admission_and_other_database_is_independent` | 不同 root/attempt 在同库精确 busy 且无副作用；另一新数据库可准入；owner close 后可再次准入。只证明 live 互斥。 |
| `single_connection_catalog_and_drain_share_admitted_backend` | `max=1/min=1` 下 catalog/排空使用同一 PID，无自增其他会话；真实额外事务仍计为 1 并拒绝，关闭后恢复 0。 |
| `dropping_admission_closes_backend_without_reopening_gate` | Drop 后旧 backend 在有界观察中消失，后继为新 PID；ACL 保持关闭，未完成日志保留并在固定根继续拒绝。 |
| `release_and_compensation_keep_admission_until_owner_closes` | 正常 release 与 Released 写入碰撞后的补偿 REVOKE 都持续排斥竞争者，直到 owner close；前序摘要为 SYNTHETIC，没有真实 dump/full pin。 |

Linux 固定离线 builder 的格式、严格 package all-targets Clippy 和 cfg 编译已完成，builder exit 0。库测试为 **149 通过/0 失败/26 忽略**，`maintenance_contract` 为 **4 通过/0 失败/0 忽略**，`maintenance_journal` 为 **1 通过/0 失败/0 忽略**；这些构建检查不增加实际 PG 五门计数。构建器已移除，五个本批精确 PG ID 全部停止，卷及内部空网络保留，pending 不存在，运行输入源码与二进制核验不变。旧失败批次和证据保留，不重放。

冻结 Rust 原始字节如下；五文件均为 LF、CR=0。

| 相对源码路径 | 字节数 | SHA-256 |
| --- | ---: | --- |
| `crates/learning-backup/src/catalog.rs` | 2719 | `8e1905fa9596bc0e7fcf48b178509d4a6c787167bc66f8cbb3ce25d05ea9460b` |
| `crates/learning-backup/src/source.rs` | 53056 | `e7fefd86e4a5ae23392495410b6a29849b5a077ef714ada6e28d5b1ab9b5f858` |
| `crates/learning-backup/src/source/admission.rs` | 1710 | `f848e8173acb813e5711b379270ba1fa8ecc7775778c0b9052219474ba2d2f3f` |
| `crates/learning-backup/src/source/admission_tests.rs` | 20842 | `ef33fb306c8a8d548afcc73771666c059c2944f80fa6374df49c3a57fc53f689` |
| `crates/learning-backup/tests/maintenance_pg.rs` | 11839 | `cbca901a9146d2cb9302ccd40a4330dd85b0124d2e48d33528f5d62de8ee3677` |

## 父任务仍待完成

五门均未调用 `pg_dump`；public prepare 的 SQL/catalog/journal/dump/seal/pin 持有链由完整静态审查支持，本轮未执行该入口的完整源端捕获。原 `c9f1cdaf` 维护三门的 3/3 属于旧提交 `07eaf416` 的历史专项；旧测试借用签名/所有权适配经静态审查未发现需要追加定向回归的实质问题，但没有在本候选重跑。不能将旧三门与当前五门写成新版本 8/8。

父 Task3 继续保留：可信固定 source control-root 的独立持久身份 pin；广义 finish/abandon 和中断捕获/封存恢复；真实源 dump、全部 ready assets/index、保留保护、独立目标全量校验与完成收据的组合验收。独立存储故障域按用户安排延后。全部 in-flight cancellation、SIGKILL、网络/掉电故障以及绕过协议的特权写入者不由此五门证明；Windows no-follow、性能和生产部署也未验收。

父 Task3 三项复选框、Task4 干净恢复与 Task5 整关故障注入继续开放，未签发 `CompleteBackup`。历史专项见[维护闸补充验证](p0c4-maintenance-gates.md)，完整记录见[C4 验证记录](p0c4-verification.md)，后续任务见[父施工单](superpowers/plans/2026-09-28-p0c4-backup-recovery.md)。
