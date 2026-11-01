# Task9 已选目标与平台预检

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

## 2026-11-01：真实安装与双机验收方案准备，尚未安装或运行

本轮只完成安装前审查、方案与填报结构、现有访问范围内的只读元数据检查及文档核验；产品源码保持 be147b3dbfd0cf4ecfc126dda4d21dfb3fc0d30c 的默认关闭状态。既有 13 道普通 Linux 检查及 Backup 默认 450 / offhost 649 passed（各 106 ignored）、46 个不同新增正文保留原批证据，本轮没有新增产品测试信用。

[真实安装与双机整链实施单](installation-and-two-host-acceptance-plan.md)已明确六项前置缺口和 A/C0/B/C/D1/D2 边界：Capture 固定 12-mount 与新 policy/fence 读取冲突；15-mount 是待审增量候选，未改代码。B1 固定安装器/受限 native parent 链、真实 key/trust/ELF/anchor 清单、旧 writer 排除、全期 global64/S5/Root/PG/C11 以及真正首签和 RA/RB 仍需后续闭合。

S 普通用户元数据已重新读取；root 私有安装信息权限拒绝则保持未知。D 本轮既有非交互访问权限拒绝，没有取得新现场状态，不沿用历史容量当当前保证。密钥用途分为 S 应用认证、SSH transport 与 D 备份签名三类；未生成、安装或收集凭证。C0/B/C/D1/D2 均未由本次准备授予。

执行顺序为现场事实和准确 C0 合同 → B 托管 public pins → 固定角色构建 → C 安装及真实生命周期证明 → D1 原调用成功/CompleteBackup/RA → 预准入 S2 与规定负例 → D2 狭窄源不可访问/RB → 原始证据独核及适用回归。成功 S1 fence 仍长期保留；不重用 S1 或覆盖 D1 trust 来做后续传输。所有旧失败/HOLD 保留，Task9/C4 开放，未合并 main、未部署生产。

以下为按日期保留的历史检查点，不能作为当前安装状态或新增权限依据。

---

## 最新检查点：Task2c签名DATA片 · 2026-10-27

Task2c完成固定SourceAuth/SourceFinalDecision的签名DATA验证片，新14项/offhost89逐名实际通过并独核；默认450通过/106忽略，特性538通过/106忽略分别计；两配置不相加。格式、严格Clippy、bin构建通过。

固定公开RFC夹具只证明单帧DATA及原owner错误/unwind后的粘性拒绝；没有运行密钥来源、跨操作真实认证、原S时钟、Child/SSH、Source/D权限、fence或一次消费发布。两个runner仍拒绝。

SourceActor准确32叶补充编码范围已获人类批准；默认关闭，尚未接入和现场安装。key/trust、Root launcher/PG配置/安全权限、目标整链、源隔离和恢复仍另批；Task9/C4五门保持OPEN。

---

## 最新实施：Task2b有界管道IO限定完成 · 2026-10-27

五叶完成原预算预付、实际管道IO、绝对deadline与EOF分账；新增21正文通过，offhost75逐名核对。默认450/特性524各106 ignored分账，格式/严格Clippy/bin构建通过并独核。新增由4单元、16原生管道及1EINTR分类组成，不是真实信号或Child/SSH/auth证明；runner仍拒绝。

SourceActor生产capture、Root发证、固定peer会话与跨crate预算扩码合同仍需独审与范围批准，现场key/trust/权限、目标整链和恢复另批。Task9/C4 OPEN，完整证据与边界见[实施进展](r2b-implementation-progress.md)。下方为历史检查点。

---

## 最新实施：Task2a纯协议编码片限定完成 · 2026-10-27

五叶提供严格字段、帧/阶段、双向计费与候选摘要；新增27正文实际通过，offhost累计54。默认450/特性503各106 ignored分别记账，格式/严格Clippy/bin构建通过并独核。净差53并非计划53项分层义务；所有输出仍Untrusted/Candidate，无真实IO/认证或发布权限，runner始终拒绝。

SourceActor生产capture、Root发证、peer认证与跨crate计费仍需准确合同和新增范围批准，不暗改旧guard。现场key/trust/配置权限、目标整链与恢复另批，Task9/C4 OPEN。完整原件来源、保留BLOCKED/RED与未验层见[实施进展](r2b-implementation-progress.md)。下方为历史检查点。

---

## 最新实施：Task1b元数据FD片限定完成 · 2026-10-27

固定路径元数据、held FD与原lease累计预算重读已实现；普通UID新批次新增8正文、offhost累计27逐名通过，默认450/特性476各106 ignored分账，格式/严格Clippy/bin构建通过并独核。Root正向完整holder、全部64FD及真实S/D authority未验。runner仍始终拒绝；Task9/C4保持OPEN。

