# RunCove v0.2.1

RunCove v0.2.1 makes the Windows local-development runtime center easier to
understand and diagnose. The distribution includes the Windows desktop app,
the cross-platform `runcove` CLI, and a compatibility executable for existing
scripts.

## Highlights

- Review the five most recent managed sessions on Overview, or search and
  filter up to 200 stored run-history records in the history drawer.
- Jump from an expected-port conflict to the exact TCP or UDP listener after a
  fresh snapshot check, without automatically terminating another process.
- See explicit saved-root discovery states, retry failed scans, and keep
  candidates available until they are reviewed or successfully imported.
- Copy launch profiles, validate project fields before saving, and copy PID,
  executable-path, or command-line details directly from the Ports view.
- Use expanded English and Simplified Chinese help for run history, conflicts,
  restore failures, project discovery, and the non-persistent log boundary.

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
- Existing SQLite data remains compatible. No schema migration is introduced,
  and raw polling snapshots are not persisted.

## Downloads

- `runcove-desktop-windows-x86_64-portable.zip` contains the Windows desktop
  application.
- `runcove-cli-*` archives contain the `runcove` CLI and a compatibility
  executable for existing scripts.
- `SHA256SUMS.txt` contains checksums for every binary archive.

The Windows desktop executable is portable and unsigned. Windows SmartScreen
may show an unknown-publisher warning. Verify the archive against
`SHA256SUMS.txt` and download it only from this GitHub release. Microsoft Edge
WebView2 Runtime is required and is included with current Windows 11 systems.

The previous `v0.2.0` release and the original `v0.1.0` tag and release remain
available unchanged.
