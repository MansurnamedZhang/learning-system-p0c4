# Task9 C 停机根盘扩容与 PG18 准备介质

2026-11-04。S1/S2 停机原盘、GPT 主备表和 ext4 超级块已只读回读。修正版 S1/S2 扩容和停机独立回读已完成。旧扩容批次继续留存 HOLD。迁移 ELF 和两案 NoCloud 准备介质已封存，PG18 首启仍是独立审批。 [PG18 离线引导](h4-c-pg18-offline-freeze.md) · [两案首启停机回执](h4-c-s1-s2-guest-boot.md)

## 2026-11-04 · 修正版 S1/S2 扩容与独立回读通过

修正版在 S 上先生成并核验 S1，随后生成 S2。新副本均为无 backing 的独立 QCOW2。独立会话重新验证两块原盘、两块新盘的身份与哈希、GPT、ext4 UUID 和可用块数，并确认两案仍为停机状态。旧 S1 候选与 HOLD 回执通过再次核对。

| 案例 | 根文件系统 | 停机回读可用空间 | 副本 SHA-256 |
| --- | --- | --- | --- |
| S1 | 6 GiB | 4461035520 字节 | `0a6650a90a8aab425ca77482c6c25d779570930156b2cd1525869a3dc76a40b3` |
| S2 | 6 GiB | 4461035520 字节 | `b2f8abf1c62d6d80e5bc45c1dae0dcd0e903492bfd21e1de73bc50feda2828cc` |

两案均达到 2 GiB 的停机容量门。该回执适用于这两块新盘。正式 PG18 首启还须绑定独立启用介质和运行单，并在 guest 内实测可用容量。PG18 首启保持独立审批。新批次结果 `c259bec630d98a2b5f8ea3ca1a66aff3fff5c3da23620c3469811ca147f64b47`，独立回读回执 `3acd7277225823e8ec3e41f75f0c2fad84522b0cc50e44860867b553c1153fff`。

## 2026-11-04 · S1 产物通过只读回检，双案批次 HOLD

已按冻结范围完成 S1 的镜像生成。协调器在封存回执前，直接比较了 Python 元组与 JSON 数组，误拒绝内容相同的区间。此时 S2 尚未进入执行。独立只读回检确认 S1 候选盘结构、GPT、ext4、UUID、数据区比对和停机状态通过，实测可用空间 `4461035520` 字节。两块原盘哈希均与扩容前一致。旧批次没有双案通过回执，继续保留 HOLD。

修正版先将区间编码成与审批清单相同的数组，再逐项比较。两案正例与三个区间变化负例均通过。拟从同一两块只读原盘在新目录重做完整双案，写入扇区和每案 2 GiB 门槛保持原范围。重新执行待修正版审批，PG18 首启独立审批。

旧批次命令回执 `a3fb79bf90055248e5ed6053a8ac2a9f1706501279e74b1812e871de721075e0`，S1 只读回检回执 `a4e0a74f2a3dfaa8dcbf8c9e7c0856ee7c7c93bad5cd9d43910357e4271abbc1`，修正版审批清单 `e5e9e8ed64cc504e4eb55b5f15a878923d9194c782056922c534d66e24c4d47b`，回归回执 `328096a5bc3366a53174964d3040d7b0605df69298b830bc674ba536806e9a8b`，协调器静态回执 `6abb6713983449b478ddb88683c0433315f0252b16112aa831bbf4f97a7aea19`。

## 扩容范围与准确命令

两案各保留 32 GiB 原始 QCOW2，现有根分区 1 从 LBA `2099200` 到 `7339998`，根文件系统各有 `821645312` 字节可用。拟各做一块无 backing 的新副本。每块副本的根分区末端增至 `14682111`，起点、分区 GUID、ext4 UUID 以及分区 14、15、16 保持。GPT 备份表从旧盘尾迁到 32 GiB 末尾，最后 LBA `67108863`，旧备份位于新根分区范围内。最终 ext4 为 6 GiB。新的固定下限仍为每案 `2147483648` 字节可用。

实际路径、独立案名、工具绝对路径和哈希均由封存私有清单绑定。下列命令用同义变量表示，实际按清单 argv 执行。

