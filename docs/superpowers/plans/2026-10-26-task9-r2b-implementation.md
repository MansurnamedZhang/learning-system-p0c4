# Task9 R2b：准确编码范围与验证计划（编码与限定Linux检查已批准）

2026-10-26。基线：`e94e00dec7cb6768f0c5db0db238c1a5f9f407ff`。

**状态：R2B_IMPLEMENTATION_AUTHORIZED / NOT_IMPLEMENTED / NOT_INSTALLED。**

准确范围独审已限定通过，冻结审查INDEX SHA-256 `de2d7208b9db06ad586f093e1912f6f5e0e9941a83c60200fa2afd46d926439b`。送审原稿和上一轮未通过记录保留；本次只把三件稿的状态元数据改为已审范围候选。这是审批前范围复审的限定结论；随后用户已明确批准准确编码和限定Linux检查，见[批准记录](../../task9/r2b-implementation-approval.md)。没有产品测试或现场启用许可。 用户要求继续完成Task9；此前明确采纳的仅是[时序规范](../../task9/b1-final-decision-adoption.md)。本计划列清可编码范围、代价和验证停止点。用户随后明确批准本准确代码范围与限定Linux检查；持久凭证、信任安装、源隔离与恢复仍未批准。本文件不是程序输入或机器permit；没有新产品测试或Task9验收信用。

## 1. 完成条件与分阶段授权

正式条件仍为[Task9五门](2026-10-04-p0c4-completion.md)：真实独立目标/托管与安装；真实原操作首签/新发布；正常完整恢复RA；源DB/registry/assets/pins不可访问后的全新RB恢复；适用负例、回归、原件独审后才关闭C4。不能用本计划、纯单测、已有FS6或帧20替代。

| 阶段 | 本阶段交付 | 当前权限 |
|---|---|---|
| 准确范围 | 本计划、源码影响表、固定候选profile、验证清单、独立设计审查 | 设计/只读准备可继续；不是编码批准 |
| R2b编码 | 以下完整纵切片，逐子任务实现和独审；默认关闭 | 用户已集中批准本准确范围，不再逐源码包确认 |
| R2b限定Linux检查 | 既有开发Linux主机上的全新普通用户目录/固定builder，格式、严格Clippy、编译、实际单测/非特权IO及FS拒绝检查 | 用户已与编码一并批准；不创建PG、root目录、key、trust、账号或安全配置，不接独立目标执行产品 |
| B/C部署准入 | 新私钥/信任、构建pin、registry/根、固定launcher、受限transport和完整writer闭包 | 仍须准确文件/指纹/主机/变更/回退与单独批准 |
| D1/D2实际整链 | 真首发、RA及源不可访问的RB；原件、拒绝、完整恢复校验 | 固定新资源/预算后另批；不碰旧项目或生产 |

没有代码前先定合同与验证计划；代码后才取得相应真实运行证明；启用/部署/正式接受前必须闭合规定证明并取得执行许可。不得形成“未写的新实现先通过实际测试”的依赖循环。

## 2. 已核源码事实与操作起点

两组只读勘察均基于e94，未运行测试、未用凭据或连接S/D：

- 源侧30件源码/规范完整摘要与行号，INDEX SHA-256 `23dad8a42405149a71372950a8accfe863258f068f1a531c9b0090869104cfaa`。
- 目的侧21件canonical源码/规范，INDEX SHA-256 `c1c1c92742c32e9446cb318fe7449fa1bac27b4378233e1443fe24c6484df271`。

`source.rs:424–439`在retained/Released后关闭capture会话，只返回pin；旧`complete.rs:365–393`本来也是在之后首次取得publication的lease并发现当前保护。故新S指**D尝试开始前首次建立的publication actor**，不是让已结束的capture权限复活。它自己取得真实`source::SourceAdmission::try_acquire`、原PG advisory lock、首次publication registry lease/预算、当前EnrolledSource/ProtectionSnapshot和包/副证明句柄，并持续持有。原`SourceLocalPin`不是权限。

