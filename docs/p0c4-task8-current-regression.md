# P0-C4 Task8 · 当前完整回归与四套旧版升级

更新：2026-10-26。状态：**实施中，Task8未完成**。当前十门与四升级已限定独审通过；源码差异独审也已采用；规定专项原件和最终整组采用仍未闭合。

本页完整十门已接受源码：`a5eb28d0ffc36d6434576fbd083eb20f07566f41`；当前单案测试修复候选：`679b11c3f280546f8d6c20ab15cf7ef59fae3c1a`，格式/严格Clippy已过，专项五阶段外层已返回，原件待独核；父源码：`638cb0f630fd4360c9d8b3df6f3f394fe8547e93`。本轮当前结果、历史采用和未执行项分别记账。

## 最新原件独核检查点 · 2026-10-26

9项A5原件完整返回，history-gap规定拒绝场景已独审并由Root限定采用，fresh专项inner接受现为1案。4,333个物理原件、实际libtest 1通过/0失败/0忽略、64次namespace audit、31个native helper清理和双PG停机已关联核对；3,815字节profile以原SHA补齐，旧4,332叶逐字及FD9元数据相同。Root再读4,436个引用、75,696,718字节。原件独审SHA：`9de44567fcb24009cd62b51ccd3887381faa4378e20caf57c9e1d850f3c1f828`；Root限定采用：`d153fc9727fc8becd37b1d2febe3ff3cd851b0beb78a33d3749236a602b415a7`。

这是history-gap的拒绝边界证明，不计正向import/recovery/effect信用。ELF未另行全文下载，issuer/inventory未各自FD9导出，相关事实来自实际编译、物化、driver与独立RPC原件；export-helper完整inspect与独立monotonic收据的旧层级限制保留。

第10项元数据读取后的全量资源投影新增一个其它项目同步容器，所有原有container、network和volume条目相同，读取helper已精确移除、双PG停止。本次HOLD保留；不停止、删除或排除其它项目，不刷新基线。新候选保持完整等值谓词和原外层期限，只作有界重新观察；已闭合9行固定继承，余16项用新读取UUID，业务不重放。原第4项读取HOLD、旧profile缺口和本轮第10项HOLD均不改写为通过。

679注册表单案的五阶段外层及格式/严格Clippy已核，inner仍待核。其余21业务操作/28正文、剩余原件独核与Task8整组采用尚未完成；Task9、独立存储、正常CompleteBackup/RecoveredInstance、C4整关、生产和main合并仍开放。

## 679测试修复与当前补验

独立审查确认A5失败来自测试自身保留的快照锁：`ProtectionSnapshot`克隆持有同一个flock文件描述符，释放原lease不释放快照。产品返回`registry busy`符合预期。679仅增加九行测试：断言快照仍持锁，再显式释放snapshot；原refresh、finish、字节和pin断言保留。611叶中的610叶、22控制器、15迁移及Cargo均未变。

### 新源码的Linux质量门已通过

`cargo fmt --all --check`与`cargo clippy --locked --offline --workspace --all-targets --all-features -- -D warnings`均自然退出0，无超时且双流EOF。35份原始记录共4,582,110字节已回读核对；611叶源码前后相同，固定构建容器已按精确ID移除，完整保护资源投影恢复原基线。本轮不执行用例、迁移或PG，不计专项正文通过。结果SHA-256：`f1909e1a1791f68167c6a253d7fcbc5a204ff05689ceac20b6c4a16a9a711fe0`；Root限定采用：`e6512916647bc4f24b71e11df69d0ae5d31d821bf994fb8de1b9e5dd3aa47fa1`。

### 单案正文与后置资源保护分别记账

679此前单案正文回执为1通过／0失败／0忽略，但后置保护因外部同步容器数量不符而HOLD，不能计为单案通过。七份完整观察原件显示最后一帧原有资源均已恢复基线，外部同步容器仍在；旧资源与HOLD保留。V5仅为该已确认形状增加有界再观察，仍须原完整保护谓词满足，不过滤、停止外部项目或刷新基线。

