# Task9 C0：Capture 十五项挂载与负约束元数据增量合同

状态：`CONTRACT_PROPOSAL_NOT_APPROVED_NOT_IMPLEMENTED`。本件是原 Task9 安装准备的有限修订草案，不是产品输入、编码批准、安装许可或 Task9 验收。当前用户只要求起草、独立审查与文档同步。代码基线为 `0531083cc7b686ee30a0df7b4c9cdae3aa84b574`，tree 为 `d85a6faddb141e5aac1bad5e9b8bc52cd272124f`，708 个公共文件。原 37 项矩阵、53 项分层义务、原批准记录及旧 FAIL/HOLD 保持原字节。

本合同的准确机器表与检查计划分别为 [机器增量表](capture-mount-amendment-machine-delta.json)和[分层检查计划](capture-mount-amendment-validation-plan.json)。它们描述未来工作，不表示已执行。代码现有 13 项普通 Linux 检查通过，只证明那一组受测字节的普通格式、Clippy、编译和回归；本 C0 代码尚未编写，真实 C 安装、产品自有 FD 上限及必要 runtime/SSH 清单与实际总数、S5、首签、RA/RB 与 Task9 正式验收均未完成。

## 1. 问题、唯一方案与权限界限

Capture 真实安装链在 `source/snapshot_effects.rs:223` 调用负向 mutation gate；其后 Enrollment、Q0、prepare、protection 的负门仍会调用同一路径。当前 loader 固定读取 `/var/lib/knowweave-c4/b1/source-policy.json`、同目录 `launch-manifest.json` 和真实 fence anchor。现有精确十二项挂载没有这两个文件视图或 fence 根，所以当前拒绝是正确的。

不能直接把宿主 PublicationSource 的两份文件挂入 Capture：`policy.rs:448–449,487–488` 必须核本 Capture 编译的 manager commit/build 与实际 Capture ELF，原 Catalog 每个 role 的 build 可以不同。不能共用错误 pin、跳过比较、制造 PublicationSource 身份或用新 DTO 代替原 native/peer/SQL 准入。

本草案只选定以下方案，不保留运行时 a/b 选择：

1. 在原 keyless Capture release 内，未来由 C 发行一份 **Capture 专用** `capture-source-policy.json`。它使用现有 SourcePolicy 的准确字段，manager build/release 对应本 Capture，不对应宿主 PublicationSource。以 `R` 表示原 C 已准入、Root 当前实测 destination 为 `/opt/knowweave-c4/b1-capture` 的唯一 RO bind 的实际宿主 `Source`；新 policy 的宿主路径唯一为 `R/capture-source-policy.json`，不是 caller 传入目录。
2. 新增两个精确只读文件 bind：该 policy 到 loader 原固定 policy 名；原 Capture own `capture-launch-manifest.json` 到 loader 原固定 launch 名。launch 只是同一原件的第二个只读名字，不能复制生成另一 inode。
3. 新增真实宿主 fence 根的只读 bind。只允许读取原 namespace、检查 anchor/held identity 并在原 lock 上做非阻塞 flock；不写 fence，不复制 fence，不形成首次 S。

Capture 的真实进程角色仍为 `capture`；DB factory 仍测量 Catalog 的 `Capture` 行、自身 ELF/native 与原 Root job。SourcePolicy 的 JSON `role:"source"` 是既有记录 schema，不是 `publication-source` 的进程资格。Capture 不打开 Source-auth、transport-key、known-hosts、verifier trust 或 D 私钥；policy 中的固定非秘密路径和公开指纹不允许转化为读这些文件的权限。

## 2. 唯一十五项拓扑

下表按 destination 字节序排列。原十二项的 destination/type/RW、实际 source/name 绑定和身份谓词全部保留。新增的三项只读；不增加父目录 bind，不改变原 volume 为 bind，不放宽为至少十二项。

