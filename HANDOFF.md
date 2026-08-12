# RunCove Handoff

## Current Checkpoint (2026-08-12)

- Draft PR #1 remains open at remote head `c281340`. The Windows resource split
  is pushed and CI run `31501585406` proved that the MSVC library test harness
  now starts and runs all tests, so the prior resource failures are resolved.
  That run exposed one separate test isolation defect: GitHub's Windows runner
  is elevated, while `successful_tray_stop_all_synchronizes_an_empty_restore_set`
  called the real administrator monitor-only guard and incorrectly assumed it
  would be allowed. Production behavior was correct; the machine-dependent
  test failed after 97 other desktop tests passed.
- The local follow-up injects the permission check into the private tray helper.
  The sole production call still passes the real privilege guard before any
  process or storage action. Tests explicitly cover both an allowed stop-all and
  a denied monitor-only action that preserves the restore set. Independent
  review found no production security regression. Desktop format,
  warnings-denied Clippy, and the complete local suite pass with `99 passed / 1
  ignored`. This follow-up is not pushed yet.
- Earlier resource diagnosis remains relevant: run `31493311756` failed when
  the MSVC linker received Tauri's VERSION resource twice, and run
  `31496735900` exposed `STATUS_ENTRYPOINT_NOT_FOUND` when the library test
  executable no longer received an application manifest.
- Tauri now owns the VERSION and icon resource, while
  `windows/app-manifest.rc` supplies only resource type 24 to
  the application and unit-test executables. Object and final-executable
  inspection proves one manifest plus one VERSION resource, with no overlapping
  resource types. Both manifest inputs have explicit Cargo change tracking so
  incremental builds cannot silently reuse stale resources.
- The local project-only MSVC toolchain at `src-tauri/target/rustup-msvc` ran
  the same `cargo test --all-targets` command through Visual Studio `link.exe`:
  `98 passed / 1 ignored`. The GNU suite has the same result. A full MSVC Tauri
  release build passed and produced a `RunCove 0.2.0` executable at
  `apps/desktop/src-tauri/target/msvc-resource-release/release/runcove-desktop.exe`
  (`12,667,392` bytes, SHA-256
  `51A29AB8313DD87D40D1442661E75A03EEA3786E35F07CE69D27E81BE2A7A9D1`).
- The npm process fixture now keeps its listener open for eight seconds instead
  of two. This removes a reproduced full-suite timing race without changing
  product timeouts; the focused test and both complete GNU/MSVC suites pass.
- `AbyssWhalen/portpeek` is public and both workflows use only standard
  `ubuntu-latest`, `macos-latest`, and `windows-latest` runners. GitHub's
  current billing documentation says those runner minutes are free and
  unlimited for public repositories; larger runners and excess artifact/cache
  storage are separate billable risks, and neither workflow uses a larger
  runner. Do not push, rerun, merge, tag, or publish without the user's explicit
  authorization. When authorized, advance the branch once and inspect that one
  CI run instead of using repeated pushes as a diagnostic loop.
- Release candidate `v0.2.0` is built and native-smoked. A Windows named
  auto-reset event complements the instance mutex: starting the same executable
  while RunCove is hidden wakes, unminimizes, and focuses the original window.
  The UAC handoff path waits without sending a wake event, and deterministic
  Windows tests cover both behaviors.
- Administrator mode is explicitly monitor-only. Backend guards reject start,
  stop, restart, restore, browser, folder, tray stop-all, and external
  termination actions before a user-controlled child process can run with an
  administrator token. The UI and tray expose and disable the same actions.
- Windows termination resolves `taskkill.exe` from the actual System32
  directory in both desktop and compatibility CLI paths, avoiding executable
  shadowing from a portable or current directory.
- Frontend tooling was updated from Vitest 2 to 4.1.10 after npm audit found a
  critical development-server advisory. The npm lockfile now uses the official
  npm registry throughout; complete and production-only audits report zero
  vulnerabilities.