新V5隔离案已返回prepare/install/plan/run/verify五阶段外层回执，测试回执1通过／0失败，资源保护满足且五个胶囊精确清理；137,635,671字节阶段原件已重核，inner原件仍待独核。A5原件已完整收取9项，固定profile叶的读取缺口已补齐；history-gap已独审并经Root限定采用，fresh专项inner接受1案。第10项元数据读取后出现其它项目新同步容器，完整保护拒绝，旧HOLD保留；已有9项不重读，剩余16项改用新批次有界观察。679注册表单案仍待inner独核，其余21业务操作／28正文及Task8整组采用仍开放。

V5本地独核已重跑17项检查、47份精确逆补及94个真实采样入口，并重新解码七份完整失败观察原件。它不改变产品、控制器、业务期限、资源基线、接受谓词或一次性执行约束；等待截止和未知资源仍HOLD。本地机制审查不提供Linux用例信用。

A5完整回归十门与四套真实旧版升级、RootFS33及源码差异保持已有限定采用。25项A5主正文外层回执已返回，fresh专项inner已限定接受1案（history-gap）。679九行测试修复的Linux格式与严格Clippy已通过，35份原始记录已回读核对；不把A5结果重标为679执行。

后续先完成修复案外层结算与原始证据独核，再按已审查明确子集收取25项A5原件，随后补余21次操作／28正文。原失败不重放、不改写，所有来源分别记账。Task8整组、Task9、独立存储、CompleteBackup、RecoveredInstance、公开恢复、C4整关、生产及main合并仍开放。

候选ZIP SHA-256：`e8f0ffe17b2e9db22d545498944562106756b7afa01767e3a789860ac8212221`；manifest：`3d61680038016b8e46140435151e0dfe7f26f6f0260d86ef1c60048faf0c723b`。九行测试独审：`10430fc7451d2dbac973dc6769da937ba47ac6aeec84e274a89182fd570e5399`；V5 Root机制审查：`3715848854e4a028ce08ddf1cfd7ee8194c6021b37bf2b1f4b5c3b125bd9a9f7`。私有原件不公开凭据或服务器路径。

## 原件收取当前进度与补齐项

snapshot-history-gap、recovery-derived、recovery-receipt-failure三项已收取；10,311个原导出文件共29,381,394字节逐份重核大小、SHA、完整清单和源端before/after元数据，另已读取各自八份导出操作原件。物理传输通过不等于语义独审接受。

第四项recovery-effects的只读子进程自然退出0，空stderr，完整协议末条可见；父读取器在随后的资源保护环节以AssertionError停止，helper退出但未移除，完整收取不授通过。既有日志未保存该失败时的完整after投影，因此不确定具体变化的资源，不将外部同步容器当作本案已确定根因。旧helper、HOLD与原件保留。

独立审查确认history-gap实际精确libtest为1/0/0，gap proof已找到原字节；同时发现profile原字节未包含在导出范围内。补齐候选仅新增由绑定batch/case构造的full-profile/profile.json固定叶，最大16KiB，并与driver的精确引用哈希关联；不跟随未核路径，不以布尔值替代原件。读取器候选保留总360秒、子进程300秒和完整资源等式，补留完整投影及有限再观察。候选仅通过本地检查，待独审和新只读批次，不执行业务重放。

Task8 fresh专项inner已限定接受1案（history-gap）；其余21业务操作／28正文、新原件补齐与独核、最终整组采用仍未完成。

## 历史A5首案失败与保留边界

原A5首案的driver与raw-index已读取，精确consumer进程自然退出101；libtest为0通过／1失败／0忽略，`finish_source_backup`返回`Invalid("registry busy")`，stderr为空。后置完整资源保护另有1800秒截止。正文错误与外围观察是两个分别核实的失败，外层默认`test_exit=-1`不代表正文未执行。原UUID、资源、HOLD和证据保持，不自动重放。

## a5完整回归十门实际返回