先推进已批准的纯wire/IO；capture生产布局、Root Unix认证与旧guard兼容、内部UUID独立Root发证仍须准确接线审查。新增跨crate/Root生产入口不视为已批准；key/trust/现场配置、目标整链与恢复另批。完整证据与保留RED见[实施进展](r2b-implementation-progress.md)。下方保留旧检查点。

---

## 最新实施：Task1a基础片已完成限定Linux检查

九文件默认关闭入口、严格policy输入与累计预算基础片已实现；新增19正文逐名通过，默认库450/特性库468各106 ignored，格式、严格Clippy和bin构建通过并独立核对原件。计数分别记账，不能与旧准备Rust19相加，不能作为53完整义务或37集成的通过数。

新runner即使pin完整也明确integration incomplete，尚无FS/PG/stdin/SSH/key/sign调用。固定FD/custody loader、真实S/D actor与后续wire/fence/发布仍待实现；安装、安全权限、源隔离及恢复仍未批准或执行。详细来源与前两轮保留RED见[实施进展](r2b-implementation-progress.md)。Task9/C4仍OPEN，下方批准时“尚未实现”保留为历史时点。

---

## 当前检查点：R2b准确编码范围已批准，开始逐任务实现

已把源码影响、原S/D操作、固定协议与预算、D session持久空间、精确摘要对象、snapshot-effects副证明和全部writer/fence门控列入[准确实施计划](../superpowers/plans/2026-10-26-task9-r2b-implementation.md)。[wire profile](../task9/r2b-wire-profile.json)与[53项分层验证义务](../task9/r2b-validation-plan.json)均为未执行候选，不是程序配置、测试正文数量或53项通过；原37条历史规格保持原字节。

首轮独审提出的D session累计容量与Ready摘要原像缺口已修订；S只重算自己的输入，并核对原D返回的有界副证明字节，不声称独立重算D物理结果。保留先前备份的负例还须在未来C/D设置中预先准入S2，保留S1 fence、D1原trust/reader及完整备份字节。

**首版即使正常成功也不自动解除源fence**，可能长期阻止该源registry的新备份及相关管理变更，甚至影响其他组；异常解除另审。用户已明确批准默认关闭的R2b代码和既有Linux普通用户隔离builder限定检查，准确边界见[批准记录](r2b-implementation-approval.md)。真实密钥/信任/registry/launcher/权限安装、目标执行、源隔离和恢复仍分别暂停待准确批准。

此次无新产品代码或测试，不重新盘点S/D安装，不把旧平台元数据升级成当前事实。Task8规定范围保持已完成；Task9五门及C4仍OPEN，main与生产保持原样。此前“待列清合同”的文字保留为历史检查点，现行准确范围见上方计划。

---

## 当前状态补充：时序语义已采纳，现场事实未更新

2026-10-26 用户选择“采纳方案，仅定稿规范（推荐）”，见[采纳记录](b1-final-decision-adoption.md)与[完成规范](../superpowers/specs/2026-10-04-p0c4-completion-design.md)。原提案 `2448f459…` 继续原字节保留；其中未采纳状态描述审批前时点。

这次只推进规范定稿/独审，没有重新连接S/D或运行测试，不能把此前点时元数据升级为新的现场安装观察。目标平台六项、Linux无权限帧20正文与各自原件/限制原样保留，不相加或新增验收信用。管理账户与自动transport身份分开；key/trust/registry、源身份/权限、fence落地、源隔离与恢复均未获本次批准或执行，Task9/C4未验收。

> 下文“最终决定仍待采纳”属于此前准备检查点；最新语义决定以本节及采纳记录为准。旧失败与HOLD不变。

---

更新：2026-10-26。**状态：目标已选、限定准备已执行；独立域/安装/B1/恢复正式验收未完成。**

## 当前事实与证据层级

用户指定与开发源端分开的腾讯云 Linux 主机作为备份目标，并确认管理该主机和云盘。备份签名私钥的方案为仅保存在目标端 root 私有目录；这不批准生成/安装 key，也不批准新增管理认证凭证、trust、registry、安全权限、源隔离或恢复写入。

目标普通账户实际 SSH 观察于 2026-10-26 08:47:48 UTC（北京时间16:47:48）：Ubuntu 24.04.4 LTS、x86_64、腾讯云 CVM、2 CPU、约2 GiB内存。单块50 GiB虚拟云盘、根ext4，`/var/lib`所在文件系统当时可用39,425,306,624 bytes（约36.7 GiB）。这些是点时元数据，不证明云存储底层故障域、备份空间峰值或硬件断电持久性。

