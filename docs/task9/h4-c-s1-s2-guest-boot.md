# Task9 C 阶段 S1/S2 首次离线引导

2026-11-04。修订批次在 S 完成 S1 与 S2 的首次离线启动和停机验证。S1 使用新建独立副本，S2 使用原盘。旧 S1 HOLD 的原盘和证据已重新核对。此轮是 guest 引导验证，PG18、固定挂载和正式 ELF 仍待后续现场阶段。 [已核验的 S 原盘与工具](h4-c-kvm-host-preparation.md) · [源端 PG18 实施单](h4-c-source-pg-and-fixed-view.md)

## 修订批次现场结果

S1 从封存旧盘进行只读完整复制，复制后核对原始哈希、独立 inode 和无 backing。新启动入口把同一根盘的只读及读写描述符加入同一 fdset。S1 获得独立的 guest boot ID、串口通过标记与内核关机记录，QEMU 退出 0。停机和磁盘结构检查通过后才下发 S2，后者通过同样的门。两案均核对实际 QEMU 参数、动态非 root 身份、单一额外 kvm 组、能力集为空、NoNewPrivileges 和私有网络。guest 在仅有 loopback 的条件下从本案 CIDATA 完成 NoCloud 引导，随后正常关机。

| 案例 | S 端结果 SHA-256 | 停机后 QCOW2 SHA-256 |
| --- | --- | --- |
| S1 | `bb52d748e63a189b6bfe425b0b7f2c7117be24a28fe8515d3a69f62ea3e0efb6` | `d163a392742b64b19bbf07b56341e72758bf6184289f96483797b4250e14b995` |
| S2 | `52bd0b986c4464d6a5d8f7a15aab0f481183e572ae630795ed6df8a807f6ae16` | `815409341cecbd4eec33564750a2037c50fc9f0a42c6c76034f2787b820ca92e` |

两块盘均为 32 GiB，单链接且无 backing，结构检查无错误、损坏或泄漏。新 S1 与 S2 都已写入首次开机数据，后续现场步骤须绑定上表所列停机原件。旧 S1 磁盘和 HOLD 回执通过独立回读，两个新 unit 无活动进程。控制器结果 SHA-256 `352af556fd281d25d4b13b6c00bc2e5a274e9a27348b235b568e8ea688af9554`，独立回读 SHA-256 `9eede78feed5dc56aeabca6fdb5cd921db31a8b51c0afe52133ee7d2e1e8f1d8`，批准清单 `5ed4787650b0caddbff5c83241dc58b824426f022ff799a9c71d437b748cd21c`。

各 guest 的 PG18 一次性引导、Unix socket 和 control 固定挂载方案已冻结。两案根文件系统容量不足，扩容和现场执行待分步审批。 [PG18 离线冻结单](h4-c-pg18-offline-freeze.md) [PG18 与固定挂载实施单](h4-c-source-pg-and-fixed-view.md)

## 旧 S1 启动失败及 HOLD

临时 unit 已停止，cgroup 已清空。QEMU 退出 1。串口有宿主 FD 检查通过行，随后报告 `Failed to find file descriptor with matching flags=0x0`。只有 `O_RDWR` 描述符的根盘 fdset 缺少 QEMU 探测所需的 `O_RDONLY` 描述符。S1 原盘仍为 32 GiB QCOW2，哈希、inode 和 `root:root 0600` 均与启动前一致，结构检查通过。S2 目录内只有未改动的原盘。

控制器在 root 已封存结果后，因回传事件与结果共用了 `status` 参数而退出。以上结论来自第二个只读会话核对 root 回执、实际串口、两个 unit 状态及原盘；未重放启动。现场回执 SHA-256 `c486f09007b5b43ba5f5079f66a7d69d1cfb73369a9f71d55f95802c7a481c55`，独立回读 `7676581da7953706d96468b0ab6373a6acb575e5fcb6c4a0455591c07d011028`，控制器结果 `f6e3632ce4728a56ff6e787ab1a64e67e22778d31d072bc260353652f0c303f4`。

## 修订批次原方案与获批差额

