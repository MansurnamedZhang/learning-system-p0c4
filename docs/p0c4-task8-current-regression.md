# P0-C4 Task8 · 当前完整回归与四套旧版升级

更新：2026-10-25。状态：**实施中，Task8未完成**。本文件记录当前运行事实；新候选尚无整组通过结论。

当前冻结源码：`a5eb28d0ffc36d6434576fbd083eb20f07566f41`；父源码：`638cb0f630fd4360c9d8b3df6f3f394fe8547e93`。本轮当前结果、历史采用和未执行项分别记账。

## 最新实际返回

638候选在全新Linux/PG18批次的常规workspace为**1052通过、0失败、169忽略**，108组汇总均与原始清单相连。四套旧程序夹具`p0a / b1 / b3-schema / b2`由冻结程序`b60056ba08892d69efdfa4778c976154a421e17d`生成；真实升级及迁移checksum验证退出0。格式、全工作区/所有target/所有feature严格Clippy均退出0。

该批在普通用户Python全发现运行901项后，以3失败、19错误、82跳过退出1，Root与deploy门因此未执行。失败原件、源码未变和全量保护资源核验、精确PG停机留卷均保留。Root回读162份原件，所有字节数和SHA-256与远端索引相符；结果摘要为`81dfa44494548592bdde1371aeb52512d60d2dd2947a6979d516bc12a20b2c9e`。这不是Task8整组通过。

另一个全新638组件批次实际通过33项Root FS/OS正文，每项精确1通过/0失败/0忽略，覆盖Root journal、custody、固定副本、candidate/supervisor、Unix socket和UID65532子进程边界。120份原件已全部哈希回读；结果为`0c459881ae0dd88131e859d5106e9f187f06f8e1dd02de1e7dd4f2aba06cbf9a`。固定镜像、实际UID0及四项必要cap、独立tmpfs、只读ELF、精确容器清理和清理后全量资源等值另由子代理独核。旧组件批次第29项因缺少broker可执行文件挂载失败，原失败保留；33项新通过不来自改写旧结果，也不代表PG专项或a5新构建执行。

## Python修复与新候选

仅修改六个`scripts/test_*.py`测试文件：临时文件移到私有临时目录；纯mock envelope不再访问宿主`/root/case`，并增加pending存在必须拒绝的负例；共享actor允许的两个固定case与测试断言对齐；打包测试使用临时真实Git仓库，另验脏工作区及未跟踪文件不能进入包。原生产控制器、Rust产品代码和迁移未变。六文件静态独审通过，最终Linux各身份检查须由新批次实际结果支持。

另已在新隔离诊断容器复现shell测试的`127 / psql: Permission denied`：默认tmpfs禁止执行临时standin。新配方只对本批私有`/tmp`显式启用exec，保留网络、只读源码、cap和全部资源守卫。a5候选正在新UUID、新PG18卷及新子网从头运行完整门，不复用失败数据库。

## 本轮发现与修复

Source36的新隔离完整workspace实际运行到`snapshot_jobs`，该二进制19项中10通过、9失败。管理交付测试自身通过，但它关闭共享测试库的runtime CONNECT后未恢复，后续9项都在建立新runtime连接时收到PostgreSQL `42501 / CONNECT privilege`拒绝。该轮退出101，后门停止，Root已逐字哈希回读78份原件，子代理另复读了关键失败原始日志与窄补，源与保护资源保持，PG停止留卷。结果摘要SHA-256：`7b223b7a1794d9ad0529d6f99bd2a9e6a42d6704b1a50e7ec92698206cc26482`。

修复仅为`crates/learning-db/tests/snapshot_jobs.rs`的11行测试收尾：全部授权、只读、锁等待和撤权断言之后，恢复该隔离库原有的runtime CONNECT，再实际建立并关闭一个新runtime连接。没有修改产品授权、旧断言、迁移或控制器。静态独审与Windows格式检查通过；运行通过须由新候选实际回执支持。

此前单独保留的准备失败包括：归档内manifest名称识别、PG初始化脚本读取权限、Docker内建网络IPAM空值，以及资产目录专项缺少UUID独立空库。它们均未改写为通过。新638批次在进入workspace正文前，因其它项目的同步容器出现、spicylabtag_default网络摘要变化而被完整资源守卫拒绝；该轮结果SHA-256为`b2018547f00df5dc981f99ec19b9eff9502b31085cc5db642922721e3dae2eea`。尚无before原始网络inspect，不把摘要差异猜作某个内部字段的确定原因。旧失败不重放、不停止或排除其它项目；正在审查有时限的只读静止窗口等待，仍要求所有保护资源精确匹配原基线才执行测试。

当前薄运行配方已修正相应环境：初始化脚本由同SHA只读副本挂载；固定Cargo adapter仅把`catalog_pg`实际测试路由到专用UUID空库，其余二进制原样执行，listing不当作测试正文。

## 新候选与验证链

| 项目 | 当前固定事实 |
|---|---|
| 源码包 | 611个精确Git文件；相对638仅六个Python测试文件改变 |
| ZIP SHA-256 | `f740f7ab7d1e62328e76f614d664912ef345b34d06cc97a492defd351df64a97` |
| Manifest SHA-256 | `f45498c18ceceac1154b685b334ba6178c2b78c215d8c200fa24aa29594c4d19` |
| 迁移／控制器 | 15项迁移与22项控制器原字节不变 |
| 完整回归 | 638实际1052通过；a5新隔离批次正在完整重验 |
| 四套旧版升级 | 638四套实际升级成功；a5新候选仍按冻结夹具重验 |
| ignored | 638实际列举1221总数／169 ignored，另有33个精确Root组件正文已实跑；其余按current专项矩阵处置，不能把列举当执行 |
| 独立审查 | 运行配方、测试窄补和新冻结包已复核；完整新结果、规定专项与整分支最终审查待完成 |

实际执行顺序为：11个命名库空库检查 → 四套冻结旧夹具生成 → 实际listing → `cargo test --offline --locked --workspace -- --test-threads=1` → `legacy-upgrades`逐项迁移checksum → 格式 → `--workspace --all-targets --all-features`严格Clippy → 普通Linux Python全发现 → Root身份与专用FS目录Python全发现 → deploy Python。每门原始stdout、stderr、进程退出、实际容器inspect和资源前后投影单独留存；失败停止后门，超时保持HOLD，不重放失败批次。

## 当前仍未关闭

- 完整workspace／四升级／严格门／Python的新候选整组成功，以及169项ignored中的必需current专项处置。
- 原始证据、当前构建/源码/二进制、迁移和角色权限的完整连接核对，以及独立整分支审查。
- Task9独立故障域、托管密钥及真实首签/新发布权限集成；CompleteBackup、RecoveredInstance、公开恢复、C4整关与生产仍OPEN。

Task6的2e5026d私有16/16、Task7的Source36私有6/6保留各自原采用范围，不改写为638候选的新执行。候选分支同步不表示main合并或部署。
