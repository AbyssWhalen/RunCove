# RunCove Implementation Notes

## Approved Product Decisions

- Windows 11 is the first desktop target; the scanning core and CLI retain their
  existing cross-platform behavior.
- The desktop stack is Tauri 2 with React, TypeScript, and Vite.
- The app does not start with Windows. Projects never start automatically.
- The previous run set is restored only through an explicit user action.
- Current-session logs are bounded in memory and are not persisted by default.
- Explicit application exit stops all RunCove-managed process trees after
  confirmation.
- External process-tree termination requires current identity verification and
  explicit confirmation. RunCove never elevates automatically; enhanced
  monitoring requires a separate explanatory confirmation and Windows UAC.
- Desktop IPC uses lowercase `tcp`/`udp` protocol values and epoch-millisecond
  timestamps. Port state refreshes through the typed `port-snapshot` event.
- Project import uses the native directory picker. Inferred associations remain
  suggestions until the user confirms them.
- Development-root discovery uses bounded recursive scanning, skips dependency
  and build directories, and presents a selectable batch before persistence.
- Profile lifecycle commands use atomic reservations. Process watchers verify
  session identity, drain both output pipes before completing a session, and
  continuously synchronize the ordered restore set.

## Persistence Boundary

RunCove creates its own versioned SQLite database in the application-local data
directory. It does not inspect or modify databases belonging to registered
projects.

## Verification Log

- Unchanged baseline formatting and Clippy checks passed after dependencies were
  available. The unchanged full test suite timed out because Windows process
  names were resolved by spawning `tasklist` for every scanned entry; the
  fixed-port kill integration test was also unsuitable for a shared machine.
- Root CLI verification after the scanner and compatibility work:
  - `cargo fmt --all -- --check`: passed.
  - `cargo clippy --offline --all-targets --all-features -- -D warnings`:
    passed with warnings denied.
  - `cargo test --offline --all-targets`: passed, 34 tests total.
  - Both `runcove --help` and `portpeek --help` executed successfully.
  - Both binaries passed the same non-destructive argument and exit-code
    compatibility matrix; a fixture test locks the legacy JSON field names.
  - A live Windows scan resolved process names for 133 of 133 entries and took
    about 64 ms on this machine.
- Final desktop/frontend verification:
  - `npm run lint`: passed.
  - `npm run typecheck`: passed.
  - `npm test -- --run`: passed, 14 tests across 5 files.
  - `npm run build`: passed.
  - Desktop Rust format and Clippy with warnings denied passed.
  - `cargo test --offline --all-targets` in `apps/desktop/src-tauri` passed 49
    tests; the environment-driven live acceptance test remains ignored by
    default.
  - The ignored live acceptance test passed separately for active services on
    ports 3100, 4321, 8080, and 1420. Each launch attempt returned Conflict
    before creating a child or run session, and the listener PID, start time,
    and executable path were unchanged after the test.
  - Playwright covered overview, ports, logs, project import, and association
    confirmation at `1280x720` and `1440x900`. The final pass also covered the
    development-root selection list at both viewports with zero console errors.
  - `npm run tauri build`: passed with the final source in the isolated target
    `C:\Users\93137\AppData\Local\Temp\runcove-tauri-final-20260807-212342`.
    The final executable is `release\runcove-desktop.exe`, size `24,740,334`
    bytes, SHA-256
    `F135EDE741512E112E1F01FC0519E5278FEB7510C56E1C968053F96974314423`.
- Idle performance checks:
  - The earlier pre-dedup main-process baseline ran for 602.3 seconds at
    0.4185% average CPU; memory decreased from 92.93 MB to 36.45 MB.
  - The earlier pre-dedup whole-tree baseline ran for 656.3 seconds at 0.3533%
    average CPU; memory decreased from 470.99 MB to 176.04 MB.
  - The final-source executable ran idle for 604.924 seconds with 122 samples
    across a stable seven-process tree. CPU, normalized across 28 logical
    processors, averaged 0.977930% (minimum 0.430837%, maximum 1.946985%),
    passing the under-1% target with 0.022070 percentage points of headroom.
  - Final-source private memory was 321.566 MiB at the first sample and 323.027
    MiB at the last (296.891-337.750 MiB range); the second-half mean exceeded
    the first by 1.346 MiB and downward steps were more common than upward
    steps. This is not evidence of a sustained or monotonic private-memory leak.
  - Final-source working set moved from 165.895 MiB to 196.934 MiB
    (165.320-210.996 MiB range), with a +11.400 MiB second-half mean and
    +2.340186 MiB/min linear slope. This looks like warm-up/residency growth,
    but it prevents claiming memory was fully flat; a longer soak remains a
    residual performance check.
  - A second isolated final-source QA run sampled the complete process tree 114
    times over 597.328 seconds. The process count stayed at seven and normalized
    CPU averaged 0.346124% (0.052815-0.867248%), comfortably passing the target.
    Working set fell from 543.973 MiB to 461.965 MiB. Private memory moved from
    289.645 MiB to 301.973 MiB, ranged from 277.625 MiB to 330.863 MiB, and had
    a +4.409032 MiB/minute fitted slope. The samples do not show unbounded
    process-count or working-set growth, but private memory is not claimed flat;
    a longer soak remains appropriate before public release.
