# Task9 B 密钥托管最小决策

2026-11-03。H4 已在获准的一次探针中测得现有 22 路径上 D 所见的 S 来源，后续 22229 仍待实际验证。两端拟用的全新暂存路径均经普通账号检查，当时未占用。D 本地账户文件未发现候选名称或 UID/GID 2401；这只是 C 的提案，尚须复核实际身份源和管理规则。[H4 现场证据](h4-field-preflight.md)

## 这次决策只覆盖生成和封存

批准后，两端分别使用私有清单中的全新 `/var/lib/knowweave-c4-b1-key-prep-<UUID>`，目录 `root:root 0700`，文件 `root:root 0600`。每把私钥只在所属主机生成和保留。输出仅为公钥、公开指纹及操作证据。

| 所属主机 | 暂存文件 | 固定用途及格式 |
| --- | --- | --- |
| S | `source-auth.pk8` | `ring 0.17.14` Ed25519 PKCS#8 v2，公钥和私钥须由严格 `from_pkcs8` 本机核对。供日后原操作认证。 [S 固定加载器](../../crates/learning-backup/src/offhost_b1/supervisor.rs) · [ring Ed25519](https://docs.rs/ring/0.17.14/ring/signature/struct.Ed25519KeyPair.html) |
| S | `transport-key`、`transport-key.pub` | 现有固定 `/usr/bin/ssh-keygen` 生成 OpenSSH Ed25519，独立于应用认证。为支持未来的非交互运行，私钥由 root 目录和权限保护，公钥另行核验。 |
| D | `destination.pk8` | `ring 0.17.14` ECDSA P-256 固定签名算法的 PKCS#8 v1，公钥为 65 字节未压缩 SEC1。由 D 的 root 私有目录托管，符合既定用户管理安排。 [D 固定加载器](../../crates/learning-backup/src/destination.rs) · [ring ECDSA](https://docs.rs/ring/0.17.14/ring/signature/struct.EcdsaKeyPair.html) |

执行前先完成匹配 Linux 工具链、生成器字节和隔离回归审查。应用密钥用产品锁定版本的 `ring` 标准生成接口；S 应用密钥保持严格的 PKCS#8 v2 加载。B 运行前在已验证的两端现有 root 管理路径中，再核 `/var/lib` 的身份和新目录未占用。只按清单独占创建，不跟随链接，不覆盖或复用路径。写入同步和公钥核验完成后保存证据；若权限拒绝、身份漂移或部分完成，保留该批状态并停止。

本次决策不写任何正式加载路径、`authorized_keys`、`known_hosts`、sudoers 或监听配置，不创建账户、不修改网络规则，也不签发备份或执行恢复。实际公钥齐备后再准备 C 的准确文件和身份差额，正式信任、注册及运行继续另行决定。Task9 保持开放。
