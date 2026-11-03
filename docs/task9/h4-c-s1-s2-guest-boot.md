# Task9 C 阶段 S1/S2 首次离线引导

2026-11-04。两份 NoCloud 介质、临时运行主体和逐案固定启动命令已冻结，待批准在 S 上首次启动。此轮是 guest 引导验证，PG18、固定挂载和正式 ELF 仍待后续现场阶段。 [已核验的 S 原盘与工具](h4-c-kvm-host-preparation.md) · [源端 PG18 实施单](h4-c-source-pg-and-fixed-view.md)

## 介质和静态核验

| 案例 | CIDATA 容量 | SHA-256 |
| --- | --- | --- |
| S1 | 376,832 字节 | `25dd7a2b837172d64a23c5babf08cbad399320f524e1417c6f8b37508075b683` |
| S2 | 376,832 字节 | `e064552c21e835d75f0f8c75281aa70befb7dd06b8b001dca3e65ac2761bada7` |

每份 ISO 含本案 `meta-data`、`user-data` 与空接口 `network-config`，实例 ID 和主机名各自独立。介质用包摘要已核对的 genisoimage 生成，Rock Ridge 和 Joliet 两种视图均逐字节读回，cloud-init 26.1 配置通过 schema 验证，介质无启动目录。原有签名基底另以只读转换回读 GPT BIOS 分区、GRUB 串口参数和实际 cloud-init 26.1 源文件。guest 启动自原 QCOW2，由 NoCloud 从 `CIDATA` 读取配置。 [NoCloud 介质格式](https://docs.cloud-init.io/en/26.1/reference/datasources/nocloud.html)

S 只读回读 systemd `259.5-0ubuntu3.4`、QEMU `10.2.1`、两块原盘 inode 与 `0600` 权限、可用内存和空间，并确认两个临时 unit 和用户／组名未占用。QEMU 的机器、加速器与设备选项已由该版本的帮助入口核对。两个等价 unit 在 Linux 经 `systemd-analyze verify` 解析，FD 交接完成一个正例和四个拒绝夹具。KVM 未重新打开，介质仍在准备目录，S 本轮无启动或安装写入。

## 启动命令和权限

每案 root 调度器只向系统 systemd 提交对应的 transient unit `kw-task9-h4-c-s1-boot.service` 或 `kw-task9-h4-c-s2-boot.service`，逐案使用动态非 root UID/GID，额外组限定为现有 `kvm`。设备策略 `closed` 加 `/dev/kvm rw`。capability 集为空，`NoNewPrivileges=yes`。服务处于私有网络、只读系统和隔离临时目录，使用 16 MiB 私有 `/run` 隔离宿主管理 socket。QEMU 树和 FD 校验入口只读映射到该私有 `/run`。 [systemd 执行隔离](https://manpages.ubuntu.com/manpages/resolute/man5/systemd.exec.5.html)

PID 1 用 `OpenFile` 打开本案原盘、只读 seed 和私有串口文件。入口校验三个 FD 的权限、访问方向、原盘 inode、字节摘要与独占锁，再把原盘、介质映射到 FD 3、4，把串口输出定向到私有文件。原盘仍是 `root:root 0600`，无账号组登记、ACL、sudoers、udev 或常驻 unit 变更。KVM 在实际 QEMU 启动期间会被打开并创建 VM，先前的 API 12 查询不代替这次验证。 [systemd OpenFile](https://manpages.ubuntu.com/manpages/resolute/man5/systemd.service.5.html) · [QEMU FD set](https://www.qemu.org/docs/master/system/invocation.html) · [Linux KVM API](https://docs.kernel.org/virt/kvm/api.html)

下列为冻结的 QEMU 参数，`CASE` 在两条私有命令中已分别固定为 `s1`、`s2`。这是 FD 入口完成身份核验后直接执行的 argv，手工从 shell 调用会缺少经过验证的 FD。

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

## 最小审批和可见影响

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
