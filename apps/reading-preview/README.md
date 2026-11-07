# 知织阅读工作台 · 合成只读预览

此目录是 P1 首用切片 B0/B1 的独立 Cargo 工作区。启动器每次创建一个临时 PostgreSQL 18 集群，通过已有 `ContentStore`、`CompositionStore` 和 `ReadingStore` 生成合成 Attention 文档。HTTP 服务只拿临时只读角色的凭据。页面按固定阅读修订展示原文、融合或个人视图，可展开来源版本与出现路径。

预览固定监听 `127.0.0.1` 的临时端口。学习者与访客分别使用启动时生成的合成访问码，访问码只在本次运行有效。登录后，服务端会话决定读取身份。两种访问码用于演示同一份阅读对两种身份的不同结果。

## 启动

先在仓库根目录构建。需要仓库锁定的 Rust 工具链、Python 3，以及 PostgreSQL 18 的 `postgres`、`initdb`、`pg_ctl` 和 `psql`。工具可以来自本机已有环境或私有解包目录。

```sh
cargo build --locked --manifest-path apps/reading-preview/Cargo.toml
python3 apps/reading-preview/launch.py \
  --binary apps/reading-preview/target/debug/knowweave-reading-preview \
  --pg-bin /usr/lib/postgresql/18/bin
```

在 Windows 使用本机编译的 `.exe`，将数据库工具放在已有的 WSL 发行版内。以下示例中的 PostgreSQL 路径应对应实际安装位置。私有解包环境需要额外的共享库时，加 `--pg-lib` 和该 Linux 绝对目录。

```powershell
cargo build --locked --manifest-path apps/reading-preview/Cargo.toml
python apps/reading-preview/launch.py --wsl Ubuntu --pg-bin /usr/lib/postgresql/18/bin --binary apps/reading-preview/target/debug/knowweave-reading-preview.exe
```

打开终端给出的地址，输入学习者访问码。使用访客访问码可看到私有阅读的拒绝页。按 Ctrl+C 停止，启动器会先核验临时数据库停机，再清除本次专用目录。浏览器的登录码只在合成预览中使用。

## 验证与范围

在启动命令后附加 `--verify` 运行浏览器验证。此模式需要 Node.js 和 Playwright，Windows 使用已安装的 Edge。可附加 `--evidence` 加一个已有目录，记录桌面与窄屏截图。

- 同一固定修订在三种视图及两种视口一致。重复出现共享块修订，各自有独立出现路径。
- 展开来源和版本，合成标记按纯文本渲染。访客、无会话、伪造 actor、错误 Host 与跨源登录请求均按权限测试。
- 测试在同一学习者会话中撤销两种授权。撤销来源后原文与来源标识消失，本人笔记列为未定位。撤销个人空间后阅读不可见，退出后旧会话失效。

当前为合成 V1 文本和平铺组合，业务接口只读。个人笔记写入、保存冲突和正式产品登录进入后续切片。真实资料使用另有准入条件。[产品接口设计](../../docs/architecture/07-product-interfaces.md) · [Axum 本地监听 API](https://docs.rs/axum/0.8.9/axum/fn.serve.html) · [PostgreSQL 18 临时集群初始化](https://www.postgresql.org/docs/18/app-initdb.html)
