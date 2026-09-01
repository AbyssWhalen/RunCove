# Changelog

All notable user-facing changes to RunCove are documented in this file.

## [Unreleased]

### Fixed

- Two things that start launch profiles in order — a restore and a whole-group
  start — no longer fail each other when they overlap. They can legitimately share
  profiles: a restore set is whatever was running last, and two groups may both need
  the same database. Whichever arrived second used to fail on the shared profile with
  an "already starting" message that named no cause the user could act on. It now
  waits for that profile to settle and carries on, so the order each one promises
  still holds, and a profile the other one already brought up simply counts as
  started. Starting two groups that share a member now works.
- A restore and a whole-group start are no longer offered at the same time. Waiting
  makes overlapping them correct, but the second one would sit and wait for work
  already underway, so the button that would start it is disabled while the first
  runs and the tray's restore item ignores a repeat.
- A failed restore now names the profile it stopped at as `Project / Profile`
  instead of printing the internal profile id, which is what a whole-group failure
  has always done.
- The release workflow writes a `SHA256SUMS.txt` that `sha256sum -c` accepts, and
  checks it before publishing. See the known limitation below for what to do about
  the file already published with v0.4.0.

### Known Limitations

- `sha256sum -c SHA256SUMS.txt` fails on every line of the checksum file published
  with v0.4.0. The archives are intact and their hashes are correct: that workflow
  run wrote three spaces between the hash and the filename where the format allows
  exactly two, so `sha256sum` read the extra space as part of the name. The published
  file cannot be changed, so verify it with
  `sed -E 's/^([0-9a-f]{64})[[:space:]]+/\1  /' SHA256SUMS.txt | sha256sum -c -`.
  Releases after v0.4.0 do not need the workaround.

## [0.4.0] - 2026-08-31

### Added

- Named launch groups. A group is an ordered set of launch profiles that starts or
  stops as one unit, and you can keep as many groups as you need. Members may come
  from different projects, so one group can bring up a database, an API, and a web
  front end together.
- Whole-group start walks the members in the order you set, waiting for each one's
  expected port before moving on, exactly as a single-profile start does. A member
  that is already running counts as started, so pressing Start again only fills in
  what is missing.
- A failed whole-group start stops before the next member and keeps everything that
  already started. The message names the member it stopped at, how many started
  before it, and offers the same View occupant action as a single-profile conflict.
- Whole-group stop walks the members in reverse. A member it cannot stop does not
  stop the rest: the report counts every failure and names the first one.
- Each group shows its startup order and whether it is fully running, partly
  running, or not running. Deleting a launch profile removes it from every group
  that used it, and a group left with no members stays visible and says so.

### Fixed

- Process stop and exit messages now follow the interface language. RunCove sends a
  machine-readable reason with each run-status event and translates it in the window,
  so a Simplified Chinese interface no longer shows English sentences such as
  `Stopped by user` in the status toast or the log drawer. A reason this build does
  not recognize still falls back to the English sentence rather than to nothing.
- Fields in the project editor keep their own names once validation errors appear. A
  field whose error message sat inside its label used to take that message into its
  accessible name, so a screen reader announced `Program This field is required.`
  as the field's name and repeated the error, and the field no longer answered to
  the name shown on screen.
- Saving an existing project records the time it was saved. Every project's
  modification time previously kept showing the time it was first added.

### Known Limitations

- The desktop database migrates from schema version 2 to version 3 to store launch
  groups. The migration runs in one transaction and stays at version 2 if it fails,
  but a successful migration is a one-way step: v0.3.0 and earlier cannot open the
  resulting version 3 database. Back up RunCove's application data directory before
  running this build for the first time.
- A launch group starts and stops only when you press its button. There is no
  start-at-login and no automatic project startup, by design.
- If the final buffered flush itself fails, an archive's line count may over-report
  which buffered line reached disk. Accepted rather than fixed: the byte count is
  reconciled from the file, the row is already reported as partial with a write
  error, and nothing but the display reads the line count.

## [0.3.0] - 2026-08-21

### Added

- An opt-in run log archive, off by default, that writes one bounded JSON Lines
  file per newly started managed session without blocking the child process.
- Archive status and counters in run history, including archiving, finalizing,
  complete, partial, removed, dropped-line, and dropped-byte states.
- A tail-first paged archive viewer and confirmed archive deletion that keeps
  the corresponding run-history record.

### Reliability And Storage

- A bounded background queue records output loss explicitly instead of slowing
  a child process. Recoverable loss is represented by gap records, while every
  dropped line and byte remains visible in the archive index.
- Per-session archives are limited to 10 MiB and the archive directory to
  200 MiB. Finished archives are reclaimed oldest-first; open archives are
  never selected for eviction.
- Startup recovery repairs interrupted archive rows, accounts for files already
  on disk, removes only recognized orphan archive files, and refuses links,
  reparse points, invalid file names, and paths outside the archive directory.
- Archive reads run off the Tauri IPC thread, page backwards under both record
  and byte limits, and tolerate an incomplete final record after a crash.

### Data And Privacy Boundaries

- The desktop database migrates from schema version 1 to version 2 by adding an
  archive index. The migration is transactional, but successful migration is a
  one-way step: v0.2.1 cannot open the resulting version 2 database.
- Archiving writes only inside RunCove's application-local data directory and
  uploads nothing. Archived stdout and stderr are not filtered and may contain
  credentials, tokens, personal data, or other sensitive service output.
- Turning archiving on never backfills a running session. Turning it off or
  quitting finishes archives already open and leaves normal in-memory logging,
  port scanning, and process control available if archive initialization fails.

### Known Limitations

- If the final buffered flush itself fails, an archive's line count may
  over-report which buffered line reached disk; its byte count is reconciled
  from the file and normal writes are unaffected.
- Backend-composed process stop and exit messages remain English when the rest
  of the interface is set to Simplified Chinese.

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
