# Task9 R2b实施进展：Task1a基础片限定完成

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
