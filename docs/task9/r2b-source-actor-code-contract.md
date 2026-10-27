# B1 源端接线：准确补充编码合同候选与剩余停止点

2026-10-27。**私有只读候选；未采纳、未新增编码权限、未安装、未测试。READY_FOR_INDEPENDENT_DESIGN_REVIEW_NOT_PERMISSION_REQUEST。**

本稿以 Git `ccf71a5bf170bb08ce5c9b4668bef90ca47acf3d` 的不可变 blob 为源码输入，不读取或改动另一代理正在实现的 Task2a wire 工作区。原 `fd5784ad…`、`25091577…` 三项独审与前稿 `889c8610…` 全部保留。第五稿（保留前四稿及c144单项constructor观察HOLD）进一步固定 B0 launcher/job schema、真实产生者与 current process/cookie 检查，并给出 B1 两种计费 glue、推荐不合并原 quota 的共享 scan backing。**这些是可独审的准确新合同候选，不是已经通过独审、获批编码或真实安装。**

以下函数/类型/路径均是计划新增，不是现有 API。现两个 runner 仍拒绝，Task9/C4 仍 OPEN。不会用本稿或程序名创造 SourceAdmission、原 cut、D 成功、签名许可或 CompleteBackup。

## 1. 不变前提与已经核实的事实

- 已批准的 R2b 仍限 learning-backup 中的默认关闭实现及普通 Linux 检查；新增 learning-db 认证入口和两个 Root/capture 入口超出该准确表，需要补充**编码**批准。PG peer maps、launcher/账号权限、Docker/卷/mount、Root 信任和实际运行仍是另外的 C 批准。
- 原 `source.rs:969–986` 同时核 admin SQL 身份及实际原认证主体；runtime 在 `learning-db/.../lifecycle.rs:1218–1227` 和 `writer.rs:105–116` 另核。Root Unix peer map 到 SQL learning_admin/runtime 后，不会自动变成原认证主体 learning_admin/runtime。broker 当前 `transition::identity:117–128` 没有 system_user，不能声称它已有同等检查。
- Task1b holder 借用真实 `&RegistryLease`。host preflight 若持 holder/lease 等待 capture，child 的 fresh lease 会被同一 flock 拒绝。
- 原 `EnrolledSnapshotBroker::install_fresh` 在 source/snapshot_effects.rs:135–144 关闭 install admission 后 drop lease；quiesce 为 capture 返回真实 cut，捕获函数也在 retained/release 后关闭 SQL/局部 lease。保留 child 的 cut/enrollment 本身不等于继续持有该 registry lease。
- `copy_snapshot_effects` 按值消费 CapturedSnapshotEffectsV2，`write_copy` 只搬数据/预算到 SnapshotCopyCore，不搬 `_cut`/`_enrollment`。因此原 local 路径只要求它们通过完整 copy/readback，而非一直持到 sign/P。新跨机路径仍须由原 child 保持 cut 到真实 D copy 完成，不可由 JSON、FD、child rc 或 host 重开重建。
- `SharedJournalBudget` 是 learning-assets 中现有 Arc<Mutex<Meter>>；clone 实扣同一累计 bytes/entries，失败永久耗尽，无公开 reset。它可跨 crate 传递；backup 私有 ScanBudget 不能假称直接传给 DB。两者信用须分别说明。

## 2. A 的准确修订：仅两次阶段发证，原件永久不覆盖

删除旧候选的“每 20s 覆盖/刷新当前 proof”、64 generations 和借 TTL 延长原 actor 的暗示。新候选只有下列两个阶段，Root 仍持续观察当前实际资源；观察失败使原 driver lock/原 operation 失效，不能通过再发一个 proof 恢复它。

固定私有根候选为 `/var/lib/knowweave-c4/b1-source-observation/jobs/<Root内部job UUID>/backups/<child内部backup UUID>/`，里面仅 `install/` 和 `capture/` 两个 generation 目录。Root job根另存固定启动/结算记录，见B0机器表；不向registry严格集合添叶。路径由内部 phase 枚举派生，外部只能回显请求 UUID，不能提供 generation、路径或脚本。backup UUID 在本次固定 capture child 内部一次生成；Root 独立核本次 exact exec/process、installed project/DB/recipe 后发证。

每个 generation 的 proof leaf 仍为 `isolation-<backup>.json`、inspection leaf 为 `inspection-<backup>.json`、driver lock leaf 为 `isolation-<backup>.lock`。因此原 SourceBackupConfig 仅选择不同父目录，不能放宽原 leaf 名/backup-ID 连接。

| generation | 唯一发布时间与消费者 | 不变原件与锁 |
|---|---|---|
| install | child 的 install admission 前请求，Root 实际全量观察后一次发布；本次 install 使用此完整路径，require_fresh≤30s | proof/inspection 独占创建、完整 sync 后发布；禁止覆盖/rename 替换；Root 原 driver 持该 lock 的同一 inode。enrollment 后续继续核相同字节/FD，不更换成 capture generation |
| capture | 全部 broker work/子进程自然 settled、quiesce 关闭其 broker/runtime pools 后，在 fresh capture admission 前新请求；Root 再全量观察并一次发布 | 与 install 是不同目录/不同 lock；source pipeline 全阶段使用 capture 路径/原 bytes；`require_fresh=true` 仅在原初始门，其后原 require_fresh=false 及真实 live-lock/native 再核不改 |

proof≤4096B、inspection≤1MiB 的原 cap 不变。proof/inspection 两个已发布文件和 lock 身份不可变；generation 目录只额外预留原 pipeline 指定的 `runtime-connect-denied-<backup>.json` 一次 append：Root 在真实 ACL 关闭后独立探测，再独占写≤1024B的固定否定记录，不覆盖已存在叶。**这是已发布 proof 集不变，不是谎称整个父目录从发布后再无目录项变化。** 原消费者无父目录时间戳/严格 roster 证明时不新增该声称。

Root 必须同时保留 install/capture 两个 live-lock owner 至各自真实消费者结算；不能为了减少 FD 提早释放 install lock。全量连续观察、请求超时和总期限不产生新的 generation。任一阶段的旧残留、重复请求、过期初始观察、Root 观察失败、未确认结算或预算耗尽均 HOLD，不复用该 backup-ID/旧 generation，不清理旧证据。

新 Root job 的请求/ack 固定≤4096B、最多两次发证；capture supervisor 总1200s、请求30s、收尾观察30s只是**本稿候选上限**，不宣称当前已经批准/实现/实测。publication 仍按已批首次 S 起1800s、取外层更早 deadline；不把超时说成内核同步/物理 kill 已完成。旧 128MiB/64代建议不随本稿带入已批 profile。

## 3. C 的准确修订：preflight 释放、原 child cut 与首次 host S

下列是选择的唯一候选顺序，不再保留“若将来要求 host 持 cut”这个悬置分支；仍待独审/补充编码批准，不改 R1 例外语义。

