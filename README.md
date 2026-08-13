# RunCove

> Local dev services, under control.

RunCove v0.2.1 is a Windows-first desktop control center for local development services. It combines live port inspection with a trusted project registry, structured launch profiles, process-tree control, session logs, and on-demand restoration of the projects that were running before the app exited.

The distribution also includes the cross-platform `runcove` port-inspection CLI. The desktop application is Windows-first; the CLI is available on Windows, macOS, and Linux.

The v0.2.1 release is distributed as a portable Windows x64 zip from the [RunCove Releases page](https://github.com/AbyssWhalen/RunCove/releases). The original v0.1.0 CLI release remains available in the release history for existing installations.

## Current Release / 当前版本

RunCove v0.2.1 improves daily Windows usability and diagnostics without changing
the local-only data or process-safety boundaries. / RunCove v0.2.1 在不改变本地数据与
进程安全边界的前提下，重点增强 Windows 日常使用体验和诊断能力。

- **Run history / 运行历史:** Overview shows the five most recent managed
  sessions, while the history drawer can search and filter up to 200 stored
  records, retain deleted-project entries, and locate profiles that still
  exist. / 概览显示最近 5 次托管会话；历史抽屉可搜索、筛选最多 200 条记录，保留已删除
  项目的历史，并定位仍然存在的项目配置。
- **Conflict recovery / 冲突处理:** Expected-port failures offer a
  `View occupant` action that refreshes the snapshot and focuses the exact TCP
  or UDP row. A disappeared listener is reported as changed instead of exposing
  stale process actions. / 预期端口冲突可通过“查看占用”刷新快照并定位到准确的 TCP 或 UDP
  行；占用已消失时会提示状态变化，不提供过期的进程操作。
- **Project discovery / 项目发现:** Saved-root scans now expose scanning,
  candidate, empty, and failure states with retry. Candidates still require
  review and are never imported or launched automatically. / 已保存开发根目录的扫描现在
  会明确显示扫描中、发现候选、无结果和失败状态，并支持重试；候选项目仍须人工确认，绝不会
  自动导入或启动。
- **Project editing / 项目编辑:** Launch profiles can be copied, and
  field-level validation covers required project/profile data, blank arguments,
  valid port ranges, and duplicate protocol/port pairs. / 启动配置支持复制；字段校验覆盖
  项目与配置必填项、空参数、端口范围以及重复的协议/端口组合。
- **Port details / 端口详情:** PID, executable path, and command line can be
  copied directly, with explicit clipboard failure feedback. / PID、可执行文件路径和
  命令行可直接复制，剪贴板失败时会明确提示。
- **Log boundary / 日志边界:** Run-session metadata is retained for history,
  but console logs remain in bounded memory and are not archived. Historical
  sessions cannot reopen old logs after RunCove exits or the buffer is cleared.
  / 运行会话元数据会保留用于历史记录，但控制台日志仍只存在于有界内存中，不会归档；
  RunCove 退出或缓冲区清空后，历史会话无法重新打开旧日志。

## Download And Run

1. Download the RunCove v0.2.1 Windows x64 portable zip from the [Releases page](https://github.com/AbyssWhalen/RunCove/releases).
2. Extract the zip to a directory you control.
3. Run `runcove-desktop.exe`.

RunCove v0.2.1 is portable and does not include an installer. The executable is currently unsigned, so Windows SmartScreen may show an unknown-publisher warning; verify that the archive came from this repository's release before choosing to run it. RunCove uses the Microsoft Edge WebView2 Runtime, which is included with current Windows 11 installations and can be installed separately on older or stripped-down systems.

## Desktop App

The desktop app is the primary RunCove experience. Its compact `Overview`, `Ports`, and `Projects` views cover the normal local-development loop:

- Refresh TCP and UDP port state every two seconds and show the owning PID and process details when Windows permits access.
- Combine active listeners with registered project ports that are currently idle.
- Discover npm or pnpm launch candidates from a selected project or scan a development root for multiple projects, including `package.json` workspaces, block or inline `pnpm-workspace.yaml` package lists, and lockfile-based package-manager detection. The last successful root is rescanned once on startup; new candidates stay non-blocking and require review before registration.
- Store launch profiles as `program`, `args[]`, and `cwd`, with optional expected ports.
- Start, stop, and restart a profile; open its directory or TCP port; and detect expected-port conflicts before launch.
- Capture stdout and stderr in an in-memory session log bounded both per line and across all profiles, with filtering, copy, and clear controls.
- Keep managed Windows process trees in Job Objects so stop and exit operations clean up child processes as well as their parent command.
- Save the active launch order on explicit exit and restore it on demand, waiting for each profile's expected ports before starting the next one.
- Use the Windows title-bar close button to choose between hiding to the system tray and safely quitting. The optional remembered choice can be reset from Help > Safety; the tray still exposes open, restore, stop-all, and confirmed exit actions.
- Open the in-app Help and usage guide from the top-bar question-mark button. It explains the first-run workflow, ports, projects, run history, conflict recovery, permissions, and safety boundaries in English or Simplified Chinese, with links back to Ports and Projects.

RunCove uses the shared status model `Idle`, `Starting`, `Running`, `Conflict`, `Exited`, and `Unknown`. Missing process metadata is reported as unavailable rather than triggering automatic elevation.

Windows IPv4 and IPv6 listeners are scanned independently. If an IPv6 table cannot be read, RunCove keeps the usable IPv4 results, marks the snapshot as degraded, and avoids changing project status from an incomplete scan.

## Build And Run

### Prerequisites

For the Windows desktop app, install:

- A current stable Rust toolchain with the MSVC target (the desktop crate requires Rust 1.77 or newer)
- Node.js and npm
- Microsoft C++ Build Tools
- Microsoft Edge WebView2 Runtime

Clone the repository, then start the desktop app:

```powershell
git clone https://github.com/AbyssWhalen/RunCove.git runcove
cd runcove\apps\desktop
npm ci
npm run tauri dev
```

Build the release executable from the same directory:

```powershell
npm run tauri build
```

Tauri bundling is disabled because the public v0.2.1 artifact is a Windows x64 portable zip rather than an installer. The release executable is written below `apps/desktop/src-tauri/target/` and then packaged with the release documentation.

### CLI

Run the new CLI directly from the repository root:

```powershell
cargo run --bin runcove --
cargo run --bin runcove -- 3000
cargo run --bin runcove -- --process node
cargo run --bin runcove -- --range 3000-4000 --json
cargo run --bin runcove -- --watch -w 2
```

Install the RunCove CLI from source:

```powershell
cargo install --path .
runcove --version
```

The primary command surface is `runcove`:

```powershell
runcove
runcove 8080
runcove --all --json
runcove kill 8080
runcove open 3000
```

Release archives also contain a compatibility executable for existing scripts.
New integrations should use `runcove`.

The CLI supports TCP/UDP inspection on Windows, Linux, and macOS, including process filters, port ranges, JSON output, continuous watch mode, opening a local TCP port in the browser, and an interactive or forced `kill` command. The desktop app remains Windows-first.

## Architecture

```text
runcove/
|- src/                         # Shared Rust scanner, CLI, renderers, process helpers
|- src/main.rs                  # runcove binary
|- src/bin/portpeek.rs          # Compatibility CLI entry point
|- tests/                       # Scanner and CLI regression tests
`- apps/desktop/
   |- src/                      # React + TypeScript interface
   `- src-tauri/
      `- src/                   # Tauri commands, SQLite, discovery, process manager
```

The React frontend has no direct filesystem, database, port-scanning, or process privileges. Typed Tauri commands and events connect it to the Rust backend, which owns those operations.

The desktop database is created in RunCove's application-local data directory and migrated by schema version. It stores projects, launch profiles, expected ports, trusted port associations, run sessions, restore order, and application settings. It never opens or modifies a project's own database.

Port ownership follows a deliberate trust order:

1. A process tree launched and managed by RunCove
2. An association explicitly confirmed by the user
3. An untrusted suggestion inferred from process information

Only managed or user-confirmed associations are persisted. Raw polling snapshots are not retained.

## Privacy And Process Safety

- RunCove operates locally and does not upload project, process, port, or log data.
- Session output stays in a bounded memory buffer by default and is not written to disk.
- Project discovery reads package metadata; it does not read or edit `.env` files.
- RunCove does not request administrator privileges automatically. The top-bar shield can explicitly restart the app through Windows UAC for enhanced process visibility; this administrator instance is monitor-only and disables project launch, stop, restart, restore, browser, folder, and external-termination actions. Restricted fields still degrade to unavailable when even the administrator token cannot read them.
- Before the desktop app terminates an external process tree, the UI requires confirmation and the backend rechecks the PID, process start time, executable path, and managed-process ownership. A changed or unverifiable identity is rejected.
- The title-bar close button asks whether to hide or quit until an optional choice is remembered. Every quit path records the restore set and stops each RunCove-managed process tree before exit.
- Launch profiles avoid interpolated shell command strings: the executable, argument array, and working directory are persisted separately.

The legacy CLI's forced kill mode skips only the interactive prompt. It still rechecks port ownership and verifies the PID, process start time, and executable path before terminating the process tree. Use `runcove kill <PORT> --force` only when non-interactive termination is actually required.

## Development Checks

Run Rust checks from the repository root:

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Run desktop frontend and Tauri checks from `apps/desktop`:

```powershell
npm run lint
npm run typecheck
npm test -- --run
npm run build
npm run e2e
npm run tauri build
```

The Playwright flow uses the installed Microsoft Edge channel and covers the
three primary views, logs, project import, browser console errors, and viewport
overflow at `900x600`, `1280x720`, and `1440x900`.

Generated `target/`, `dist/`, `node_modules/`, runtime database, and captured-log data are not source artifacts and should not be committed.

Project discovery imports a selected npm or pnpm package/workspace root or performs a bounded best-effort recursive scan of a development root. The last successful development root is saved in RunCove's settings and scanned once on the next startup; findings remain review candidates and are never registered automatically. Unreadable or excessively deep unrelated subtrees are skipped without discarding valid projects already found. Workspace patterns are read from `package.json` and the common block-list or inline-list forms of `pnpm-workspace.yaml`.

## v0.2.1 Scope

RunCove v0.2.1 focuses on reliable Windows port and process lifecycle management. It intentionally does not include:

- Start at login or automatic project startup
- Docker or remote-host management
- Device previews
- Git status integration
- Environment-variable or `.env` editing
- Persistent log archives
- Usage-time analytics
- Published installers or package-manager distribution

## License

[MIT](LICENSE) - Copyright (c) 2026 AbyssWhalen and RunCove contributors