当时 `/var/lib/knowweave-c4` 及其固定 registry/verifier/key/trust 路径不存在；这是此次目标普通身份元数据观察。此前源端私有目录 `UNKNOWN_PERMISSION_DENIED` 的旧观察仍保留，不能把它改写为源端未安装或借目标身份重读它。

目标 Docker 盘点在08:48:29 UTC看到已有MySQL和Redis运行，另有退出的测试数据库容器；本轮未修改任何已有容器、卷、网络或服务。未在目标部署编译工具。目标机只读盘点与后来普通账户新隔离目录中的合成预检分开记账。

公开文档不包含连接密码、私钥、token、实际IP或账号；完整路径/非秘密资源归属保存在私有运行单。原始密码未写入项目脚本、配置、Git或本批证据。

## 普通账户文件系统预检：6/6

新建普通账户私有隔离目录，保留少量合成文件与脚本，不使用sudo。实际通过：

1. 排他新建、完整写入和文件fsync。
2. `renameat2(RENAME_NOREPLACE)` 对已有目标拒绝，保留两边原字节和身份。
3. 对新目标成功重命名、保持inode并同步目录。
4. `O_NOFOLLOW` 拒绝合成symlink。
5. 持有文件描述符的原inode与完整字节回读。
6. 当前隔离根身份保持，根目录/父目录同步。

这是实际目标上的原生Linux平台检查，不是Rust产品、root固定安装根、云后端灾难持久性或37条Task9集成用例验证。`/home`与`/var/lib`当时同device的观察只作位置参考，不能把普通账户目录当作已安装的root管理根。

| 原件 | SHA-256 |
|---|---|
| `probe.py` 实际执行源码 | `337245461fb2f6004613d54bb691cd10826e4ce757b1bf4459afda091d90cdea` |
| `result.json` 六项结果 | `41e9db4abdaf6ab954dc847d31f1b76eade864c898f181e7af88e402524e1fae` |
| `target-readiness.json` 私有准备计划 | `f97b1886d17babb3e15f890f72d0862e04157de71660c0e12f209705fa52d912` |
| `NEXT-STEPS.md` 私有下一阶段单 | `660f15fb138a0b1d6184a87e6fcb39d8ef19a9bbe896e4875c66eda81612d01c` |
| `ROOT-READBACK.json` 原会话自查收据 | `bb12d70fc0c95af635bd4ad2a94a46dc9e0e710273a61b295d45890afb59c90f` |

Root实际回读四份文件，核对摘要/普通文件身份以及写入计划前的七项目录清单。它是同一Root会话的自查，**不是另一观察者对远端原文件的独立审查**。本轮后续B1设计独审也不被回填成当时平台预检的独立现场见证。旧私有计划中“独审代理额度受限”的说明是该时点状态；后续同一代理已接手设计独审，未重写旧原件。

## 已关闭的准备缺口与仍未满足的条件

- 已有选定的异机目标、可用管理连接和用户确认的托管方案；不能继续把“尚未提供目标”写作当前状态。
- root私钥、trust/registry、构建公钥pin和源manager认证身份仍未安装/核实；普通文件权限和SSH账号不单独证明独立托管或原活操作。
- B1具体接入候选见[异机活操作RFC](b1-offhost-admission-rfc.md)。旧 `finalize_use` 不是一次消费；旧publisher的本地根/新lease路径也不能接上远端IP直接使用。
- S原manager的可信独占身份、跨机死亡/撤销线性化及D最终消费仍是阻断。心跳、最后消息或持久负fence都不能未经规范裁决替代原S活性。
- 历史[37条规格](b1-preparation-test-matrix.json)保持原字节与未执行状态；其中旧 `independent_target.available=false` 属于c1时期准备输入，**不是实时目标盘点**。现有离线检查器也不查询当前目标可用性，不因结构通过授Task9资格。
- 下一步按[执行准备单](execution-readiness.md)和RFC分层推进；安装、安全权限、源隔离和恢复仍需各自具体审批。

正式Task9五门、C4、正常CompleteBackup/RecoveredInstance与生产保持未验收；Task8不重跑、不重新记账，旧失败/HOLD继续保留。

## 后续：无权限 Rust 帧准备的 Linux 检查

子代理仅新增 `scripts/rust/task9_frame_preparation.rs`，顶部 `#![cfg(test)]`，不被产品crate或Cargo工作区导入。它只处理不可信的固定header、长度/预算、序号和语法流关闭，不接文件/网络/密钥/真实根/认证/活性/首签或发布。Metadata payload未做JSON语义认证，上游真实IO的分配界限也未实现；不能把纯 `finish` 当实际socket EOF。