1. host 固定 source 启动：Root installer/实际 ELF/launcher 初检、原 registry/group 根初检均为 **preflight**。按明确作用域先 drop Task1b metadata holder，再 drop 它借用的 registry lease；随后才启动 capture child。此 preflight 不是 publication S，全部holder、lease wrapper/keys、FD和其authority均先释放；本host process ctx的scan quota仍连续保留同实例，不reset/refund。保留计费不是续用旧permission。
2. capture child 仍真实处于 PG pid/mount namespace，执行新十二 mount recipe 和两阶段 Root 发证；install 原 close/drop、真实 work settled、quiesce、fresh capture 的各原 owner/锁/预算均不变。
3. 原 prepare capture 自然返回 pin 和 CapturedSnapshotEffectsV2 后，child 仅报告固定 `CAPTURED_DATA_READY` 数据状态，并继续持它的真实 cut/enrollment/FD。child 不持 install/capture 局部 RegistryLease；不把该 Rust 对象序列化或转交 host。
4. host 此刻才 **首次** 创建 publication S：重新核实际安装/metadata，重新 acquire 真 SourceAdmission、原 control binding、首次 publication RegistryLease/累计 budget、当前 ProtectionSnapshot 和相同 inode/完整原字节的 pin/adjunct。旧 preflight/capture 成功不代替它。任一失败时 D/SSH 尚未开始。
5. 这个原 S 才启动并监督自己的固定 SSH child；D 用本次原 authority 完成十一步骤、真实 remote input copy/readback、副 proof，S 核经认证的准确 Ready/摘要。收到未验证 Ready 形状不触发任何 child 结算或最终许可。
6. **认证 Ready 后、S final recheck/fence/Final 之前**，S 向原 Root supervisor 发唯一固定 settlement 请求；Root 通知该 exact capture child 只结算本次原 cut、无在途操作。必须观察 child 的受支持自然 exit0、完整控制 stdout/stderr EOF、exact Docker exec/process owner 与 Root driver 的自然结算结果。最后一条 child ACK 不证明物理活性/停止，不代替 EOF/wait。host 原 S/lease/保护在这一阶段连续保留，不能换 S 重试。
7. settlement 已自然确认不追溯撤销 D 已完成的独立 copy。S 再按原 held 对象做最终核对、持久写/回读负 fence，然后发唯一 Final。D Pending→Consuming 之后迟到 S 退出/取消不回夺许可；原 R1 的 D 本地错误/deadline/unwind拒绝及 ACK/End/EOF/自然退出分账不变。正常成功同样保留源 fence。

失败排序固定如下：

| 时点 | 结果 |
|---|---|
| 认证 Ready 前 child 异常、失联、未知或原 cut revalidate 失败 | 原 S 停止新许可；D 尚 Pending 则取消/拒绝，owner 未 settled 保留 HOLD；不建立第二 child 或第二 S |
| 认证 Ready 后 child 仅发 ACK，EOF/wait/资源结算未确认 | 不算正常 settlement；S 不发 Final，双方按未完成/HOLD分账；已复制字节不是调用成功 |
| 认证 Ready 后按第6步自然 settlement 完成 | D copy 保持有效输入；host S 原保护继续，随后仍须 final recheck/fence 才能发许可 |
| 第6步 child exit非0、extra bytes、协议错、超时或精确 owner未知 | 无 Final；不按“copy已经好了”放宽，不自动重开 child |
| Final 已被 D 原状态机消费后的 S/Root/child 迟到取消/退出 | 不追溯撤销 D 许可；原 D 的本地错误/deadline/自然完成判据继续生效；源 fence 留存，不以本稿扩展重试 |

cut 不是在 host 重建的能力；D Ready 也不是 child 的内核活性证明。第6步的正常 settlement 只是原 child 的实际结算，不让磁盘状态恢复原 S/D 权威。新 source/export seam 在 child 内保留原 cut、逐次 revalidate，host 只接受由本次真实保护验证的数据；D receiver 的 authenticated remote input 仍是原 D 私有消费者，不用它伪装本地 CapturedSnapshotEffectsV2。

## 4. B 的具体 factory 方向：无 caller AuthMode、无跨 crate 私有伪装

本节是待独审蓝图，**没有声称现有源码能直接构造此上下文**。推荐将真正的 Root installed-process 例外准入放在 learning-db 自己可见的新 `snapshot_broker/b1_peer.rs` 中；learning-backup 不向它传“已经验证”布尔、任意 claim、path、role、DSN、key 或 signer callback。

具体无caller connection/pool的owned API见4.2；所有型别字段private，无Serialize/Deserialize/Clone/Default/Debug，也不提供AuthMode/allow布尔。Linux+默认关闭DB feature是必要条件，不是资格本身。factory根据新编译process-role选唯一自身row/固定路径，实测本进程ELF与实际Root原启动/当前job；不能通过caller声明角色或选择路径。RootLauncher自己的producer入口不能因此取得DB source ctx。

### 4.1 factory 必须自己完成的真实核验

1. 缺 feature 或编译 role/profile/public source identity pins 直接拒绝；Linux 实际 r/e/s uid/gid 均 Root，实际 ELF 是单link Root私有不可写安装文件，与 `/proc/self/exe` 同 inode/准确 bytes/hash；安装元数据、祖先、具名重开及 held FD 不可替换。**这些是必要条件，不是仅凭 UID0 或 feature 就准入。**
2. 固定 Root descriptor 只能由 C 的独立 installer 核发；host固定leaf为`/var/lib/knowweave-c4/b1/source-peer-execution.json`，capture用4.7列出的#9 RO release同字节副本而不打开host路径，它引用已经生成的 source/capture/launcher manifests 的准确 hash，不能含自己的 hash。先编译/测实际 ELF，再发行各 launch manifest，再发行此 descriptor，依赖无环；旧 policy 不反向 hash 此 descriptor。此 leaf 本身不是“自报 build”，还须以下原启动核验。
3. host source 必须来自新的固定 Root launcher ELF 的原 fork/exec。Root launcher 自己创建私有 Unix socketpair，只把固定 control FD3给 exact source child。factory 用实际 SO_PEERCRED、原 parent PID/start、pidfd/Root uid、父 ELF held digest/身份及本次启动 nonce/active-job 原锁核验父。任何来自登录 shell、另一个 Root Python、caller env/CLI 或假 FD3 都拒绝。不可把 shell路径/进程名/最后 ACK 当真实 launcher 身份。
4. capture 经 Docker exec，不能谎称它的 PPID 是 host launcher。Root launcher须掌握并独立核 exact Docker exec ID/PID/start/cmd/exe/PG namespaces/native/socket/image/十二mount recipe，给该唯一 child 的新内部 backup-ID写只读 generation 原件并持原 lock。capture factory自己核实际本进程 PG native、实际 ELF、对应原 Root-issued job/generation 的准确 bytes及live lock；child不能写这些文件，不能仅把收到的JSON反序列化成上下文。
5. 原 Root launcher 的可写 Root observation/manifest/job 权限与 Docker 管理面不交给 SSH transport/UID65532 worker。Root恶意管理员故意越权不在既有防护声称中，但普通调用者/另一个未列 Root binary 不能从 root-owned同名字节取得这项 factory 资格。
6. ctx绑定唯一 source deployment/group/DB、实际PG OID/SID/postmaster、实际Unix socket dev/ino和允许role，host alias必须另由C证明同一socket。实际SQL会话须由4.2的factory自建固定Unix options并核非TCP SQL事实，不能收caller连接；admin仍current/session/owner=learning_admin；broker仍固定broker role；runtime仍restricted LOGIN/current/session=learning_runtime。每个实际查询的system_user只允许严格 `peer:root`；不允许trust NULL、任意method:root、SET ROLE或其它PG。旧普通入口的原主体检查不变。

**B0具体化：** 上述第3–4项按本稿4.4和 `MACHINE-SCHEMA.json` 的唯一固定Root Rust launcher、job和两generation状态机实现。当前源码尚没有这套实现，schema仍待独审。capture在PG pid namespace中不能假称持host Root pidfd或核host父PPID；它按真实本进程ELF/native/internal nonce、Root原件/live lock与Root限定producer闭包核准。Root/Docker管理员故意伪造文件仍按既有可信运维排除；本候选不新增host-proc、CAP_SYS_PTRACE或host namespace往返。

