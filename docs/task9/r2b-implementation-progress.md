# Task9 R2b实施进展

## 当前进展 · 2026-11-02：CF2 限定普通25门及20项补验通过，正式Task9仍OPEN

受测候选为source index `24282f8a`、CODE gate `22bcb1dd`、Delta `b60dc254`。它保持原五个既有Rust叶范围、默认关闭与所有权限边界，旧109及新20测试的属性和正文均原字节保持。此前两轮Clippy失败分别对应两处冗余表达式与私有函数8参数：已仅移除冗余借用/解引用，并将同原ProcView和File只读借用组合成一个tuple参数；没有新增协议字段、依赖、authority入口或放宽检查。

CF2以全新7f109批次完成13阶段编译与列举（原结果 `bcd7005b`、原件独核 `9638f408`），八份完整名单和既有ignored集合重新冻结并独核（`4fbf8ff6`／`e1c373f1`）。随后全新15c7批次的25阶段全部退出0，包含自身fmt-check、四配置严格Clippy、类型检查、库测试编译、完整列举和整库普通用例。实际结果 `50212fc5`、证据包 `1fcc9574`、原件独核 `c2ce628c`，独审报告 `7da793b3` 已核完整names、源码前后、固定资源身份和自然停机清理。

| 普通配置 | 通过 | 既有忽略 | filtered |
| --- | ---: | ---: | ---: |
| Backup默认 | 450 | 106 | 0 |
| Backup offhost | 663 | 106 | 0 |
| DB默认 | 62 | 24 | 0 |
| DB peer | 129 | 26 | 0 |

新增20个普通定义逐名通过，四配置归属为0/14/0/6；跨配置重复正文不相加。这些是限定库普通检查，不能替代原37/53现场义务。受测CF2输入集与出版711文件集分开记录：出版只允许10份当前文档、一个现存生成器和manifest共12项文档/ledger差异，其余699份文件须与CF2逐raw相同，包括403份Rust及11份Cargo/lock文件。文档变更采用独立生成与结构QA，不宣称整个最新出版711都在该批次编译。

旧PF格式原件/有限语义桥、CF1编译及旧名单保留原范围；8ace和311b两次失败都未运行用例，原件与所有AUTH/SCP/SFTP/HOLD保持，不重放、不转授本轮通过信用。53个旧检查点保留深层值与JSON类型，仅新增第54检查点；[原合同](capture-mount-amendment-contract.md)及三件合同原文保留历史含义，不追溯改为安装批准。

当前完成的是默认关闭代码与普通补验。真实C0-C02原十二项namespace兼容、三项RO挂载/ROflock、来源写面排除、产品自有FD≤64与必要runtime/SSH全期owner/重叠/实际总数、Root/PG/C11/S5、B/C安装及密钥/信任/launcher、首次跨机签名和RA/RB仍待对应范围闭合。没有安装或安全权限变更、源隔离或恢复，Task9/C4继续OPEN，main与生产未改。

---

以下保留此前完整历史记录；当前普通检查事实以上述2026-11-02段为准，各历史原件保留自己的源码与验收范围。

## 当前进展 · 2026-11-01：Capture 代码静审已过，首轮控制台认证失败并保留 HOLD

准确五叶默认关闭代码的组合静审仍为 `PASS_C0_EXACT_FIVE_SOURCE_CODE_NOT_RUNTIME_NOT_TASK9`，CODE gate `a009984a1760bfa4d46f1c761de041962647390027740b74d2b49dcb1adf91f0`，独审 `9d32925343f808d0439d8b05af114d46279a32b028587c747a119661b7976f5c`。原109个测试的全部属性与正文保留，新增20个普通定义按Backup默认/offhost、DB默认/peer映射为0/14/0/6。本候选尚未运行 rustfmt、类型编译、Clippy或测试；这些静态定义不计作通过数量。

已准备用户真实控制台运输。首轮仅在第一次SSH创建目录连接的认证阶段退出255；未观察到后续上传、LAUNCHED、runner执行或证据采集。用户随后遇到的 `FileExistsError` 与既有本地独占尝试标记、防重放门一致，不是Rust编译失败。认证拒绝的根因尚未确定；原批次与证据封为HOLD，未清除、绕过或重放，也未新增凭证或权限。

下一步先由用户通过正常标准SSH客户端只读验证登录，再在正常访问确认后建立全新精确绑定批次；不重跑原批。密码只由标准客户端直接在真实控制台交互，Python不读取、传递、缓存或记录；常规运输设计四次连接，不以此保证认证一定成功。后续独立format、四配置compile/list和全新25门普通检查目前均无执行信用。

用户“继续推进”只承接前述五叶编码和原限定普通Linux检查，不追溯修改[原合同](capture-mount-amendment-contract.md)的历史审批记录。原37/53、原合同三件、52个历史检查点和所有旧FAIL/HOLD保持。新十五项配方不回退原十二项，真实C0-C02命名空间、三项RO挂载/flock、来源写面排除、产品自有FD≤64及必要runtime/SSH所有权/重叠/实际总数、S5、Root/PG/C11、keys/trust/安装、D1首签与RA、D2源不可访问与RB、Task9/C4继续OPEN；main与生产未变。

---

以下完整保留此前检查点，均为当时的历史记录；先前“准备控制台后执行”的表述应以上述AUTH255/HOLD为当前事实，旧编码审批、源码或测试口径不转授本候选。

## 当前进展 · 2026-11-01：Capture 五叶代码静审通过，普通 Linux 检查尚未运行

用户在前述最小下一步后要求继续推进，本轮据此完成准确五个现有 Rust 文件的默认关闭候选，并准备原限定普通 Linux 检查；这不追溯改写[原 Capture 增量合同](capture-mount-amendment-contract.md)的历史审批记录，也不授予现场安装或安全权限。

组合静审为 `PASS_C0_EXACT_FIVE_SOURCE_CODE_NOT_RUNTIME_NOT_TASK9`：CODE gate `a009984a1760bfa4d46f1c761de041962647390027740b74d2b49dcb1adf91f0`，独立报告 `9d32925343f808d0439d8b05af114d46279a32b028587c747a119661b7976f5c`。原 109 个测试的全部属性和正文逐字保留；新增 20 个普通测试定义，Backup 默认/offhost、DB 默认/peer 的新增映射为 **0/14/0/6**。这些是源代码定义及静态审查，当前候选尚未运行 rustfmt、类型编译、Clippy 或测试；旧普通回归不计作本候选通过。

接下来依次准备独立格式批次、四配置只编译/列举批次，再冻结实际完整名单并运行全新 25 门普通批次。每阶段使用新 UUID、原件独核和精确绑定，不自动重跑失败。当前仅有现成密码登录；未来须用户在真实控制台启动一次运输脚本，由标准 SSH 在四次连接中直接交互认证，Python 不读取、传递、缓存或记录口令。本候选没有 Linux 产品执行结果；现有非交互 SSH 身份的只读元数据连接尝试已权限拒绝，未绕过或重试，原 HOLD 保留。后续由真实控制台的标准客户端认证。

新候选固定十五项配方，保留原十二项和原十二件证据，不能用原十二项实际命名空间冒充新配方成功或自动回退。真实 C0-C02 兼容性、三项只读挂载与 flock、来源可写别名排除、产品自有 FD≤64及必要 runtime/SSH 的归属/重叠/实际总数、S5、真实 Root/PG/C11、keys/trust/安装、D1 首签与 RA、D2 源不可访问与 RB、Task9/C4 均保持 **OPEN**。原37/53、原合同三件、52个历史检查点以及全部旧 FAIL/HOLD 保留；没有合并 main 或部署生产。

---

以下完整保留此前检查点，均为当时的历史记录；其中“编码待批准”“产品源码未改”等表述不代表上述当前候选状态，旧运行计数也不授予本候选或现场运行信用。

## 2026-11-01：Capture 挂载增量合同完成独审，编码与现场安装仍待批准

已完成[Capture 挂载增量合同](capture-mount-amendment-contract.md)、准确机器表与分层检查计划的准备和独立审查。当前产品源码保持 0531083cc7b686ee30a0df7b4c9cdae3aa84b574 的字节与默认关闭状态，本轮没有编码、产品测试或现场安装信用。