- Release metadata is consistently `0.2.0` across root Cargo, desktop Cargo,
  npm, Tauri, both CLI version outputs, and the embedded WebView executable
  version. Release notes document the unsigned portable artifact and
  monitor-only administrator boundary.
- CI now verifies the cross-platform CLI plus the complete Windows desktop
  Rust, frontend, dependency-audit, and Edge E2E matrix. Release automation
  validates tag/manifests, builds four cross-platform CLI archives containing
  both commands, builds the Windows portable desktop archive, generates
  SHA-256 sums, and publishes the GitHub release.
- Current verification is green: root Rust format/Clippy/38 tests; desktop Rust
  format/Clippy/98 tests with one explicit live test ignored; frontend lint,
  typecheck, 85 tests, production build, npm audit, and three Edge Playwright
  flows at `900x600`, `1280x720`, and `1440x900`.
- The isolated release executable is
  `apps/desktop/src-tauri/target/final-20260811-v0.2.0/release/runcove-desktop.exe`
  (`25,348,756` bytes, SHA-256
  `C80B1B6601ED42A34F333773A3C5997FDDFBA74E7B81610F490479F931192A13`).
  Its local portable zip has SHA-256
  `E49A9408FE2BE7A89ACCC23376956C4627E8C781682CC05C68B00860C94CEA50`.
- Native smoke proved the final process remains PID 33824 after the title-bar X
  hides it and a second launch restores the same window; process count remains
  one. The old pre-fix PID 9728 was identity-checked, found to contain only
  RunCove/WebView2 processes, and stopped. The final instance is left running.
- GitHub publication is not complete. PR #1 is still Draft and the final local
  resource fix has not advanced the remote branch.

- Replaced the internal-sounding restore-set labels with user-facing language:
  Overview, Help, the elevation explanation, and native tray now consistently
  use `Previously running profiles`, `Restore previous run`, and `Startup
  order` (localized as `上次运行的配置`, `恢复上次运行`, and `启动顺序`).
- Project cards now expose a visible delete icon beside Open Folder and Edit.
  Deletion is disabled while any profile is starting, running, has a PID, or is
  already being operated on. A single confirmation dialog explains that
  RunCove registration, launch profiles, and port associations are removed but
  project files on the computer are not. The old editor-only delete flow was
  removed, and ref-level submission protection prevents duplicate requests.
- Focused deletion tests cover success, backend failure containment, and rapid
  repeated confirmation. Frontend totals are now `85/85` tests.
- Real-browser QA at `900x600` verified the restore action, disabled and active
  project delete controls, and the deletion dialog with no horizontal overflow,
  clipping, console errors, or warnings. Three local evidence screenshots are
  under `apps/desktop/output/playwright/qa-20260811-understandability/`.

- Automatic project discovery and optional Windows UAC elevation are complete.
  A saved development root is scanned once on startup and on demand; startup
  discovery reports non-blocking candidates, and registration still requires
  review and confirmation. The top-bar shield requests an administrator copy
  only after an explanatory dialog; standard monitoring remains the default,
  and protected kernel processes may still hide details.
- Added an in-app Help and usage guide from the top bar. It has five keyboard-
  navigable topics, contextual defaults for Overview/Ports/Projects, direct
  navigation actions, localized Chinese/English copy, and an independently
  scrolling responsive drawer for the `900x600` minimum window.
- Removed the duplicate top-bar hide and quit buttons. The Windows title-bar X
  now uses one native close policy: ask, hide to tray, or safely quit. The
  first prompt can remember a choice in the existing version-1 settings JSON,
  and Help > Safety exposes the current value and an ask-again reset.
- The planned Windows-first v0.1 is implemented and locally release-buildable.
- Final reliability work now corrects a live profile when its expected listener
  disappears or changes owner, propagates association persistence failures, and
  reports repeated dashboard refresh failures without notification spam.
- Session logs retain the existing per-profile behavior while sharing one
  global in-memory event cap. Project discovery runs off the Tauri IPC thread,
  skips unrelated inaccessible/deep subtrees, overlays observed runtime data in
  both single and bulk import, and accepts common block and inline pnpm workspace
  package lists.
