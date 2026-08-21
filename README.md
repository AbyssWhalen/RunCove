# RunCove

> Local dev services, under control.

RunCove v0.3.0 is a Windows-first desktop control center for local development services. It combines live port inspection with a trusted project registry, structured launch profiles, process-tree control, session logs, and on-demand restoration of the projects that were running before the app exited.

The distribution also includes the cross-platform `runcove` port-inspection CLI. The desktop application is Windows-first; the CLI is available on Windows, macOS, and Linux.

The v0.3.0 release is distributed as a portable Windows x64 zip from the [RunCove Releases page](https://github.com/AbyssWhalen/RunCove/releases). The original v0.1.0 CLI release remains available in the release history for existing installations.

## Current Release / 当前版本

RunCove v0.3.0 adds an opt-in run log archive while preserving the local-only
and process-safety boundaries. / RunCove v0.3.0 新增可选的运行日志归档，同时保持
本地运行与进程安全边界不变。

- **Opt-in persistence / 按需持久化:** Archiving is off by default and affects
  only runs started after you enable it. / 归档默认关闭，只影响开启后新启动的运行。
- **Bounded storage / 有界存储:** Each session is limited to 10 MiB and the
  archive directory to 200 MiB; finished archives are reclaimed oldest-first.
  / 单会话上限 10 MiB，归档目录总量上限 200 MiB，超限时优先回收最早结束的归档。
- **Visible loss / 明确记录丢失:** A bounded background queue never slows the
  child process; dropped lines and bytes are reported instead of hidden.
  / 有界后台队列不会拖慢子进程，丢弃的行数和字节数会被明确记录。
- **History access / 历史查看:** Run history shows archive status, opens a
  tail-first paged viewer, and supports confirmed deletion without removing the
  run record. / 运行历史显示归档状态，查看器默认打开尾部并支持向前分页；删除归档时仍保留运行记录。

## Download And Run

1. Download the RunCove v0.3.0 Windows x64 portable zip from the [Releases page](https://github.com/AbyssWhalen/RunCove/releases).
2. Extract the zip to a directory you control.
3. Run `runcove-desktop.exe`.

RunCove v0.3.0 is portable and does not include an installer. The executable is currently unsigned, so Windows SmartScreen may show an unknown-publisher warning; verify that the archive came from this repository's release before choosing to run it. RunCove uses the Microsoft Edge WebView2 Runtime, which is included with current Windows 11 installations and can be installed separately on older or stripped-down systems.

## Desktop App

The desktop app is the primary RunCove experience. Its compact `Overview`, `Ports`, and `Projects` views cover the normal local-development loop:

- Refresh TCP and UDP port state every two seconds and show the owning PID and process details when Windows permits access.
- Combine active listeners with registered project ports that are currently idle.
- Discover npm or pnpm launch candidates from a selected project or scan a development root for multiple projects, including `package.json` workspaces, block or inline `pnpm-workspace.yaml` package lists, and lockfile-based package-manager detection. The last successful root is rescanned once on startup; new candidates stay non-blocking and require review before registration.
- Store launch profiles as `program`, `args[]`, and `cwd`, with optional expected ports.
- Start, stop, and restart a profile; open its directory or TCP port; and detect expected-port conflicts before launch.
- Capture stdout and stderr in an in-memory session log bounded both per line and across all profiles, with filtering, copy, and clear controls. The same drawer holds the opt-in run log archive switch, which is off by default and is the only way this output reaches a file.
- Keep managed Windows process trees in Job Objects so stop and exit operations clean up child processes as well as their parent command.
- Save the active launch order on explicit exit and restore it on demand, waiting for each profile's expected ports before starting the next one.
- Use the Windows title-bar close button to choose between hiding to the system tray and safely quitting. The optional remembered choice can be reset from Help > Safety; the tray still exposes open, restore, stop-all, and confirmed exit actions.
- Open the in-app Help and usage guide from the top-bar question-mark button. It explains the first-run workflow, ports, projects, run history, the optional run log archive, conflict recovery, permissions, and safety boundaries in English or Simplified Chinese, with links back to Ports and Projects.

RunCove uses the shared status model `Idle`, `Starting`, `Running`, `Conflict`, `Exited`, and `Unknown`. Missing process metadata is reported as unavailable rather than triggering automatic elevation.

Windows IPv4 and IPv6 listeners are scanned independently. If an IPv6 table cannot be read, RunCove keeps the usable IPv4 results, marks the snapshot as degraded, and avoids changing project status from an incomplete scan.

## Run Log Archive / 运行日志归档

The run log archive is opt-in and is **off by default**. It is included in v0.3.0. Note the one-way schema step described under [Architecture](#architecture) before switching back to an older build.

The switch sits in the log drawer, next to the session it belongs to. While it is off, RunCove writes no log file at all and session output stays in the bounded memory buffer described above.

- Turning it on affects runs started from then on. A session that is already running is not backfilled, and turning it off stops new archives while archives already open are finished rather than truncated. Quitting RunCove finishes every open archive the same way, marking it partial because RunCove exited first.
- Each archived session becomes one JSON Lines file, `<session-id>.jsonl`, in a `run-log-archives` directory beside `runcove.sqlite3` in RunCove's application-local data directory. A line is `{"t":<epoch ms>,"s":"stdout|stderr|system","l":"<captured line>"}`; the session id lives in the file name rather than on every record.
- **These files can contain tokens, credentials, or personal data that your own services print to stdout or stderr.** RunCove does not filter them, and nothing is uploaded — the file is as sensitive as the output it captured.
- Limits are enforced rather than assumed: 10 MiB per session, 200 MiB across the directory. A session that reaches its own cap closes as partial with a size-limit reason; finished archives are removed oldest-first to make room for a new one and are marked as removed to free space.
- Output that arrives faster than it can be written is dropped instead of slowing the child process. Every dropped line and byte is counted in the history row, and the file also carries a gap line wherever one can still be written — on the next record that session archives, or at the end when the archive is closed normally. An archive that closes *because* writing failed or the size cap was reached gets no gap line, since a failed file cannot take one and a full file must not grow: there the row's counters are the whole report.
- Run history gains an `Archive` column: not archived, archiving, finalizing, a finished archive's line count and size, partial with a reason, or removed with a reason. A status a future build introduces is shown as unrecognized rather than hidden.
- The viewer opens at the end of the file, which is what an old run is usually opened for, and pages backwards on request until it reports the start of the archive. Pages are bounded by both record count and size, so one page is never an unbounded IPC message.
- Deleting an archive asks first, then deletes the file and keeps the run history entry, recording that you deleted it. A session whose file is still being written cannot be deleted.
- If the archive cannot start — an unwritable data directory, for example — RunCove reports it next to the switch and keeps port scanning, project launch, and in-memory logs working.

运行日志归档默认关闭，位于日志抽屉中的开关。开启后，此后启动的运行会把 stdout 与 stderr 写入 RunCove 数据目录下 `run-log-archives` 中的 JSON Lines 文件；**这些文件可能包含你的服务自己打印出的令牌或敏感信息**。单个会话上限 10 MiB，目录总量上限 200 MiB，超出时按最早的已完成归档回收；输出过快时会丢弃日志行而不拖慢子进程，丢失量一律计入历史记录，并在还能写入的位置补一条 gap 行——因写入失败或达到容量上限而关闭的归档不再补写，此时历史记录中的计数就是全部记录。关闭开关只影响新的运行，已经打开的归档会正常收尾。

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

Tauri bundling is disabled because the public v0.3.0 artifact is a Windows x64 portable zip rather than an installer. The release executable is written below `apps/desktop/src-tauri/target/` and then packaged with the release documentation.

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

The desktop database is created in RunCove's application-local data directory and migrated by schema version. It stores projects, launch profiles, expected ports, trusted port associations, run sessions, restore order, application settings, and — since v0.3.0 — one index row per archived session. It never opens or modifies a project's own database.

A build opens a database at its own schema version or older and refuses one that is newer, so this is a one-way step: after v0.3.0 has upgraded a database to schema version 2, v0.2.1 reports that version as newer than it supports and will not open it. Keep a copy of the data directory before first launching v0.3.0 if you may need to return to v0.2.1.

Port ownership follows a deliberate trust order:

1. A process tree launched and managed by RunCove
2. An association explicitly confirmed by the user
3. An untrusted suggestion inferred from process information

Only managed or user-confirmed associations are persisted. Raw polling snapshots are not retained.

## Privacy And Process Safety

- RunCove operates locally and does not upload project, process, port, or log data.
- Session output stays in a bounded memory buffer by default and is not written to disk. The opt-in run log archive is the only path that writes it to a file; it stays off until you turn it on, writes only inside RunCove's own data directory, and uploads nothing.
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

## v0.3.0 Scope

RunCove v0.3.0 adds the opt-in run log archive without expanding into unrelated integrations. It intentionally does not include:

- Start at login or automatic project startup
- Docker or remote-host management
- Device previews
- Git status integration
- Environment-variable or `.env` editing
- Usage-time analytics
- Published installers or package-manager distribution

## License

[MIT](LICENSE) - Copyright (c) 2026 AbyssWhalen and RunCove contributors