固定source runner可以先调用现有`prepare_source_backup_with_snapshot_effects`完成一次fresh capture，再建立首次S publication actor；不加resume/repair路径。真实SourceAdmission需再核当前PG身份及control binding，旧publisher并没有这项SQL准入，不能声称直接复用即已具备。S首次准入失败时D evidence attempt尚未开始；D开始后、S失效/HOLD后不得换S lease续做。

D首次取自己的本地lease，只需真实DestinationGroup及destination/evidence两根，不在D创建源PG/SourceGroup或打开源路径。D从十一证据步骤、outcome、snapshot-effects副证明、最终许可到receipt必须使用同一原authority/累计预算，不能重开lease接旧成功。旧`finalize_use`只串行使用、不消费，不能改名代替新状态机。

## 3. 首版明确选择与代价

1. 使用独立同级、另准入的source-fence namespace；不向旧registry七项、source control/pin/protection严格集合或公共v2添加文件/字段。
2. **首版不实现自动关闭或释放fence，也不读取磁盘close文件解除约束。正常成功后同样保留fence。** 该源registry的后续相关管理变更/新备份可能被长期阻止，跨group的generation/根变更也可能受阻。旧fence不删、不按TTL解除；人工/新进程解除留给独立管理合同及审批。这是本次编码范围需要明确接受的可用性代价，不伪称已解决常态周期备份。
3. Task9先使用全新隔离源资源，不复用旧失败或HOLD。旧构建/在途writer隔离是C现场启用门；代码单测不能冒充现场已收敛。
4. 整链RA/RB优先在D的两个先后创建的独立PG/目标卷上运行，调用原完整恢复引擎；不把远端CompleteBackup对象复制成另一台主机的本地authority，也不增加假远程文件系统。D现有2GiB内存只是点时信息：B0必须重新核容量/可用内存，RA结束并停机后才创建RB。若不足则HOLD，不能停现有MySQL/Redis、调低校验或擅自扩容。

## 4. 拟改文件与公共边界

下列名称是计划新增，不是现存接口。只在`learning-backup`内形成一个纵切片，不新增通用框架、任意RPC或签名代理。

| 文件/位置 | 准确职责与限制 |
|---|---|
| `crates/learning-backup/Cargo.toml` | 新默认关闭feature `offhost-b1`，依赖现`independent-verifier`；两个新bin明确required-features。复用ring0.17.14、sha2、serde_json、tokio、libc，不引新第三方库或改数据库迁移。 |
| `src/lib.rs`及新`src/offhost_b1/mod.rs` | 仅Linux+feature导出两个无caller root/key/trust/permit/command参数的固定installed runner：source一次capture-and-publish、destination固定stdio receiver。库与bin是不同crate，不能假称bin可直接调用pub(crate)。所有actor/permit/constructor保持私有。 |
| 新`src/bin/knowweave-c4-b1-source.rs`、`...-receiver.rs` | 只接受固定模式，不接受任意backup重试、路径、DSN、key、命令或override。ID/challenge从原actor内部生成；无已安装profile和构建pin时拒绝。旧destination CLI原样默认拒绝。 |
| 新`offhost_b1/{policy,auth,wire,supervisor}.rs` | 固定安装策略读取、source管理认证、下述typed协议/真实有界IO、自己SSH child的监督与settlement。不是把test-only帧文件直接导入产品。 |
| 新`offhost_b1/{source_actor,source_fence}.rs` | 首次S publication准入、原对象持有/再核、负fence、元数据/文件流、固定最终决定。无公开SourceAdmission构造器，不从磁盘重建成功或自动关闭fence。 |
| 新`offhost_b1/{destination_actor,finalize}.rs` | 原D首次lease、认证输入、十一阶段及副证明、同锁一次消费、内部sign+receipt、结果分账。 |
| `src/destination.rs`、`destination/attempt.rs` | 在原真实writer中抽取固定Local/AuthenticatedRemote证据输入，保留Local所有same-operation/Arc检查；新consume状态与authority/C/cancel共同排序。不得runtime调用Synthetic或新建测试专用成功constructor。 |
| `destination/snapshot_effects.rs`及必要的已有纯验证叶 | **纳入真实snapshot-effects副证明。** 用已认证、完整流核验后的受限remote view复用现parser/复制/读回；不得把D暂存目录伪装成原S的CapturedSnapshotEffectsV2。副证明失败失效原D；DReady只在基础proof和副证明都完成后产生。 |
| `src/complete.rs` | 提取具体canonical receipt写入/禁止覆盖/父同步/完整验证的内部动作；原公开publisher guards、fresh-lease拒绝及历史reader不放宽。新D消费内不调用会重锁的公开sign_statement/read_authorized。 |
| `src/registry.rs`、`protection.rs`、`source.rs`、`source/lifecycle.rs`、`source/snapshot_effects.rs` | 在会破坏源保护的管理入口、任何journal/ACL/pin/保护记录变更前接入同一source mutation gate；保留原API/旧部署拒绝。正常D-only open/restore不查询S fence；本次S只能以原持有对象有限核对自己的fence，不加ignore-id开关。 |
| 后续`tests/`与相邻cfg(test) | 对应分层验证；计划JSON只列义务，不作为产品输入。旧37矩阵与帧20保持原字节/信用，不重标为新执行。 |