最终本批十门全部自然退出0，源码字节和完整资源投影保持一致，PG按精确容器ID停机留卷。192份原件共50,690,216字节已逐份与远端索引核对；结果SHA-256为`22c499e1ecb27060fa9475cd3af06f8ebaab9dd0ee336dfe1f79cf4835d6c85c`。这192份原始证据的独立限定复审已通过；规定专项与整分支最终采用仍待完成。

| 当前a5实际检查 | 返回 |
|---|---|
| Rust workspace | 108组；1052通过、0失败、169忽略 |
| 四套冻结旧版升级 | p0a、b1、b3-schema、b2真实生成与升级；迁移checksum核验退出0 |
| 格式／严格Clippy | workspace、all-targets、all-features、-D warnings均退出0 |
| 普通用户Python | 906项：824通过、82跳过 |
| Root Python | 906项：903通过、3跳过；含专用私有FS正负例 |
| deploy Python | 21通过、0跳过 |

普通与Root两组去重为904个实际通过的不同Linux测试正文。两个共同跳过项要求私有历史frame；Root另跳过的普通用户权限负例在普通用户组已通过。两份历史frame分别经固定大小/SHA核验后，用当前a5纯parser在Windows只读执行2/2通过，单独保留其平台和历史输入范围，不改写原Linux跳过。补齐Root测试环境所需`FOWNER`仅用于夹具在拒绝错误owner后恢复文件权限；产品授权、测试断言和普通用户cap均未改变。

完整回归通过不包括169项ignored正文。清单已按原始listing逐行处置：规定主专项60项中，54项安排当前新资源执行，6项Attention仅采用Source36实证及其业务代码闭包未变的显式桥接。其它FS/OS、兼容入口、可选EXPLAIN、父进程专用子入口与额外PG正文分别披露；未执行入口不授current pass，Task9排除为0。

## 当前a5 Root文件系统／OS补验已独审采用

另一个全新a5组件批次在Root身份下精确运行33个被workspace忽略的FS/OS正文，逐项1通过／0失败／0忽略；120份原件共4,256,229字节已全部回读并经独立复审，结果SHA-256为`14c3116f0d89c093b745240fa4c1cdaff358af7a0c10ab995e207f927c413865`。源码/七份ELF、UID0及四项必要cap、只读10 bind与3 tmpfs、4CPU／8GiB／pids512、精确容器清理和清理后完整资源投影等值均逐项关联。ELF没有另行下载，依据producer实际FD前后哈希观察；rm/ps各原reply未另存的限制也保留。

这33项属于当前a5的额外限定补验，与历史638组件分开，不改写workspace原169 ignored数，不计为PG专项或Task8整关完成。

## 规定专项：旧失败保留，新资源串行补验

恢复基础/快照效果16项中，`snapshot-history-gap`、`recovery-derived`、`recovery-receipt-failure`、`recovery-effects`、`snapshot-p6`五项已完成五阶段并返回公开runner的1/0/0回执。suite内原始stdout、driver结果及raw_index尚未导出独核，仍不计专项接受。

第6项`recovery-leases`执行结束后，外围保护谓词以`PROTECTED_RESOURCE_CHANGED_OR_UNKNOWN`拒绝，原run-settled未生成。只读原件对比确认，原有资源唯一摘要变化为`spicylabtag_default`网络；同一观察内新增外部同步容器。后续只读投影中外部同步容器已消失，原有C/N/V逐项恢复原基线。本案两PG均Running=false、Pid=0、无ExecIDs，控制capsule当前不存在，卷保留。以上定位保护检查的差异，不推断该网络内部具体字段或外部任务的因果关系；内部公开回执虽报1/0/0，仍不能覆盖外围HOLD。

新增的被动观察窄修经代码独审及11项本地机制检查后，在另一全新批次的前置sampler暴露`time`模块缺失；它在创建PG和执行正文之前以0.5秒退出1，case_id为空，0项接受。该记录另留HOLD。最小导入修复已独审，94条真实生成脚本命名空间检查与11项本地等待机制检查均通过；业务源码与既有十门/33组件结论未变。