| destination | Type | RW | 新增项的准确宿主 source |
|---|---|---|---|
| `/opt/knowweave-c4/b1-capture` | bind | false | 原项，原 C 准入映射不变 |
| `/run/kwc4/sfx` | bind | true | 原项，原映射不变 |
| `/usr/bin/docker` | bind | false | 原项，原映射不变 |
| `/var/lib/knowweave-c4-source-fences` | bind | false | `/var/lib/knowweave-c4-source-fences` |
| `/var/lib/knowweave-c4/b1-source-observation` | bind | false | 原项，原映射不变 |
| `/var/lib/knowweave-c4/b1/launch-manifest.json` | bind | false | `R/capture-launch-manifest.json` |
| `/var/lib/knowweave-c4/b1/source-policy.json` | bind | false | `R/capture-source-policy.json` |
| `/var/lib/knowweave-c4/registry` | volume | true | 原项，原映射不变 |
| `/var/lib/knowweave-c4/snapshot-effects-v2` | volume | true | 原项，原映射不变 |
| `/var/lib/knowweave-c4/snapshot-export-v2` | volume | true | 原项，原映射不变 |
| `/var/lib/knowweave-source` | volume | true | 原项，原映射不变 |
| `/var/lib/knowweave-source/assets` | bind | false | 原项，原映射不变 |
| `/var/lib/postgresql` | volume | true | 原项，原映射不变 |
| `/var/run/docker.sock` | bind | false | 原项，原映射不变 |
| `/var/run/postgresql` | volume | true | 原项，原映射不变 |

“十五项”指 Docker 当前 `Mounts` 投影的精确闭集，不是说 `/proc/*/mountinfo` 总共只有十五行。`R` 必须来自原同一 Root/Catalog/runtime 的唯一已准入 release bind，保持原十二项的实际 Source 不动；`R` 通过持有 root FD、祖先和新两个 leaf 的相对 open及具名重核连接。容器中的固定 `/opt/...` 路径不被误当宿主 Source；即使新安装预定宿主同名路径，也须 C 另单实例化并实核。mountinfo 的十五个目标各出现一次，RW/RO 对应表值；原镜像必要的 `/`、proc/sys/dev 挂载不因此被误计成业务挂载。已列目标下的额外 mount 拒绝；新增文件的父目标 `/var`、`/var/lib`、`/var/lib/knowweave-c4`、`/var/lib/knowweave-c4/b1` 不得被额外挂载遮盖。`/` 的原镜像 rootfs 不在该父目标拒绝表。原 assets 精确子挂载是原十二项本身，不成为通配子目录例外。

容器镜像中的 `/var/lib/knowweave-c4/b1` 须是 root:root 0700 的真实目录，不挂入宿主整个 b1 目录。祖先逐组件 no-follow，root-owned 且 group/other 不可写；文件是 root-owned 普通单 link 0600；policy 最多 16 KiB，原 Capture own launch 与新增 DB launch mirror 均沿原更小 4096 B cap。Task1b 通用 launch reader 的 16 KiB cap 保持原样，但不能覆盖原 own launch/新增 mirror 的 4096 B 门。fence 根及 operations/staging 保持 root-owned 0700，anchor 最多 4 KiB，lock 保持原空普通单 link 0600。所有最终具名对象、持有 FD 与原完整 stamp/原 bytes 再核；不能以“RO bind”代替这些检查。

## 3. 元数据字段与同原件连接

### 3.1 Capture 专用 SourcePolicy

不增加、删除或改名 SourcePolicy、FenceAnchor、LaunchManifest、ExecutionCatalog、CaptureStart、C11 record 的字段。机器表逐字段给出发行约束。Capture policy 的 `manager_build_input_sha256` 等于本 Capture 编译 manager build；`manager_release_sha256` 等于实际 Capture ELF SHA256，亦等于同一 Capture launch 的 `binary_sha256`。它们不必等于 PublicationSource 的两个值。

