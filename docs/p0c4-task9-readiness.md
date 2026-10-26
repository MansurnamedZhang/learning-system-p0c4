# P0-C4 Task9：独立目标与首签／新发布准备

更新：2026-10-26。状态：**本地准备检查已限定收尾；正式执行条件待具体化，Task9／C4 均未验收**。

用户已确认：**暂时没有独立目标，继续本地准备**。B1草案、37条未执行规格、工具38与Rust19的历史检查及导出复用检查均保留。本轮仅形成可填写的目标事实／审批／执行依赖单，没有新增或重跑测试，没有新的目标／安装状态观察；草案尚未被采纳为新权限合同。

Task8 已按规定私有单机范围完成，发布基线为 `07fa721d9469da7755d2cd2b647aace309fc26f9`；实际产品代码来源仍为 `679b11c3f280546f8d6c20ab15cf7ef59fae3c1a`。本轮不重跑或改写 Task8，也不把单机夹具变成独立存储证据。

依据：[现有施工单 Task9](superpowers/plans/2026-10-04-p0c4-completion.md)、[完成规范 Appendix B/B1](superpowers/specs/2026-10-04-p0c4-completion-design.md)、[父设计](superpowers/specs/2026-09-28-p0c4-backup-recovery-design.md)。以下是原 Task9 的准备记录，不是替代规范、安装配置或运行授权。

## 执行准备单与当前停止点
+
+已整理[目标事实、最小审批与八步依赖单](task9/execution-readiness.md)：目标仍暂无，历史安装权限UNKNOWN不视作实时事实；未来每项记录来源／时间／观察身份／限制。A连接／信任、B托管安装、C B1部署权限、D1正常恢复写入、D2源隔离与第二次恢复分别待实例化／未授权／未执行。模板不产生机器准入或人类批准，不覆盖既有授权，也不按每条命令重复确认。
+
+当前停止在目标事实准备：尚无目标／托管／通道事实，不能开始安装、权限启用、源端隔离或恢复。文档与路线图同步不推进Task9五个正式门，不分配业务UUID，不提供自动执行命令。
+
+## 既有本地检查收尾（25b）

已完成[本地准备收尾清单](task9/local-preparation-closeout.md)：在已发布27fc规范Git字节的新导出中核654MF／655文件，既有38个工具body重跑通过，仓库内及异cwd CLI报告相同、未知参数实际退出1；完整文件集合与字节前后不变。不是38个新用例，不累计76；Rust19仍为历史，37条集成规格未执行。工具十份相关文件／八份读取数据与后续文档提交另做字节桥，实际执行不改名为未来提交。

此前工具38及Rust19等检查点保留在下方；Task9五个正式门与C4继续开放。真实独立目标、托管／安装与B1异机准入仍未具备，没有新SSH、权限、key/trust、服务、断联或恢复动作。

## 历史准备与只读观察检查点

- 已重读 Task9 五项要求与 B1，复核 Task8 最终收尾记录和发布基线。
- 已沿用原测试服务器的既有 SSH 身份完成普通 Linux 用户只读连接；未创建新凭证、提权、安装信任或读取密钥内容。
- 新增[固定路径元数据探针](../scripts/p0c4_task9_metadata_probe.py)及[安全边界测试](../scripts/test_p0c4_task9_metadata_probe.py)。探针只使用 `lstat` 和固定 `findmnt` 元数据查询，无路径／凭证覆盖参数，不调用产品 signer、Docker、数据库、恢复接口或 sudo。
- 六项本地纯 Python 安全测试通过：不打开密钥内容；权限拒绝保持 UNKNOWN；链接祖先停止遍历；拒绝任意／会话路径；丢弃挂载来源及选项；元数据不能产生 Task9 通过资格。这是本地工具测试，不是 Linux/PG 或独立故障域验收。

<!-- TASK9_OBSERVATION_BEGIN -->
已以既有 SSH 普通 UID1000 单次运行已独审的固定探针，实际退出 0、stderr 为空。C4 根目录为 root:root、0700；其下 registry、authority、verifier、trust 和 key 五项均返回 `UNKNOWN_PERMISSION_DENIED`。没有 sudo／其它身份重读，不能据此写成“未安装”。

所在挂载为本机 `/`、ext4；仅是当前路径的挂载元数据，不能证明设备／主机／管理方独立。托管、安装内容有效性、构建公钥 pin 和真实独立目标继续 UNVERIFIED。探针未读文件内容、未写远端文件、未调用 signer／Docker／DB／恢复。