唯一方案保留原十二项挂载及实际 Source/name 绑定，增加 Capture 专属 policy、原 Capture own launch manifest 的两个只读别名，以及真实 fence 根只读视图。新 policy 绑定 Capture 自身的构建与 ELF；不能直接借用 PublicationSource 的材料。两文件来源唯一由原已审 Capture release 根 R 加固定叶名派生，不能传任意路径或挂整个 b1 父目录。

未来编码准确限定五个现有文件：Root/source_launch、S5/source/binding、Capture local cutover、DB process 及原 DB tests。需要批准的是这五叶的固定增量和原限定普通 Linux 检查；Cargo、依赖、bin、policy/installed/fence 校验器、打包、部署、CI 与原批准合同均不改。真实三项挂载、Capture 专属文件发行、私有父目录和权限仍属于 C 现场包另批；安装器与目标端资料缺口没有由本合同关闭。

修订保持产品自有 FD 上限64；必要 runtime/SSH 句柄逐项归属、数量和重叠及实际总数另核，原更严局部门不放宽。新只读来源的全 host 可写别名/祖先与 host writer 排除、原件持续再核、纯 parser 与真实 Group/Catalog 身份检查均分层列明。计划的 20 项普通检查与 12 组现场义务全部尚未执行，不抵扣原37/53，不用旧普通回归冒充新代码通过。旧草案未通过项与所有失败/HOLD保留。

产品仍默认关闭；C0 编码、B/C 安装、真实 Root/PG/C11、全期 global64/S5、首次跨机签名、RA/RB、Task9/C4 正式验收仍开放。未合并 main、未部署生产。下方为历史检查点。

---

## 2026-11-01：真实安装与双机验收方案准备，尚未安装或运行

本轮只完成安装前审查、方案与填报结构、现有访问范围内的只读元数据检查及文档核验；产品源码保持 be147b3dbfd0cf4ecfc126dda4d21dfb3fc0d30c 的默认关闭状态。既有 13 道普通 Linux 检查及 Backup 默认 450 / offhost 649 passed（各 106 ignored）、46 个不同新增正文保留原批证据，本轮没有新增产品测试信用。

[真实安装与双机整链实施单](installation-and-two-host-acceptance-plan.md)已明确六项前置缺口和 A/C0/B/C/D1/D2 边界：Capture 固定 12-mount 与新 policy/fence 读取冲突；15-mount 是待审增量候选，未改代码。B1 固定安装器/受限 native parent 链、真实 key/trust/ELF/anchor 清单、旧 writer 排除、全期 global64/S5/Root/PG/C11 以及真正首签和 RA/RB 仍需后续闭合。

S 普通用户元数据已重新读取；root 私有安装信息权限拒绝则保持未知。D 本轮既有非交互访问权限拒绝，没有取得新现场状态，不沿用历史容量当当前保证。密钥用途分为 S 应用认证、SSH transport 与 D 备份签名三类；未生成、安装或收集凭证。C0/B/C/D1/D2 均未由本次准备授予。

执行顺序为现场事实和准确 C0 合同 → B 托管 public pins → 固定角色构建 → C 安装及真实生命周期证明 → D1 原调用成功/CompleteBackup/RA → 预准入 S2 与规定负例 → D2 狭窄源不可访问/RB → 原始证据独核及适用回归。成功 S1 fence 仍长期保留；不重用 S1 或覆盖 D1 trust 来做后续传输。所有旧失败/HOLD 保留，Task9/C4 开放，未合并 main、未部署生产。

以下为按日期保留的历史检查点，不能作为当前安装状态或新增权限依据。

---

## 2026-11-01：first S/M6 默认关闭实现完成限定普通检查，正式验收继续开放

本轮在已批准的原 R2b 职责及准确增量范围内实现 first S / M6 接线，功能默认关闭。源端同一原操作、原定时钟、固定 wire/auth/fence 和目标端同一原件的一次 Final 消费均属于本轮代码范围；普通组件检查不授予实际运行权限。

准确受测源码的完整 13 道普通 Linux 门均退出 0，包含本轮源码的首个 format、严格 Clippy、类型检查、编译、完整列举及非 ignored 库单元测试。Backup 默认配置 450 passed / 106 ignored，offhost 配置 649 passed / 106 ignored；46 个不同新增测试体按实际名单核对，跨 feature 的重复执行不累加。既有 84 个受影响测试的正文和属性保留，未增加 ignored、删测或改名。DB 本轮仅作为依赖编译，没有新增 DB 测试信用。

本轮受测源码共 706 文件，准确 22 个产品路径为 18 既有加 4 新增；其余 684 文件与发布基线字节一致。成功批次 af1986cd-f101-47bf-880a-c2134468013f，源码索引 SHA-256 6efcea40845eef4193a48ee62a7774ca3691b183b1784f525d1ad9cd42f9e48f；结果 SHA-256 95fd387f5e92a168f74b2ba1b7d29e5b2b3436312de60706e90f3cf9e9fc424d。实际源文件前后不变、容器自然退出与精确 ID 清理、原始阶段/流日志和完整名单均经独立核对。旧源码的 format 或单项成功没有继承为本轮完整通过。

以上是默认关闭实现及限定普通检查，不是已安装的 S/D 权限、真实 Root/PG/C11、首次 S、D Ready、fence 或 CompleteBackup 运行验收。B/C 安装、公钥信任与 SourceCapture 新挂载、旧 writer 写入面排除、全期 global64 / S5、真实 Root/PG/C11、首次跨机签名与 RA/RB、Task9 和 C4 正式验收均继续 OPEN。

本轮未生成或安装密钥、信任或 launcher，未改变 PG 或安全权限，未运行目标端整链、源端断联或恢复，未合并 main 或部署生产。所有旧失败和 HOLD 保留；私有文档副本准备也不覆盖旧证据。后续同步仍须守住原 171 处 EOL、private16/P0、protected5/root20 及真实索引 CAS，不以文档脚本代替同步证明。

---

## 2026-10-31：Source 前段准备代码与发布顺序谓词通过限定检查；Task9 继续实施

本轮沿用已批准的 37 文件编码合同，实际改动五个现有源码文件并新增一个私有 source_actor.rs。内部 Source 前端保存同一真实 peer 原件，复用累计扫描计费，补齐 registry 快照与 Task1b 安装元数据的预检桥；重复进入、失败及借用中止会保留拒绝状态。该预检范围尚不包括完整 SourceGroup/Catalog 或首次 S 的准入。

代码明确在释放 holder → lease → registry 后，才通过同一原件请求唯一 Capture，并在原 Book 保留返回的 opaque DATA。DB 的 Source 发布阶段相应要求输入采集已返回实际 DATA，原 child/cut 仍保留且尚未结算。当前公开 Source 入口继续在任何副作用前拒绝；上述内部准备没有接通 M6，也没有形成真实 Root/PG/C11 或首次 S 的运行验收。

已在既有开发服务器的普通用户、固定离线 builder 中完成格式检查、Backup 13 道门及 DB 13 道门，包含严格 Clippy、编译、精确列举与全部非 ignored 库单元测试。Backup 默认配置 450 passed / 106 ignored，offhost 配置 603 passed / 106 ignored；DB 默认配置 62 passed / 24 ignored，peer 配置 123 passed / 26 ignored。新增普通组件测试共 12 个独立测试体（Backup 9、DB peer 3）；不同 feature 配置中的重复执行不重复计数，既有 ignored 用例没有执行或改名。

本轮受测全源候选共 702 个文件；六个源码路径之外的 696 个文件与基线字节完全一致。Linux 格式器仅合并 source_actor.rs 中一个表达式的换行，测试体与属性保持原样。三批检查均保留源码和原始证据，源文件前后未变，检查容器自然退出并按精确 ID 清理；格式检查本身不提供编译或测试信用。三批原始日志、源码字节与容器收尾均已完成独立核对，并与准确源码 CODE 闭环。 这一字节比较针对受测候选，随后同步的进度文档另行更新。

下一步在同一真实原件和批准文件范围内接入私有 Publication 准入、当前 registry/保护快照与 Catalog/SourceGroup 关联，建立首次 S 及原定时钟、Root/C11 生命周期与完整累计计费链路，再完成 Source/M6 和对端交互。此次普通组件回归没有证明全链 global64、S5、D Ready、fence 或 CompleteBackup。

