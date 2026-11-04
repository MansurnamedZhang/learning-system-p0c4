# Task9 C 源端 PG18 双案首启审批单

2026-11-04。两块已扩容的停机副本、正式 NoCloud 启用介质、宿主启动器及每案命令均已绑定。S 主机只读预检已通过。独立审批后的现场结果记录在下节。 [停机扩容回读](h4-c-stopped-disk-growth.md) · [PG18 离线引导](h4-c-pg18-offline-freeze.md)

| 案例 | 首启前 QCOW2 SHA-256 | 只读启用 ISO SHA-256 |
| --- | --- | --- |
| S1 | `0a6650a90a8aab425ca77482c6c25d779570930156b2cd1525869a3dc76a40b3` | `a6ca202848e909db4a0ae69a87cb1582790906e7786ba366c497960355c4b156` |
| S2 | `b2f8abf1c62d6d80e5bc45c1dae0dcd0e903492bfd21e1de73bc50feda2828cc` | `fe36b0fc9abb1aab7bf8b89c957a50fddbf20769906a5c87b052e1f97a91955a` |

## 2026-11-04 · S1 离线 PG18 首启进入 HOLD，停机已复核

S1 首启未取得完整回执，独立确认已停机。S2 未发送介质，也未启动。 两块原盘和旧 HOLD 的哈希重新核对一致。

宿主内核日志记录该 S1 临时 unit 命中 pids 控制器。运行单设置 `TasksMax=64`，串口记录 `qemu_thread_create` 返回资源暂时不可用，unit 记录进程以 SIGABRT 退出。没有记录到该 unit 的 OOM 或超时。独立回读确认 S1 盘结构无损坏，字节哈希已变化，保留该盘与原回执作为 HOLD。现有证据无法确认 PostgreSQL 已启动或任何首启验收项通过。 [systemd 任务上限](https://www.freedesktop.org/software/systemd/man/latest/systemd.resource-control.html#TasksMax=) · [Linux pids 控制器](https://docs.kernel.org/admin-guide/cgroup-v2.html#pid)

拟为新批次准备一块干净的 S1 副本，仅提高新临时 QEMU unit 的任务上限，并采集运行期间的 `pids.current`、`pids.peak` 和 `pids.events`。CPU、内存、设备和网络边界沿用原单。S2 保留已经核验的原始首启前字节，仍等新 S1 通过四项门后才投送。此处是待封存的修正范围。

宿主失败代码 `SYSTEMD_RUN_NONZERO, SERVICE_RESULT_NON_SUCCESS, QEMU_EXIT_NONZERO, NO_VALID_READY_AND_POWEROFF_SEQUENCE`，失败案停机后 QCOW2 SHA-256 `e960766f33eaf60dea13a6d9f7243f61dbaac4c60bc2a28987350216b7448e25`，资源诊断回执 `c89b61881592d0dad1c20e3cb2e22efb69eb92a0594b8d1d3f0b940c04250373`。旧批次保持 HOLD。Task9 双机验收尚未进入。

批准清单 `d3abc266ce0b92584a4d07dc7718f66b0925d7d1d75b7a550e3f589e8e3d937d`，现场执行记录 `260921f5ae0373c1b9865e3abeefdd955d139029119a08b704a0a74467aea34b`，独立回读回执 `4ab343df937eb9162cbb544d9f0f99628ccaaafcbebc296abfdce178c9299103`。

## 审批范围

1. S 只使用既有 root 权限，在独占批次目录保留介质、串口及回执。每案临时 QEMU unit 沿用 DynamicUser、仅 KVM 设备、私有网络命名空间、无虚拟网卡与四个已绑定描述符。每案运行上限 900 秒。磁盘写入仅限表列两块新副本。旧盘、旧 HOLD 和宿主设备及用户权限在启动前后复核。
2. Guest 启动后先确认仅有 loopback，核 NoCloud instance-id、只读 ISO 与每个载荷哈希，再测根盘可用空间至少 2 GiB。仅本案私有目录可安装工具和 daemon 状态，启动 containerd、dockerd 与 PostgreSQL。固定镜像只离线导入，校验镜像 ID 与 RepoDigest，容器使用 `--pull=never`。 [Docker containerd 存储](https://docs.docker.com/engine/storage/containerd/)
3. 本案 PG18 使用三个私有卷及一个内部网桥。初始化和运行容器的 PG 用户为 `999:999`。初始阶段创建本案角色和数据库，运行静态迁移 ELF。切换最终 HBA 后，在只读 Unix socket alias 上核验三个 peer 正例和四个拒绝例，并获取数据库、socket 和 control 身份。该阶段仅创建 control 目录。 [PostgreSQL peer](https://www.postgresql.org/docs/18/auth-peer.html) · [PostgreSQL 启动协议](https://www.postgresql.org/docs/18/protocol-flow.html)
4. 先完成 S1。以精确容器 ID 停 PG，再停 guest 内两个 daemon。宿主要求串口证明、内核关机、unit 和进程组清空、QCOW2 停机复核全部通过，才发送 S2 介质。任何失败留存 HOLD，停止后续案，重新执行需另审。

迁移 ELF SHA-256 `114dbf837df88e93ad229b5db76b0f86fe849b8bf280cec26d8eff4fd8e4448d`。两份 ISO 的 cloud-init schema、文件目录和载荷逐字节读回已核验。启动输入配对含四个拒绝用例，串口证明含十个拒绝用例，三个模拟协议用例验证无密码连接流程、拒绝和异常认证方式。模拟协议和只读预检没有运行 PostgreSQL。审批清单 `d3abc266ce0b92584a4d07dc7718f66b0925d7d1d75b7a550e3f589e8e3d937d`，介质回执 `4032ec18eec0d51799f891945510566a5b4f40633f08f29671ac4862ae8caaee`，本地用例回执 `722b71b07030bde9e97113c39350d08099e81e89223ec1a31e9391feef431ade`，静态启动器回执 `6597fb015520831094ddf7f0ad4d34adf6c389d6bbae4b19d07c2ca0808cef83`，S 只读预检回执 `27aee961fe634e3fe4d63ea0087a2a5a1df53f36c8e43366c9a21d21bdd7294c`。