- Image generation was unavailable with HTTP 403. The checked-in deterministic
  `source.svg` and generated Tauri icon set are the local fallback.
- Runtime data was observed at
  `C:\Users\93137\AppData\Local\com.abysswhale.runcove\runcove.sqlite3`, outside
  the repository.
- Final root Rust checks passed: format, Clippy with warnings denied, and 34
  tests including real TCP/UDP listeners and legacy CLI compatibility.
- Final executable smoke after the endpoint fix launched PID 84412 from the
  isolated release target. The real window reported `0 running`, `0 conflicts`,
  74 active ports, and no configured profiles. A same-source CLI scan returned
  124 endpoint rows, zero exact duplicate groups, and retained the original
  listener PIDs for ports 3100, 4321, 8080, and 1420.
- Computer Use successfully selected and read the unique final RunCove window,
  but its first view-switch click returned `node_repl exec context not found`.
  The same error recurred on a read-only recovery capture, so no further input
  or blind-coordinate fallback was attempted. The equivalent navigation,
  search, detail, and refresh flows remain covered by the completed Playwright
  browser QA rather than being claimed as final executable input automation.

## 2026-08-08 Responsive, Language, And UX Optimization

- Added typed English and Simplified Chinese dictionaries with
  `system / en / zh-CN` selection. WebView storage provides the immediate
  preference while the existing version-1 settings JSON remains the durable
  source; no SQLite schema migration was introduced.
- The native tray menu, tooltip, and running/conflict/unexpected-exit summary
  update in the selected language. System mode now follows the primary locale
  consistently in the WebView and Windows backend.
- Invalid, blank, or missing WebView language values no longer overwrite a
  valid backend preference. A successful database save is the language commit
  point; tray refresh failures are reported separately and the next status
  refresh retries the complete tray text without rolling the UI back.
- Removed the root 980-pixel minimum width and set the Tauri minimum window to
  `900x600`. Compact tables retain the high-value columns and expose all hidden
  data in the port detail row. Ports keep compact columns through the last
  width that cannot fit all eight columns.
- Expanded port details now switch their table `colSpan` between four and eight
  as the compact media query changes. This prevents hidden columns from
  distorting the visible row widths after a detail row opens.
- Fixed top/right tooltip placement, including the log drawer and project-card
  actions; modal actions stay visible, and dialogs trap focus, close on Escape
  when idle, restore trigger focus, and cannot be dismissed while busy.
- Operation errors are no longer cleared by the two-second port refresh. Log
  loading, subscription, clipboard, and clear failures leave recoverable state
  and localized user-facing conclusions while retaining raw technical detail.
- Browser actions are enabled only for running profiles with an active trusted
  TCP association matching that profile.
- Frontend verification passed: lint, typecheck, 31 tests across 10 files, and
  the production build. Root Rust format/Clippy/tests passed with 34 tests;
  desktop Rust format/Clippy/tests passed with 57 tests and one explicitly
  configured live acceptance test ignored.
- Playwright verified English and Chinese at `900x600`, `964x601`, `1024x640`,
  `1120x700`, `1280x760`, and `1440x900`, plus `1121`, `1198`, `1199`, and
  `1200` boundary widths. Document, content, and table client/scroll widths
  matched; Actions stayed visible; detail rows, import modal, log drawer, and
  hovered right-edge tooltips stayed in bounds; the console had zero errors and
  zero warnings. Screenshots are under `apps/desktop/output/playwright/`.
- The final isolated Tauri build passed. Executable:
  `C:\Users\93137\AppData\Local\Temp\runcove-tauri-i18n-20260808-0047\release\runcove-desktop.exe`;
  size `24,797,285` bytes; SHA-256
  `57DCF54FE1663D6EF6814AFB06C99DF0AE29C1D55F12B08FC14864B41502E550`.
- PID 84412 still runs the previously accepted executable. It was not stopped,
  and the new release was not launched. Read-only post-build scanning confirmed
  the original listeners on 3100/PID 68036, 4321/PID 66584, 8080/PID 105280,
  and 1420/PID 86304 remained present.
- After the second isolated soak, QA PID 133636 and its WebView2 descendants
  were identity-checked and stopped. PID 84412 plus listeners 1420/PID 86304,
  3100/PID 68036, and 4321/PID 66584 were still present. The previously observed
  external 8080/PID 105280 process and listener were no longer present; RunCove
  did not terminate or restart them.