`maintenance.rs`低层journal、公开seal/transfer与`learning-assets`文件API不是天然registry-aware。首版不伪称给registry加一个检查就覆盖它们，也不让assets反向依赖backup制造循环。新固定runner的调用闭包须由原mutation guard覆盖；C安装必须证明其它程序没有这些受保护根的写权限/可写mount/在途FD。任何仍可触达保护面的未受控路径均阻止启用，不用“只是低层函数”排除。任意可信root越权不在声称已防护范围。

## 5. 固定候选profile：task9-b1-limited-v1

这些值已随准确编码包批准为**实现上限，尚未运行，现场配置未批准**。较大合法v2包会被此部署profile拒绝，不更改公共格式或声称支持任意尺寸；不因超限自动调大。

| 项目 | 固定候选值/计费 |
|---|---|
| package完整字节 | 268,435,456B（256MiB），含实际manifest/index/所有包文件；发送前checked sum，实际读/写仍逐字计费 |
| 捕获源证据+snapshot-effects副证明传输 | 总100,663,296B（96MiB）；不抵扣原registry/journal累计预算 |
| 双向总wire/帧数 | 536,870,912B、8192帧，**合计双向**，含全部20B header和payload；全会话不重置 |
| 控制JSON/Data payload | 控制≤16,384B；Data payload≤65,536B；FileChunk控制≤1024B、raw body≤61,440B；CommittedResult body≤8192B |
| 文件/流 | package与两类证据合计≤512个逻辑文件；同一时刻仅1个文件流；≤64个产品自有打开FD，runtime/SSH必要句柄另列精确清单并核实际总数，不继承无关句柄；不是已测峰值 |
| 旧单文件上限 | 继续原manifest/index/snapshot64MiB、generation512KiB、普通record16KiB、released4KiB、receipt/witness8KiB/key/trust4KiB；新profile可更早拒绝，不能提高旧限制 |
| fence扫描/保留 | ≤256个operation目录，anchor≤4096B，open≤16,384B，无close；另计所有列举/metadata/重读和≤原100000 entry/512MiB累计ScanBudget，cap+1拒绝；无独立未计费scan |
| 时间 | handshake≤60s；无IO进度≤30s；自S首次准入起总deadline≤1800s；子进程收尾观察≤30s后如未settled报UNCONFIRMED并保留owner，不谎称已停机 |
| 日志 | stderr每侧≤1MiB，结构化控制诊断≤1MiB；wire stdout不混日志，不输出密钥/DSN/口令；原始敏感业务字节仅留在获准私有工件 |
| 资源 | 编译只在S既有builder；D无编译。未来D manager候选512MiB/1CPU、RA或RB PG候选384MiB/1CPU、额外256MiB余量，B0重新核实际可用资源；这些不是已观测RSS峰值或已批准配置 |