- Partial Windows IPv6 scan failures preserve usable IPv4 entries, surface a
  degraded-scan message, and do not recalculate project status from incomplete
  data. CLI output fields and exit behavior remain compatible.
- Frontend technical failures now keep their raw detail behind a localized
  conclusion. Empty command lines render as unavailable.
- Reproducible Playwright coverage lives in `apps/desktop/e2e/` and uses the
  installed Microsoft Edge channel at `900x600`, `1280x720`, and `1440x900`.
  It builds and serves the production bundle, avoiding Vite development-mode
  cold-module stalls. The compact flow switches to `zh-CN` and back to English;
  all flows assert drawer, dialog, and expanded port-detail viewport bounds.
- Current final verification is green: root Rust format/Clippy/37 tests;
  desktop Rust format/Clippy/96 tests with one explicit live test ignored;
  frontend lint, typecheck, 85 tests, production build, and 3 Playwright flows
  that cover Help, close choice, and project deletion at all target viewports.
- The 2026-08-11 full matrix was rerun after the deletion changes. Playwright's
  `.last-run.json` is `passed`; the final system check found no listener on
  1424 or 14231 and no RunCove QA preview process.
- A separate read-only P0/P1 audit found no reproducible high-priority
  functional or security issue. It covered scanner degradation, external PID
  identity checks, managed Job Objects, lifecycle reservations, ordered restore,
  SQLite transactions/settings compatibility, and the Tauri IPC boundary.
- Visual QA produced 19 validated screenshots under
  `apps/desktop/output/playwright/qa-20260810-final/`; all target viewports,
  drawers, dialogs, tooltips, toasts, and console checks passed. The additional
  `900x600-long-toast.png` timing capture is superseded by
  `900x600-long-toast-final.png` and is not counted as evidence.
- An additional real-browser `900x600` snapshot verified the localized Help
  drawer, five-topic rail, four-step content, long Chinese wrapping, and the
  Projects navigation action; the temporary preview and browser were stopped
  after identity checks.
- Close-flow visual QA produced `900x600-close-choice.png` and
  `900x600-help-close-reset.png` under
  `apps/desktop/output/playwright/qa-20260810-close/`. The preview session and
  its port 1422 listener were identity-checked and stopped afterward.
- The current isolated release executable is
  `apps/desktop/src-tauri/target/final-20260811-understandability/release/runcove-desktop.exe`
  (`25,314,504` bytes, SHA-256
  `80A5729A1D58531BCC005B0CBB1F6216C36B9D2908DC260150C5ED4A69664CEC`).
- Completed: fixed the final close lifecycle race. Native remembered quit now
  runs shutdown on a worker and reports failure without blocking the Tauri
  event thread; repeated native close events are ignored while shutdown is
  active. The React shell also coalesces tray/title-bar quit actions through a
  shared in-flight guard, with regression tests for both entry points.
- Post-reboot verification reran the complete Rust/frontend matrix: root
  `cargo fmt`, Clippy, and `37/37` tests; desktop `cargo fmt`, Clippy, and
  `96 passed / 1 ignored`; frontend lint, typecheck, `82/82` tests, build; and
  Playwright `3/3` at `900x600`, `1280x720`, and `1440x900`.
- The release was built but not launched, installed, committed, pushed, or
  published. The older Help-build instance was gone after the post-reboot
  check and was not replaced. Ports 1421, 1422, and 14231 were free at final
  cleanup.

## Milestone History

- Completed: cloned `AbyssWhalen/portpeek` at commit
  `a6fb18d0dd79a0abc986de984c3ebca1ec6305c7` into the local `runcove` directory.
- Completed: established project instructions and implementation boundaries.
- Completed: renamed the root crate to `runcove`, extracted a shared CLI entry,
  and retained the `portpeek` compatibility binary.
- Completed: replaced per-entry `tasklist` calls with one Toolhelp process
  snapshot per scan, preserved distinct PID/state rows, and added verified
  process-tree termination.