```sh
"$QEMU_IMG" convert -f qcow2 -O raw -S 4096 "$STOPPED_ORIGINAL" "$RAW"
"$QEMU_IMG" compare -f qcow2 -F raw "$STOPPED_ORIGINAL" "$RAW"
/usr/sbin/e2fsck -f -n "$EXT4_COPY"
/usr/sbin/e2fsck -f -p "$EXT4_COPY"
# 私有普通文件通过 ftruncate 扩至 6442450944 字节。
/usr/sbin/resize2fs "$EXT4_COPY" 1572864
/usr/sbin/e2fsck -f -n "$EXT4_COPY"
# 将 ext4 复制回 RAW，更新保护 MBR、主 GPT 与新盘尾备 GPT。
"$QEMU_IMG" convert -f raw -O qcow2 -o compat=1.1,cluster_size=65536,lazy_refcounts=off -S 4096 "$RAW" "$GROWN_COPY"
"$QEMU_IMG" compare -f raw -F qcow2 "$RAW" "$GROWN_COPY"
"$QEMU_IMG" check -f qcow2 --output=json "$GROWN_COPY"
"$QEMU_IMG" info -f qcow2 --output=json --backing-chain "$GROWN_COPY"
```

协调器先从 RAW 提取分区 1 到私有普通文件。两次只读 fsck 与一次可写 fsck 都要求退出 0，可写检查只更新工作副本并使其满足 resize2fs 的新鲜检查要求。RAW 修改范围按 LBA 限于 `0–33`、`2099200–14682111`、`67108831–67108863`，范围外计算全字节摘要并对比。无需挂载分区、绑定块设备、启动虚拟机或写入原盘。 [QEMU 磁盘工具](https://www.qemu.org/docs/master/tools/qemu-img.html) · [resize2fs 参数](https://man7.org/linux/man-pages/man8/resize2fs.8.html)

## 停机回读与失败处理

1. 用 S 既有 root 在原案例根下独占创建一个新 `0700` 批次目录，仅写两个隔离子目录的 RAW、ext4 工作文件、新 QCOW2、日志和回执。启动前核停机 unit、原盘 inode、模式、原始哈希、工具哈希及 S 宿主至少 64 GiB 可用，旧 S1 HOLD 单独复核。
2. 先处理 S1。原件哈希保持，输出 QCOW2 结构检查无错误、泄漏及损坏，无 backing，主备 GPT 校验、非根分区字节、ext4 UUID 与 6 GiB 容量通过。用 QCOW2 重新只读提取 GPT 和 ext4 超级块，每案的可用空间以空闲块减保留块后的值为准，至少 2 GiB 才封存新的 inode、大小和 SHA-256。
3. S1 通过并确认继续停机后才进入 S2。任何失败保留工作文件和现场证据，批次进入 HOLD。两案的通过回执供下一轮 PG18 启动绑定。PG 首启还须在 guest 内检查实时可用空间。

签名 Ubuntu 基础镜像的非现场副本已完成一次完整离线演练，扩容后读回 `4472033280` 字节可用。六个负例分别覆盖 GPT CRC、起始扇区、其他分区边界、UUID、容量与可用空间。S1/S2 现场容量仍各为 `821645312` 字节。S 只读回执 `aae076d349980d1b12cfb4c0b55f7e48d44684d7a001325e2b05b2728b02123d`，演练回执 `d1a2f206678fba75a03368e383ed5173395ca0ad55392b5da50374eded6e2313`，审批清单 `0493fa8d4e20b5f2ad4570e2f6cb359b49a83b5efbb84a1d1c49d579e1ec3d17`，协调器静态回执 `c4e9d3e1d9b74eb6682a12762d5a695e386b715fae463bff7414f36f457066e5`。

## 迁移 ELF 与 NoCloud

固定 Linux/amd64 builder 无网络构建迁移程序。已核对 ELF 无动态解释器和动态依赖、配置契约单测通过、不支持的 profile 在连接数据库前拒绝。迁移 ELF `7432544` 字节，SHA-256 `114dbf837df88e93ad229b5db76b0f86fe849b8bf280cec26d8eff4fd8e4448d`，构建回执 `005e611db9ff5c86f905014ce87a7159734b0e9ef64f34cc475dec84bd0d19b8`。

| 案例 | CIDATA 准备 ISO 字节 | SHA-256 |
| --- | ---: | --- |
| S1 | 375373824 | `dd6799c0975ee3e36c32a5e79105e9e5289af7c7ea66c4e6b81d687859d236dc` |
| S2 | 375373824 | `0e0d889c1bac99e7f681198c7fc9da0c9addbe0c53699966d515404d58ead65b` |

每案 ISO 封存 PG18 OCI、迁移 ELF、七份容器工具 ELF 和本案配置，使用不同 NoCloud instance-id。两案通过 cloud-init schema、Rock Ridge 与 Joliet 文件逐项哈希、错案及错清单拒绝检查。准备介质只含只读载荷校验入口，正式 PG18 首启需在扩容回执核验后封存独立启动入口和启用介质，再提交审批。回执 `c77eae19d56292c3c727aee4ca07de206d4b84cf93c10b3b7ca5756de7a60cff`。 [NoCloud 介质格式](https://docs.cloud-init.io/en/26.1/reference/datasources/nocloud.html)
