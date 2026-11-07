# 知织阅读工作台 · 合成阅读与笔记预览

此目录是 P1 首用切片 B0–B2 的独立 Cargo 工作区。启动器每次创建一个临时 PostgreSQL 18 集群，通过已有 `ContentStore`、`CompositionStore` 和 `ReadingStore` 生成合成 Attention 文档。读取使用只读角色，保存使用独立角色，其写权限限于笔记事务所需的追加表和两个修订头列。[PostgreSQL 权限语法](https://www.postgresql.org/docs/18/sql-grant.html)

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

打开终端给出的地址，输入学习者访问码。在融合或个人视图下方找到“追加私有笔记”，输入正文并保存。笔记追加在合成文档指定的间隙，刷新可读取同一次运行中已保存的笔记。使用访客访问码可看到私有阅读的拒绝页。按 Ctrl+C 停止，启动器会核验停机并清除本次临时数据库。

## 保存和冲突

- 保存结果暂未确认时，使用“重试同一次保存”。浏览器沿用原请求 ID 和请求体，服务器返回同一回执。
- 另一页先保存时，旧修订的提交会被拒绝。草稿留在当前标签页并可编辑。点“载入最新修订”，检查后再按“保存笔记”。
- 正文上限为 4,000 个 UTF-8 字节，按纯文本显示。未保存的草稿保存在当前页面内存。整页刷新或关闭会清空草稿。

## 验证与范围

在启动命令后附加 `--verify` 运行浏览器与数据库验证。此模式需要 Node.js 和 Playwright，Windows 使用已安装的 Edge。可附加 `--evidence` 加一个已有目录，记录桌面与窄屏截图。

- 同一固定修订在三种视图及两种视口一致。重复出现共享块修订，各自有独立出现路径。
- 展开来源和版本，合成标记按纯文本渲染。访客、无会话、伪造 actor、错误 Host 与跨源登录请求均按权限测试。
- 测试在同一学习者会话中撤销两种授权。撤销来源后原文与来源标识消失，本人笔记列为未定位。撤销个人空间后阅读不可见，退出后旧会话失效。
- 同时提交两个相同请求，丢弃浏览器收到的第一份响应。重试返回原回执，数据库只新增一个笔记块和一个回执。
- 两页使用同一个旧修订。第一份保存成功后，第二页收到 409 并保留草稿。显式载入最新版后再次保存。验证来源不变、访客和撤权后写入被拒，以及拒绝请求不增加数据库行数。

当前为本机回环中的合成 V1 文本和平铺组合。正式产品登录、持久部署和真实资料使用按产品准入条件评审。[产品接口设计](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/feat/task7-attention-recovery/docs/architecture/07-product-interfaces.md) · [Axum 本地监听 API](https://docs.rs/axum/0.8.9/axum/fn.serve.html) · [PostgreSQL 18 临时集群初始化](https://www.postgresql.org/docs/18/app-initdb.html)