其余 SourcePolicy 数据必须与已经单独准入的同一 Source 部署相符：deployment/group、registry 根/lock/authority、source-control binding、两类公开 key pins、fence anchor、connection、transport 非秘密配置及 launch_id。`source_connection.socket_dir` 仍是 schema 中的宿主逻辑 alias `/run/knowweave-c4-source-pg`；它只是此负元数据的 DATA，不改变 Capture 原 SQLx endpoint `/var/run/postgresql` 或 role。不得为让 Capture parser 通过而修改 connection predicate。

### 3.2 同一个 Capture own LaunchManifest

DB 的 `job::LaunchManifest`（`job.rs:2910–2951`）、Root 复制 schema（`source_launch.rs:4320`）与 Task1b `policy::LaunchManifest`（`policy.rs:274–289`）均为相同十四字段，JSON role 均为 `source`。可复用同一原件，不需要新消息、记录类型或新 parser 宽容分支。

原 Capture manifest 与新 loader launch 别名必须是同一个宿主文件 bind 到两个名字：Linux 同一 `dev/ino`，完整 `uid/gid/mode/nlink/len/mtime/ctime` stamp 相同，完整 canonical raw 和 SHA 相同；不允许“相同内容的新副本”。原 Catalog 的 Capture RoleImage `launch_manifest.bytes/sha256`、本 Capture 编译 commit/build、实际 own ELF 和 RootLauncher 实测引用继续严格连接。policy SHA 是新 Capture policy 原 bytes 的 SHA；anchor SHA 是原真实共享 fence anchor；common launch_id 与原 Catalog、SourceStart、CaptureStart、job/live lock 相连。PublicSource 和 RootLauncher 的 own manifest 仍各是自己的行，不用 Capture manifest 替代。

新 policy 在 release 路径与 loader 路径也必须为同一原件别名；policy SHA 必须由原 Capture manifest 绑定。Root 的两条真实 full-host 观察路径必须在 Install before、真实 genesis 后的完整 after/fullpostobs，以及后续每个原 critical recheck 中持有并比较 R 下 policy 原 leaf 与 Capture 根视图中 loader policy leaf 的两个 FD：同 dev/ino、完整 stamp、完整 raw/hash，且原 bytes SHA 等于已持 Capture own manifest.policy_sha256；同样核 launch 双别名和真实 fence root。对象先存原 Root job 的固定私有 custody，再做后续 fallible 工作；每次完整读/比较及临时/持有 FD 均在原 Root 账本先付，具名重开不得接纳新 inode。不能仅在 C 发行文字中写“相同”，不能只比较路径字符串。DB 新增持有镜像仅为 launch，不扩成 policy raw 资格或改 job schema；Task1b 原 loader 自己继续持有、完整再读 policy/anchor/launch。所有 alias、父目录、RO mount 和内容核验不接 caller 指纹、容器 ID、路径或“已验证”布尔。具体 DB 接缝是在 `process.rs` 的原 `OriginalState` 增加 Capture-only 的私有 held launch mirror：固定读取新 `/var/lib/knowweave-c4/b1/launch-manifest.json`，沿原 own launch 的 4096 B 上限、使用现有 `job::LaunchManifest` schema，与该 state 已持的原 own manifest 逐项比较完整 stamp/dev/ino、raw/hash，并在原 `revalidate_bounded` 每次持续核；PublicationSource 分支没有此字段的正准入用途，也不走 Capture alias。新增对象先入原 acquisition/Book 固定 storage、再做后续 fallible 工作。Task1b 原 loader 随后的完整 SourcePolicy 校验仍负责把 policy raw SHA 连到同一 launch 的 `policy_sha256`。DB 新 held-view 只增加私有负约束资源保管，不可构造 SourceAdmission、Original S、签名资格或第二个 peer。

### 3.3 release、发行顺序与 hash 环