原失败批次与后续自动停止队列均保留，未在旧资源重放。已采用独审后的外围有界只读等待：固定原基线与严格谓词、记录全部完整观察，到期不接受，不排除或停止外部项目，不重放业务正文。新UUID与新资源仅补余11项，原前5不重复执行。新批次已返回租约、重试上限、P1、P2、P3-live、P3-expired六项五阶段成功回执。P4在plan前600秒只读观察内始终看到现有同步任务，按截止条件退出；case_id为空，只有prepare/install两阶段，未创建本案PG或执行正文。九份完整观察和deadline原件已回读，两个控制capsule确认为当前不存在；其HOLD以及后续队列、core导出等待、八操作原件等待的停止记录均保留。只延长外围被动观察的新配方已独审：等待上限1800秒，stage sampler外围1890秒，普通/运行阶段外围4200/11400秒；原调用参数600/7800与业务正文180/7500秒、原资源基线、拒绝谓词、单次业务调用、应用源码及所有文件/字节预算均保持。原600秒11项机制检查、新1800秒11项检查、8个区分边界子例、47套代码逆补和94条实际生成脚本命名空间检查均通过。更长Root观察和更多完整帧仍可能触及原8192文件/32MiB单件/512MiB总预算并HOLD，不截断、不放宽预算。本轮全新5项已完成全部五阶段并逐份核对stage原件，回执均为1/0/0；与原前5、后续6合为当前a5恢复/快照效果16项分别返回。旧失败批次不重放、已返回11项不重复；旧HOLD均保留。完整恢复9案已闭合外层；余22操作首案失败后串行队列已HOLD。47次操作／54个正文的有限原件收取、八份既有操作记录回读及独审清单已分别限定独审通过；收取只在整个新执行队列闭合后开始，原始正文/效果/终态仍待独核，当前主专项最终接受数仍为0。

独立十门审查的证据边界亦保留：PG完整inspect/image Env没有另存原raw，依赖已固定配方、调用摘要及全资源投影；rm回复保存哈希，源码末态由执行守卫与期望聚合值核对，并非另外下载611叶末态。两项私有frame的Linux跳过不改写。Task8、Task9与C4整关保持开放。

## Source36→a5完整差异独审已采用

两份冻结包各611个源码叶逐字核对：604相同、7个测试文件改变；没有删除原测试方法，没有改变产品授权、业务预算、22项控制器、15项迁移或Cargo配置。当前611个实际Git blob也与a5包逐字相同。Windows工作树中447叶物理字节相同、164叶仅checkout换行不同，不将Git clean误写为全部工作树物理字节等包。

独立审查还回读了Task6和Task7既有采用及相关peer索引，保留各自来源和私有单机范围，不把旧16/16和6/6改记为新a5执行。Root重新核对676条原件引用并采用该源码差异审查；独审索引SHA-256为`4010a6c5d78a2a1b663e57d9d29cf4d9565e6006bfe1ec59d79ceb80dee38de9`。该结论覆盖Source36→a5完整七叶差异，未新增C3→a5全库安全扫描结论；规定54项fresh的inner原件及Task8最终采用仍开放。

## 638历史候选实际返回

638候选在全新Linux/PG18批次的常规workspace为**1052通过、0失败、169忽略**，108组汇总均与原始清单相连。四套旧程序夹具`p0a / b1 / b3-schema / b2`由冻结程序`b60056ba08892d69efdfa4778c976154a421e17d`生成；真实升级及迁移checksum验证退出0。格式、全工作区/所有target/所有feature严格Clippy均退出0。

该批在普通用户Python全发现运行901项后，以3失败、19错误、82跳过退出1，Root与deploy门因此未执行。失败原件、源码未变和全量保护资源核验、精确PG停机留卷均保留。Root回读162份原件，所有字节数和SHA-256与远端索引相符；结果摘要为`81dfa44494548592bdde1371aeb52512d60d2dd2947a6979d516bc12a20b2c9e`。这不是Task8整组通过。