此次工作没有生成或安装密钥、信任、launcher，没有改变 PostgreSQL 或安全权限，没有执行真实目标整链、源端断联或恢复；没有合并 main 或部署生产。真实独立目标上的安装、签名托管和正式异机验收仍须按具体目标、影响及最小权限范围另行实施与确认。Task9 和 P0-C4 的正式验收状态继续保持未完成。

---

## Root生产者与M3内部片段完成两组普通回归，Source整链仍开放 · 2026-10-29

默认关闭的Root生产者与M3原始持有者片段已完成源码独审和同一701文件快照的两组完整普通Linux回归：Backup默认450通过／106忽略、offhost594通过／106忽略；DB默认62通过／24忽略、peer120通过／26忽略。各组13门全部退出0，均0失败／0过滤。新增29个不同正文为Root17、M3 Backup9和DB peer3；配置之间重复执行的正文不累加。

Root侧增加实际子进程／pidfd、固定工具和库身份闭包、持续监督、原始IO计费与期限、异常保留清理的内部实现；M3保留原始future及资源所有者直到自然结束，取消、panic或完成校验失败不能提前把资源当作settled。C11既有15类消息和durable后确认组件保留；本轮普通UID检查不执行真实Root／PG／C11正向整链，也不证明已安装或可用的生产权限。

旧68个受影响测试正文及ignore属性按真实字节保留，本轮无新增ignored、改名或删测。此前DB批次875be4e3在peer Clippy因一个未用私有函数退出101；删除339字节私有死代码经独审后，以本轮全新格式／Backup／DB批次从头验证。旧失败、HOLD及旧快照PASS全部保留，未重放或借作新源码信用。

受测源码基线a3a209711f5441c25093aeb5052fe538537d03b0／treef7915076d0d613d08a1443383e18c25be26c4122，最终701源索引SHA256 fceee5c0aeb439f989051d257e039ff4c5b14724225761edd48118866d81aeb6。本轮格式批次c58ae018-b231-492d-b6b2-5d17980febff、Backup批次5283b8f9-ed50-4de5-95e7-c8daadbefcba、DB批次61f3279a-85df-45fa-8e7d-69ab8ed23400分别独核原始证据；结果SHA256依次为fa2925c2a3f3da3eefb2d52b70a0383d9d432817906b5f721dafe3ffe065b5cb、ecb047f13f0c724ee5d7b6df2f115e32ed3c00f11e3eb90c76464d217a98d917、33982f824c41782590561810f8d80ab7120cad329f22c8d4ac8dafd196221ad8。实际格式导出17项与受审源逐字相同；两组完整名单、既有ignored、源704项前后原始快照、同一容器自然退出0与精确清理均核对。这是收集回件的独立核对，不是额外远程亲见；输出限制为退出后读取校验，未声称连续OS输出上限。

仍未接通正常Source入口：当前入口继续明确拒绝未接线执行，source_actor.rs尚未形成。下一步在已批准文件范围内接Source原操作、共享原始预算的preflight与先关持有者再取lease、唯一Capture生成的CapturedInputData，以及重新取得的首次Source会话；不能把capture-only会话转为Publication权限。Source首S、D/Ready、fence和最终settlement的M6闭环，以及S5封闭计费与全局64资源实证仍开放；whole M3／C11与Task9正式验收均未完成。真实密钥／信任／launcher安装、PG／安全权限、目标整链、源断联和恢复仍须按具体目标与影响另行确认。

本次仅追加task9_r2b_root_m3_original_owners_ordinary检查点；既有47个task*键的深层JSON类型和值保持原样，总48。源文件701／manifest行700，17个产品路径为5新＋12既有，其余684原始文件不变。功能默认关闭；source_READY、CompleteBackup、Task9／C4正式验收、main合并与生产部署均为false。

---

## Root／C11控制与持久记录组件完成普通检查，实际Root／M3接线继续 · 2026-10-29

一个既有backup源码叶增加两端共用wire15／64帧／262400字节预算、严格关联与原期限、native IO永久负向，以及C11的0600独占持久对象和durable后单次ACK。新隔离Linux完整13门全通过：backup默认450通过／106忽略、offhost568通过／106忽略，均0失败／0过滤；offhost新增14个普通正文通过，配置间重复正文不相加。入口与部署门保持默认关闭；这不代表实际Root持有者、M3整体或C11正向整链已完成。

原6个DATA／genesis测试保留；新增14覆盖共享预算、字段／方向／顺序／请求重用、期限、取消／extra／EOF、真实socketpair原语及C11 typed编码。C11 Root:Root0600正向写入未在普通UID下执行；durable writer属于本轮编译审查内容，不能把其编码测试当真实Root观察或ACK生产验收。

上一新批次e8a70c89的format与默认Clippy通过，offhostClippy因collapsible_match退出101，后10门未运行；等价guard修复经独审后在全新批次重跑，旧失败result 53cb9d0241b61eeffaac185485b7774549fb56d8cacc334ecada7f7976602497与独核ba21534fe42f054c77bc0be1bf8637a5aecbdada0ec328cadf3fa6b7fface3d3保留。未降低-D warnings、未新增ignored、未改名或删除测试、未重放旧batch。

本片受测基线983de1f2f0072c646d2cd6147b15f9ea82ab9c63／treecc3c309bc562f7044db61fbbd58aa9990a2f3532；唯一产品叶source_launch.rs SHA256 93d93b4fbfbe307dbf4ba77a9378d5ab204a6fee52130bc1662d7ad03b671fa7。批次a46a5ea3-4d7e-450e-852d-f88b4b1d6fc5，result da27cda980bdac1299888b296f3c17d3ef51eaf6fe61f0c9970762235e54a67b，完整evidence ZIP d97cb0a0d45d099f14a10e102eaa86efcdb8b4c50201289e6072954ef1a66d82；13门exit0、完整556／674名单与各106顶层ignored、精确CID自然退出和清理、699包内文件未变。原始回件独核9b2dab82f244f3407f734366ba9591205800d89ae36b276b2c6437fc39b4d913／INDEX2fd0a1f2ff010947c065cddc407798b3a8c5ae64e6690b24ca9f0eb6a2aed28f；不是独立远程亲见或连续OS输出cap证明。

仍需实现once-burn实际Root job、source child／pidfd／wait、自己创建的Docker exec／attached mux、全host before/after与两generation issuer、原产物独立CapturedData及settlement生产者。M3整个future／finally、Source首S与preflight／Publication实际会话接入、M5保留完整cut／enrollment仍开放；不得放宽capture-only getter或转旧None路径。这些是已批叶内实现缺口，不概括为缺权限。S5下游封闭计费和cap inventory尚未闭合；现场密钥／信任／launcher、PG／安全权限、目标整链、断联与恢复仍按具体方案批准。

仅追加task9_r2b_root_c11_component_ordinary；原46个task* checkpoint深层类型和值保留，总47。产品696／清单695；新路径0。Task9／C4、真实Root／PG／C11、M3整体、M5、完整计费、source_READY、CompleteBackup、main合并和生产部署信用仍为false。

---

## DB3 固定控制动作完成限定普通检查，Root／M3接线继续 · 2026-10-29

已批37叶内，本片6个既有DB源码叶复用同原peer、控制FD、共享预算和期限，增加三个已批窄控制动作、opaque DATA读取及原Book持有的真实阻塞任务。注册未完成时实际RAII保持unknown／cancelled并毒化同原custody；取消借用不detach已开始任务，原close自然join。公共backup runner仍关闭，不能据此产生Root或ACK资格。

DB3六个既有源码叶完成限定普通Linux13门：DB默认62通过／24忽略、peer117通过／26忽略，均0失败／0过滤；peer新增12个独立普通正文通过，配置间重复正文不相加。backup默认与offhost只有依赖check。真实Root发行、M3整个future／finally、M5、C11整链、S5封闭管线与Task9／C4验收仍开放。

13门为fmt check、DB默认／peer严格Clippy、backup默认／offhost cargo check，以及DB两配置各自库no-run、完整名单、ignored名单和普通正文。完整默认名单86、peer143；peer新增12且ignored0／rename0／remove0。新正文不构造genuine context；普通包装入口精确调用两个既有ignored fresh-child helper，因此24／26是各自顶层ignored数，不表示所有ignored函数都没有执行。本轮不执行backup完整库正文或真实Root／PG正向套件。