现实现只固定读取 Capture ELF、own manifest、catalog 和其 Catalog 已列库文件，没有对整个 release 目录实现“只有三项”的枚举证明；job 的独占目录 roster 检查是另一个目录，不能混称 release roster 已闭合。

本增量把准确 metadata 叶集合从 `{capture-launch-manifest.json,source-peer-execution.json}` 增为这两个名字加 `capture-source-policy.json`。Capture ELF 名保持 `knowweave-c4-b1-capture`，依赖仍只来自同一 Catalog 的原 Capture `libraries` 闭集及原 32 项上限；不增加依赖、不把 policy 塞入仅允许原记录名的 `RecordRef` 枚举。新 policy 由已有 manifest 的 `policy_sha256` 绑定。现场 C 必须给出实际完整包清单和计数，拒绝未申报文件，不能把以上三个 metadata 名误报为已采样的整个安装目录。

未来 C 发行顺序固定为：公开身份与 build-input 准入→各角色 ELF 构建与实际测量→原真实 fence anchor→分别发行 PublicationSource 与 Capture policy→分别发行各自 own manifest→最后发行包含各 manifest SHA、十五行 recipe SHA 与实际安装事实的 Catalog→当前 Root job/start/generation/C11。policy 不引用其后的 manifest/Catalog SHA；manifest 不含自身/Catalog SHA；不以最终 ELF hash 反向构造其编译输入。确切 bytes、数量及安装权限必须在未来 C 包中另审，本草案不生成任何实例。

## 4. recipe 版本、前像与两端门

元数据层的版本名称是 `task9-b1-capture-ro-negative-15-v1`，只标识本修订的固定代码/安装配方；它不是新产品 JSON 字段、环境变量、CLI 参数、wire kind、purpose 枚举或 caller selector。未来经批准的新 Source/Root/Capture 构建在原 `offhost-b1` 固定角色分支只接受十五项。无 feature、无原编译 pins 的默认拒绝保持。旧 B1 十二项安装在新构建下必须拒绝；旧构建遇十五项也继续拒绝。不能自动回退、热升级原 job 或重发旧 generation。非 B1 普通旧路径不被此次改为十五项。

Hmount 仍为对实际十五行按 destination 排序后，以既有 canonical JSON 计算的 SHA256；每行准确键为 `destination,source,kind,rw,name`，bind 的缺省/null name 只在此五字段前像中规范成空字符串。volume 的准确实际 name/source 不能换成模板。完整 Docker assets 对象仍保留原缺省/存在语义，before/after 仍逐字/typed 原谓词比较；不得因 Hmount 规范化而丢弃它们。Hmount 不加入 profile 名、wrapper、未知字段或 Hexec 数据。Hexec 的原参数/env 前像不变。

同一新 Hmount 必须贯通：Catalog.capture_recipe_sha256、installed_runtime.capture_mount_recipe_sha256、原 CaptureStart.mount_recipe_sha256、Root before/after/fullpostobs、C11 record 与 DB 消费、backup S5 原数据消费；任何一处不同就 HOLD。没有真实 source/name 时不能计算并发布所谓实际 recipe hash。

新增 RO destination 不证明宿主来源不可写。Root 的 before、after/fullpostobs 与原 critical recheck 必须在完整 all-host 容器集合中，对三项真实来源（R 下两个固定文件与真实 fence 根）拒绝任何容器 Mounts 中的 RW source、可写祖先或重叠子树/物理别名；同一 PG 的其它 RW mount 也不能豁免。既有 pure check_projection 只保护 companion/export 的 source 比较不足，未来 source_launch.rs 要明确补齐这三项谓词；不得缩成只查本项目或只查字符串相等。Root 通过已持源 root/leaf FD、完整 no-follow 父链与实际对象身份核物理重叠/别名，重复读/stat/枚举先扣原预算，未知或不能证明直接 HOLD，不自动停止别的项目。完整已认证 Mounts DATA 和十五行 Hmount 仍用原字段传给 DB；DB 只核该完整 DATA 的形状、来源公式、顺序与原哈希，不谎称在 Capture 中实测宿主物理 alias，不新增 wire 字段。C 还须排除非 Docker 的 host 发行/改配、旧 writer、在途写者和其它可写别名；可信 Root 的维护窗边界不被 RO bind 替代，也不声称抵御恶意 Root/kernel。