实际观察原件 SHA-256：`49cbe1e00062f97b99f652e07a6bc101341b892c1aefb2df58c83c5f407af600`；探针独审：`dc58aff63594c6fd743f680bedef2430c618355725b97c5d95acbb1e5990d03a`；B1 代码链独审：`2383fdb9150a34b0aeb9a3322cfcbe446d15936db8bd9aab782736a9e0ffea3b`。原件保存在本地私有证据中，不上传 Git。
<!-- TASK9_OBSERVATION_END -->

## Appendix B1 的三个实际接口缺口

| 位置 | 当前代码行为 | Task9 必须补上的连接 |
|---|---|---|
| `ProtectionSnapshot::require_same_operation`；`destination.rs` 的证据持久化 Admit 阶段 | 校验同一个扫描预算、`Arc` 操作上下文、registry 路径及原身份。另进程或异机不能用拷贝字节复原该授权。 | 真实部署中源证据、目标提交操作与独立目的端的准入协议；不能把磁盘 checksum 当授权。 |
| `knowweave-c4-destination` → 新 `try_lock` → `verify_enrolled_destination` | 每个新 lease 的 `OperationAuthority` 为空；`require_id` 在打开 signer 之前拒绝无原操作授权的首次签名。安装密钥或启用所有 feature 仍不足。 | 独立目的端原操作成功后，受审的跨操作／异机首签准入；CLI 当前保持关闭。 |
| `publish_complete_backup` → 自取新 lease → `check_publication_proof` | 原 lease 仍持锁时可能先报 registry busy；原操作释放后，新 lease 仍缺首次发布授权。 | 源端当前保护、目标已提交证据、独立签名与新收据发布之间的原子授权衔接；不能仅重开 registry 或信任 terminal 文件。 |

现有底层同一原 lease 及其合法 clone，在所有证据写入、同步和最终回读真正成功后，可以持有原操作授权；不是所有签名函数都无条件不可用。缺口是独立部署所需的跨边界集成，不能删除 `require_id`、放松同操作校验或公开 capability 构造器来“修复”。

签名还需现有 `independent-verifier` feature、构建中钉住的公钥摘要、固定安装 trust 和独立私钥托管同时满足。目录可见、0600 模式、不同设备号／UUID／容器、有效外观 terminal 均不能单独证明这些条件。

## 当前确切前置阻碍

1. **用户确认暂时没有独立目标。** 真实域验收暂停；以后取得目标时仍须核对运行方、存储／供电／管理边界和授权连接事实。不扫描网络或猜测另一台主机；原单机测试服务器不能自行升级为独立目标。
2. **托管与安装有效性尚未核实。** 元数据不会读取私钥、验证 key/trust 匹配、审查 registry 内容或证明部署二进制已嵌入目标公钥。普通身份遇到权限拒绝时保持 UNKNOWN，不转经其它提权通道重试。
3. **B1 跨操作／异机准入尚未集成。** 上述三处代码断点已定位。需要结合真实目标、托管和允许的管理通道细化协议并独审，现有 standalone verifier 与新 receipt 发布入口继续默认关闭。
4. **正常独立整链尚未运行。** 未签发正常 CompleteBackup，未执行正常恢复或源端不可访问恢复；Task9 五个框均保持未完成。

这不阻碍本地准备。当前不新增持久 signer、私钥、信任记录、网络权限或 service，也不创建一套平行的备份格式／恢复实现。

## 继续实施的原范围

先把三个断点映射到现有 Task3 证据提交／签名和 Task5–7 正常恢复路径。接下来的接口方案必须明确源／目的端各自管理操作的拥有者、锁和预算生命周期、精确备份／尝试／manifest／proof 绑定、异机请求认证、拒绝重放与撤销语义。成功 capability 保持不透明，不序列化、不靠终态文件重建；公开 v2 包／statement／witness／receipt 格式不变。

目标部署事实与通道边界明确后，先独审这一最小集成方案，再按对应 Task3 单元实现与回归；沿用现有全量导入和恢复引擎。当前元数据报告只供人核对事实，产品不读取它，不据它签名、清理、发布或恢复。

## 本地新增交付（不授运行资格）