- A QA-flavored build used the inline identifier
  `com.abysswhale.runcove.qa`, started as PID 116720, created
  `C:\Users\93137\AppData\Local\com.abysswhale.runcove.qa\runcove.sqlite3`,
  and initialized a six-process WebView2 descendant tree without any Node/npm
  project process. The QA process identity was rechecked before it was stopped;
  its executable and isolated database remain recoverable in the temp and app
  data directories. Computer Use could uniquely enumerate and bind the window,
  but both launch/capture paths returned `node_repl exec context not found`.
  OS handle capture exposed only cloaked helper surfaces, so no blind input or
  full-desktop screenshot was used.

## 2026-08-10 Lifecycle And UI Hardening

- Process output uses a streaming 16 KiB per-line bound. Oversized lines are
  drained through the following newline and marked as truncated; ordinary
  non-newline tails remain visible.
- Process exit commits keep the managed entry visible while persisting status,
  session, restore-set, log, and event state. PID and session generation checks
  prevent a stale watcher from touching a restarted profile.
- CLI termination considers only listening endpoints and rechecks PID identity.
  Established client connections cannot become kill candidates.
- Project suggestions normalize extended drive and UNC paths before boundary
  comparisons. Matching inactive expected ports and association history merge
  for the same owner while different-owner history remains separate.
- The log drawer registers its event listener before requesting history and
  merges overlap by occurrence counts. Real duplicate lines are preserved and
  delayed listener creation is cleaned up after unmount.
- Conflict, unknown, and explicitly unexpected status events use the error
  notification channel. The event contract carries a backward-compatible
  `unexpected` flag across Rust and TypeScript.
- Editing a stopped project clears cached runtime state for only profiles
  removed by that edit, under the old profile lifecycle reservations.
- Tray language/status snapshots are copied under the mutex; native menu and
  tooltip APIs run after release to avoid lock-order stalls.
- Red-to-green evidence covers failure-event UI, removed-profile state, tray
  mutex release, log subscription ordering, watcher generation, oversized
  output, listener-only CLI selection, path normalization, and port-history
  deduplication.
- Final verification: root Rust format/Clippy/37 tests; desktop Rust
  format/Clippy/86 tests plus one ignored live test; frontend lint, typecheck,
  57 Vitest tests, production build, and three Playwright flows all passed.
- Running profiles now become `Unknown` when a managed process remains alive
  but an expected listener disappears, `Conflict` when another owner takes the
  port, and `Running` again only when every expected listener is managed by the
  profile. `Starting` remains controlled by readiness logic.
- Managed/confirmed association write failures propagate through `dashboard()`.
  The background refresh loop deduplicates repeated failures and publishes them
  through the frontend lifecycle-error channel.
- The log store is globally bounded across profiles in addition to the 16 KiB
  line limit. Project discovery is async, best-effort across unrelated bad
  subtrees, and supports both block and inline pnpm workspace package lists.
- Windows scanner reports partial IPv6 failures as degraded snapshots while
  preserving IPv4 entries and disabling status reconciliation for incomplete
  data. Legacy CLI output and exit behavior are unchanged.
- Final visual QA covered `900x600`, `1280x720`, and `1440x900` across all main
  views, logs, import dialogs, long tooltips, and long toasts with no overflow,
  console errors, or warnings. The same core flow is now reproducible through
  `npm run e2e` using Microsoft Edge.
- The repeatable Edge suite now builds and serves `dist/` with `vite preview`
  instead of loading the Vite development module graph. This removed a cold
  start failure where HTML had committed while required modules were still
  pending. A retry is limited to a page that remains exactly `about:blank`, so
  HTTP, rendering, assertion, console, and page errors still fail normally.
- The `900x600` browser flow changes the durable UI selection to `zh-CN`, checks
  `<html lang="zh-CN">` and the translated launch-profile heading, verifies no
  horizontal overflow, then returns to English. All three viewports now scroll
  expanded port details into view and assert their full bounding box, matching
  the existing log-drawer and import-dialog boundary checks.
- Post-reboot verification on 2026-08-10 passed frontend lint, typecheck, all 57
  Vitest tests, a production build, and all three Microsoft Edge E2E flows.
  Vitest, Vite, and Edge were run outside the Windows process sandbox because
  in-sandbox child creation failed before execution with `spawn EPERM`.
- The same post-reboot matrix passed root Rust format, warnings-denied Clippy,
  and 37 tests, plus desktop Rust format, warnings-denied Clippy, and 86 tests;
  the one environment-configured live-service test remained explicitly ignored.
  The desktop termination fixture first hit the sandbox's expected `taskkill`
  denial, then passed with the complete suite outside that sandbox.
- Final hygiene checks report `git diff --check` clean, the Playwright run marker
  as `passed`, no listener on 14231, and no remaining RunCove Playwright/Edge
  process. `.gitignore` now excludes all generated Playwright output by default
  and re-includes only the 19 curated final QA PNGs.