另一个全新638组件批次实际通过33项Root FS/OS正文，每项精确1通过/0失败/0忽略，覆盖Root journal、custody、固定副本、candidate/supervisor、Unix socket和UID65532子进程边界。120份原件已全部哈希回读；结果为`0c459881ae0dd88131e859d5106e9f187f06f8e1dd02de1e7dd4f2aba06cbf9a`。固定镜像、实际UID0及四项必要cap、独立tmpfs、只读ELF、精确容器清理和清理后全量资源等值另由子代理独核。旧组件批次第29项因缺少broker可执行文件挂载失败，原失败保留；33项新通过不来自改写旧结果，也不代表PG专项或a5新构建执行。

## Python修复与新候选

仅修改六个`scripts/test_*.py`测试文件：临时文件移到私有临时目录；纯mock envelope不再访问宿主`/root/case`，并增加pending存在必须拒绝的负例；共享actor允许的两个固定case与测试断言对齐；打包测试使用临时真实Git仓库，另验脏工作区及未跟踪文件不能进入包。原生产控制器、Rust产品代码和迁移未变。六文件静态独审通过，当前a5新批次已实际完成普通与Root Linux检查，结果与跳过边界见上方十门表。

另已在新隔离诊断容器复现shell测试的`127 / psql: Permission denied`：默认tmpfs禁止执行临时standin。新配方只对本批私有`/tmp`显式启用exec，保留网络、只读源码、cap和全部资源守卫。此后a5候选已在新UUID、新PG18卷及新子网完成十门返回；规定专项和后验仍待完成，失败数据库未复用。

## 本轮发现与修复

Source36的新隔离完整workspace实际运行到`snapshot_jobs`，该二进制19项中10通过、9失败。管理交付测试自身通过，但它关闭共享测试库的runtime CONNECT后未恢复，后续9项都在建立新runtime连接时收到PostgreSQL `42501 / CONNECT privilege`拒绝。该轮退出101，后门停止，Root已逐字哈希回读78份原件，子代理另复读了关键失败原始日志与窄补，源与保护资源保持，PG停止留卷。结果摘要SHA-256：`7b223b7a1794d9ad0529d6f99bd2a9e6a42d6704b1a50e7ec92698206cc26482`。

修复仅为`crates/learning-db/tests/snapshot_jobs.rs`的11行测试收尾：全部授权、只读、锁等待和撤权断言之后，恢复该隔离库原有的runtime CONNECT，再实际建立并关闭一个新runtime连接。没有修改产品授权、旧断言、迁移或控制器。静态独审与Windows格式检查通过；当前a5新批次workspace已1052通过/0失败，原Source36失败证据仍保留。

此前单独保留的准备失败包括：归档内manifest名称识别、PG初始化脚本读取权限、Docker内建网络IPAM空值，以及资产目录专项缺少UUID独立空库。它们均未改写为通过。新638批次在进入workspace正文前，因其它项目的同步容器出现、spicylabtag_default网络摘要变化而被完整资源守卫拒绝；该轮结果SHA-256为`b2018547f00df5dc981f99ec19b9eff9502b31085cc5db642922721e3dae2eea`。尚无before原始网络inspect，不把摘要差异猜作某个内部字段的确定原因。旧失败不重放、不停止或排除其它项目；此后已独审并采用有时限的只读静止窗口等待，仍要求所有保护资源精确匹配原基线才执行测试。

当前薄运行配方已修正相应环境：初始化脚本由同SHA只读副本挂载；固定Cargo adapter仅把`catalog_pg`实际测试路由到专用UUID空库，其余二进制原样执行，listing不当作测试正文。

## 新候选与验证链