- [B1最小集成与失败测试草案](superpowers/specs/2026-10-26-task9-b1-integration-preparation.md)：明确四个职责、十一现有seam、最终消费／撤销排序和四个仍待裁决的实施输入；不新增API、认证服务或token格式。
- [37条机器可读测试规格](task9/b1-preparation-test-matrix.json)：11 seam＋6原操作边界＋8绑定替换＋8结果／竞争＋4真实正向链；全部NOT_EXECUTED。七份源码摘要、十一现有步骤及“六个历史故障位置／五个新增纯单测”的精确映射已做结构检查；不是37项测试通过，也没有新增Task9信用。
- 上一检查点新增五个 cfg(test) Store 拒绝测试全部通过（Windows offline/locked，0失败/忽略），另有 rustfmt 和包级 lib/tests 严格 Clippy 通过；只覆盖内存夹具的 pre-action 故障，不是真实FS/跨机验证。为关闭已有 Windows lint，仅将非Linux不支持分支改成同一 Err 尾表达式；权限入口和Linux业务实现不变，旧失败原件保留。
- 既有只读探针及六项本地 Python 纯安全测试沿用上轮原件，不重跑Task8、SSH探针或真实域用例；不把不同阶段的测试数累计成Task9验收数。

## 上一检查点：最终结果与身份替换（2026-10-26）

- 在同一 `destination/attempt.rs` 的 `cfg(test)` 内追加五个测试：outcome返回错误、自洽但backup身份错误、成功登记前撤销、outcome异常展开，以及八个identity字段替换后的最终消费拒绝。
- 前四项复用原Store并加测试专用outcome适配器。所有Store步骤完成、terminal可解析时，错误仍会关闭原操作及其clone上的先前成功授权；残留和无关历史字节保留，换attempt_id的同backup_id重试继续拒绝。适配器不做真实IO、签名、租约重开或网络通信。
- 八个变体位于同一个单测内：backup/attempt、目的端与证据dev/ino、manifest/proof摘要。每个变体先通过canonical完整性解析，再证明不能进入最终消费closure；这不是八条跨机集成用例，也不覆盖尚缺的source/build、registry代际和异机认证衔接。
- 实际Windows Rust1.97 offline/locked：模块19 passed／0 failed／0 ignored＝既有14＋本次5；本轮任务前缀共10个unique body，不把以前的5再累计。格式和包级lib/tests严格Clippy通过，测试源码及执行binary已封存。
- 生产前缀和原14个测试逐字不变；没有新运行时API、wire格式、权限或安装变化。37条Task9集成规格与五个正式验收框原样保持未完成；本次没有Linux/PG、物理FS、独立域或正常恢复验收。

## 已交付检查点：可复用离线准备检查器

仓库新增[离线检查器](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/feat/task7-attention-recovery/scripts/p0c4_task9_preparation_check.py)及[38项纯Python回归](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/feat/task7-attention-recovery/scripts/test_p0c4_task9_preparation_check.py)。检查器本身只使用标准库；固定读取本Git检出中的matrix和七个公开源码文件，不调用网络、子进程、环境配置、sudo、key/trust、数据库或恢复，也不写文件。产品不引用该工具或其报告。

在具有规范源码字节的Git仓库根目录执行：

```sh
python -B scripts/p0c4_task9_preparation_check.py
python -B -m unittest discover -s scripts -p test_p0c4_task9_preparation_check.py -v
```

工具没有路径／权限覆盖参数。有效输入上限每件128KiB、固定八件有效内容合计最多1MiB；每次读取最多额外一字节用于溢出探测，并拒绝已观察到的链接／reparse、非普通文件、字节漂移、重复JSON键、非整数／非finite数字、超过20位的整数、未知嵌套字段或伪通过标签。校验当前schema的37个ID、分组、真实目标要求、11步骤文本映射、七份源码摘要、历史／本轮声明计数及八变体仅一个body。源码或清单版本改变应按对应实施单复审并更新，不能自动刷新摘要或从错误退化为通过。七个已pin源码的CRLF等字节漂移会因摘要不符而拒绝；matrix自身没有外部已审SHA基线，只报告其实际matrix_sha256，合法JSON空白／换行改变仍可能结构通过，不能据此证明等于已审原件。工具不自动改写或规范化文件。