- An independent final read-only audit found no reproducible P0/P1 functional
  or security issue with high confidence. Its scope included Windows scan
  degradation, external identity revalidation, Job Object process ownership,
  lifecycle reservation/generation handling, ordered restore, transactional
  SQLite persistence, settings compatibility, and the Tauri IPC capability
  boundary. It did not mutate files or run live-service tests.
- The 2026-08-10 isolated Tauri release build passed. Executable:
  `apps/desktop/src-tauri/target/final-20260810/release/runcove-desktop.exe`;
  size `25,109,671` bytes; SHA-256
  `5CE330BF09F0E4E867925EF66A346B60CE121D56D3E1EE14AC26C711C771FD0E`.

## 2026-08-10 Automatic Discovery And UAC Elevation

- The last successfully scanned development root is stored in the existing
  version-1 settings JSON with a serde default, so old settings remain valid and
  no SQLite schema migration was introduced.
- Startup scans that root once in a worker thread. Safe service-script
  candidates produce a non-blocking notice and a Projects review action; no
  project, command, or port association is saved until the user confirms it.
- Paths are compared case-insensitively on Windows with trailing separators
  removed, preventing registered projects from returning as new candidates.
- Standard monitoring remains the default. The top-bar shield opens a localized
  explanation before `ShellExecuteW` uses the Windows `runas` verb to start the
  same absolute executable with `--elevated-monitor`.
- The elevated copy validates its administrator token instead of trusting the
  command-line flag, then waits up to 15 seconds for the old instance to save
  its restore set, stop managed process trees, and release the instance mutex.
- UAC denial is distinguished from other launch failures. Administrator access
  improves process metadata coverage but does not promise access to protected
  kernel processes; the port table continues to degrade explicitly when details
  remain unavailable.
- Verification passed: frontend lint/typecheck, 70 Vitest tests, production
  build, and three Edge Playwright flows at `900x600`, `1280x720`, and
  `1440x900`; root Rust format/Clippy/37 tests; desktop Rust
  format/Clippy/91 tests with one configured live test ignored; and the isolated
  Windows Tauri release build.
- Current executable before the Help milestone:
  `apps/desktop/src-tauri/target/final-20260810-auto-elevation/release/runcove-desktop.exe`;
  size `25,249,498` bytes; SHA-256
  `E221A0077A9D2630CD549B95DFB010BF472A3761AFFC08E6BD150B1D3C9A6A1A`.
- The final temporary preview tree was identity-checked and stopped. Ports 1421,
  1422, and 14231 were free afterward, and Playwright recorded a passed run.

## 2026-08-10 In-App Help And Usage Guide

- Added a top-bar `CircleHelp` entry that opens a right-side Help drawer without
  changing the current view. The default topic follows Overview, Ports, or
  Projects; users can switch among Quick start, Ports, Projects, Access, and
  Safety.
- Each topic uses short numbered items instead of a long static manual. The
  drawer supports tablist semantics, Arrow/Home/End topic navigation, Escape,
  focus trapping, focus restoration, and direct navigation to Ports or Projects.
- Added complete English and Simplified Chinese copy for discovery, profiles,
  expected ports, status meanings, logs, restore, UAC behavior, process limits,
  ownership confirmation, and icon hover labels.
- The drawer uses an independently scrolling content region and a horizontal
  topic rail at narrow widths. A real `900x600` browser snapshot showed the
  Chinese content wrapping naturally with no clipping or horizontal overflow.
- Verification passed: `npm run lint`, `npm run typecheck`, `npm test -- --run`
  with 70 tests, `npm run build`, and three Edge Playwright flows covering the
  Help drawer at `900x600`, `1280x720`, and `1440x900`.
- The latest release executable is:
  `apps/desktop/src-tauri/target/final-20260810-help/release/runcove-desktop.exe`;
  size `25,254,860` bytes; SHA-256
  `E0CFA78FBE99F9441835F0030C016AB1E6FB776D0502CC950654811156A05AFA`.

## 2026-08-10 Native Window Close Behavior

- Removed the top-bar hide and quit controls because they duplicated Windows
  title-bar behavior and the old frontend hide path did not provide a reliable
  native result.
- The native close request now prevents immediate destruction and resolves one
  persisted `ask | hideToTray | quit` policy. `ask` focuses the main window and
  emits the typed close-choice event; `hideToTray` uses a Rust IPC/native hide;
  `quit` completes the existing restore-set and managed-process shutdown before
  allowing application exit. Failures keep the window available and surface a
  lifecycle error.
- The default remains `ask`. Remembering is opt-in and saved in the existing
  version-1 settings JSON with serde backward compatibility, so no SQLite
  migration or schema change was introduced.
- The close-choice dialog defaults focus to the non-destructive tray option,
  states that quit stops all RunCove-managed services, disables every control
  while an action is pending, and leaves the window open on cancel or failure.
  Repeated native close events coalesce while the dialog is open.
- Help > Safety shows the current title-bar close behavior and resets a
  remembered choice to ask every time. Tray Exit retains its separate explicit
  confirmation and is not changed by the title-bar preference.
