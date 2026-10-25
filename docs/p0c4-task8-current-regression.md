# P0-C4 Task8 · 当前完整回归与四套旧版升级

更新：2026-10-25。状态：**实施中，Task8未完成**。本文件记录当前运行事实；新候选尚无整组通过结论。

当前冻结源码：`638cb0f630fd4360c9d8b3df6f3f394fe8547e93`；父源码：`36c89f2e94dccdc90652f8b196372bb62fa398a5`。

## 本轮发现与修复

Source36的新隔离完整workspace实际运行到`snapshot_jobs`，该二进制19项中10通过、9失败。管理交付测试自身通过，但它关闭共享测试库的runtime CONNECT后未恢复，后续9项都在建立新runtime连接时收到PostgreSQL `42501 / CONNECT privilege`拒绝。该轮退出101，后门停止，Root已逐字哈希回读78份原件，子代理另复读了关键失败原始日志与窄补，源与保护资源保持，PG停止留卷。结果摘要SHA-256：`7b223b7a1794d9ad0529d6f99bd2a9e6a42d6704b1a50e7ec92698206cc26482`。

修复仅为`crates/learning-db/tests/snapshot_jobs.rs`的11行测试收尾：全部授权、只读、锁等待和撤权断言之后，恢复该隔离库原有的runtime CONNECT，再实际建立并关闭一个新runtime连接。没有修改产品授权、旧断言、迁移或控制器。静态独审与Windows格式检查通过；运行通过须由新候选实际回执支持。

此前单独保留的准备失败包括：归档内manifest名称识别、PG初始化脚本读取权限、Docker内建网络IPAM空值，以及资产目录专项缺少UUID独立空库。它们均未改写为通过。新638批次在进入workspace正文前，因其它项目的同步容器出现、spicylabtag_default网络摘要变化而被完整资源守卫拒绝；该轮结果SHA-256为`b2018547f00df5dc981f99ec19b9eff9502b31085cc5db642922721e3dae2eea`。尚无before原始网络inspect，不把摘要差异猜作某个内部字段的确定原因。旧失败不重放、不停止或排除其它项目；正在审查有时限的只读静止窗口等待，仍要求所有保护资源精确匹配原基线才执行测试。

当前薄运行配方已修正相应环境：初始化脚本由同SHA只读副本挂载；固定Cargo adapter仅把`catalog_pg`实际测试路由到专用UUID空库，其余二进制原样执行，listing不当作测试正文。

## 新候选与验证链

| 项目 | 当前固定事实 |
|---|---|
| 源码包 | 611个精确Git文件；仅上述测试文件相对Source36改变 |
| ZIP SHA-256 | `653078579d00da9ec84009049a307f0f5f4ac7899a0dd95a17f6f1fc3ecbd7ec` |
| Manifest SHA-256 | `9f5e7cc6f798dc88006193175d3a073dbce987d706568ddb8df829be1fec77ae` |
| 迁移／控制器 | 15项迁移与22项控制器原字节不变 |
| 完整回归 | 新隔离Linux/PG18正在执行，未宣称完整通过 |
| 四套旧版升级 | `p0a / b1 / b3-schema / b2`由冻结旧程序生成；等待新候选升级后原始checksum验证 |
| ignored | Source36实际workspace listing为1221总列举／169 ignored；这是列举，不是执行。新候选清单及每项处置另核 |
| 独立审查 | 配方、故障因果与11行窄补已静态复核；新结果与整分支最终审查尚待完成 |

实际执行顺序为：11个命名库空库检查 → 四套冻结旧夹具生成 → 实际listing → `cargo test --offline --locked --workspace -- --test-threads=1` → `legacy-upgrades`逐项迁移checksum → 格式 → `--workspace --all-targets --all-features`严格Clippy → 普通Linux Python全发现 → Root身份与专用FS目录Python全发现 → deploy Python。每门原始stdout、stderr、进程退出、实际容器inspect和资源前后投影单独留存；失败停止后门，超时保持HOLD，不重放失败批次。

## 当前仍未关闭

- 完整workspace／四升级／严格门／Python的新候选整组成功，以及169项ignored中的必需current专项处置。
- 原始证据、当前构建/源码/二进制、迁移和角色权限的完整连接核对，以及独立整分支审查。
- Task9独立故障域、托管密钥及真实首签/新发布权限集成；CompleteBackup、RecoveredInstance、公开恢复、C4整关与生产仍OPEN。

Task6的2e5026d私有16/16、Task7的Source36私有6/6保留各自原采用范围，不改写为638候选的新执行。候选分支同步不表示main合并或部署。
