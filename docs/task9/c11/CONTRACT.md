# C11 最小补充编码合同候选：安装后实测确认

状态：DRAFT_NOT_APPROVED_NOT_IMPLEMENTED。本件仅供独立审查及用户决定；不是运行输入、安装授权或验收。现有 37 源文件许可不变。原 14 Wire / 零记录增量合同以及所有旧 RED/HOLD 保持原件，本件获批前不实施下述增量。

## 准确问题与选择

9a31b62480422e302f558f5b7771213bbb13fd84 的 `Enrollment::install_records` 必须写 host export/journal 路径、完整实际 assets mount、legacy 与完整 all-container projection。原 generation inspection 只有 destinations 和摘要，不能逆推这些值。Root 预写 companion record 又会改变 producer、独占写和 binding/descriptor 时序。见独审 `52537c04100879640c7f4157868d5c2e95d8d36a7637dcb8150c9cae868445ab`。

选择保留原 **genesis 后 fresh after → companion records → 第一 work** 顺序；不提前 after 观察，不覆盖 install proof，不给 child Docker/host-proc 或新权限，不用空数组、猜测路径、角色或 hash 代替实测。沿用 API2 原本就要求 lifecycle 发送的 BrokerInstalled，补充一个显式回应，不设置隐式文件轮询握手。

## 仅申请的合同增量

1. 原 `BrokerInstalled` 增加 `binding_id`、`epoch_id`、`descriptor_sha256` 三个 DATA 字段，由实际 lifecycle 在专用安装池自然关闭、真实 frame1 genesis 持久完成后从自身对象取值。调用者不能传入这些值或调用 mark_installed。
2. 新增一个 Root→capture `BrokerInstallObserved` 消息，Wire 类型由 14 增至 15。字段、关联和限额见 MACHINE-DELTA.json；它只回应本原操作的一次 BrokerInstalled，不新增通用请求、参数化 SQL、命令、授权 DTO 或重试接口。
3. 原 Install generation 目录只再允许一次追加 `install-cutover-observation.json`。这是新运行期 record leaf，不是新增产品源文件。该 leaf 由本原 M4 Root job 独占写、自然 sync 文件及目录后才发上述回应；原 proof、inspection、generation-binding、lock 不覆盖。capture generation 不追加此安装记录，既有 runtime-connect-denied 的一次追加规则不改。

准确的 37 产品源文件、13 个主动作、既有 public API 资格边界不增加；私有固定控制面、纯 DATA parser 和响应等待的代码限于已批 source_launch、capture/job/process/session、lifecycle、snapshot_effects/cutover 等既有叶。没有第 38 产品叶、依赖升级或迁移。旧 None 的真实路径和旧记录输出不变。新 B1 reader 不据文件/JSON/ACK 构造资格。

## 唯一因果顺序