- Regression coverage verifies old settings compatibility, stable wire values,
  persistence without schema changes, native decision mapping, save-before-
  action ordering, save failure containment, modal preservation, native hide
  IPC use, reset behavior, accessibility, and removal of duplicate toolbar
  actions.
- Final verification passed: root Rust format/Clippy/37 tests; desktop Rust
  format/Clippy/96 tests with one configured live acceptance test ignored;
  frontend lint/typecheck/82 Vitest tests/production build; and three Edge
  Playwright flows at `900x600`, `1280x720`, and `1440x900`.
- Browser visual QA confirmed the Chinese close dialog and Help reset control at
  `900x600`; screenshots are under
  `apps/desktop/output/playwright/qa-20260810-close/`. This proves the WebView
  UI, not a physical click on the native Windows title bar.
- The isolated release is
  `apps/desktop/src-tauri/target/final-20260810-shutdown-race/release/runcove-desktop.exe`;
  size `25,314,336` bytes; SHA-256
  `4EFFB8F27994B74D75DDBEC6706DC73EDE7A993313B6F66AA3CB02C0ABC5CE20`.
- The older Help build was gone after the post-reboot check, so no RunCove
  executable was stopped or replaced. The newest release's real title-bar X
  and tray recovery remain a manual smoke check. Preview/browser QA was closed
  and ports 1421, 1422, and 14231 were free.

## Unresolved Issues

- Public trademark, domain, crates.io, and npm name clearance for `RunCove` is
  outside this local implementation and remains pending before publication.
- A native smoke test of the newest 2026-08-10 executable remains optional. An
  earlier isolated QA flavor proved native process, database, and WebView
  initialization, while browser-mode QA covers the final UI source. The prior
  Help-build instance was gone after the post-reboot check; no RunCove
  executable was replaced. The ask/hide/quit title-bar paths should still be
  manually exercised in an explicitly authorized smoke instance.
- The real Windows UAC cancel and successful relaunch paths were not invoked in
  unattended validation because they require desktop interaction and would
  close the current RunCove instance. Token checks, denial mapping, instance
  handoff, IPC, and UI confirmation are covered deterministically.
- The environment-driven live acceptance checks that each target port is
  present but does not compare the complete observed port set. The deterministic
  Astro/workerd unit fixture provides the exact auxiliary-port exclusion check;
  the live assertion can be tightened later if its case schema is expanded.
- The final-source ten-minute CPU result passed the under-1% target by only
  0.022070 percentage points, and working set increased during the observation
  window. No sustained private-memory growth was observed, but a longer idle
  soak is advisable before treating performance headroom as settled.

## Real Runtime Acceptance Hardening

- Read-only inventory on 2026-08-07 confirmed external development listeners
  on ports 3100, 4321, 8080, and 1420. These processes are acceptance inputs
  only and must not be stopped or adopted without separate approval.
- The first real-data audit found that importing every `package.json` script
  would expose deployment, database, and port-killing commands as one-click
  actions. Automatic discovery now selects only exact `dev`, `start`, `serve`,
  and `preview` scripts. Projects with no safe service candidate remain
  available for manual configuration but are excluded from batch import.
- Confirmed port ownership must be treated as historical when the current
  process points to a different registered project. The active suggestion is
  shown separately, and confirming a new owner replaces other confirmed owners
  for the same port and protocol without discarding the original first-seen
  timestamp when reconfirming the same owner.
- Lifecycle hardening keeps manual starts in `Starting` until every expected
  port is owned by the managed process tree. User stops return to `Idle`, normal
  exits remain non-alerting `Exited`, and only unexpected exits contribute to
  the tray exit count.
- The ordered restore set is updated after start, stop, and watcher-observed
  exit. Explicit shutdown freezes watcher synchronization, persists the
  pre-stop order, and is idempotent when `app.exit` raises a second exit event.
- Tray stop-all emits `tray-stop-all-error` with `{ action, message, timestamp }`
  and reveals the main window on failure; successful stop-all persists an empty
  restore set.
- A Tauri mock-app npm fixture now covers stdout, stderr, non-newline tail
  fragments, exit code 7, SQLite session completion, abnormal classification,
  restore-set removal, and port release without touching the production DB.
- Runtime import only persists wrapper arguments after filtering sensitive flag
  names, including camelCase credentials and opaque payload carriers such as
  `--header` and `--define`.
- Astro exposed the intended `4321/tcp` listener plus auxiliary listeners on
  62853 and 62859 under the same `npm run dev` tree. Runtime observation now
  reads exact descendant `--port` evidence solely to filter those listeners;
  descendant argv is not copied into the saved launch profile. A red-to-green
  fixture preserves this behavior while the existing ambiguous-listener rule
  remains conservative.
- External termination fixtures reject mismatched start time, executable path,
  and RunCove-managed PIDs. A verified self-created Node parent/child tree is
  terminated and its port released without touching unrelated processes.