- Completed: root Rust format, Clippy, tests, and both CLI help paths pass.
- Completed: implemented and locally exercised the Tauri backend and React
  desktop client, including tray behavior, structured project profiles,
  process-tree control, logs, port associations, and ordered restore.
- Completed: added bounded development-root scanning and selective bulk import.
- Completed: persisted the last successful development root in the existing
  settings JSON, rescanned it once on startup, and exposed non-blocking review
  candidates without silently registering projects.
- Completed: added explicit Windows UAC relaunch for enhanced monitoring,
  administrator-token validation, bounded single-instance handoff, localized
  status and confirmation UI, and clear denial/failure reporting.
- Completed: added the in-app Help and usage guide with contextual topics,
  keyboard tab navigation, direct Ports/Projects actions, bilingual copy, and
  viewport-checked responsive layout.
- Completed: replaced unconditional close-to-tray behavior with an explicit
  native ask/hide/quit policy, fixed native tray hiding, removed duplicate
  top-bar window actions, persisted optional remembering without a schema
  migration, and added Help > Safety reset controls.
- Completed: serialized per-profile lifecycle mutations so concurrent starts,
  edits, deletes, restores, and shutdown cannot race or publish stale events.
- Completed: final release rebuild, full verification rerun, and browser QA.
- Completed: moved remembered native quit cleanup to a worker thread, blocked
  duplicate close events while shutdown is active, added frontend race tests,
  reran the complete verification matrix, and rebuilt the isolated release.
- Completed: final delivery inventory and diff hygiene check.
- Completed: renamed restore-set UI language around the user goal, moved
  project deletion to each project card, and added a single safe confirmation
  flow with active-profile and duplicate-submission guards.
- Completed: real-runtime read-only inventory identified the active development
  services on ports 3100, 4321, 8080, and 1420 without stopping or adopting
  any of their processes.
- Completed: hardened project discovery so only exact service scripts
  (`dev`, `start`, `serve`, and `preview`) are selected automatically; projects
  containing only maintenance or destructive scripts are not batch-importable.
- Completed: persist the ordered live restore set on start/stop/exit, preserve
  the pre-stop snapshot across explicit shutdown, and make shutdown idempotent.
- Completed: keep profiles `Starting` until expected ports belong to their
  managed process tree; classify user stops, normal exits, and abnormal exits.
- Completed: surface tray stop-all failures through `tray-stop-all-error` and
  show the main window instead of silently discarding errors.
- Completed: harden confirmed associations, managed exit PID events, external
  termination identity checks, and sensitive runtime argument filtering.
- Completed: enrich real npm imports from descendant `--port` evidence without
  copying descendant argv, excluding Astro/workerd auxiliary listeners.
- Completed: isolated live acceptance passed for the active services on ports
  3100, 4321, 8080, and 1420; PID, start time, and executable identity remained
  unchanged and no child process or run session was created before Conflict.
- Completed: reran the complete verification matrix and rebuilt the isolated
  Windows release executable from the final source.
- Completed: inspected the real Tauri window against the active local services,
  then fixed interleaved duplicate Windows UDP endpoints discovered in the live
  5353/mDNS rows. Exact duplicates are now collapsed at the scanner boundary
  while distinct PID, protocol, state, and IPv4/IPv6 bindings remain visible.
- Completed: after explicit approval, exited old RunCove PID 50656, launched the
  final executable as PID 84412, and left the final instance running for the
  user. Existing services on ports 3100, 4321, 8080, and 1420 retained their
  original PIDs and identities.
- Completed: final-source desktop read-only smoke and ten-minute whole-process-
  tree soak. The window reported zero managed runs and zero conflicts; the
  scanner reported no exact duplicate endpoint groups. Computer Use could read
  the final window but could not inject a view-switch click because its Node
  bridge returned `node_repl exec context not found`; no blind input fallback
  was used.
- Completed: added typed `system / en / zh-CN` language selection across the
  React client and native tray, persisted it in the existing version-1 settings
  JSON, and kept old settings compatible without a database schema change.