| 项 | 结果与证明边界 |
|---|---|
| 最终源码 | 19,585B；SHA-256 `4265db3b16386b70132e429786f6729d287f5a490f9a7b6c6de5b3863b8c3bc1` |
| 独立代码审查 | 限定允许无权限framing；不批准B1 authority、安装/启用或Task9 |
| 实际Linux批次 | `902dabe2-4c23-4cd3-a97a-d46ec25cb089`，既有开发源端固定builder，不在Windows执行 |
| 工具链/格式/编译 | Rust1.97.0；本文件rustfmt检查、`rustc --edition=2024 --deny warnings --test`均退出0 |
| 列举与正文 | 列举20；实际20 passed、0 failed、0 ignored；两阶段分别有原始日志与退出码 |
| 实际结果原件 | `d98efdaf4ca23d4549dc63fa3f12ab22bebff4332e38625a46b3bfc1f5804ccf` |
| 隔离/收尾 | 新私有目录；driver检查固定镜像、无网络、只读源码/根FS、UID1000、2CPU/1GiB；精确ID删除builder，源码/日志/二进制保留。完整前后Docker inspect原件未封存，派生身份摘要不能冒充完整原件独证 |

首次尝试在创建批次/上传之前因原实验父目录0775被guard拒绝；只读核实后使用新私有目录，没有放宽guard或改旧目录权限。第一实际批次 `90ad0db8-2821-4db7-9fcd-c0df1afa5908` 的format退出1，编译/列举/正文未运行，源码为18,129B/SHA `5fd2988d72154cbaa58f2443032e95ee091383a79cf10fc1cbdf1019872541c0`；失败结果 `dece26516037ea738f54f55b94572c0bc4b8891aa23c76c52ef0b637987a6ae9` 保留。之后仅应用18处真实Linux rustfmt差异，核心与20正文逻辑不变，使用全新UUID重跑。

20是实际执行的纯解析器测试body，近512MiB预算/溢出测试使用私有白盒计数器，不是512MiB网络传输。没有执行Cargo工作区、Clippy、PostgreSQL、37条B1集成、首签/新发布或恢复；不把20与历史19/38或目标FS6相加。原七份产品源码pin、十份旧工具字节桥和37条历史规格保持。正式门不推进。

另一子代理随后仅以既有SSH身份只读采集上述两个Linux批次，独核34份原件的普通文件/所有者/链接/前后元数据与全量SHA；其中33份非binary作字节副本，7,084,816B测试binary只在128MiB上限内分块hash、未复制或执行。20项列举和正文逐项一致，旧format失败不计通过。独审INDEX SHA-256为 `d896ab257a894daedd14e43ec528dab8a1c22c8d188a1a1099306ad7bd1d5276`，结论仅为20个实际Linux纯parser正文的准备范围。

该独审没有补造旧测试进程/时钟或完整Docker原件；阶段现有exit+双流，不具备独立OS process原件。新SSH采集收据只见证此次只读采集，不能冒充当时运行观察。没有再跑测试，也没有获得Task9、B1、workspace或Clippy信用。

## 后续检查点：最终决定合同仍待语义采纳

[最终决定时序合同建议](b1-final-decision-contract-proposal.md)已封存，SHA-256 `2448f4592a64bc9bc5ec5b2e618f936b8f47412c2ab0c23526ae3623d6b7ae59`；状态仍为 **NOT_ADOPTED / NOT_IMPLEMENTED / NOT_AUTHORIZED**，提案正在审阅，尚未采纳。当前下一步是明确是否接受其中的S死亡窗口、D一次消费、负fence及UNKNOWN/历史只读分账语义，不是重复增加parser。语义采纳也只允许后续正式规范定稿及独审，不授权R2b权限编码或机器配置。

补充的目标观察来自Root工具结构化元数据回报（2026-10-26 11:56:38 UTC），不是另一观察者的独立原始采集或独核：管理身份可访问本机系统Docker Unix socket，daemon info未报告rootless。不能把该管理身份默认当作受限自动transport身份；Docker官方说明docker组具有root-level权限，这是官方权限模型，与上述点时观察分别记账。[Docker官方权限说明](https://docs.docker.com/engine/install/linux-postinstall/#manage-docker-as-a-non-root-user)。未据此实施root容器、密钥读取或恶意管理员防护验证。

D key/trust/registry、负fence namespace、S管理认证和受限传输身份、固定调用权限，以及实际首发、源隔离和恢复仍须具体方案、独审及对应执行审批；管理登录或托管方案选择不覆盖它们。Task9五个正式门、37条历史规格及旧失败/HOLD不变。本次仅追加文档检查点，没有新测试或验收信用。
