# Task9 本地准备检查与收尾

2026-10-26。状态：**已列本地准备项完成检查与收尾；Task9／C4 正式验收仍 OPEN。**

用户已确认暂无真实独立目标。此页只收束当前不依赖该目标、无需新增权限的准备成果，不采纳新的首签／发布权限合同。依据仍为[原施工单 Task9](../superpowers/plans/2026-10-04-p0c4-completion.md)与[完成规范 Appendix B/B1](../superpowers/specs/2026-10-04-p0c4-completion-design.md)。

## 本地检查表

| 准备项 | 实际完成范围 | 仍不具备的资格 |
|---|---|---|
| B1 接口缺口与方案草案 | 三处代码断点、四职责、失败／撤销排序和四项实施输入已形成文档 | 未实现或启用异机准入；原默认拒绝保持 |
| 当前目标与安装事实 | 用户确认目标暂缺；既有普通身份私有安装元数据 UNKNOWN 的历史记录保留 | 本轮未重探；不把权限拒绝当未安装或独立性证明 |
| 37 条集成规格 | ID、分组、真实目标需求、源码与覆盖引用已校验 | 全部 NOT_EXECUTED，未发生 Task9 集成执行 |
| 原操作纯回归 | 历史 Windows Rust19／0失败／0忽略，仍按原检查点留证 | 本轮未重跑 Rust，不是 Linux／物理FS／签名／恢复证据 |
| 可复用离线检查器 | 既有38项Python工具回归；新导出中相同38个body重跑通过 | 不累计76，不验证独立域或历史私有执行原件 |
| 脱离原工作目录复用 | 规范Git字节导出、654MF、655全路径／字节前后核对、两种cwd正例及真实参数拒绝 | 只证明本地分发与调用方式；不证明OS沙箱／安装／部署 |

## 本次实际导出检查

执行源码提交：`27fc0371c2d797c60b66fe911a6b0896ff200bfd`。本地Git对象导出 **655个普通文件blob**，包含`.gitignore`与`.gitattributes`；不包含`.git`、未跟踪内容、凭证或编译缓存。导出内容合计 13037857 字节。逐项核对原654条source-sync-manifest记录，另核manifest自身的Git blob及SHA。记录Git mode，但未证明Windows上的可执行位、原Unix权限或OS出生／命名空间身份。

导出目录内运行既有38个Python工具body：实际 **38 passed／0 failed／0 skipped**，没有新增测试正文。该执行与此前38项是相同用例重跑，分别留证、不相加。其后实际执行三条CLI探针：

| 探针 | 实际退出码 | 核对结果 |
|---|---:|---|
| 导出仓库内的相对脚本路径 | 0 | 结构检查报告，Task9_accepted=false |
| 无源码的另一个cwd＋绝对脚本路径 | 0 | 与前一个完整JSON报告相同 |
| 未支持的`--allow-signing`参数 | 1 | ARGUMENTS_NOT_SUPPORTED；Task9_accepted=false |

日志保存在导出根外。controller在执行前后完整枚举并比较目录：**655个文件的路径集合、普通文件类别及字节SHA全部相同**，比较包括新出现的文件。两时间点inventory只在controller内比较，没有各自另行落盘；独审对当前655文件逐件回读，确认与Git及初始导出roster相同，不能冒称分别独立观测了两个时间点。三个CLI探针不计作三项Task9集成场景。测试使用库管理的私有临时合成文件，不修改产品数据；完整前后清单只覆盖该导出，不是全主机文件系统审计。

本次结果原件SHA-256：`c850f941fc453f03aa88d117d0deecfd0661ad7fc9e05e593402bc29099b0f0c`。导出索引原件SHA-256：`b325eefb2f3eda34cf9a31cc810693148183bae1ec5241f9970b65c5258bf4b6`。原始双流、退出码、导出与目录清单保留在本地私有证据中，不上传Git。它们是本地进程记录，未增加独立OS执行见证或私有历史日志／binary证明。

