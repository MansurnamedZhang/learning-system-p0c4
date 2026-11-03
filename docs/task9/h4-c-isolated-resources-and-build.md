# Task9 C 隔离资源与目标端构建

2026-11-03。S 为源端开发机，D 为独立备份目标。专用账号 `2401/2401` 已核实，B 三把密钥均有封存证据。原 B 批次 HOLD 单独保留。

## 两端隔离资源清单

| 范围 | 已核对或拟用 | 实装前需补齐 |
| --- | --- | --- |
| S1 与 S2 | 两个独立 UUID、候选根、PG 项目、数据卷、Unix socket 卷及内部子网。S 本机已有锁定的 PG18 镜像和固定 Rust builder。最初一个候选子网与实际路由及 Docker 网络重叠，已淘汰并只读复核替代子网。 | 每个实例分别取得真实 DB OID、PG system identifier、owner、socket、容器与卷身份，冻结 control 根、实际 15 项 Capture 挂载及源端编译绑定。PG 使用私有 Unix socket。 |
| D 的 V1 与 V2 | 两个独立 UUID 和候选存储根，分别承接 S1 和 S2。每个视图分别需要 registry、session namespace、证据/备份根以及 policy、trust 和 launch manifest。 | 固定路径须在各自实际隔离视图中取证。session 目录要求 `anchor.json`、`lock`、`operations`、`staging`。root 身份和 registry 锁必须取实际值。 |
| RA 与 RB | 两个独立 UUID、候选根、PG 数据卷、Unix socket 卷与内部子网。D 的本地路由和 Docker 网络对候选子网未见冲突。 | D 缺少锁定的 PG18 镜像。先另行批准精确镜像暂存与隔离恢复资源，再复查。RA、RB 顺序运行，正式恢复和源端不可访问验证各自另批。 |
| 隔离目录 | S 创建一个批次根与两个空案例子目录。D 创建一个批次根与四个空案例子目录。八个目录均已回读 `root:root 0700`。 | 已取得 backing root 身份。挂载、固定视图、数据库、网络和信任待后续定稿与审批。 |

S 当前有 24 个逻辑 CPU。D 当前有 2 个逻辑 CPU、约 1.18 GiB 可用内存和约 36.63 GiB 可用磁盘。D 拟按 manager 512 MiB、单个恢复 PG 384 MiB、保留 256 MiB 估算，实际运行前需要重新评估可用内存和留存预算。D 的 22229 当前无监听。云侧映射和源端实际可达性待验证。

Capture 合同同时约束 fence 的主机源路径和容器目标路径。S2 需要单独证实自己的固定路径视图，S1 已成功或留存的 fence 与 registry 继续保留。D 的 V2 也需要单独证实固定路径和受限入口。目录候选只分配物理落点，后续仍须证明应用和 Docker 实际看到的身份。 [Capture 挂载合同](capture-mount-amendment-contract.md) · [固定路径实现](../../crates/learning-backup/src/offhost_b1/source_launch.rs)

## 隔离目录创建回读

按批准的八目录清单，两端均先以不跟随符号链接的目录描述符核对 `/var/lib` 设备及 inode，并验证候选根不存在。随后独占创建。S 创建 3 个目录，D 创建 5 个。最终回读确认 `root:root 0700`、目录名与目录描述符身份一致，两个批次根只包含各自的案例子目录，6 个案例子目录均为空。目录和父目录已同步到磁盘。

创建回执 SHA-256 `81d03b5705488499e1beccbc8f6dabdc5100cee928fa3c5c4704d5792aa50016`。已把 6 个案例的实际 backing root 身份接入后续资源清单，SHA-256 `c5e4b6a9a39b5cbae5291b21ac1343bf371513d4b208e0dd677b2fa0a754b92b`。本轮写入范围限于这 8 个目录。固定路径、registry、fence、PG 及 socket 身份继续等待现场取证。

## 候选资源复查与源端构建准备（目录创建前）

用两端既有管理入口重新只读检查。两处 `/var/lib` 父目录身份匹配，候选根均未创建。S1/S2 和 RA/RB 的候选 Compose 项目名、卷名、网络名和子网未发现冲突。S 有锁定的 PG18 和 builder，D 的锁定 PG18 镜像仍缺，22229 本机监听数仍为零。D 此次约有 1.19 GiB 可用内存和 36.62 GiB 磁盘。容量、命名和路由均须在获准操作前重查。两份私有复查回执的 SHA-256 为 `20962a09b244e0fbd3cbd8a2b45c3baecb63a6fe9e9f5884a5fb9749f2e44c33` 和 `59ff4186cc9cd16f07d4195f3ad5b775a48cfb48ce345d5c9c04907c121a7444`。