- Live-import failure diagnostics report PID, cwd, and executable file name but
  never print the full process argv, which may contain credentials or headers.
- Real Windows UI inspection exposed repeated 5353/mDNS rows for Chrome and
  Termius. Windows legitimately reported multiple reusable UDP sockets with the
  same observable endpoint, but RunCove could neither distinguish nor operate
  on individual socket handles. Scanner-level identity now uses
  `port + protocol + state + PID + bind address`; an interleaved `[A, B, A]`
  red-to-green test proves exact duplicates collapse while distinct PID,
  protocol, state, and IPv4/IPv6 bindings remain. The live scan dropped from
  136 rows to 122 at that instant and reported zero exact duplicate groups.

## Historical Delivery Inventory (2026-08-08)

- Final `git status --short --untracked-files=all` reports 137 changed paths:
  15 tracked modifications and 122 new files. The tracked modifications are
  `.gitignore`, `Cargo.lock`, `Cargo.toml`, `README.md`, `src/cli.rs`,
  `src/lib.rs`, `src/main.rs`, `src/model.rs`, `src/process/mod.rs`,
  `src/render/json.rs`, `src/render/watch.rs`, `src/scanner/mod.rs`,
  `src/scanner/windows.rs`, `tests/cli_tests.rs`, and
  `tests/scanner_tests.rs`.
- New root paths are `AGENTS.md`, `HANDOFF.md`, `notes.md`,
  `src/bin/portpeek.rs`, and `src/cli_app.rs`. The remaining 117 new paths are
  under `apps/desktop`, including the complete frontend/backend application, 53
  generated Tauri icon assets, and 17 Playwright QA screenshots.
- Root changes cover crate naming, shared `runcove`/`portpeek` CLI entrypoints,
  Windows scanning and verified process-tree termination, plus regression tests.
- `apps/desktop` contains the React client, typed IPC adapter, deterministic
  browser mock, Tauri backend, SQLite migrations, generated icon set, and QA
  screenshots.
- `git diff --check` passed. Build outputs, Playwright session files, logs, PID
  files, dependencies, and runtime data are ignored rather than source files.
- A browser preview was served during the 2026-08-08 evaluation. The final
  2026-08-10 QA preview on port 1421 was identity-checked and stopped; no QA
  preview was intentionally left running.
- No commit, push, remote rename, CI/release edit, installer publication, or
  package publication was performed.

## 2026-08-10 Shutdown Race Hardening

- Native `CloseBehavior::Quit` now runs `shutdown` through Tauri's async runtime
  and `spawn_blocking`, so the close event thread does not wait up to the
  eight-second managed-process cleanup limit. Successful cleanup calls
  `app.exit(0)`; worker or cleanup failures restore the window and emit the
  existing `shutdown-error` event.
- `ProcessManager::shutdown_is_in_progress()` gates native `CloseRequested`
  handling. A second title-bar X during an active shutdown is prevented and
  ignored instead of starting a competing hide/quit action.
- React now shares `shutdownInFlight` between tray quit and title-bar quit. New
  tray or native close requests are ignored while the promise is pending, and
  the ref is released in `finally` on success or failure. Two App tests cover
  the tray and title-bar pending-shutdown races.
- Verification after reboot:
  - Root `cargo fmt --all -- --check`, Clippy with warnings denied, and
    `cargo test --offline --all-targets`: `37/37` passed.
  - Desktop Rust format, Clippy, and `cargo test --offline --all-targets`:
    `96 passed / 1 ignored`.
  - Frontend lint, typecheck, `npm test -- --run`: `82/82` passed, and
    `npm run build`: passed.
  - `npm run e2e`: `3/3` Playwright flows passed at `900x600`, `1280x720`, and
    `1440x900`.
  - `npm run tauri build` passed with the isolated release at
    `apps/desktop/src-tauri/target/final-20260810-shutdown-race/release/runcove-desktop.exe`;
    size `25,314,336` bytes; SHA-256
    `4EFFB8F27994B74D75DDBEC6706DC73EDE7A993313B6F66AA3CB02C0ABC5CE20`.

## 2026-08-11 Restore And Project Removal Clarity

- Replaced implementation-oriented restore-set copy across Overview, Help,
  elevation guidance, tests, mock data, and the native tray. Users now see
  `Previously running profiles`, `Restore previous run`, and `Startup order`,
  localized as `上次运行的配置`, `恢复上次运行`, and `启动顺序`.
- The restore action still starts only the profiles that were running before
  RunCove last exited, in their saved order. It does not import projects,
  restore every registered profile, or enable Windows startup automation.
- Added a visible trash action to every project card. It is disabled when any
  profile is starting, running, has a PID, or has a lifecycle operation in
  flight. The confirmation dialog states that RunCove registration, launch
  profiles, and port associations are removed without deleting project files
  from the computer.
