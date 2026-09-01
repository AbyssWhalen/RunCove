# RunCove Implementation Notes

## 2026-08-31 Post-Release Review

A read-through of the shipped v0.4.0 code, with the user's standing approval to fix what
it found. One real defect, one regression the fix caused and its cause, and one fix that
was written, measured against an existing test, and then withdrawn.

- **A restore and a whole-group action could run at once, and the guards were asymmetric
  in a way that hid it.** `runGroupAction` checked `restoreInFlight`, so clicking a group
  during a restore was refused — but the button stayed live, so the click just did nothing.
  The other direction was worse: `restoreLastRunSet` checked only its own latch, and the
  restore button ignored `busyGroups`, so a restore started during a group action and both
  walked overlapping profiles. Nothing corrupts — the backend reserves each profile — but
  whichever arrived second failed with an "already starting" the user did nothing to cause.
  Fixed at both layers, latch and button, because either alone leaves the other door open.
- **Adding the profile label to the restore message broke two tests, and the cause is worth
  keeping.** `profileLabel` closed over `snapshot`, so putting it in `restoreLastRunSet`'s
  dependency list made that callback change identity on every one-second poll — and the
  tray subscription effect depends on that identity, so it tore down and re-subscribed once
  a second. The two tests that failed
  (`coalesces repeated tray restore requests while restore is running` and the toolbar one)
  were pinning exactly that. Fixed by reading the snapshot through a `latestSnapshot` ref so
  the callback is referentially stable. The rule this leaves behind: a callback that feeds
  an effect's dependency array must not close over the polled snapshot.