必须同步的四个消费叶在机器表闭列：Root 的 C11 pure projection 及两条真实 full-host 观察路径、Capture mountinfo、DB factory/revalidate 的 Capture RO/alias 检查与 C11 consumption、backup binding 的十五行及 recipe 重算。Root postobs 仍在真实 genesis 后、持久观察 record 和 BrokerInstallObserved 确认后，才允许首次 work。仍是十五种 Root 控制消息、十三个主动作；不添加第十六种消息，不加 C11 字段、不挪观察到提前的 caller DATA。

## 5. 原持有者、锁、预算与失败

registry 原 exclusive lease → fence 原独立 lock 的顺序保持。不能由宿主一直持 registry 锁“替 Capture 兜底”，那会与 Capture 自己的真实 lease 冲突。原 Enrollment lease 只在 genuine Q0 自然成功、真实 cut 存入原 Book并重核后释放；Capture 的 prepare lease 按原自然路径释放，随后首次 S 才创建 fresh wrapper/lease，共享原累计 scan backing。不得为了本增量改变这些顺序、new Once、结算门或源码中 Capture/Publication getter 拒绝。

新增 held FD、named reopen、目录遍历、metadata read/hash、canonical 分配/比较及所有错误路径，必须在工作前向各自原 scan/journal/Observation 账本精确付费，不使用 default budget、不退款、不换 wrapper、不以 policy 长度支付更大 Catalog 或 mountinfo 工作。仍为 scan 512 MiB/100000 项、Observation 64 MiB、Root/container 原 1024 上限、C11 record 1 MiB、Root frame 4096 B/累计 262400 B，已有更小 cap 优先。新增 policy 16 KiB、原 own launch 和新增 DB mirror 各 4096 B、anchor 4 KiB；Task1b 通用 16 KiB launch reader 不变，不提高任何限额。

新增资源必须先进入原 Book 固定私有 storage 再做后续 fallible 工作；重核要持原 FD并核具名对象，失败为 sticky negative。原 query、task、child 已开始时必须自然结算，不能 kill-on-drop、abort、Drop未知原 owner或用 timeout 销毁原 future 后声称已停。首次 S 1800 秒及 Root 更早 deadline、已有 1200/120/30 秒门不重开。不能从 retained 文件恢复第二个原操作。

RO 文件 bind 能被当前 no-follow openat reader 打开，在源码层没有 NO_XDEV 阻断；这只是静态可行方向。实际 bind 后 inode/parent stamp、RO flock 的 kernel/filesystem 语义、产品自有打开 FD ≤64、必要 runtime/SSH 句柄的准确清单与实际总数、原预算与 S5 首 work 都必须在未来 C 实测。原 26 计划第 5 节第 75 行的 FD 预算域保持：必要 runtime/SSH 句柄另列真实归属、数量、是否与产品持有 FD 重叠、各自生命周期及实际总数；不能把产品的文件/套接字/pipe/child/AsyncFd 包装重命名为 extra 来豁免计数。原实现已有更严格的实际全表局部门及固定余量检查保留，不因此放宽。局部计数、简单分类表或普通 parser pass 不成为全期实测证明。任何不能证实项保持 HOLD；不提高 RLIMIT、不加 capability、privileged、host-proc、host PID namespace或可写 fence。

永久 fence 仍没有 close/TTL/GC；真实首签后可能长期阻止此源后续备份和共享 registry/global generation 管理变更，也可能影响其它组。本合同不批准解除。失败资源、原 FAIL/HOLD、partial 与证据保留，不重跑旧批次；异常处理必须独立具体审查。