38项工具回归会在自动清理的系统私有TemporaryDirectory中创建合成文件，不修改产品数据；只读限制指检查器本身。实际Windows检查器单测38 passed／0 failed／0 skipped；CLI退出0，报告仅为 `TASK9_PREPARATION_STRUCTURE_CHECKED_NOT_ACCEPTANCE`。38是工具回归数量，不与历史Rust19相加，更不是38个Task9场景。独审发现的指数溢出、嵌套伪权限字段和超长整数缺口已补反例；旧初版与本地封存器失败原件保留。

**证明限制：** 这是同一检出内的文本／摘要一致性帮助器，不是独立可信基线、Rust语义证明、Git commit／binary／私有执行原件复核器或恶意命名空间的完整TOCTOU防护。报告中的19与5均标记为历史声明，`test_execution_evidence_verified=false`、`tests_executed=false`、`Task9_integration_cases_executed=0`、`Task9_accepted=false`固定保留；不读取禁止会话路径或实际安装内容。检查成功不能授予首签、新发布、源端断联或恢复资格。

## 真实目标就绪后的测试矩阵（全部待执行）

| 项 | 原路径与必须观察的事实 |
|---|---|
| 独立域／托管／构建 | 真实部署事实、独立私钥所有权、实际安装 trust/registry、构建公钥 pin 与允许的管理身份一致。不同 UUID/设备号不代替域证明。 |
| 首签正例 | 新源捕获与新目的端资源；目标原操作逐步自然成功，真实独立签名；证据、终态与 witness 原件相连。 |
| 新发布正例 | 当前源保护重新核验；只在有效新发布授权内写完整收据；之后通过现有 `open_complete_backup` 重开。 |
| 可见残留与失效 | pre/post-rename、最终回读、terminal-sync/readback、签名或 receipt 同步失败：原授权失效，保留残留；新进程不能重建首签资格；同 ID 不重试。 |
| 跨操作替换／重放 | 错 backup/attempt/proof/manifest/root、换 registry、失效或复用会话均拒绝；旧成功备份与无关历史根保持。 |
| 真实目标字节 | dump/原件缺失、损坏、额外文件和当前发布根漂移均失败；中断新传输保留以前的完整备份。 |
| 正常全量恢复 | 正常 CompleteBackup → 既有 full engine → 全表、授权、原件、任务归一化及效果证明；不能用 rehearsal 标签替代。 |
| 源端不可访问 | 取得单独审批后，仅隔离本批源 DB/registry/asset/pin；第二个全新目标仅凭目的端／trust／captured evidence 完成恢复，源读取计数为零。 |
| 适用回归与独审 | 仅针对真实改动和新部署运行相应回归，保留实际源码、binary、原件及精确资源收尾；不重复授予 Task8 信用。 |

既有源不可访问结构测试与其它B1覆盖保留历史索引；此前module19检查点的19项是历史Windows原件，本轮仅执行38项工具回归，未重跑Rust。二者都不登记为Task9集成执行，更不能作为真实异机正例。每个实际用例以后另分配全新 UUID／卷／路径并核资源，旧失败／HOLD 不复用、不转绿。

## 动作边界与最小审批包

当前只执行本地审查、工具实现／纯测试、既有身份的固定只读元数据观察，以及正常 feature 文档发布。

下列动作各自停在执行前；此表是待准备的最小审批信息，不是当前授权：

| 动作 | 审批前必须具体列出 | 影响与最小范围 |
|---|---|---|
| 新目标连接／主机信任 | 精确主机、运行方、连接身份、既有凭证是否可用；新 host-key 的核验来源 | 只授权目标上的指定只读查询；不泛化为网络扫描、复制凭证或任意管理权。 |
| 持久凭证、私钥托管、trust/registry 安装 | 精确目标／目录／文件、所有者／模式、公开指纹、密钥生成／保管方、变更前后和回退 | 秘密由人或目标托管设施持有，不发聊天、不写仓库／证据；公钥／信任安装同样属于待审批变更。 |
| B1 认证／服务／权限集成 | 独审后的源码、调用双方身份、允许的窄操作、网络／ACL／service 变更和撤销办法 | 不添加任意路径签名、终态重建、公共 capability、重试或降低 v2 验证的通道。 |
| 源端隔离与正常恢复 | 本批源与全新目标的精确容器／网络／卷／目录、原件保留与恢复连接步骤 | 单独授权源端断联及目标写入；不碰生产、旧失败资源或其它项目，不删除源数据。 |

无需提供或存储密码。真实独立目标和安装／托管事实未核实之前，Task9 和 C4 保持开放；不合并 main、不部署生产。