- Completed: hardened language synchronization so invalid WebView storage
  cannot overwrite the durable backend preference, system language follows the
  primary locale, and a failed tray refresh cannot roll back a committed
  preference or split frontend and database state.
- Completed: fixed default/minimum-window clipping, responsive table columns,
  tooltip bounds, modal focus/escape behavior, persistent operation errors,
  log failure recovery, and browser-action trust checks. A Playwright boundary
  check found and fixed the remaining `1121-1199px` Ports action-column gap.
- Completed: the post-fix frontend suite passes lint, typecheck, 31 tests across
  10 files, and the production build. Root and desktop Rust format, Clippy with
  warnings denied, and full tests pass; desktop reports 57 passed and one
  explicitly configured live test ignored.
- Completed: final bilingual Playwright checks cover `900x600`, `964x601`,
  `1024x640`, `1120x700`, `1280x760`, and `1440x900`, plus the responsive
  boundaries. Pages, tables, detail rows, import dialogs, logs, actions, and
  tooltips remain inside the viewport with no console errors or warnings.
- Completed: built the optimization-pass release in an isolated target. The
  existing PID 84412 remains the previously accepted build and was not stopped
  or replaced.
- Completed: launched an additional QA-flavored binary with application id
  `com.abysswhale.runcove.qa`. It created only its isolated database and normal
  WebView2 process tree, with no Node/npm project descendants, then the
  identity-checked QA PID 116720 was stopped. Native screenshot/input remained
  unavailable because the Computer Use bridge repeatedly returned
  `node_repl exec context not found`; no blind coordinates were used.
- Completed: repeated the final-source whole-process-tree soak with an isolated
  QA executable. The seven-process tree remained stable for 597.328 seconds,
  averaged 0.346124% total-machine CPU, and was stopped only after PID 133636,
  executable path, and process-tree identity were rechecked. Its WebView2
  descendants exited with it. The previously running RunCove PID 84412 and
  listeners on 1420, 3100, and 4321 were untouched; the previously observed
  external 8080/PID 105280 service was already absent at the post-soak check.
- Completed: bounded each captured stdout/stderr line to 16 KiB while retaining
  normal unterminated tails, and made watcher exit commits generation-safe so
  an old process cannot overwrite a restarted profile's state.
- Completed: classify CLI kill candidates from `LISTEN` endpoints only, handle
  extended Windows/UNC paths during project inference, and merge an inactive
  expected port with matching association history for the same owner.
- Completed: subscribe to live logs before loading history without dropping or
  collapsing genuine duplicate lines, classify conflict/unexpected run events
  as errors, clear runtime state for profiles removed by project edits, and
  release the tray runtime mutex before calling native menu APIs.
- Completed: moved repeatable Edge E2E from the Vite development server to a
  production preview, added a bounded cold-start retry for a genuine blank
  `about:blank` page, and automated Chinese language plus vertical port-detail
  checks. A clean post-reboot run passed all three target viewports.
- Completed: made Playwright output deny-by-default in `.gitignore`, retaining
  only 19 curated final QA PNGs and excluding traces, session snapshots, logs,
  PID files, duplicated captures, and the superseded long-toast image without
  deleting local evidence.

## Key Paths

- Shared Rust core and CLI: `src/`
- Rust integration tests: `tests/`
- Desktop application target: `apps/desktop/`
- Decision and verification log: `notes.md`

## Decisions

- Product and primary command: `RunCove` / `runcove`.
- Legacy `portpeek` command remains compatible.
- Windows 11 desktop app with tray; no application or project autostart.
- SQLite is isolated to RunCove's application-local data directory.
- Project registration or explicit confirmation is authoritative; inference is
  never silently promoted.
- The Windows title-bar X asks whether to hide or safely quit until the user
  remembers a choice. Remembered quit still saves the restore set and stops
  managed runs; Help > Safety restores ask-every-time behavior.

