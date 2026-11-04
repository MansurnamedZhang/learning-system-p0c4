# Task9 C 源端 PG18 双案首启审批单

2026-11-04。两块已扩容的停机副本、正式 NoCloud 启用介质、宿主启动器及每案命令均已绑定。S 主机只读预检已通过。当前等待独立审批，实际 guest、Docker 和 PG18 运行尚未发生。 [停机扩容回读](h4-c-stopped-disk-growth.md) · [PG18 离线引导](h4-c-pg18-offline-freeze.md)

| 案例 | 首启前 QCOW2 SHA-256 | 只读启用 ISO SHA-256 |
| --- | --- | --- |
| S1 | `0a6650a90a8aab425ca77482c6c25d779570930156b2cd1525869a3dc76a40b3` | `a6ca202848e909db4a0ae69a87cb1582790906e7786ba366c497960355c4b156` |
| S2 | `b2f8abf1c62d6d80e5bc45c1dae0dcd0e903492bfd21e1de73bc50feda2828cc` | `fe36b0fc9abb1aab7bf8b89c957a50fddbf20769906a5c87b052e1f97a91955a` |

## 审批范围

1. S 只使用既有 root 权限，在独占批次目录保留介质、串口及回执。每案临时 QEMU unit 沿用 DynamicUser、仅 KVM 设备、私有网络命名空间、无虚拟网卡与四个已绑定描述符。每案运行上限 900 秒。磁盘写入仅限表列两块新副本。旧盘、旧 HOLD 和宿主设备及用户权限在启动前后复核。
2. Guest 启动后先确认仅有 loopback，核 NoCloud instance-id、只读 ISO 与每个载荷哈希，再测根盘可用空间至少 2 GiB。仅本案私有目录可安装工具和 daemon 状态，启动 containerd、dockerd 与 PostgreSQL。固定镜像只离线导入，校验镜像 ID 与 RepoDigest，容器使用 `--pull=never`。 [Docker containerd 存储](https://docs.docker.com/engine/storage/containerd/)
3. 本案 PG18 使用三个私有卷及一个内部网桥。初始化和运行容器的 PG 用户为 `999:999`。初始阶段创建本案角色和数据库，运行静态迁移 ELF。切换最终 HBA 后，在只读 Unix socket alias 上核验三个 peer 正例和四个拒绝例，并获取数据库、socket 和 control 身份。该阶段仅创建 control 目录。 [PostgreSQL peer](https://www.postgresql.org/docs/18/auth-peer.html) · [PostgreSQL 启动协议](https://www.postgresql.org/docs/18/protocol-flow.html)
4. 先完成 S1。以精确容器 ID 停 PG，再停 guest 内两个 daemon。宿主要求串口证明、内核关机、unit 和进程组清空、QCOW2 停机复核全部通过，才发送 S2 介质。任何失败留存 HOLD，停止后续案，重新执行需另审。

迁移 ELF SHA-256 `114dbf837df88e93ad229b5db76b0f86fe849b8bf280cec26d8eff4fd8e4448d`。两份 ISO 的 cloud-init schema、文件目录和载荷逐字节读回已核验。启动输入配对含四个拒绝用例，串口证明含十个拒绝用例，三个模拟协议用例验证无密码连接流程、拒绝和异常认证方式。模拟协议和只读预检没有运行 PostgreSQL。审批清单 `d3abc266ce0b92584a4d07dc7718f66b0925d7d1d75b7a550e3f589e8e3d937d`，介质回执 `4032ec18eec0d51799f891945510566a5b4f40633f08f29671ac4862ae8caaee`，本地用例回执 `722b71b07030bde9e97113c39350d08099e81e89223ec1a31e9391feef431ade`，静态启动器回执 `6597fb015520831094ddf7f0ad4d34adf6c389d6bbae4b19d07c2ca0808cef83`，S 只读预检回执 `27aee961fe634e3fe4d63ea0087a2a5a1df53f36c8e43366c9a21d21bdd7294c`。