1800s指首次publication S准入后的同一操作，不是重新计时已结束capture；原capture期限/预算不改，外层获准case总期限取更早者。首次SourceAdmission/PG advisory lock可使用C单独准入的私有Unix socket与固定owner角色，不新存登录密码；该本地认证/挂载权限仍须C审批。

输入预算不等于精确RSS/物理fsync时限保证。同步FS阻塞时原owner持有全部FD/authority，计时器不能先释放上下文再声称自然完成；无法确认settlement就HOLD。预算耗尽/资源不足不清理旧失败卷、不越权停止已有服务。

## 6. typed wire、认证与结束顺序

### 6.1 帧与消息编码

保留准备帧的20B header形状作为候选：`KWB1 | version=1 | kind | direction | reserved=0 | seq:u64be | payload_len:u32be`。产品profile在首Metadata中固定`task9-b1-limited-v1`，**不是宣称旧test-only实现已是运行时合同**。各方向seq从0连续；首Metadata每方向恰一次，分别为HelloS/HelloD；之后只能Data typed消息、End空帧或Abort≤256B。每帧必须先读20B、校验所有长度/预算再读payload，短读/短写/Interrupted按固定原deadline处理。

Data payload固定为`tag:u8 | control_len:u32be | canonical_control_json | body_len:u32be | raw_body`；合计必须等于header长度。body只允许FileChunk、CommittedResult，以及DestinationReady的两份定长前缀原记录；其余为0。每tag固定字段struct、deny_unknown_fields、拒所有层重复键/重复角色、严格整数/UUID/小写digest和递归lexical canonical bytes；不先parse成会吞重复键的Value/BTreeMap。不得由调用者上传capability对象。

控制tag固定为SourceAuth、AuthAccepted、FileBegin、FileChunk、FileEnd、SourceDataEnd、DestinationReady、SourceFinalDecision、CommittedResult、SourceResultAck；Abort为单独帧且不携秘密/任意文本。控制均带schema/profile和会话绑定，tag不兼作权限。逐tag字段/签名前缀/phase表见[候选wire profile](../../task9/r2b-wire-profile.json)，该JSON也不是产品输入或已运行协议。

HelloS只携S原actor新nonce、源部署/构建/group/代际及包/捕获roster绑定；HelloD回显该nonce并给自身新nonce、原D attempt、D部署/构建/代际/两根绑定。根字段来自各自原对象；长路径或控制超限直接拒绝。S原actor与D新attempt的操作ID不可由重放消息选择。

每方向独立transcript链：初值为SHA256(固定domain+direction)，每次对该方向的准确header+payload长度和字节更新；不依赖两台主机观测到的跨方向时间顺序。SourceAuth/FinalDecision签名的unsigned canonical body包括**该消息之前**的本方向链值和最近固定对端消息链值、双nonce与完整绑定；签名完成后再把完整含signature帧计入链，避免循环hash。方向/序号/应用phase和body摘要都参与绑定。

### 6.2 认证与固定配置

S应用认证候选采用ring0.17.14 Ed25519，仅固定内部函数从原SourcePublishActor派生SourceAuth/SourceFinalDecision，不接受任意待签字节。未来生成/加载只用带公钥一致性检查的PKCS#8 v2严格API，不用unchecked解析；不在本轮生成或安装。[ring官方版本API](https://docs.rs/ring/0.17.14/ring/signature/struct.Ed25519KeyPair.html)

三个用途分别托管：S管理认证key、S→D受限SSH transport key、D既有v2备份签名key。D备份key保持现算法/statement签名，不签handshake或充当消息oracle。D结果真实性由已审SSH host trust/当前session、固定receiver启动与原D状态共同建立；v2签名单独不证明本次nonce或进程。

