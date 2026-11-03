# Task9 C 源端 PG18 与固定挂载实施单

2026-11-04。S1、S2 已各有 root 私有的宿主案例目录。本次冻结隔离设计、候选配置字节、挂载清单和构建入口。状态为待现场审批与实测。

同日更新已完成 S 私有 QEMU、KVM API 12 查询及两块 32 GiB 磁盘。新 S1 副本与 S2 原盘已完成离线首启并正常停机，旧 S1 HOLD 单独留存。下一步冻结各案 PG18 的镜像、启动配置和可写对象。 [引导与权限](h4-c-s1-s2-guest-boot.md)

## S1 与 S2 的隔离边界

每个案例使用一台独立的 x86_64 KVM guest，内有独立 Linux 内核、rootful Docker daemon、数据盘和固定路径。宿主案例目录用于存放本案例私有磁盘和证据。guest 的控制根、卷和 socket 身份在 guest 内重新测量。S1 的磁盘、fence、registry 和证据单独留存，S2 使用新盘、新 daemon 和自己的三类角色 ELF。两个 guest 属于同一源端 S 的实验资源，独立备份目标仍是 D。

采用两台 guest 的原因是 Capture 的 fence bind 同时要求宿主 source 和容器 target 为 `/var/lib/knowweave-c4-source-fences`。Docker 在 daemon 所在的主机视图中解析 bind 源。只改变 Root 启动者的 mount namespace，无法让现有同一个 daemon 按调用者分别解析两个同名目录。使用 guest 后，两套固定路径、daemon 和内核各自可查，现场仍须逐项证明。[固定路径校验](../../crates/learning-backup/src/offhost_b1/source_launch.rs) · [Docker bind 文档](https://docs.docker.com/engine/storage/bind-mounts/)

S 以普通账号只读确认 x86_64、24 逻辑 CPU、约 26.91 GiB 可用内存、198.23 GiB 可用磁盘、CPU 虚拟化标志和字符设备 `/dev/kvm`。标准路径未发现 QEMU、qemu-img、virsh、virt-install 或 libvirt socket。还需冻结宿主工具包和依赖、guest 镜像与 guest Docker 版本，并获批核验 KVM 可用性。预算为每台 2 vCPU、4 GiB 内存、独立最大 32 GiB 的稀疏磁盘，顺序启动。准备阶段无 guest 网卡、共享目录或宿主端口映射，使用本地受控控制台和校验后的只读输入介质。后续跨机测试另核 S 实际出口身份和 D 侧授权。 [QEMU 启动选项](https://www.qemu.org/docs/master/system/invocation.html)

## PG18 与 Unix socket

每个 guest 单独导入此前冻结的 PG18 镜像并检查 digest、架构和本地 image ID。使用该案例的新项目、internal bridge、PG 数据卷、Unix socket 卷和 UUID 数据库。最终仅 PG 容器接入该 bridge，容器上限 1 CPU、1 GiB 和 128 PID，内部 socket 端口为 5432。PGDATA 固定 `/var/lib/postgresql/18/docker`。PostgreSQL 以 `999:999` 启动并作为最终容器 PID 1。`listen_addresses` 为空，host publish 和 host TCP listener 数为零。PG 目录与 socket 卷使用各自已核的卷名，禁止卷初始化自动复制镜像内容。 [PG 连接设置](https://www.postgresql.org/docs/18/runtime-config-connection.html) · [Docker 卷文档](https://docs.docker.com/engine/storage/volumes/)

1. **一次性引导。** 只对新数据卷初始化。首个 postmaster 启动前装入只供引导的本地 `postgres → postgres` peer 规则，复核无 TCP 监听。维护连接只从本案例引导环境发起。按现有迁移和角色合同准备该案例数据库。固定角色、CONNECT/TEMP、schema/function ACL、membership 和权限拒绝需逐项核对；引导 SQL 的准确字节随执行单审批。
2. **切换到三角色 peer。** 引导自然停机后，在本案例 PGDATA 内写入已冻结的 `pg_hba.conf`、`pg_ident.conf` 和 PG 配置。唯一允许项为该案例数据库的 OS `root` 分别映射到 `learning_admin`、`learning_snapshot_claim_broker`、`learning_runtime`。本地其余入口及 IPv4/IPv6 全拒绝。最终规则排除引导登录。PGDATA `999:999 0700`、配置文件 `999:999 0600`，socket 目录和 socket 权限均为 `0700`。 [PG HBA](https://www.postgresql.org/docs/18/auth-pg-hba-conf.html) · [PG 用户映射](https://www.postgresql.org/docs/18/auth-username-maps.html)
3. **采集构建绑定。** 用该 guest 的只读观察轮次启动同一份已初始化数据卷。三个角色分别实测 `system_user=peer:root`、`session_user=current_user`、数据库和 owner 符合预期，TCP 地址和端口为 NULL。管理员另取 DB OID、system identifier 和 postmaster start。另做未知 OS 用户、未列 SQL 角色、错误数据库及 TCP 拒绝。分别封存数据卷、容器和 postmaster 的回执。两套集群的 system identifier 必须不同，DB OID 数值可在不同集群中碰巧相同。 [PG 会话身份函数](https://www.postgresql.org/docs/18/functions-info.html) · [产品会话校验](../../crates/learning-db/src/snapshot_broker/b1_peer/session.rs)
4. **最终容器重取易变身份。** 观察容器先自然停机，继续使用同案例的数据卷与 control 原件。最终带完整 15 项挂载的 PG 容器启动后，重新取容器 ID、postmaster start、namespace、socket dev/ino 和网络 ID。只有 database/OID/system identifier、control 根及 canonical binding 均一致，才能使用已绑定的 ELF。中间失败保留本批磁盘、卷和证据，按不可变 ID 停止本批对象。

## 两个 guest 各自的固定视图

| 对象 | 源与目标 | 现场证据 |
| --- | --- | --- |
| socket 卷 | guest 的专用命名卷以 volume 挂至 PG `/var/run/postgresql`。同一个卷的实际 Mountpoint 在 guest 宿主只映射到 `/run/knowweave-c4-source-pg`。 | 验 PG native `/run/postgresql`、Capture `/var/run/postgresql` 与 guest 宿主 alias 的目录和 `.s.PGSQL.5432` 是同一原件，记录类型、owner、mode 和 dev/ino。宿主 alias 在容器外。 |
| control 卷 | 同一个本案例 source 卷以 volume 挂至容器 `/var/lib/knowweave-source`，在 guest 宿主同名固定根映射同一卷。`control` 为其中的 `root:root 0700` 子目录。 | guest 与容器的固定 `/var/lib/knowweave-source/control` 解析到同一原件。`source-binding.json` 为 `root:root 0600`、普通单链接。九字段 canonical 字节哈希进入构建。 |
| Capture 只读元数据与 fence | Release 原文件 `capture-source-policy.json`、`capture-launch-manifest.json` 各自映射到 loader 固定文件名。Fence 由本 guest 的固定路径映射到容器同名路径。 | 两份文件分别比较双视图 inode、完整 stamp 和字节。Fence 在同一原锁上实测只读视图及 flock。检查本 guest 所有 Docker 容器和非 Docker 写者，排除额外可写别名。 |
| 完整容器投影 | 逐案从当前源码冻结 15 项 `type/target/RW`，包括 6 个独立命名卷及 9 个 bind。只读 bind 使用 `rprivate`，源缺失时停止。 | 固定核对完整 `Mounts`、真实 mountinfo、内部网络、PID 1、Docker API/CLI 和权限边界。镜像内 `b1` 父目录需要以准确的容器内初始化方式形成 `root:root 0700`，不能增加第 16 项挂载。 |

PG 配置文件落在既有 PGDATA volume 内。guest 宿主的 socket 和 source alias 不计入容器挂载。S1/S2 以 guest boot ID、daemon ID、卷、集群 system identifier 及各自的固定视图共同区分身份，跨不同内核的 dev/ino 数值不可单独用于判断共用。[十五项合同](capture-mount-amendment-contract.md) · [control 绑定解析](../../crates/learning-backup/src/source/binding.rs)

## 构建顺序与输入

每案构建 `root-launcher`、`publication-source`、`capture` 三个角色，共 6 份。使用已验证的固定 Linux/amd64 builder、Rust 1.97.0、锁定 Cargo 和 `offhost-b1`，普通用户、无网络、只读输入。每个角色用独立构建目录和 manifest。构建前固定 manager commit、源码归档、公钥摘要、该案实际应用 commit/归档摘要、guest `/usr/bin/docker` 摘要、canonical `source-binding.json` 和角色标记。guest 的 Docker CLI 摘要单独量取。

`cargo build` 使用 `--locked --offline --release --target x86_64-unknown-linux-gnu`，静态参数限定目标编译。六份 ELF 分别核 amd64、无解释器、无动态依赖、角色输入摘要和保留字节。按既有顺序发行后续 policy、launch manifest 和 Catalog，避免用最终 ELF 哈希反向定义编译输入。现场绑定、实际构建和实参限定检查均待取得真实输入。[构建角色入口](../../crates/learning-backup/src/offhost_b1/mod.rs) · [发行顺序](capture-mount-amendment-contract.md)

## 分步审批清单

| 审批 | 最小写入与权限 | 执行前须冻结 |
| --- | --- | --- |
| S-VM 虚拟化 | S 侧批准的 QEMU/KVM 工具、限定运行主体对 `/dev/kvm` 的使用、两个已批准案例根下各一个新 guest 盘及专用控制台。guest Docker 仅使用 guest 内 root 管理面。 | 工具包及依赖的准确清单、guest 镜像 SHA-256、CPU/内存/盘上限、KVM 运行主体、精确命令和停止方式。宿主若需新增组或设备 ACL，作为可见单列变更。 |
| S-PG 引导及角色 | 两个 guest 内分别创建新 PG18 项目、内部 bridge、数据卷与 socket 卷，写入本案 HBA/ident/PG 配置。SQL 权限只针对本案数据库与三个固定角色，引导登录随阶段关闭。 | 精确新对象名、镜像 ID、配置与引导 SQL 哈希、ACL 逐条差额、owner/mode、实际 deny 判据、停机和保全。 |
| S-view 固定路径与发行 | 每个 guest 内建立固定 socket/source/fence 视图、六个已列 volume 和九个 bind 的完整投影，写入各案独立的 control/registry/fence/元数据。 | 逐路径及原件身份、权限、运行主体、十五行清单、镜像内父目录初始化的准确命令及私有路径。正式 C 安装、真实 Root 执行权限和信任安装分别批准。 |
| 双机网络与 D 启用 | S guest 至 D 的实际出口、D 的最窄放行、单条 digest sudo 授权与专用 listener。 | 先核实际出口与 host key，提交独立审批。 |

## 本轮证据

开发机只读回执 `0508fd13fdc85b0e2cb3e5cd2a426bfe5052be265965c3edbc1758d016935bce`。私有 PG 计划 `2db10c8d7b89d1b9a54c7eccd56944f095666933e37e72ba93d3808f848c8b12`。两份 15 行挂载清单 `8f0208ac9f5db978dcafdf130e39cf73b127b5b816817e73b088e99fe7cb3253`。8 个配置或观察模板、6 个内存合成角色清单和 8 个拒绝变体通过离线检查。实际 guest、PG、socket、control 和正式 ELF 继续待验。