格式化前45旧测试属性与正文、先前新增11正文raw保持；实际formatter后，独立formatted CODE确认旧45完整属性／正文原raw保持，新12完整属性／正文格式等价，后者不声称raw相同。formatter result a6d0a69cd1d3ab3a666b433c8a591984f9ed7d04d684b46a11af8fc36a0b0b50，FORMAT OTHER e0375d131e59a0b1a6b124fcfad313c8e3c4db696be4cc1a7f829fdf6f7bd6e8；其源包699／产品696／返回6、5个格式变化叶及同CID自然清理按原独审核证；FORMAT自身没有typecheck或库正文信用，完整普通检查信用来自新的2fb批次。

基线63a66327f032f01201eb3c4d6d6d60c0894a81bf／tree06b50c03d0092f9d3391f501114898225c69fe66，产品696、源清单695行，6个路径全已存在、新增路径0。仅新增task9_r2b_db3_fixed_control_ordinary；原45个task* checkpoint深层类型和值保持，总46。source9 S5实际批次、历史DB参考、旧RED／HOLD／未观测字段各归原来源，不并入本轮计数，不重放旧批次。

批次2fb62369-09c1-41fa-8126-9d73b12fe40f，result 1dbfec0f2548f14f601b8898ea2e7dd6d35577570f795328c188a909de5a33bc；evidence ZIP 0f6eb27597aeaf456764c47771dfa77323dce89e4113c86fb22b9986b7316c6b。源CI 546f0cded8c37a685e914874610cfbd80aa0f7e02400562804caac8aa2d018a9／MF 922ebbee717c11ba9256a4f617b47c205412fc166bb90bc4d477ed20b41758e1／TEST 5882ad2f4be6eeff7e30c7d9e35f362dbf25d751f4aba7d4a4635966e0531cb5，Root CODE b45411a2b78eff8448a77456c5b37e15e8f17818b27374654c41d39ddd64f651。最终checks CONTROL 0c97cc470c8e51a20bb24812a9fbca6436d13214087df8b509f63e8ab1be39ab；独立完整原件OTHER 296108a050d5e17956f511b342c0f5df9fccd7d2e333ffbfecdcdbbcb117ad6c／INDEX bdd755139f3112a3d9d6501c4d86ecbbef2ab2176841b39eae125cc92a370322。实际13门全exit0、完整86／143名单与24／26顶层ignored、共享历史资源和新12正文，按该独审有限范围核证；源码原件未变、精确CID自然退出及清理、返回证据链已核。不是独立远程亲见、连续OS cap或全链资源计费证明。

C3历史标签纠正：按既有docs/p0c3-verification.md第3、7至10行，将误写9823的C3受测源码标签恢复为3342c0554c18787749d0f06a772743f867c18720；full原结果c939cc71与独核c22f9485保持原批次。这里只纠正既有标签，不新增、重放或重新核发C3验收信用；当前DB3源基线由sourceCommit=63a66327f032f01201eb3c4d6d6d60c0894a81bf单列。

真实Root持有者、全host before枚举、实际genesis／after、持久leaf与ACK生产者仍待实现；M3整个actor future／finally、Source首S与Publication阶段／capture-only admission差异、M5保持仍开放。S5下游sealed／fs等读链的完整有界计费及cap inventory未闭合，缺口清单仍不完整。37叶是本轮补充编码的范围，并不把范围外实现概括为未授权；需要扩大本轮施工范围时应逐项列明。现场密钥／信任／launcher安装、PG／安全权限、源隔离／恢复和真实整链按具体方案办理。
本次有限普通检查不授source_READY、Root／M3整体／M5／C11／S5整链、封闭总计费、cap inventory、CompleteBackup、Task9／C4、main合并或生产部署信用。

---

## S5 消费端与 C11 DATA 完成限定普通检查，Root／M3 接线继续 · 2026-10-28

已批 37 个产品源码叶内，本轮 9 叶 S5 消费端与 source_launch DATA 候选完成限定 backup 普通 Linux 检查。这里的 S5 消费端检查不表示 S5 封闭管线、M3 实际 actor、Root 真实发行或 C11 整链完成；Task9／C4 仍开放。

新批次 15741a1e-f2c4-4244-b7d4-4edd3fd70438 的 13 道门全部 exit0：格式检查；backup 默认／offhost 严格 Clippy；两配置 cargo check；默认库编译 no-run、完整名单、ignored 名单与普通正文；offhost 库同样四项。默认库 450 passed／106 ignored／0 filtered；offhost 库 554 passed／106 ignored／0 filtered。两配置重复正文不相加，106 是各自顶层 libtest ignored 数。原完整旧名单保留，offhost 增加 12 个独立普通正文：source_launch 的 6 个 DATA 正文、4 个原共享预算下的有界目录正文、2 个结构预扣与预算耗尽正文；没有新增 ignored 或删除旧正文。

本轮 learning-db 只作为依赖编译，没有新执行 DB 测试正文。此前普通 UID 包装入口精确调用两个既有 ignored child helper 的执行信用仍属于对应历史 DB 批次；不概括为“所有 ignored 函数均未执行”。本轮 backup 的普通 SIGKILL 子进程沿用原受审 profile，未运行真实 Root／PG 正向、安装或恢复套件，也未重跑无关 Task8。

源基线 9823d820982a7b452fab87ffe3fef9d060745009／tree cce0f805b6609fa4762268c8ab61bc7ab7dd2389，原 695 个产品文件叠加 8 个既有路径与 1 个新增 source_launch.rs，计划发表后产品 696、源清单 695 行；数量须由实际发表 PLAN 再核。实际检查源 INDEX 90dec313a42a7325747e48ee0750cd78271de40088ba30f39ae857bf0915477e，MF 75157472ed32586df300ee029cc3d031287f3b17aa8b1cccd56a5167b615f780；TEST-DELTA d5208afa6ae53837b9e995f2f8e89e735d68df7c6d819f27450075db5b568f48 固定 12 个新名称。当前 CODE gate 556fd1da85a55c6cb2f101f8bd6ab074b6428d7a64857d92d5f57f6381360f7a 与最终 CONTROL 适配报告 249cad8c2534915f726fa242642b1ce10c0128ca8f18f1edd5f33973ba0c9fac 绑定同一 source9。

实际 result SHA-256 4e6456910163ab2c274d00e0f104fc157699f528cb83b4064b85790ddef17f51；返回证据包 c91ac50157f6986149324aed80d340c22cd1cbb91de1c92b906e544431873839。结果记录源码原件未变和精确容器清理。完整原件独立核验通过，独审 INDEX 8f64b9e1475b4a094680983b78ccc8ed179c98e7c404d879e5ce843d03e9af5b／REVIEW f3b0d2f41f661d63835e49339647ba3069489aa19313f0fb102df1fdf7eeafdf。原始有限日志的完整退出码、名单与正文按各自原件核验，不据此声称独立远程亲见或全链资源计费已经闭合。

首个 43b63e46 批次因默认配置的 endpoint 未使用诊断保持 RED；第二个 7e22250d 批次格式及默认 Clippy 通过、offhost Clippy 的 7 条 lint 保持 RED，其余 10 门未执行。两个原件和失败证据保留、不重放。前一修复只改同一函数绑定名与引用；后一修复将原借用 DATA／原流资源参数分组后立即拆包，保留原字段、Arc、预算、None、判断与清理次序，并等价折叠一次发送的失败分支。两个尚未接入的根入口只标注准确 pending M3／SourceActor 的 dead_code 原因；没有抑制 correctness 或 too_many_arguments，没有跳用例或放宽检查。新 formatter 与 CODE／CONTROL 后，以新 UUID 完整重跑 13 门。

原 44 个 checkpoint 保留深层类型和值，新增的第 45 个仅为 task9_r2b_s5_consumer_c11_data_ordinary。历史 DB 消费端普通结果、旧版本数量、失败、HOLD 与未观测字段各归原批次，不并入本轮计数。两份已批 C11 合同原 bytes 已在基线中并继续保留；没有新增第 38 个批准源码叶。

真实Root owner、安装观察与持久ACK，M3整个actor future／finally及M5保持仍待接线；S5封闭预算尚有sealed／catalog／assets／protection／lifecycle等下游读链待清点。当前缺口清单不完整，已批37叶尚未闭合，最小扩范围方案及审批材料仍待完整审计。继续Source／D、fence和同原操作发布；cap-scope、closed source、receiver及genuine链保持开放。安装、密钥／信任／launcher、PG／安全权限、源隔离与恢复仍按准确审批停止点办理。 本轮不授 C11 整链、S5 完整、M3／Root 实装、Task9／C4 验收、CompleteBackup、公开恢复、main 合并或生产部署信用。