- Removed the editor-only deletion path so there is one confirmation flow.
  Ref-level in-flight protection prevents rapid duplicate submissions; failure
  leaves the dialog and project visible and surfaces the backend detail.
- Added focused frontend coverage for successful deletion, deletion failure,
  and repeated confirmation. The frontend suite now passes `85/85` tests.
- Final verification passed:
  - Root `cargo fmt --all -- --check`, Clippy with warnings denied, and
    `cargo test --offline --all-targets`: `37/37` passed.
  - Desktop Rust format, Clippy, and `cargo test --offline --all-targets`:
    `96 passed / 1 ignored`.
  - Frontend lint, typecheck, `85/85` Vitest tests, and production build passed.
  - Playwright passed `3/3` workflows at `900x600`, `1280x720`, and
    `1440x900`, including disabled and active project deletion plus cancel.
- In-app browser QA at `900x600` produced
  `900x600-restore-previous-run.png`, `900x600-project-delete-actions.png`,
  and `900x600-project-delete-confirm.png` under
  `apps/desktop/output/playwright/qa-20260811-understandability/`. Document and
  viewport widths were both 900 pixels; the confirmation dialog stayed inside
  the viewport and the browser console had no errors or warnings.
- The verified preview was PID 27960, the project's Vite preview on port 1424.
  Its identity was rechecked before stopping it, and port 1424 was released.
- `npm run tauri build` passed with `CARGO_BUILD_JOBS=2` and the isolated
  release at
  `apps/desktop/src-tauri/target/final-20260811-understandability/release/runcove-desktop.exe`;
  size `25,314,504` bytes; SHA-256
  `80A5729A1D58531BCC005B0CBB1F6216C36B9D2908DC260150C5ED4A69664CEC`.
- The release was built but not launched, installed, committed, pushed, or
  published. No unrelated development service was stopped or modified.

## 2026-08-11 v0.2.0 Release Candidate

- Added a named Windows wake event beside the single-instance mutex. A normal
  duplicate launch signals the existing process, whose listener shows,
  unminimizes, and focuses the main window. `acquire_after_previous` suppresses
  that signal so administrator handoff does not flash the old window while
  waiting for shutdown. Two Windows regression tests cover wake and no-wake.
- A native final-executable smoke proved the real behavior: title-bar X hid PID
  33824, the window disappeared from the targetable window list, relaunching the
  same path restored the same window, and exactly one `runcove-desktop.exe`
  process remained. The UI reported live local ports with zero managed runs or
  conflicts; port 14231 was free after Playwright.
- Administrator instances are monitor-only. Typed backend guards and UI/tray
  disabled states cover project start/stop/restart, restore, browser/folder
  opening, tray stop-all, and external termination. This closes the path where
  a user-writable launch profile could otherwise execute with elevated rights.
- Desktop and CLI Windows termination use the absolute System32 `taskkill.exe`.
  The helper is covered by a path test and external identity tests still
  recheck listener, PID, start time, executable, and managed ownership.
- Upgraded Vitest to 4.1.10 and Vite resolved to 6.4.3. The lockfile uses only
  `registry.npmjs.org`; complete and production-only `npm audit` both report
  zero vulnerabilities. A stricter Vitest mock type exposed and fixed one
  test-only `LaunchProfile` callback annotation.
- Version `0.2.0` is synchronized across root Cargo, desktop Cargo, npm, Tauri,
  CLI version output, and the WebView child `--webview-exe-version` argument.
- Final verification after all code and dependency changes:
  - Root format passed, warnings-denied Clippy passed, `38/38` tests passed, and
    both CLI binaries report `runcove 0.2.0`.
  - Desktop Rust format passed, warnings-denied Clippy passed, `98 passed / 1
    ignored`; the ignored test requires explicitly configured external local
    services.
  - Frontend lint, typecheck, `85/85` Vitest tests, production build, and all
    three Edge Playwright workflows passed.
  - `git diff --check` passed and no listener remained on Playwright port 14231.
- Final isolated executable:
  `apps/desktop/src-tauri/target/final-20260811-v0.2.0/release/runcove-desktop.exe`;
  size `25,348,756` bytes; SHA-256
  `C80B1B6601ED42A34F333773A3C5997FDDFBA74E7B81610F490479F931192A13`.
- Local packaging rehearsal:
  - Desktop portable zip contains `runcove-desktop.exe`, `README.md`,
    `CHANGELOG.md`, and `LICENSE`; SHA-256
    `E49A9408FE2BE7A89ACCC23376956C4627E8C781682CC05C68B00860C94CEA50`.
  - Windows CLI zip contains `runcove.exe`, `portpeek.exe`, and the same three
    documents; SHA-256
    `13D11F813CE72FB9164B7C4452F925E106938C89559CAE15CACE710A4FEF141C`.
- CI/release workflows now cover the desktop and new artifact names. GitHub is
  the authoritative workflow parser because no local YAML checker is installed;
  publication remains incomplete until PR CI, merge, tag, workflow, and asset
  download verification all pass.