## 6. 准确未来文件范围与批准顺序

未来 C0 编码请求仅列五个现有 Rust 文件（四个产品消费叶、一个追加 DB 测试叶），详见机器表，五个均已在原 Source32 路径表内。Capture policy 的纯 parser 测试放在 `source_launch.rs` 内新增的局部 `cfg(test)` 模块，调用父域已有可见的 `parse_source_inputs`；不改 parser 可见性，不打开 `policy::tests` 的私有 fixture，不额外修改 `policy/tests.rs`。 普通测试只核现有 parser 的形状、canonical、跨记录一致性和编译 pin 门；合法但不同的 v4 group 值不因 parser 本身而获得或失去 current group 资格。真实 deployment/current registry/current SourceGroup/Catalog/admission 的语义匹配仍由原 held loader 与真实准入闭包负责，在 C0-C-03 等现场义务完整收证，不能用坏哈希或非 canonical 字节的拒绝冒充该证明。没有新增/删除源文件，没有改 Cargo 依赖、bin、policy.rs、policy/installed.rs、source_fence.rs、registry.rs、source/admission.rs、Root wire schema、原 37/53 JSON 或原批准合同。不能把本合同理解成整个原 37 文件可以任意修改。

`source_launch.rs` 中已有十二项 DATA fixture helper可在该叶内明确升级为十五项 fixture；旧测试函数的正文与属性不改，新增旧十二项拒绝用例。旧 fixture 并非已安装十二项证明，部分行只为此前局部 predicate 准备；新 fixture 必须按固定真实 tuple 生成 kind/RW/派生 source，不能把旧 fixture 的简化值当作实际挂载的兼容例外。所有来源中的数字 12 不能全局替换：captured-source-evidence 的十二份原文件、ProofRecord、包 manifest 与 fresh S/full snapshot 的原语义全部不变。

未来打包器、部署脚本与产品 CI 的改动允许集合均为空。四角色现场 issuer/installer不存在已获本合同批准的新脚本。若实际实现还需要本闭集以外的源/test/package/deploy 叶，先补准确合同与独审，不能以“相关文件”自扩。当前三件文档发布可原样使用机器表的三个公共文件名。

批准与实施顺序：

1. **现在**：只完成这三件非执行草案、独立审查和文档同步；不给 C0 编码或现场许可。
2. **另需最小 C0 编码批准**：确认五文件准确范围、固定十五项语义及普通非特权 Linux 检查。私有候选实现→逐叶独审→新批次精确源包/控制器准入→格式、Clippy、编译、普通测试及原始证据独核。不得继承旧构建通过结论。
3. **之后另需 C 现场批准**：明确两主机、全新项目/卷/子网/身份、实际文件清单/hash/mode/owner、两个新文件 bind、真实 fence RO bind、新私有镜像父目录、当前 Catalog/每角色 manifest/policy 核发、原 UID/PG peer map和 launcher。重建会改变 Source manager ELF，D 对 Source manager 的已有 policy pin若需重发也必须在该实际安装包逐文件列清。当前不安装 key/trust、改 HBA/ACL/sudo/SSH、启用服务或重建既有 PG 容器。现场安装还须冻结完整 all-host 与非 Docker source-writer 排除清单，所有不能确认的来源别名或在途改配保持 HOLD。
4. **C 通过后才申请真实整链/故障注入**：先做限定 native/C11/S5、产品自有64FD与必要 runtime/SSH 清单/实际总数、预算验证，再作真实独立 D 首签与新发布；RA 全恢复和源端不可用 RB 仍分别批准精确隔离与破坏性目标。37/53 按原 ID分层收证，不合并为本 C0 的新通过数。

本草案完成与独审通过均不代表 C0 实现完成、真实安装可直接进行或 Task9/P0-C4 已完成。
