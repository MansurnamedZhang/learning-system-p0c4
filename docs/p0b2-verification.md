# P0-B2 验证记录

状态：`P0_B2_VERIFIED / NOT_PRODUCTION`。基线 `182fa2a`，工作分支 `feat/rust-learning-core`。独立审查、缺陷修复、宿主与全新 Compose 验收均已完成；UI、图片、关系、影响查询仍属后续阶段。

## 已取得的证据

| 固定批次 | 实际结果 |
|---|---|
| 原 B1 固定包复跑 | 68 项通过；旧迁移与原接口基线成立 |
| Core 契约 / 锚点 | 4 项契约、2 项锚点均先行为失败再通过；后加独立 Python golden、样例合同与总量边界 |
| schema RED → GREEN | 缺表行为失败；新表权限 1 项和原 PostgreSQL 20 项通过 |
| reading RED → GREEN | 3 项 Storage stub 失败 → 3 项通过；同时 schema 通过、Clippy 无警告 |
| migration RED → GREEN | 2 项 Storage stub 失败 → 2 项通过；纯分类 2 项失败 → 通过 |
| expanded-green2 | 三个全新库，全 workspace 93/93，P0-A 与真实 B1 store 升级通过 |
| history-privacy-red2 | 1 项真实失败：当前来源不可用时旧来源锚点仍泄露；不是编译或连接失败 |
| boundaries-green1 | 隐私回归修复；迁移 5、容量 1、schema 2、权限 2，共 10/10 |
| prereview | 宿主三新库 101/101，fmt/Clippy/test 均 exit 0 |
| manual-merge-red | 人工合并应有两条旧组映射，实际一条；真实 PostgreSQL 断言失败 |
| reviewed-final 宿主 | 三个新库，105/105；人工合并单项 1/1；fmt/Clippy/workspace 均 exit 0 |
| compose-reviewed-final | 全新项目/卷，105/105；config、标准 build、up、stop 均 exit 0 |

最终宿主全量日志 SHA256：`f3f855044fbf53aedf677bfb962f2dc2711fa15f3ee235b9e56cbbbb33e260ff`；Clippy 日志 `39c36223aa879908fd7a1b9812e8db5a2c36f67f445fefeff5c6b6fb67606915`。105 项由 core 23、DB 纯单元 8、原 B1/P0-A 集成 51、新增 B2 集成 23 构成；原有 68 项继续保留，共新增 37 项。

## 独立审查与修复

独立只读审查范围 `182fa2a..b213d6e`，没有 Critical，三个 Important：待放置项按随机 placement UUID 排序；显式待放置仍返回历史锚点作为有效 location；人工安置合并只保留一侧旧组映射。三个缺陷均先复现，再在一次修复阶段处理，没有用二次审查替代测试。

顺序回归使用故意反序 UUID，修复前返回 second/third/first 而预期 first/second/third；location 回归在当前与旧来源均可读时仍要求 null。两项本地单元已 RED→GREEN。人工合并数据库 RED 日志 `ce895f31042e48176ae43bbf707f7f1aeddfac56fda19c8181e14406069a65ff`；修复改为一次记录所有旧组→新组，并追加收据故障回滚及历史视图检查。

审查提出嵌套迁移覆盖建议；因施工单承诺验证路径身份，执行者将其作为验收缺口处理。新增重复章节路径、父路径消失、跨父移动、嵌套空章节变化的确定性单测，原算法即通过，没有虚称行为 RED。没有延后 Minor。

审查明确未判断最终 Compose（当时尚未运行），也不评价范围外 HTTP/UI/资产/关系、生产吞吐、备份及在线升级；前者保留为交付门槛，后者保留阶段边界。

`expanded-green2` 包 SHA256：`99e35a8e39c2603e494efb3d6c0381fba6f3e581637c8d26e8c6a8b51c88c407`，86 个源文件；全量日志 `bc600286b1869ccd4bb382797ecdaaa9d63f85e790740b625d824e14d243a463`。

隐私 RED 包 `6e1027ae0b50535c1696633f19436ed0e9c78f37a3d68ae166165c26ebd54b9f`，失败日志 `e4b8038091b840917b11f1c2d05796b0df3ad2de8652f6782f7c6e77d536689e`；修复后 targeted GREEN 包 `d568c6858c465b6a7fc82a3c6fbf5dcea22d14ed519545d9e690a1cb90e48185`，日志 `f98294c085efd399aa451ba393da41cd4aa3a01e2cf3f571bb535e5d674a6e81`。

每批包含源文件清单、运行前后逐文件哈希、完整命令、实际退出码、日志摘要和 runtime 角色身份。宿主证据在 `C:/Users/hans/Documents/ubuntu_Seoul/learning-system-p0b2/evidence/`；本项目独立复核副本在 `artifacts/p0b2/`，不纳入源码仓库。`.runtime/audit_b2.py` 重新计算日志及 ZIP 内文件摘要，并确认旧迁移、Cargo.lock、旧正文规范未改变。未将密码、DSN 或课程原件打包。

## 验证环境与边界

Linux 测试由既有「本地 Linux 服务器」任务协作，专用目录 `/home/hans/experiments/learning-system-p0b2`，PostgreSQL 18.6，Rust 1.97.0，SQLx 0.8.6，离线锁定依赖。PG 限制 2 CPU / 4 GiB、独立内部网络、无宿主发布端口，普通 runtime 无所有权/超级用户/角色创建权限。

P0-A 与 B1 升级各使用单独空库，旧迁移经 SQLx 正常登记，不手工伪造迁移历史。B1 升级夹具通过真实 ContentStore/CompositionStore/ReleaseStore 创建及重放。并发测试用独立 backend PID 和 Barrier；授权及多块顺序用 pg_blocking_pids 证明等待关系，不以固定睡眠替代锁证据。

最终修复包 SHA256 `5d90ca9c4d5ff5597f9ceed70753f9203920923f0c18543e7626c476889c49de`，93 源文件，对应实现 `b213d6e` 与审查修复 `de71766`。测试后仅更新交付说明，代码/依赖/迁移/deploy 均逐字节核对。

最终 Compose 项目 `learning-system-p0b2-reviewed-final`，标准 BuildKit 构建直接成功，无代理回退或 daemon 配置变更。测试镜像 index 为 `sha256:ad08a801b13b8212d2705fe220c17d69eddb8679a561e7fe13caaedb5c09b39d`，平台 manifest 为 `sha256:a34ca501b2a739fbd281c794fd4120540beb77b499b1156a81c06a4e6ac9f78c`；官方 Rust/PG 固定 digest 不变。构建日志 SHA256 `d3b2338acd1878921349dbfe1cc977dcc527121c278213dd3a62ec6407088197`，全量 up 日志 `ddda8dbe2a6fdc6ad67eb87b93dc6ec6817cafa07c347c36d0f091d228b92106`。

独立证据复核覆盖 14 批服务器结果、31 份日志及 10 份本地行为测试日志。两个最终全量均明确 105 passed / 0 failed / 0 ignored；93 个源文件运行前后一致。所有三个 B2 专用容器 Exited(0)，PG 2 CPU / 4 GiB，test 4 CPU / 4 GiB，内部网络、无宿主端口；数据卷保留。原有 dev-postgres、dev-redis、dev-neo4j、process-log 继续运行。

验证不包含线上性能、滚动二进制升级、生产身份认证、HTTP/前端或备份恢复。下一关为 B3 关系与认识审查；B2 通过不表示 P0-B 全部完成。完整执行取舍见 [执行账本](p0b2-execution.md)。