### 4.2 原会话由factory自己创建：OwnedB1AdminSession与固定三pool

本稿明确选择强路径，删除旧候选的caller PgConnection/三个PgPool输入。新的opaque process ctx不接受DSN、caller session/pool、用户名、path、cap、认证模式或callback；固定endpoint从本角色实际准入的Catalog取得。`OwnedB1AdminSession`也无公开字段、Serialize/Deserialize/Clone/Default/Debug或接受现成connection的constructor。

```rust
pub struct B1InstalledSourceProcess { /* private Arc<PeerOriginal> */ }
pub struct OwnedB1AdminSession { /* private original, pool/connection owner */ }
impl B1InstalledSourceProcess {
    pub fn admit_installed() -> Result<Self>; // no caller parameters
    pub fn revalidate(&self) -> Result<()>;
    pub fn scan_accounting_meter(&self) -> SharedJournalBudget; // same clone only
    pub async fn acquire_fixed_admin(&self) -> Result<OwnedB1AdminSession>;
}
impl OwnedB1AdminSession {
    pub fn database(&self) -> &str; // actual validated data, no setter
    pub fn connection_in_original_owner(&mut self) -> &mut PgConnection;
    pub async fn close(self) -> Result<()>;
}
impl RootSnapshotBroker {
    pub async fn install_fresh_b1_peer(
        original: Arc<B1InstalledSourceProcess>,
        installed: InstalledSnapshotBinding,
        source: BrokerSourceDescription,
        sockets: (&UnixListener, &UnixListener),
    ) -> Result<Self>;
}
```

RootLauncher角色不能取得B1InstalledSourceProcess或DB连接；该factory只接受实际PublicationSource或Capture角色。Root launcher自己的有限只读观察/probe由另批C的固定producer配方负责，不通过本API取得可泛用SQL代理。

`acquire_fixed_admin`不收purpose枚举。Capture ctx的private状态自动限定初次install admission，实际close之后才允许RootBroker消费同一专用install pool；真正RootBroker安装完成才记Installed；实际quiesce等待broker/runtime pools与全部work自然settled后才记Quiesced；当前不可变capture generation同时有效时，才允许一次新的capture admin session。PublicationSource ctx只有在本次Root capture-data已连接后允许一次首次publication admin session。caller消息、bool、文件success或调用第二次方法不能推进这些private状态；任何失败/未知永久拒绝后续acquire。

固定池构造在DB私有session叶中，只有private持有者能clone：

- Capture install admin为1连接专用pool；source install admission关闭的是其真实close_on_drop连接，pool继续由ctx private owner持有，供随后RootBroker固定安装消费，之后无论安装成功/失败都close该专用pool并等自然完成。
- Broker/runtime pools由新的B1 install内部自己创建并持有；固定max_connections=4各、min_connections=0，admin=1。role分别为learning_admin/learning_snapshot_claim_broker/learning_runtime。quiesce按原owner关闭broker/runtime pools，不能仍留管理pool给business work。这些是待批候选资源数值，不是已测并发或FD保证。
- Capture阶段的admin为quiesce后另一个真正新session/pool，仍同ctx/fixedDB；它不是恢复已结束install admission。Publication阶段亦只有host首次S自己取得的新session/pool。两者max_connections=1/min=0，在各自显式close时关闭connection及对应专用pool；原held lease/FD/owner一直保留到已开始操作自然结算。
- 所有acquire得到连接后立即close_on_drop，先于任何fallible SQL、ctx重核或await；其后才读真实角色/owner/system_user、DB/OID/SID/postmaster。连接不回池供另一个操作adopt，pool无公开getter，ctx失效后不创建新pool/fresh session或fallback到legacy。