## 与后续文档同步的字节桥

实际执行来源始终是上面的27fc提交，不能重标成后续文档提交的新执行。工具相关的**十份文件**是checker、测试、matrix和七份源码；其中checker CLI真正读取的是**八份数据**（matrix＋七源码）。本次文档／路线图同步要求这十份文件与该执行逐字相同；发布前后另核ref、完整tree和改动blob，不由本页预先授予发布成功或Task9资格。

| 文件 | 字节 | SHA-256 |
|---|---:|---|
| `scripts/p0c4_task9_preparation_check.py` | 16505 | `3770babece9b15dbc1e5947e5b54288b28034ad1a21c662761e75da5f6609450` |
| `scripts/test_p0c4_task9_preparation_check.py` | 9965 | `b022a2363cb85145cafa6c4b422e6d3df110c4de75284208cb1fa41f7c9c7907` |
| `docs/task9/b1-preparation-test-matrix.json` | 31920 | `4eea0c2b2d4fd6a101db1b117a7553fb05f61f3b00af66f555606f4c168e4bcd` |
| `crates/learning-backup/src/destination/attempt.rs` | 31190 | `299c70aeb207573ae00341306e5c6d5875453b56ef70ebc181e9e29d263d3c67` |
| `crates/learning-backup/src/destination.rs` | 95168 | `507212af06baeba07103d89b01e14f99daf5bed042e47b76ebc67ac9813c3b8a` |
| `crates/learning-backup/src/protection.rs` | 60142 | `3a6bbbb7faefdce876a6a68c9f4d3c461ead84a4784c0854851a666b704aa5d5` |
| `crates/learning-backup/src/registry.rs` | 47069 | `ae50d7942b922bc1879f6aab613e8f70eab39537afa590f15a415e92429c4abb` |
| `crates/learning-backup/src/complete.rs` | 22240 | `405e1f7e4bacb9fcb0cd395f16171ee8bdac2e1a21fe3e7fddabc923effa5019` |
| `crates/learning-backup/src/bin/knowweave-c4-destination.rs` | 1192 | `b5949c38fccb81906882dbceb6153190aa0364b72404e910721ed1a3aab51381` |
| `crates/learning-backup/src/full_restore/data/rows.rs` | 27837 | `30a4ea7f146477cb97fa3e72cdcbbbdbdf3628066adccc7ec1138c9442748212` |

离线checker仅做当前检出中的声明／文本／摘要一致性检查：不验证Git提交可信性、外部已审matrix摘要、Rust语义、历史执行原件或独立故障域。matrix合法空白变化可仍结构通过，报告只给其实际SHA；七个已pin源码的字节漂移则因摘要不符而拒绝。既有B1草案和v2格式不变。

## 可复用命令与确切剩余阻碍

在规范源码字节的源码目录运行，无需`.git`或新凭证：

```sh
python -B scripts/p0c4_task9_preparation_check.py
python -B -m unittest discover -s scripts -p test_p0c4_task9_preparation_check.py -v
```

命令与输入限制详见[准备文档](../p0c4-task9-readiness.md)。本地检查器总是`Task9_accepted=false`；用户未提供新目标事实前，不扫描网络、不重读被拒绝的私有安装路径，不新增凭证、trust、权限或服务。

剩余工作按原Task9五个正式门保留：真实独立目标与托管／安装事实；依据实际通道独审并实现B1首签与新发布准入；正常完整备份及恢复、经单独审批的源端不可访问恢复；适用真实回归与独审；最终父任务与C4收尾。现在前三项所需的真实目标／托管事实和准入实现尚未具备，不能用本地导出替代。

任何后续密钥／信任安装、持久凭证、网络／ACL／service权限、源端断联或恢复写入，仍须先明确精确目标、影响和最小审批。旧失败／HOLD、Task8限定结论与全部历史原件保留；本轮不合并main、不部署生产。