## Historical Verification Evidence

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --offline --all-targets --all-features -- -D warnings`: passed.
- `cargo test --offline --all-targets`: passed, 34 tests total.
- `cargo run --offline --bin runcove -- --help`: passed.
- `cargo run --offline --bin portpeek -- --help`: passed.
- Live Windows scan: 133 of 133 entries resolved process names; one scan took
  about 64 ms on this machine.
- Frontend lint, typecheck, production build, and 14 Vitest tests passed.
- Playwright flows were checked at `1280x720` and `1440x900`, including ports,
  logs, project import, development-root bulk import, and association
  confirmation. The final browser console had no errors.
- Desktop Rust debug tests passed: 49 passed and the explicitly configured live
  acceptance test was ignored by default. The same live test passed separately
  for ports 3100, 4321, 8080, and 1420. Desktop Clippy passed with warnings
  denied.
- A read-only Windows UI capture confirmed the real Tauri Ports view was using
  the native scanner and displaying the active `3100/tcp` listener with PID
  68036. A post-fix CLI scan returned no exact duplicate endpoint groups and
  retained all four acceptance services.
- `npm run tauri build`: passed with the final source using an isolated target.
  The executable is
  `C:\Users\93137\AppData\Local\Temp\runcove-tauri-final-20260807-212342\release\runcove-desktop.exe`
  (`24,740,334` bytes, SHA-256
  `F135EDE741512E112E1F01FC0519E5278FEB7510C56E1C968053F96974314423`).
- The 2026-08-08 optimization pass reran frontend lint, typecheck, 31 Vitest
  tests, production build, both Rust format/Clippy/test matrices, and the Tauri
  release build. The new executable is
  `C:\Users\93137\AppData\Local\Temp\runcove-tauri-i18n-20260808-0047\release\runcove-desktop.exe`
  (`24,797,285` bytes, SHA-256
  `57DCF54FE1663D6EF6814AFB06C99DF0AE29C1D55F12B08FC14864B41502E550`).
- Final responsive measurements showed equal client and scroll widths for the
  document, content, and Ports table at every required viewport. Actions were
  visible throughout; expanded detail rows use four columns below the compact
  breakpoint and eight columns when the full table fits.
- The earlier pre-dedup ten-minute idle baseline averaged 0.3533% CPU over
  656.3 seconds and showed no sustained memory growth.
- Final-source idle monitoring passed the target over 604.924 seconds and 122
  samples: the seven-process tree averaged 0.977930% of total machine CPU.
  Private memory changed from 321.566 MiB to 323.027 MiB with no monotonic or
  sustained-growth pattern. Working set changed from 165.895 MiB to 196.934
  MiB and showed a warm-up/residency increase, so a longer soak remains a
  residual performance check rather than a first-release blocker.
- A second isolated final-source soak covered 597.328 seconds and 114 samples.
  Process count stayed exactly seven and average CPU was 0.346124% (0.052815%
  minimum, 0.867248% maximum). Working set fell from 543.973 MiB to 461.965
  MiB. Private memory moved from 289.645 MiB to 301.973 MiB with a positive
  4.409032 MiB/minute fitted slope and a wide 277.625-330.863 MiB range; this
  run independently passes the CPU target but does not justify claiming fully
  flat private memory, so longer monitoring remains a residual check.
- Runtime state was confirmed outside the repository at
  `C:\Users\93137\AppData\Local\com.abysswhale.runcove\runcove.sqlite3`.

## Next Session Prompt

RunCove `v0.2.0` is implemented, fully reverified, release-built, packaged, and
native-smoked, including real hide-to-tray and second-launch wake behavior.
Start by reading the current checkpoint above and `notes.md`; do not repeat the
completed hardening or resource diagnosis. PR #1 is at `c281340`; resource
linking is fixed, and the only remaining local change makes the tray stop-all
test independent of the runner's administrator state. It passes format,
warnings-denied Clippy, and `99 passed / 1 ignored` locally and still needs one
focused commit/push, PR CI, merge, tag, release, and asset verification. The
repository is public and its standard hosted runners do not incur Actions
compute charges. Preserve historical `v0.1.0`, do not rename the remote
repository, and do not stop unrelated development services.
Real UAC cancel/success and a longer idle soak remain residual manual checks and
must not be overstated.