实际连接构造只用锁定SQLx0.8.6的`PgConnectOptions::new_without_pgpass()`，在真实Root固定启动的七键环境及其准确值重核后才调用；PGHOST/PGPORT/PGUSER/PGDATABASE使constructor本身不进入default_host/whoami，而非后设setter消除已发生观察。随后仍显式设置固定Unix endpoint、内部固定role/DB、Disable TLS及固定application_name；准确分支和同预算收费见4.9。该API只跳过passfile，其余12个固定env键查询均明确分账；不使用URL/FromStr/new/Default/password setter或caller连接，不声称SQLx有peer-only handshake开关。[SQLx0.8.6官方API](https://docs.rs/sqlx/0.8.6/sqlx/postgres/struct.PgConnectOptions.html#method.new_without_pgpass)

SQLx不公开底层已连接Unix FD的这项所需接口，**不声称OwnedB1AdminSession取得了该FD/SO_PEERCRED，也不把stat某socket当其底层连接证明**。这里的代码保证是fixed Catalog→private fixed options→自己创建并持有的实际session provenance；代码另核实际SQL的Unix/nonTCP、角色/原主体/DB/OID/SID/postmaster。host alias与capture socket同inode、无RW替换/错误alias、实际PG/native/code均由Root真实producer及C闭包独立证明，不能由caller PgConnection拼装。公开raw borrow在Rust类型层面仍可执行SQL，不能声称它本身是restricted SQL API；其合法性依赖factory已核的固定实际ELF中只有backup私有原owner固定调用闭包，没有脚本/插件/任意SQL RPC。另一库caller不能伪造opaque ctx，也不能拿自己构建的ELF通过factory；任何未来新增动态SQL入口须另审，不能据此泛化权限。

backup的SourceAdmission新增private连接owner分支，B1只从OwnedB1AdminSession进入；旧PgPool路径及旧require_admin_owner完整保留。它仍取得原两个advisory keys，binding与源profile查询仍用原实际借出的connection。借出raw connection仅供该原owner内既有固定SQL，不能换连接/还池、变成任意SQL RPC或因借用而让ctx重核义务消失。B1正向管理/写入/最终许可门前后必须重核同ctx；失效后只允许原held session执行既有固定rollback/负向reclose并自然settle，不能GRANT、创建新正向authority或开新session。负向结算失败/未知继续HOLD；不保证总能reclose，不用新的资格绕过旧失败。

新的DB install将同genuine Arc保留到RootSnapshotBroker和history::runtime::Original；runtime first-write与broker transition仍在原slot里核它、原实际pool/session和精确DB事实。旧None分支只跑旧谓词，永不从B1错误转为None。`peer:root`例外仅在上述真实ctx与原fixed-session owner连接中接受；不是公开一个allow_peer_root标志。



### 4.3 B1：两种跨 crate 计费 glue，推荐共享 scan backing 而不合并 quota

先区分现有两个原账本：registry ScanBudget 为512MiB/100000项；epoch SharedJournalBudget也为512MiB/100000项，但它是不同scope，DB ObservationBudget还有64MiB更小子限。`cutover.rs:88–95`明确CLI实际read/hash只扣registry，epoch只付固定binding。**不能因两个数值相同就把账本合为同Arc，更不能覆盖64MiB子限或减少真实重复read/hash。**

| 选择 | 准确改动与可行性 | 代价/结论 |
|---|---|---|
| Q1：仅计费callback | DB固定factory收一个只接受bytes/entries的计费closure/trait，backup私有固定调用点用它捕获真实原Arc<Mutex<ScanBudget>>；factory仍必须自己核真实ELF/Root父/active job，callback不提供auth资格 | 外部库caller能够传noop；只能依靠已测固定ELF、无caller RPC/可加载代码、原调用闭包独审证明当前body绝无noop。还需在factory元数据IO前先有真正registry计费owner。它与已批准“不接caller budget”的表述有张力，需明确扩合同，不能悄悄当内部实现细节。**不推荐** |
| Q2：genuine factory自建scan backing，私有registry B1入口取同实例 | factory不接budget参数，仅在第一次真实进程准入开始创建一个scan用途SharedJournalBudget；DB ctx实际元数据IO与backup全部registry IO扣同原scan实例。epoch journal仍是InstalledSnapshotBinding原独立meter；保持所有现有更小子限。 | 需要准确改registry.rs和新增assets纯身份比较一叶；没有caller选择/重置预算。更早耗尽可能拒绝原本两个独立stage各自能做完的合法输入，属于本候选可用性代价，不承诺任意256MiB通过。**推荐此候选，待独审/用户补充编码批准** |

Q2的准确拟新增表面：

```rust
// learning-db opaque factory privately creates this once, never accepts one.
impl B1InstalledSourceProcess {
    pub fn scan_accounting_meter(&self) -> SharedJournalBudget;
}
// learning-backup private entry; not public, no caller path/cap/meter.
impl ManagementRegistry {
    pub(crate) fn open_installed_b1(
        original: &B1InstalledSourceProcess,
    ) -> Result<Self, BackupError>;
}
// learning-assets: identity comparison only, not authority or a reset handle.
impl SharedJournalBudget {
    pub fn shares_meter_with(&self, other: &Self) -> bool;
}
```

getter返回同ctx内scan的clone，只能进一步charge/usage，无法构造fresh inner、refund或reset；这不返回SQL/writer/sign权限。DB factory的process-static一次性入口在**任何fallible IO之前**原子标记已用，失败也不能在同一进程再次mint ctx/meter。新root进程不能adopt旧job/backup或绕过原fence/unfinished记录。

原 `ManagementRegistry::open_installed()` 本身已经open root/read_snapshot；因此**不能等try_lock后才接账本**。新私有open_installed_b1在第一个Root祖先遍历/open/stat前就建立以ctx scan clone作backing的ScanBudget，再读原snapshot；原try_lock/recheck继续使用它。legacy open/default完全不变，B1错误后不能退回legacy。

每次fresh Registry打开仍产生独立 `Arc<Mutex<ScanBudget>>` wrapper，**旧require_shared_scan_budget的Arc::ptr_eq一字不放宽**。两个不同lease就算共享quota backing，也不因此成为相同operation/authority；原CapturedSourceProof/ProtectionSnapshot等仍须原wrapper/lease身份。keys集合仍属于该wrapper，新增key及所有metadata费用须在插入/读取前实扣相同scan；remaining_entries取原scope剩余，不用局部zero重新宣称100000可用。

准确call graph：

| 调用点 | 原账本 | 所有者/失败 |
|---|---|---|
| DB factory初次Root metadata/actual executable read+hash、Root lineage/job检查 | factory自建scan，512MiB/100000，动作前/中收费；元数据与ELF原cap各保留 | 原B1Process ctx；失败永久poison，无新ctx/新meter |
| host preflight Root/registry初检 | 同host ctx scan；不是publication authority | holder先drop→lease再drop；ctx和meter仍是本次同Root进程身份对象，不把旧lease续用 |
| child install/capture registry/控制原件、DB ctx再核 | 同child ctx scan backing；每个原lease仍独立wrapper | install close/drop、capture真实新lease；ctx计数不reset |
| child epoch journal/prefix/namespace、DB observation/firstwrite | InstalledSnapshotBinding现有shared_budget()给出的原journal；ObservationBudget64MiB子限不变 | 原cut/Original owner；B1 install拒绝journal与ctx scan误为同Arc |
| adjunct files::charge同一次实际IO | 原registry scan和原journal各扣一次，两个scope始终distinct | **不改该helper，不因为指针相同做泛化减计**；意外同Arc在install前拒绝 |
| cutover CLI executable read/hash | 仍只扣原scan；epoch仍只固定binding | 不把payload挪入journal或扩子限 |
| host首次publication S、后续真实metadata/held FD再核 | 仍本host ctx scan backing，但fresh首次S原lease wrapper；所有实际重复read/hash每次都扣 | 原S连续持有，预算耗尽HOLD，无第二S接续 |

assets新增shares_meter_with仅做Arc::ptr_eq；DB B1 install用它检查 `!ctx.scan_meter.shares_meter_with(installed.shared_budget())`，相同就拒绝，不把账本去重当成功证据。old normal API不调用新branch，旧双ledger计费不变。仅两个scope真实相同且规范明确要求一个原操作只一次reservation时才有讨论去重的必要，本候选根本不制造该同Arc情况。

这是同原scope账本跨crate共享的具体设计，不是事后累加peer或child报告；Root/Docker observer作为另一个实际进程的控制采样费用另在Root原job账本中实扣，不能把其自报计数抵扣S。产品≤64FD与runtime/Root/SSH各FD实际清单仍须独立动态验证，几份局部计数不成为global64证明。

### 4.4 B0：固定Root launcher/active-job机器合同

完整固定字段、值域、产生者、前像、held FD及不可变顺序见同目录 `MACHINE-SCHEMA.json`。第三稿补齐第二稿机器表遗漏的installed_runtime：固定准确project/CID/daemon/network/PG image/十二mount recipe、PG pid/mount namespace和postmaster start；Root launcher只能从该已准入项选本次真实PG，不能从CLI/wire或“观察到某个PG”反推授权目标。capture实际process/native和Root exact exec必须匹配此项；重启/namespace变化拒绝，不自动重发安装或reenroll。另明确每RoleImage的code_commit/build_input匹配其实际编译pin/launch manifest，**不与SourceGroup业务application_commit/build混为一项**。第二稿只是未审候选，从未运行或安装；此纠正不改任何测试结果/旧HOLD。它是**待审设计资料，不是产品输入或已核发安装文件**；下列为必要的解释性约束。

**可信边界。** Root source-launcher改为准确的固定Rust ELF，不用Python进程名/脚本路径冒充已运行代码。它自己从固定Root descriptor取唯一 source group/CID/命令配方，env_clear，绝不收manager argv/path/DSN/SQL/username/command。它不持source-auth/transport/D signing key，不代S生成签名或启动S自己的SSH。Root/Docker管理员在维护窗中可信的既有运维假设保留，不能据此宣称抵御恶意Root、kernel、ptrace或Docker operator。

**无hash环。** C先批准真实公开身份pin→构建实际S/Capture/Launcher ELF→测actual ELF/依赖→发各自launch manifest（只引用launch_id及已知policy/anchor/ELF）→最后发source-peer-execution descriptor引用这些已知manifest hash。descriptor不包含自身hash；每job start记录可hash此已存在descriptor，generation binding可hash已发布proof/inspection，ACK再带已写binding的hash。被引用文件从不反向包含后写对象hash。

**host核真实原父。** Root launcher建立仅本次的Unix socketpair，自己保留server端，fork/exec唯一host S child时仅把child端放到固定FD3；其余非stdio按4.6的准确CLOEXEC机制封闭，不假设Command自动关闭。host factory先按4.6在任何helper前burn once并恢复/核FD3的CLOEXEC，再核实际自身ELF/固定安装及FD3实际SO_PEERCRED（Root launcher PID/Root credentials）、SO_COOKIE、Root原父start/pidfd及实际parent ELF；Root私有start记录必须精确对应这些kernel事实。Root launcher知道自己fork出的source PID/start，记录socketpair child endpoint cookie；不把登录shell、caller env、普通stdin或任意“Root已验证”文字当父证明。真实PID/start/cookie或held执行文件替换即sticky失败。

**capture不假核host父。** Root launcher创建/监督准确Docker exec，当前capture child内部fresh backup-ID和nonce发一次固定CaptureInit。Root producer自己从同daemon的该exec ID及实际host proc采样，确定host PID/start、container内PID、namespace/native、实际ELF与精确十二mount。capture factory自己核其 `/proc/self` 的namespace PID/start/ELF/ns/native与自身nonce，比较Root-owned immutable capture-start、当前generation bytes、job/driver live-lock及固定installed descriptor。host PID和Root父PID在capture namespace外时只是Root producer的已绑定观测，**不谎称child自己打开了其pidfd或验证它是PPID**。不新增host-proc、CAP_SYS_PTRACE或namespace跳转。另一手工Root进程会因当前process tuple/内部nonce/真实exact exec不匹配被拒绝；可信Root故意伪造全部观察属于明示排除，不能算已防御。

**freshness/owner。** factory第一次准入只能消费当前本进程新Root job，不从目录挑“最近成功”或读取旧成功恢复原ctx。Root job UUID、root cookie、child内部nonce/backup-ID分别new；cookie只是本次关联挑战，不是持久凭证或sign key，不能单独授权。所选proof始终保留原bytes；install/capture初始30s门和原require_fresh true/false调用点不改。任何另外存在的原fresh门也仍30s；不把1200s外层当proof有效期。合法但慢的capture/Root采样/大输入仍可能HOLD，未有实际可行性证明。

**有界固定control。** Root控制通道不同于S↔D的KWB1：单独四字节u32be长度前缀+唯一typed canonical JSON，消息≤4096B，双向两通道合计最多64帧/262400B（包含4B前缀），无raw body。header先核长度/总budget再分配，duplicate/unknown/noncanonical/错cookie/错phase/EOF/额外bytes均永久拒绝；不resync。固定消息集合/准确字段在机器表，不携任意path/cap/command/SQL/权限对象。Root只处理自己拥有的两个子进程本次已发生的固定phase，不成为通用broker。

**不可变与失败。** Root job目录只独占创建。start/capture-start/proof/inspection/generation-binding/captured-data/settlement各leaf至多一次，文件和父目录自然sync后才发该阶段ACK；已发布leaf不覆盖。每generation binding最后发布，引用既存proof/inspection准确bytes和同lock inode，不hash自己。partial、额外、同ID残留、同步未知、request重放或角色/phase冲突拒绝新准入；不删除失败。

Root失败用一次追加 `hold.json` 与固定Abort reason码使当前ctx permanent-failed；consumer每个原关键门检查其不存在及Root原lock/控制通道。释放lock只是否定资格，**不是“子进程已停机”证明**。Root仍保留known process/exec原owner并等真实settlement；观察期后不明报UNCONFIRMED并保留owner，不重建/换job假成功。Root正常exit只在自己的全部known owner真实结算后；原SourceFinal/DConsuming已发生时仍遵从R1，不因迟到Root控制断线倒转D已消费许可。

**read-only目标和预算。** 本方案只说明未来source Root控制面。没有提出C创建PG/卷/peer maps/Root信任的当前批准；这些仍另批。Root观察log每侧1MiB、控制总262400B，proof/inspection及原执行输出沿各原cap；总capture1200s/request30s/settlement观察30s为本稿待批候选。所有真实执行、Root metadata正向、fullFd、故障注入和预算最坏call graph均未执行，不因机器表写出而获得测试信用。


### 4.5 两个编译pin族及真实B1 registry准入桥

两个族不能再有双重含义，不能只给JSON字段改名：

| 编译输入 | 唯一语义与生产者 | 实际消费 |
|---|---|---|
| 现`KNOWWEAVE_SOURCE_COMMIT`、`KNOWWEAVE_BUILD_ID_SHA256` | source业务application commit/build；由C对真实SourceGroup/业务构建准确核发 | 原registry validate_build_binding/admit_source继续原样使用，SourceGroup.application_*不得删除或拿manager值替代 |
| 新`KNOWWEAVE_C4_B1_MANAGER_CODE_COMMIT`、`KNOWWEAVE_C4_B1_MANAGER_BUILD_INPUT_SHA256` | 当前所构建的B1 manager/receiver/capture/launcher release code commit及该角色build-input，不是业务app | offhost BuildPins的commit/build_input改取此族；manager launch/policy比较、新DB factory自身RoleImage实测及C的对应release manifest使用此族 |
| 新`KNOWWEAVE_C4_B1_SOURCE_PROCESS_ROLE` | publication-source/capture/root-launcher精确编译角色，不是CLI/env运行override | factory按编译角色选择固定自身row/路径；RootLauncher不成为DB source ctx；各bin逐角色构建并测实际ELF |
| 既有source-binding/profile/source-auth/verifier等pin | 原准确用途 | 不被上述拆分放宽；缺失/值错照旧拒绝 |

每个RoleImage的code_commit/build_input必须等于该实际binary对应的新manager族、launch manifest及实际ELF摘要；不同role的build_input可不同，不能强令等于host source的build-input。SourceGroup.application_commit/build则必须等于旧application族和实际原registry组；Wire Hello/SourceBinding的application字段仍来自这个组，manager_release来自实测manager ELF。无需更改公共v2或现wire字段名。

新B1 private registry桥必须实际存在：`RegistryLease::admit_control_source_b1(control,database,&genuine_ctx)`及assets/config路径的`admit_source_b1`，先走原unique group/root/source binding/compile application pins校验，再连接同ctx的真实manager release/new compile pins、SourcePolicy/Catalog的deployment/group/DB和原group record。普通旧admit入口不改；本B1入口任何检查失败不退回旧入口或重开lease。preflight只作数据/根初检且随后丢弃全部wrapper/FD/authority，真正source admission只在对应原session/lease内取得。

准确改叶包括offhost_b1/mod.rs的编译pin构造、policy.rs对manager语义的说明/真实检查、新DB process factory与registry.rs的B1桥；测试须同时覆盖manager≠application合法匹配和任一族错/缺失拒绝。不能删原app guard、接受动态wire字段当pin，或在新源码里假称旧SOURCE_COMMIT一直就是manager。该拆分也影响B1 destination launch使用的manager pin输入；它不授权目标执行/安装。

### 4.6 FD3和环境：唯一handoff及后续exec隔离

Root→S的privilege-bearing endpoint只有一次handoff例外。Root launcher同步入口先建立受控FD集合，使用Linux `close_range(3, UINT_MAX, CLOSE_RANGE_CLOEXEC)`或等价经独审的明确机制，把全部非stdio继承FD标CLOEXEC；失败/不支持在任何子进程前拒绝，不能退成“Command会帮忙”。Root parent预留数值FD3，避免后续dup覆盖Rust std内部exec-error pipe；Root→S的唯一pre-exec把**准确child socket endpoint**放到3并临时清CLOEXEC，其余实际非stdio均CLOEXEC，不能广泛inherit。

S同步factory最早阶段先burn一次attempt，再对实际FD3设置并复核FD_CLOEXEC，之后才有后续spawn；保持同一OwnedFd，无dup/公开raw-fd getter，核socket类型、SO_PEERCRED、SO_COOKIE、原parent PID/start/pidfd及Root启动记录。任何flags/身份失败sticky拒绝。随后SSH/Docker/pg工具的白名单一律只有stdio；Root endpoint、metadata/lock/pidfd/registry等FD不可继承。所有新建FD用原子CLOEXEC；spawn gate核实际flags/闭包，不能用caller bool报告已关闭。相关stat/flags/遍历计费、产品/runtime FD实际清单仍保留检查义务。

不建议在Rust Command的pre_exec盲目close_range所有≥3：它可能破坏stdlib内部exec-error pipe。上述机制是在同步入口先设CLOEXEC，并对后建FD保持该属性，只有Root→S的准确handoff在exec前清一个已预留FD；stdlib自身的CLOEXEC错误管道仍保留原exec语义。Root→Docker和S→SSH没有这项例外，Root库调用也不得清FD3标志。若实现选不同fork/exec机制，必须另给准确错误管道/白名单和失败证明，不临时假设。

host S由Root Rust launcher以env_clear及4.9七键/准确值启动，PGHOST/PGPORT/PGUSER/PGDATABASE全部由已准入固定Catalog生成。capture不能假称Docker exec Env overlay等于env_clear，也不在tokio/多线程后unsafe修改全局环境。新固定配方选择镜像内**准确`/usr/bin/env -i` wrapper**：argv严格按4.9为`/usr/bin/env -i PATH=<固定系统路径> HOME=/root LANG=C PGHOST=/var/run/postgresql PGPORT=5432 PGUSER=learning_admin PGDATABASE=<同已准入DB> /opt/knowweave-c4/b1-capture/knowweave-c4-b1-capture`，无caller字符串/任意command。C须核该镜像内env ELF/依赖/实际SHA及存在性；仍用现十二mount，不新增第十三项。

Root要分开观察Docker ProcessConfig的wrapper/精确argv和同exec ID/PID/start最终变成的capture ELF；wrapper只exec不fork，若未转成准确capture、改PID、额外程序或未知结算即HOLD。捕获internal nonce/Root当前owner必须在最终capture进程里产生并绑定；不能把见到env wrapper当capture已就绪。capture先同步CLOEXEC屏障/FD白名单，再运行factory/runtime/PG逻辑。

这只清理新capture进程环境，不改PID1的固有PG_MAJOR/PG_VERSION/PGDATA；不把这些无害镜像元数据称密码。进入SQLx factory的Source/Capture实际环境闭集仅为4.9准确七键；PGHOST/PGPORT/PGUSER/PGDATABASE是Root按已准入Catalog生成的固定值，不能由caller覆盖。除此之外的PGHOSTADDR/PGPASSWORD/PGPASSFILE/PGSERVICE/PGOPTIONS/PGSSL*等PG键、DATABASE_URL/TEST_* DSN、LD_*和其它override均拒绝；禁止秘密键只识别key即拒绝，不读/记录其value，不混入PG工具的独立env白名单。Root launcher自己不由这些env构造DB连接，它的工具子进程也明确env_clear。未来C实际验证此固定wrapper/环境/FD闭包之前，代码默认拒绝，不授部署信用。

PG工具与SQLx进程环境分开：新的B1私有固定工具配方在env_clear后仅给固定`LC_ALL=C`、`PGCONNECT_TIMEOUT=5`、`PGPASSFILE=/dev/null`、`PGSSLMODE=disable`，固定Unix host/port/user/database argv及`--no-password`；exec前对`/dev/null`以no-follow打开并实核char major1/minor3、具名身份稳定，拒绝其它passfile或继承环境。`--no-password`只禁止提示，并不禁止读取默认passfile，所以不能单靠该flag声称无credential-file IO；不设置password，不读取/root/.pgpass或任何现有凭证、不作口令/TCP fallback。原普通非B1工具路径不改。[PG18 pg_dump](https://www.postgresql.org/docs/18/app-pgdump.html)、[PG18 passfile](https://www.postgresql.org/docs/18/libpq-pgpass.html)

### 4.7 per-role固定路径与十二mount可达性

| 编译process role | 固定catalog及own manifest | 可实际测量范围 |
|---|---|---|
| publication-source | `/var/lib/knowweave-c4/b1/source-peer-execution.json`；同根`launch-manifest.json` | host自身、原Root父/launcher、实际固定host文件/FD；parent事实必须来自真实SO_PEERCRED/pidfd/ELF连接 |
| root-launcher | 同host catalog；同根`launcher-launch-manifest.json` | 自己固定Root ELF与Root producer资源；不由DB factory取得source ctx |
| capture | `/opt/knowweave-c4/b1-capture/source-peer-execution.json`；同release根`capture-launch-manifest.json` | 既有#9 RO release内自身manifest/ELF/libs及本PG namespace/native；不去打开host `/var/lib/.../b1` 或host source/launcher ELF |

C将同一keyless catalog按**完全相同canonical字节/SHA**复制到#9 RO release，当前RootSourceStart.catalog_sha再连接host和capture副本。两个副本inode可以不同，各自按本角色实际held metadata谓词核；不能要求跨namespace同inode。catalog不带key/trust/secret，仅公开指纹和已准入角色/根/运行事实。自己的manifest先于最后catalog生成，不反向包含catalog hash；不制造self-hash环。

factory按不可被caller覆盖的编译role取唯一自身row，再实测该row和自己的实际ELF；不得遍历并尝试打开其它角色不在本namespace的host路径。Capture对host角色的事实只按本次原Root producer绑定的记录/当前锁核，明确可信Root运维边界；不能伪称child独立打开了host父pidfd或实测host ELF。C若不能证明两副本同byte/hash、RO release和无RW替换，就拒绝，不通过添mount/key文件掩盖。

### 4.8 新ELF保持核验与scan实际收费的call graph

本节只规定新增B1Process/Root launcher代码身份对象，不降低任何既有record/tool/registry谓词。冻结`policy/installed.rs::Record::recheck`每次仍从held FD与当前named leaf完整read、hash、比较原bytes/full stamp；原generation proof/inspection再读、原PG工具hash、原registry/pin再核均原样保留。新ctx不能通过“已核过ELF”缓存替代这些旧检查。

新增`PinnedB1Code`只在本角色首次genuine factory中打开其实际自身ELF、确需独立实测的host父launcher及各自固定依赖；capture不打开host其它role。每一个被当前原进程测量的code file做初始完整有界read+hash，并持同一opened FD、完整`dev/ino/uid/gid/mode/nlink/len/mtime/ctime`纳秒stamp、原hash和固定named path。文件须root-owned、单link、不可由group/other写，新安装manager ELF固定0500；镜像内env wrapper/native/依赖文件按C固定manifest的真实准确mode核验，不能假定镜像系统工具也是0500。父路径从可信固定根no-follow遍历且不得group/other写。所有实际read和hash分别沿原scan语义实扣bytes，open/stat/祖先遍历逐项扣entries；不把hash CPU/IO记成0或事后相加报告。

新ctx在每个原关键门的`revalidate`对这些新code对象**不默认重新整读大ELF**，而是逐个检查held FD完整stamp、固定named no-follow重开完整stamp/同dev+ino、祖先权限/identity、当前`/proc/self/exe`/真实parent的same opened inode与原process PID/start、当前已准入映射/无额外可执行映射、Root job/lock/socket flags/cookie。file-backed executable mapping逐项连接实际已测CodeFile；kernel特殊vdso/vsyscall区域须在C实际映射闭包单列，不冒充可打开的普通ELF。未知匿名可执行映射或不能证明的映射拒绝。full stamp或任何named/mapping/process事实变化直接sticky拒绝，不重hash后换成新原件；这不是热更新入口。Root可信维护窗、固定不可写alias、无恶意Root/kernel/ptrace、固定实际ELF无dlopen/代码注入的原闭包是该选择的必要前提，不能声称抵御能原位写文件并伪造ctime的恶意Root。若C不能证明这套代码/alias/映射闭包，保持拒绝而不是弱化为仅inode。

需要读的小metadata/canonical catalog、launch manifest、SourceStart、CaptureStart、generation binding、proof/inspection、deny/hold/结算记录等每次仍在原cap内完整read/hash/原bytes比较。每次重复read/hash都再次收费；每次stat/maps/proc/FD遍历也实扣，没有按“同文件读过”退款。Root observer自己的code/sample费用在Root原job账本，不抵扣S/child的真实scan。

因此512MiB scan仍可能在初始自身+父ELF+依赖read/hash，或后续大量小metadata再核/原工具hash时耗尽；DB Observation64MiB和epoch journal的既有更小子限也保留。C阶段必须记录每角色实际文件大小、初始full-read/hash收费及正向operation预算轨迹和global64FD原始清单，真实不足即HOLD。没有对任意256MiB输入、任意依赖集合或1200s执行的必过保证；不扩大512MiB、不删真实收费，也不把未执行call graph当作可用性结果。


### 4.9 锁定SQLx构造观察：不可变启动环境与固定options

本节只窄修-4独审的constructor观察P2，不变四项已闭合设计、32叶范围、原owner/累计quota、freshness或C/D停止点。以随稿冻结且逐包匹配Cargo.lock checksum的SQLx0.8.6真实源码为准，不能用后设host/username声称撤销已发生观察。

| 启动键 | host publication-source | capture | 真实产生者/来源 |
|---|---|---|---|
| PATH、HOME、LANG | 固定系统PATH、/root、C | 完全相同 | Root launcher `.env_clear()`；Docker exec内固定已测`/usr/bin/env -i`清掉PID1继承环境后设置 |
| PGHOST | `/run/knowweave-c4-source-pg` | `/var/run/postgresql` | C已准入Catalog.peer_endpoint相应固定角色字段，非caller/wire/登录环境 |
| PGPORT | `5432` | `5432` | 同Catalog固定5432，只用这一个十进制表示 |
| PGUSER | `learning_admin` | `learning_admin` | Catalog固定admin_role；不是允许caller选择角色；broker/runtime options稍后由内部固定username setter选原两角色 |
| PGDATABASE | Catalog.installed_source.database准确字节 | 同一已准入database准确字节 | 原Root已准入唯一source group/DB，非命令参数；实际PG OID/SID/owner仍要再核 |

这七个键是Source/Capture SQLx进程的**完整启动闭集**。Root在创建child前按当前真实Catalog产生，不接caller环境/`PG*`覆盖；capture env-i argv顺序固定为`-i`、`PATH=...`、`HOME=/root`、`LANG=C`、`PGHOST=/var/run/postgresql`、`PGPORT=5432`、`PGUSER=learning_admin`、`PGDATABASE=<同已准入DB>`、最后固定capture ELF路径。PGDATABASE形状检查不授资格，必须再等于Catalog实际字段；Root source host环境同样逐值等于Catalog。

factory在任何PgConnectOptions创建前核实际完整环境的key集合/重复键/UTF-8、键值长度和上述准确值；仅此七个键被允许。缺失、空值、多个同键、值错或任何额外键直接sticky拒绝，不把“缺失”交给默认构造继续观察。尤其PGHOSTADDR、PGPASSWORD、PGPASSFILE、PGSERVICE、PGSERVICEFILE、PGOPTIONS、PGAPPNAME、PGSSLMODE、PGSSLROOTCERT、PGSSLCERT、PGSSLKEY、DATABASE_URL、TEST_*、LD_*等均不在闭集，不能借“合法PG变量”带入。秘密字段如果出现，只识别键并拒绝，不读取/记录其值；不能用先拷贝所有值的vars_os()/proc-environ读取伪称只读键。

准确gate放在同步入口、任何runtime线程/options/helper之前：按当前Linux进程真实environ逐entry只扫描key到首个`=`，最多七项，key超过闭集最长10字节或第八项即拒绝；仅key确认属于闭集后才按已知准确值长度+终止NUL有界比较该非秘密value，拒绝重复/缺项/多余字节，不拷贝禁止key后的value。原factory自己持该私有启动快照/角色/Catalog，不能由caller交入“checked env”。后续每次创建options前同样核当前完整七项与原准确字节并计费；这依赖已准入实际代码/所有依赖在整个进程内没有setenv/putenv/unsetenv等修改环境的固定闭包。C不能建立该闭包或观察到变化就拒绝，不在多线程中以unsafe更改环境去修复，不声称无约束外来线程同时改环境仍安全。Root launcher自身工具子进程仍按原各自env_clear配方，不把其PGPASSFILE白名单混进本Source/Capture进程；startup env只含前表非秘密固定事实。

每次私有factory要创建options时，先对本次实际完整环境有界遍历/比较及下列12个固定constructor键查询预扣同ctx的原scan entries/实际非秘密字节；键/值总量原cap内实际收费，不写事后报告抵扣，不给新quota。若含禁止秘密键，则仅收费已读key并拒绝，不调用constructor。source process静态一次attempt在所有fallible IO前burn保持；此后每次options创建仍重复核真实环境并按实际观察收费，计费不产生session资格。Root/child启动环境产生与实际进程复核的开销分别在各自原账本，不能按“同字节”省略真实再次读取。

固定构造路径只用`PgConnectOptions::new_without_pgpass()`，然后显式`.host(本角色固定Unix目录).port(5432).username(内部固定角色).database(已准入DB).ssl_mode(PgSslMode::Disable).application_name(固定B1名称)`；连接池只用已构造options的`connect_lazy_with`、原min_connections=0和本稿既有1/4/4 caps，避免`connect_with`先隐式acquire→release一个尚未经过原owner检查的连接。原第一次真实acquire后立即close_on_drop，再运行原实际SQL/ctx门；不以URL/FromStr/Default/new()/connect(url)重建options，不用`apply_pgpass`、password setter或caller factory/pool。broker/runtime的固定username setter不改变启动PGUSER，也不让caller提供新role。

锁定源码的准确可达分支如下：

| 源码位置 | 本固定配方下的真实观察/结果 |
|---|---|
| options/mod.rs56–69 | 查PGPORT、PGHOSTADDR、PGHOST、PGUSER、PGDATABASE；PGHOSTADDR确定缺失而PGHOST准确存在，**不进入default_host**的三个Path.exists；PGUSER准确存在，**不调用whoami::username/NSS用户查找**。没有声称后设setter修复早先默认观察 |
| options/mod.rs76–92 | 另查PGPASSWORD、PGSSLROOTCERT、PGSSLCERT、PGSSLKEY、PGSSLMODE、PGAPPNAME、PGOPTIONS，共12个固定env键；除前表允许项外均确定缺失。CertificateInput::from不被执行，password/cert/key/options为None，PGSSLMODE缺失只得到Prefer枚举，随后在任何连接前显式改Disable。其余statement cache/extra_float_digits/LogSettings默认是内存常量，LogSettings::default由同锁定sqlx-core connection.rs核实，不做HOME/TLS/DNS观察 |
| options/mod.rs97–105及options/pgpass.rs | apply_pgpass及home::home_dir/PGPASSFILE/默认.pgpass加载没有被新构造路径调用；HOME=/root是固定启动值，不因它存在就声称读取了/root/.pgpass，也不允许后续换new()/Default触发 |
| options/mod.rs::fetch_socket、connection/stream.rs及锁定sqlx-core net/socket/mod.rs::connect_uds | 固定host以绝对`/`开头、port5432，fetch_socket只构造唯一目录下`.s.PGSQL.5432`，走UnixStream::connect；该分支不是connect_tcp，不做DNS/default-host候选搜索，无TCP失败后重试路径 |
| connection/tls.rs::maybe_upgrade | 最终options的Disable分支在TLS available/upgrade/证书链之前直接返回，故不加载HOME默认TLS证书、Root CA/client key，不作TLS/DNS/证书路径探测；不靠“Unix就自然不TLS”的猜测 |
| connection/establish.rs | 仍是真实受支持SQLx协议；若错误服务端发口令认证请求，SQLx在password=None时可能发送空认证响应或失败，并没有可宣称的peer-only handshake开关。这里不提供任何秘密/密码值或凭证文件fallback；实际连接后system_user必须严格peer:root，否则原资格门拒绝。C必须另证固定PG配置真实peer身份，不把无秘密输入夸成客户端在所有错误配置下从不发任何认证帧 |

上述只界定受选constructor/固定连接路径，不宣称整个OS/动态链接器/runtime没有其它syscall。当前源码级排除了这条路径的默认socket/NSS用户/HOME passfile/TLS/DNS探测；实际库/环境/FD/SQL轨迹仍须以后限定检查及C真实安装核实，不在本轮执行，不扩大旧scan、Observation、journal限额。

同轮owner文字桥：原EpochWork正常Drop按冻结history.rs759–767仅毒化它自己的Original/slots；它不是持久journal写入，也不能反向使已完成capture的host ctx永久HOLD或撤回原D复制。异常ctx失效仍sticky。此约束保持原C/已采纳R1分账，并非新增权限或改变结算协议。


## 5. 最小拟改叶表与批准分层

下表给出可交审的**候选最小修改表**。不是允许实施全部内容的批准。尤其B0/B1本稿方案尚未独审，不能将表直接当已独审最终施工单。

| 叶 | 准确职责 |
|---|---|
| `learning-backup/Cargo.toml` | offhost-b1显式依赖新DB feature；新增capture及Root source-launcher两个required-feature bin，无新第三方库 |
| `src/bin/knowweave-c4-b1-capture.rs`、`...-source-launcher.rs`（新） | 固定无参数入口；不能接任意manager argv/path/DSN/command；缺真实安装拒绝 |
| `offhost_b1/{source_launch,capture_child,capture_job,capture_export}.rs`（新）及`mod.rs` | 固定Root父子/exec协议、两immutable generation、child原cut owner、仅数据export；不开通通用RPC/签名代理 |
| `offhost_b1/source_actor.rs`（原表拟新增） | preflight holder→lease作用域drop、首次host S原SQL/registry/保护、Ready后明确childsettlement再final |
| `source/admission.rs`、`source.rs`、`source/binding.rs` | 私有B1原会话/ctx分支及固定原controller连接；普通旧谓词不改；不从child/data/file制造权限 |
| `source/endpoint.rs` | 新production capability配方只在真实Root-issued/native核验后接受；旧七mount/cfg(test)规则不放宽 |
| `source/snapshot_effects.rs`、`.../cutover.rs` | 私有B1固定install接缝、十二mount原件、两阶段immutableconfig选择、原cut只留child；正常原API不变 |
| `destination/snapshot_effects.rs`（原表已列） | 同原D authenticated remote data消费者、原copy/readback/副proof；不构造CapturedSnapshotEffectsV2 |
| `learning-db/Cargo.toml`及`Cargo.lock`（超原准确表） | 默认关闭b1-source-peer；复用workspace sha2可选直接依赖，Cargo.lock仅learning-db节点新增sha2依赖边，不增/升/改第三方package版本或checksum |
| `learning-db/src/snapshot_broker/{mod,lifecycle,history,writer,transition}.rs`（超原表） | 窄public opaque类型/新install入口、原Arc ctx/共享budget/terminal传播及实际session检查；旧None路径原谓词 |
| `learning-db/src/snapshot_broker/b1_peer.rs`、`b1_peer/{process,job,session,tests}.rs`（新、超原表） | 固定无参数factory、实际Root ELF/launcher/job/socket核验、不可反序列化ctx、拒绝与元数据原语测试；未具真实安装保持拒绝 |

| `offhost_b1/mod.rs`、`policy.rs`及原source bin同步入口 | 新manager编译pin族、真实B1角色匹配及最早FD3/CLOEXEC屏障；旧application pin族/旧registry谓词不改；无新增目标执行权限 |
| `learning-backup/src/registry.rs`（已在R2b表，但新增明确B1计费职责） | 私有open_installed_b1(ctx)从第一Root遍历/registry IO使用genuine ctx原scan backing，之后复用原try_lock；每open仍新Arc wrapper，旧operation identity规则不放宽 |
| `learning-assets/src/root_journal.rs`（新增超原表单叶） | 仅新增shares_meter_with的Arc身份比较；不放宽limits，不返回内层Meter/reset；DB B1 install用它拒绝scan与journal误共享，不据其值发权限 |

不建议把Root installer认证泛化成新框架或新crate。`InstalledSnapshotBinding::shared_budget()`已经存在，故无需新增custody getter或改assets其它叶；也不改source/snapshot_effects/files.rs的双ledger计费。任何后续发现需要的新叶仍先明确，不自动扩大。

批准分三层，不合并为一次现场授权：

1. **编码补充（独审后才可请求）**：先独审本稿B0/B1机器schema/call graph，通过后请用户批准上述准确差异及A/C明确所有者语义、候选Root资源上限、普通Linux可做的字段/状态/预算/FS拒绝测试。没有Root/PG正向条件的case明确未执行。
2. **C安装（另批）**：实际公钥/ELF/Root manifest指纹、Root launcher和有限调用权限、active-job根/锁及RO子mount、peer maps/PG/socket alias/Root broker权限、十二mount及无旧writer闭包、真实容量/限额/停机/回退；不请求或存储登录密码。
3. **D实际整链（另批）**：独立目标原D首发/D1+RA、源不可访问后的D2/RB、原件故障矩阵和独审；旧失败/HOLD/已签历史仍保留。没有独立安装/真实证明时不得用单机夹具替代Task9验收。

本稿没有修改R1关于SourceFinal、D Pending→Consuming、迟到取消、自然完成/历史读取或永久fence的语义。没有运行任何新产品、Rust、Python业务测试、服务器命令或读取密钥。A/B/C建议不因文档写出而自动采纳；剩余前置点是本准确候选的独立设计审查、用户补充编码批准及之后的真实实现/限定检查。C安装、跨机首发和恢复不在此编码批准内。


## 四项独审的准确修订桥

- P2-1：固定image_ref/image_digest与C真实准入config ImageID分列；当前container.Image==imageInspect.Id==已准入ID，RepoDigests连接固定digest，CaptureStart再核同值。绝不要求configID等于RepoDigest。
- P2-2：删除caller PgConnection/三PgPool，选factory自建owned session/三pool；增加session叶与明确close/settlement、tool env/passfile和每角色可达路径，SQLx无底层FD不作伪证明。
- P2-3：实际编译输入拆为旧application族和新manager族；新B1 registry桥保留原app验证后joingenuine manager ctx，缺失/错族拒绝，不只分列JSON。
- P2-4：Root唯一FD3 handoff后S最早恢复CLOEXEC，后续exec只stdio，Root权限FD不继承；同步/动态库/错误管道边界明确，无“Command天然关闭”假设。

Cargo.lock机械边随P2-2真实新dep闭包列入；源码基线仍是独立冻结ccf71a5，后续Task2a发布不被本稿改写。Root capture候选上限一直为1200s，独审主文末段曾写1220的文字差异按e12c补件纠正，本稿不把它作为额外20s。全部仍待独审/用户新增编码批准，C安装/真实运行另批。


-5准确差异：仅修c144独审的constructor观察合同，4.9列锁定源码可达路径、固定七键Root来源/角色、同quota费用及默认路径不触发的条件；-4原四P2已在159b独审闭合，32叶/原owner及C/D批准层次不变。正常EpochWork drop不反向污染host ctx的文字桥保持原结算语义。所有新增实现/检查/安装仍未获本私有稿授予。