源端构建输入合同已核对六份代码文件，固定 Linux/amd64 builder 和 Cargo.lock，并封存 S 的 `/usr/bin/docker` 候选摘要。正式构建时还要从实际隔离视图回读 CLI 身份。源端每个案例分别构建 Root launcher、publication source 和 Capture，逐角色固定编译标记和输入摘要。应用 commit 与构建归档的摘要需要独立取证，与 manager 的 commit 和构建输入分列。合同摘要 `dd382093045ef9172ab7883dc323c561bc4d248aad353d6f77fd93ec742078d8`。 [角色与编译标记](../../crates/learning-backup/src/offhost_b1/mod.rs) · [Cargo 二进制目标](../../crates/learning-backup/Cargo.toml)

`source-binding.json` 的九个字段使用排序、紧凑且无尾换行的 JSON 字节，SHA-256 进入编译固定值。实际数据须来自该案例的 control 根和 PG18，另记录独立数据库身份、owner、postmaster 启动时间、Unix socket 及各角色的真实 `peer:root` 登录主体。观察值与文件字节分别核对。正式源端 ELF 继续等待现场绑定。 [绑定解析规则](../../crates/learning-backup/src/source/binding.rs) · [PG 会话核验](../../crates/learning-db/src/snapshot_broker/b1_peer/session.rs)

## 目标端 ELF

在 S 的固定 Linux/amd64 builder 内使用只读构建输入、无网络和普通用户构建。Rust receiver 的真实输入包括 B 的源端应用公钥指纹、D 签名公钥指纹、manager 源码提交、构建输入摘要与 `task9-b1-limited-v1`。目标端编译规则允许源端 control binding 留空。源端三角色的正式构建仍需实际隔离 PG 和 control binding。 [角色构建规则](../../crates/learning-backup/src/offhost_b1/mod.rs)

三个 Linux/amd64 静态 ELF 已逐个核对字节和 SHA-256。Go 使用经官方 SHA-256 校验的 `go1.27.2`。通过 Rust 角色 4 项、Go native 4 项、receiver 拒绝 2 项。三个 ELF 均无动态解释器或 DT_NEEDED 项。launcher 嵌入这份 receiver 的真实摘要。

| ELF | 字节 | SHA-256 |
| --- | --- | --- |
| `knowweave-c4-b1-receiver` | 6764272 | `b28d7d94ea8c5e0e4044822610e0cd3792c7403ab8858fb3c735bf4397c7a0b3` |
| `knowweave-c4-b1-receiver-launcher` | 3046516 | `754693a6c4fea5433342bf5a00c3dcd0a34c47cf33cbe3594d0a3adc4d731d0b` |
| `knowweave-c4-b1-ssh-gate` | 3046524 | `3be69a49c1f78e74e0798291efdc3744af55ce4235452e14a79b07c5eb8f0bed` |

三份 ELF 已和四份配置文件一起封成仅供本地审查的非启用包。包内 sudoers 用实际 launcher 摘要，配置仍以 22 路径来源为候选，22229 需在批准后复核。七文件清单 SHA-256 `1ca36be64a69e4956c8359d7cd26e8572a30767d9f582e9a913be75e5057555c`。正式安装和生效验证继续开放。

Rust 构建输入 `517e9d95e42818dc9f8ad51533ebf0893eba098dd15a494a64b4e3d1636d5d18`，receiver 回读 `e90aa432826ff399112468911ced44b215536936e36a05b3768fe19951caf062`，Go 构建及三份 ELF 回读 `b61651f94571fef4d4d33f40236888293dd73de8c81e1a47a00b9e314d16e693`。

首轮静态参数作用到了宿主 proc macro，修正为 Cargo 的显式 target 后通过 Rust 构建和检查。下一轮 Go 测试因容器 `/tmp` 的 `noexec` 退出，之后将 Go 测试临时文件移到可执行的构建工作区。旧失败回执保留。 [Cargo 参数作用域](https://doc.rust-lang.org/cargo/reference/config.html#buildrustflags) · [Go 官方发布](https://go.dev/dl/)

## 接下来

先完成 S1、S2 两套隔离 PG18 配置和 control 视图的准确实施单。各自使用独立 PG 数据卷与 socket 卷，核验无 TCP 监听、host 与容器见到同一 socket，以及三个固定 SQL 角色的 `peer:root` 会话。信任文件、单条 sudo 权限、22229 最窄放行和 listener 分步审批。实装还需要实际 policy、trust、registry/session anchor、launch manifest 和精确 ELF 文件回读。管理 22 的专用账号拒绝、真实启动链、父 ELF、FIFO/FD 及退出证据均待现场验证。 [安装前核验与最小审批](h4-c-installation-preflight.md) · [双机实施单](installation-and-two-host-acceptance-plan.md)