1. 原 Root 在 Install generation 发行前实际采集并持有全部 before object。child 通过原 initial fresh admission，持真实 owned session，完成原 install admission close、真实安装/专用池 close、实际 binary epoch header/frame1 genesis。原 1200s total、30s request、120s cutover 等已存在界限不重开、不延长，取其原作用域的更早期限。
2. 原 I0 driver 保留真实 broker、journal、pool/handle owner，发送增加三项 DATA 的 BrokerInstalled。**在收到并核完回应及原 record 前，不返回 broker 给 S5、不接受第一 work、不启动 Q0/capture。** 发送成功不是资格或安装完成证明。
3. 原 Root 只从自己已准入的 Catalog/CID/daemon、实际 before-mapped companion root 与精确 binding/epoch 的固定路径，读取 `journals/<binding>/epochs/<epoch>/epoch.header` 和 `00000000000000000001.frame`。不得使用 caller 任意路径、最近目录、旧前缀或改造旧 Prefix 的 `sequence>=2` 门。新私有 reader 只验证 DATA：完整原二进制身份/长度/序号/前帧/digest、exact 原目录与 held/named 原件、只存在当前 genesis 等；不返回可写 journal/epoch authority，不 adoption/repair/replay。路径和二进制格式须对实际 frozen source 核定，未核定就 HOLD。
4. `descriptor_sha256` 恰为 **frame1 原 payload bytes** 的 SHA256；必须同实际 broker descriptor bytes 相等。`genesis_frame_sha256` 是**整帧** SHA256，`epoch_header_sha256` 是**整 header** SHA256；三域不能混用、不能对 decode 后 map 重编码冒称原 bytes。
5. Root 实际完成原 FreshMounts 的 after 全枚举、daemon/current PG/精确 capture process/code/native/十二 mount source/name/type/RW、旧 producer/RW alias/RO assets 与全 projection/namespace mountinfo 的比较；所有 before 与 after 的稳定谓词仍真实核验。不得删成 project-only subset、把记录里的 running 当停机证明或用 driver lock/ACK/hash 代替物理控制。可信 maintenance window、真实 Root 控制 owner 与两代锁持续保留，不能声称防御恶意 Root。
6. 原 Root 写一次新 record，其中 host_cutover 是以上真实观察的完整 DATA。实际 assets_mount 保存完整原 Docker Mounts 对象及原缺省/存在状态，不能缩成 normalized 五项；Hmount 的五项前像另算。legacy 只能来自当前完整投影严格 filter，真实为空才可空。descriptor-dependent PID/UID/ELF/application/build 仍由 child 自身实际 Descriptor 填原 companion 输出，并核 Root actual child/native/code 的一致性。不得从 host DATA mint owner/session/cut。
7. Root 持续原 owner，通过既有 authenticated 原控制端点发送 BrokerInstallObserved。I0 核 same original job/cookie/backup/request/sequence、binding/epoch/descriptor、当前 genuine ctx 和新 record 的 held/named 完整 bytes/hash/metadata 与 current generation join。S5 完成同原本地十二 mount、mountinfo、held roots、native 和 record 再核后，才用同 Descriptor 与真实 host DATA 填原 install_records；自然 seal/sync/readback 原输出，再允许 wrapper 逃逸/第一 work。hash 与 ACK 单独都不能成功。

Install admission 原 SQL 门在其真实 close 前完成；此后不得复活或重开已关闭的 admin 会话。I0 原 broker/runtime bootstrap 与 genesis 的真实固定门已经由原 retained work owner 完成；本补充不增加 post-close SQL、pool 或 admin factory。Root 的 fresh-after 是实际 host 观察，不是用额外 SQL 代替原物理观察。

## 原预算、异常与版本边界

原 Docker::ids 的 MAX_CONTAINERS=1024 更小界限严格保留，完整 all-host set 超过即 HOLD；以下数组或累计 100000 项上限不覆盖它。新 record 总 UTF-8 canonical JSON 原 bytes 上限 1 MiB；每项字符串/数组/实际 raw assets object 还有 MACHINE-DELTA 的固定更小界限，完整观察放不下即 HOLD，禁止截断或抽样。原 generation inspection 的 <=1MiB、proof<=4096B 均不扩大；这不是共享另一个可 reset 的 1MiB 运行预算。所有新编码/分配、目录项、stat、重复 read/hash/真实 Root 观察都先向各自原 scan/journal/64MiB 子限收费；不换 meter、退款、扩大512MiB总限或给负向收尾无界 tail。新增消息仍每帧<=4096B，累计仍<=262400B；不因两种 profile 共用数字而合并账本。

新消息/leaf、额外或重复字段、错误关联、未知/超时/同步失败、旧残留、重放、丢失回应或任何观察变化都永久否定该原操作，不重发/覆盖/复用 ID，不把晚读到有效 record 补证原消息调用成功。I0 此时可能已经持有真实安装及持久 genesis；失败必须保留原 broker/资源/实际 futures，自然负向 close/join，不能 `?` 后 Drop、假用 Q0/capture、重新安装或回复正向成功。资源清理结果与资格/业务结果分账，未知结算留 HOLD/证据。原 Root 控制与 known child owner 保留至真实结算。

这是 wire 不兼容的准确编码修订；旧 build/缺回应/缺 record 均拒绝，不提供旧 B1 fallback。原物理 C/真正 M4 producer 尚未安装/实测，本合同不能给其一致发行信用。正式启用仍需准确新 ELF/构建针与 C/D 另审；旧普通回归、所有旧失败/HOLD 和 Task9/C4 OPEN 不追改。

## 本次不申请的权限

只申请以上默认关闭代码、对应纯 DATA/负例测试和原普通 Linux 限定检查。**不**生成或安装密钥/信任/launcher，不改 PG/安全权限，不运行真实 Root/PG positive 或备份目标整链，不断联/隔离源、不恢复、不签发 CompleteBackup、不删除旧证据、不合并 main 或部署生产。后续任何这类动作仍按准确目标、影响与最小审批范围单独决定；不索要或保存密码。