- **The group-versus-group overlap was first attacked in the frontend, and that attempt
  failing pointed at the real layer.** Two groups may share a member, and starting both at
  once made the second fail on the shared one. A guard that disabled the second group's Start
  worked, but `LaunchGroupSection.test.tsx`'s `locks a group's whole row while one of its
  actions is in flight` asserts at its last line that a *different* group's Start stays
  enabled, and that fixture's `Morning stack` `[db, web]` and `Everything down`
  `[web, astro]` share `web`. The guard and a deliberate, shipped assertion were in direct
  conflict, so the guard came out rather than the assertion.

  Read again, that conflict was information rather than an obstacle. The assertion is right:
  a user with two overlapping groups should be able to start both, and a UI that greys out
  the second is answering a question the UI cannot answer — it does not know whether the
  shared member is about to be ready. What was wrong was the backend refusing. **The fix is
  therefore in `commands.rs`, and it makes the assertion true rather than working around it.**

  Three candidate semantics for reaching a profile another operation holds:

  | Semantics | Verdict |
  | --- | --- |
  | Fail (what shipped) | Rejected. The situation is ordinary, and the report reads as a fault the user caused. |
  | Skip the member and continue | **Rejected, and this is the subtle one.** It breaks the ordering promise: the group's whole point is that `web` does not start until `db` is ready, and skipping `db` starts `web` against a database that is still coming up. |
  | Wait for it, then take the ordinary path | Chosen. Ordering holds, and the reserved start's `AlreadyRunning` early return makes a member the other operation finished cost one event. |

  Waiting also composes with the two directions that are not a start racing a start: a member
  another operation *stopped* is then started, which is what this group asked for; and a group
  *stop* that waits out a start re-checks `processes.info` afterwards, because the operation
  it waited out may have been the very stop that member needed.

  The wait is polled rather than signalled. A reservation is a `HashSet` entry, and adding a
  condition variable to serve a wait that resolves in seconds would put a second
  synchronization mechanism inside the lock that guards every lifecycle operation. A shutdown
  is returned immediately instead of waited out, since nothing starts during one.

  The budget is `PROFILE_READY_TIMEOUT_SECS + 5`, derived rather than written as its own
  number: the operation being waited on may spend its entire readiness budget, so an equal
  budget would let the waiter report a failure for a start that had not failed yet.

  `try_reserve` was added to `ProcessManager` for this, because `AppError` is a flat string
  with no discriminator — without it, the caller would have to match on
  `"Another lifecycle operation is already in progress"` to tell a held profile from a
  refusal, and a message edit would silently change control flow. `reserve` is expressed
  through it, so no existing caller or message changed. Both tests were mutation-checked
  against the old behavior. No existing assertion was touched at any point.

  The frontend guards from the withdrawn pass stayed, re-justified: they stop the user
  queueing work already underway. Correctness in the backend, courtesy in the UI.
- **The release workflow's checksum step was defective, and the fix removed the step rather
  than correcting it.** `release.yml` ran `sha256sum ./*.zip ./*.tar.gz | sed 's# \./#  #'`.
  `sha256sum` already emits `HASH␣␣./name`, so substituting `␣./` with `␣␣` produced *three*
  spaces, one more than `sha256sum -c` accepts — every line of the published file failed to
  open on a byte-perfect download. The narrow fix was `sed 's#\./##'`, but the better one is
  to stop needing a fix-up at all: `sha256sum -- *.zip *.tar.gz` emits the accepted shape
  directly, and `--` protects a filename starting with `-` exactly as the `./` prefix did.
  A `sha256sum -c SHA256SUMS.txt` now runs before the publish step, so the file RunCove tells
  people to verify with is proven to work in the same job that writes it. That check, not the
  substitution's absence, is what stops this class of defect from shipping again.

  The `./` prefix was not pointless — it was guarding against a leading-dash filename — which
  is why this is worth recording: the defect was a correct instinct implemented with the wrong
  tool, not carelessness. `RELEASE_NOTES.md` also now prints the actual commands, including
  `--ignore-missing`, because a plain `-c` counts the four archives a Windows-only user did
  not download as failures.
- **Verified clean, so these are not suspects:** `launch-group.ts` handles the empty group
  (`every` over nothing is vacuously true, and it returns `idle`); `PRAGMA foreign_keys` is
  ON in both `Storage` constructors, so the `ON DELETE CASCADE` that justified real tables
  over settings JSON actually fires; `save_launch_group`'s early returns drop the transaction,
  which rolls it back; the eight `RunStatusReason` variants are all covered by
  `run-status.ts`; `zhCnMessages: Record<MessageKey, Message>` makes the compiler enforce
  bilingual parity, which `typecheck` passing proves — 450 keys each side, no key on one side
  only; and no `TODO`, `FIXME`, or `todo!` remains in any production source file.
- **Every new test was checked against a reverted fix, not just for passing.** Each fails
  without its guard and passes with it. A test that is green either way pins nothing.
- **The one step reserved for the user was rehearsed on a copy, and the rehearsal changed
  what should be said about it.** The `1 → 2 → 3` migration of the real database was the
  outstanding risk, so the live file was copied and the copy handed to `Storage::open` —
  the exact call `lib.rs:240` makes at startup, reached through a temporary test in
  `storage.rs` that was deleted afterwards. The copy migrated to `user_version = 3` with all
  three added tables present and every reader (`settings`, `list_projects`,
  `list_launch_groups`, `list_sessions`) working; the live file was re-read afterwards and is
  still at 1.

  Rehearsing on a copy rather than reasoning about it is the point: the migration tests use
  synthetic fixtures, and a fixture cannot tell you whether *this* file migrates. What it
  found is that the live database holds 0 projects, 0 profiles, 0 associations, 4 run
  sessions, and 2 settings rows, last written 2026-08-11. So the honest description is
  "irreversible, rehearsed successfully, with essentially no user data at stake" — not
  "risky". The earlier framing was accurate about irreversibility and silent about volume,
  which made it read as more dangerous than it is. The backup stays anyway, because the
  argument for it never depended on the row count.
- **Then the same rehearsal was run through the shipped binary, which is a different claim.**
  A library call proves `Storage::open` migrates; it does not prove the application starts.
  So an isolated build was made — identifier `com.abysswhale.runcove.verify0901`, and
  `INSTANCE_MUTEX_NAME` changed with it, because that constant is hardcoded rather than
  derived from the identifier and an isolated build would otherwise contend with a
  production instance for the same mutex — its data directory was seeded with another copy of
  the live `user_version = 1` file, and it was launched. It came up with a window titled
  `RunCove` and migrated the seeded database to 3 during startup. Both patched files were
  restored with `git checkout --`.

  Worth stating plainly because it was not true before today: **this is the first evidence in
  the project that the built application boots.** Everything else — 252 desktop tests, 211
  frontend tests, 7 e2e — exercises units or a mocked frontend. Cite the two separately; a
  green suite has never implied a binary that runs.

  The isolated-build pattern is the reusable part: patch the identifier *and* the mutex name,
  seed the isolated data directory with a copy, launch, then `git checkout --` the two files.
  It answers "does it work on the user's data" without ever opening the user's data.
- **The launch-group feature had no test of its own happy path, and that gap survived the
  release.** Asked whether v0.4.0 could be called stable, the honest answer required checking
  rather than recalling — and the check found that of the three group tests in `commands.rs`,
  one injects a closure, one exercises the empty-group refusal, and one exercises a port
  conflict. The e2e suite drives `mock-data.ts`. The isolated launch above booted the binary
  but clicked nothing. So **"press Start and the stack comes up" had never been executed by
  anything**, in a release whose single headline feature is launch groups.

  `a_whole_group_starts_real_processes_in_order_and_stops_them_together` closes it: two real
  `node.exe` members, each binding its own expected port, asserted on launch order, both
  processes running, a second start reusing the same PIDs, and a reverse-order stop that
  leaves nothing behind.

  **The port assertions are the load-bearing part, and the reason is worth keeping.** Reading
  `processes.info` alone would pass on a walk that never waited for readiness — `info` is
  populated at spawn, not at readiness. Refusing a fresh `TcpListener::bind` on *both* ports
  after the walk returns is what makes the ordering promise observable, because it can only
  hold if every member is still up when the last one finishes. Mutation-checked by walking the
  stop forward instead of in reverse.

  The general lesson, since this is the second time this shape has come up: a feature can have
  several tests and still have none on the path a user takes. Count the paths, not the tests.
  Both times the missing one was the happy path, because failure paths are easier to write.
- **The desktop executable was the one shipped binary built without the project's release
  settings, and a size measurement is what exposed it.** Installing the app locally put a
  25,912,797-byte exe next to the 13,212,672-byte one published with v0.4.0. A 1.96x gap
  between the same source at the same version is not explainable by build noise, so it was
  worth chasing rather than shrugging at.

  The cause is a structural assumption that was never true here. The root `Cargo.toml` carries
  `[profile.release] strip = true, lto = true, codegen-units = 1`, and profile settings apply
  to a whole workspace — but **the root manifest has no `[workspace]` section**, so
  `apps/desktop/src-tauri` is an unrelated package rather than a member. When cargo's build
  root is the desktop manifest, the root's profile is not read at all. Cargo does not warn:
  ignoring a profile in a non-root manifest is normal, and there is nothing here for it to
  call suspicious. So the CLI binaries got strip and LTO and the desktop application — the
  artifact users actually download — got neither, which is the exact inverse of what writing
  those settings once was meant to achieve.

  Fixed by repeating the block in `apps/desktop/src-tauri/Cargo.toml` with a comment saying
  why it is duplicated, so the next reader does not "clean up" the duplication. Creating a
  real workspace would remove the duplication instead, and was rejected for this change: it
  moves both packages' target directories and lockfiles and touches the release workflow, all
  to tidy six lines. The exe is now **8,557,568 bytes** — 0.33x the local build before the
  change and 0.65x the published v0.4.0.

  **The published artifact was checked directly rather than inferred from the local size.**
  `/tmp/zipx/runcove-desktop.exe`, extracted from the v0.4.0 download, holds one
  `runcove_desktop.pdb` reference and 632 source-path strings; the new build holds zero PDB
  references and 288. A `strip` removes the debug directory that names the PDB, so that pair
  of readings is the evidence the shipped binary was built without it.

  Two corrections to the obvious reading of those numbers, both of which the first write-up
  got wrong. First, **most of the shrink is LTO, not `strip`**: the release profile defaults
  to `debug = false`, so there was little debuginfo to remove, and `lto = true` with
  `codegen-units = 1` did the bulk of the work. Attributing 17 MB to `strip` would send the
  next reader looking in the wrong place. Second, 288 source paths survive in the stripped
  build because panic messages embed `file!()` as ordinary `.rdata` string data — `strip`
  takes the symbol table and debug directory, not string literals, so do not describe it as
  removing path traces.

  **One thing is measured and unexplained, and is left that way on purpose.** The local build
  before the change was 25,912,797 bytes while CI's published build was 13,212,672, and
  neither had the profile. Nothing found accounts for a 1.96x gap between them: no
  `RUSTFLAGS`, no `.cargo/config.toml` build overrides in the repository, and the only global
  cargo config sets a registry mirror. The pre-change binary has been overwritten, so the
  question is no longer testable here. It does not affect the fix — the shipped binary
  demonstrably lacked the settings and demonstrably works with them — but a guess written
  down as a cause would be worse than the gap.

  The build-time cost was overstated on first measurement and the correction is the useful
  part. The build right after the change took 3m22s, which invited "LTO doubles the build" —
  but that build recompiled every dependency under a profile none of them had been built
  with. The next full release build took **1m53s** against roughly 1m34s before, so the
  recurring cost is about twenty seconds and the large number is paid once per profile
  change. Measure a build-time regression on the second build, not the first.

  **`cargo test` cannot verify this change**, which is the part worth carrying forward: tests
  build under the `test` profile, so a green suite says nothing about whether LTO and strip
  produced a working release binary. The isolated-build recipe from the migration rehearsal
  answered it instead — identifier and mutex patched to `lto0901`, data directory seeded with
  a `user_version = 1` copy, launched. It came up titled `RunCove`, spawned six WebView2
  processes including a live renderer, sampled 1,415 distinct colours across its 1295×800
  client area, and migrated the seeded database to 3 with the 4 sessions and 2 settings intact.
  The colour count is the assertion that matters: a binary whose embedded frontend assets had
  been damaged would still migrate the database and still show a window, just an empty one.

## 2026-08-31 P3 Housekeeping Disposition

P3 had two halves. One was done, one was declined after measuring it, and the second is the
decision worth keeping.

- **Deleted the three stale remote branches.** `feat/launch-groups` (`9e6ea53`),
  `codex/release-v0.3.0` (`4ca80a4`), and `codex/runcove-v0.2.0` (`0a14cea`) were all
  ancestors of `main`, so deleting the refs lost no commit and rewrote no history; each can
  be recreated exactly with `git push origin <sha>:refs/heads/<name>`. Two of the three
  named a tool in the branch name and were publicly visible on the repository page, which
  is a better reason to remove them than housekeeping was. `origin` now holds `main` alone.
- **Did not move `V0.2.1_PLAN.md` and `V0.3.0_PLAN.md` out of the tracked tree, which the
  plan called for.** `notes.md` and `HANDOFF.md` cite `V0.3.0_PLAN.md` about forty times
  and most of those citations carry line numbers — they are how a decision record points at
  the design it came from. Untracking the file would leave every clone holding forty
  references to a file it does not have, which is a direct cost to the one thing this
  repository's documents are for: letting another reviewer follow a decision to its
  evidence. The alternative of moving only `V0.2.1_PLAN.md` (three citations, none by line)
  trades the pair's consistency for one filename. The original motive for this half of P3
  was the 软著 application's AI-trace scrubbing, and that workstream was dropped on
  2026-08-30, so what remained was tidiness — not enough to pay that price. Reopen it only
  together with a plan that rewrites the citations.

## 2026-08-31 v0.4.0 Release Decisions

- **The release preparation went on the feature branch, not on `main` after the merge.**
  Version numbers, both Cargo lockfiles, `package-lock.json`, the changelog, the release
  notes, and the README all landed in `f90b8a6` on `feat/launch-groups` so that PR CI would
  run against them. The reason is asymmetric: `release.yml`'s CLI job builds with
  `cargo build --locked` and `ci.yml` does not, so a lockfile that disagrees with a manifest
  is invisible until the tag already exists — and by then the tag is the thing that would
  have to be moved. `ci.yml` does run `npm ci`, so npm drift would have failed the PR
  either way. Both packages were also checked locally with `cargo check --locked` before
  the push.
- **Three third-party crates sit at `0.3.0` in the desktop lockfile** — `dtor`,
  `fallible-iterator`, `urlpattern` — so a blind search-and-replace of the old version
  string would have corrupted it. Both lockfiles were regenerated with `cargo check`
  instead of edited, and ours were identified by reading the surrounding
  `[[package]] name = ` lines.
- **The release note's upgrade path was widened after the first release-prep commit**
  (`9e6ea53`). It said the database migrates "from schema version 2 to version 3", which is
  the step v0.4.0 adds but not the path a v0.2.1 user takes — theirs goes through version 2
  on the way. The consequence sentence was already right for everyone; the path was not, and
  an irreversible schema step is the wrong place to be right for only the most recent
  release. One extra CI cycle was cheaper than a release note that misdescribes a one-way
  upgrade.
- **`README.md`'s `## v0.3.0 Scope` became `## v0.4.0 Scope` rather than gaining a sibling.**
  The launch-group plan said to keep the v0.3.0 section as a historical record and add a new
  one, but that was written when the README still described v0.3.0 as the current release.
  Two Scope sections would have carried the same seven-item exclusion list twice in a
  user-facing file, and the per-release detail already lives in `CHANGELOG.md`, so the
  duplicate was dropped and the exclusions kept.
- **The published archives were verified twice, by two methods, and the fallback is worth
  keeping.** For most of release day `release-assets.githubusercontent.com` refused every
  connection from this machine — `gh release download` and `curl -L` both reset, ~60
  attempts, while `github.com` and `api.github.com` worked — so the v0.3.0 method of
  downloading all five archives and running `sha256sum -c` was unavailable. The substitute:
  the `sha256sum` output printed by the `Publish GitHub release` job, which *is* the content
  of the published `SHA256SUMS.txt`, diffed against the `digest` field the API reports for
  each stored asset. Those are produced independently — one on the runner before upload, one
  by GitHub from the bytes it stored — and all five matched, which proves the published bytes
  are what CI built without proving what a downloader elsewhere receives. Later the same day
  the asset host recovered, and the real check ran: all six assets downloaded, `sha256sum -c`
  `OK` on all five archives, agreeing with both the digests and the sums below. The desktop
  zip holds `runcove-desktop.exe` at `FileVersion 0.4.0`, a `README.md` byte-identical to
  `git show v0.4.0:README.md`, a `CHANGELOG.md` headed `[0.4.0] - 2026-08-31`, and `LICENSE`.
  Keep the digest method written down: it is the only verification available when the asset
  CDN is unreachable, and it very nearly was the whole record for this release.
- **`sha256sum -c SHA256SUMS.txt` fails on the published file, and the bytes are not the
  reason.** All five lines report `No such file or directory`. `release.yml` builds the file
  with `sha256sum ./*.zip ./*.tar.gz | sed 's# \./#  #'`, and since `sha256sum` already emits
  `<hash><space><space>./<name>`, replacing `" ./"` with two spaces yields **three** spaces
  where the format allows exactly two — so `sha256sum` takes the leading space as part of the
  filename and looks for `" runcove-…"`. Every archive is intact, and no document gives a
  wrong command: `RELEASE_NOTES.md:66` says only "verify the archive against
  `SHA256SUMS.txt`" and `README.md` never mentions the file. The defect is that the obvious
  way to carry out that instruction fails on a good download, which is worse than cosmetic
  because five `FAILED` lines read as a corrupt or tampered archive.
  Normalizing works today —
  `sed -E 's/^([0-9a-f]{64})[[:space:]]+/\1  /' SHA256SUMS.txt | sha256sum -c -` — and the
  real fix is `sed 's#\./##'` in the workflow, which is out of bounds here and cannot be
  applied to an already-published asset regardless. Carried as a v0.5.0 item together with
  the release-note wording, not patched now.
- **The checksums themselves, recorded so the claim above can be re-checked without
  trusting it.** These are the sums the workflow computed on the runner and published as
  `SHA256SUMS.txt`, and they equal GitHub's stored-object digest for every asset. Re-verify
  by downloading an archive and hashing it, or by reading `.assets[].digest` from
  `gh api repos/AbyssWhalen/RunCove/releases/tags/v0.4.0`.

  ```text
  853d269b36d8065db4b173611a9a79b791b123487524509e9299be06243da96c  runcove-cli-linux-x86_64.tar.gz
  fd4e23586fa922a4f54af0b529a58b5b702c081bd6b8f0c8c978add075c67ec4  runcove-cli-macos-aarch64.tar.gz
  1691e2d03c020cd2424835396538236593a802ac61c6fec85319e4f4a604ecc7  runcove-cli-macos-x86_64.tar.gz
  febb3c20847c0ab685b603eb642af8e00df7f91b1e516aec413321796dc3f73e  runcove-cli-windows-x86_64.zip
  8707794802c3cf5c0563809376a22be2c24078d14ffd934334b125968fb4051f  runcove-desktop-windows-x86_64-portable.zip
  ```
- **The network to GitHub was unreliable all session and retrying was the right first
  move, not diagnosing.** Pushes failed with `Recv failure: Connection was reset` and
  `Failed to connect to github.com port 443` and then succeeded unchanged — one on the
  third attempt, one on the sixth. No proxy is configured and none was added. A retry loop
  written as `if git push ... | tail -2` is useless, because the pipeline's exit status is
  `tail`'s; capture the output and test `$?` instead.
- **The merge commit body carries no AI marker.** `bd2b777`, the v0.3.0 merge, contains
  `[codex]`; `0d6b934` does not, and the subject follows the same
  `Release RunCove vX.Y.Z (#N)` shape so the two releases still read as a series. The two
  `[Qoder]` markers and that `[codex]` one stay where they are — rewriting published history
  would break the tags and the release associations.

## 2026-08-31 Accessible Names: A Wrapping Label Absorbs Its Own Error Text

- **The defect, measured before it was fixed.** Five fields in `ProjectModal.tsx` wrapped
  their `<input>` in a `<label>` that also renders that field's validation error, and the
  accessible name of an input labelled by a wrapping `<label>` is the label's whole text
  content. So the moment an error appeared, `Program` became
  `Program This field is required.` and the field answered to a name nobody would look
  for. The error was already wired through `aria-describedby`, so it was also being
  announced twice. Confirmed as red first: the new test failed at
  `getByLabelText("Project name")` with all five fields invalid, then passed after the
  fix, with no other assertion touched.
- **Why the existing suite could not see it.** `ProjectModal.test.tsx`'s validation test
  queries every field *before* it submits and then holds the element references
  (`:125-129`, asserting at `:135-136`), so it never asks for a field by name while an
  error is on screen. That is the exact window the defect lives in — worth remembering as
  a shape, not just as one bug: a query taken before the state change cannot observe a
  name that only breaks after it.
- **The fix is `id` + `aria-labelledby` on the caption**, matching `LaunchGroupModal.tsx`,
  which pins the name to the caption regardless of what else the label renders. Five
  source sites, three of them inside the profile loop so their ids carry the index:
  `project-name-label`, `project-path-label`, and `profile-${index}-{name,program,cwd}-label`.
  **Five, not the "~13" an earlier note estimated** — that number counted runtime
  instances of the looped fields, and the source sites are what get edited.
- **Deliberately unchanged**: the argument and expected-port inputs, whose labels are
  `<div>`s and whose inputs already carry explicit `aria-label`s, and the root-import
  checkbox, which needs its `aria-label` precisely because its label holds a paragraph of
  project metadata.

## 2026-08-31 Branch And PR: Three Commits Because Two Milestones Share Four Files

- **P1 and P2 cannot be split by commit.** `models.rs`, `commands.rs`, `App.tsx`, and
  `messages.ts` each carry both the run-status `reason` work and the launch-group work
  (measured interleave: `commands.rs` group 46 / reason 16, `messages.ts` group 66 /
  reason 18, `models.rs` group 12 / reason 30, `App.tsx` group 30 / reason 8). Splitting
  them needs hunk-level staging, interactive `git add -p` is unavailable in this
  environment, and the halves would not build. So the granularity chosen was **every
  commit builds on its own**: `f8a2447` the matrix fix, `fc56693` all code, `6b80f61` all
  documentation, `842efb9` the accessible-name fix. The feature commit's body records why
  the run-status fix rides along, so the decision is auditable from `git log` alone.
- **`git push` to `origin/feat/launch-groups` failed once with `Recv failure: Connection
  was reset` and succeeded on the immediate retry**, with no proxy configured. Retry
  before diagnosing; nothing was half-pushed.
- The `gh pr create` body went through a file rather than a heredoc: a heredoc carrying
  the body tripped bash's parser (`unexpected EOF while looking for matching '`). Write
  the body to a file outside the repository and pass `--body-file`.

## 2026-08-31 P2 Decision: Launch Groups Add Persistence And A UI, Not A Second Launcher

- **Decision: the runtime mechanism was already there, so the feature is persistence plus a
  thin command layer.** `restore_profiles` is already an ordered, fail-fast launcher and
  `wait_for_profile_ready` already waits for the expected ports to listen under this
  profile's own process tree. `start_launch_group` therefore loads the group and calls
  `restore_profiles`, and no new state machine, no new event, and no new poll exist. Three
  behaviors come free rather than being re-implemented: a member that is already running
  returns `AlreadyRunning` and counts as started, which makes a group start idempotent and
  turns Start into "fill the gaps"; the first failure stops the walk and keeps what
  started; and the `relatedPort` payload that drives `View occupant` arrives unchanged.
- **Decision: two tables, not a field in the settings JSON, and `ON DELETE CASCADE` is the
  entire reason.** Deleting a profile has to remove it from every group that listed it. In
  the JSON that means every read filters dangling ids forever and one missed filter is a
  crash; in SQLite it is one clause the database enforces. The price is a schema version
  bump, which the user authorized on 2026-08-31 for exactly this. Positions may then have
  holes, which costs nothing because every read is `ORDER BY position` and no code treats a
  position as an index.
- **Decision: stop walks in reverse and does not stop the rest.** Start fails fast because
  continuing past a failed dependency starts things that cannot work; stop cannot borrow
  that reasoning, because interrupting a stop leaves *more* processes running than
  finishing it. Each member goes through the same `stop_profile_inner`, a failure lands in
  `failures`, and the walk continues — the same choice `processes.rs`'s
  `stop_all_with_intent` already made.
- **Decision: a group has no stored state.** Running / partial / idle is derived in
  `components/launch-group.ts` from member statuses the snapshot already carries, so the
  backend gained no status column and no group event, and the derived answer cannot drift
  from the members. "Up" means `running` or `starting`; `conflict` is not up, which is the
  repository's existing rule and not a new one.
- **Decision: groups may cross projects.** Members reference `launch_profiles(id)`
  directly. A database in one project and a web app in another is the case that motivates
  the feature, so forbidding it would remove the point.
- **Decision: reservations stay per member.** `restore_profile` already reserves one
  profile at a time and `reserve_many` was not used, so
  `start_profile_inner_reserved`'s contract did not change. The cost is that a user can
  still act on an individual profile while a group is starting; restore shipped and was
  verified with exactly that cost.
- **Decision: empty groups are refused at validation.** `validate_launch_group` requires a
  trimmed non-empty name, at least one member, no duplicate members, and every referenced
  profile to exist. A group with no members has a Start button that means nothing.
- **Decision: `group.*` message keys, not the planned `dialog.group*`.** The plan proposed
  putting the modal's strings under `dialog.`, which would have split one feature's
  vocabulary across two prefixes for no gain. Everything the feature says now lives under
  `group.`, in both `enMessages` and `zhCnMessages`; `zhCnMessages` is typed
  `Record<MessageKey, Message>`, so a missing or extra key fails `typecheck` rather than
  shipping a blank label.
- **Decision: `busyGroups` is a `Map<string, GroupAction>`, not a `Set`.** The button has to
  say which action is in flight — "Starting…" and "Stopping…" are different labels on
  different buttons — so membership alone is not enough information. The per-profile busy
  set is then *derived*: `effectiveBusyProfileIds` unions the real `busyProfileIds` with
  every member of every busy group, so a group action disables its members' individual
  buttons without a second source of truth that could get out of step.
- **A real defect was found in passing and fixed: `save_project` rewound `updated_at`.** The
  insert read `VALUES (?1, ?2, ?3, ?4, ?4)`, binding `created_at` into both timestamp
  columns, so every new project's `updated_at` was its creation time and the column was
  wrong from the first write. It is `?5` with `now` bound separately
  (`storage.rs:63-66`). Unrelated to launch groups; found while reading the upsert that
  `save_launch_group` was modeled on.
- **`LaunchGroupModal`'s inputs have programmatic names; `ProjectModal`'s still do not.**
  The modal's name field uses `id="group-name-label"` plus `aria-labelledby`, because a
  label that is only visually adjacent leaves the input nameless to a screen reader.
  `ProjectModal.tsx` has the same pattern at roughly thirteen sites and was **left
  unfixed on purpose**: it is a pre-existing defect in a file this feature does not touch,
  and folding a thirteen-site accessibility change into this diff would make both harder to
  review. The new `.field-error` rule in `styles.css` does incidentally style
  `ProjectModal`'s existing error text, which is a cosmetic improvement, not the fix.
- **The e2e overflow assertion had to be rewritten, and the reason generalizes: `scrollWidth`
  cannot measure overflow anywhere in this app.** Four `responsive.spec.ts` failures were my
  own instrument, not the layout. Every `IconButton` renders its tooltip as an absolutely
  positioned `::after` pseudo-element that is invisible until hover and up to 220px wide;
  `scrollWidth` includes it, so any row containing an icon button reports overflow it does
  not have. `expectRowFitsViewport` (`responsive.spec.ts:41`) measures the right edge of the
  row's real buttons against the viewport instead. No assertion was weakened — the new one
  fails on real overflow and the old one failed on a tooltip. Note also that
  `.restore-sequence` is a horizontal scroll container by design, so overflow *inside* it is
  correct behavior.
- **No `V0.4.0_PLAN.md` was created.** `V0.2.1_PLAN.md` and `V0.3.0_PLAN.md` are precedent,
  but P3's goal is to get agent-facing documents out of the repository, so adding a third
  would push against the direction already chosen. The plan stayed in the session plan file;
  the durable record is this section plus the `HANDOFF.md` checkpoint.
- **Verification, run before any number was written into a document.** Root crate: fmt ok,
  clippy clean under `-D warnings`, `38 passed; 0 failed` across five targets — unchanged,
  since the root crate has no part in this feature. Desktop crate in
  `apps/desktop/src-tauri`: fmt ok, clippy clean in 56.30s, `cargo test --all-targets`
  `250 passed; 0 failed; 1 ignored` (from 240), including the environment-dependent
  `external_termination_with_verified_identity_stops_tree_and_releases_port`, which passed
  through its `Ok` path so its P1-3 guard is still unexercised. Frontend in `apps/desktop`:
  lint and typecheck clean, `npm test -- --run` `26 passed (26)` files /
  `208 passed (208)` tests in 21.70s (from 23 / 171), `npm run build` JS 335.03 kB / CSS
  38.32 kB, `npm run e2e` `7 passed (18.8s)` (from 6), `npm run tauri build` exit 0 —
  `1m 37s` first, then `56s` on the rebuild that restored the production identifier.
- **Migration evidence, on a throwaway bundle identifier, with the real database never
  opened.** The desktop tests build their fixtures in memory, so only this exercises an
  install path. A pinned version 2 database — `V1_SCHEMA` + `V1_FIXTURE` + `V2_ADDITION`
  copied out of `storage.rs`, plus one `complete` archive row, `user_version=2`, eight
  tables — was staged into `%LOCALAPPDATA%\com.abysswhale.runcove.demo0831\`, opened once by
  a build whose identifier and instance mutex both carried `demo0831`, and afterwards read
  back: `user_version=3`, ten tables, both new tables matching the pinned DDL including
  `COLLATE NOCASE`, `integrity_check` ok, `foreign_key_check` ok, and every fixture row
  intact down to the restore set's order. A group whose members were deliberately out of
  profile sort order then survived a second 14-second session unchanged, with nothing on
  stderr. `%LOCALAPPDATA%\com.abysswhale.runcove\runcove.sqlite3` is still `user_version=1`,
  last written 2026-08-11, read from its SQLite header without opening the file.
- **`INSTANCE_MUTEX_NAME` is hardcoded, so changing the bundle identifier alone does not
  isolate a build.** `lib.rs:42` is a literal and `single_instance.rs:61` derives the wake
  event as `{name}.Wake`, so a demo build that changed only the identifier would still share
  the guard with production: launching it would wake the running RunCove and start nothing,
  and the experiment would measure a process that never ran. Both were changed for the demo
  build and both were reverted; `git grep demo0831` finds nothing.
- **A verification trap that silently produced a false pass.** The first attempt seeded the
  database with Microsoft Store Python, whose writes under `%LOCALAPPDATA%` are redirected
  into `%LOCALAPPDATA%\Packages\PythonSoftwareFoundation.Python.3.13_*\LocalCache\Local\`.
  The app found no database, created a fresh version 3 one, and every reading came back
  "correct" while the version 2 → 3 upgrade had never run. Two files with the same path
  existed and Python and PowerShell each saw a different one. Rule for next time: stage
  anything under `%LOCALAPPDATA%` with PowerShell, and let Python work on a copy under
  `D:\tmp`.
- **Three post-run differences in the isolated database are correct behavior, not data
  loss.** The session left `running` by the previous kill became `interrupted`, which is
  startup reconciliation; `archiveRunLogs` appeared in the settings JSON, which is the
  `#[serde(default)]` round-trip; and `languagePreference` went from `zh-CN` to `system`
  because `App.tsx:389-393` deliberately lets WebView `localStorage` win over the database
  when they disagree, and an earlier run in that throwaway WebView profile had stored
  `system`. On a real install the two agree, so this is an artifact of reusing one WebView
  profile across differently-seeded databases.
- **The one-way upgrade is now stated in `README.md` in both languages**, together with the
  recommendation to back up `%LOCALAPPDATA%\com.abysswhale.runcove\` before running a `main`
  build for the first time. A failed upgrade rolls back and stays at version 2; a successful
  one cannot be undone, and v0.3.0 then refuses the database as newer than it supports.

## 2026-08-30 P1-4 Decision: Green Across The Matrix, `0.3.1` Held Back For `0.4.0`

- **Verification, run from the repository root unless noted.** Root crate: fmt ok, clippy
  clean under `-D warnings`, `cargo test --all-targets` `38 passed; 0 failed` across five
  targets (`12 + 0 + 0 + 10 + 16`). Desktop crate in `apps/desktop/src-tauri`: fmt ok,
  clippy clean, `240 passed; 0 failed; 1 ignored`. Frontend in `apps/desktop`: lint and
  typecheck clean, `npm test -- --run` `23 passed (23)` files / `171 passed (171)` tests,
  `npm run build` 1607 modules in 1.73s, `npm run e2e` `6 passed (18.5s)`,
  `npm run tauri build` exit 0 with the release profile finishing in `1m 55s`.
- **`tauri build` producing no installer is the configured behavior, not a failure.**
  `tauri.conf.json` sets `bundle.active: false`, so the command builds
  `target/release/runcove-desktop.exe` and stops; packaging belongs to the release
  workflow. That exe was rebuilt and not launched — it carries the production identifier.
- **A defect in `AGENTS.md`'s matrix was found while following it.** It listed the three
  `cargo` commands once, at the root. The root package is not a workspace
  (`Cargo.toml` has no `[workspace]`), so those commands never reach `runcove-desktop`:
  following the recipe literally skipped 240 tests plus that crate's fmt and clippy.
  `AGENTS.md` now carries both Rust blocks and states the reason, so the omission cannot
  be read as intentional. The enforced gate was never weaker — `ci.yml:110-119` already
  runs all three commands in `apps/desktop/src-tauri` — so this was a documentation
  defect in the local completion gate, not a hole in CI.
- **Decision: the next version is `0.3.1`, and it should not be released on its own.**
  The delta since `v0.3.0` is a single user-visible bug fix (RunCove's own lifecycle
  sentences showing in English under a Chinese interface) with no feature and no break;
  the IPC `reason` field is additive and optional, and 0.x still gets a patch bump for
  that. Holding it: a release costs a tag, a CI run, and published artifacts, and `0.4.0`
  is already reserved for the P2 feature, which this fix can ride with. `CHANGELOG.md`'s
  `[Unreleased]` section accumulates either way, so nothing is lost if the user decides to
  ship the patch sooner.
- **No version file was edited.** A bump would touch root `Cargo.toml`,
  `apps/desktop/src-tauri/Cargo.toml`, `tauri.conf.json`, `apps/desktop/package.json`, and
  `package-lock.json` (lines 3 and 9), plus the two cargo lockfiles — all four manifests
  currently read `0.3.0`. A bump is only meaningful with a release, and release is
  unauthorized, so the number is a recommendation and nothing on disk asserts it.

## 2026-08-30 P1-3 Decision: The Environment's Refusal Is Not The Test's Verdict

- **Decision: a runtime guard on the refusal, not `#[ignore]`.**
  `external_termination_with_verified_identity_stops_tree_and_releases_port`
  (`commands.rs:1830`) is the only test that performs a real termination, and it fails
  intermittently on this machine with Windows `Access denied` while passing on CI. It now
  treats a refusal from `taskkill` as an environment refusal — reported through
  `eprintln!`, then `return` — and keeps failing on every error RunCove decides for
  itself. Test code only; no production behavior changed.
- **Why not `#[ignore]`, which is what the other live test uses.**
  `live_imports_detect_conflicts_without_touching_existing_processes`
  (`commands.rs:1884`) needs live services configured by hand and can never run
  unattended. This one builds its own fixture and works on CI, so `#[ignore]` would
  remove the only check of the successful path everywhere — CI does not run
  `-- --ignored` — to quiet one machine. That is coverage loss disguised as a guard.
- **Why the predicate matches RunCove's wrapper and not the OS reason.**
  `termination_refused_by_environment` (`commands.rs:1822`) tests for the prefix
  `"Could not terminate process tree:"` (`commands.rs:1050`). `taskkill` prints in the
  system language and its stderr reaches that message through `from_utf8_lossy`, so on a
  non-English Windows — this one is Windows 11 Home China — matching `Access is denied`
  would be either a translation miss or mojibake. The prefix is narrow by enumeration:
  every other `Err` in `terminate_external_windows` is RunCove's own verdict (changed
  identity, changed executable, managed process, missing `taskkill.exe`, refusing itself,
  an unreadable handle) and none of them begins with it.
- **Why `taskkill`'s own failure is environmental rather than a product defect.** Its
  `/T` walk reports `Access is denied.` for a child that exited between enumeration and
  termination, or for one security software stands in front of, and it returns a failing
  status even when the root is already gone. RunCove reporting that failure is correct —
  judging success by "the root looks dead" instead would risk leaving children behind, so
  the process-safety model stays as it is and the guard belongs in the test.
- **The guard is unexercised.** The flake did not reproduce: 10 consecutive dedicated
  runs plus a parallel and a single-threaded full suite all took the `Ok` path. Say it
  that way rather than claiming the flake is fixed — the next `Access denied` on any
  machine is the first real exercise, and it will show as a pass with a `--nocapture`
  line.
- Verification: desktop crate `240 passed; 0 failed; 1 ignored`, identical in parallel
  and single-threaded; `fmt --check` and `clippy --all-targets --all-features -D warnings`
  clean. Test count unchanged, since a body changed and no test was added.

## 2026-08-30 P1-2 Decision: The Line-Count Over-Report Is Accepted, Not Fixed

- **Decision: close P2-1 as a disclosed limitation.** When the *closing* flush itself
  fails, `line_count` can name a line the file does not hold. The reasoning now lives
  at `return_file` (`archive.rs:2618-2636`), where the defect is, so nobody re-derives
  it. No production behavior and no test changed — this item was a decision, not an
  implementation.
- **Why it is tolerable.** Nothing decides on `line_count`: the quota and eviction read
  `byte_size` and the timestamps, and `byte_size` is already re-measured from the file
  on exactly this path (both close sites, `archive.rs:2728` and `:2955`). The viewer
  already treats the row's counters as possibly stale and pages from the file itself
  (`models.rs:415-417`). The error is always an over-count, never a silent loss, and the
  same close already labels the row `partial` / `write-error`, so the user is told the
  archive is broken before they read any number on it.
- **Why "count at flush boundaries" — the plan's own option — does not work.** A flush
  can also go out *partially*. Saying which buffered lines survived needs every buffered
  line's byte length, kept for every open session on every write, to describe a disk that
  has already failed. This was already the code's stated reason and it still holds.
- **Why recounting the file does not work either — the finding that decided it.** The
  close already measures that file, so a recount looks nearly free. But the two counts
  would then disagree about identical bytes: `Sweep::count_lines` (`archive.rs:1311`)
  uses `text.lines().count()`, which counts a trailing fragment as a line, while a short
  write here charges the fragment to `dropped_lines` and reports `line_count` 0 — pinned
  by `"a fragment is not a line"` (`archive.rs:4988`). The same file would read one line
  longer after a crash than after a close. Exactness therefore means unifying that
  definition across the sweep, the writer, and their tests: re-opening a settled contract
  for a display-only number. Two smaller costs point the same way — `read_to_string`
  fails on a flush truncated mid-UTF-8 and would fall back to the same over-count, and
  the read is up to 10 MiB per session at close time, including `close_all` at shutdown,
  on a disk that has just failed.
- **What the decision obliges.** The limitation stays disclosed. `CHANGELOG.md` gained an
  `[Unreleased]` section that carries it forward and records P1-1's fix; the published
  `[0.3.0]` entry is left exactly as it shipped, including its now-superseded English-
  message limitation, because a released section is a historical record.

## 2026-08-30 P1-1 Decisions: Lifecycle Reasons Cross The Wire As Values

- **The frontend translates, the backend does not.** RunCove's own lifecycle
  sentences were being composed in Rust and shown verbatim in a Chinese window.
  The fix sends `RunStatusReason` — an internally-tagged enum whose `kind` is
  kebab case — on both `RunStatusEvent` and `RunLogEvent`, and translates it in the
  existing i18n catalog. The alternative, passing the window's language down to the
  backend, was rejected: it would put copy in two places and make every emit site
  language-aware.
- **Additive, not a replacement.** `reason` is optional and skipped when absent;
  `message` and `line` keep the English sentence from `RunStatusReason::describe`.
  So an unrecognized `kind` — a newer backend, or a payload nothing validated —
  falls back to readable English instead of rendering nothing. This is why
  `types.ts` declares `kind: string` rather than a union of literals.
- **Scope is RunCove's own sentences only.** The eight converted strings are the six
  `watch_child` exit arms and `commands.rs`'s `"Stop requested"` and
  `"Profile is already running"`. `AppError` text — the port-conflict message and the
  `error.to_string()` start failures — is untouched: it has no fixed enumeration, it
  is already framed by `t("error.lifecycleDetail", …)`, and the conflict wording also
  travels as the command's `Err`. Localizing it is a separate, much larger job.
- **Archived lifecycle lines stay English.** The archive stores `line`, and
  `encode_record` writes exactly three keys (`{t,s,l}`) at schema version 2. Adding a
  reason to the file would be a format change for a cosmetic gain, so the archive
  viewer shows what was written. The live drawer shows the localized form because it
  has the event.
- **`logKey` still hashes `line`.** The dedupe that merges history with live output
  must compare the stable wire text, not a rendering that changes with the language.
- Tests were written on the defect path only, per the new policy: the bilingual
  mapping with its two fallbacks, a zh-CN notice, a zh-CN failure alert, a zh-CN log
  line whose clipboard copy matches the screen, and — in Rust — the kebab-case `kind`
  and the omitted-field shape, because a rename there would quietly restore English.
- Verification: root crate `38 passed` across six test targets; desktop crate
  `240 passed; 0 failed;
  1 ignored`; `fmt --check` and `clippy --all-targets --all-features -D warnings`
  clean in both crates; frontend `lint`, `typecheck`, `test -- --run`
  (23 files / 171 tests), `build`, `e2e` (6) green. `npm run tauri build` is left to
  P1-4's full matrix. `external_termination_with_verified_identity_stops_tree_and_releases_port`
  passed in this run, so P1-3's guard is still needed — the failure is intermittent
  on this machine, not gone.

## 2026-08-30 Decisions: 软著 Dropped, Product Work Resumes

- The software copyright registration goal is abandoned. Reported policy since
  2026-03 is that 中国版权保护中心 refuses applications built on AI-generated
  code, with 失信名单 / 个人征信 consequences, and that 2026 review adds
  AI-material screening and code-similarity comparison. RunCove's code is
  substantially agent-written and its public repository documents that, so the
  application path is closed. Removing AI traces to make it pass was declined and
  is not a task anyone should revive. The official text could not be fetched from
  this environment, so this is consistent secondary reporting; confirm at
  ccopyright.com.cn if the question ever matters again.
- The AI-trace audit that preceded the decision is worth keeping because it is
  measured: production and test source carry zero AI-vendor references — every
  `git grep` hit under `apps/desktop/src` and `apps/desktop/e2e` is the user's own
  `D:\CodexProject\personal-projects` path in fixtures. The references live in
  `HANDOFF.md` (21), `notes.md` (12), `V0.3.0_PLAN.md` (1), and in two commit
  bodies carrying `🤖 Generated with [Qoder]` plus `[codex]` in the `bd2b777`
  merge body.
- Testing process changed: the slice-by-slice red-to-green method used for the
  v0.3.0 archive is retired. Tests follow real defect paths and regression risk
  from now on. The rule against weakening, retargeting, or deleting an assertion
  to reach green is unchanged.
- The next plan is in `HANDOFF.md`, not here: P1 defect closure, P2 one v0.4.0
  feature awaiting the user's pick, P3 authorization-gated housekeeping.

## 2026-08-22 v0.3.0 Publication Checkpoint

- PR #3 was merged and the public `v0.3.0` release was published successfully.
  `main`, `origin/main`, and the annotated tag resolve to
  `bd2b7776d56ddf750ffe97a3d8219168fbb04069`.
- Release workflow `32500425361` passed all validation, cross-platform CLI,
  Windows desktop, packaging, checksum, and publish jobs. The five downloaded
  binary archives matched the published `SHA256SUMS.txt` entries by SHA-256.
  No workflow file was changed.
- The post-restart repository check is clean. Local scratch verification lives
  outside the repository in `D:\tmp`; no build output or real application
  database was introduced into the tree.
- Local verification caveats remain recorded rather than hidden: the managed
  session saw Windows `Access denied` in the real-process termination test and
  `spawn EPERM` when trying to start Node children. GitHub's clean Windows job
  passed the complete release matrix, including six E2E flows and `tauri build`.
- Decision: freeze v0.3.0 for exploratory use. The next change should be a new
  user-approved product scope, not a cleanup refactor or soft-copyright padding.

## 2026-08-21 v0.3.0 Release Decision

- The user explicitly authorized commit, push, tag, and public release after the
  isolated local demo. Release readiness is defined as no known P0/P1, the complete
  local matrix green, GitHub CI green on the release branch, and the existing tag
  workflow producing all expected archives plus `SHA256SUMS.txt`.
- The release keeps two disclosed P2 limitations: a closing flush failure can
  over-count which buffered line reached disk while byte size is reconciled, and
  backend-composed stop/exit messages remain English under the Chinese interface.
  Neither affects the normal archive path, process safety, or local-only boundary.
- Tool-specific `CLAUDE.md` remains local. `AGENTS.md` is restored to its published
  form and no CI, release-workflow, `.env`, real database, other project, or existing
  process is modified as part of the release preparation.
- The final release commit is intentionally one cohesive 43-path change: the frozen
  36-path implementation/demo candidate plus seven manifest, lockfile, and public
  release-document paths. Generated build output and local `CLAUDE.md` are excluded.
- The official npm audit caught high-severity advisory `GHSA-2v37-7h3g-55p8` in the
  transitive development dependency `nanoid` 3.3.17. A package-lock-only update to
  3.3.18 followed by `npm ci` reduced the official-registry audit to zero findings; no
  direct dependency was added. The configured mirror's audit endpoint returns 404, so
  release evidence uses `registry.npmjs.org` explicitly.
- The fresh local release run is honest but not wholly green in this managed Codex
  session. Root Rust (38), frontend lint/typecheck/Vitest (157), archive (99), and
  archive-service (11) checks pass. The desktop run is 237 pass / 1 fail / 1 ignored,
  with only the known real-process termination test failing because Windows returned
  `Access denied`. Node child-process creation is denied with `spawn EPERM`, preventing
  a fresh Playwright and Vite/Tauri rerun even though both passed on the frozen tree the
  previous day. Decision: do not change product code or tests for environment behavior;
  require every unchanged GitHub PR check to pass before merge and tag.
- PR #3 exposed a toolchain-drift lint, not a behavior failure: Rust 1.98 rejects the
  test-only expression `format!("{uuid}")` as `clippy::useless_format`, while the local
  stable toolchain accepted it. Replace it with `uuid.to_string()`; test inputs and
  assertions are unchanged. The same CI run already proved npm audit, frontend build,
  and all six Playwright workflows on the clean Windows runner.

## 2026-08-20 v0.3.0 Freeze: The Wrap-Up Decisions

- Scope: documentation and verification only — `README.md`, `HANDOFF.md`, `notes.md`, and
  one pointer bullet in `CLAUDE.md`.
  **No production or test source file changed**, so every decision recorded below this
  section still stands as written. Untouched: CI, the release workflow, tags, `.env`,
  every real database, unrelated projects, and existing developer processes. No commit,
  no push.
- **Decision: the README states the archive as a `main`-only, off-by-default exception
  rather than restating v0.2.1's promise.** The contradiction was not one sentence but a
  premise — v0.2.1's public text says session output is never written to disk, and that
  is still true of the published zip. Six places now name the boundary explicitly: the
  log-boundary bullet, the Desktop App and Help bullets, the new **Run Log Archive**
  section, Architecture's index row and one-way schema step, Privacy And Process Safety,
  and the v0.2.1 Scope exclusion. Wording rule for anything written next: the published
  v0.2.1 build has no archive at all; on `main` it exists, stays off until the user turns
  it on, and only affects runs started after that.
- **Decision: both P2s stay open under the freeze, and neither is a silent risk.**
  `line_count` over-counting after a failed *closing* flush is documented at its cause
  (`archive.rs:2624-2630`) and can only ever overstate what was written, while the byte
  side self-corrects from the disk. The English stop and exit messages
  (`processes.rs:562`, `:580`) are a localization gap in strings the backend composes,
  not a behavior defect. Fixing either would mean new production code, which the freeze
  excludes.
- **Finding, not a defect: a per-session `begin` failure cannot reach the drawer's
  warning.** `unavailable_reason` short-circuits to `None` while the writer exists, so
  that warning describes *this run's initialization* and nothing else; a session that
  fails to open is reported through the transient `ArchiveReporter` channel and surfaces
  as a 未归档 badge. That is the honest reading of `enabled` versus `available`
  (`models.rs:324-335`), and it means the initialization-failure notice is reproducible
  only by obstructing the directory **before** launch. Recorded rather than changed:
  making a per-session failure sticky in the drawer is a product decision, not a wrap-up
  item.
- **Method: the initialization-failure proof deleted nothing, twice over.** The archive
  directory was renamed aside and a 15-byte file left in its place; undoing it *moved*
  the placeholder out to a scratch path instead of removing it, then renamed the
  directory back. All four archives were then verified byte-identical by SHA256 (4 files
  / 682,845 bytes). Keep this shape for any future destructive-looking proof: rename and
  move, never delete, and re-verify by hash rather than by size.
- Verification at the freeze, all green: root crate fmt and clippy clean with 38 tests;
  desktop crate fmt and clippy clean with `238 passed; 0 failed; 1 ignored` out of 239;
  `npm run lint`, `npm run typecheck`, `npm test -- --run` at 22 files / 157 tests,
  `npm run build`, `npm run e2e` at 6 passed, and `npm run tauri build`. The manual demo
  checklist was re-run end to end on the frozen build; its measurements — paging to
  「已到归档开头」 at 2,261 records, the 150,054-byte delete credit measured on disk, the
  instant toggle-off finalization with zero `"s":"system"` records — and the
  criterion-7 nuance are in the top section of `HANDOFF.md`.
- Carried forward: `apps/desktop/src-tauri/target/release/runcove-desktop.exe` is a
  production-identifier build after that last `tauri build` and must not be launched —
  the demo exe is `D:\tmp\runcove-v030-demo\RunCove-demo.exe`. Two `commands.rs`
  port/child-timing tests have each failed once on some machine, so no cross-machine
  all-green baseline may be claimed. `commit`, `push`, `tag`, CI, and Release remain
  unauthorized.

## 2026-08-19 v0.3.0 Local Demo Candidate: The Archive Has A Runtime Caller

- Scope: `apps/desktop/src-tauri/src/` — `archive_service.rs`, `commands.rs`, `lib.rs`,
  `models.rs`, `processes.rs`, `state.rs`, `storage.rs` — plus the frontend
  (`App.tsx`, `api.ts`, `types.ts`, `mock-data.ts`, `styles.css`,
  `components/{LogDrawer,RunLogArchiveDrawer,RunHistory*,OverviewView}`,
  `components/archive.ts`, `i18n/messages.ts`) and the docs. No schema change: the
  version 2 migration was already in place and `SCHEMA_VERSION` is untouched.
  Untouched: CI, the release workflow, tags, `.env`, and every real database. No
  commit, no push.
- **Decision: `update_counters` is thinned, and nothing else is.**
  `ThrottledArchiveIndex` (`archive_service.rs:103`) lets a session's *first* refresh
  through and then requires 4 s or 1 MiB, whichever comes first. A pump batch can be a
  single line, so untouched this turns a chatty child process into one SQLite write per
  printed line. What makes it safe is that the row a reader finally sees is written by
  `ArchiveIndex::close`; all that is thinned is how fast a *running* row catches up with
  its file. It lives in the service rather than in `archive.rs` on purpose — the
  writer's contract is that it refreshes after every batch and its tests assert exactly
  that, while "often enough for a user watching a row" is an application decision. A
  clock that has gone backwards is deliberately *not* thinned.
- **Decision: shutdown is bounded by the queue's caps, not by a clock.** `shutdown`
  stops the pump signal, then closes every open archive as `Interrupted`. One pump
  drains everything queued and what can be queued is capped by `QueueBounds`, so there
  is no timeout to tune and no file half-written by one. `close_open_archives` pumps
  first so the bytes it can account for pass through the quota — a close does not
  consult it — and a failed pump is reported without stopping the closes, because an
  unwritable disk is exactly when the rows matter.
- **Decision: the setting is persisted before it is applied.** `persist_run_log_archiving`
  (`commands.rs:538`) writes `AppSettings` and only then calls `set_enabled`, so a
  database that refuses the write leaves the runtime and the stored value agreeing
  instead of archiving with nothing to remember it by.
- **Decision: a read never crosses IPC whole.** `read_run_log_archive` is `async` +
  `run_blocking`, seeks to the end, and walks backwards under *both* a record cap and a
  byte cap (`MIN_PAGE_RECORDS = 1`, `DEFAULT_PAGE_RECORDS = 500`,
  `MAX_PAGE_RECORDS = 2_000`, `archive.rs:79-85`). The viewer's cursor is the previous
  page's `page_start_offset`, so "load earlier" is a bounded backwards walk rather than
  a re-read of the file.
- **Decision: turning the setting on never backfills.** A session's archive is opened at
  launch and only there, so `set_enabled(true)` mid-run leaves the running session
  unarchived — proven on screen, not merely intended. Turning it off closes what is open
  as `partial` / `user-disabled` while the process keeps streaming to the in-memory
  drawer.
- **Two closes, two different reload problems.** The exit path emits
  `run-archive-closed` (`processes.rs:675`) because the reload the exit event itself
  triggers still sees `writing` — the close finishes its writes after the lock that
  event is emitted under is released. The toggle path emits nothing at all
  (`close_open_archives` announces no rows), so `App.tsx`'s `toggleRunLogArchiving`
  awaits a history reload after `setRunLogArchiving` resolves; that is deterministic
  only because the command is `async` + `run_blocking`, which makes every affected
  archive final on disk before the promise settles. Both were caught on screen. The
  second guard was checked for teeth by breaking it and watching
  `App.history.test.tsx` fail, then restoring it.
- **`finalizing` is a frontend inference, not a wire status.** `components/archive.ts`
  derives it from a `writing` row whose session already has `endedAt`, which is the one
  state the backend cannot name — the row is still open while the writer is closing it.
  `canViewArchive` excludes `none`/`removed`; `canDeleteArchive` allows
  `complete`/`partial`/`unknown`.
- Verification: root crate fmt and clippy clean with 38 tests; desktop crate fmt and
  clippy clean with `238 passed; 0 failed; 1 ignored` out of 239 for
  `cargo test --all-targets` (99 `archive`, 31 `storage`, 26 `commands`, 17 `state`,
  13 `processes`, 12 `discovery`, 11 `archive_service`, 7 each `import_observation` /
  `models` / `tests`, 3 each `language` / `single_instance`, 2 `privileges`, 1 `error`);
  `npm run lint`, `npm run typecheck`, `npm test -- --run` at 22 files / 157 tests
  (29 of them the two new archive files), `npm run build`, `npm run e2e` at 6 passed,
  and `npm run tauri build`. All green.
- Local acceptance: all seven criteria proven on an isolated demo build
  (`com.abysswhale.runcove.demo0819`), with no production RunCove running, no real
  database opened, and no existing developer process touched. Evidence and the
  reproduction path are in the top section of `HANDOFF.md`.
- **The initialization-failure case was proven without deleting anything.**
  `run-log-archives` was *renamed* aside and a 15-byte file left in its place, so the
  failure was real (`os error 183`) while every archived byte stayed on disk. Port
  scanning, project launch, and the in-memory drawer were all unaffected; the
  placeholder never grew, and both archives measured byte-identical afterwards.
- Residual limitations, recorded and not blocking the demo. **P2:** `line_count` can
  over-count a session whose *closing* flush failed — a small record is counted when
  `write_all` returns and a partial flush cannot be attributed per line; the byte side
  self-corrects from the disk and the normal path is unaffected. **P2:** the
  process-exit toast reads English ("Process exited normally") under a Chinese UI,
  because that string is composed in the backend. **Measurement trap:** on NTFS,
  `Get-ChildItem` reports `Length = 0` for an archive whose writer handle is open — the
  directory entry is stale until the handle closes, so an open archive must be measured
  through an opened handle, which is what RunCove's own reader does.

## 2026-08-18 v0.3.0 Writer Slice C: The Caller-Facing Close

- Scope: `apps/desktop/src-tauri/src/archive.rs` only — `ArchiveWriter::close`
  (`archive.rs:2567`) and `close_all` (`:2694`), the four private helpers they are built
  from (`begin_close` `:2729`, `write_taken` `:2773`, `write_residual_gap` `:2806`,
  `append` `:2837`), the `ClosingSession` struct (`:1655`), the `worsened` fold
  (`:1680`), and three behavior-preserving refactors (`take_session` `:1417`,
  `pending_write` `:1625`, `next_write` `:2093`). **No `todo!` remains in the module.**
  Untouched: the `Storage`-backed `ArchiveIndex`, the commands, the frontend, CI, the
  release workflow, tags, `.env`, and any real database. No commit, no push. `lib.rs`
  unchanged, so the feature still has no runtime caller.
- **Decision: the close holds `pump_lock`, and that is not optional.** The pump borrows
  the session's handle out of its slot for the length of a write. A close that did not
  serialize against it could reach `begin_close` mid-write, find `slot.file == None`,
  and silently write nothing — every record it had accepted lost with a `complete` row
  over an incomplete file. The lock order is `pump_lock → open → queue → total`.
- **Decision: the boundary marks and extracts in the same critical section.** Marking
  `Closing` tells `enqueue` to refuse; taking the session's records tells the close what
  it owes. Doing them separately leaves a window in which a record is neither refused
  nor written — stranded in the queue, blocking `finish_session`, holding room. So
  `begin_close` does both under the open lock and the queue lock, and does all the file
  work after releasing them.
- **Decision: a refusal is inert, and that is the whole of the double-close rule.** A
  slot that is missing, `Opening`, or `Closing` is refused with a per-state message and
  nothing else happens: no file, no counter, no `index` call. So a second close cannot
  overwrite the first one's row, and `close_all` with nothing open is `Ok` and equally
  inert.
- **Decision: the first write failure ends the session, and everything behind it is
  charged.** A handle that has refused bytes will refuse them again, so retrying inside
  a close would only stall it. The remaining records are `discard`ed — charged to the
  row's drop counters — rather than dropped silently, because a line that left a capture
  thread has to end up in a counter somewhere.
- **Decision: the residual gap belongs to *this* close and to no other.** The trailing
  run of losses is written as one `LogStream::System` line whose text is `gap_line(gap)`
  and whose timestamp is `ended_at`, then counted into `line_count` and charged to the
  quota. `writer_close` still writes none: it is closing *because* the disk refused a
  write or the cap refused a byte, and such a file must not be asked for one more line.
  The distinction is the file's state, not a change of policy.
- **Decision: a close does not consult the quota.** Its records were accepted while the
  session was open, and its linearization point is already behind it, so a refusal would
  have nowhere to go but the drop counters of a session that did nothing wrong. Worse,
  `room_for` can fail *retryably* through eviction, and a close cannot retry. The
  overshoot is bounded — one session's queued bytes plus one gap line — and every byte
  is still charged, so the next `pump` and the next sweep both see the truth.
- **Decision: the verdict accumulates.** `worsened` folds `QueueOverflow` when the drop
  counters are non-zero and `WriteError` when the file refused or could not be synced, on
  top of the caller's `reason`; `Complete` iff nothing folded. `most_severe` is a total
  order, so the result is fold-order independent — a clean `close_all(UserDisabled)`
  still reports `user-disabled`, and a session that also lost lines reports the worse of
  the two.
- **Decision: the slot is removed before the row is written.** An index failure then
  returns `Err` over durable bytes and a `writing` row — precisely what the startup
  sweep repairs to `partial` / `interrupted` — instead of leaving a session that is open,
  unclosable, and undeletable. The user-visible consequence is that a failed close is
  retryable by restarting, never by closing again.
- **Decision: `finish_session`'s `Err` is defaulted rather than propagated, and the
  comment says why.** It refuses only while the session still has queued records, and the
  boundary took every one of them, so it is unreachable; returning there would leave the
  slot `Closing` forever.
- **One test added, `a_close_writes_the_trailing_gap_no_later_record_could_carry`.** The
  14 red tests never reach the residual gap — every existing gap test has a following
  record that carries it — so the acceptance criterion "residual gap correct" had nothing
  behind it. The new test drops two records with nothing after them and pins the file's
  last line as the gap, its `t` as the close's `ended_at`, `line_count == 3`,
  `byte_size == text.len()`, the row's drop counters, `partial` / `queue-overflow`, and
  an empty queue. It uses only existing helpers: no new seam, and no production change
  was needed to make it pass.
- **Residual limitation, carried at P2 by the user's instruction.** A small record is
  buffered, counted into `line_count` when `write_all` returns, and can still be lost if
  the closing flush fails; the byte side self-corrects from the disk, so the error is
  only ever an over-count on a failed-disk path. No design was expanded for it this
  round, the normal path is unaffected, and it is documented on `return_file`.
- Verification in `apps/desktop/src-tauri`: each of the 14 C tests alone (all 14 pass),
  then the new test alone, then the archive suite **83 passed; 0 failed**, then
  `cargo test --lib` **`202 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out`**
  out of 203, identical under `-- --test-threads=1`, `cargo fmt --all -- --check` clean,
  `cargo clippy --all-targets --all-features -- -D warnings` clean with no `#[allow]`
  added. The first `--all-targets --all-features` run failed
  `commands::tests::manual_start_stays_starting_until_managed_expected_port_is_ready`
  (`commands.rs:2108`, the child bound its port before the check that it had not yet);
  it passed alone three times and the whole matrix passed on retry. That test was neither
  read for this purpose nor modified, and **no stable full-suite all-green baseline is
  claimed**.

## 2026-08-18 v0.3.0 Writer Slice B: The Steady-State Write Path

- Scope: `apps/desktop/src-tauri/src/archive.rs` only — the queue's six settle bodies
  with the reservation accounting, `ArchiveWriter::enqueue` and `pump`, the `Closing`
  state and the private writer-initiated close, and the quota's eviction. Ten `todo!`
  bodies became **two**: the caller-facing `close` (`archive.rs:2506`) and `close_all`
  (`:2513`), which are slice C. Untouched: the `Storage`-backed `ArchiveIndex`, the
  commands, the frontend, CI, the release workflow, tags, `.env`, and any real
  database. No commit, no push. `lib.rs` unchanged, so the feature still has no
  runtime caller.
- **Decision: a record for a session that is not `Open` is ignored, not charged.** The
  queue's bound refusals are charged as before, but a record whose session is
  `Closing` or gone is dropped without a counter, because the drop counters live in
  that session's row and a closed session's row is already written — charging it would
  produce a number nobody can ever read. This is the one place where "every line ends
  up in a counter" does not hold, and it holds nowhere else: the three unreachable
  arms inside `pump` (a slot that vanished between two locks, a missing handle, a
  missing slot on the way back) all charge rather than drop.
- **Decision: `enqueue` is hand-over-hand, and `state` is what it reads.** It holds
  the open-session lock only to test `state == SlotState::Open`, takes the queue lock,
  and drops the open lock before touching the queue, so the fixed `open → queue` order
  holds with no window where both are free. It reads `state`, never the presence of
  the file handle, because the pump borrows that handle out of the slot for the length
  of a write — a record arriving mid-write is still `Open` and must be accepted. That
  is what makes `SlotState` load-bearing rather than decorative.
- **Decision: `room_for` takes bytes, not a session id.** The per-session cap is
  checked first because no eviction can relieve it — removing someone else's archive
  does not shrink this session's file — and only then the directory total, in a loop
  that evicts one archive per turn. Neither cap is about *who* is writing, so the
  function takes the two numbers and nothing else. The first draft passed the session
  id through and silenced it with `let _ = session_id;`; that is exactly the bypass
  marker this repo forbids, so the parameter was removed instead of suppressed.
- **Decision: an `Unavailable` total refuses without evicting.** A directory whose
  size could not be measured must not be grown, and it must also not be *emptied* on a
  guess: evicting archives to make room under an unknown total would delete real logs
  to satisfy arithmetic nobody has. So the session closes `partial` /
  `quota-exceeded` and only the next startup sweep can restore a real total.
- **Decision: eviction filters before it sorts, and credits the disk.** Candidates are
  narrowed to `Complete`/`Partial` rows with a non-null `ended_at` that this writer
  does not hold open, and only then ordered by `ended_at`, `started_at`, `session_id`.
  `index.rows()` is read *before* the open-session lock so the lock order survives.
  The file is resolved through `verified_file_name` — a row naming another session's
  file is reported, not skipped, because `file_name` is the single field that decides
  which bytes go — measured, removed, credited at its **measured** length, and only
  then marked `removed` / `quota-evicted`. The credit precedes the row write because
  the bytes are gone whatever the index answers.
- **Decision: the writer-initiated close writes no gap line.** `writer_close` takes
  both halves of the drop history from `finish_session` and deliberately leaves
  `residual_gap` unwritten. A session closing because its disk failed or because the
  quota left it no room cannot be asked to append one more line — the write that would
  carry the gap is the write that just failed, or the byte the quota just refused. So
  the row's `dropped_lines` / `dropped_bytes` are the only surviving record of the
  loss, which is why every writer-close path charges the batch remainder instead of
  discarding it.
- **Decision: the failure order inside a write is handle, charge, close.** `write_one`
  hands the file handle back before charging the record and closes only after, because
  `writer_close`'s critical section calls `discard_session` and then `finish_session`,
  and `finish_session` refuses while that session still has records queued. Charging
  first is what leaves nothing behind for it to refuse over.
- **Decision: `finish_batch` flushes once per session, and a failed flush is that
  session's close.** One `flush` and one `update_counters` per session that actually
  wrote, rather than per record, so a batch of a hundred lines makes one index write.
  A flush that fails becomes `writer_close(WriteError)` for that session and the batch
  continues with the others — a write error is a session-level fact, never a pump
  error, so `pump` still returns `Ok`.
- **Residual limitation, recorded rather than papered over: `line_count` can overstate
  a session whose closing flush failed.** A record small enough to fit the 64 KiB
  `BufWriter` is counted into `line_count` when `write_all` returns, before the bytes
  reach the disk. If the batch's closing flush then fails, the close measures the file
  and corrects `byte_size` and the directory total from disk, but `line_count` can name
  a line the file does not hold. A pending-line counter would not close the gap either,
  because a flush can go out *partially* — saying which buffered lines survived needs
  each line's byte length, i.e. per-line state the writer does not keep. No current
  test covers this case, it is reachable with the existing `fail_write_of` seam, and it
  is documented on `return_file`. The exposure is bounded: at most one buffer's worth
  of lines, in one session, on a failed-disk path, and only in the direction of
  over-counting what was written — no line the pump refused goes uncounted.
- **The authorized test rewrite, and its limit.** `ArchiveQueue::drain` is removed and
  a `settle_all(&mut queue)` helper (`begin_batch`, then `take_front` + `release` until
  empty) replaced it at all six call sites, with `lines_and_gaps` rebuilt on it. In
  `every_short_enqueue_sequence_keeps_the_queues_invariants` the local `drained` became
  `settled` and three assertion messages were reworded ("a settled queue holds
  nothing", "settled in arrival order", "the gap each settled record carries"). That is
  the whole of the permitted change — "drain 后为空" → "settle 后为空". No assertion was
  weakened, retargeted, or removed, and the exhaustive sweep still checks every
  invariant it checked before. `allow_write_of` was not added, as instructed.
- **Two visibilities narrowed, closing the debt CLAUDE.md recorded.**
  `ArchiveWriter::queue` is now a private field read by `enqueue`, `pump`, and
  `writer_close`, and `ArchiveQueue::finish_session` is module-private with
  `writer_close` as its production caller. `ArchiveQueue::peek_front` remains `pub`
  with test-only callers — `next_write` uses the module-private `front()`, which yields
  the whole record rather than a name and a length — and being on a `pub` type, that is
  not `dead_code`.
- Verification in `apps/desktop/src-tauri`: each of the 15 target tests run alone
  first, all 15 green individually, plus the rewritten exhaustive sequence test; then
  `187 passed; 14 failed; 1 ignored; 0 measured; 0 filtered out` out of 202 for the lib
  target, identical in parallel, under `-- --test-threads=1`, and under
  `--all-targets --all-features`; `cargo fmt --all -- --check` exits 0 and
  `cargo clippy --all-targets --all-features -- -D warnings` is clean with no
  `#[allow]` added. The archive suite is 82 tests, **68 green / 14 red**, up from
  53/29 — 172 + 15 = 187, 29 − 15 = 14, so exactly the fifteen turned and nothing
  regressed. All 14 failures are `todo!` panics: 14 `panicked at` lines, 14 `not yet
  implemented` lines, **zero** assertion lines; sites `archive.rs:2506` (`close`) ×12
  and `:2513` (`close_all`) ×2, three of the twelve reported on the close-race seam's
  helper thread. `commands::tests::external_termination_with_verified_identity_stops_tree_and_releases_port`
  passed in all three modes here, but it kills a real process tree and binds a real
  port and failed on the reviewer's machine, so **no stable full-suite all-green
  baseline is claimed** and `commands.rs` was not touched.

## 2026-08-18 v0.3.0 Retry-Contract Red Tests: What A Failed Pump May Not Lose

- Scope: `apps/desktop/src-tauri/src/archive.rs` only, and deliberately red — the five
  corrections the review attached to its refusal of the B/C plan, as seven new tests,
  two new test seams, one shared assertion helper applied to seven existing close
  tests, one test renamed and tightened, and the API surface the retry contract needed
  in order to be stated at all. **No body was implemented.** Untouched: the
  `Storage`-backed `ArchiveIndex`, the commands, the frontend, CI, the release
  workflow, tags, `.env`, and any real database. No commit, no push.
- **Decision: the queue owns every record until it is settled; the pump owns none of
  them.** The earlier sketch — `in_flight: usize` on the queue and a local
  `Vec<QueuedRecord>` inside `pump` — cannot survive its own failure: a `pump` that
  returns `Err` drops that vector and the records with it. So `begin_batch` moves the
  queued records into an in-flight list and frees nothing, `peek_front` and `take_front`
  hand them out without freeing anything, and exactly two calls settle a record —
  `release` (written) and `discard` (lost, and charged). A failed pump therefore needs
  no undo: nothing has to be put back, the next `begin_batch` appends new arrivals
  *behind* what the last one left, and the retry resumes at the same record in the same
  order. `requeue_front` was rejected for the same reason it looked convenient — a
  record leaving the queue and coming back is two chances to reorder or lose it.
- **Decision: the four bounds count reservations, not queue length.** A record whose
  fate is undecided still occupies memory, so `len()`, `is_empty()`, and
  `queued_bytes()` must report queued plus in-flight plus taken-but-unsettled, and
  `enqueue`'s `total_records` test must stop reading `self.queued.len()`. If any bound
  stopped counting an in-flight record, a long run of failing pumps would admit an
  unbounded number of records behind the batch it cannot write, which is the exact
  failure the bounds exist to prevent. Every existing green queue test keeps passing
  unchanged, because with no batch in flight the two readings coincide.
- **Decision: a failed removal is retryable; nothing eligible is terminal.** These are
  different states and the writer must treat them differently — report the first and try
  again on the next tick, close the session `partial` / `quota-exceeded` only for the
  second. On Windows a removal usually fails because someone else holds a handle for a
  moment (a scanner, the indexer), and discarding a session's logs over another file's
  handle is the wrong trade. What makes the retry safe rather than unbounded is the
  reservation accounting above: memory is capped by the bounds however long the
  transient lasts.
- **Finding: only one injectable failure is actually repeatable.** Two obvious
  candidates are not. `index.fail("mark_removed")` *progresses* — the file is removed
  and its bytes credited on the first round, so a second round needs no eviction at all
  — and a write error is *terminal* for its session, which closes and can never fail
  again. Only an eviction whose candidate exists and whose file will not go leaves the
  writer in the state it started in, so sticky `TestFs::fail_remove` is the seam the
  repeated-failure test uses.
- **Decision: `verified_file_name` gates eviction, and a crossed row is reported rather
  than skipped.** `file_name` is the one row field that decides which bytes go, and the
  row is data out of a database this build does not exclusively own, so the ownership
  gate belongs on the eviction path exactly as it does on `read` and `delete`. Skipping
  to the next candidate would make a corrupt row invisible; returning `Err` makes it
  visible, and the writing session's record has to survive that `Err` still queued so
  the retry after the row is repaired writes it exactly once.
- **Decision: after a short write, `byte_size` is the residue the disk actually holds
  and the incomplete record is a drop.** A `write` may legally accept less than it was
  given, so a fragment of a line can be on the disk with the rest gone. Counting the
  fragment's bytes keeps the hard quota from being under-counted; counting the record as
  a dropped line keeps `line_count` honest, because a fragment is not a line. This has a
  consequence B must face: `io::Write::write_all` never reports how much it wrote, so
  the write-error close has to measure the file (or write through a counting loop) rather
  than assume the buffer went in whole.
- **Decision: no `allow_write_of` seam.** A write failure is terminal for its session,
  so nothing in the writer could ever recover that file's writes and no test can
  observe a recovery. An uncalled test helper is `dead_code` under `-D warnings`, and
  an `#[allow]` bypass is not permitted here, so the approved by-name injection landed
  as `fail_write_of` and `short_write_then_fail_of` only. Both inject by file name
  rather than by call count, so a two-session test says *which* session's disk failed
  instead of depending on which one the writer reached first.
- **Decision: the closing boundary is a deterministic refusal.** Because the committed
  close marks a session closing *and* extracts its accepted records in one critical
  section, a record arriving during the file work that follows can only be refused. The
  boundary test's permissive "written or refused" branch was therefore replaced by a
  single equality — a tightening, not a retarget: the outcome it used to allow as a
  second correct answer is the one the design now rules out.
- **Residual limitations.** `every_short_enqueue_sequence_keeps_the_queues_invariants`
  was not converted to the batch protocol this round: a `todo!` at the first sequence's
  `begin_batch` would abort the test before any sequence finished and cost a round of its
  enqueue and bounds coverage, while the reservation contract is already fully stated by
  the new dedicated tests. It converts in B, green in the same round. `ArchiveWriter`'s
  `queue` field is `pub` for this one slice, for the same reason
  `ArchiveQueue::finish_session` is, and becomes private when `enqueue` and `pump` are
  its production readers; `drain` is superseded and should go in that same slice. Ten
  `todo!` bodies remain — six in `ArchiveQueue`, four in `ArchiveWriter` — and nothing
  in the application calls this module, so none of it can be switched on by accident.
- Verification in `apps/desktop/src-tauri`: `cargo fmt --all -- --check` exits 0,
  `cargo clippy --all-targets --all-features -- -D warnings` is clean, and
  `cargo test --lib --all-features` reports
  `172 passed; 29 failed; 1 ignored; 0 measured; 0 filtered out` out of 202 — identical
  in parallel, single-threaded, and under `--all-targets --all-features`. All 29 failures
  are `todo!` panics (29 `panicked at`, 29 `not yet implemented`, zero `assertion` /
  `left ==` lines), at `enqueue` ×23, `begin_batch` ×3, `close` ×2, `peek_front` ×1. The
  archive suite is 82 tests, 53 green / 29 red, with the green count unchanged from slice
  A. `commands::tests::external_termination_with_verified_identity_stops_tree_and_releases_port`,
  which the reviewer saw fail with a Windows "Access denied", passed here in all three
  run modes and again alone; it kills a real process tree and binds a real port, so it is
  environment-dependent and flaky, and no stable all-green baseline is claimed for it.
  `commands.rs` was not modified.

## 2026-08-18 v0.3.0 Writer Slice A: Reading And Deleting One Archive

- Scope: `apps/desktop/src-tauri/src/archive.rs` only — the quota total behind a
  mutex, `credit_removed`, `read`, `delete`, and the shared `row_of` they both start
  from, plus the module header, two stale placeholder comments removed, and one
  in-test comment whose number was wrong. No test was added, weakened, or retargeted.
  `pump`, `close`, `close_all`, and eviction were deliberately left alone, so four
  `todo!` bodies remain instead of six. Untouched: the `Storage`-backed
  `ArchiveIndex`, the commands, the frontend, CI, the release workflow, tags, `.env`,
  and any real database. No commit, no push.
- **Decision: implement the two paths that only ever take bytes away, before the one
  that adds them.** `read` and `delete` need no queue, no buffered writer, and no
  per-session counters; what they need is the ownership gate and the quota total.
  Slicing them off first means the total's interior mutability arrives with its one
  real writer — the delete that decrements it — instead of as a field nothing yet
  uses, and it puts the whole `row → verified_file_name →
  resolve_ordinary_archive_file` chain under test before any code depends on it.
- **Decision: the quota total is a mutex and the leaf of the lock order.** A delete
  arrives on a command thread and gives bytes back while a pump on another takes
  them, so the number cannot belong to one thread. Nothing else is taken while it is
  held and it is never held across a file operation or an index write, so the two
  callers never wait on each other's disk. `total_bytes` reads through it rather than
  copying a field.
- **Decision: `credit_removed` saturates, and an unavailable total stays
  unavailable.** A total that has drifted below the length of the file just removed
  must not wrap into a number the size of the disk, so the subtraction saturates. And
  a delete reports how long one file was, not how much the directory holds — nothing
  about it can make an unmeasurable directory measurable again, so
  `QuotaTotal::Unavailable` is left alone and only the next startup sweep recovers a
  real total. That is the same asymmetry the sweep already states: an unknown total
  means "no room", and guessing is worse than refusing.
- **Decision: `delete`'s open-session refusal is a check before the work, and the
  reason it is safe is written down.** No state lock spans a file removal, so the
  check and the removal are not one critical section. What closes the gap is not the
  check alone: an archive file is created with `create_new`, so a `begin` racing a
  delete cannot take the file that is still there, and a session id is generated once
  for the run that produces it, so the id a user deletes is not one a later run
  begins. Stating the two real reasons is better than claiming a critical section the
  code does not have.
- **Decision: a row whose file is already gone is reported, not quietly marked
  removed.** The startup sweep already finishes that state, and it finishes it as
  `removed` / `file-missing` — which is what happened — rather than recording a user
  delete that removed nothing. The writer's job here is to refuse to guess.
- **Residual limitation.** The slice turns five tests green, not the six the plan
  claimed. `deleting_an_archive_is_refused_while_its_writer_is_open` asserts the
  refusal, then closes the session and deletes again, so its panic site moved from
  `delete` to `close`; its delete half passes and the test stays red until the `close`
  slice lands. Nothing in this slice has a runtime caller yet, since `lib.rs` still
  only declares the module, so `read` and `delete` are proven by their tests and not
  by use. `read` loads the whole archive into a `String`, which is bounded by the 10
  MiB per-session cap and is the shape the plan's viewer expects; a streaming read is
  not in v0.3.0's scope.
- Verification in `apps/desktop/src-tauri`: each of the six target tests was run
  alone first — five `ok`, the sixth failing at `close`. Then
  `cargo fmt --all -- --check` and
  `cargo clippy --all-targets --all-features -- -D warnings` clean;
  `172 passed; 22 failed; 1 ignored; 0 measured; 0 filtered out` out of 195 for the
  lib target, identical in parallel, single-threaded, and under
  `--all-targets --all-features`. Every failure is a `todo!` panic — `enqueue` ×20,
  `close` ×2 — with zero assertion lines in any run. The archive suite is 75 tests,
  53 green / 22 red; four are `#[cfg(windows)]`, so a non-Windows host runs 71.

## 2026-08-17 v0.3.0 Review Revisions: Four Corrections Before The Writer

- Scope: `apps/desktop/src-tauri/src/archive.rs` only — six new red tests, one new
  capability in the filesystem double, a renamed gate, two rewritten doc comments, one
  new session id. No production body was written: `ArchiveWriter`'s six `todo!` bodies
  are the same six. Untouched: the `Storage`-backed `ArchiveIndex`, the commands, the
  frontend, CI, the release workflow, tags, `.env`. No commit, no push.
- **Decision: the closing boundary is a linearization point, and the rejected design
  was a real defect rather than a style preference.** Draining a session and marking it
  closed afterwards leaves a window between the last drain and the state change,
  because `enqueue` does not take the pump's lock. A record accepted in that window is
  neither written nor refused: it sits in the queue of a session that will never pump
  again, which blocks `finish_session`, strands the entry, and holds queue room other
  sessions need. The correction is to mark the session closing *and* extract everything
  it has accepted inside one critical section, under the fixed open → queue lock order,
  and to do the file work only after both locks are released.
- **Decision: a writer-initiated close counts every record it accepted and never
  wrote.** A write error or a crossed quota stops a session in the middle of a batch
  while `enqueue` is still accepting, so records exist that the archive took
  responsibility for and did not persist. They are charged to `dropped_lines` /
  `dropped_bytes`. Discarding the batch remainder without counting it was rejected: the
  row would then report a smaller loss than the archive actually took, and a partial
  archive that under-reports is indistinguishable from a complete one, which is the one
  thing the drop counters exist to prevent. A quota close writes no gap line for those
  records — the file is stopping precisely because it is at its cap, and a gap line is
  more bytes in that same file — so the row's counters are the entire record of the
  loss and have to be exact.
- **Decision: eviction filters on eligibility before it sorts.** A session is a
  candidate only when its status is `complete` or `partial` and its `ended_at` is
  non-null, and never while this writer holds it open. The order is `ended_at`, then
  `started_at`, then `session_id`: the question eviction asks is which archive has been
  finished longest, `started_at` only breaks a tie between archives that ended
  together, and the session id makes the choice deterministic when both agree. Reading
  a missing `ended_at` as zero would invert the rule and delete the row this build
  understands least, first.
- **Decision: freed bytes are the disk's number, not the row's.** When the writer
  deletes an archive file and the row update that follows fails, the in-memory quota
  total is credited with the length measured on disk. The row's `byte_size` can be
  stale by any amount, and crediting it would make the archive refuse room the
  directory actually has. The inconsistent row is left for the next startup sweep,
  which already knows how to mark a row whose file is gone, and the failure is
  reported rather than swallowed. Both paths obey this: quota eviction and explicit
  delete.
- **Decision: state `enqueue`'s guarantee as "no I/O", not "never blocks".** It takes
  the open-session lock and then the queue lock, because whether a session is still
  accepting is part of the queue's answer, so it is not lock-free and calling it
  non-blocking is false. What it guarantees is that a capture thread never waits on a
  disk or a database. For the same reason "no lock is held across I/O" is scoped to the
  three state locks — open sessions, queue, quota total — while the pump's lock spans
  the writer's file and index work by design. An overstated invariant is worse than a
  modest one, because the next reader designs against it.
- **Residual limitation.** All six tests are red at a `todo!`, and five of them stop at
  `enqueue` — the first unwritten body they touch — so the machinery they were written
  to exercise is still unexercised. That includes the new seam: `TestFs::hold_write_of`
  holds one named file's next `write`, which is the only place a test can stand inside
  the post-drain, pre-closing window, since a `sync_data` hold arrives after the state
  has already changed under every ordering. Neither race test reaches its gate yet. The
  boundary test deliberately permits two outcomes, written or refused, and forbids only
  the third, stranded in the queue; which of the two the writer picks is still an open
  design choice. And the eviction filter's `ended_at` half is asserted against a row
  RunCove's own schema cannot store — `CHECK ((status = 'writing') = (ended_at IS
  NULL))` rejects a `complete` row with a null `ended_at` — so it is tested as what it
  is, a defence against a database this build did not write.
- Verification in `apps/desktop/src-tauri`: `cargo fmt --all -- --check` and
  `cargo clippy --all-targets --all-features -- -D warnings` clean;
  `167 passed; 27 failed; 1 ignored; 0 measured; 0 filtered out` out of 195 for the lib
  target, identical in parallel, single-threaded, and under
  `--all-targets --all-features`. Every failure is a `todo!` panic — `enqueue` ×20,
  `delete` ×5, `close` ×1, `read` ×1 — with zero assertion lines in any run. The archive
  suite is 75 tests, 48 green / 27 red; four are `#[cfg(windows)]`, so a non-Windows
  host runs 71.

## 2026-08-17 v0.3.0 Closing Boundary: Nothing Follows A Close

- Scope: `apps/desktop/src-tauri/src/archive.rs` only, tests and test seam only. Six
  new red tests, one new capability in the filesystem double, one new bounds
  fixture. No production body was written: `ArchiveWriter`'s six `todo!` bodies are
  the same six. Untouched: the `Storage`-backed `ArchiveIndex`, the commands, the
  frontend, CI, the release workflow, tags, `.env`. No commit, no push.
- **Decision: state the closing boundary as tests before implementing the writer.**
  The write path's hardest question is not how a line is written but what may follow
  a close, because that is where three pieces of state can disagree — the queue's
  per-session entry, the file handle, and the row. Three cases are now pinned: a
  late record after `close_all(UserDisabled)`, a record racing a close that is
  already inside `sync_data`, and closing a session that produced nothing or has
  already been closed. In each case the file, the row (drop counters and `ended_at`
  included), and the index call log must be exactly what the close left.
- **Decision: assert the queue's state through the bounds, not through a field the
  writer does not have.** `ArchiveWriter` still owns no `ArchiveQueue` —
  `initialize` discards `bounds` and `limits` — so a test cannot read the writer's
  queue, and adding an accessor for the test to read would be inventing production
  API for the test's benefit. `one_slot_bounds()` (one queued record in total)
  reverses the problem instead: a record wrongly queued for a closed session would
  be occupying the only slot, so the *next live session* loses a line, and that loss
  is an ordinary assertion on that session's row and file. Two of the six tests read
  the boundary that way.
- **Decision: the concurrency seam is a rendezvous, not a sleep.**
  `TestFs::hold_sync_of(file_name)` holds the next `sync_data` on one named file
  until the test lets it go, so a close can be stopped exactly where it has drained
  and flushed the session and has not yet released the handle or written the row.
  Three properties keep it honest: the gate is taken out from under the state lock
  before it blocks (no lock is held across the pause, so the concurrent `enqueue`
  cannot deadlock against the double); it fires once and by file name, matching the
  double's existing "by call count or by name, never by timing" rule; and
  `SyncHold::wait_for` re-raises the closing thread's panic with `resume_unwind`
  when that thread finishes without reaching the seam, so an unimplemented body is
  reported rather than hung on. The 25 ms poll inside `wait_for` is a liveness
  check, the only duration in the file, and no assertion depends on it.
- **Residual limitation.** The seam is not yet exercised by anything: both race
  tests panic at `enqueue` on the main thread before the close thread is spawned, so
  the gate is machinery waiting for the slice that makes `enqueue` real. Whether the
  writer *refuses* or *ignores* a record for a closing session is also still open —
  the tests deliberately pin the consequences (nothing in the file, nothing in the
  counters, no room taken) rather than the choice.
- Verification in `apps/desktop/src-tauri`: fmt clean (one rustfmt diff — a stray
  blank line — applied and re-checked), clippy clean under
  `--all-targets --all-features -- -D warnings`, `cargo test --lib` at
  `167 passed; 21 failed; 1 ignored` out of 189 — identical in parallel,
  single-threaded, and under `--all-targets --all-features`. All 21 failures are
  `todo!` panics with **zero** `assertion` lines: `enqueue` ×15, `close` ×1,
  `read` ×1, `delete` ×4. The archive suite is 69 tests, 48 green / 21 red.

## 2026-08-17 v0.3.0 Queue Lifecycle: Finishing a Session Frees It

- Scope: `apps/desktop/src-tauri/src/archive.rs` only, in response to a review
  finding against the queue slice below. One new operation, one new value type, six
  new tests, and the doc comments that stated the behavior it replaces. Untouched:
  `ArchiveWriter`'s six `todo!` bodies, the `Storage`-backed `ArchiveIndex`, the
  commands, the frontend, CI, the release workflow, tags, `.env`. No commit, no push.
- **The finding, accepted.** `sessions` held a `SessionQueue` for every session the
  process had ever seen and `drain` walked all of them on every pump, so both memory
  and pump cost grew with historical session count rather than with the number of
  open sessions. The queue slice's notes below state that as intentional; this
  section supersedes that.
- **Decision: one terminal operation that returns both halves, or the leak comes
  back.** `finish_session(session_id) -> AppResult<FinishedSession>` where
  `FinishedSession { residual_gap: Option<DropCounters>, dropped: DropCounters }`.
  Two accessors would have been the obvious shape — `take_pending_gap` already
  returns the residual and `dropped` already returns the totals — and it was
  rejected: a caller that took one and forgot the other would either write a gap
  line twice or file a row whose counters no longer match the file, and neither
  accessor can free the entry safely on its own. `AppResult` rather than `Option`
  is not a `#[must_use]` argument — `Option` carries that attribute too. It is that
  there are three outcomes and an `Option` holds only two: a residual gap to write,
  nothing owed (`Ok(FinishedSession::default())`, for a session that never queued
  anything or has already been finished), and *not now* (`Err`, records still
  queued). Folding the third into the second is precisely the mistake that would let
  a close file its row while bytes were still owed.
- **Decision: refuse while records are queued, and take nothing when refusing.**
  Queued records still owe their bytes and their gaps, so forgetting the session
  before a pump would lose both. The check reads `session.records` before anything
  is taken, which makes "pump, then finish" a safe retry instead of a race with a
  half-consumed entry. The refusal is per session, so a batch shutdown can finish
  the sessions that are drained without waiting on the ones that are not.
- **Decision: a finish is final, and that is a constraint on the writer, not on the
  queue.** A second finish, or a finish for a session never seen, yields
  `FinishedSession::default()` — owed nothing, no losses — which is what makes
  writing one gap twice impossible. The price is that a record arriving after a
  finish gets a fresh entry with no history and carries no gap, so the writer must
  finish a session only once that session's capture threads can no longer enqueue.
  That price is written down as the test `a_record_after_a_finish_starts_a_fresh_entry`
  rather than as a comment, and `ArchiveWriter::close`'s doc comment now names
  `finish_session` as the close path so the next slice cannot reintroduce the leak
  by reaching for `take_pending_gap`.
- **Deviation from the instruction.** The patch was requested with a 私有 (private)
  terminal operation. `finish_session` is `pub` instead: with no production caller
  yet, a private or `pub(crate)` method is `dead_code` under `-D warnings`, and the
  alternatives were an `#[allow]` bypass marker (forbidden here) or writing the
  writer (out of scope this round). What holds today is that the method has **no
  runtime caller** — every call site is a test — not that a `pub` method of a `pub`
  module is unreachable; it is reachable by definition. The narrowing to
  module-private is owed the moment the writer becomes its production caller, which
  is the next slice.
- **Residual limitation, unchanged from below.** Nothing writes a gap line yet.
  `pump` is still `todo!`, so the gap's on-disk placement stays pinned in the
  queue's output and in the still-red
  `gap_records_sum_to_the_dropped_counters_of_a_closed_archive`.
- Verification in `apps/desktop/src-tauri`: fmt clean (one rustfmt diff in a new
  test applied and re-checked), clippy clean under
  `--all-targets --all-features -- -D warnings`, `cargo test --lib` at
  `167 passed; 15 failed; 1 ignored` out of 183 — identical in parallel,
  single-threaded, and under `--all-targets --all-features`. The 15 failures are the
  same writer `todo!` panics (`enqueue` ×10, `delete` ×4, `read` ×1) and the run
  holds zero `assertion` lines. The 17 queue tests run focused are
  `17 passed; 0 failed`. The archive suite is 63 tests, 48 green.

## 2026-08-17 v0.3.0 The Queue Slice: `ArchiveQueue` and the Gap Partition

- Scope: `apps/desktop/src-tauri/src/archive.rs` only. The nine `ArchiveQueue`
  bodies, the gap carrier the contract needed, five new tests, and one rewritten
  assertion. Untouched, as instructed: `ArchiveWriter`'s `enqueue`, `pump`, `close`,
  `close_all`, `read`, and `delete`; the `Storage`-backed `ArchiveIndex`; the
  commands; the frontend; CI; the release workflow; tags; `.env`. No commit, no push.
  `lib.rs` is unchanged, so `pub mod archive;` remains the module's only reference in
  the crate and the feature is still unreachable from the application.
- **Decision: the gap rides on a wrapper, not on `ArchiveRecord`.**
  `V0.3.0_PLAN.md:337-341` had left both open; that paragraph now records the
  resolution and reads at `V0.3.0_PLAN.md:335-357`. `drain` now returns
  `Vec<QueuedRecord>`, where `QueuedRecord` is `{ record: ArchiveRecord, gap_before:
  Option<DropCounters> }` (`archive.rs:219`). The alternative — a `gap_before` field
  on `ArchiveRecord` that the capture side never sets — was rejected for two reasons.
  The gap describes the hand-off, not the line the child process wrote, so a field
  capture can set but never should is a field that lies; and `encode_record` writes
  exactly three keys, so a gap sitting on `ArchiveRecord` would be handed to it and
  silently discarded, while a caller holding a `QueuedRecord` must reach through
  `.record` and therefore sees `gap_before` at the point of use. The predicted cost
  was paid in full and was nothing more: two test call sites gained one level of
  nesting, asserting the same values.
- **Decision: one rewritten assertion, and it is stronger than what it replaced.**
  `every_short_enqueue_sequence_keeps_the_queues_invariants` had required the
  post-drain `take_pending_gap` to return everything the session lost. That is false
  under the approved contract — a loss a later record picked up leaves with that
  record — and it described the merged-gap design the user rejected, so leaving it
  would have pinned the losing alternative. The replacement compares the whole
  drained sequence of `gap_before` values against a positional model vector, so the
  test now checks *where* each run is reported and not only how much was lost. No
  other existing assertion changed.
- **Residual limitation.** The queue decides where a gap is reported; nothing yet
  writes it. `ArchiveWriter::pump` is still `todo!`, so the claim "the gap line
  stands immediately before the first line that survived the loss" is pinned in the
  queue's output and in `gap_records_sum_to_the_dropped_counters_of_a_closed_archive`
  — which is still red at the writer — but is not yet demonstrated end to end in a
  file on disk. Judge the next slice on that test going green without its assertions
  being touched.
- Evidence the new assertions are not vacuous: two mutations were compiled, run, and
  reverted. Carrying nothing (the merged-gap behavior) and carrying without clearing
  (double-reporting) each failed four of the five new tests plus the exhaustive one;
  the exhaustive test caught the second on "the residual is the trailing run, and
  nothing else". `a_trailing_drop_has_no_record_to_carry_it_and_stays_the_residual`
  stayed green under both, correctly — it pins the residual half, which neither
  mutation can reach. Details and the failing traces are in `HANDOFF.md`.
- Verification after the revert, in `apps/desktop/src-tauri`: fmt clean, clippy
  clean under `--all-targets --all-features -- -D warnings`, and `cargo test --lib`
  at `161 passed; 15 failed; 1 ignored` out of 177 — identical in parallel,
  single-threaded, and under `--all-targets --all-features`. All 15 failures are
  `todo!` panics in the writer (`enqueue` ×10, `read` ×1, `delete` ×4) and the run
  holds zero `assertion` lines. The archive suite is 57 tests, 42 green.

## 2026-08-17 v0.3.0 The Drop-Counter Contract Is Corrected (Step 4b Unblocked)

- Scope: the version 2 schema and the documents that describe it. Production code did
  change — the DDL inside `upgrade_to_version_2` (`storage.rs:715`) is not the text it
  was, and the pinned `V2_ADDITION` copy matches it — plus one parametrized rejection
  case removed and four documents updated. What did **not** change: no new function
  body, no new run path, no part of the queue implementation, no schema version, no
  version 3 migration, no CI, release, tag, or `.env` file, no commit and no push.
  This is the correction the section below paused for, and it happened only after the
  user approved it, because a schema change is one of this project's stop-and-ask
  lines.
- The decision, and the alternative it beat. The user answered
  **「就地改 v2 DDL，不加 v3」**: correct the version 2 DDL in place, keep the schema
  version at 2, add no version 3 step. The alternative was to treat version 2 as
  frozen and repair it with an `upgrade_to_version_3` table rebuild. That would be
  the right move if a version 2 database existed — SQLite keeps a `CHECK` inside the
  table definition, so a DDL edit cannot reach a table that already exists. Neither
  verified artifact has one: the two developer databases on this machine and the
  published `v0.2.1` baseline. That is the scope of the census further down, and it is
  what makes the in-place edit not merely cheaper but complete for the artifacts that
  were actually checked.
- The new constraint is `CHECK (dropped_bytes = 0 OR dropped_lines > 0)`. It states
  the one direction that is genuinely impossible — bytes cannot be lost without a
  line, because every archived byte belongs to some line — and says nothing about the
  other, because a dropped empty line costs `1 line / 0 bytes` and is ordinary data.
  The old form, `((dropped_lines = 0) = (dropped_bytes = 0))`, collapsed that
  implication into an equivalence, which is the whole of the bug. Both copies now
  carry a four-line SQL comment saying so, in English like the rest of the
  repository, so the next reader does not re-derive it.

- Applied at five sites in one round, because nothing in the crate compares them to
  each other: the design DDL (`V0.3.0_PLAN.md:634`), the prose invariant restating it
  (`V0.3.0_PLAN.md:844-846`), the production migration (`storage.rs:715`), the pinned
  `V2_ADDITION` copy (`storage.rs:1240`), and the `"drop counters agree"` case in
  `the_archive_index_rejects_impossible_rows`. The migration and the pinned copy are
  now held to the same rule by two tests that each run against both
  (`an_archive_row_may_lose_a_line_that_carried_no_bytes` and
  `an_archive_row_may_not_lose_bytes_without_losing_a_line`, through
  `create_pinned_version_2_database`), so the next half-applied edit fails instead of
  drifting.
- One existing assertion was removed, deliberately and with approval. The
  `"drop counters agree"` case required
  `('sess-exited','a.jsonl','complete',NULL,0,0,5,0,10,11)` — five dropped lines, no
  dropped bytes — to be rejected. That row is five dropped empty lines, which is
  ordinary data, so the case rested on the same wrong premise as the constraint and
  could not survive the fix. A comment at `storage.rs:1553` records what stood there
  and names the two tests that now pin both directions, so the removal reads as a
  contract change rather than as a test that quietly went missing. It is the only
  assertion this round touched; nothing was weakened, retargeted, or `#[ignore]`d to
  reach green.
- Residual limitation, recorded rather than fixed: nothing in the build compares the
  plan's DDL text to the crate's. The three-way agreement between `V0.3.0_PLAN.md`,
  the migration, and `V2_ADDITION` is maintained by hand and by review. Only the last
  two are pinned against each other, and only through behavior — no test reads DDL
  text out of `sqlite_master`, which is also why SQL comments could be added inside
  the table definition without breaking a comparison.

- Verification, in `apps/desktop/src-tauri`: `cargo fmt --all -- --check` clean;
  `cargo clippy --all-targets --all-features -- -D warnings` clean with zero warnings
  and no `allow` added; `cargo test --lib` at `150 passed; 21 failed; 1 ignored;
  0 measured; 0 filtered out` out of 172, identical in parallel and with
  `--test-threads=1`. The change is visible as exactly one test flipping — the
  previous `149 passed; 22 failed; 1 ignored` minus the storage failure that had been
  quoting the old constraint verbatim. The other 21 reds are unwritten step 4b bodies
  (`archive.rs:1098` ×6, `:1355` ×10, `:1410` ×1, `:1420` ×4, every one a `todo!`),
  and the suite output holds zero `assertion`, `left ==`, or `left !=` lines. Module
  counts are unchanged at archive 52, storage 24, processes 12, because a
  parametrized case was removed, not a test.
- Safety was established before the edit rather than inferred from the result: all
  four `INSERT INTO run_log_archives` sites and every row they carry were enumerated,
  and none writes bytes without a line, so no previously accepted row became
  rejected. The five storage tests touching the changed DDL also pass individually
  under `--exact`, so none is riding on another test's side effect.
- The census that licenses the in-place fix, re-taken on 2026-08-17 from the SQLite
  header (bytes 60..63 of page 1, no connection opened): both developer databases,
  under `%LOCALAPPDATA%\com.abysswhale.runcove` and `...com.abysswhale.runcove.qa`,
  are at `user_version = 1`, and neither directory holds a `-wal` or `-shm` file, so
  the header is authoritative rather than a stale page. The version 2 code is
  unreleased and uncommitted, and the published `v0.2.1` baseline never mentions
  `run_log_archives`, so no v0.2.1 install can hold a version 2 database either.
  Stated at its true scope: **the verified local developer databases and the published
  baseline have no version 2 database.** Machines that were never measured are not
  covered, which is exactly why the rebuild recipe is kept in `HANDOFF.md` — for this
  working tree copied elsewhere and run there.
- A separate, smaller pass in the same round repaired stale line-number citations in
  `V0.3.0_PLAN.md` — some shifted by this round's four added comment lines per DDL
  copy, some already stale from the blocker round, one by 128 lines. The
  migration-test list now names all nine tests instead of listing bare offsets, and
  the note above it says outright that the names are authoritative and the numbers
  rot with each slice. Two factual errors in the plan went with it, both exposed by
  this round's own evidence: the data directory does not hold `-wal` and `-shm`
  companions (`Storage::open` never sets `journal_mode`), and
  `the_archive_index_rejects_impossible_rows` now carries eight cases rather than
  nine. Neither correction changes a design decision.
- Second decision, recorded and **not implemented**: the gap partition. The user
  chose **「gap 挂到下一条记录上」** over the recommendation to merge. Pending drop
  counters attach to the next accepted record for that session, so `take_pending_gap`
  returns only the trailing residual, which the writer flushes at close. This buys
  exact placement of gap records in the archive file, at the price of a carrier field
  on the queued record and a rewritten assertion at `archive.rs:3369`. It is the
  queue slice's first move, and the slice has not started: the user asked to stop for
  review once the schema correction landed.

## 2026-08-16 v0.3.0 Step 4b Paused (A Wrong Drop-Counter Contract In The Version 2 Schema)

- Scope: tests and documentation only. Seven tests were added across three files,
  four documents were updated, and nothing else changed — no production DDL, no
  `CURRENT_SCHEMA_VERSION`, no queue or writer body, no CI, release, tag, or `.env`
  file, no commit and no push. The round exists because the user found a defect in
  the *design* before the queue was written, and the cheapest place to record a
  design defect is a failing test that says what the code should do.
- The defect. The version 2 `run_log_archives` table ends with
  `CHECK ((dropped_lines = 0) = (dropped_bytes = 0))`. Read literally it says: a
  session has lost lines if and only if it has lost bytes. That is a guess about the
  data, and the guess is wrong, because a line and its bytes are not the same
  quantity. `capture_stream` splits on `\n` and emits an event per line; a lone
  newline is a line whose text is empty. Traced by hand and then pinned by test,
  input `"\n"` produces exactly one event with `line == ""`, and `"a\n\nb\n"`
  produces `["a", "", "b"]`. Drop one empty record and the honest counters are
  `dropped_lines = 1, dropped_bytes = 0`, which the constraint refuses. The archive
  would have failed to record a loss it was built to record — `close` would have
  written a row the database rejects, on data the queue is specified to produce.
- The correct rule is one-directional. Bytes cannot be lost without a line, because
  every byte of archived text belongs to some line, so `dropped_bytes > 0` with
  `dropped_lines = 0` is genuinely impossible and must stay rejected. The reverse is
  ordinary. The original constraint collapsed an implication into an equivalence,
  which is the whole bug; it is the same shape of error as writing `a == b` where
  the domain only justifies `b != 0 -> a != 0`.
- A knock-on correction to an earlier note: the rejection case labelled
  `"drop counters agree"` in `the_archive_index_rejects_impossible_rows`, which
  pins `5 lines / 0 bytes` as impossible, is not impossible data. It is five dropped
  empty lines. That case was written against the same wrong premise as the
  constraint, and it will have to go when the constraint is fixed.
- No analogous defect exists for `line_count` and `byte_size`. Their `CHECK`s are
  independent (`>= 0` each), and an archive of five empty lines still has a non-zero
  `byte_size` because each stored record is a JSON object with a timestamp and a
  stream field around the empty text. So the archive's own counters cannot reach the
  `n lines / 0 bytes` shape that the drop counters reach naturally.
- The defect has three copies, and nothing was comparing them. `V0.3.0_PLAN.md:598`
  holds the design, restated in prose at `V0.3.0_PLAN.md:772-773`;
  `storage.rs:711` is the production migration; `storage.rs:1232` is the
  `V2_ADDITION` constant whose stated purpose is that "drift between the two fails a
  test instead of silently redefining the schema". That purpose was only half met:
  `V2_ADDITION` had two users, both checking version handling
  (`a_version_2_database_opens_and_only_a_higher_version_is_rejected`), and no test
  compared its text or its behavior to the migration's. A fix applied to production
  and not to the pinned copy would have gone unnoticed. Both new schema tests
  therefore run twice — once against a database migrated by `Storage::open`, once
  against one built straight from `V2_ADDITION` through the new
  `create_pinned_version_2_database` helper — so a half-applied correction fails.
- The seven tests, by what they require and where they stop today:
  - `a_lone_newline_is_captured_as_one_empty_log_line` (`processes.rs:1104`) —
    **green**, and expected to be. It is a control test: it establishes the premise
    the whole blocker rests on, so a future change to `capture_stream` that stopped
    emitting empty lines would show up as a failing premise rather than as silent
    dead weight in the schema.
  - `an_archive_row_may_lose_a_line_that_carried_no_bytes` (`storage.rs:1592`) —
    **red on the constraint**, with the panic message quoting it verbatim:
    `CHECK constraint failed: (dropped_lines = 0) = (dropped_bytes = 0)`. It requires
    both `1 line / 0 bytes` and `5 lines / 0 bytes`.
  - `an_archive_row_may_not_lose_bytes_without_losing_a_line` (`storage.rs:1638`) —
    **green**, and it must remain green after the fix. Its job is to stop the
    correction from over-shooting into "no relationship at all": it refuses
    `0 lines / 40 bytes` and a negative count in either column.
  - `dropping_an_empty_line_counts_one_line_and_no_bytes` (`archive.rs:3153`) —
    **red at `todo!`** in `ArchiveQueue::new` (`archive.rs:1098`). It requires
    `DropCounters { lines: 1, bytes: 0 }` and the user-visible
    `[RunCove: dropped 1 line / 0 bytes]`. Its bounds force the refusal through the
    *record* bound on purpose: an empty record consumes no bytes, so a byte bound
    could never reject it, and a test that tried would be testing nothing.
  - `the_queue_counts_utf8_bytes_and_not_characters` (`archive.rs:3183`) — **red at
    the same `todo!`**. It is written with `\u{...}` escapes rather than literal
    characters so the byte counts cannot depend on how this file is encoded, and it
    asserts its own premise first. A queue that counted characters fails it in two
    independent places, which is deliberate: `queued_bytes()` would report 1 for
    a three-byte character, and a record that must be refused would be accepted.
  - `every_short_enqueue_sequence_keeps_the_queues_invariants` (`archive.rs:3232`)
    and `a_pending_gap_is_taken_once_and_the_cumulative_total_is_never_cleared`
    (`archive.rs:3388`) — both **red at the same `todo!`**.
- The exhaustive test is a deliberate substitute for a property-test dependency, not
  a poor imitation of one. `proptest` would bring a new crate, a random seed, and a
  shrinking step whose output is a fresh puzzle every failure; a full enumeration of
  a deliberately tiny space brings none of that and fails identically every time. It
  walks all `4^6 = 4096` sequences of six records over a four-symbol alphabet — an
  empty line and a four-byte line on one session, a three-byte multi-byte character
  on a second, a four-byte line on a third — against bounds `{2, 4, 3, 8}` chosen so
  that every one of the four bounds is met exactly by some sequence and exceeded by
  another. Each step is compared to a model the test recomputes from the records it
  believes were kept, and every failure message carries a readable trace such as
  `[A:abcd,B:han,A:empty,...] step 3`, so a red result names the sequence instead of
  a seed. Cost is roughly 25,000 enqueues, well under a second.
- What it pins: no accepted record puts any per-session or total record or byte count
  over its bound; equality with a bound is allowed and only the *incoming* record is
  ever refused, which is checked by comparing `queue.len()` with the model at every
  step so an implementation that evicted something already queued would fail;
  `drain` returns every kept record in global arrival order and resets `len`,
  `queued_bytes`, and `is_empty`; a pending gap is taken once and `None` after, while
  the cumulative `dropped` total survives both the take and the drain; and `dropped`
  equals the line count and summed UTF-8 text length of exactly the refused records.
- One thing was left unpinned on purpose. `V0.3.0_PLAN.md` promises one `system` gap
  record per *contiguous* gap, but it never settles how a drop → accept → drop run
  partitions when no take intervenes, and `take_pending_gap` returns a single
  `Option<DropCounters>` per session, so the API has nowhere to hold two. Rather than
  invent a partition and freeze it in a test, both new tests assert only that a take
  returns everything lost since the previous take and that gaps sum to the cumulative
  total, which is what `V0.3.0_PLAN.md:319` already requires. If the queue's author
  later needs a different partition, these tests do not stand in the way.
- A contradiction was left standing in the test suite, on instruction and on
  purpose. No existing assertion was deleted, weakened, or retargeted, so
  `storage.rs` now contains two tests that disagree about the same row:
  `the_archive_index_rejects_impossible_rows` requires `5 lines / 0 bytes` to be
  rejected, and `an_archive_row_may_lose_a_line_that_carried_no_bytes` requires it to
  be accepted. Measured, not predicted: the old one passes today, the new one fails.
  No constraint can satisfy both, so the correction must also delete the
  `"drop counters agree"` case — an edit to an existing assertion, which is exactly
  the kind of change that should be approved deliberately rather than slipped in
  beside a test-only round.
- Verification, in `apps/desktop/src-tauri`: fmt clean, clippy clean at
  `--all-targets --all-features -- -D warnings` with zero warnings and no `allow`
  added, and `cargo test --lib` at `149 passed; 22 failed; 1 ignored; 0 measured;
  0 filtered out` out of 172 — the same three numbers single-threaded and under
  `--all-targets --all-features`. The previous checkpoint was `147 passed; 17 failed;
  1 ignored` out of 165, so the arithmetic closes exactly: seven tests added, two
  green, five red, nothing else moved. Module counts are archive 52, storage 24,
  processes 12; four archive tests are `#[cfg(windows)]` and nothing added here is
  platform-gated, so a non-Windows host runs 168.
- Failures were classified by panic site rather than by reading names:
  `archive.rs:1098` ×6, `archive.rs:1355` ×10, `archive.rs:1410` ×1,
  `archive.rs:1420` ×4, `storage.rs:1620` ×1. The captured output contains zero lines
  matching `assertion`, `left ==`, or `left !=`. That matters more than the failure
  count: every red test stops at an unwritten body or at the constraint under
  discussion, so none of them is making a wrong claim about behavior that exists.
- Two small things worth keeping. Clippy's `int_plus_one` rejected the model's
  `count + 1 <= bound`, which had been written that way to read in parallel with the
  byte lines beside it; the resolution was `count < bound` plus a comment explaining
  both readings, not an `allow`. And `cargo fmt --all` reflowed exactly one
  `assert_eq!` in the new gap test and touched nothing else, confirmed by re-running
  `--check` afterwards.
- Residual limitation, and the reason this round stops here: the correction is a
  schema change, so it waits for the user. When authorized it has five parts, and
  omitting any one of them leaves the repository inconsistent — the plan at `:598`
  and its prose at `:772-773`, the production DDL at `storage.rs:711`, the pinned
  `V2_ADDITION` at `storage.rs:1232`, the `"drop counters agree"` case, and a
  decision about `CURRENT_SCHEMA_VERSION`. On the last one: v0.3.0 is unreleased, so
  no user database is at version 2 and correctness needs no version 3 migration —
  but SQLite stores a `CHECK` inside the table definition, so any *developer*
  database already migrated by an earlier build keeps the wrong constraint however
  the DDL reads. Editing the DDL alone does not reach it; that database must be
  discarded by hand or the table rebuilt.

## 2026-08-16 v0.3.0 Step 4b, Third Slice (Begin And The Open-Session Gate)

- Scope: `ArchiveWriter::begin`, `ArchiveWriter::is_open`, and the writer state those
  two need — the private `OpenArchive` slot, the `open: Mutex<BTreeMap<String,
  OpenArchive>>` map, and the `fs` and `index` handles the writer now keeps instead of
  dropping after the sweep. The queue, `enqueue`, `pump`, `close`, `close_all`, `read`,
  `delete`, quota eviction, the `Storage`-backed `ArchiveIndex`, the commands, and the
  frontend are untouched and still `todo!`. Evidence, from
  `apps/desktop/src-tauri`: fmt clean, clippy clean with zero warnings,
  `cargo test --lib --all-features -- --test-threads=1` at `147 passed; 17 failed;
  1 ignored; 0 measured; 0 filtered out` out of 165, with the default parallel run and
  `--all-targets --all-features` reporting the same three numbers and no failure
  outside `archive::tests`. The archive suite is 48 tests, 31 green / 17 red, up from
  24 / 19. All 18 tests this slice could affect were confirmed alone under
  `-- --exact`, every one of the 17 remaining failures is a `todo!` panic, and no run
  contains an assertion line.
- `begin` is three phases, and the reason is requirement 8, not tidiness: holding the
  open-session lock across `create_new` and `insert_writing` would put a SQLite write
  inside a lock that every capture thread wants. So phase one validates the id and
  resolves the path while touching nothing, phase two takes the session's slot under
  the lock and releases it, and phase three creates the file and inserts the row with
  no lock held, re-taking it only to store the handle or drop the slot.
- The slot is what makes concurrency deterministic instead of lucky. It is inserted as
  `OpenArchive { file: None }` before the file exists, so a second `begin` for the same
  session finds the key already present and loses immediately. That `None` window is
  also the only state a concurrent caller can observe, and only to lose, which is why
  the two refusals read differently: "already has an open archive" when a handle is
  there, "is already opening its archive" when it is not. Reading `file.is_some()` for
  that message is also what keeps the field from being `dead_code` under `-D warnings`
  before `pump` exists to write through it — no `allow` was needed.
- Failure cleanup order is forced by Windows: the handle is dropped before
  `remove_file`, because this process's own open handle blocks a delete. If the delete
  fails too, `begin` still returns an error, the slot is still released, `is_open` is
  false, the total does not grow, and the message says the empty file is left for the
  next startup sweep rather than pretending it is gone. That is the same orphan the
  sweep already handles, so the two failure modes compose instead of leaking.
- `begin` deliberately does not consult the quota, and the doc comment says why: a new
  archive is an empty file, so there is nothing yet to weigh against a cap. The caps
  and a `QuotaTotal::Unavailable` total refuse the first *record*, in `pump`. This was
  settled by reading `an_unavailable_total_stops_the_archive_instead_of_growing_it`,
  which arranges an unavailable total and then requires `begin` to succeed — guessing
  the other way would have made that test fail for the wrong reason.
- `is_open` is true from slot-taking until close or failure, which deliberately covers
  the instant before the file exists, because `delete` must refuse a session that is
  being opened. A session the index knows about but this writer never opened is not
  open; `is_open` answers about this writer's live sessions, not about the database.

- Two of the 25 tests that had been waiting on `initialize` went green here:
  `a_refused_writing_row_leaves_no_orphan_file_behind` and
  `an_orphan_left_by_a_failed_cleanup_is_deleted_by_the_next_sweep`. Eleven advanced to
  a body the boundary excluded — ten to `ArchiveWriter::enqueue`, and
  `deleting_an_archive_is_refused_while_its_writer_is_open` to `delete`. That is the
  measurable result the boundary allows, and it is the same standard the second slice
  was judged by: which named tests go green, and where each remaining one stops.
- The 17 remaining failures panic at four sites, each read to confirm the line:
  `archive.rs:1098` `ArchiveQueue::new` ×2, `:1355` `enqueue` ×10, `:1410` `read` ×1,
  `:1420` `delete` ×4. Fifteen `todo!` bodies remain, down from 17.
- Five tests were added, all for requirement 7, which a `.begin(` / `.is_open(` grep
  showed no existing test covered. No existing assertion was touched.
  - `beginning_a_session_this_build_could_not_have_generated_touches_nothing` — eight
    bad ids: empty, `.`, `..`, a non-UUID, an uppercased UUID, one character short, one
    long, and an underscored one. Each errors, `is_open` stays false, and afterwards
    the directory is empty, the filesystem log has no removal, the index log has no
    call at all, there is no row, and the total is still `Known(0)`. Validation
    genuinely precedes any effect, rather than being cleaned up afterwards.
  - `beginning_a_session_whose_file_already_exists_refuses_and_keeps_the_file` — the
    file is planted after `initialize`, so the sweep cannot have seen or deleted it.
    `create_new` fails, the planted bytes are byte-for-byte intact, and no index call
    happens, which is the assertion that `create_new` is the guard rather than a
    `remove_file`-then-recreate.
  - `beginning_the_same_session_twice_is_refused_and_keeps_the_first_archive` — the
    second call errors, the first archive stays open, its row stays `writing` with the
    first `started_at`, and the index call log does not grow.
  - `two_threads_beginning_the_same_session_leave_exactly_one_open_archive` — two
    threads share the writer through an `Arc` and meet at a `Barrier`. Exactly one call
    is `Ok`, and there is exactly one `insert_writing:` call, one row, one file, and no
    removal. No assertion names which thread wins, so the test cannot pass because an
    interleaving happened to be favorable; it was also repeated 20 times alone, 20 ok /
    0 failed. `ArchiveWriter` is `Send + Sync` by construction, which is what lets the
    test share it: `Arc<dyn ArchiveFs>` and `Arc<dyn ArchiveIndex>` are both, and
    `Box<dyn ArchiveFile>` is `Send`.
  - `is_open_is_false_for_a_session_this_writer_never_opened` — a `complete` archive
    from an earlier run is on disk and in the index; `is_open` is false for it, for an
    unopened id, and for two invalid ids, and true only for the session this writer
    actually began.
- One regression this slice caused and fixed, recorded because reading the diff would
  not explain it: a placeholder-anchored edit used `ArchiveWriter::enqueue`'s doc
  comment as its anchor and deleted those three lines. `cargo fmt --all -- --check`
  caught it (exit 1, diff at `src\archive.rs:1347`), the original text was restored
  verbatim, and fmt returned to exit 0. Because the fix moved panic line numbers by two
  or three, the whole suite was re-run to re-record them instead of reusing the earlier
  values; the four sites above are the post-fix numbers.
- The root `runcove` crate was not touched and was re-verified anyway: fmt clean,
  clippy clean, `cargo test --all-targets` green (12, 0, 0, 10, 16). The frontend half
  of the matrix — `npm run lint`, `npm run typecheck`, `npm test -- --run`,
  `npm run build`, the Playwright E2E, and `cargo tauri build` — was not run, because no
  frontend file changed in this slice. The full matrix is still required and is
  scheduled for plan step 7.
- Nothing was committed, pushed, tagged, or released; no CI, release, tag, or `.env`
  file was touched; and `pub mod archive;` in `lib.rs` is still the module's only
  reference in the crate, so the feature remains unreachable from the application.

## 2026-08-16 v0.3.0 Step 4b, Second Slice (Initialize And The Startup Sweep)

- Scope: `ArchiveWriter::initialize`, the startup sweep behind it, `archive_dir`,
  and `total_bytes`. The queue, the write path, close, quota eviction, the
  `Storage`-backed `ArchiveIndex`, the commands, and the frontend are untouched and
  still `todo!`. Evidence, from `apps/desktop/src-tauri`: fmt clean, clippy clean
  with zero warnings, `cargo test --lib --all-features -- --test-threads=1` at
  `140 passed; 19 failed; 1 ignored; 0 measured; 0 filtered out` out of 160, and
  the default parallel run at the same numbers with no failure outside
  `archive::tests`. The archive suite is 43 tests, 24 green / 19 red, up from
  14 / 27. Each of the eight newly green tests also passes alone under
  `-- --exact`, every one of the 19 remaining failures is a `todo!` panic that was
  confirmed alone, none of them stops at `initialize` any more, and the run
  contains zero assertion lines.
- Eight of the 25 tests that were waiting on `initialize` could go green in this
  slice; the other 17 assert on `begin`, `pump`, `close`, `read`, `delete`, or
  eviction, which the authorized boundary excludes. They now stop at their own
  subject instead — 15 at `begin`, 1 at `read`, 1 at `delete` — which is the
  measurable result the boundary allows. Two of them assert sweep output before
  reaching their `todo!`, and those assertions run and hold: the total cap test
  sees `Known(800)` from both `measured_bytes` and `total_bytes()`, proving nothing
  is evicted at initialization, and the unavailable-total test sees `Unavailable`.
- Only two failures stop the sweep, and both mean it has nothing to work with: an
  index that cannot be read at all, and a directory that cannot be listed at all.
  Creating the directory is the third failure that stops `initialize` itself. Every
  other failure belongs to one entry or one row, and is reported as an anomaly and
  stepped over, because one file the filesystem refuses must not cost every other
  row its repair.
- An entry is classified by its name before anything else. A name this build's
  `archive_file_name` could not have produced is reported once and otherwise left
  entirely alone — not measured, not deleted, not read — whether or not the
  filesystem could describe it. `is_archive_file_name` and the sweep now share
  `archive_file_stem`, so the rule that admits a name and the id that name yields
  cannot drift apart into two rules.
- A row's `file_name` is checked against the name its own session generates before
  the row is acted on, and its status is parsed before anything is decided. The
  order matters: parsing second is what keeps a row whose status this build does not
  know — one a newer build wrote — from being mistaken for a session whose file has
  gone missing. A row that fails either check is reported and left byte-identical.
- A `complete` or `partial` row whose file is still there is never rewritten. Its
  numbers were final when it closed, and re-measuring would replace a fact with a
  guess. Only a `writing` row is repaired, to `partial` / `interrupted`, and it is
  repaired from the file rather than from itself: the row stopped being updated the
  moment the run died and the file did not, so the file's length and its recounted
  lines win. When the name is taken by something this build did not write the repair
  records zero bytes and zero lines; when the entry cannot be measured the row keeps
  its own numbers, which are then the only ones anyone has. Dropped-line and
  dropped-byte counters are preserved in every case.
- A file is deleted as an orphan only when no row at all exists for its session —
  whatever such a row would have said. A file some row still names is kept even when
  the sweep refused to act on that row, because reporting a strange row costs a line
  in a log and guessing at one costs a user their log. A delete the filesystem
  refuses is an anomaly, and those bytes still count towards the quota, because they
  are still on the disk.
- An index write that fails during the sweep is reported and stepped over. A repair
  that fails leaves the row `writing`, which is exactly the state this sweep
  repairs, so the next startup tries once more; nothing is retried in a loop and
  nothing is lost.
- One entry that nobody can size and no row remembers makes the whole total
  `Unavailable` rather than a smaller number. `Unavailable` already means "no room",
  so the archive stops growing instead of growing against a total it had to guess
  at: that total feeds the only cap standing between an opt-in feature and a full
  disk.
- Anomaly text may echo an entry name, because the OS listing yields a single path
  component, and never a row's `file_name`, because that is exactly the column that
  may be a path. A row is labelled by its session id only when the id is one this
  build generates, and by a fixed phrase otherwise.
- The writer holds `archive_dir` and the measured `total`, and nothing else. The
  filesystem and index handles are used by the sweep and dropped rather than
  stored, and `bounds` and `limits` are accepted and discarded, because a private
  field no body reads is `dead_code` under `-D warnings` — every field arrives with
  its first reader, while the signature stays pinned so it does not move when those
  readers land.
- Two tests were added against code that already existed, with no growth in
  implementation scope. `the_most_severe_reason_is_the_documented_order_over_every_pair`
  checks all 64 pairs against the documented order and its symmetry via an
  exhaustive-match rank helper, so the private `severity` numbers cannot move
  without the documented order moving too, and a ninth reason cannot be added
  without being placed in the order. `a_carriage_return_in_the_text_never_becomes_a_line_of_its_own`
  pins that `\r`, `\r\n`, `\n`, and `NUL` inside a captured line stay inside the JSON
  string and out of the file's line structure, which is what keeps one record one
  line.
- The intermittent port and timing failures recorded in earlier sessions did not
  reproduce today, in either the parallel or the single-threaded run. Nothing was
  changed to address them; they remain out of this slice's scope.

## 2026-08-16 v0.3.0 Step 4b, First Slice (Leaf Behavior Only)

- Scope: the pure, leaf-level half of `archive.rs` — the enum mappings, the file
  name and path rules, the record and gap encoding, the documented default bounds,
  and `RealArchiveFs`. The writer, the sweep, the queue, and the quota are
  untouched and still `todo!`. Evidence, from `apps/desktop/src-tauri`: fmt clean,
  clippy clean with zero warnings, `cargo test --lib --all-features` at
  `130 passed; 27 failed; 1 ignored; 0 measured; 0 filtered out` out of 158. The
  archive suite is 14 green / 27 red, up from 1 / 40. Each newly green test also
  passes alone under `-- --exact`, and all 27 remaining failures are `todo!`
  panics — 25 at `ArchiveWriter::initialize`, 2 at `ArchiveQueue::new` — with zero
  assertion lines anywhere in the run.
- The id rule checks the shape and not the version nibble. `is_generated_session_id`
  requires 36 characters, hyphens at 8/13/18/23, and lowercase hex elsewhere, which
  is exactly what `storage::new_id` emits. The version and variant nibbles are
  deliberately not pinned: the shape is what makes a name one harmless path
  component — it admits no separator, colon, dot, space, or `..` — so pinning adds
  nothing, while it would turn every archive already on disk into an unreadable
  orphan the day the id generator moved to another UUID version. All 28 names in
  `rejected_file_names` are refused by the shape rule alone, so no subtractive
  blocklist exists in the code.
- `most_severe` is a total order over all eight reasons, not over three. The plan
  only ranks `write-error` > `quota-exceeded` > `queue-overflow`, which are the
  reasons a session can accumulate while writing. Ranking only those would leave the
  answer for any other pair dependent on argument order, so all eight are ranked and
  the tie-free order is documented on the private `severity` helper: what happened to
  the archive outranks what RunCove or the user chose to do with it.
- `RealArchiveFs::create_new` is unbuffered. `WRITE_BUFFER_BYTES` belongs to
  `ArchiveWriter`, which owns every open file, so buffering inside the seam would
  double-buffer in production and make the shipped seam behave differently from the
  double the tests substitute. The constant stays unused until the writer lands.
- Error messages echo a session id, never a `file_name` from the database. A session
  id is short, ours, and the thing a user needs to identify the archive; it is
  formatted with `{:?}` on the one path where it may not have passed the shape rule
  yet. A rejected `file_name` is exactly the value that may itself be a path, so the
  message states the rule instead of quoting it back.
- `RealArchiveFs::list_dir` turns a single entry's failed metadata read into
  `Err(UnreadableEntry)` inside the vector and keeps going, reserving `Err` for a
  directory that could not be listed at all. The `TestFs` double reaches the same
  outcome by injecting on the entry's name, so the shipped implementation is the one
  holding to the documented contract rather than inheriting it from the double.

## 2026-08-16 v0.3.0 Archive Test Hardening (Still Step 4a)

- Two review batches took the archive suite from 27 tests to 41 without writing a
  line of production behavior. Every body in
  `apps/desktop/src-tauri/src/archive.rs` is still `todo!("step 4b: ...")` and
  `pub mod archive;` is still the module's only reference in the crate, so nothing a
  user can reach changed. The batches were reviewed separately: nine tests in the
  first, five in the second.
- Evidence, from `apps/desktop/src-tauri`: `cargo fmt --all -- --check` clean,
  `cargo clippy --all-targets --all-features -- -D warnings` clean with zero
  warnings, and `cargo test --lib --all-features` at `117 passed; 40 failed;
  1 ignored; 0 measured; 0 filtered out` out of 158 run. `--list` reports 41
  `archive::tests` entries, so the archive suite is 40 red and 1 green. All 40
  failures panic at a `todo!("step 4b: ...")`; none is a failing assertion. The one
  green test is `the_test_filesystem_reports_a_link_as_a_reparse_point`, the test
  filesystem's own control, and it is supposed to pass. The group list and the body
  each test reaches are in `V0.3.0_PLAN.md` → Verification.
- Twenty-five of the 40 still stop at `ArchiveWriter::initialize`, so 4b is judged
  on each test going green individually rather than on the failure count. The other
  fifteen reach their own subject, which is what makes a first slice of 4b
  measurable at all.
- Why the suite grew, stated plainly so the count is not mistaken for padding. Each
  new test names a way the archive could have been wrong: a row naming another
  session's file; an index write refused while a file already exists; an orphan left
  by a failed cleanup; a reparse point measured into the quota or read as an
  archive; an entry whose metadata the filesystem refuses; an orphan whose delete
  is refused; a directory total that cannot be established; and `RealArchiveFs`
  never having been executed by any test.
- Four decisions this round settled, all now in `V0.3.0_PLAN.md`:
  - Read and delete stay keyed by `session_id`, and a row is accepted only when its
    `file_name` is the name that same session generates. Passing the name rule is
    necessary and not sufficient — otherwise one row could serve or delete another
    session's archive, which is the name rule's own failure one level up. Reviewed
    and kept rather than reverted.
  - A single entry the filesystem refuses is data, not an error. `list_dir` returns
    one result per entry, so one file another process holds exclusively becomes an
    anomaly the sweep reports and does not stop it; only a directory that cannot be
    listed at all is an error.
  - The measured quota total is known or unavailable, and unavailable means "no
    room". Under-counting a hard byte cap is how a tool fills someone's disk, so the
    archive refuses to grow a directory it cannot measure. It is not a fatal
    initialization failure and not a new row state: a session that tries to write
    closes `partial` / `quota-exceeded`, which is the only truthful reason the
    version 2 `CHECK` admits.
  - What each entry contributes to the total: an ordinary file its own length; a
    reparse point nothing; an unreadable entry with a row that row's last known
    `byte_size`; the same entry with no row nothing measurable, making the total
    unavailable; an orphan whose delete was refused its own length, because those
    bytes are still on the disk. "Definitely not our file" and "could not be read"
    are deliberately different rules, and the reparse-point test seeds a non-zero
    `byte_size` so an implementation cannot satisfy both with one branch.
- A test double that classified entries differently from the real filesystem would
  make every sweep test pass for the wrong reason, and until this round the double
  was the only implementation any test had executed. `RealArchiveFs` and the double
  are now compared over the same directory, entry by entry, for an ordinary file, a
  nested directory, a file symlink, and a directory symlink. Residual limit,
  recorded rather than papered over: no non-name-surrogate reparse point — cloud
  placeholder, dedup stub, `AppExecLink` — can be created through `std`, so those
  rest on both implementations reading `FILE_ATTRIBUTE_REPARSE_POINT` from the file
  attributes instead of `FileType::is_symlink`, which is true only for name
  surrogates. `list_dir` also had to promise ascending name order for the
  comparison to be an equality.
- Four of the 41 tests are `#[cfg(windows)]`. On a non-Windows host the archive
  suite is 37 tests with none green, because the one green test is Windows-only. CI's
  `desktop` job is windows-latest, so all 41 compile and run there.
- Test boundaries held: every test works inside a `tempfile::TempDir`, no archive
  test opens a database, the real application data directory is never opened, and no
  test sleeps. Failure injection is by call count or by entry name, never by timing.
- Working-tree scope is still the same nine paths, and nothing was committed,
  pushed, tagged, or released. No CI or release file was touched and no
  software-copyright application material was produced.

## 2026-08-15 v0.3.0 Archive Red Tests (Step 4a)

- Step 4 was split: 4a writes the archive API surface and its red tests, 4b writes
  the behavior after the user reviews the signatures. The reason for a split at all
  is that a Rust test cannot run in a crate that does not compile, so the API has
  to exist before the tests can. Every body in
  `apps/desktop/src-tauri/src/archive.rs` is `todo!("step 4b: ...")`, and the
  `pub mod archive;` line in `lib.rs` is the module's only reference in the crate,
  so nothing a user can reach changed.
- Evidence, from `apps/desktop/src-tauri`: `cargo fmt --all -- --check` clean,
  `cargo clippy --all-targets --all-features -- -D warnings` clean with zero
  warnings, `cargo test --all-targets` at `116 passed; 27 failed; 1 ignored;
  0 measured; 0 filtered out` for the lib target out of 144 run and
  `0 passed; 0 failed` for `main.rs`. All 27 failures are `archive::tests::*`,
  every one at a `todo!("step 4b: ...")`, and none passes. The full name list with
  the body each test reaches is in `V0.3.0_PLAN.md` → Verification.
- Fourteen of the 27 stop at `ArchiveWriter::initialize`, the first line of their
  arrangement, so their later assertions are written but not yet exercised. 4b is
  judged on each test going green individually, not on the failure count reaching
  zero.
- Two lint facts decided the shape of the file, and both were confirmed by probe
  rather than recalled. A `pub` item inside a private module, returned by a `pub fn`
  in a `pub mod`, does not trigger `private_interfaces` — so the module can be `pub`
  and every item in it is a dead-code root, which is why an unreached constant or
  method is not a warning under `-D warnings`. A private field no body reads *is*
  dead code and a derived `Debug` does not excuse it — so the stub structs are
  fieldless and the state 4b gives them lives in their doc comments.
  `#[allow(dead_code)]` was rejected as exactly the bypass marker `CLAUDE.md`
  forbids, and `#[expect(dead_code)]` needs Rust 1.81 against a declared 1.77.
- Four things the tests decided that the plan had left open, all now written into
  `V0.3.0_PLAN.md`: the gap line is singular-aware
  (`[RunCove: dropped 1 line / 1 byte]`); `QueueBounds` and `QuotaLimits` are
  parameters with documented defaults, so an overflow or eviction test costs a few
  hundred bytes instead of 200 MiB; the symlink-refusal test is `#[cfg(windows)]`
  and needs an elevated shell or Developer Mode, which CI's windows-latest runner
  has; and the write-failure test enqueues a record larger than
  `WRITE_BUFFER_BYTES`, because with a 64 KiB buffer a short record would not reach
  the file until close and the injection would fire at the wrong moment.
- No archive test opens a database. The index seam is a recording in-memory double,
  so the tests need only a `tempfile::TempDir`; the real application data directory
  is never opened. The `Storage`-backed `ArchiveIndex` is deferred to 4b, because a
  `pub(crate)` adapter that only tests use is dead code in the library build and
  would duplicate SQL that belongs in `storage.rs`.
- Working-tree scope is now nine paths: modified `AGENTS.md`, `HANDOFF.md`, this
  file, `apps/desktop/src-tauri/src/lib.rs`,
  `apps/desktop/src-tauri/src/models.rs`,
  `apps/desktop/src-tauri/src/storage.rs`, plus untracked `CLAUDE.md`,
  `V0.3.0_PLAN.md`, and `apps/desktop/src-tauri/src/archive.rs`. Nothing was
  committed or pushed, no CI/release file, tag, or remote state was changed, and no
  software-copyright application material was produced.

## 2026-08-15 v0.3.0 Scope Approval And Schema Migration

- Approved scope for v0.3.0: the opt-in run log archive only — writing, reading,
  the run history summary, the viewer, delete, and the documentation those
  require. Project Git status is deferred out of the release and will only be
  evaluated separately if it can be shown to provide value the editor and the Git
  client do not already provide.
- The SQLite schema upgrade from version 1 to version 2 was approved in principle,
  conditionally: red tests first, covering a complete version 1 upgrade, idempotent
  reopening, rejection of a version above 2, a failed migration leaving an openable
  version 1 database, and no regression in the existing queries and settings
  behavior. That condition is discharged — the tests were written and run red, then
  the migration was implemented.
- Red-test evidence, before the migration, `cargo test --lib` in
  `apps/desktop/src-tauri`: `109 passed; 6 failed; 1 ignored`. The six failures are
  `migration_is_idempotent_and_sets_version` (`left: 1, right: 2`),
  `a_populated_version_1_database_upgrades_to_version_2` (version stays 1),
  `reopening_an_upgraded_database_is_idempotent` and
  `the_archive_index_rejects_impossible_rows` (`run_log_archives` does not
  exist), `a_version_2_database_opens_and_only_a_higher_version_is_rejected`
  (version 2 is currently rejected as too new), and
  `a_failed_migration_leaves_the_version_1_database_intact` (opening succeeds
  today because nothing tries to create the table). `cargo fmt --all -- --check`
  and `cargo clippy --all-targets --all-features -- -D warnings` are clean in
  that crate.
- Honest limitation of that evidence: `version_1_user_data_survives_the_upgrade`
  was green rather than red, because a version 1 database already opened and read
  correctly on the pre-migration build; it is now the upgrade's data guarantee. And
  "the file is still openable by the previous build" is asserted by proxy — the test
  cannot run the v0.2.1 binary, so it checks that the version is still 1, that
  every version 1 row still reads, that no partial object was left behind, and that
  opening succeeds once the conflict is removed.
- Verification-matrix correction found while running this: the desktop crate is
  not a workspace member of the root package, so `cargo test --all-targets` at the
  repository root never covers `storage.rs`. CI runs the desktop Rust tests with
  `working-directory: apps/desktop/src-tauri`, and `V0.3.0_PLAN.md` now records
  that as a separate required invocation.
- Migration implemented the same day. Post-migration verification in
  `apps/desktop/src-tauri`: `cargo fmt --all -- --check` clean;
  `cargo clippy --all-targets --all-features -- -D warnings` clean with zero
  warnings; `cargo test --all-targets` at `116 passed; 0 failed; 1 ignored` for the
  lib target and `0 passed; 0 failed` for the `main.rs` target. The single ignored
  test predates this work. All six red tests are green and none was weakened,
  relaxed, or deleted to get there. The `npm` half of the matrix and
  `cargo tauri build` were not run because no frontend file changed; the full
  matrix is still owed before the milestone is called done.
- Production code changed, and nothing else: `models.rs` gained
  `RunLogArchiveSummary` and `RunSession.archive`; `storage.rs` gained
  `SCHEMA_VERSION`, the `> SCHEMA_VERSION` guard, `upgrade_to_version_2` and its
  call site, and the `list_sessions` `LEFT JOIN run_log_archives`. No writer, no
  commands, no frontend, no startup sweep.
- Defect the red tests caught, worth recording because reading the DDL did not
  reveal it: the constraint arms were written
  `status = 'partial' AND reason IN (...)`, which does not reject a null reason.
  `NULL IN (...)` evaluates to NULL, `1 AND NULL` to NULL, the surrounding
  `0 OR NULL OR 0` to NULL, and a SQLite `CHECK` passes when its expression is NULL
  instead of failing. The constraint therefore accepted exactly what it was written
  to forbid — a `partial` or `removed` archive with no reason, which the UI would
  render as a badge with no explanation. `the_archive_index_rejects_impossible_rows`
  reported it as
  `partial must carry a reason: ('sess-exited','a.jsonl','partial',NULL,...) was
  accepted`. Fixed with an explicit `reason IS NOT NULL` in both arms, in the
  migration and in the test's pinned `V2_ADDITION`, and pinned by a ninth rejection
  case for the `removed` arm.
- Second, smaller finding: that same test had been passing vacuously. Its eight
  rejection assertions all held while `run_log_archives` did not exist, and the run
  only failed at its final acceptance insert. An `object_exists` guard now runs
  before the loop, so a missing table fails at the guard rather than silently
  validating nothing. A rejection test needs a positive control.
- Migration direction, one sentence used unchanged in every document and release
  note: 迁移失败时 SQLite 事务回滚并保持 v1；迁移成功后没有应用级回退或数据库降级
  路径。In English: a failed migration rolls back the SQLite transaction and stays
  at v1; a successful migration has no application-level fallback and no database
  downgrade path. Concretely, a v1 database is opened unchanged by this build and by
  v0.2.1 with user data untouched, while a v2 database is refused by v0.2.1 because
  that build rejects any `user_version` above 1, and no code returns a version 2
  database to version 1. The two halves are not a pair and neither is a rollback of
  the other; do not call a successful upgrade revertible. A fresh install runs
  0 → 1 → 2 as two separate atomic transactions, each resumable on the next launch,
  while the version 1 to version 2 upgrade itself is a single transaction with
  `PRAGMA user_version=2` as its last statement.
- The startup sweep was deliberately kept out of this step even though the plan's
  execution order had bundled it with the migration. It re-measures archive files,
  repairs `writing` rows, marks `file-missing`, deletes orphan files, and
  initializes the quota counter — all of which require an archive directory that
  does not exist until the writer exists. It moves to the writer step and owes red
  tests together with the writer, the queue, the quota, and the lifecycle.
- Working-tree scope is now seven paths: modified `AGENTS.md`, `HANDOFF.md`, this
  file, `apps/desktop/src-tauri/src/models.rs`, and
  `apps/desktop/src-tauri/src/storage.rs`, plus untracked `CLAUDE.md` and
  `V0.3.0_PLAN.md`. No database file, CI/release file, tag, or remote state was
  changed, nothing was committed or pushed, and no software-copyright application
  material was produced.

## 2026-08-14 External Handoff And Registration Direction

- The user is temporarily handing RunCove to Claude for possible product
  expansion and may later return the resulting changes to Codex for review.
  `CLAUDE.md` now provides a concise cross-agent entry point while `AGENTS.md`,
  `HANDOFF.md`, and this file remain the authoritative project record.
- The intended use in a software copyright registration application is a future
  direction only. No jurisdiction-specific checklist, legal conclusion,
  submission package, feature scope, or version number has been approved.
  Current authoritative requirements must be checked when the user starts that
  work; the project should not be padded with meaningless code or features.
- The verified baseline remains published `v0.2.1`: local `main` and
  `origin/main` were synchronized at
  `97943d7fabbbd400481171568bf970b38a2c9afa`, and the release tag targets
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`.
- The project entered a waiting state on 2026-08-14. This handoff changes only
  project documentation. It does not authorize code, database, CI/release,
  tag, published-asset, runtime-process, or remote changes.
- The handoff documentation is intentionally uncommitted and unpushed. Its
  expected working-tree scope is five paths: modified `AGENTS.md`, `HANDOFF.md`,
  and this file, plus untracked `CLAUDE.md` and `V0.3.0_PLAN.md`. Superseded on
  2026-08-15: six paths after the red tests, seven after the migration. See the
  section above.
- `V0.3.0_PLAN.md` is a reviewed proposal for an opt-in on-disk run log archive.
  Its second draft answers an external review, drops project Git status to a
  deferred proposal, and gates implementation on three explicit approvals: the
  SQLite schema version 1 to version 2 migration, archive-only v0.3.0 scope, and
  whether project Git status is worth building at all. None has been granted, so
  no implementation is authorized. Superseded on 2026-08-15: scope approved, Git
  status deferred, the migration approved conditionally and then implemented after
  its red tests ran red.

## v0.2.1 Local Completion (2026-08-13)

- Implemented the approved `V0.2.1_PLAN.md` from `main` commit `bf3d532`
  without changing CI/release workflows. PR #2 passed every configured check
  and merged into `main` as `a771c55f402bcdce3d0ec29fe739d4e47bd847c5`;
  publication evidence is recorded separately after the tag workflow completes.
- Overview now shows the five newest managed run sessions, with a searchable
  and filterable drawer capped at 200 records. Orphaned history remains
  readable, known projects can be located, and future unknown stored statuses
  degrade to `Unknown`.
- Structured port conflicts carry an optional port/protocol through start,
  status events, and restore results. `View occupant` rechecks the snapshot and
  focuses only the exact protocol row; stale occupancy and stale error-action
  context are explicitly cleared.
- Saved-root discovery exposes scanning, candidates, empty, and error states,
  coalesces concurrent scans, preserves candidates while review is closed, and
  supports retry. Imports and starts always remain user-confirmed.
- Project editing can duplicate a launch profile without persistent IDs or
  observed-runtime state. Frontend and backend validation cover required
  fields, blank arguments, working directories, port range, and duplicate
  port/protocol pairs while retaining PATH-resolved programs such as
  `npm.cmd`.
- Port details can copy PID, executable path, and command line with explicit
  failure feedback. Help and all new controls have English and Simplified
  Chinese coverage, keyboard semantics, and accessible labels.
- No SQLite schema migration was added. Existing `run_sessions` data is reused;
  polling snapshots and console logs remain non-persistent.

### Fresh Verification

- Root Rust: format passed; Clippy passed with warnings denied; all targets
  passed `38/38` tests.
- Desktop Rust: format passed; Clippy passed with warnings denied; all targets
  passed `109` tests with the one explicitly configured live-service test
  ignored by default.
- Frontend: ESLint, TypeScript, and production Vite build passed. Vitest passed
  `114/114` tests across 20 files after the final audit fixes, including
  polling-safe port/project focus timers and partial-import candidate cleanup.
- Microsoft Edge Playwright passed `6/6`: the complete primary workflow at
  `900x600`, `1280x720`, and `1440x900`, plus profile copy/validation, exact
  conflict navigation, and saved-root failure/retry. All three viewports cover
  English and Simplified Chinese run history and Help with no captured console
  warnings/errors or horizontal overflow.
- `npm run tauri build` produced
  `apps/desktop/src-tauri/target/release/runcove-desktop.exe` as RunCove
  `0.2.1`. Size: `25,418,344` bytes. SHA-256:
  `4B00DD7F72B6AAD29646684DB7F852D691C7183A4BF78DA703C06556A9BA3A78`.
  The executable is unsigned and was built but not launched, installed,
  packaged, or published.
- Root and desktop Cargo metadata, npm metadata, Tauri metadata, and embedded
  executable file/product versions all report `0.2.1`. `git diff --check`
  passed.

### Residual Boundaries

- The desktop app remains Windows-first; only the port-inspection CLI is
  cross-platform in this version.
- RunCove cannot guarantee metadata for every protected kernel/system process,
  even after an explicit UAC relaunch. Administrator mode remains monitor-only.
- Historical session metadata does not include archived console logs. The one
  environment-driven live-services acceptance test still requires explicit
  fixture configuration and remains ignored in the normal suite.
- The local release executable has no code signature. No installer, startup
  automation, Docker/remote management, `.env` editing, or persistent log
  archive was added.

## v0.2.1 Publication (2026-08-13)

- PR #2 merged into `main` as
  `a771c55f402bcdce3d0ec29fe739d4e47bd847c5`. Main CI run `31691103911`
  completed successfully across all configured CLI, Rust lint, frontend,
  browser, and Windows desktop checks.
- Release documentation was recorded without changing the code tree. Annotated
  tag `v0.2.1` targets release commit
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`.
- Release workflow `31692200475` completed successfully and published the
  latest, non-draft, non-prerelease release at
  <https://github.com/AbyssWhalen/RunCove/releases/tag/v0.2.1>.
- The release contains six assets: four cross-platform CLI archives, the
  Windows x64 portable desktop archive, and `SHA256SUMS.txt`. GitHub's SHA-256
  digest for every archive matches the corresponding checksum-file entry:
  - Linux x64 CLI: `109FE838A097D4312434FB4D4597DF9110E9A9BFCF8D0FC37A8C676284BA5A7F`
  - macOS arm64 CLI: `CD689CD9635A40C4D243CEC2759BE45DD796FC343FD1869B685D7E4826114EEE`
  - macOS x64 CLI: `4D287D0EDC27C0C76D527124B97634FF9B972062FB9E5699C35F509D98B6E688`
  - Windows x64 CLI: `7C2DB4C3BA9DE7641DF92F5363D478BBACC2BCF206F8614A6B075FD37F45676A`
  - Windows x64 desktop: `58CAB696B97FAE7EDF5E273FA57D32984F826B01F3473FE7AB008CEBF8436096`
- A redundant main CI run created by the no-content release marker was canceled
  because the identical code tree had just passed run `31691103911`; no job was
  manually rerun. Release annotations only report GitHub's Node.js 20 action
  deprecation warning and did not affect the successful build or publication.
- Local Git HTTPS pushes triggered a Windows `git-remote-https.exe` crash dialog.
  Further publication writes used the authenticated GitHub API instead, and no
  Git or Git remote helper process remained afterward. No unrelated application,
  project, or development process was stopped or modified.

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

## Initial Verification Log (2026-08-07)

- Unchanged baseline formatting and Clippy checks passed after dependencies were
  available. The unchanged full test suite timed out because Windows process
  names were resolved by spawning `tasklist` for every scanned entry; the
  fixed-port kill integration test was also unsuitable for a shared machine.
- Root CLI verification after the scanner and compatibility work:
  - `cargo fmt --all -- --check`: passed.
  - `cargo clippy --offline --all-targets --all-features -- -D warnings`:
    passed with warnings denied.
  - `cargo test --offline --all-targets`: passed, 34 tests total.
- Both the primary and compatibility CLI help paths executed successfully.
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
    a longer soak was not completed before `v0.2.0` publication and remains a
    post-release performance follow-up.
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

- Formal trademark, domain, crates.io, and npm registry clearance for `RunCove`
  was not part of the `v0.2.0` repository release. Repeat name clearance before
  wider package-manager or commercial distribution.
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
  the compatibility CLI entry point, and `src/cli_app.rs`. The remaining 117 new paths are
  under `apps/desktop`, including the complete frontend/backend application, 53
  generated Tauri icon assets, and 17 Playwright QA screenshots.
- Root changes cover crate naming, shared primary/compatibility CLI entrypoints,
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
  - Windows CLI zip contains the primary and compatibility executables, plus the same three
    documents; SHA-256
    `13D11F813CE72FB9164B7C4452F925E106938C89559CAE15CACE710A4FEF141C`.
- CI/release workflows now cover the desktop and new artifact names. GitHub is
  the authoritative workflow parser because no local YAML checker is installed;
  at this checkpoint publication was incomplete pending PR CI, merge, tag,
  workflow, and asset download verification.

## 2026-08-11 Windows Resource Linker Follow-up

- PR CI run `31493311756`, while PR #1 was still a draft, reached the desktop
  Rust test link and failed
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

- Commit `c281340` was pushed once while PR #1 was still a draft. CI run
  `31501585406` passed
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

## 2026-08-12 v0.2.0 Publication

- PR #1 merged into `main` as
  `9b935857fcc79b2811a5a1fb16df9aae55a91e7a`. PR CI run `31561867655` and the
  resulting `main` CI run `31562443457` both completed successfully across the
  configured CLI and Windows desktop jobs.
- Annotated tag `v0.2.0` points to `9b93585`. Release workflow run
  `31563084142` completed successfully and published the non-draft,
  non-prerelease latest release at
  <https://github.com/AbyssWhalen/RunCove/releases/tag/v0.2.0>.
- The published release has six assets: Linux x64, macOS x64, macOS arm64, and
  Windows x64 CLI archives; the Windows x64 portable desktop archive; and
  `SHA256SUMS.txt`. Freshly downloaded asset digests matched GitHub metadata,
  all five archive hashes matched the checksum file, and every archive extracted
  with the expected files and executable format.
- Verified archive SHA-256 values:
  - Linux x64 CLI: `817BE042DCBCB3C747E5E08E450E7B4C2957FB00BDDC8BEA247F8B29C611A3E2`
  - macOS arm64 CLI: `795671AA37EB384FD943C8C8AA95527F90878902486113FD181A3AF6E84F1E87`
  - macOS x64 CLI: `6401B3B126358708B39F6E4157EB085B2DE0E91A74E1339C8309740F078F98CF`
  - Windows x64 CLI: `CFE0090E62180D7779A76418A57FE513746B06EEB6A86E5DA2A672BAAF9B5041`
  - Windows x64 desktop: `BE99C188B6DEF910D5DE4ADEA02028EF8410D0D46AD6B500FF2095D341BF6A7E`
- The published Windows desktop executable is `12,692,992` bytes, reports file
  and product version `0.2.0`, and is unsigned as documented. The CLI archives
  contain the primary and compatibility CLI executables; release-package compatibility checks
  matched JSON, range-filter, and error exit-code behavior.
- `gh release download` could not validate the local certificate chain
  (`x509: certificate signed by unknown authority`). Public release URLs were
  therefore downloaded with Windows `curl.exe`, then independently hashed and
  inspected. This was a local client trust-store limitation, not a release
  failure.
- The public repository is now named `RunCove`; its historical v0.1.0 release
  remains available. Its description and topics identify the current product.
  No force-push, history rewrite, larger runner, or removal of the old release
  occurred.

## 2026-08-12 Repository Branding Alignment

- GitHub renamed the public repository to `AbyssWhalen/RunCove`. The local
  `origin` remote and Cargo package metadata now use the canonical URL
  `https://github.com/AbyssWhalen/RunCove`.
- The repository description now says: `RunCove is a Windows-first desktop app
  for monitoring local ports, launching npm projects, managing process trees,
  viewing logs, and restoring development services.`
- README, changelog, release notes, handoff, and implementation notes now use
  RunCove as the public product name. The compatibility CLI remains in code and
  tests for existing scripts, but new documentation presents `runcove` as the
  only primary command.
- The existing `v0.2.0` tag, six release assets, historical `v0.1.0` release,
  and CI/release workflows were not rebuilt or rewritten. Root Rust format,
  warnings-denied Clippy, and all root targets passed after the metadata and
  unsupported-platform message update.