## 2026-08-11 Windows Resource Linker Follow-up

- Draft PR CI run `31493311756` reached the desktop Rust test link and failed
  with `CVT1100: duplicate resource. type:VERSION` and `LNK1123`. The linker
  command contained the same generated `resource.lib` twice because
  `tauri_build::build()` linked it to binary targets and the custom build step
  also used `compile_for_everything`.
- Commit `f10c4d4` limited the custom embedding to GNU. CI run `31496735900`
  confirmed that MSVC linking no longer duplicated VERSION, but the library
  test executable then exited before the harness with
  `0xc0000139 / STATUS_ENTRYPOINT_NOT_FOUND`. The same behavior reproduced in a
  clean local GNU target when no manifest was linked to the library test.
- The final local implementation separates resource ownership instead of
  choosing between those failures:
  - Tauri uses `WindowsAttributes::new_without_app_manifest()` and remains the
    only producer of VERSION and icon resources.
  - `windows/app-manifest.rc` contains only resource type 24 and is linked to
    every executable, including unit-test harnesses.
  - Both the `.rc` and `.xml` inputs emit `cargo:rerun-if-changed`, preventing a
    manifest-only edit from being hidden by an incremental build cache.
  - `objdump` shows the Tauri archive contains types 3, 14, and 16, while the
    manifest archive contains only type 24. The final MSVC executable contains
    exactly those four top-level resource types.
- Local verification after the split:
  - Windows GNU format, warnings-denied Clippy, and full desktop tests passed:
    `98 passed / 1 ignored`.
  - A project-local Rust `1.97.1 x86_64-pc-windows-msvc` toolchain used Visual
    Studio 2022 `link.exe 14.44` and Windows SDK `10.0.26100`; the exact
    `cargo test --all-targets --no-fail-fast` command passed with `98 passed / 1
    ignored`.
  - Full GNU and MSVC `npm run tauri build` commands passed. The MSVC executable
    reports product/file version `0.2.0`, is unsigned as documented, and has
    SHA-256
    `51A29AB8313DD87D40D1442661E75A03EEA3786E35F07CE69D27E81BE2A7A9D1`.
- One full GNU suite initially exposed a timing-only npm fixture failure: the
  fixture closed its listener after two seconds while parallel scanner tests
  were active. The focused test passed, and extending the fixture lifetime to
  eight seconds made the subsequent complete GNU and MSVC suites pass without
  changing application behavior.
- No commit or push of the final fix and no new GitHub workflow run followed
  `31496735900`; PR #1 remains Draft. Live checks confirmed that the repository
  is public and both workflows use only standard hosted runner labels. GitHub's
  billing documentation states that standard hosted runner minutes are free
  and unlimited for public repositories; larger runners remain billable and
  are not used here. Artifact/cache storage remains quota-bound, so release
  artifacts keep the existing seven-day retention. References:
  <https://docs.github.com/en/actions/concepts/billing-and-usage> and
  <https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job>.
- The accidental 1.85 GB `apps/desktop/src-tauri/src-tauri/` build tree was
  moved without deletion to the ignored project-local path
  `apps/desktop/src-tauri/target/accidental-nested-build-20260811/`. No project
  source or file outside RunCove was removed or changed.
- A final post-review rerun after adding manifest change tracking passed desktop
  Rust format, warnings-denied Clippy, and the complete GNU suite (`98 passed / 1
  ignored`). The already-built MSVC library test executable was also launched
  directly and passed a filtered test, proving it no longer exits with
  `STATUS_ENTRYPOINT_NOT_FOUND`; its PE resource table contains only one type,
  `RT_MANIFEST` (24). No commit, push, or new GitHub workflow was triggered.

## 2026-08-12 Elevated Runner Test Isolation

- Commit `c281340` was pushed once to Draft PR #1. CI run `31501585406` passed
  all three CLI jobs, root Rust lint, frontend audit/lint/typecheck/tests/build,
  Edge browser workflows, desktop formatting, and desktop Clippy. The MSVC test
  harness started normally and ran 99 tests, proving the manifest resource fix.
- The single failure was
  `tests::successful_tray_stop_all_synchronizes_an_empty_restore_set`: GitHub's
  Windows runner uses an elevated token, so the real production guard correctly
  returned `Administrator monitoring mode is read-only` and the test's
  unconditional `unwrap()` failed. This was a machine-dependent test defect,
  not a product failure or resource-link regression.
- The private `stop_all_from_tray` helper now accepts a permission-check closure.
  Its only production call passes `privileges::ensure_process_action_allowed`,
  and the guard remains the first operation before process reservation or
  storage mutation. Tests inject deterministic allow/deny results; the denied
  path verifies the error and confirms that the saved restore set is unchanged.
- Independent review found no production security regression. Post-fix desktop
  format, warnings-denied Clippy, and all targets passed locally: `99 passed / 1
  ignored`. No CI or release workflow file was changed.
