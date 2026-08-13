# Changelog

All notable user-facing changes to RunCove are documented in this file.

## [0.2.1] - 2026-08-13

### Added

- A five-entry Recent Runs section on Overview and a searchable, filterable
  history drawer for up to 200 stored run sessions. Deleted-project history is
  retained, while profiles that still exist can be located in Projects.
- Actionable expected-port conflict feedback. `View occupant` refreshes the
  snapshot and focuses the exact TCP or UDP listener without automatically
  terminating it; a listener that has disappeared is reported as changed.
- Launch-profile copying with new identities and no copied runtime-observation
  metadata.
- Field-level project validation for required project and profile data, blank
  arguments, valid port ranges, and duplicate protocol/port pairs, mirrored by
  authoritative backend validation.
- Copy actions for PID, executable path, and command line in port details, with
  explicit clipboard failure feedback.
- English and Simplified Chinese help for run history, conflict recovery,
  restore failures, and the non-persistent log boundary.

### Changed

- Saved development-root scans now show scanning, candidates-found, empty, and
  failure states with retry. Concurrent requests are coalesced, review
  candidates survive closing the review window, and successful partial imports
  remove only the imported candidates.
- Lifecycle, restore, and discovery errors include the affected project,
  profile, or port context when available.
- Run history refreshes after relevant lifecycle actions and exit events rather
  than on every two-second port poll. Unknown persisted statuses degrade safely
  to `Unknown`.

### Data And Safety Boundaries

- No SQLite migration is introduced. Existing run-session metadata is reused;
  polling snapshots and console logs are not persisted.
- Conflict handling continues to require confirmation and process-identity
  revalidation before an external process tree can be terminated.

## [0.2.0] - 2026-08-12

RunCove replaces the original port-only application in this repository while
preserving the original v0.1.0 CLI release and tag.

### Added

- Windows desktop control center with Overview, Ports, and Projects views.
- Two-second TCP and UDP listener monitoring with available process metadata.
- npm and pnpm project discovery, reviewed import, structured launch profiles,
  expected ports, and remembered development-root rescans.
- Managed start, stop, restart, bounded in-memory logs, and ordered restoration
  of the profiles that were running before the previous explicit exit.
- System-tray operation, localized English and Simplified Chinese help, and an
  optional UAC restart for improved process visibility.

### Security And Reliability

- Managed Windows process trees are placed in Job Objects for bounded cleanup.
- External termination revalidates PID, start time, executable path, and
  managed ownership after user confirmation.
- Port associations are persisted only for managed processes or after explicit
  user confirmation; inferred ownership remains a suggestion.
- Project commands are stored as program, argument array, and working directory
  rather than as interpolated shell strings.
- Session logs remain in a bounded memory buffer and are not persisted by
  default; project discovery does not read or edit `.env` files.
- Administrator monitoring is read-only and disables every action that could
  launch or terminate a user-controlled process with elevated rights.
- Windows system process termination resolves `taskkill.exe` from the trusted
  system directory instead of the application or current directory.
- Starting RunCove while it is hidden in the tray wakes and focuses the existing
  single instance instead of silently exiting.
- Partial IPv6 scan failures retain usable IPv4 results and mark the snapshot
  as degraded instead of treating it as complete.

### Compatibility

- `runcove` is the primary CLI command.
- A compatibility CLI entry point remains available with the original flags,
  JSON fields, and exit-code behavior.
- The canonical repository URL is `https://github.com/AbyssWhalen/RunCove`.

### Known Limitations

- The desktop application is Windows-first; the port-inspection CLI remains
  cross-platform.
- The public artifact is an unsigned Windows x64 portable zip. There is no
  installer, code-signing certificate, or package-manager distribution yet.
- Microsoft Edge WebView2 Runtime is required for the desktop interface.
- Automatic start at Windows login, unattended project startup, Docker and
  remote-host management, persistent log archives, and `.env` editing are not
  included.
- Elevated mode can improve process visibility but cannot guarantee access to
  every protected system process.

## [0.1.0]

- Historical pre-RunCove CLI release. Its Git tag and GitHub release are retained
  unchanged for existing users and scripts.