---

## C11 DB消费者完成普通 Linux 检查，Root 接线继续 · 2026-10-28

已批 37 个产品源码文件内的 15 个 DB 源码叶，完成 C11 新 DATA、15 消息与安装观察回应消费端的普通 Linux 编译和回归。当前仅形成 DB 消费端普通检查结果，尚未闭合真实 Root 发行者、M3 原入口整体 finally、S5 封闭内部计费或完整 C11 管线；Task9／C4 仍开放。

新批次 eeca0765-37de-4b99-99b7-5f62592335df 的 13 道门全部 exit0：格式检查、DB 默认／peer 严格 Clippy、backup 默认／offhost 编译检查，以及 DB 两套库编译、完整列举、忽略名单与普通正文。默认库 62 passed／24 ignored；peer 库 105 passed／26 ignored。两配置的重复正文不相加。新增 5 个 C11 DATA 正文通过，保留原 3 个资源生命周期正文；仅 2 个原协议测试名称随获批 14→15 协议适配，原忽略名称集合保持不变。

24／26 是顶层 libtest 的 ignored 数量。原普通 UID 包装负例会在全新普通子进程中精确调用两个既有 ignored child helper；不能据此概括“所有 ignored 函数均未执行”。没有执行真实 Root／PG 正向套件；backup 两套仅 cargo check，没有重跑其完整测试。

本轮基线 43e212665083e4c0a8dc0a58adc1ba413e14b1fb，加封存 lint 修复候选 INDEX f93bd823c6c5aa1ed34a13efb80819fa9f23f3a74a930a091ac13b7a9ec06aed／MF dae15e70cfb23c51ec3d25ad0e98814be7e844eca65c0488d6fb89d42a6e804a。当前 695 个产品文件中 15 路径全部既有，5 处 C11 源差异、10 处原 R7。实际 result SHA-256 f816b881868f7b585eb4a0a82ef08f28ae21d8a03f52bd5bbea321d683933da3；返回结果记录源码原件未变、精确容器清理完成。完整原件已经独立核验：源码包、全部实际退出码、完整名单与正文、源码前后、同一容器身份及精确清理、返回证据均已逐项核对。独审 INDEX SHA-256 a6771c08a79abf2231e973f1e087c31dcdc55fef7e8b2d181ddf5b4cad20b96b；不声称独立远程亲见、OS 连续日志上限或全局 64FD 实测。

首个 02f0f0a2 批次因一个 collapsible_if 严格 Clippy 诊断保持 RED；原始失败和证据保留、不重放。新候选只把该处嵌套 if 等价合为短路条件，保留 legacy.next 的调用条件／次序／次数；未添加 allow、跳过用例或放宽任何检查，再以全新 UUID 完整检查。

原 R7 的 62／24、100／26 属于旧 14 消息版本，原 43 个历史 checkpoint、旧失败及 HOLD 原样保留，不倒改或重复计入本轮。两份已批 C11 合同原字节已在公开基线中，继续原样保留。本轮新增的仅是 C11 DB消费者普通检查点。

后续继续真实 Root 发行者、M3 整个 actor future／finally 与 S5 原始封闭计费的接线，再推进 Source／D、fence 和 C／D 安装、真实 RA／RB 资格及异机整链。密钥／信任／launcher 安装、PG／安全权限、源隔离、恢复和 CompleteBackup 仍按原定准确审批；不宣称 genuine、C11 整链完成、Task9／C4 验收、main 合并或生产部署。

---

## M2b 原会话与资源结算完成限定 Linux 检查 · 2026-10-28

14 个 DB 源码叶已实现固定 peer 会话、实际连接与三组 pool 的持有、安装／排空的原任务衔接及负向结算接缝；新增一个私有 session 源文件。该候选完成普通 Linux 编译与回归，不等于真实 Root／PG 正向准入或完整 M2b／Task9 验收。

全新批次的 13 道门全部 exit0：格式、DB 默认／peer 严格 Clippy、backup 默认／offhost 编译检查，以及 DB 两套库编译、完整列举、忽略名单与普通正文。默认库 62 passed／24 ignored；peer 库 100 passed／26 ignored。新增 3 个资源生命周期正文实际通过；旧 123 个 peer 名称及两套旧忽略名单保留，两配置不相加。backup 两套只做编译检查，没有重跑其完整测试或执行忽略的 Root／PG 用例。

本轮验证基线为 9a31b62480422e302f558f5b7771213bbb13fd84 加封存候选 f11d886d…；实际批次 df2fc491-b3bd-4fdb-a4f1-8d1c5b92b575，result SHA-256 544827f8e87ad3c652ec26f57e3e840652f4533e1b3a8398c6cd7fc8bf59da47。普通 UID1000、固定 fb91 builder、network none、4CPU／8GiB、只读源码与 rootfs。完整原件单独核对源码前后、精确容器 ID 与返回证据；不声称审查者独立远程亲见、OS 连续日志上限或全局 64FD 证明。先前 fmt／Clippy／缺类型及参数数量失败均保留，未复用失败批次或放宽检查。

C11 新增回应和安装观察记录的准确补充合同已获人类批准，见 [合同](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/feat/task7-attention-recovery/docs/task9/c11/CONTRACT.md)及[字段表](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/feat/task7-attention-recovery/docs/task9/c11/MACHINE-DELTA.json)。准确产品源码范围仍为 37 文件、13 个主动作。消费者七叶与安装后 DATA／held-genesis 读取组件完成有限源码独审，扫描预算问题已修正；DB 的 15 消息及回应等待在独立私有候选内继续。本次发布并验证的 DB 候选仍是接入前的 14 消息版本，不能把上述有限源码审查算成本轮 C11 类型检查或整链通过。

下一步闭合 C11 的真实 I0 等待与持久记录核验、Root 原发行者、原入口整体 finally 以及封存内部计费，再继续原 Source／D actor、fence 和一次消费发布。真实发行、密钥／信任／launcher 安装、PG／安全权限、独立目标整链、源隔离和恢复仍需原定准确审批。旧失败／HOLD与历史检查点保留；Task9／C4 开放，main 与生产不变。

---

## M2b 事务借用接缝限定完成 · 2026-10-28

本次五文件补充编码已获直接批准，当前准确源码范围为37文件。六个现有DB文件的14处私有事务参数改用借用生命周期；函数体、SQL、返回语义和公开API不变。

普通Linux13门全部exit0：格式、DB默认/peer严格Clippy、backup默认/offhost编译检查，以及DB库默认62通过/24忽略、peer97通过/26忽略。名单与旧c3原件逐名相同，新增测试正文0；两配置不相加，backup未重跑完整测试。

这只关闭借用事务接缝片，未实现或证明完整M2b会话/pool消费者。五字段只读DATA细化已按独审采纳，实际会话及消费者仍待实现；C/M4真实发行与Root-PG-capture join、M5/M6和全局64FD仍HOLD，Task9/C4未验收。

下一步继续真实原会话资源owner、固定连接/三pool与完整消费者、自然关闭/安装/排空结算。密钥/信任/launcher安装、PG与安全权限、真实目标整链、源隔离和恢复仍未获本轮许可；不合并main或部署。

实际批次addac5ac-85e0-4d2d-8757-aed1fe2d02d7；result a8fc34509475108a1041a7c39a0a9efe773a9ce5ee351432a8adbf0ac172a8ae；独审 fd535068fd276372bf8870435c7255a832d4ff953cc9e03bd7930be9e24c18db。UID1000、固定fb91 builder、network none、4CPU/8GiB、只读源码/rootfs；源前后相同及精确ID清理由返回原件独核，不称独立远程亲见或OS连续日志上限。旧RED/HOLD和所有历史检查点保留。

---

## M2a 原进程与唯一控制owner组件限定完成 · 2026-10-28

默认关闭的M2a只发布五个DB源码叶及四个公开窄方法；保留真实原process/control owner、原FD/记录/锁/native与同scan。SQL/admin/action消费者未接入。

同一46b基线加五叶在普通Linux完成十四相位原件核验；default DB库 62 passed / 24 ignored，peer库 97 passed / 26 ignored。component37声明，实际35正文通过、2 helper ignored。实际名字及分类由完整全文parser重算；重复whole-lib/component不增不同正文信用。

