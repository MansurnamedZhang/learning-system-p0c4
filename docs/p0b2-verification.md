# P0-B2 验证记录

状态：`IMPLEMENTING / REVIEW_PENDING`，不是生产部署。基线 `182fa2a`，工作分支 `feat/rust-learning-core`。用户要求“推进吧”后执行 B2；UI、图片、关系、影响查询仍属后续阶段。

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

`expanded-green2` 包 SHA256：`99e35a8e39c2603e494efb3d6c0381fba6f3e581637c8d26e8c6a8b51c88c407`，86 个源文件；全量日志 `bc600286b1869ccd4bb382797ecdaaa9d63f85e790740b625d824e14d243a463`。

隐私 RED 包 `6e1027ae0b50535c1696633f19436ed0e9c78f37a3d68ae166165c26ebd54b9f`，失败日志 `e4b8038091b840917b11f1c2d05796b0df3ad2de8652f6782f7c6e77d536689e`；修复后 targeted GREEN 包 `d568c6858c465b6a7fc82a3c6fbf5dcea22d14ed519545d9e690a1cb90e48185`，日志 `f98294c085efd399aa451ba393da41cd4aa3a01e2cf3f571bb535e5d674a6e81`。

每批包含源文件清单、运行前后逐文件哈希、完整命令、实际退出码、日志摘要和 runtime 角色身份。宿主证据在 `C:/Users/hans/Documents/ubuntu_Seoul/learning-system-p0b2/evidence/`；本项目独立复核副本在 `artifacts/p0b2/`，不纳入源码仓库。`.runtime/audit_b2.py` 重新计算日志及 ZIP 内文件摘要，并确认旧迁移、Cargo.lock、旧正文规范未改变。未将密码、DSN 或课程原件打包。

## 验证环境与边界

Linux 测试由既有「本地 Linux 服务器」任务协作，专用目录 `/home/hans/experiments/learning-system-p0b2`，PostgreSQL 18.6，Rust 1.97.0，SQLx 0.8.6，离线锁定依赖。PG 限制 2 CPU / 4 GiB、独立内部网络、无宿主发布端口，普通 runtime 无所有权/超级用户/角色创建权限。

P0-A 与 B1 升级各使用单独空库，旧迁移经 SQLx 正常登记，不手工伪造迁移历史。B1 升级夹具通过真实 ContentStore/CompositionStore/ReleaseStore 创建及重放。并发测试用独立 backend PID 和 Barrier；授权及多块顺序用 pg_blocking_pids 证明等待关系，不以固定睡眠替代锁证据。

最终全量、独立审查和空卷 Compose 结果待补充；只有全部交付门槛满足才改为 `P0_B2_VERIFIED`。验证不包含线上性能、滚动二进制升级、生产身份认证、HTTP/前端或备份恢复。
