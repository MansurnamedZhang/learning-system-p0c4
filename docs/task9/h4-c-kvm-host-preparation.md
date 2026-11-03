# Task9 C 源端 QEMU/KVM 与 guest 磁盘审批清单

2026-11-04。获批的 S 私有解包、一次 KVM API 查询及两块离线磁盘均已完成。FD 修订批次中，新 S1 与 S2 的首次离线启动和停机均已核验。 [首次引导实施单](h4-c-s1-s2-guest-boot.md) [S1/S2 实施单](h4-c-source-pg-and-fixed-view.md)

## S 现场执行与独立回读

S 将 21 个已核包解包到本批私有树，重新核对了所有包和签名基底的字节，并在 S 用实际 loader 验证了 `qemu-img`、`qemu-system-x86_64` 和 SeaBIOS 固件。两个二进制均回读 `10.2.1 (Debian 1:10.2.1+ds-1ubuntu3.3)`。一次 `KVM_GET_API_VERSION` 返回 `12`，关闭后 `/dev/kvm` 的身份、权限和 ACL 与执行前一致。系统 dpkg 状态、组文件和相关单元状态的前后读数一致。

两块 QCOW2 由同一签名基底分别完整复制、独占创建，`root:root 0600`，不同 inode、单链接且无 backing。`qemu-img check` 的错误、损坏和泄漏计数均为零。两块盘同源且尚未启动，离线字节摘要相同，文件独立性以实际 inode 和 backing 记录核验。

| 案例 | 虚拟容量 | 宿主已分配字节 | 最终 SHA-256 |
| --- | --- | --- | --- |
| S1 | 32 GiB | 621,293,568 | `8343cad1375c9064c3f5c6728260d3fc45aadd14cc23d363adf0c448cba454ab` |
| S2 | 32 GiB | 621,293,568 | `8343cad1375c9064c3f5c6728260d3fc45aadd14cc23d363adf0c448cba454ab` |

现场回执 SHA-256 `0292d93010d39484d9ff457d353e5883015ee7bde9e85c0d5baecb63bce598c7`，本地执行索引 `539933a2438f57c5e5d69ac4e63b6c47601054ec14e007dfcd529a48418c031a`，独立只读回执 `ec75c0bba2fb9585a20f03ab3d1ab20bb9553d1ff3e8d60d7c7c07cea58f57e4`。第二个 root 读取过程逐份核对 21 个包、基底、两个磁盘、QEMU/固件及回执，核查设备元数据与单元状态。KVM API 查询总计一次。后续双访问模式 FD 已在新 S1 与 S2 的现场首启中通过。 [首次引导实施单](h4-c-s1-s2-guest-boot.md)

## 软件和镜像

| 对象 | 冻结依据 |
| --- | --- |
| S 主机 | Ubuntu 26.04 `resolute` amd64。现场 APT 缓存返回 `1:10.2.1+ds-1ubuntu3.2`。Canonical 签名的 11 月 3 日 `resolute-updates` 已提供 `.3.3`。以 S 实际 dpkg 状态和签名索引在本地独立解算，固定到 `qemu-system-x86` 和 `qemu-utils` 均为 `1:10.2.1+ds-1ubuntu3.3`。 |
| APT 依赖 | 两次离线模拟均为 23 个新增、零升级、零移除，23 份实际 `.deb` 共 19,851,720 字节，均已逐包匹配签名索引。根包、依赖版本、包名、字节数和 SHA-256 已逐项写入私有审批清单。 |
| 私有运行树 | 仅需要上述 23 项中的 21 项载荷，省去 `ubuntu-virt` 和 `ubuntu-helper-virt-hwe`。21 份共 19,764,120 字节，包含 QEMU 程序、未安装的运行库、固件及数据。S 已装的 13 个依赖库仅用于弥补本地 WSL 测试环境差异。现场核验以 S 实际 loader 为准。 |
| Guest 基底 | Canonical Ubuntu 24.04 LTS amd64 `release-20261017` 的 `ubuntu-24.04-server-cloudimg-amd64.img`。已用官方公布指纹验证 `SHA256SUMS.gpg`，再对 621,284,352 字节整镜像和软件清单回读。镜像 SHA-256 `04e8f2461a16a44e78a370284471f29616cdd9baf1893df25807b71f985b6c94`。QCOW2 v3，原虚拟大小 3.5 GiB，无 backing。 |

