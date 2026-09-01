# RunCove

> 本地开发服务，尽在掌控。

RunCove 是一个 Windows 优先的本地开发服务控制台：实时查看端口占用、注册项目、用结构化的
启动配置拉起服务、按组批量启停、查看会话日志，并按需恢复上次退出前正在运行的项目。发行包
同时包含跨平台的 `runcove` 端口查询 CLI。

当前版本 **v0.4.1**，以免安装的 Windows x64 便携 zip 发布。见
[Releases](https://github.com/AbyssWhalen/RunCove/releases) 和
[CHANGELOG.md](CHANGELOG.md)。

> **English:** RunCove is a Windows-first desktop control center for local development
> services — live port inspection, a trusted project registry, structured launch profiles,
> process-tree control, named launch groups, and on-demand restore. The bundled `runcove`
> CLI also runs on macOS and Linux. Release notes and the changelog are in English.

## 下载与运行

1. 从 [Releases](https://github.com/AbyssWhalen/RunCove/releases) 下载 Windows x64 便携 zip
2. 解压到你自己的目录
3. 运行 `runcove-desktop.exe`

便携版没有安装程序。可执行文件未签名，Windows SmartScreen 可能提示未知发布者——运行前请
确认压缩包来自本仓库的 Release。界面依赖 Microsoft Edge WebView2 Runtime，Windows 11 已
自带，旧系统可单独安装。

发布页的 `SHA256SUMS.txt` 用来校验下载：

```bash
sha256sum -c SHA256SUMS.txt   # 只下了其中一个文件时加 --ignore-missing
```

## 端口与项目

- 每两秒刷新 TCP/UDP 监听状态，在 Windows 允许时显示占用进程的 PID 与详情，并把活跃监听
  和已注册但当前空闲的项目端口合并展示
- 从选定项目或开发根目录发现 npm / pnpm 启动候选，支持 `package.json` workspaces 与
  `pnpm-workspace.yaml`。候选一律需要你确认后才注册，不会自动写入
- 启动配置以 `program` / `args[]` / `cwd` 分开存储，可选期望端口，启动前检测端口冲突
- 启动 / 停止 / 重启，打开目录或浏览器。受管进程树放进 Job Object，停止时一并回收子进程
- 会话日志保存在有上限的内存缓冲里，可过滤、复制、清空
- 显式退出时记录当前启动顺序，下次可按需恢复，逐个等待期望端口就绪
- 关闭按钮询问「最小化到托盘」还是「退出」；托盘提供打开、恢复、全部停止、确认退出。
  顶栏问号打开中英双语的应用内帮助

进程状态统一为 `Idle` / `Starting` / `Running` / `Conflict` / `Exited` / `Unknown`。读不到
的进程信息显示为「不可用」，不会自动提权。IPv4 与 IPv6 分别扫描，IPv6 读取失败时保留可用
的 IPv4 结果并标记为降级，不会用不完整的扫描改变项目状态。

## 启动组

v0.4.0 起。启动组是一个具名、有序的启动配置集合，可以一次启停整套服务。和「恢复」的区别：
恢复只有一份、由上次退出时的状态决定；启动组有名字、可编辑、可以有多个。

- 成员可以跨项目，一个组能同时拉起数据库、API 和前端
- 你设定的顺序就是启动顺序，逐个等待期望端口，与恢复走同一条路径。已在运行的成员算作已
  启动，再点一次只补齐缺失的部分
- 启动失败停在下一个成员之前、保留已启动的部分，并说明卡在哪个成员
- 整组停止按逆序进行；某个成员停不掉不会中断其余成员，最后汇总失败
- 两个组共享同一成员时可以同时启动，第二个会等待而不是失败（v0.4.1 修复）
- 删除启动配置会把它从所有组中移除；成员被清空的组仍然可见并给出说明

启动组只在你点击时启动：没有开机自启，也没有自动启动项目。

> [!IMPORTANT]
> 启动组需要数据库 schema 版本 3，**升级不可逆**：v0.4.0 打开过数据库之后，v0.3.0 及更早
> 版本会拒绝打开它。如果你可能回退，先把
> `%LOCALAPPDATA%\com.abysswhale.runcove\runcove.sqlite3` 复制出来。

## 运行日志归档

v0.3.0 起，**默认关闭**，开关在日志抽屉里。关闭时 RunCove 完全不写日志文件，输出只留在
内存缓冲中。

- 开启只影响此后启动的运行，不会补写正在运行的会话；关闭时已打开的归档正常收尾
- 每个会话一个 JSON Lines 文件，写在数据目录的 `run-log-archives` 下
- **这些文件可能包含你的服务自己打印出的令牌、凭据或个人信息。** RunCove 不做过滤，也不
  上传任何内容——文件的敏感程度等于它捕获的输出
- 单会话上限 10 MiB，目录总量 200 MiB，超出时按最早的已完成归档回收。输出过快时丢弃日志
  行而不拖慢子进程，丢失量一律计入运行历史
- 查看器从文件末尾打开、向前翻页；删除归档需确认，运行历史记录会保留

## 从源码构建

需要 Rust stable（MSVC target，桌面 crate 需要 1.77+）、Node.js 与 npm、Microsoft C++
Build Tools、WebView2 Runtime。

```powershell
git clone https://github.com/AbyssWhalen/RunCove.git runcove
cd runcove\apps\desktop
npm ci
npm run tauri dev        # 开发
npm run tauri build      # 构建 release 可执行文件
```

Tauri 打包是关闭的，公开产物是便携 zip 而不是安装包。

CLI 在仓库根目录：

```powershell
cargo run --bin runcove -- 3000    # 查某个端口
cargo run --bin runcove -- --json  # JSON 输出
cargo install --path .             # 安装
runcove kill 8080                  # 交互式结束占用进程
```

CLI 支持 Windows / Linux / macOS 的 TCP/UDP 查询、进程过滤、端口范围、JSON 输出、watch
模式、在浏览器打开本地端口，以及交互或强制的 `kill`。桌面应用仍是 Windows 优先。发行包里
还有一个兼容旧脚本的可执行文件，新集成请用 `runcove`。

## 架构

```text
runcove/
|- src/                    # 共享的 Rust 扫描器、CLI、渲染、进程助手
|- tests/                  # 扫描器与 CLI 回归测试
`- apps/desktop/
   |- src/                 # React + TypeScript 界面
   `- src-tauri/src/       # Tauri 命令、SQLite、项目发现、进程管理
```

React 前端没有任何文件系统、数据库、端口扫描或进程权限，全部通过带类型的 Tauri 命令和
事件与 Rust 后端通信，这些能力由后端独占。

数据库建在 RunCove 自己的应用数据目录下，按 schema 版本迁移，存放项目、启动配置、期望
端口、受信端口关联、运行会话、恢复顺序、应用设置、归档索引和启动组。它从不打开或修改项目
自己的数据库。**迁移是单向的**：每一步在一个事务里完成，失败则停在原版本，但成功之后没有
降级路径。首次运行 v0.4.x 前请备份数据目录；v0.4.1 相对 v0.4.0 没有 schema 变化。

端口归属有明确的信任顺序：RunCove 自己启动并管理的进程树 > 你明确确认过的关联 > 从进程
信息推断出的建议。只有前两者会持久化，轮询快照不保留。

## 隐私与进程安全

- 全部本地运行，不上传项目、进程、端口或日志数据
- 会话输出默认只在内存缓冲里。可选的运行日志归档是唯一写文件的路径，默认关闭，只写在
  RunCove 自己的数据目录下
- 项目发现只读取包元数据，不读也不改 `.env`
- 不自动申请管理员权限。顶栏盾牌可以显式通过 UAC 重启以获得更完整的进程可见性，但该实例
  是**只读**的，会禁用启动、停止、重启、恢复和结束外部进程等所有动作
- 结束外部进程树前界面要求确认，后端会重新核对 PID、进程启动时间、可执行路径和归属，身份
  变化或无法核实则拒绝
- 启动配置不拼接 shell 命令字符串，可执行文件、参数数组、工作目录分开存储。每条退出路径
  都会记录恢复集并停止 RunCove 管理的进程树

## 开发检查

仓库根目录（Rust 主包）：

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

`apps/desktop`（前端与 Tauri）：

```powershell
npm run lint && npm run typecheck && npm test -- --run && npm run build
npm run e2e
npm run tauri build
```

`apps/desktop/src-tauri` 是**独立的 Cargo 包**，不是根包的 workspace 成员，需要单独跑
fmt / clippy / test。Playwright 走查覆盖三个主视图、日志、启动组、项目导入、浏览器控制台
错误，以及 900x600 / 1280x720 / 1440x900 三个视口的溢出。`target/`、`dist/`、
`node_modules/`、运行时数据库和捕获的日志都不是源码产物，不要提交。

## 不包含的功能

有意不做的，不是待办：开机自启或自动启动项目、Docker 与远程主机管理、设备预览、Git 状态
集成、`.env` 编辑、使用时长统计、安装包或包管理器分发。

## License

[MIT](LICENSE) - Copyright (c) 2026 AbyssWhalen and RunCove contributors