编译feature alone不够：角色、固定profile版本及public-key pin由获准构建输入固定，缺失即拒绝新runner；完整安装policy/实际ELF摘要由Root已审安装manifest另行核对，不能在二进制内嵌入包含自身ELF摘要的policy hash。源角色启用后源policy/anchor缺失或替换必须拒绝相关mutation，不能退回legacy“无fence”；D-only历史路径不读取S policy。固定路径在新隔离runtime namespace内为`/var/lib/knowweave-c4/b1/source-policy.json`、`.../destination-policy.json`、独立同级`/var/lib/knowweave-c4-source-fences`；角色/allowlist/实际binary digest/registry与fence FD anchor绑定由B/C真实issuer核发，不从CLI或wire接受。旧主机同名目录不覆盖，宿主实际路径/挂载由C准确资源单另批。 编译/安装依赖必须无环：先经B/C批准生成真实托管key并取得公钥指纹，再按代码/角色/公钥pin构建S/D，计算实际ELF摘要，核发只含launch_id的anchor和policy，最后核发引用ELF/policy/anchor摘要的Root launch manifest；policy不反向引用该manifest hash。该Root信任文件和实际启动/独占key使用须C独立核验，不以自报build替代。

将来S→D用固定SSH executable/argv、host-key pin、禁PTY/agent/转发及环境allowlist；D受限principal仅能执行固定receiver。`command=`本身不禁止转发，须连同restrict及实际sshd/权限核验；必要root调用/sudoers或其它launcher机制属于C安全变更，不在本包安装。[OpenSSH官方授权键选项](https://man.openbsd.org/sshd.8#AUTHORIZED_KEYS_FILE_FORMAT)

### 6.3 固定交互，避免EOF自拒和结果重放

1. S完成fresh capture，创建首次publication actor和当前保护；之后才spawn自己的固定SSH child。
2. HelloS→HelloD→SourceAuth→AuthAccepted；D尚未信任身份前不创建evidence attempt或写包。Auth通过后先在另准入的固定`/var/lib/knowweave-c4-b1-sessions`记录该backup-ID的session准入，才写入包；其残留也禁止fresh session重放。它不是原11步的成功记录，不被反序列化为authority；不污染原attempt/v2严格roster。该新根及anchor属于后续C安装范围。
3. S按manifest/既有parser派生允许roster发送package、原12件captured evidence及真实snapshot-effects副证明。FileBegin以受限role+index映射固定逻辑名，不接远端绝对路径或任意命令；FileEnd核长度/digest，全部SourceDataEnd后才继续。
4. D全字节重读、原11步/outcome及副证明完整复制/核验都自然成功后发DestinationReady。Ready带主proof、源副证明input绑定和现有D副proof的witness/intent原canonical字节及各自digest。S从自己原材料重算input与主captured proof，另对实际收到的D witness/intent字节算hash、严格解析并核capture/payload/epoch/journal/base-attempt关联；D新生成的effect_attempt_id/operation_nonce与D根事实是本次经认证原D的输出，不伪称S可独立重算物理结果。D只在真实原操作自然完成后派生这些输出，S最终决定绑定input和两个D输出摘要，D消费前再核原副proof。Ready不泄露签名DestinationWitness。
5. S原最终核对、持久负fence并回读，再发一次SourceFinalDecision；仍保持stdin打开。D在同一裁决内Pending→Consuming；此前已观测拒绝胜出。之后迟到S退出/取消不回夺许可，D本地错误/deadline/unwind仍失败。
6. D内部签名、receipt全部写入/同步/禁止覆盖发布/最终验证自然完成才Committed，并发CommittedResult（控制绑定+≤8192B原canonical receipt body）。此时尚未发最终End或退出。
7. S核完整结果后发SourceResultAck、End并shutdown其child stdin；D在**已Committed**后核该Ack/End/真实EOF及无额外字节，再发D End并自然退出0；S见真实EOF+自然exit0才报告正常成功。不能要求D在消费前先见S EOF，否则会触发Pending拒绝；不把纯finish当实际EOF。
8. Committed后reply/ACK/EOF/exit失败不能回退D状态或重发；S保持UNKNOWN/HOLD，保留fence。未完整结束或额外字节不能算本次正常调用成功；合法完整已签残留的历史reader结果另记。首版没有自动close，即使正常成功亦报告fence retained。

phase、transcript、声明长度、预算或不允许消息的错误在Pending中拒绝；Consuming中的本地验证/协议处理错误在本地完成前失败，迟到远端cancel/EOF按已采纳例外分账。全部IO owner须settled后才释放权限；不得让后台任务持可签对象而前台先返回成功。

### 6.4 D session持久空间与摘要对象

固定session root精确顶层为anchor.json/lock/operations/staging；按backup UUID独占目录，仅admission.json≤4096B、binding.json≤16384B、snapshot-effects-transport-index.json≤16MiB。全根含staging合计≤256 operation目录、1536目录项、32MiB元数据；不是把单会话files512当永久保留上限。实际包/大副件留原destination/evidence staging，operator原始日志另列不进入此metadata根。

所有列举/stat/open/内容读取/hash/重复核对均计入**原D同一累计ScanBudget**，不另开预算、重置或抵扣peer自报scan。达到任一上限或cap+1在新写入/分配前拒绝；partial/empty/extra/损坏/同步不明保留并关闭新准入，不自动清理。原持有操作的有限staging视图不能被新进程/ignore-ID复活。任何同ID残留阻止fresh session，磁盘admission不恢复D成功。现v2历史reader与恢复不扫描此session root或S fence。

精确hash对象在wire profile的digest_objects中固定：actor binding是域分隔后canonical {source_operation_id,binding_sha256,s_nonce,d_nonce}；root record SHA对应原registry enrollment的canonical原字节，不hash含自身摘要的wire summary；receipt SHA是完整现v2 receipt原字节，不是unsigned statement或自哈希。副proofSHA专指Ready实际携带的现D CopyWitness canonical原字节，intent另算原字节hash并绑定新operation_nonce；source input另有无D新字段的确定descriptor摘要。transport index不列自己的FileRef，source roster不混D输出，避免self-hash或最终签名反向依赖。

## 7. 负fence与旧writer覆盖

候选独立布局：`anchor.json`、`lock`、`operations/<内部operation UUID>/open.json`和固定`staging`。anchor绑定真实registry deployment/根/锁、fence根FD身份、profile及受审管理实现。open只含原S派生的backup/operation、group/build/代际/原snapshot与roster摘要、双nonce/原D attempt/最终绑定；不塞64MiB快照或任意caller路径。所有JSON严格固定字段与canonical，root私有单link、无follow，独占写/文件与目录同步/禁止覆盖rename/持有FD回读。

原S在registry原锁下创建open并最终再核。空/partial/staging/额外叶/未知版本/损坏/同步不明/超限都按仍阻塞处理。后继相关mutation通过同一registry锁加独立source gate检查；读/扫描计费仍用原累计budget。v1没有close记录或可被后来reader消费的解除凭据；额外close文件也拒绝，不能当“已完成”。

必须覆盖source prepare/finish/abandon/reclosure、pending/catalog/retained/journal/ACL、broker安装/cutover及相关副件写、registry enrollment/generation/root变更和restore目标与受保护根的关系；新blob不可覆盖已有digest不等于所有append都无影响。不能证明不会改变精确records/counts/generation/root观察的管理变更即拒绝。

**代码门与部署门缺一不可：** 新Rust入口检查不会让旧同pin binary认识fence；公开低层文件API不天然持lease。C必须逐项证明受审固定launcher/实际二进制、受限身份/可写mount、无旧自动入口和在途相关writer；合法维护/重装同样受门。v1实验源使用全新隔离namespace、没有业务worker随意写这些根。已具广泛Docker权限的管理登录身份不能转授S自动调用。此处不声称抵抗可信root故意绕过。

原S自己的有限再核只能用原内存对象+held FD；不提供ignore ID/允许布尔/反序列化所有者。D-only历史reader/恢复只验证当前D根、trust及captured proof；不因S fence存在或S不在线而拒绝正常历史读取。

`PRIOR_COMPLETE`不能在被成功fence封住的S1 registry再采集第二个ID。未来C/D设置必须另用已明确批准的fresh S2 namespace/registry/源根/固定profile，并在第二D会话开始前完成D接纳S2精确身份的独立准入，或经审等价的预准入设置；DestinationPolicy单一精确S身份不接受临时caller覆盖。现reader也依赖固定trust中的源根/control根/host/storage；S2准入必须保留D1原trust及reader语境，可使用另行准入的隔离view/实例或经审等价方案，不能覆写D1 trust后再声称旧CompleteBackup仍可读。保留S1 fence及D1既有package/proof/receipt原字节，第二传输前后独立核验原reader；不重置D1、不关闭旧fence、不在D2开始后换S lease。此设置未批准或未闭合就停，不能以流程图代替实际负例。

## 8. 逐子任务实现与审查顺序

1. **Policy/actor源起点与门清单。** 固定类型/布局/编译role，创建S首次publication actor与D首次local actor；明确feature缺pin拒绝、旧API不变和snapshot-effects输入。先审唯一public runner及allwriter闭包设计，未获R2b范围批准不编码。
2. **真实wire/auth/supervisor。** 按第5–6节实现有界读写、严格schema/phase/transcript、应用认证、真实EOF/自然退出和owner settlement；不先接任意签名器。
3. **Source fence。** 实现独立anchor/全局保护负约束、cap+1/部分发布拒绝和原S有限核对；不实现close或GC。部署旧writer排除仍是C门，不以单测宣告完成。
4. **原D真实证据。** 复用11步/validator/副证明路径、每步失败和同ctx预算；远端输入不冒充本地ProtectionSnapshot，不删Arc检查。
5. **一次消费与内部P。** 同一状态/锁，先Pending→Consuming，再内部原v2签名与具体receipt writer，全部自然成功Committed；错误/poison/unwind不可回退。抽取不递归锁/不再取得lease，不提前暴露witness。
6. **限定Linux检查与代码独审。** 实际执行格式/Clippy/编译/实际正文分账，ordinary用户新私有目录+固定builder；crypto只用公开测试向量，不生成/落盘运行密钥；有root/PG/真实安装需求的case具名保留未执行，不跳过后冒充整关通过。修复后全新测试批次，旧失败/HOLD保留。
7. **B/C及D执行单。** 在代码独审后才按实际binary/公钥指纹和新资源实例化；部署前证明固定launch/全部writer/资源，随后D1真首发+RA、D2源不可访问+RB与否定用例、原件独核、适用回归。不能把R2b单测或一份witness变成Task9正常整链。

验证清单单列于[计划JSON](../../task9/r2b-validation-plan.json)，全部为NOT_EXECUTED。旧37case的ID/内容/状态保持字节不变；后续以versioned映射分开Pending早拒与Consuming迟到事件，不把原规格抹掉或拼接为通过数。

## 9. 用户已集中批准的准确范围

**仅R2b产品代码纵切片、逐任务实现/独审，以及既有开发Linux服务器普通用户隔离目录/固定builder上的限定非特权验证；不用Windows运行产品测试。** 允许的代码面为第4节，候选数值/协议/不自动关闭代价为第3、5–7节。默认入口保持关闭，未安装pin/profile/key不得新首签/新发布。

明确不包含：生成/安装任何S认证、transport或D备份私钥；trust/registry/fence根/公钥pin现场安装；用户、authorized_keys、forced-command、sudoers/服务/防火墙或写权限变化；目标D产品运行；源停机/断联；PG/volume创建及恢复写入；旧失败/HOLD删除；main合并、生产或调度。B/C/D仍按实际准确资源与影响单独批准，不重复逐源码ZIP询问。

如果独审发现本候选会改变已采纳时序或必须加入未列权限/新服务，应先修订准确范围，不靠实现时临时加开关。用户已明确批准R2b编码与限定Linux检查，可以按本准确范围开始；当前仍NOT_IMPLEMENTED，B/C/D安装、安全权限及实际整链继续待准确批准。
