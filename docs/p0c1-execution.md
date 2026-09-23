# P0-C1 执行与验收记录（待总验收）

本记录是后续隔离验收的运行清单，不预先声明测试通过。执行者应在此填入最终提交、精确源码包 SHA-256、镜像摘要、独立数据库/卷项目名、原始日志路径、命令退出码和测试通过/失败/忽略计数。

## 验收顺序

1. 冻结同一提交和源码清单，核对执行前后每个文件的大小与 SHA-256；不得修改 0001–0010 或 v1/v2 历史契约。
2. 在全新隔离 PostgreSQL 中使用普通非 owner runtime 角色运行 `cargo test --offline --locked -p learning-db --test asset_reading -- --test-threads=1`；运行 `cargo test --offline --locked -p learning-assets --test fs`，核对精确使用授权、撤权与恢复、原字节完整性、固定 Reading 移动及对账两阶段行为。
3. 执行 `cargo fmt --all -- --check`、`cargo test --offline --locked --workspace -- --test-threads=1`、`cargo clippy --offline --locked --workspace --all-targets -- -D warnings`，再以冻结旧程序执行 P0-A/B1/B3-schema/B2 升级夹具并核对旧数据与迁移校验和。
4. 用新 Compose 项目、新数据库和独立资产/暂存卷复验；保留失败现场与原始日志。由独立审查者核对授权、路径/字节完整性、迁移兼容与对账无删除语义。

## 操作注意

- 文件先定稿，数据库后提交；数据库失败后可能留下不可见的物理孤儿。对账前汇总已引用摘要、活跃上传和备份保护键。不能从“报告候选”直接推导删除操作。
- 活跃上传可以只提供上传 UUID，也可以提供该上传的精确暂存或 `.finalizing` 键；任一形式均使本轮摘要候选暂缓。上传结束、保护清单重新计算后才有意义再次检查旧摘要对象。
- 缺失/损坏的已登记原件需要记录明确错误和修复来源；不得以空文件或假图片作为成功响应。C3 导出预检、C4 备份恢复另行验收。

## 最终证据

待总验收后补充。未填完整前不标记 `P0_C1_VERIFIED`，也不标记生产可用。
