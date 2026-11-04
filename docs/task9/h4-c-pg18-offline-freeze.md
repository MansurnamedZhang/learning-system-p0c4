# Task9 C 源端 PG18 离线引导冻结单

2026-11-04。S1 独立副本与 S2 原盘均以已核验的停机字节为基线。两案的镜像、容器工具、SQL 配置、Unix socket 和 control 挂载方案已冻结。启动前还须解决根文件系统容量，封存迁移 ELF 和最终 NoCloud 介质，再批准现场动作。 [首启停机回执](h4-c-s1-s2-guest-boot.md) · [源端固定视图](h4-c-source-pg-and-fixed-view.md)

## 停机盘容量和执行顺序

| 案例 | QCOW2 虚拟容量 | ext4 文件系统 | 可用字节，扣除保留块 |
| --- | --- | --- | --- |
| S1 | 32 GiB | 2,683,285,504 字节 | 821,645,312，约 784 MiB |
| S2 | 32 GiB | 2,683,285,504 字节 | 821,645,312，约 784 MiB |

只读检查从停机盘读取 GPT 和 ext4 超级块，输出留在内存，读取前后均核对原盘哈希。工具 ELF、镜像压缩层及解压 tar 合计约 778 MiB，实际 unpack 还要容纳文件系统分配、PGDATA、WAL 和证据。本轮把 guest 根文件系统的可用空间下限固定为 2 GiB。虚拟盘已经是 32 GiB，需单独审批在各 guest 内扩展现有 GPT 分区 1 的尾端和 ext4，确认起始扇区及其他分区不变，停机后封存新哈希。PG 引导随后只绑定新的停机回执。

## 离线输入与导入门

