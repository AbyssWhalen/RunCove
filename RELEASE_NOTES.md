# RunCove v0.2.0

RunCove turns the original port-only repository into a Windows-first local
development runtime center while preserving the `portpeek` command as a
compatible CLI entry point.

## Highlights

- Monitor active TCP and UDP listeners with PID, process, executable, command,
  binding, and trusted project association details when Windows permits them.
- Discover npm and pnpm projects, review structured launch profiles, and keep
  expected ports together with each project.
- Start, stop, and restart managed process trees, inspect bounded in-memory
  logs, detect conflicts, and restore the profiles that were running before the
  previous explicit exit.
- Use the native title-bar close choice and system tray without losing managed
  service state. Launching RunCove again now restores an existing hidden window.
- Use the built-in English and Simplified Chinese help guide for the main
  workflows and safety boundaries.

## Security Boundaries

- Administrator monitoring is deliberately read-only. It improves process
  visibility but disables project launch, stop, restart, restore, browser,
  folder, and external-termination actions so user-controlled commands never
  execute with administrator rights.
- External process termination revalidates the listener, PID, process start
  time, executable path, and managed ownership immediately before using the
  Windows system `taskkill.exe`.
- Project commands remain structured as `program`, `args[]`, and `cwd`; logs are
  bounded in memory and are not persisted by default.

## Downloads

- `runcove-desktop-windows-x86_64-portable.zip` contains the Windows desktop
  application.
- `runcove-cli-*` archives contain both the new `runcove` CLI and the legacy
  `portpeek` compatibility command.
- `SHA256SUMS.txt` contains checksums for every binary archive.

The Windows desktop executable is portable and unsigned. Windows SmartScreen
may show an unknown-publisher warning. Verify the archive against
`SHA256SUMS.txt` and download it only from this GitHub release. Microsoft Edge
WebView2 Runtime is required and is included with current Windows 11 systems.

The historical `v0.1.0` tag and release remain unchanged.