| 项目 | 当前固定事实 |
|---|---|
| 源码包 | 611个精确Git文件；相对638仅六个Python测试文件改变 |
| ZIP SHA-256 | `f740f7ab7d1e62328e76f614d664912ef345b34d06cc97a492defd351df64a97` |
| Manifest SHA-256 | `f45498c18ceceac1154b685b334ba6178c2b78c215d8c200fa24aa29594c4d19` |
| 迁移／控制器 | 15项迁移与22项控制器原字节不变 |
| 完整回归 | a5新隔离十门全部实际通过，192份原件已限定独审采用；专项与整分支终审仍开放 |
| 四套旧版升级 | a5四套冻结夹具实际升级、checksum核验均通过 |
| ignored | 当前a5列举1221正文、workspace实际1052通过／169 ignored；另有当前a5 Root组件33正文已独审。规定54 fresh与6 Attention历史桥单列，不能把列举当执行 |
| 独立审查 | 测试窄补、冻结包、完整十门/四升级与当前Root FS33已限定独审；规定专项inner原件与整分支最终审查待完成 |

实际执行顺序为：11个命名库空库检查 → 四套冻结旧夹具生成 → 实际listing → `cargo test --offline --locked --workspace -- --test-threads=1` → `legacy-upgrades`逐项迁移checksum → 格式 → `--workspace --all-targets --all-features`严格Clippy → 普通Linux Python全发现 → Root身份与专用FS目录Python全发现 → deploy Python。每门原始stdout、stderr、进程退出、实际容器inspect和资源前后投影单独留存；失败停止后门，超时保持HOLD，不重放失败批次。

## 当前仍未关闭

- 169项ignored中的必需current专项处置：当前主专项54 fresh逐项补验及6 Attention显式历史桥，另有当前Root FS33已独审。完整workspace、四升级、严格门和Python不能替代未闭合的专项。
- 原始证据、当前构建/源码/二进制、迁移和角色权限的完整连接核对，以及规定专项原件的独立语义审查与最终整组采用。
- Task9独立故障域、托管密钥及真实首签/新发布权限集成；CompleteBackup、RecoveredInstance、公开恢复、C4整关与生产仍OPEN。

Task6的2e5026d私有16/16、Task7的Source36私有6/6保留各自原采用范围，不改写为638/a5候选的新执行。候选分支同步不表示main合并或部署。

本轮五项返回索引SHA-256：`a82e4d08beda8eceebcc256b7fccb0a0445e2749e17238c9c02c14c14b021ef1`；五阶段原件核对索引：`41741dff546b4042009e5dab01628869ba4f9107e8ae3a85ac993a806cf81fce`。这些是公开runner回执与stage字节核对，不能替代inner原始证据独核。

## 完整恢复九场景回执与外层原件限定采用

当前a5的正向导入、提交前断开、取消、提交结果未知、错误端点、损坏输入、错误角色、脏目标及原件损坏九项已分别闭合全部五阶段，逐项回执为1/0/0。45阶段、143个完整被动观察帧经三个互斥三案集合独审，Root以完整九案stage原件索引关联限定采用；未重读或覆写旧失败、不从晚到成功推断旧失败根因。

九案返回SHA-256：`f0ed6e841103b9a8291930903612cc73408a365dc0b666ea2398f39c1706323a`；stage逐份核对索引：`c084c571e6b1f7f7a343047135188fb789e3b0e0ed828d68f1b85538b6dea160`；互斥peer组合摘要：`f352269ca0aab600132798ed865d15551d8ebcb2c2f67025ac54491ca9982805`；Root限定采用：`1a5ff57d0bcd41cec67c9589d74d3e6164258084dc5fc12b1d6c2e57ede65113`。

与恢复/快照效果16项合为25个主正文的外层回执返回，不等于25项inner接受。原余22次操作首案失败的旧队列HOLD保留；679新修复案已返回五阶段，其余21操作／28正文待补，25项A5原件正在收取；主专项fresh限定接受1案（history-gap），原始driver/raw_index/libtest/proof/终态与Task8整组审查待完成。私有inspect仅留digest和非独立外层OS计时的限制继续保留；Task8/Task9/C4整关、CompleteBackup、RecoveredInstance、公开恢复和生产仍开放。
