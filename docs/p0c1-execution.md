# P0-C1 执行与验收记录

本记录保留执行清单；[最终验证记录](p0c1-verification.md)给出精确提交、源码包与清单 SHA-256、镜像与卷身份、隔离项目、原始日志、退出码、通过/失败/忽略计数，以及两次控制器失败的原始证据。C1 结果为 `P0_C1_VERIFIED / NOT_PRODUCTION`。

## 验收顺序

1. 冻结同一提交和源码清单，核对执行前后每个文件的大小与 SHA-256；不得修改 0001–0010 或 v1/v2 历史契约。
2. 在全新隔离 PostgreSQL 中使用普通非 owner runtime 角色运行 `cargo test --offline --locked -p learning-db --test p0c1_acceptance --test assets --test asset_reading -- --test-threads=1`；运行 `cargo test --offline --locked -p learning-assets --test fs --test upload_declaration -- --test-threads=1`，核对综合 Attention 原件/图片/附件、精确使用授权、撤权与恢复、上传声明及两次跨卷写入限额、媒体声明、原字节完整性、固定 Reading 移动及对账两阶段行为。具体测试能否计入最终通过，以冻结候选的 Linux 原始日志为准。
3. 执行 `cargo fmt --all -- --check`、`cargo test --offline --locked --workspace -- --test-threads=1`、`cargo clippy --offline --locked --workspace --all-targets -- -D warnings`，再以冻结旧程序执行 P0-A/B1/B3-schema/B2 升级夹具并核对旧数据与迁移校验和。
4. 用新 Compose 项目、新数据库和独立资产/暂存卷复验；在**实际挂载**的 assets/staging 路径上用外部只读源码探针调用公开文件适配器，检查 PDF/PNG/Notebook 原字节、摘要、大小、存储键及重新打开后的字节。记录卷身份、挂载、镜像摘要、容器权限/资源限制/端口/网络检查。保留失败现场与原始日志，由独立审查者核对授权、路径/字节完整性、迁移兼容与对账无删除语义。

## 操作注意

- 文件先定稿，数据库后提交；数据库失败后可能留下不可见的物理孤儿。对账前汇总已引用摘要、活跃上传和备份保护键。不能从“报告候选”直接推导删除操作。
- 活跃上传可以只提供上传 UUID，也可以提供该上传的精确暂存或 `.finalizing` 键；任一形式均使本轮摘要候选暂缓。上传结束、保护清单重新计算后才有意义再次检查旧摘要对象。
- 缺失/损坏的已登记原件需要记录明确错误和修复来源；不得以空文件或假图片作为成功响应。C3 导出预检、C4 备份恢复另行验收。
- 上传由可信服务策略提供 `max_size_bytes`，必须声明 `expected_size_bytes`；staging 和 assets 两段复制都逐块限制且要求最终字节数吻合。声明为 PDF/PNG/Notebook 时只做固定签名或 Notebook JSON 形状检查，非完整解析或恶意内容扫描；`application/octet-stream` 是不进行类型特异性检查的通用回退。媒体字段先通过语法校验；对语法合法的字段，同请求键不同内容由历史收据规则优先判冲突，其后才验证新媒体字节和支持范围。

## 最终证据

冻结应用提交 `a962ab8faa6abc2506f2f47d3e23ae99e2ee781c`，源码包 SHA-256 `79a24f6f158f25f1edc470956e1232c9bc9b7a38f75b7abedd19986ce9ca55ec`。隔离 Linux 主门综合夹具 1/1、工作区 328 通过/0 失败/3 既有忽略，格式/严格 Clippy/四套旧程序升级退出 0，78/78 证据 SHA 与源码前后一致。新 Compose 项目真实 assets/staging/evidence 卷 A/B 持久字节探针通过，39/39 证据 SHA；同项目补 PostgreSQL 四卷、健康、五空库与非 owner runtime，并在 PG 运行时经只读挂载复读 B，13/13 阶段退出 0、60/60 SHA 一致。独立源码审查无阻断。具体路径与失败重试历史见[最终验证记录](p0c1-verification.md)。此状态不扩大到 C2–C4、P1 或生产环境。