在原 QEMU 10.2.1 二进制的无主板、无 CPU 夹具上，使用签名基底的私有 QCOW2 副本和原只读 CIDATA。只传根盘读写 FD 可复现现场错误。增加同一 inode 的只读 FD 后，32 GiB QCOW2 读写打开和 CIDATA 只读打开均通过，夹具盘结构检查通过。两条修订 unit 完成静态解析，FD 入口通过同 inode 正例、异 inode 和错误访问模式两个拒绝例。现场结果见本页首节。 [QEMU 的 FD set](https://www.qemu.org/docs/master/system/invocation.html)

修订把根盘 `O_RDWR` 和 `O_RDONLY` 分别放在 FD 3、4，归入同一 fdset 1。介质移至 FD 5，仍属 fdset 2。PID 1 在每次任务中只额外打开同一个原件的只读 FD，权限和离线隔离设置保持原范围。入口按实际 dev/inode/长度确认两个根盘 FD 指向同一个文件。

新批次最小范围为 S 上独占创建一个新的 S1 私有案例和一块 32 GiB 无 backing QCOW2，使用本次确认字节未变的 S1 磁盘做只读完整复制，核对哈希并绑定新 inode。旧 S1 盘、unit 和回执保持 HOLD。S2 用尚未启动的原盘和新的引导目录。两个新临时 unit 均依照原 2 vCPU、4 GiB guest 内存和 300 秒上限，只在 S1 获得串口与停机证明后运行 S2。

修订清单 SHA-256 `5ed4787650b0caddbff5c83241dc58b824426f022ff799a9c71d437b748cd21c`，真实 QEMU 块路径回执 `973d3b6a8e0fd582cf277014945ebad2edf18b38dfcde3b91cc90808e137cf30`，修订 unit／FD 静态检查 `c0d75cd555fdba3e024149cb4221944a5c83b361a8926ad5df6d756c59682863`。

## 原轮介质和静态核验

| 案例 | CIDATA 容量 | SHA-256 |
| --- | --- | --- |
| S1 | 376,832 字节 | `25dd7a2b837172d64a23c5babf08cbad399320f524e1417c6f8b37508075b683` |
| S2 | 376,832 字节 | `e064552c21e835d75f0f8c75281aa70befb7dd06b8b001dca3e65ac2761bada7` |

每份 ISO 含本案 `meta-data`、`user-data` 与空接口 `network-config`，实例 ID 和主机名各自独立。介质用包摘要已核对的 genisoimage 生成，Rock Ridge 和 Joliet 两种视图均逐字节读回，cloud-init 26.1 配置通过 schema 验证，介质无启动目录。原有签名基底另以只读转换回读 GPT BIOS 分区、GRUB 串口参数和实际 cloud-init 26.1 源文件。guest 启动自原 QCOW2，由 NoCloud 从 `CIDATA` 读取配置。 [NoCloud 介质格式](https://docs.cloud-init.io/en/26.1/reference/datasources/nocloud.html)

S 只读回读 systemd `259.5-0ubuntu3.4`、QEMU `10.2.1`、两块原盘 inode 与 `0600` 权限、可用内存和空间，并确认两个临时 unit 和用户／组名未占用。QEMU 的机器、加速器与设备选项已由该版本的帮助入口核对。两个等价 unit 在 Linux 经 `systemd-analyze verify` 解析，FD 交接完成一个正例和四个拒绝夹具。在准备轮中，KVM 未重新打开，介质当时仍在准备目录。随后现场执行结果见上。

## 原轮启动命令和权限

每案 root 调度器只向系统 systemd 提交对应的 transient unit `kw-task9-h4-c-s1-boot.service` 或 `kw-task9-h4-c-s2-boot.service`，逐案使用动态非 root UID/GID，额外组限定为现有 `kvm`。设备策略 `closed` 加 `/dev/kvm rw`。capability 集为空，`NoNewPrivileges=yes`。服务处于私有网络、只读系统和隔离临时目录，使用 16 MiB 私有 `/run` 隔离宿主管理 socket。QEMU 树和 FD 校验入口只读映射到该私有 `/run`。 [systemd 执行隔离](https://manpages.ubuntu.com/manpages/resolute/man5/systemd.exec.5.html)

PID 1 用 `OpenFile` 打开本案原盘、只读 seed 和私有串口文件。入口校验三个 FD 的权限、访问方向、原盘 inode、字节摘要与独占锁，再把原盘、介质映射到 FD 3、4，把串口输出定向到私有文件。原盘仍是 `root:root 0600`，无账号组登记、ACL、sudoers、udev 或常驻 unit 变更。KVM 在实际 QEMU 启动期间会被打开并创建 VM，先前的 API 12 查询不代替这次验证。 [systemd OpenFile](https://manpages.ubuntu.com/manpages/resolute/man5/systemd.service.5.html) · [QEMU FD set](https://www.qemu.org/docs/master/system/invocation.html) · [Linux KVM API](https://docs.kernel.org/virt/kvm/api.html)

下列为历史冻结参数，仅用于审计旧 HOLD，`CASE` 在两条私有命令中已分别固定为 `s1`、`s2`。这是 FD 入口完成身份核验后直接执行的 argv，手工从 shell 调用会缺少经过验证的 FD。

```sh
/run/kw-task9-qemu/bin/qemu-system-x86_64 \
  -name kw-task9-h4-c-CASE-boot -no-user-config -nodefaults \
  -machine pc-q35-10.2,usb=off,vmport=off,dump-guest-core=off -accel kvm \
  -cpu host -smp 2,sockets=1,cores=2,threads=1,maxcpus=2 -m 4096 \
  -L /run/kw-task9-qemu/share/qemu -bios /run/kw-task9-qemu/share/seabios/bios-256k.bin \
  -display none -monitor none -nic none -parallel none \
  -chardev stdio,id=kwserial,signal=off -serial chardev:kwserial \
  -no-reboot -boot strict=on -add-fd fd=3,set=1 -add-fd fd=4,set=2 \
  -drive if=none,id=kwroot,file=/dev/fdset/1,format=qcow2,cache=writeback,aio=threads,discard=ignore \
  -device virtio-blk-pci,drive=kwroot,serial=kwroot-CASE,bootindex=1 \
  -drive if=none,id=kwseed,file=/dev/fdset/2,format=raw,readonly=on \
  -device virtio-blk-pci,drive=kwseed,serial=kwseed-CASE \
  -sandbox on,obsolete=deny,elevateprivileges=deny,spawn=deny,resourcecontrol=deny
```

## 原轮获批范围和可见影响

| 对象 | 获批后执行的最小动作 |
| --- | --- |
| 宿主写入 | 仅 S。本批锁与每案私有 `boot/` 目录使用独占创建。逐案写入只读 CIDATA 和入口程序、`0600` 串口与执行回执。每次重查已冻结原盘、主机工具、身份、资源与两案新鲜性。 |
| 短时运行 | 先 S1，确认通过并停机后才启动 S2。每案 2 vCPU、4 GiB guest RAM，宿主 cgroup 上限 2 核、5 GiB、无 swap、64 个任务。启动超时 20 秒、运行上限 300 秒，停止宽限 10 秒。无重启策略。 |
| guest 写入 | Linux 常规开机与 cloud-init 写入各自原盘，离线盘 SHA-256 会变化，稀疏盘宿主占用可增长。ISO 只读，分区和文件系统保持首启前大小。guest 内关闭 SSH 服务、socket 和自动 apt 定时器，保持无账号凭证。每案检查实例 ID、独立 boot ID、仅 loopback、两块设备访问方向与串口证明，然后正常关机。 |
| 外部边界 | QEMU 无网卡、共享目录、监视器、TCP listener 或宿主端口映射。后续 guest Docker、PG18、固定挂载、D 端操作、22229、信任与签名链仍分别审批。 |
| 失败处理 | 串口超过 4 MiB、超时、身份漂移或缺失证明时保持 HOLD。只按本轮 unit 名、InvocationID 与 MainPID 核实停机，待 cgroup 清空。保留两案盘和已产生的证据，S1 失败时 S2 保持未启动，禁止自动重试。 |

启动通过要求唯一的宿主 FD 通过行和本案 guest 串口成功行、真实关机序列、QEMU 退出 0、unit 无活动进程，以及停机后原盘结构检查和新摘要。两案 boot ID 必须不同。静态核验与 FD 夹具不计入现场启动证据。首次运行后不得把原盘当作未启动基底复用。

## 回执摘要

| 证据 | SHA-256 |
| --- | --- |
| S 只读检查 | `fa980382532e367fa1e98673d1a95735ab672a1fa09a9cf976af7f472aea5aee` |
| NoCloud 介质 | `e36b3da2e066d2cd4764aeff5fa0fdfabeb28f93bc9e2fbc5da80fe9e5e1d5e1` |
| 签名基底只读结构 | `24cbf06f0af11dfef1fc13cf6f560645eaf76be840726dc6187bf7be5cb5529c` |
| 两案实际 argv 和权限清单 | `572549be0a62357d8e163da92431737cb432b7bd2e23adcaa0310b776d6faaf1` |
| 命令静态核验和 FD 夹具 | `337fe88cddb8c5e8fc999d9c3ec45c1ca32ebe16448144fd092345b852f6ed23` |