普通DATA/native原语与缺compilepin/普通UID拒绝不是genuine factory正例。C初始主机/proc来源和M4原Root-PG-capture producer join仍HOLD；全局64FD与真实发行权限未证明。Task2c/M1a/M1b旧检查点、旧失败/HOLD完整保留且不相加，Task9/C4未验收。

下一步M2b原owned SQL与完整pool消费者；其拟新增五个既有DB源码文件尚待准确人类范围决定，本片不纳入。M4/M5/M6、真实目标整链、launcher/密钥信任/PG与安全权限、源隔离或恢复不在本次批准中。

实际批次`c3eebe79-eafc-448a-af2c-704b10063202`；result `8a0668e8d3f9e49ed379b345241a30e3aac74104df7348eb3e76e4bc25f833f1`；独审 `b23b484557f5b630bc8f8abb30f1348a686fb778a119671d021eba43fa11d5b6`；源码ZIP `846223b8ae41ce22bae772e2ee87778675b590b87581862c3234a90c83b191c5`。UID1000/fb91固定builder、networknone、4CPU/8GiB、只读源码/rootfs；原件证明源前后相同和精确ID收尾，不声称审查者独立远程亲见；日志上限为事后检查。

---

## M1b 编译身份与默认关闭依赖限定完成 · 2026-10-27

M1b完成manager与业务application两族编译pin拆分：原业务registry谓词不变；learning-db新增默认关闭b1-source-peer，仅激活已有可选sha2直接边，offhost-b1显式传递该feature，无第三方版本或checksum变更。

同一bfe44c6基线＋四叶候选在普通Linux固定builder执行27门全部exit0：格式、四套Linux metadata、三套tree与三套check、严格Clippy、bins，以及learning-backup库默认450/特性542通过，各106 ignored未执行。五种独立编译环境各跑相同4个新增正文，共20次调用，不是20个不同测试；特性下offhost93＝旧89＋本片4。

这证明实际option_env编译族、原registry的DATA join和默认关闭依赖边；五变体连接独立target/Cargo编译路径、compileID及实际4ok，各变体ELF SHA未返回，不作其摘要独核声明。未执行旧Root/PG getter，未构造genuine factory或完成真实权限接线。Task2c默认450/特性538/offhost89与M1a assets6保留各自历史检查点，不相加、不改写。原37规格、53义务和五门仍开放，Task9/C4未验收。

下一步在已批32叶内继续真实固定peer factory、原控制owner及其后SQL/Root/capture/Source/D接线。M2a私有候选未在本次Linux验证，也不随本包发布；密钥/信任/launcher安装、PG与安全配置、独立目标整链、源隔离和恢复仍另批。旧失败/HOLD保留。

实际批次`2fc37585-67fd-4113-bb94-d6a336097c50`；result `25522950c25b09b7d07cd5ef8995fc16cb9791ff5c481c93de0cdeaca66da447`；独审 `5a7c0a954d6aa1cf423dbb48de7112a421ed536067540e503542572a5f0307ad`；源码ZIP `c97f5c7bedb09f1d41f101f9dddee78928a85b8b5af8a0826f7dbfe9dbd09285`。UID1000、固定builder、network none、4CPU/8GiB、只读源码/rootfs；源前后相同，精确ID收尾由原驱动与返回原件核验，非审查者独立远程观察。日志上限为事后读取检查，不是OS连续写限制。

---

## M1a scan账本身份组件限定完成 · 2026-10-27

M1a新增 `SharedJournalBudget::shares_meter_with` 借用只读 Arc 身份比较；不加锁、不计费、不重置额度、不产生权限。单叶按Git053592d＋此候选运行格式、严格Clippy、assets构建、精确列举及六项正文：旧3＋新增3，6 passed/0 failed/0 ignored，30 filtered未执行。

该组件在旧053源码实际验证；全部assets/core/锁文件/工具链依赖与本次ab8caae集成基线逐字节桥接一致。字节桥不是新合并树全量测试，也不证明真实跨crate原预算接线、Root/PG主体、Source/D actor或全局64FD。Task2c的默认450/特性538、offhost89属于先前独立检查点，保持原记录且不与本轮6项相加。

下一步继续已批准SourceActor32叶的默认关闭实现，先接manager编译身份与真实固定peer factory/原控制owner。密钥/信任/launcher安装、PG与安全配置、目标整链、源隔离与恢复仍另批；Task9/C4及原五门保持OPEN，旧失败/HOLD保留。

实际M1a批次`02bf4d0b-fcfa-4e83-8837-b55d04a371ba`；ZIP `e897691db450cd6e9dc994a72b5c7f5b37d97234293d74375dd59ed42ba89608`；result `45781eb0709dc57e3e8b952753594345b87b2069937a22ae3c0d9adfeb17f804`；独审 `efe3bef4097825627a2988bcf0f46cf6762892216464717045f1a93356b07248`；依赖桥针 `531492274e0f112f8862ec3f73f427b5db18adb029633b22855da243859e6a52`。普通UID1000、固定builder、network none、4CPU/8GiB、只读源码/rootfs；五门exit0、源前后相同。精确ID删除/缺席来自原驱动与返回原件核验，非审查者独立远程观察；日志1MiB上限为事后读取检查，不是OS追踪或连续写限制。没有Root正例或运行安装。

---

## 最新检查点：Task2c签名DATA片 · 2026-10-27

Task2c完成固定SourceAuth/SourceFinalDecision的签名DATA验证片，新14项/offhost89逐名实际通过并独核；默认450通过/106忽略，特性538通过/106忽略分别计；两配置不相加。格式、严格Clippy、bin构建通过。

固定公开RFC夹具只证明单帧DATA及原owner错误/unwind后的粘性拒绝；没有运行密钥来源、跨操作真实认证、原S时钟、Child/SSH、Source/D权限、fence或一次消费发布。两个runner仍拒绝。

SourceActor准确32叶补充编码范围已获人类批准；默认关闭，尚未接入和现场安装。key/trust、Root launcher/PG配置/安全权限、目标整链、源隔离和恢复仍另批；Task9/C4五门保持OPEN。

实际批次`43fadcb8-89df-4ab3-be72-e062a7153dbf`；ZIP `06b999e20a7f90369e40e8a1ff6dc63eba979e5261d141cb3b74828bcfadb180`；result `c41ac78c38c866d2e7ea52492989f76405abe0f17397715537c6be79dcf7855b`；源码五叶refs `cddad1f103675b3e62e24e2d24bd9fca752110c9d92d6ee2d4e11c41ea84ac82`；代码与实际原件独审 `f85f7475ea40a39443e4b1a3021a9581f4630908f389c900df62d382f8475abc`。普通UID、固定builder、网络none、只读源码/rootfs，七门exit0、源前后相同、精确ID清理确认来自驱动断言与返回原件；不宣称独立远程观察、OS追踪或全局64FD。日志限制是事后读取检查。

公开夹具批次`9c11b182-0d87-4a89-a3cb-7430e2c9f87b`，result `11d6211e34f751f2e8def2583d97ce171ad2bcb4676f590f0f9cd2393663ec8a`，独审 `d3898ac44a5e34b1184c4bf719a5e6def18e7e1650ae6dace0ecdd2918edf36d`：一套独立Python framer、两种签名算法重算六槽，不是两个编码器，也不是产品认证。旧79b7源目录准入RED保留，未启动容器/签名；旧source_unchanged=false不改写。

旧75项、计划53项分层义务和原37规格是不同集合；忽略用例未执行。五叶签名DATA不能补证原Hello/整个operation认证或产生D permit。历史检查点和旧失败/HOLD原样保留。

---

## 最新检查点：Task2b有界管道IO片 · 2026-10-27

状态：**OWNED_PIPE_IO_LINUX_CHECKED / UNAUTHENTICATED / R2B_NOT_FULLY_WIRED / NOT_INSTALLED / TASK9_OPEN**。五叶实现拥有FIFO句柄的有界AsyncFd读写、分段预付计费、绝对deadline、取消后粘性拒绝及实际read(0)观察。数据仍是Untrusted；两个runner仍始终拒绝，未产生Source/D权限或发布许可。

20B帧头与一帧额度在首次原生读取前预付，payload在分配/读取前计费；取消、Drop与失败不退款。EOF探针也在读取前从同一SessionBudget预留1B，不增加帧计数，读到0不退款；额度不足即拒绝，不能免费越过wire cap探测。旧完整帧的原子计费语义不变。无新增独立预算重置。

