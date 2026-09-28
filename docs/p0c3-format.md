# P0-C3 v1 目录格式

状态：`P0_C3_VERIFIED / NOT_PRODUCTION`。Task 7 联合 Linux/PostgreSQL/容器验收与独立证据审计已完成；[验证台账](p0c3-verification.md)记录精确源码包和结果。本页描述实际接口，不单独充当验收证明。

输入为 `SnapshotRequest { reading, mode, include_personal, include_originals, resource_versions, source_segments }`。`reading` 精确包含 view/revision UUID；显式 resource/version/segment 引用不能靠可变 head 推断。`include_personal=false` 产生阅读副本；原样导入要求本人个人层和完整可见的必要闭包，隐藏必要对象使整个 exact 请求失败。

## 两种不可混淆的 capability

`manifest.json` 是 canonical JSON，SHA-256 是**完成封装后的实际 manifest 字节**摘要；不能使用 planner 的早期清单代替。它不把自己列入 files。每个 files 项为 `{path,size,sha256}`，按路径排序且不得重复，size 是实际字节数，sha256 为小写 64 位十六进制。

| capability | manifest 其余字段 | 文件 |
| --- | --- | --- |
| `exact_import_v1` | `format_version:1, root:{view_id,revision_id}, files, requires_destination_assets` | `objects/<table>/<identity>.json`、可选 `assets/sha256/<前两位>/<完整SHA>`、`validation.json` |
| `reading_copy_v1` | `format_version:1, copy_id, files` | 经授权裁剪的 `reading.md`、`reading.html` 及 validation；不能送入 exact import |

`validation.json` 由封装器写入并进入最终 manifest。文件级摘要绑定完整 row 包装字节；`SnapshotRow { table, identity, immutable_values, sha256 }` 内部 sha256 绑定 canonical immutable_values。正文自身业务摘要另行校验，重算外层摘要不能让非法正文、依赖、身份、审查语义通过。

identity 按固定主键顺序编码为 `u-<规范UUID>`、`k-<reference kind>`、`p-<无前导零u32>`，多段用 `__` 连接。路径必须反向解析后完全一致；不接受绝对路径、`..`、反斜杠、Unicode/大小写别名、任意表名或任意 SQL。时间用 UTC RFC3339 微秒规范化。

29 张固定表：asset、resource、resource_version、source_segment、block、block_revision、block_asset_use、composition、composition_revision、composition_occurrence、overlay、overlay_revision、overlay_group_identity、overlay_placement_identity、overlay_group、overlay_placement、placement_manual_decision、reference_object、reference_dependency、relation、relation_revision、relation_review_head、relation_review、epistemic_stream、epistemic_review、reading_view、reading_view_revision、reading_relation_selection、reading_epistemic_selection。它们是白名单，不要求每个包有每张表。附属行身份、位置顺序和 registry 完整集合都要验证。

明确排除 app_user/space/grant、普通 request/receipt、release/lineage、outbox/job/导出结果与目标导入 receipt。可变 head/published/last_release 不作为 immutable_values 迁移。目标必须预置同 UUID 的身份、空间和所需授权。

## 预算

| 项目 | 上限 |
| --- | ---: |
| 必要对象 | 2048 |
| 必要引用边 | 4096 |
| 引用深度 / composition 深度 | 32 / 16 |
| composition occurrences | 4096 |
| 合计正文 | 8 MiB |
| 包内文件数（含 manifest） | 2048 |
| 单 JSON / 合计 JSON | 8 / 64 MiB |
| 单个 / 合计 included originals | 128 / 512 MiB |

原件按 storage digest 去重复制，不抹掉两次使用的各自逻辑资产/usage 身份。128 MiB 单文件和 512 MiB 合计限制仅用于随包携带的 included originals。`requires_destination_assets=true` 不携带原件，因此允许超过 128 MiB 的资产声明，也不消耗 included-originals 合计预算；大小仍须为非负、可表示的 PostgreSQL bigint。目标必须拥有**同 space + 同 asset ID**、相同元数据和真实 hash/size 字节，并逐项验证；另一个资产恰好同 SHA 不算满足。metadata-only 规划只读取授权元数据，导入 preflight 仍读取并验证真实目标原件。

## 本地安全入口

`stage_incoming` 只接收已打开 Read 流；服务端选私有根，输入路径仅是包内逻辑文件名。Linux 私有 0700 根经 retained directory handles/O_NOFOLLOW 操作，完成前不发布 ready，seal 后重新验证长度/hash/文件全集。delivery 返回 sealed 文件句柄，不返回可绕过授权的公开路径。root/job/attempt 是服务内存储索引，绝不是授权令牌。

Windows authoritative staging/delivery 当前 fail closed，未实现等价 ACL、共享规则和持久目录发布；不能据 Windows 编译声称支持。ZIP 只是源码交付媒介，不是本格式的解包入口。