常规 APT 安装会运行 `qemu-system-common` 的 `postinst`。本次核对的原包脚本会启用并尝试启动 `qemu-kvm.service`，该 unit 涉及 KVM 模块、KSM 和 hugepages。本批采用 `dpkg-deb -x` 私有解包，安装目标限定为 `kvm-prep/tree`。系统级安装与服务操作另审。 [Ubuntu APT 签名链](https://documentation.ubuntu.com/security/software-integrity/archive-verification/) · [Canonical 镜像校验](https://ubuntu.com/docs/public-images/public-images-how-to/verify-image-checksum/) · [Ubuntu 虚拟化包组成](https://ubuntu.com/server/docs/how-to/virtualisation/virt-hwe/)

## 已批准并执行的最小范围

| 项 | 获批的精确操作 | 回读条件 |
| --- | --- | --- |
| S 私有解包 | 仅在 S 的本批根新增 `kvm-prep`，`root:root 0700`。传输并逐份校验 21 个包与一份签名镜像，包和 `base.img` 为 `root:root 0400`。提取包载荷到私有 `tree`，只对指定 QEMU ELF 验 loader、版本和摘要。 | 现场 dpkg 状态、目录身份与准入 pin 匹配。二进制及 SeaBIOS 摘要一致。系统 APT 安装、service unit 注册或启用属于独立范围。 |
| `/dev/kvm` | 当前为 `root:kvm 0660`、字符设备 `10:232`。普通账号没有读写访问。只用既有 root 会话打开已核对的设备、调用一次 `KVM_GET_API_VERSION`、关闭。 | 返回 API 12，回读 owner/mode、设备编号和 ACL。长期组、ACL、udev、sudo 及未来运行主体的授权均待另审。 |
| 两块磁盘 | 仅在已回读的 S1、S2 私有目录下各独占创建 `guest-root.qcow2`，`root:root 0600`。逐份校验基底完整复制，离线扩至 32 GiB 虚拟大小。 | 两块盘为不同 inode、单链接、无 backing、无共用可写块设备或文件；封存最后 SHA-256、虚拟大小和分配空间。审批前回读两个目录为空，名称未占用，S `/var/lib` 约 198.23 GiB 可用，执行门槛为 80 GiB。 |

本批依次完成宿主加载检查、KVM API 检查和磁盘创建。执行单要求遇到身份漂移、加载错误或部分写入时留证并保持 HOLD。Guest 启动、guest Docker 和 PG18 初始化、固定挂载、信任、22229 及服务启用各自进入后续审批。 [Linux KVM API](https://docs.kernel.org/virt/kvm/api.html) · [QEMU 离线磁盘工具](https://www.qemu.org/docs/master/tools/qemu-img.html)

## 本地验证与现场边界

普通用户在本地 WSL 私有临时树解包 21 项载荷，以 S 已安装的相同版本补足 13 项本地差异。`qemu-img` 与 `qemu-system-x86_64 --version` 均回读 `.3.3`，两份离线 QCOW2 副本完成独占复制、扩容和无 backing 检查。临时副本已由本地测试清理。S 的首轮观察仅回读包状态、设备权限及目录身份，实际执行证据见本页首节。正式源端 ELF 仍等待真实 guest 内的 PG18 和 control 身份。

私有审批清单 SHA-256 `c430ba2888bd8e4de91e71dac561e0298494f8ed41d0337879824cee1391b346`，签名镜像回执 `f8b70ae41c096493fa22ed86e28f8f2d430b6140838c40fe8e569a63d262300a`，新依赖闭包 `6cc764d793d62d5382e845dac1d4110766c2d7dddde8cad96e5ec412850a3707`，本地验证 `8408c58a1e70dcbf54375b71bf43ace5a36ff404d6c6a1d4d466f09a49081478`。