实际管道End、Abort、半关闭和内核EOF分账，finish_untrusted不证明自然Child退出或Committed。D输出End前要求观察S真实EOF；语法夹具不是认证握手。构造和每次写前只读检查SIGPIPE为SIG_IGN，不改全局信号或FD标志；不满足即拒绝。未来固定运行时仍须封闭并发改信号的调用图。

IO对象只持有一个创建时钟，1800s操作、60s握手、30s空闲以绝对deadline取最早者，真实正向IO才更新空闲时间。当前未接真实认证actor，UntrustedAuthAccepted不能解除握手时限；本片IO创建时钟也不等于原S首次准入时刻，后续接线必须绑定原时钟。

| 实际Linux门 | 结果 | 边界 |
|---|---|---|
| 格式、全target新feature严格Clippy、全部bin构建 | 三项exit0 | 不等完整workspace或实际manager执行 |
| 默认库列举与正文 | 450 passed / 0 failed / 106 ignored | 独立配置，不相加 |
| feature库列举与正文 | 524 passed / 0 failed / 106 ignored | offhost75逐名ok：旧54＋本片21 |
| 本片21项 | 4项reservation单元＋17项IO | 17项中16项原生管道正文、1项EINTR错误分类；后者不是真实信号中断实验 |

16项原生管道正文覆盖短读写、阻塞/取消、实际EPIPE、实际EOF与半关闭等，不冒充OS追踪、内核墙钟严格测量、Child/SSH、认证或全局64FD证明。75项offhost、计划53项分层义务和旧37规格是不同集合；106 ignored未执行，不相加450与524。既有54项名称和本次正文通过由原始日志逐名核对。