- PostgreSQL 为 `docker.io/library/postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d` 的 linux/amd64 单架构清单。13 个压缩层及 13 个解压摘要已逐项核验。OCI 归档 157,286,400 字节，SHA-256 `82cb6e05ff2195ec47ce5116985c41f059171d9ed1225d61323b9350de3083a1`，预期 Docker image ID 为 `sha256:8d76d8de17e883d3995f76252e260bc6b7761e9c23edbf3356b0676d37c49ddb`。现场在本案 containerd 的 `moby` 命名空间离线导入，再核 Docker API 的 ID 和 RepoDigest。任一缺失即停。创建 PG 使用 `--pull=never`。 [Docker containerd 存储](https://docs.docker.com/engine/storage/containerd/) · [Docker pull 策略](https://docs.docker.com/reference/cli/docker/container/run/#pull)
- Ubuntu 签名索引匹配的三份包为 Docker 29.1.3、containerd 2.2.1、runc 1.3.4。只提取核验过的运行 ELF 到 guest 的本案私有树，现有依赖按签名基底和现场 loader 回读。containerd 使用独立数据根、v3 配置和 Unix socket。原生解析显示该发行包会附加 `/etc/containerd/conf.d/*.toml`；现场必须只读确认该目录不存在或为空，存在的祖先路径由 root 所有、禁止组和其他用户写入、无符号链接。否则停止。Docker 使用独立数据根和 guest root 专用 Unix socket。默认 bridge 关闭，按案创建唯一 internal bridge。
- 迁移沿用 `c4_task3_migrate` 的 `snapshot_c0_d1_v1`，固定 0001–0015、两份扩展 SQL 和 Cargo.lock。现阶段封存的是源码、构建参数和迁移输入，精确 ELF 字节与最终 NoCloud 执行器和介质哈希仍待补齐。后续以新 NoCloud instance-id 从本案只读介质触发一次性 guest root 执行，无需交互 root 登录。 [cloud-init 实例判定](https://docs.cloud-init.io/en/26.1/explanation/first_boot.html)

## PostgreSQL 与固定视图

1. **初始化。** S1、S2 使用不同的 UUID 数据库、项目、数据卷、socket 卷和 source 卷。初始化容器以 `999:999`、无网、无额外 capability 运行 `initdb`，UTF-8、`C.UTF-8`、数据校验和。引导 HBA 只允许容器 OS `postgres` 对维护库和本案库做 peer，另开放 guest OS `root` 映射本案 `learning_admin` 供迁移。引导完成先停 PG，再装入最终 HBA 和 ident。
2. **三角色 peer。** 最终仅 OS `root` 可在本案库映射 `learning_admin`、`learning_runtime` 或 `learning_snapshot_claim_broker`。管理员为库主，拥有最小观察函数权限。runtime 获 `CONNECT/TEMP` 和迁移定义的对象授权，broker 为 `NOINHERIT`、连接上限 4，获 `CONNECT` 和扩展定义的授权。 `learning_auth_lock` 保持 `NOLOGIN`，只允许管理员 SET。账号密码均为 NULL。本地其他匹配及 IPv4/IPv6 全拒绝，`listen_addresses=''`，Unix socket 端口号 5432。核对三个正例、错误 OS 用户、角色、数据库和 TCP 四个拒绝例。 [PostgreSQL peer](https://www.postgresql.org/docs/18/auth-peer.html) · [连接设置](https://www.postgresql.org/docs/18/runtime-config-connection.html)
3. **Unix socket 和 control。** 两个 alias 只在各自 guest 建立。socket 卷挂入容器 `/var/run/postgresql`，guest 内的 `/run/knowweave-c4-source-pg` 只读 bind 到同一卷，核目录及活 socket 的 dev/inode 与实际 peer 连接。source 卷原件创建 `control`，guest 内的 `/var/lib/knowweave-source` 先建立只读 bind，核 `control` 同 inode。此段只准备目录，`source-binding.json` 需取得真实库 OID、system identifier 和 control 身份后，另审从原卷独占写入 canonical 九字段记录。
4. **停机。** 每案以准确容器 ID 停止 PG，确认容器与 daemon 都退出，校验 PGDATA、卷身份、日志与 guest 停机哈希。S1 全部门通过并停机后再进入 S2。现阶段 PG 容器只带数据和 socket 两个可写卷，最终 Capture 的 6 个卷和 9 个 bind 仍按既有 15 项合同另行准入。 [Capture 挂载合同](capture-mount-amendment-contract.md)

## 可写对象和最小权限

| 主体 | 需要审批的最小写入 |
| --- | --- |
| S 现有 root | 两案各自的新只读介质和证据目录、已核验的 case QCOW2、单次临时 QEMU unit。动态非 root QEMU 仍只使用本案 FD 和既有 kvm 设备权限。 |
| 各 guest root | 本案容器工具和 daemon 私有数据目录、`/run` socket 和 pid 文件、三个新卷、两个固定 bind、单一 internal bridge 及其 guest 内规则。挂载需要 guest 内 CAP_SYS_ADMIN，bridge 需要 guest 内 CAP_NET_ADMIN。 |
| PG `999:999` | `/var/lib/postgresql/18/docker` 为 `0700`，配置文件 `0600`，本案 socket 卷目录及 socket `0700`。容器根文件系统只读，初始化器独用有上限的临时 `/tmp`。 |
| 未来 binding 写入者 | 仅 guest root。source 卷的 `control` 为 `root:root 0700`，真实绑定文件 `0600`、单链接、独占创建并回读。 |

修正版两案独立回读通过，均达到 2 GiB 停机容量门。迁移 ELF 与 NoCloud 准备介质已核验。两块新停机盘已绑定正式启用介质和首启运行单，S 只读预检通过，等待 PG 首启审批。 [扩容及介质运行单](h4-c-stopped-disk-growth.md) · [首启审批单](h4-c-pg18-first-boot.md)

审批材料须包含实际迁移 ELF 和两份最终介质的哈希、精确启动和停机指令。HOLD、镜像或别名身份漂移、peer 拒绝门失败时保留证据并停止后续案。

配置与审批清单 SHA-256 `c4993299ce84412b40c6f3ea656efc00097498c91af3d147d0d71beb745a92cf`。28 个配置或命令文件完成回读，10 个拒绝夹具和两次 containerd 原生配置解析通过。静态回执 `51ce934a844530b917460219404d425f455d46255c6b2b5af118f77b2bb7fd89`，OCI 回执 `15719e519d54a8c55348e7ae13f4a9ae2de530b126edf16e363464a356228c92`，容器工具回执 `d3b938243f60d29fe46baf3a80dbccf0d060618efeaf481ebd44d60122d148ec`，容量只读回执 `088da5eb133d098a8e7e0053a85f8d88b5f8eb27707ba0ce7b3beac69fbf7c46`，空间预算 `6b7b1cc7697060fb17ecaa145ca2d116f91c38103457868ad84193e7273812db`。