实际批次`c424c99d-313e-42aa-b44a-081f5925c773`；ZIP `f48f6a45ce8051d9bfcee2ebbb33031c9957d2a0903a5d757beb9cbc864560a2`；result `1260f0acb36190816a1bca7888241eebe9e4861c3d275d7755561a96a495c6a3`。普通UID、固定builder、网络none、只读rootfs/源码、4CPU/8GiB，七门exit0且源码前后相同。独立复核完整14日志、684包条目/682源码条目、680基线＋3改2新、完整容器inspect；INDEX SHA-256 `c8757971ce1c52dad30ea3af7f1cead60c95ed116cf99bf0050e35ccf19fb4b5`。精确ID清理确认来自固定Root驱动断言/返回证据，非审查者独立远程观察；rm/ps原始输出及out/*.exit未单独导出。日志上限是事后读取检查，本批无新增完整ELF独立hash或OS时钟证明。

保留早期设计HOLD：免费EOF额外1B探针已改为同预算预留，未放宽cap。`5490f440…`仅Linux shadow格式准备、零正文，result `6af7db45170e5007c4a68dba0da5af53c03ec1e05bceb7e33228ffe06e5c281b`；四份格式差异和五叶原字节桥经独审后才运行本轮。没有复用旧失败批次，也未清理旧失败/HOLD。

没有Child/stdout与stderr双EOF/自然退出监督、SSH、PG、密钥、验签、Root完整holder、真实S/D actor、fence或一次消费writer；不以纯IO取消/EOF回退已Consuming的P。未创建PG、网络或卷，未连接目标端或在Windows执行产品测试。

下一步接受控Child监督和认证准备。SourceActor的生产capture布局、Root发证、固定peer会话与跨crate累计计费合同仍在独审；这部分超出原learning-backup表的新增代码尚未批准。现场key/trust/权限/PG配置、目标整链、源隔离与恢复另批；Task9五门、父Task3/4/5与C4保持OPEN。下方为已发布历史检查点。

---

## 最新检查点：Task2a纯协议编码片 · 2026-10-27

状态：**UNTRUSTED_WIRE_CODEC_LINUX_CHECKED / NO_AUTH_OR_IO / R2B_NOT_FULLY_WIRED / NOT_INSTALLED / TASK9_OPEN**。五叶提供20B帧头、2种Hello、10种typed控制、精确字段/重复键/canonical检查、双向同一SessionBudget、单文件流语法及候选摘要链。Ready unsigned CopyWitness/Intent各≤16KiB并借用原字节；receipt≤8KiB；不重排旧v2 body或替代原业务验证。空Abort受支持，非空固定Unsupported拒绝，未擅自新增reason编码。

所有输出/phase/hash仍是Untrusted或Candidate数据。没有IO、SSH、PG、key、签名、真实SourceAdmission或D原上下文。两个runner仍始终integration incomplete。SourceAuth/Final的语法与候选preimage不证明验签、原actor身份或最终许可；finish_syntax的Ok只表示语法终态，可包含Abort，不是正常完成、真实EOF、自然退出或Committed。

| 实际Linux门 | 结果 | 边界 |
|---|---|---|
| 格式、全target新feature严格Clippy、全部bin构建 | 三项exit0 | 不等完整workspace或实际manager执行 |
| 默认库列举与正文 | 450 passed / 0 failed / 106 ignored | 独立配置，不相加 |
| feature库列举与正文 | 503 passed / 0 failed / 106 ignored | offhost54逐名ok：基础19＋元数据8＋本片27 |
| 本片新增27 | 实际执行并通过 | 纯字段/阶段/长度/单流/共享计费/候选摘要；无真实管道、认证、FS或进程退出信用 |

两配置净差53来自offhost54与一项仅默认存在的旧测试；**此53与计划JSON的53项分层义务不是同一集合**。不累加450与503，106 ignored未执行；也不把旧准备Rust19、帧20或原37规格重新记为本片运行。R2B-WIRE条目的UNIT部分有本片来源，真实IO/deadline、受信安装/认证、跨机重放、原D消费与正常结束尚未验证，不能逐条宣布全部义务关闭。

实际批次`a05d75b5-67c9-4823-b9dc-99604eb11897`；ZIP `629d8d7c42d1289a220e948a27393eda8272fc6dac03bc5fd94844d0fb650ec8`；result `0f2231fca8f5555be8ed49ef2435978ff1bb7f5bf7f0fbbd6d62a706420d5564`。固定builder、普通UID、网络none、只读rootfs/源码、4CPU/8GiB，源码前后相同，精确builder已清理。保留编译和原始证据；未创建PG、网络或卷，未操作目标机或在Windows运行产品测试。原始日志/完整listing与正文、包与执行源码由独立代理限定回读，INDEX SHA-256 `3698ee2e61bcb02271598fd8dcd639ce7f8c949784313cde5165fdd8f5638027`。测试脚本日志大小仍为事后读取上限，不替代产品连续有界IO；没有新的完整ELF独立散列或独立OS时钟证明。

保留旧候选：首份纯Linuxshadow格式准备`9c5f452e…`仅fmt0、零正文；独审`20cad0ea…`发现S End后同方向Abort仍被接受，旧候选因此BLOCKED。旧29c2971c包从未启动，不记为实际失败或通过。修复先统一拒绝已关闭方向，再补27项中的具名反例；正常S End→D End不变。84dab8ec实际轮只fmt1（新增assert排版），零编译/正文，原result `5198c4bd2306e81e4695c6fcad10d235a93633007eccf26e7636e1a68d59b181`保留。按612B原差异修正后用本次新UUID重跑七门；没有复用失败资源或放宽判据。

纯语法Abort/poison也不能被未来D actor用来回退已经Consuming的P；已采纳的迟到cancel/EOF例外仍由原D状态分账。真实Root holder/全局64FD/管道EOF、认证链、原S/D权限、fence、十一证据与副证明、一次消费writer及安装整链均非本片完成项。

下一步为已批准的有界IO/监督与认证准备；SourceActor的生产capture布局、独立Root发证、peer认证及跨crate累计计费合同仍在具体化和独审，尚未获新增编码或现场变更批准。旧guard不暗改。key/trust/权限/PG配置、目标整链、源隔离及恢复仍另批；Task9五门、父Task3/4/5及C4保持OPEN，main与生产不变。下方保留已发布历史检查点。

---

## 最新检查点：Task1b固定元数据FD片 · 2026-10-27

状态：**METADATA_FD_SLICE_LINUX_CHECKED / R2B_NOT_FULLY_WIRED / NOT_INSTALLED / TASK9_OPEN**。三文件变化接入私有installed模块，固定policy/launch/anchor路径及Root文件元数据谓词；保留打开的文件/父目录句柄，在原RegistryLease的同一累计ScanBudget中计费stat、读取、EOF和重读，按原FD与同名路径的身份/字节再核。错误后holder保持失败，不以重新开预算恢复。返回的仍是私有非权威数据，runner仍始终拒绝。

本片没有创建Root安装文件、key或fence，也没有接真实SourceAdmission、D上下文、launcher认证、PG、SSH、签名或发布。文件stamp包含dev/ino/uid/gid/mode/nlink/len/mtime/ctime；父目录当前只核dev/ino、私有目录及可信祖先，不宣称完整父目录时间戳重核。静态局部保留5FD、重开峰值7FD，**不是已测全局64FD**。

| 实际Linux门 | 结果 | 边界 |
|---|---|---|
| 格式、全target新feature严格Clippy、bin构建 | 三项exit0 | 没有执行安装manager |
| 默认库列举与正文 | 450 passed / 0 failed / 106 ignored | 独立配置，不能相加 |
| feature库列举与正文 | 476 passed / 0 failed / 106 ignored | 27项offhost逐名ok：既有19＋本片新增8 |
| 新增8项 | 实际执行并通过 | 普通UID的实际FS拒绝/身份元数据原语及纯谓词/预算；不冒充Root正向完整holder链 |

Root正向三文件准入、完整holder动态再核及真实authority仍未执行；106 ignored不算通过。新增8不重复计既有19，不关闭53项分层义务或原37规格。默认和feature净差26来自27项offhost及一项仅默认存在的旧测试，不能宣称净增27个所有配置正文。

实际批次`483b1354-7c27-426e-85f4-a1003050c79e`；ZIP SHA-256 `e398854a7ed91efa62b04b95d99a8c37223efc4f182aeed388a45738583af3d7`；源码manifest `8c1493e395cdeffe5eefab8b2e93b3ad82dae024857c3729e14633eda9c3041a`；result `e0e36bbea8f3b96d8686c5ff4a12a5171dc6ebffffcbb66b802cd9f5c80903d5`。固定builder普通UID、无网络、只读rootfs/源码、4CPU/8GiB；源码前后相同，精确builder已清理，编译/原始证据保留。未创建PG、网络或卷，未操作目标机或运行Windows产品测试。日志上限仍是可信测试脚本事后检查，不替代产品连续有界IO。

首轮`4dc91a47-0d1f-4acc-917d-f64c6e712aaa`只在格式门失败，未执行新增正文；保留result `c3a90c6362ad3149d4012408398a56ddd0b39c7d6db4440ffbc3022a260ba151`。仅两新文件按Linux rustfmt差异修正后使用全新批次跑七门；旧失败不重放。格式桥、当前三文件和原始结果已独审，INDEX SHA-256 `ff9ebb0080c274f2f8ab5ce7d7e843675b6e7492063a0ea6fb965bfc0842053f`。

下一片先完成已批准的纯wire/schema/phase/transcript与非特权IO，真实actor仍有三项准确接线缺口：完整effects所需生产capture挂载；Root Unix peer认证与旧admin/runtime原主体检查的兼容；内部UUID对应独立Root发证及child原cut保留。只读方案正在独审，尚未采纳或新增授权；不把路径表/JSON重建为原cut，也不暗改旧guard。需要跨learning-db或新增Root生产入口时先交准确补充编码范围；现场key/trust/PG认证配置/权限/挂载、目标整链与恢复仍另批。

Task9五门、父Task3/4/5与C4保持OPEN；main、生产、公共v2和原37规格不变。下方Task1a为已发布历史检查点。

---

## 历史检查点：Task1a基础片 · 2026-10-26

2026-10-26。状态：**FOUNDATION_IMPLEMENTED_AND_LINUX_CHECKED / R2B_NOT_FULLY_WIRED / NOT_INSTALLED / TASK9_OPEN**。用户已批准的准确范围见[批准记录](r2b-implementation-approval.md)和[实施计划](../superpowers/plans/2026-10-26-task9-r2b-implementation.md)。

## 本片实际代码

九个Rust/Cargo文件提供默认关闭的`offhost-b1`、两个required-feature固定无参数bin、编译role/pin拒绝、固定profile及checked累计预算、严格canonical的policy/anchor/launch数据类型和原始字节摘要连接。输入仅返回私有`Untrusted*Inputs`，不是SourceAdmission、原D上下文或可重放permit。

**即使编译pin完整，新runner仍返回integration incomplete。** 本片不读安装文件、stdin、PG、SSH或密钥，不创建fence、不签名或发布。真实FD/custody loader、首次SourceAdmission/registry lease/snapshot actor、原D context、真实wire/auth/IO/fence与一次消费writer留给后续片。未接线数据模块局部允许dead_code，运行消费者接入时须移除；未抑制Clippy正确性检查。

## 实际Linux检查与独核

在新普通用户目录、固定builder、UID1000、网络none、只读rootfs和源码、仅私有out/tmp可写、4CPU/8GiB中实际执行：

| 项目 | 自然结果 | 信用边界 |
|---|---|---|
| workspace格式检查 | exit0 | 没有写入测试源 |
| learning-backup全target严格Clippy，新feature | exit0，`-D warnings` | 不代表全workspace回归 |
| 新feature全部bin构建 | exit0 | 未安装/执行真实manager |
| 默认配置库列举与正文 | 450 passed / 0 failed / 106 ignored | 默认无offhost基础测试 |
| 新feature库列举与正文 | 468 passed / 0 failed / 106 ignored | 包含逐名通过的19项offhost基础正文 |

不相加450与468。feature依赖independent-verifier，实际listing差集为新增19项offhost、少一项仅默认配置的`complete::linux_tests::default_build_cannot_open_or_publish_complete`，所以净差18。106 ignored未在本批执行；不是“全部测试运行”。新增基础19与此前历史准备Rust19是不同来源，不合并信用；也不关闭53项完整分层义务或原37集成规格。

实际批次`c75123c7-77c0-43f1-9195-dce4891ef1c3`，ZIP SHA-256 `79835354b1b4c02ad7095c9ce61d74fb63d35c6ce530a25ee225dbda38bc578d`；包清单`25bc34b16e66fca701488c2ed11a00b455617dc304bef17acf7dfa4371f247f3`；结果`d94051d4e37853468739f20923f1a48a48a4e84948cde80fe47822cb1e645f26`。前后源码全部字节一致，实际builder精确ID确认停止并移除；普通目录、编译产物和证据保留。没有创建PG、网络或卷，没有操作独立目标或运行Windows产品测试。

前两轮保留：`e2807045…`因Docker local日志配置在启动前拒绝，零正文；`9e90af67…`仅格式门失败，零正文。仅按Linux原始差异修正四文件格式后，使用新UUID全量重跑以上七门，未复用失败批次。控制脚本日志大小是事后读取检查，不宣称连续硬输出上限；固定可信builder检查也不替代未来产品有界IO。

四个ELF在测试完成后按精确路径/已观测大小只读散列；该收集不是重跑、独立OS执行时钟或原进程内签名。首次收集以256MiB的测试ELF假定停止，后见两个库测试ELF约287MB，另按精确四文件/总≤1GiB只读收集并保留旧空件说明；**未放宽256MiB产品package上限或任何测试判据**。

代码语义、格式桥与原始执行证据由另一代理限定独核，INDEX SHA-256 `fdda961ea83ad8801e45f1216998e6e4898843a55d3500a4ee2a83b8e3d879d6`。source-sync-manifest记录本片九文件精确摘要及各阶段来源；不是从磁盘结果或历史记录恢复运行authority。

## 接下来

| 子片 | 当前情况 |
|---|---|
| Task1a类型、预算与关闭入口 | 本片限定完成 |
| Task1b固定FD/custody与真实S/D actor | 待实现与相应审查/限定检查 |
| wire/auth/监督、fence及全部writer、真实D证据/副证明、一次消费writer | 后续R2b实现，未接线 |
| B/C key/trust/registry/launcher/安全权限 | 未批准、未安装 |
| D1正常首发与RA、D2源不可访问后的RB、原件负例与回归 | 未获对应执行批准、未验收 |

旧失败/HOLD及Task8规定收尾保持；成功保留fence的已接受代价不变。Task9五门、父Task3/4/5、C4及公开恢复资格保持OPEN，main和生产不变。
