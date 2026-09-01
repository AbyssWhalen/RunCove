# RunCove Handoff

## 2026-09-01 v0.4.1 Released

**v0.4.1 is published, and the user delegated the decision to release it.** They had
twice been told the call was theirs; on 2026-09-01 they answered "没有问题的话你就自行
确定发布吧" after asking for a review first. So the review came first and the release
followed from it — not the other way round.

It is a patch release by the semver reading that actually applies: every change fixes
behavior, none adds a feature. The five version files moved together because
`validate-version` refuses a tag that disagrees with any of them — root and desktop
`Cargo.toml`, both `Cargo.lock` entries, `package.json`, `package-lock.json`, and
`tauri.conf.json`.

**The review found one release-blocking defect that no test could have caught.** The
workflow publishes with `--notes-file RELEASE_NOTES.md`, and that file still said
`# RunCove v0.4.0` throughout. Tagging without rewriting it would have put v0.4.0's
notes on the v0.4.1 release page — a defect in the *release*, invisible to fmt, clippy,
every test, and CI. Check that file against the tag before every future release; it is
the one artifact the workflow reads and nothing validates.

**Run 33479967618 published it: all seven jobs green, and the artifacts were verified by
real download afterwards.** `sha256sum -c SHA256SUMS.txt` now passes all five lines with
a plain invocation and no workaround, on files fetched from the release page — `od -c`
shows exactly two spaces after each hash. That was the checksum fix's whole purpose, and
it is now demonstrated on a published artifact rather than argued from the workflow. The
release page carries v0.4.1's own notes, and the desktop exe inside the zip reports
`FileVersion 0.4.1`. The release-asset CDN aborted the desktop zip mid-transfer once, as
it has before on this machine; a plain retry completed it, and the CLI archives all
matched their release metadata on the first try.

**The size claim in the first published notes was wrong, and it was corrected after
publication.** They said the download was "about a third smaller — roughly 8.6 MB against
13 MB". The measured truth, builder-to-builder, is 13,212,672 → 11,978,752 bytes: 9%
smaller, about 91 KB off the compressed zip. The error was pairing the *local* post-fix
size (8,557,568) with the *published* pre-fix size (13,212,672) — two different machines
on two different compilers, across the very 1.96x local-vs-CI gap this repository had
already recorded as unexplained. Having flagged that gap and then read across it is the
part worth remembering: **a before/after size number is only meaningful when both halves
come from the same builder.** Both releases were built by rustc 1.98.0 on
`windows-latest`; local was 1.96.0.

What the profile actually buys on that builder, measured on both exes: the `.pdb`
reference and its `RSDS` signature survive in both, because MSVC keeps debug info in a
separate file and `strip` has almost nothing to take out of the exe — so the win is
`lto` and `codegen-units = 1`, visible as cargo-registry path literals halving from 368
to 186. The profile is still worth keeping; it is simply worth 9% here, not 67%.

**`main` is one commit ahead of the tag on purpose.** `v0.4.1` is at `3f7be41`, which
the artifacts were built from; the size correction above landed afterwards as a
docs-only commit. A published tag does not move, and unlike v0.4.0's situation no
shipped code differs between them.

Three findings from reading the shipping code, recorded because they are properties
rather than changes:

- **The overlap fix cannot deadlock.** `start_walk_member` takes exactly one
  reservation and drops it at the end of its own body, before `restore_profiles` reaches
  the next member, so a walk never holds one reservation while waiting for another. Two
  groups started in opposite orders therefore cannot lock each other — hold-and-wait,
  one of deadlock's four necessary conditions, is absent by construction. This follows
  from the per-member reservation granularity, which was a deliberate choice rather than
  a convenience.
- **Three groups contending for one member can exceed the handoff budget.** The third
  waiter may wait past `RESERVATION_HANDOFF_TIMEOUT` (25s) and fail with "Another
  lifecycle operation is still in progress for this profile". That is honest reporting of
  real contention and better than an unbounded wait; it is a known degradation, not a
  defect.
- **The UI's restore↔group and group↔group rules are deliberately asymmetric.** Restore
  and a group action exclude each other, while two groups may run at once. This is not an
  oversight: the shipped `LaunchGroupSection.test.tsx` asserts a second group's Start
  stays enabled, and the backend now waits, so both are safe. Both guards are courtesy,
  not correctness.

**The checksum fix was verified by evidence, and the local simulation deliberately does
not count.** Git Bash's `sha256sum` defaults to binary mode and emits `HASH *name`,
while the `publish` job runs on `ubuntu-latest` where GNU coreutils emits `HASH  name`
— so the old `sed` never misfired locally and a local test cannot reproduce the defect
or validate the cure. What does count: the actually-published v0.4.0 `SHA256SUMS.txt`
was read byte by byte and has **three** spaces after the hash where GNU requires two,
which confirms the diagnosis on the artifact itself; and the fixed workflow runs
`sha256sum -c` on the file it just wrote, in the same job on the same runner, so a wrong
format fails the release instead of shipping. Prefer that shape — self-verification in
the target environment — over a local reproduction whenever the two disagree.

## 2026-08-31 Post-Release Review Fixes

**`main` carries unreleased fixes on top of the `v0.4.0` tag.** The release itself is
unchanged — the section below still describes it — but a review pass after publication
found and fixed a concurrency defect and a release-workflow defect, so the tag and `main`
no longer have identical behavior. `CHANGELOG.md` has an `[Unreleased]` section; `notes.md`
has the reasoning. The four version manifests stay at `0.4.0` deliberately: a version
number means something only at an actual release, and that needs the user's word.

**The overlap defect is fixed in the backend, which is where it belonged.** Two things
start launch profiles in order — `restore_last_run_set_inner` and
`start_launch_group_inner` — and both walk `restore_profiles` taking a per-profile
reservation. Overlapping membership is legitimate (a restore set is whatever ran last; two
groups may share a database), so the second walk used to fail on the shared profile.

- `ProcessManager::try_reserve` (`processes.rs:325`) returns `Ok(None)` for a profile
  another operation holds and still `Err` for a shutdown. It exists because `AppError`
  carries no discriminator, so the alternative was matching on message text. `reserve` is
  now expressed through it, so every existing caller and the error string are unchanged.
- `reserve_walk_member` (`commands.rs:419`) polls it to the `RESERVATION_HANDOFF_TIMEOUT`
  deadline, and `start_walk_member` (`commands.rs:402`) then takes the ordinary start path.
  **Both walks call it** — a restore and a group behave identically for the identical
  situation, which is the point. A shutdown is returned at once rather than waited out.
- The handoff timeout is derived as `PROFILE_READY_TIMEOUT_SECS + 5`, not written as its own
  number: the operation being waited on may spend its whole readiness budget, so a waiter
  with an equal budget could report a failure for a start that had not failed.
- Whole-group stop got the same treatment, plus a second `processes.info` check after the
  wait, because the operation it waited out may have been the stop that member needed.
  `stop_profile_inner_reserved` (`commands.rs:300`) was split out for it, mirroring the
  `start_profile_inner` / `start_profile_inner_reserved` pair that already existed.
- Four timeout literals became named constants (`PROFILE_READY_TIMEOUT`,
  `PROFILE_STOP_TIMEOUT`, `STATE_POLL_INTERVAL`). `stop_all_and_wait`'s 8 seconds was left
  a literal on purpose: it is a total budget for every tree at once, not a per-profile one,
  so naming it with `PROFILE_STOP_TIMEOUT` would be wrong.

Two Rust tests cover it: `try_reserve_separates_a_held_profile_from_a_refusal`
(`processes.rs`) and `a_walk_member_another_operation_holds_is_waited_for_rather_than_failed`
(`commands.rs`). The second was mutation-checked — reverting `reserve_walk_member` to a
plain `reserve` makes it fail with the old message — so it is not vacuous.

**An earlier frontend attempt at this was withdrawn, and the reason it failed is worth
keeping.** A guard that disabled the second group's button conflicted with a deliberate
assertion in `LaunchGroupSection.test.tsx` (its fixture's `Morning stack` and
`Everything down` share `web`, and the test asserts the other group stays enabled). The
backend fix leaves that assertion true, which is the sign it is at the right layer. No
existing assertion was changed at any point.

The frontend guards from that pass **stay**, with a different justification: they now stop
the user queueing work already underway rather than preventing a failure. That is the
better layering — correctness in the backend, courtesy in the UI.

- A restore and a whole-group action cannot be started at the same time, in either
  direction, at both the latch (`App.tsx`, `restoreLastRunSet` refuses while
  `groupActionsInFlight` is non-empty; `runProfileAction` refuses a profile a group holds)
  and the button (`OverviewView.tsx` restore button takes `busyGroups.size`;
  `LaunchGroupSection.tsx` takes a `restoreBusy` prop). The latch matters on its own because
  the tray's restore item does not go through the button.
- `profileLabel` moved above `restoreLastRunSet`, became a `useCallback` reading a new
  `latestSnapshot` ref, and now names the profile in a failed restore instead of printing
  its raw id. **The ref is load-bearing, not tidiness:** closing over `snapshot` makes the
  callback change identity every poll, which re-subscribes the tray effect once a second and
  fails two existing tests. Do not "simplify" it back.

**The release workflow's checksum step was wrong and is fixed** (`release.yml:172`). It ran
`sha256sum ./*.zip ./*.tar.gz | sed 's# \./#  #'`, and since `sha256sum` already emits two
spaces before its `./name`, the substitution produced *three* — one more than
`sha256sum -c` accepts, so the published file failed on a perfectly good download. It now
runs `sha256sum -- *.zip *.tar.gz`, which needs no post-processing, and then
`sha256sum -c SHA256SUMS.txt` before publishing, so the same class of defect cannot ship
again silently. `RELEASE_NOTES.md` gained the actual verification commands, including
`--ignore-missing` for a partial download set. The v0.4.0 file that is already published
cannot be changed; its workaround stays in `CHANGELOG.md` as the sole known limitation.

## 2026-08-31 v0.4.0 Published

**RunCove v0.4.0 is released.** PR #4 is merged, `main` and `origin/main` are at the merge
commit `0d6b934`, the annotated tag `v0.4.0` points at that same commit, and the GitHub
Release is live and marked latest at
https://github.com/AbyssWhalen/RunCove/releases/tag/v0.4.0 with six assets. The working
tree is clean. Everything below this section describes states before the release; read this
one for where things stand.

The release is launch groups plus the three defects fixed alongside them — localized stop
and exit reasons, project-editor accessible names, and a project's saved-at time. Both are
recorded in `CHANGELOG.md` under `[0.4.0] - 2026-08-31`.

Eight commits reached `main`, the last three of them release work:

| Commit | Contents |
| --- | --- |
| `f8a2447` … `842efb9` | the four feature and fix commits described in the section below |
| `f90b8a6` | `chore(release):` `0.4.0` in four manifests, three lockfiles regenerated, `CHANGELOG.md` closed, `RELEASE_NOTES.md` rewritten, `README.md` un-`main`-ed |
| `9e6ea53` | `docs:` the upgrade note now covers the v0.2.1 → v0.4.0 path, not only v0.3.0 → v0.4.0 |
| `0d6b934` | the `Release RunCove v0.4.0 (#4)` merge commit — **no AI marker in its body**, unlike `bd2b777` |

**CI was green on every commit that mattered, and the release workflow ran clean.** Branch
run [33380718489](https://github.com/AbyssWhalen/RunCove/actions/runs/33380718489) on
`f90b8a6` proved the version bumps and the three regenerated lockfiles — all five jobs
`success`, `Windows desktop` 9m13s. Run
[33382118980](https://github.com/AbyssWhalen/RunCove/actions/runs/33382118980) on the tip
`9e6ea53` was green the same way (`Windows desktop` 10m19s). Release run
[33383018390](https://github.com/AbyssWhalen/RunCove/actions/runs/33383018390) passed all
seven jobs, `Validate release version` included, so the tag matched all four manifests.

**The published archives are verified end to end, by download, to the same standard v0.3.0
got.** This took two attempts and the first one's method is worth keeping. For most of
release day `release-assets.githubusercontent.com` refused every connection from this
machine (`gh release download` and `curl` both reset on ~60 tries, while `github.com` and
`api.github.com` worked), so the artifacts could not be fetched; the substitute was to diff
the checksums the workflow computed on the runner — the literal bytes of the published
`SHA256SUMS.txt`, read out of the `Publish GitHub release` job log — against GitHub's own
`digest` for each stored asset, and all five matched. Later the same day the asset host
recovered (`http=200`), all six assets were downloaded, and `sha256sum -c` reported `OK` for
all five archives, matching both the digests and the five sums recorded in `notes.md`. The
desktop zip was also opened: it carries `runcove-desktop.exe` at `FileVersion 0.4.0`, plus
`README.md` byte-identical to `git show v0.4.0:README.md`, `CHANGELOG.md` headed
`[0.4.0] - 2026-08-31`, and `LICENSE`.

**One real defect surfaced in that check, and it is in the published artifact.** Running
`sha256sum -c SHA256SUMS.txt` on the published file **fails all five lines** with
`No such file or directory` — not because any byte is wrong, but because `release.yml`'s
`sed 's# \./#  #'` leaves **three** spaces between the hash and the filename where
`sha256sum` requires exactly two, so the extra space is read as part of the filename. The
bytes are fine. State the scope precisely: neither `README.md` nor `RELEASE_NOTES.md` prints
a `sha256sum -c` command — `RELEASE_NOTES.md:66` says only "verify the archive against
`SHA256SUMS.txt`" and `README.md` does not mention the file — so no document gives a wrong
instruction. What is wrong is that the obvious way to follow the one instruction there is
reports five `FAILED` lines on a perfectly good download. Normalizing works:
`sed -E 's/^([0-9a-f]{64})[[:space:]]+/\1  /' SHA256SUMS.txt | sha256sum -c -`. Fixing the
`sed` belongs to `.github/workflows/release.yml`, which is unauthorized, and a published
release's asset cannot be corrected in place anyway — so this is a **v0.5.0 fix plus a
release-note line**, not something to patch now.

**The local `apps/desktop/src-tauri/target/release/runcove-desktop.exe` was rebuilt on
2026-09-01 and now reads `FileVersion 0.4.0`.** It had been a stale `0.3.0` build, predating
the version bump, which was a trap worth removing. Note what it is and is not: it is
`main` **including the unreleased fixes above**, so it is not byte-comparable with the
published v0.4.0 zip and is not the artifact users get. For testing the fixes it is the
right build; for reproducing what a user sees, use the downloaded portable zip at
`D:\tmp\runcove-v040-dl\`.

The published release body was also diffed against `git show v0.4.0:RELEASE_NOTES.md` and
matches apart from one trailing newline the API adds.

**Operational state for actually using it.** The pre-upgrade backup sits at
`%LOCALAPPDATA%\RunCove-backup-v0.3.0-2026-08-31\runcove.sqlite3`, and both it and the live
database were re-read on release day at `user_version = 1` — so this machine's real
database has still never been opened by a v0.3.0 or later build, and the first v0.4.0 launch
will migrate it 1 → 2 → 3 in one go. The constraint that
`apps/desktop/src-tauri/target/release/runcove-desktop.exe` must not be launched has now
served its purpose and lapsed with the release: v0.4.0 is published, so opening a v0.4.0
build against the real database is the intended next step rather than something to avoid.
Prefer the published portable zip over that local build, since the zip is the artifact
users get. Nothing here has launched either one.

**That migration was rehearsed twice on 2026-09-01 against copies of the real database, and
it succeeded both times.** The second rehearsal is the one that counts, because it ran the
shipped binary rather than a library call.

1. *Library path.* The live file was copied (the original opened read-only, never written)
   to `D:\tmp\migration-check\runcove.sqlite3` and handed to `Storage::open` — the exact
   call `lib.rs:240` makes at startup — through a temporary test in `storage.rs` that was
   deleted afterwards, leaving that file byte-identical to its committed state.
2. *Whole application.* An isolated build (`com.abysswhale.runcove.verify0901`, with
   `INSTANCE_MUTEX_NAME` changed to match so it could not collide with a production
   instance) was produced, its data directory seeded with another copy of the live
   `user_version = 1` file, and launched. It came up with a window titled `RunCove`, 78
   threads, 96 MB working set, and **migrated the seeded database 1 → 3 on startup**. Both
   patched files were restored with `git checkout --` afterwards and the tree is clean.

Both runs ended at `user_version = 3` with all three tables the two migrations add
(`run_log_archives`, `launch_groups`, `launch_group_members`) present, the 4 run sessions and
2 settings rows intact, and every reader (`settings`, `list_projects`, `list_launch_groups`,
`list_sessions`) working. The live file was re-checked after each and is still at
`user_version = 1`.

This is also the first evidence in this project that the built application *starts* — every
other check is a test. Keep the distinction when citing it: the tests prove the units, the
isolated launch proves the binary boots and migrates.

**A test gap found the same day, and it was in the release's headline feature.** Nothing
covered a launch group actually starting real processes. The three group tests in
`commands.rs` inject a closure, refuse an empty group, and report a port conflict; the e2e
suite drives `mock-data.ts`; the isolated launch clicked nothing. So the path a user takes —
press Start, the stack comes up in order — had never been executed by any automated check in
a release whose one feature is launch groups.
`a_whole_group_starts_real_processes_in_order_and_stops_them_together` (`commands.rs`, Windows
only, spawns `node.exe`) now covers it, and the desktop crate is **253** tests as a result, so
a document citing 252 predates it.

Read the assertion set before changing it: checking `processes.info` alone would pass on a
walk that never waited for readiness, because `info` is populated at spawn. The test instead
asserts a fresh `TcpListener::bind` is refused on *both* members' ports after the walk
returns, which can only hold if every member is still up when the last one finishes. It also
pins that a second start reuses the same PIDs, which is what makes the button safe to press
twice. Mutation-checked: walking the stop forward instead of in reverse fails it.

**What that measurement also settled: there is almost nothing at risk.** The real database
holds 0 projects, 0 launch profiles, 0 expected ports, 0 port associations, 0 restore-set
rows, 4 run sessions, and 2 settings rows, and was last written on 2026-08-11. Both survived
the rehearsal intact. So the irreversibility is real and the tag-vs-build asymmetry still
matters, but "irreversible migration of the user's data" should not be described as risky
here — say instead that it is irreversible, was rehearsed successfully on a copy, and has
essentially no user data to lose. The backup stays regardless.

**P3 is closed, and only one half of it was carried out.** The user authorized both halves
on 2026-08-31 and the decision below is the one that came back from actually looking:

- **The three stale remote branches are deleted.** `origin` now holds `main` and nothing
  else. All three were already ancestors of `main`, so no commit was lost and no history
  was rewritten: `feat/launch-groups` at `9e6ea53`, `codex/release-v0.3.0` at `4ca80a4`,
  `codex/runcove-v0.2.0` at `0a14cea`. Any of them can be recreated exactly with
  `git push origin <sha>:refs/heads/<name>`. Two of the three carried a tool's name in the
  branch name, which was publicly visible on the repository; that made deleting them worth
  more than tidiness alone.
- **The agent-facing plan documents stay tracked.** `V0.2.1_PLAN.md` and `V0.3.0_PLAN.md`
  were to be moved out of the tracked tree, and they are not, because `notes.md` and this
  file cite `V0.3.0_PLAN.md` about forty times *with line numbers* — the decision records
  point into it as evidence. Untracking it would leave a clone with forty references to a
  file it does not have, which contradicts the standing rule that decisions and verification
  evidence stay easy for another reviewer to audit. Moving only `V0.2.1_PLAN.md`, which is
  cited three times and never by line, would buy one filename and cost the consistency of
  the pair. Reopen this only with a plan that also rewrites the citations.

**RunCove is now installed on this machine, and the first launch is still the user's to
make.** Asked how to actually use it, the answer was that nothing was installed: `tauri.conf`
has `bundle.active: false` with no targets, so this project produces **no installer** by
design and the published artifact is a portable zip. So the exe was copied to
`%LOCALAPPDATA%\Programs\RunCove\RunCove.exe` alongside the same three documents the zip
carries, and Start Menu and Desktop shortcuts were created with `WScript.Shell` and verified
to resolve to it with the right working directory. To remove it: delete that folder and the
two `.lnk` files. It was handed over unlaunched, because a production-identifier launch
migrates the real database `1 → 3` irreversibly and that was the user's call to make.

**The user then made it, and the migration succeeded on the real database.** The installed
app started at 13:56:51 on 2026-09-01 and the live file was at `user_version = 3` by
13:56:52 — ten tables, `launch_groups`, `launch_group_members`, and `run_log_archives` all
created, and the row counts unchanged at 4 run sessions and 2 settings. The backup is
untouched at version 1
(`%LOCALAPPDATA%\RunCove-backup-v0.3.0-2026-08-31\runcove.sqlite3`, last written
2026-08-11).

So update three things when reading older notes. The irreversible step is **done**, not
pending. The live database is **3**, not 1 — every claim that it "is still at 1" describes
the state before 13:56:51. And the one-way property is now load-bearing rather than
theoretical: **v0.3.0 and earlier can no longer open this file**, and restoring the backup
is the only way back, at the cost of the sessions written since. What this also settles is
that the two rehearsals predicted the real outcome exactly, which is the argument for
rehearsing on a copy rather than reasoning about it.

**The desktop executable had been missing the project's release settings, and it is the
artifact users download.** Installing it is what surfaced this: the local exe was 25,912,797
bytes against the 13,212,672 published with v0.4.0, and a gap that size at the same version
was worth chasing. The published binary was then checked directly — it carries a
`runcove_desktop.pdb` reference and 632 source-path strings where the new build carries zero
and 288, and a `strip` is what removes the debug directory naming that PDB. Note that **most
of the size reduction is LTO rather than `strip`**, since the release profile already
defaults to `debug = false`; and the 1.96x gap between the two builds that *both* lacked the
profile is measured but unexplained, with the pre-change binary overwritten and so no longer
testable. `notes.md` keeps that open rather than guessing. The root `Cargo.toml` carries `[profile.release] strip / lto /
codegen-units`, but it has **no `[workspace]` section**, so `apps/desktop/src-tauri` is an
unrelated package and cargo never reads the root profile when that manifest is the build
root. The CLI binaries got strip and LTO; the desktop application got neither. Fixed by
repeating the block in `apps/desktop/src-tauri/Cargo.toml` with a comment explaining why it
is duplicated — do not "clean up" that duplication without creating a real workspace, which
was rejected here as moving both packages' target directories and lockfiles to tidy six
lines. The exe is now **8,557,568 bytes**, 0.65x the published v0.4.0.

The build cost is worth stating precisely, because the first measurement overstated it.
The build immediately after the change took 3m22s, but that is the one where every
dependency recompiles under a profile it had not been built with; the next full release
build took **1m53s** against roughly 1m34s before. So the steady-state cost is about
twenty seconds, and CI pays the larger number once. The workflow sets no
`timeout-minutes`, so GitHub's six-hour default applies and neither figure is a risk.

State the verification this way, because the obvious check does not apply: **`cargo test`
cannot validate this change at all**, since tests build under the `test` profile. The
isolated-build recipe was used instead — identifier and `INSTANCE_MUTEX_NAME` patched to
`lto0901`, data directory seeded with a `user_version = 1` copy, launched, then both files
restored with `git checkout --`. It came up titled `RunCove` with a live WebView2 renderer,
1,415 distinct colours sampled across its 1295×800 client area, and migrated the seeded
database to 3 with its 4 sessions and 2 settings intact. The colour count is the load-bearing
assertion: a binary whose embedded frontend assets had been damaged by `strip` would still
migrate the database and still show a window, just an empty one.

**About 24 GB of stale build staging is gone, and four older notes now cite paths that do not
exist.** With the user's approval on 2026-09-01, thirteen one-off staging directories under
`apps/desktop/src-tauri/target/` — `ci-resource-fix` (5.5 GB), `msvc-resource-test` (3.5 GB),
`msvc-resource-release`, `accidental-nested-build-20260811`, `ci-resource-split-release`,
`rustup-msvc`, and the seven `final-20260810*` / `final-20260811*` directories — plus
`suite.txt`, `suite2.txt`, and four v0.2.0 verification artifacts in the root `target/` were
deleted. `debug` and `release` in both trees were kept. `du` had estimated 26.9 GB and the
drive gained 24.1 GB; the difference is cargo hardlinking inside a target tree, which makes
`du` count some files twice. All of it was gitignored build output that cargo regenerates.

The four affected citations are `HANDOFF.md:3232`, `HANDOFF.md:3321`, `notes.md:2819`, and
`notes.md:2860`, each naming a `final-20260811-*/release/runcove-desktop.exe`. They are left
as written rather than edited, the same way the deleted `D:\tmp\` scratch directories were
handled: they are historical records of where a build stood at an August checkpoint, and
rewriting a past record to match present disk state loses more than it fixes. Read them as
"this is where that build was", not as a path to open.

Four orphaned isolated data directories went too — `com.abysswhale.runcove.demo0819`,
`.demo0831`, `.lto0901`, and `.qa`, about 146 MB. Notes measuring those databases
(`HANDOFF.md:476` reads "demo0819 remains at 2, qa at 1") describe what was true when
written; the directories are gone, and the isolated-build recipe recreates one in minutes if
a measurement needs redoing.

**Still unauthorized, each needing its own ask:** any rewrite of published history (the
`[codex]` marker in `bd2b777` and the two `[Qoder]` markers stay) and any change to
`.github/workflows/`.

**The scratch space is gone.** The four temporary working directories under `D:\tmp\` —
`runcove-v030-demo`, `runcove-v030-release-verify-20260822-153954`, `runcove-v040-demo`,
`runcove-v040-verify`, about 59 MB — were deleted on 2026-08-31 with the user's approval.
They held the two isolated-identifier demo builds, v0.3.0's downloaded release archives, and
this release's verification files; nothing in the repository depended on them, the isolated
builds are reproducible, and the five checksums that mattered are written into `notes.md` so
the verification is still re-checkable. So an older note that points at
`D:\tmp\runcove-v030-demo\RunCove-demo.exe` is describing a file that no longer exists.

## 2026-08-31 Committed, Pushed, And Open As PR #4

The launch-group work is on **`feat/launch-groups`**, pushed to `origin`, and open as
[PR #4](https://github.com/AbyssWhalen/RunCove/pull/4) against `main`. `main` and
`origin/main` are **still at `cae4d28`** — nothing is merged — and no version file was
touched, so all five manifests still read `0.3.0`. The working tree is clean. The section
below this one describes the same feature before any of it was committed; read this one
for where the code now lives.

Four commits, chosen so that **each one builds on its own**:

| Commit | Contents |
| --- | --- |
| `f8a2447` | `chore:` the `AGENTS.md` matrix fix — both Cargo packages spelled out |
| `fc56693` | `feat:` all 30 code paths: schema 3, storage, commands, the whole frontend |
| `6b80f61` | `docs:` `CHANGELOG.md`, `README.md`, `HANDOFF.md`, `notes.md` |
| `842efb9` | `fix:` the project-editor accessible-name defect and its regression test |

**P1 and P2 could not be separated**, and the reason is measured rather than preferred:
`models.rs`, `commands.rs`, `App.tsx`, and `messages.ts` each contain both the run-status
`reason` additions and the launch-group additions, interactive `git add -p` is unavailable
here, and a hunk-level split would produce commits that do not build. `fc56693`'s body
says so, so the decision is auditable from `git log` without this file.

**The accessible-name defect is now fixed** — the item the previous section carried as
still open. Five fields in `ProjectModal.tsx` wrapped their input in a `<label>` that also
renders the field's validation error, so an input's accessible name became
`Program This field is required.` as soon as validation ran, and the error was announced
twice. Each caption now carries an `id` and each input an `aria-labelledby`, matching
`LaunchGroupModal.tsx`. It is **five source sites, not the "~13"** the earlier note
estimated — that number counted runtime instances of the three looped per-profile fields.
The regression test clears all five fields, submits, and looks each one up by its own name
while invalid; it was confirmed red before the fix. Frontend counts moved from
26 files / 208 tests to **26 files / 209 tests**.

Verified locally after the fix: `npm run lint`, `typecheck`, `test -- --run`
(26 files / 209 tests), `build`, and `e2e` (7 passed) all clean. Both Cargo packages were
left untouched by this commit — `git status` showed only the two `ProjectModal` files — so
their numbers stand from the matrix run recorded below: root 38, desktop 250 + 1 ignored.

**CI is green on `842efb9`**, the branch tip — run
[33376342697](https://github.com/AbyssWhalen/RunCove/actions/runs/33376342697), all five
jobs `success`: `Rust lint` (1m20s), `Windows desktop` (10m6s), and `CLI` on
`ubuntu-latest` (23s), `macos-latest` (1m8s), and `windows-latest` (1m8s). This is the
first CI run this feature has had, and it is the one that matters most: `Windows desktop`
is the job that runs the desktop crate's tests, the frontend suite, and `tauri build` on
the target platform, so it independently reproduces the local matrix rather than
restating it. No workflow file was touched — opening the PR is what triggered it.

**Still unauthorized, each needing its own ask:** merging PR #4; publishing `0.4.0` at all
(the version bump across five manifests, the tag, the release workflow, the GitHub
Release); and every P3 housekeeping item. Two operational notes carry forward unchanged:
`apps/desktop/src-tauri/target/release/runcove-desktop.exe` is a **production-identifier
build that must not be launched**, because opening it would upgrade the real database to
schema 3 irreversibly; and the pre-upgrade backup of that database sits hash-verified at
`%LOCALAPPDATA%\RunCove-backup-v0.3.0-2026-08-31\runcove.sqlite3`.

## 2026-08-31 P2 Done: Launch Groups, On Schema Version 3

The v0.4.0 feature the user picked is implemented end to end and the full `AGENTS.md`
matrix is green. Nothing is committed; `main` and `origin/main` are still at `cae4d28`
and no version file was touched. A group is a **named, editable, ordered set of launch
profiles that starts or stops as one unit**, and there can be as many as the user keeps —
which is what the single implicit restore set could never be.

**What landed.**

- **Schema version 3** (`storage.rs:967`, `upgrade_to_version_3` at `:1094`): the two
  tables `launch_groups` and `launch_group_members`, one transaction with
  `PRAGMA user_version=3` last, so a failure rolls back and stays at version 2. Built
  exactly like `upgrade_to_version_2`, `IF NOT EXISTS` deliberately omitted for the same
  reason. `SCHEMA_VERSION`, the test literal `CURRENT_SCHEMA_VERSION` (`:1164`), and the
  pinned `V3_ADDITION` fixture (`:1937`) are three independent statements of the same
  shape, so drift fails a test rather than passing silently.
- **Storage**: `save_launch_group` (upsert in one transaction, members deleted and
  reinserted with `position` from the input order), `delete_launch_group`,
  `list_launch_groups`, `launch_group`, and `validate_launch_group`.
- **Backend types and commands**: `LaunchGroup`, `LaunchGroupInput`,
  `LaunchGroupStartResult`, `LaunchGroupStopResult`, `LaunchGroupStopFailure`;
  `save_launch_group`, `delete_launch_group`, `start_launch_group`, `stop_launch_group`,
  registered in `lib.rs:275-278`. `DashboardSnapshot` carries `launch_groups`, so groups
  arrive on the existing one-second poll and there is no new fetch command and no new
  event.
- **Start is one line of reuse**, not a second ordered launcher: `start_launch_group`
  calls the existing `restore_profiles`, which already stops before the next member on a
  failure and keeps what started. Three behaviors come free — the full per-profile start
  path including the conflict pre-check and the expected-port wait, an already-running
  member counting as started (so Start only fills gaps and the whole action is
  idempotent), and the `relatedPort` payload that drives `View occupant`.
- **Stop walks in reverse and does not stop the rest.** Each member goes through the same
  `stop_profile_inner`; a failure is recorded in `failures` and the walk continues.
  Interrupting a stop would only leave more processes running, and this matches
  `processes.rs` `stop_all_with_intent`, which already collects rather than aborts.
- **Frontend**: `components/launch-group.ts` (the only place the judgment lives —
  `deriveGroupStatus`, `resolveGroupMembers`, validation), `LaunchGroupSection.tsx`,
  `LaunchGroupModal.tsx`, the Overview placement below the restore band, `App.tsx`
  wiring, `types.ts`, `api.ts`, `mock-data.ts` with a seeded group, bilingual `group.*`
  messages, and `styles.css`.

**Decisions worth not re-deriving.**

- **Authorized 2026-08-31**: schema version 3 and its two tables; start stops at the first
  failure and keeps what started; scope is whole-group start and stop only, with no group
  integration into the restore set; UI lives on Overview under the restore band. The
  `HANDOFF.md` clause forbidding "any schema change beyond version 2" is superseded — see
  **Not Authorized From This Checkpoint** below, which now says so.
- **Groups may cross projects.** Members reference `launch_profiles(id)` directly, so a
  database in project A and a web app in project B is an ordinary group. That is the point
  of the feature, not an accident of the schema.
- **A group has no stored state.** `deriveGroupStatus` computes running / partial / idle
  from the member statuses already in the snapshot, so the backend gained no status field
  and no group event. A group is "running" only when every member is up, where up means
  `running` or `starting` — `conflict` is not up.
- **`ON DELETE CASCADE` to `launch_profiles` is the whole reason groups are tables** rather
  than a field in the settings JSON: deleting a profile removes it from every group that
  listed it, so no read has to filter dangling references. Positions may then have holes,
  which is harmless because every read is `ORDER BY position` and no code treats a position
  as an array index. An emptied group stays visible instead of vanishing.
- **Reservations stay per member**, as `restore_profile` already does; `reserve_many` was
  not used. The cost is that a user can still act on one profile mid-group; the benefit is
  that `start_profile_inner_reserved`'s contract did not change. Restore shipped this way
  and was verified this way.
- **No new `V0.4.0_PLAN.md`.** P3 wants agent-facing documents out of the repository, so
  adding one would push the wrong way. The plan lives in the session plan file; the record
  lives here and in `notes.md`.
- **Still refused, by design**: no start at Windows login and no automatic project startup.
  A group starts only when its button is pressed. Launch groups must not become a back door
  to a feature `README.md:209` and `CHANGELOG.md` record as out of scope.

**Verification, all green, run before any number was written down.**

- **Root crate** (`runcove`): fmt ok, clippy `--all-targets --all-features -D warnings`
  clean, `cargo test --all-targets` `38 passed; 0 failed` (`12 + 0 + 0 + 10 + 16`) —
  unchanged, as expected: the root crate has nothing to do with this feature.
- **Desktop crate**: fmt ok, clippy clean (56.30s), `cargo test --all-targets`
  `250 passed; 0 failed; 1 ignored`, up from 240. The environment-dependent
  `external_termination_with_verified_identity_stops_tree_and_releases_port` passed here
  again through its `Ok` path, so its P1-3 guard is still unexercised.
- **Frontend**: `lint` and `typecheck` clean, `npm test -- --run` `26 passed (26)` files /
  `208 passed (208)` tests in 21.70s (from 23 / 171), `npm run build` JS 335.03 kB / CSS
  38.32 kB, `npm run e2e` `7 passed (18.8s)` (from 6).
- **`npm run tauri build`**: exit 0. It was run twice — once at `1m 37s` on the tree, and
  again after the isolated-identifier experiment so the artifact matches the reverted
  source: `56s` total, cargo `46.57s`, `25,869,303` bytes at `2026/8/31 15:23:18`. The
  rebuilt exe contains `com.abysswhale.runcove` twice, `demo0831` zero times, and the
  launch-group strings — checked by reading the binary, not assumed. **It carries the
  production identifier, so it must not be launched**: doing so would upgrade the real
  database to version 3.

**The migration was proven on a throwaway identity, and the real database was never
opened.** This is the part that cannot be replaced by tests, because the tests build their
fixtures in memory and never touch an install path.

- **Method**: `tauri.conf.json`'s identifier temporarily set to
  `com.abysswhale.runcove.demo0831` **and** `lib.rs:42`'s `INSTANCE_MUTEX_NAME` set to
  match, built, copied to `D:\tmp\runcove-v040-demo\RunCove-demo0831.exe`, both edits then
  reverted and the revert confirmed with `git diff` and `git grep demo0831` (no hits).
- **The mutex had to change too, and that is a real trap.** `INSTANCE_MUTEX_NAME` is a
  hardcoded literal, not derived from the bundle identifier, and `single_instance.rs:61`
  builds the wake event as `{name}.Wake`. An isolated build with only the identifier
  changed still shares the single-instance guard with production: it would wake the
  running RunCove instead of starting itself, and the experiment would silently measure
  nothing.
- **Genuine version 2 → 3 upgrade**: a pinned version 2 database (the `V1_SCHEMA` +
  `V1_FIXTURE` + `V2_ADDITION` text copied out of `storage.rs`, plus one `complete`
  archive row; `user_version=2`, eight tables) was staged into the isolated data
  directory, the demo build opened it once, and afterwards `user_version=3`, ten tables,
  both new tables with the exact pinned DDL including `COLLATE NOCASE`, `integrity_check`
  ok, `foreign_key_check` ok, and every fixture row intact — project, both profiles in
  order, both expected ports, the port association, the three run sessions, the restore
  set still `0=prof-2, 1=prof-1`, `restore_saved_at`, and the archive row.
- **Real group rows survive a real session**: a group whose members are deliberately out
  of profile sort order (`0=prof-2, 1=prof-1`) was seeded into the upgraded database; the
  build ran 14s with the snapshot loop reading it and exited with the rows and the order
  unchanged and nothing on stderr.
- **Two changes the run made are correct, not data loss**: the session left `running` by
  the previous kill became `interrupted` (startup reconciliation), and `archiveRunLogs`
  appeared in the settings JSON (`#[serde(default)]` round-trip). The
  `languagePreference` went from `zh-CN` to `system` for a third reason that is also not
  a defect: `App.tsx:389-393` makes WebView `localStorage` win over the database when the
  two disagree, and an earlier run in that same throwaway WebView profile had stored
  `system`. A real install's `localStorage` and database agree.
- **A verification trap worth remembering**: the first attempt seeded the database with
  Microsoft Store Python, whose writes under `%LOCALAPPDATA%` are redirected into
  `%LOCALAPPDATA%\Packages\PythonSoftwareFoundation.Python.3.13_*\LocalCache\Local\`. The
  app therefore found no database, created a fresh version 3 one, and the check would have
  reported success while never exercising the upgrade at all. Stage files under
  `%LOCALAPPDATA%` with PowerShell and let Python work on a copy under `D:\tmp`.
- **The real database was confirmed untouched**: `%LOCALAPPDATA%\com.abysswhale.runcove\
  runcove.sqlite3` is still `user_version=1`, last written 2026-08-11, read from its
  SQLite header without opening it. `demo0819` remains at 2, `qa` at 1.

**Carried limitations, unchanged by this work.**

- **The version 3 upgrade is one-way.** Each upgrade runs in one transaction and stays at
  the previous version if it fails, but a successful one cannot be undone: once a version 3
  database exists, the frozen v0.3.0 refuses it as newer than it supports. Anyone running a
  `main` build for the first time should back up
  `%LOCALAPPDATA%\com.abysswhale.runcove\` first. `README.md` now says this in both
  languages. **A backup was already taken on 2026-08-31**:
  `%LOCALAPPDATA%\RunCove-backup-v0.3.0-2026-08-31\runcove.sqlite3`, 69,632 bytes,
  `user_version=1`, hash-verified against the original, which was not modified. Restoring it
  means copying that one file back; `EBWebView` beside it is a regenerable WebView cache and
  was not copied.
- **`line_count` can overstate a session whose closing flush failed** (the 2026-08-18 P2).
  The byte side self-corrects from disk and the normal path is unaffected. Still open, still
  deferred, and untouched by launch groups.
- **`ProjectModal.tsx` still has the accessible-name defect that `LaunchGroupModal.tsx`
  fixed.** Its labels are plain text with no `id`, so roughly thirteen inputs there have no
  programmatic name. Left unfixed on purpose: it is a pre-existing defect in a file this
  feature does not need, and folding a thirteen-site accessibility change into the
  launch-group diff would make both harder to review. It is worth its own small change.
- **Nothing is committed** and the version number is still `0.3.0` in all five manifests.
  Whenever the user authorizes a release, this feature makes the number `0.4.0`, and the
  `0.3.1` bug fix P1 produced rides along with it.
- **The isolated build is kept** at `D:\tmp\runcove-v040-demo\RunCove-demo0831.exe` with
  its data directory and the before/after database copies, so the migration check can be
  repeated without another identifier edit. It is outside the repository and must stay
  there.

Next: **P3 housekeeping**, every item of which needs authorization — and the standing
question of whether to publish `0.4.0`, which needs commit, push, tag, CI, and release
authorization that has not been given.

## 2026-08-30 P1-4 Done: The Full Matrix Is Green, And The Next Number Is 0.3.1

Everything in `AGENTS.md`'s matrix passed, including the `npm run tauri build` that P1-1
through P1-3 had deferred. Nothing is committed; no version file was edited. P1 is now
closed.

- **Root crate** (`runcove`): `cargo fmt --all -- --check` ok, `cargo clippy
  --all-targets --all-features -- -D warnings` clean, `cargo test --all-targets`
  `38 passed; 0 failed` across five targets (`12 + 0 + 0 + 10 + 16`).
- **Desktop crate** (`runcove-desktop`): fmt ok, clippy clean, `cargo test --all-targets`
  `240 passed; 0 failed; 1 ignored`. Also verified single-threaded during P1-3.
- **Frontend**: `npm run lint` and `npm run typecheck` clean, `npm test -- --run`
  `23 passed (23)` files / `171 passed (171)` tests, `npm run build` 1607 modules in
  1.73s, `npm run e2e` `6 passed (18.5s)`.
- **`npm run tauri build`**: exit 0, release profile in `1m 55s`, built
  `apps/desktop/src-tauri/target/release/runcove-desktop.exe`. **No installer was
  produced, and that is correct**: `tauri.conf.json` sets `bundle.active: false`, so this
  command only ever builds the executable — the release workflow is what packages. That
  exe carries the production identifier and still must not be launched.
- **A defect in the matrix itself was found and fixed.** `AGENTS.md` listed the three
  `cargo` commands only at the repository root, but `apps/desktop/src-tauri` is a separate
  package and not a workspace member, so those commands never reached it — a literal
  reading of the recipe skipped 240 tests plus that crate's fmt and clippy. `AGENTS.md`
  now has both Rust blocks and says why. The gate itself was never weaker: `ci.yml:110-119`
  already runs fmt, clippy, and `cargo test --all-targets` in `apps/desktop/src-tauri`.
- **The version number is `0.3.1`**, and the recommendation is **not to publish it on its
  own.** The whole user-visible delta since `v0.3.0` is one bug fix — RunCove's own
  lifecycle sentences appearing in English under a Chinese interface — with no new feature
  and no breaking change; the IPC `reason` field is additive and optional, so `0.3.1` is
  what SemVer says. Publishing it separately costs five authorizations and a CI run for a
  fix the user can already get by rebuilding locally, and `0.4.0` is reserved for the P2
  feature, which the fix can simply ride along with. `CHANGELOG.md`'s `[Unreleased]`
  section keeps accumulating either way, so nothing is wasted if the user decides they
  want the patch out sooner.
- **Not done, deliberately**: no version file was touched. A bump means editing five
  manifests — root `Cargo.toml`, `apps/desktop/src-tauri/Cargo.toml`, `tauri.conf.json`,
  `package.json`, `package-lock.json` (two places) — plus the two lockfiles cargo
  refreshes, and it only means anything alongside a release, which is unauthorized.

Next: **P2 — the user picks one v0.4.0 feature.** The recommendation in the plan below is
launch groups.

## 2026-08-30 P1-3 Done: A Refused Termination Is Reported, Not A Failed Test

`external_termination_with_verified_identity_stops_tree_and_releases_port`
(`commands.rs:1830`) now distinguishes the machine refusing the operation from RunCove
getting it wrong. Nothing is committed. Test code only — no production behavior changed,
and the test count is unchanged at 240.

- **`#[ignore]` was rejected**, even though that is the guard the other live test
  (`commands.rs:1884`) carries. That test needs configured live services and can never
  run unattended; this one spawns its own fixture and **passes on CI**, so `#[ignore]`
  would silence the only test of the successful termination path everywhere in order to
  quiet one machine. CI does not run `-- --ignored`.
- **What it does instead.** On `Err`, `termination_refused_by_environment`
  (`commands.rs:1822`) decides: a refusal from `taskkill` itself is reported with
  `eprintln!` and the test returns; anything else fails the test with that error. On `Ok`
  the assertions are exactly as before, so CI keeps full coverage.
- **The predicate matches RunCove's own wrapper text**, `"Could not terminate process
  tree:"` (`commands.rs:1050`), and deliberately not the reason inside it: `taskkill`
  prints in the system language and its bytes reach that string through
  `from_utf8_lossy`, so matching `Access is denied` would silently stop working on a
  non-English Windows — this machine included. The wrapper is narrow by enumeration:
  every other `Err` in `terminate_external_windows` is RunCove's own answer (changed
  identity, changed executable, managed process, missing `taskkill.exe`, refusing
  itself, an unreadable handle) and none of them starts with that prefix.
- **The guard is unexercised here, and that is the honest state.** The flake did not
  reproduce: the test passed through the `Ok` path in 10 consecutive dedicated runs plus
  a parallel and a single-threaded full suite. Whoever next sees the `Access denied`
  failure will get a pass with a `--nocapture` line instead, and that is the first real
  exercise of this path.
- **Verified**: desktop crate `240 passed; 0 failed; 1 ignored`, identical in parallel
  and single-threaded (`40.70s`), `fmt --check` and `clippy --all-targets --all-features
  -D warnings` clean. The root crate and the frontend were untouched.

Next: P1-4 — the full `AGENTS.md` matrix, including the still-unrun
`npm run tauri build`, then the version number.

## 2026-08-30 P1-2 Done: The Line-Count Over-Report Is Accepted And Recorded

P1-2 is closed as a **disclosed limitation, not a fix**, which is a decision the plan
required and forbade deferring again. No production behavior changed, no test changed,
nothing is committed. Three documents carry it: `return_file`'s doc
(`archive.rs:2618-2636`), a `notes.md` top section with the full reasoning, and a new
`CHANGELOG.md` `[Unreleased]` section.

- **The defect, stated exactly.** A small record is counted as a line when `write_all`
  returns into the 64 KiB `BufWriter`. If the *closing* flush then fails, `byte_size` is
  re-measured from the file (`archive.rs:2728` and `:2955`) but `line_count` can still
  name a line the file does not hold. Only that path; a normal close is exact.
- **Both candidate fixes were refuted, and the second one is why this is a decision
  rather than laziness.** Counting at flush boundaries cannot work — a flush can go out
  *partially*, so it needs every buffered line's byte length. Recounting the file looks
  nearly free, since the close already measures it, but `Sweep::count_lines`
  (`archive.rs:1311`) counts a trailing fragment as a line while a short write here
  reports that fragment as a dropped line with `line_count` 0
  (`archive.rs:4988`, `"a fragment is not a line"`). The same file would read one line
  longer after a crash than after a close, so exactness means unifying that definition
  across the sweep, the writer, and their tests.
- **Why accepting is safe.** `line_count` is display-only: the quota and eviction read
  `byte_size` and the timestamps, the viewer pages from the file and already treats the
  row's counters as possibly stale (`models.rs:415-417`), the error is always an
  over-count, and the row is already labeled `partial` / `write-error`.
- **`CHANGELOG.md` now has `[Unreleased]`** with P1-1's fix under `Fixed` and this
  limitation under `Known Limitations`. The published `[0.3.0]` entry was **not** edited,
  including its English-message limitation that P1-1 has since fixed: a released section
  is a historical record, and `[Unreleased]` is what supersedes it.
- **Verified** (Rust doc comment only, so the Rust matrix is the relevant part): desktop
  crate `240 passed; 0 failed; 1 ignored`, root crate `38 passed` across its six test
  targets (`12 + 0 + 0 + 10 + 16 + 0`; the P1-1 note below said `16`, which was only the
  largest binary — count all six), `fmt --check` and
  `clippy --all-targets --all-features -D warnings` clean in both crates. The frontend
  was untouched; its full matrix belongs to P1-4.

Next: P1-3, the environment guard for
`external_termination_with_verified_identity_stops_tree_and_releases_port`.

## 2026-08-30 P1-1 Done: A Lifecycle Reason Is Localized, Its Log Line Is Not

P1-1 is closed, and with it the open P2-2. The defect was that RunCove composed its
own English sentences into `RunStatusEvent.message`, so a Chinese window showed
`Stopped by user` and `Process exited normally`. Nothing is committed.

- **The wire now carries a value, not a sentence.** `models.rs` gained
  `RunStatusReason`, an internally-tagged enum (`#[serde(tag = "kind",
  rename_all = "kebab-case")]`) with eight variants, and `RunStatusReason::describe`
  is the English form. Both `RunStatusEvent` and `RunLogEvent` gained an optional
  `reason` under `#[serde(default, skip_serializing_if = "Option::is_none")]`.
  `message` and `line` are unchanged and still hold the English sentence, so this is
  an additive upgrade: nothing existing changed meaning.
- **Eight sentences were converted**, all of them text RunCove itself wrote: the six
  arms of `watch_child`'s exit classification in `processes.rs`, plus
  `"Stop requested"` and `"Profile is already running"` in `commands.rs` through the
  new `status_event_with_reason` helper.
- **`AppError` text is deliberately out of scope.** The port-conflict message
  (`commands.rs:702`) and the `error.to_string()` start failures carry no reason and
  pass through unchanged. That surface is far larger, already framed by
  `t("error.lifecycleDetail", …)`, and the conflict text also travels as the command's
  `Err`. Say it that way rather than calling P1-1 partial.
- **The frontend translates it.** New `src/run-status.ts` mirrors
  `components/archive.ts`: a kebab-case → `MessageKey` table, `describeRunStatusReason`
  returning `null` for a kind this build cannot name, and `runStatusText` for the
  event pair. Nine `runStatus.*` keys were added to both catalogs. `App.tsx`'s status
  listener and `LogDrawer.tsx` (rendering *and* clipboard, through one `lineText`)
  are the callers.
- **An unknown `kind` degrades to the backend's English**, which is why `kind` is a
  plain `string` in `types.ts` and not a union: nothing validates an IPC payload at
  runtime.
- **Two things stay English by design.** `LogDrawer`'s `logKey` dedupe still hashes
  `line`, so the history/live merge is unaffected; and the archive still stores
  `line`, so archived lifecycle records remain English. The on-disk format
  (`{t,s,l}`, schema 2) is untouched and no migration was needed.
- **Tests, on the defect path only** (per the new policy): `run-status.test.ts` pins
  all nine keys in both languages plus the unknown-kind and missing-detail fallbacks;
  two `App.test.tsx` cases assert a zh-CN notice and a zh-CN failure alert show
  Chinese and not the English `message`; one `LogDrawer.test.tsx` case asserts the
  drawer renders Chinese, leaves child-process output alone, and copies what it shows.
  In Rust, `models.rs` pins the kebab-case `kind` and the omitted-field shape, because
  a rename there would silently return the UI to English.
- **Verified**: root crate `38 passed` across six test targets; desktop crate
  `240 passed; 0 failed;
  1 ignored` (baseline 238/1, +2 new tests); `fmt --check` and
  `clippy --all-targets --all-features -D warnings` clean in both crates; frontend
  `lint`, `typecheck`, `test -- --run` (23 files / 171 tests, baseline 22 / 157),
  `build`, and `e2e` (6) all green. `npm run tauri build` was **not** run here — it
  belongs to P1-4's full matrix. In this run the environment-dependent
  `external_termination_with_verified_identity_stops_tree_and_releases_port` passed,
  which is why P1-3 still needs doing rather than being closed as absent.

Next: P1-2, the `line_count` over-report decision.

## 2026-08-30 Direction Reset: Product Work Resumes, 软著 Dropped

Decisions made today, all by the user:

- **The 软著 workstream is dropped.** Secondary sources agree that since 2026-03
  中国版权保护中心 forbids applying with AI-generated code, with 失信名单 /
  个人征信 consequences, and that 2026 review adds AI-material screening and
  code-similarity comparison. RunCove's code was substantially agent-written and
  the public repository says so, so that path is closed. No AI-trace scrubbing
  will be done to serve it. The official text was not readable from this
  environment; `CLAUDE.md` carries the finding and the confirm-at-source rule.
- **Testing policy changes.** No more red-tests-first, no more micro-seam tests.
  Write a test when a real defect path or regression risk justifies it. Existing
  tests stay; none is deleted to save time. This replaces the slice-by-slice
  red-to-green process used for the v0.3.0 archive.
- **Output style.** Chinese, conclusion first, short, no filler.

### The Plan

**P1 — Close the known defects. Start here; no scope debate needed.**

1. **Done 2026-08-30 — see the top section.** `processes.rs:562` and `:580` composed
   English `"Stopped by user"` and `"Process exited normally"` into
   `RunStatusEvent.message`, which then appeared under the Chinese UI. The events now
   carry a machine-readable reason that the frontend i18n layer translates. This was
   the open P2-2.
2. **Done 2026-08-30 — see the top section.** P2-1 (`archive.rs:2636`, `return_file`):
   `line_count` can over-report when the closing flush itself fails. Decided as an
   accepted, disclosed limitation, with both candidate fixes refuted on record. It is
   now in `CHANGELOG.md`'s `[Unreleased]` section as well.
3. **Done 2026-08-30 — see the top section.**
   `external_termination_with_verified_identity_stops_tree_and_releases_port`
   fails on this machine with Windows `Access denied` and passes on CI. It now reports a
   refusal from `taskkill` and returns instead of failing, while every RunCove-decided
   error still fails it. `#[ignore]` was rejected: it would drop the coverage on CI too.
4. **Done 2026-08-30 — see the top section.** Run the full matrix in `AGENTS.md`, then
   pick the version number. All of it is green, `AGENTS.md`'s own matrix was missing the
   desktop crate and was fixed, and the number is `0.3.1` — recommended to ship with the
   P2 feature as `0.4.0` rather than as its own release.

**P2 — One feature for v0.4.0. The user picks.**

**Picked and implemented 2026-08-31 — launch groups. See the top section.** The
alternatives below stay on record as later candidates, not as open questions.

Recommended: **launch groups.** An ordered set of profiles that start as a unit,
reusing the expected-port wait and the ordered restore that already exist. It is
the product form of machinery RunCove already has, and starting a db → api → web
stack by hand is the daily annoyance the app still does not remove.

Smaller alternatives, as the pick or as a companion:

- Optional HTTP health check: a listening port is not a working service. Treat
  `Running` as confirmed only when an optional URL answers.
- Start-failure diagnosis: put the last stderr lines into the failure report
  instead of one error string.

Rejected with reasons on record: start at login and automatic project startup
(`notes.md:1612`), `.env` editing (`AGENTS.md:57`), project Git status
(`V0.3.0_PLAN.md:1905-1908`), Docker and remote hosts, device previews, usage
analytics.

**P3 — Housekeeping. Each item needs authorization.**

- Untrack the agent-facing documents (`AGENTS.md`, `HANDOFF.md`, `notes.md`,
  `V*_PLAN.md`) if the user still wants them off GitHub. Move, never delete. The
  justification is the preference recorded at 2026-08-21 — not publishing
  tool-specific collaboration files — and not an AI-trace cleanup.
- A supply-chain audit job, and either a minimum-Rust-version job or dropping the
  unenforced `rust-version` declarations. Both touch CI, which is a red line.

### Not Authorized From This Checkpoint

Commit, push, tags, CI or release workflow edits, `.env`, any real database. **The
"no schema change beyond version 2" clause that stood here is superseded**: on
2026-08-31 the user authorized schema version 3 and its two launch-group tables for
the v0.4.0 feature (see the top section). Everything else in this list still holds.
The worktree held no product changes at `cae4d28`;
all of P1's edits are uncommitted on top of it. No version file was bumped, because a
bump only means something with a release. Published `v0.3.0` stays exactly as it is.

## 2026-08-22 v0.3.0 Published And Verified

- The v0.3.0 release is public at
  https://github.com/AbyssWhalen/RunCove/releases/tag/v0.3.0. It is a non-draft,
  non-prerelease release with the Windows portable desktop archive, the CLI
  archives, and `SHA256SUMS.txt`.
- PR #3 is merged. `main`, `origin/main`, and the annotated `v0.3.0` tag all
  resolve to merge commit `bd2b7776d56ddf750ffe97a3d8219168fbb04069`.
- Release workflow `32500425361` completed successfully. Its Windows desktop
  job ran the frontend lint, typecheck, Vitest, build, six E2E flows, Rust
  checks, and Tauri build on a clean runner. The downloaded five binary
  archives matched the published SHA-256 manifest. An initial local parser
  check used the wrong spacing assumption; the final evidence is the direct
  hash comparison, not that parser result.
- The local worktree is clean after the restart. Release verification scratch
  data is outside the repository under `D:\tmp`; no generated output or user
  database was added to the project. `CLAUDE.md` remains local and ignored.
- Local Codex verification remains partially environment-limited: one real
  process-termination test returned Windows `Access denied`, and fresh Node
  child-process creation returned `spawn EPERM`. The unchanged GitHub CI run
  passed the corresponding Windows checks. The release workflow emitted only
  existing Node 20 deprecation annotations.
- v0.3.0 is frozen for user exploration. Do not start another feature or
  soft-copyright workstream from this checkpoint without a new scoped plan.

## 2026-08-21 v0.3.0 Release Preparation Authorized

- The user accepted the local-demo level of stability and explicitly authorized
  commit, push, the `v0.3.0` tag, and a public GitHub Release. No new feature work is
  authorized or needed for this release; the two known P2 limitations remain disclosed.
- Release preparation is on `codex/release-v0.3.0`. Product manifests, lockfiles,
  `CHANGELOG.md`, `RELEASE_NOTES.md`, and the current-release sections of `README.md`
  now say `0.3.0`. The existing `.github/workflows/release.yml` is unchanged and will
  validate all four manifest versions, build the cross-platform CLI and Windows
  portable desktop archives, generate `SHA256SUMS.txt`, and publish the release when
  the tag is pushed.
- The release commit stages 43 paths. This is the frozen 36-path product/documentation
  tree plus seven release-preparation paths (version and lockfile updates, changelog,
  release notes, and current-release README text); generated build output is ignored.
- `CLAUDE.md` remains a local agent entry point and is excluded through
  `.git/info/exclude`; the temporary tracked references to it were removed from
  `AGENTS.md`, restoring that file to its published form. This follows the user's
  preference not to publish tool-specific collaboration instructions.
- Current gate: run the complete local verification matrix. Any P0/P1 or required
  validation failure stops the release. If green, push the release branch, merge it
  through GitHub CI, then tag the resulting `main` commit and verify every release
  asset and checksum.
- Release-preparation verification on 2026-08-21 found and removed one CI blocker in
  the lockfile: the official npm registry reported the transitive development package
  `nanoid` 3.3.17 under high-severity advisory `GHSA-2v37-7h3g-55p8`. Regenerating only
  the affected lock entry selected 3.3.18; `npm ci` and `npm audit --audit-level=high`
  against `registry.npmjs.org` then reported zero vulnerabilities. No direct dependency
  or production source changed for this fix.
- Fresh local evidence before the PR: both Rust format checks and both Clippy runs are
  clean; the root crate has 38 passing tests; frontend lint, typecheck, and all 157
  Vitest tests pass; all 99 archive and 11 archive-service tests pass. The desktop suite
  reached `237 passed; 1 failed; 1 ignored`: the sole failure is the pre-existing
  real-process test
  `external_termination_with_verified_identity_stops_tree_and_releases_port`, where
  Windows `taskkill` returned `Access denied`. It also failed alone in this managed
  session and has passed in earlier full runs; no archive assertion failed.
- This Codex process cannot freshly repeat Playwright or Vite/Tauri builds because Node
  child-process creation returns `spawn EPERM` (the same `esbuild.exe` runs directly
  from PowerShell). The frozen tree already passed 6 Playwright workflows and a Tauri
  production build on 2026-08-20. These two environment-limited checks and the process
  test are therefore delegated to the unchanged GitHub PR CI on a clean Windows runner;
  the PR must be fully green before merge or tag.
- PR #3's first CI run passed all four CLI/lint jobs and every frontend step, including
  the official npm audit, production frontend build, and six Playwright workflows. The
  Windows desktop job then found one Rust 1.98-only Clippy error in test data:
  `format!("{uuid}")` triggered `clippy::useless_format`. It is corrected to
  `uuid.to_string()` with no production-code or assertion change; the follow-up CI run
  remains the merge gate.

## 2026-08-20 v0.3.0 Local Demo Candidate: Frozen After The Pre-Release Wrap-Up

- Status: **frozen** at the user's instruction — no new features, no new seams, no new
  red tests. The `v0.2.1` baseline is untouched: local `main` and `origin/main` are both
  still at `97943d7`, the `v0.2.1` tag still targets
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`, and nothing was committed, pushed, tagged,
  or released. No CI or release workflow, no `.env`, no real application database, no
  unrelated project, and no existing developer process was touched. The working tree is
  still **36 paths** — 28 modified, 8 untracked; confirm with `git status --porcelain`
  rather than trusting the number.
- Scope of this session: documentation and verification only — the README
  contradictions, the known P2s, the verification matrix, the manual demo checklist,
  `HANDOFF.md`, `notes.md`, and one pointer bullet in `CLAUDE.md`. **No production or
  test source file was changed**, so the 2026-08-19 section below is still the
  description of the code.
- **The README no longer contradicts the feature.** Six corrections, all of the same
  confusion: v0.2.1's "logs are never written to disk" promise versus the archive that
  now exists on `main`. The log-boundary bullet (`README.md:39-46`) says v0.2.1 keeps
  console logs in bounded memory only and names the archive as the one exception, off by
  default, added after v0.2.1; the log bullet under **Desktop App** (`:65`) and the Help
  bullet (`:69`) point at the switch in the log drawer; the new **Run Log Archive**
  section (`:75-91`) leads with opt-in / off by default and states it is not in the
  published portable zip; **Architecture** admits the one index row per archived session
  (`:171`) and the one-way schema step (`:173`) — a version 2 database is not openable by
  v0.2.1, so keep a copy of the data directory before trying `main`; **Privacy And
  Process Safety** (`:186`) says session output stays in bounded memory by default and
  the archive is the only path that writes it to a file; and the **v0.2.1 Scope**
  exclusion (`:233`) now reads "Persistent log archives (added on `main` after v0.2.1 …,
  still off by default)" instead of a flat denial.
- **Both known P2s were re-verified in code and on screen, and both stay open.**
  (1) `line_count` can over-count a session whose *closing* flush failed; the reason is
  written out above `return_file` (`archive.rs:2624-2630`) — the byte side is corrected
  from the disk at close, and naming which buffered lines survived a partial flush would
  need each line's byte length. The normal path is unaffected. (2) The stop and exit
  messages are composed in the backend in English (`processes.rs:562` `"Stopped by
  user"`, `:580` `"Process exited normally"`), so they read English under a Chinese UI —
  seen again as 「21:32:09 系统 Stopped by user」. Neither is a data-loss or safety issue
  and neither was fixed under the freeze.
- **The NTFS measurement trap is a fact about Windows, not a defect**, and it appeared
  twice more: `Get-ChildItem` reports `Length = 0` for an archive whose writer handle is
  open, because the directory entry is stale until that handle closes. Measure an open
  archive through an opened handle
  (`[System.IO.File]::Open(..., FileShare::ReadWrite).Length`), which is what RunCove's
  own reader does. One live archive read `dirEntry=0` and `true=28813` in the same second.
- Verification matrix, re-run at the freeze, every command exiting 0. Root crate:
  `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features -- -D
  warnings` clean, `cargo test --all-targets` **38 tests**. Desktop crate
  (`apps/desktop/src-tauri`): fmt and clippy clean, `cargo test --all-targets`
  **`238 passed; 0 failed; 1 ignored`** out of 239. Frontend (`apps/desktop`):
  `npm run lint`, `npm run typecheck`, `npm test -- --run` at **22 files / 157 tests**,
  `npm run build`, `npm run e2e` at **6 passed**, and `npm run tauri build`.
- **`apps/desktop/src-tauri/target/release/runcove-desktop.exe` is now a
  production-identifier build and must not be launched.** The `npm run tauri build` in
  that matrix rebuilt it without the demo config, so it would open the *real*
  `%LOCALAPPDATA%\com.abysswhale.runcove` data directory. Use only the separate demo exe
  at `D:\tmp\runcove-v030-demo\RunCove-demo.exe` to look at the feature.
- **The manual demo checklist was re-run end to end on the frozen build**, in the
  isolated demo data directory, with no production RunCove running at any point. Beyond
  the seven criteria recorded below, four measurements are worth keeping. (1) Paging
  reached the start: 500 → 1,000 → 1,500 → 2,000 → 2,261 records, ending at
  「已到归档开头」 with 「已显示 2,261 行 · 记录 2,261 行 · 147 KiB」, and every prepend
  kept the reading position. (2) A delete credited exactly the file's measured length —
  the directory went 4 files / 778,861 bytes to 3 files / 628,807 bytes, a difference of
  **150,054 bytes**, which is what `credit_removed` measured on disk rather than the row's
  `byte_size`; the row stayed with 「已删除 · 由你删除」 and 显示 9 / 9 was unchanged.
  (3) Turning the switch off mid-run finalized the open archive instantly and completely:
  `dirEntry=54038 true=54038`, 822 records, last record `info handled request 816` at
  21:27:21.986, **zero** `"s":"system"` records — so no gap line and nothing dropped —
  while the drawer kept streaming to 891/2000 and the process ran until 21:30:02. The row
  read 「不完整 · 归档已被关闭」 while still 运行中. (4) The next run with the switch off
  read 未归档, the file count stayed at 4, and the closed archive's mtime stayed frozen at
  21:27:22.
- **Criterion 7 has two halves, and only one of them is reachable after startup.** The
  inert half reproduced at once: with `run-log-archives` obstructed, RunCove launched,
  kept port scanning on its own tick, started the profile (PID 2200 at 21:56:08), streamed
  1,194 in-memory lines through the drawer, and badged the row 未归档. The **warning** half
  needs the obstruction in place *before* RunCove starts. `unavailable_reason`
  (`archive_service.rs`) returns `None` whenever the writer exists, so an instance that
  initialized successfully can only fail the per-session `begin`, and that failure is
  reported through the transient lifecycle channel (`ArchiveReporter`, one-message-deep
  dedupe) — it never fills the drawer's warning slot. Obstruct, *then* start, and the
  drawer shows the recorded text verbatim: 「本次会话无法归档运行日志：Could not create the
  run log archive directory: 当文件已存在时，无法创建该文件。 (os error 183)」 with
  「归档运行日志」 still checked, because `enabled` is the user's setting and `available` is
  this run's initialization (`models.rs:324-335`). State it that way rather than as a bug:
  the drawer reports the *run*, and a per-session failure is a transient report plus a
  未归档 badge.
- **The obstruction was undone and nothing was deleted.** The 15-byte placeholder was
  *moved* to `D:\tmp\runcove-v030-demo\run-log-archives.placeholder` (still 15 bytes,
  `not a directory`) instead of removed, `run-log-archives.saved` was renamed back, and
  all four archives verified byte-identical by SHA256 — 4 files / **682,845 bytes**:
  `01daa900…` 54,038 `ED2DFFFE…`, `1e3fa524…` 36,260 `78FD2F77…`, `a50b5606…` 82,805
  `13543956…`, `ccbee61c…` 509,742 `04528437…`. A final launch with the directory healthy
  showed the warning gone and the switch still on; no run was started from it, so the
  directory holds exactly those four files.
- **Environment note, unchanged and load-bearing.** Two `commands.rs` port/child-timing
  tests — `external_termination_with_verified_identity_stops_tree_and_releases_port` and
  `manual_start_stays_starting_until_managed_expected_port_is_ready` — have each failed
  once, here or on a reviewer's machine, and pass in every mode here. **No stable
  full-suite all-green baseline may be claimed for another machine** on the strength of
  the numbers above.
- Not authorized and not done, and the freeze holds until the user says otherwise:
  `commit`, `push`, `tag`, CI, Release, `.env`, any real database, and any schema change
  beyond the version 2 already in place.

## 2026-08-19 v0.3.0 Local Demo Candidate: The Archive Is Wired End To End

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are both
  at `97943d7`, the `v0.2.1` tag still targets
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`, and nothing was committed, pushed,
  tagged, or released. No CI or release workflow, no `.env`, and no real application
  database was touched. The working tree is now **36 paths** — 28 modified, 8
  untracked, confirmed with `git status --porcelain` — so the "ten paths" figure in
  every earlier section below describes step 4b, not this milestone. `CLAUDE.md` has
  been corrected to the current count.
- Scope: the whole approved milestone in one run, with no per-component approval
  stops. The `Storage`-backed `ArchiveIndex`, `archive_service.rs` with its background
  pump and process-lifecycle wiring, the `archive_run_logs` setting behind
  `#[serde(default)]` with **no** new migration, the three async Tauri commands
  (`set_run_log_archiving`, `read_run_log_archive`, `delete_run_log_archive` — the
  state itself rides the dashboard snapshot), and the complete frontend surface.
  `archive.rs` is no longer reference-only: the service is its runtime caller, so the
  "no runtime caller" wording in the sections below is now historical.
- **Reading an archive never blocks the IPC thread and never loads the file.**
  `read_run_log_archive` is `async` + `run_blocking`, seeks to the end, walks backwards,
  and returns one page bounded by *both* a record count and a byte count
  (`MIN_PAGE_RECORDS = 1`, `DEFAULT_PAGE_RECORDS = 500`, `MAX_PAGE_RECORDS = 2_000`).
  The cursor the viewer pages with is the previous page's `pageStartOffset`, so
  "load earlier" is a bounded backwards walk, not a re-read. `delete_run_log_archive`
  still refuses a session this run holds open.
- **Two `run-archive-closed`-family fixes, and they are different bugs.** (1) The exit
  path emits `run-archive-closed` (`processes.rs:675`) because the reload the exit event
  itself triggers still sees the row as `writing` — the close writes its remaining
  records and syncs *after* the lock that event is emitted under is released. (2) The
  toggle path does **not** emit anything: `archive_service::close_open_archives` closes
  every open archive silently, so `App.tsx`'s `toggleRunLogArchiving` now awaits
  `loadRunHistory()` after `setRunLogArchiving` resolves. That is deterministic because
  the command is `async` + `run_blocking`, so every affected archive is already final on
  disk when the promise settles. Both were found on screen, not in review; the second
  was verified non-hollow by breaking the guard (`if (false && …)`) and watching
  `App.history.test.tsx` fail, then restoring it.
- **The badge vocabulary has one value the wire does not.** `finalizing` is not a
  status: it is a `writing` row whose session already has `endedAt`
  (`components/archive.ts`). `canViewArchive` excludes `none` and `removed`;
  `canDeleteArchive` allows `complete`, `partial`, and `unknown`.
- Verification, all green, run from the repository root and from `apps/desktop`:
  root crate `cargo fmt --all -- --check` / `clippy --all-targets --all-features -D
  warnings` clean and **38 tests** passing; desktop crate fmt and clippy clean and
  **`238 passed; 0 failed; 1 ignored`** out of 239 for `cargo test --all-targets`,
  of which **99** are `archive` and **11** `archive_service`; `npm run lint`,
  `npm run typecheck`, `npm test -- --run` at **22 files / 157 tests**,
  `npm run build`, `npm run e2e` at **6 passed**, and `npm run tauri build`. The
  archive frontend files alone are 29 tests
  (`RunLogArchiveDrawer.test.tsx` + `archive.test.ts`).
- **Local acceptance was proven on screen in an isolated demo build**, identifier
  `com.abysswhale.runcove.demo0819`, data directory
  `%LOCALAPPDATA%\com.abysswhale.runcove.demo0819`. No production RunCove ran at any
  point (`INSTANCE_MUTEX_NAME` at `lib.rs:41` is hardcoded and shared), no real
  database was opened, and no existing developer process was touched. All seven
  criteria: default-off creates nothing; an enabled run archives stdout *and* stderr
  (1,208 lines / 79,632 bytes / 0 drops); the archive survives a full RunCove restart;
  the viewer opens on the tail and pages backwards to 「已到归档开头」; a delete keeps
  the run row and shows 「已删除 · 由你删除」; turning the setting off finalizes the
  open archive as 「不完整 · 归档已被关闭」 while the live drawer keeps streaming, does
  not touch the next session's row, and does **not** backfill a session that was
  already running when it was turned back on; and an initialization failure is inert
  for everything else.
- **The initialization-failure proof, and how the directory was preserved.** The demo
  data directory's `run-log-archives` was **renamed** to `run-log-archives.saved` and a
  15-byte file put in its place, so no archive byte was ever at risk. RunCove then
  reported 「本次会话无法归档运行日志：Could not create the run log archive directory:
  当文件已存在时，无法创建该文件。 (os error 183)」 while port scanning kept refreshing
  on its own two-second tick, the profile started normally (PID 31364), the in-memory
  drawer streamed stdout and stderr, and the session's badge read 未归档. The
  placeholder was
  removed and the directory renamed back; both archives measured byte-identical
  afterwards, and the placeholder never grew past 15 bytes.
- **One Windows measurement trap worth knowing.** `Get-ChildItem` reports `Length = 0`
  for an archive that is still open, because the NTFS directory entry is not updated
  while a handle is open. The real length needs an opened handle
  (`[System.IO.File]::Open(..., FileShare::ReadWrite).Length`) — which is what
  RunCove's own reader does. A 7,146-line open archive read 0 bytes in Explorer terms
  and 413,411 in fact.
- **Still open, recorded and not blocking.** P2: `line_count` can over-count a session
  whose *closing* flush failed (byte side self-corrects from disk; normal path
  unaffected). P2: the process-exit toast reads English ("Process exited normally")
  under a Chinese UI because that string comes from the backend. Environment note,
  unchanged: two `commands.rs` port/child-timing tests
  (`external_termination_with_verified_identity_stops_tree_and_releases_port`,
  `manual_start_stays_starting_until_managed_expected_port_is_ready`) have each failed
  once on a machine here or on a reviewer's; both pass in every mode now, but **no
  stable full-suite all-green baseline may be claimed** across machines.
- Not authorized and not done: `commit`, `push`, CI, Release, tags, `.env`, any real
  database, and any schema change beyond the version 2 already in place.

### Running The Local Demo Build

The demo build already exists; nothing needs to be rebuilt to look at it. As of the
2026-08-20 freeze it is **not running** — start the exe below when you want it.

- Executable: `D:\tmp\runcove-v030-demo\RunCove-demo.exe`, 25,788,332 bytes.
- **Close every production RunCove first.** `INSTANCE_MUTEX_NAME` is compile-time and
  identical in both builds, so the second one to start refuses to run.
- Data directory: `%LOCALAPPDATA%\com.abysswhale.runcove.demo0819`, archives under
  `run-log-archives\<session-id>.jsonl`, one record per line as
  `{"t":<epoch ms>,"s":"stdout|stderr|system","l":"<line>"}`.
- Demo project: `D:\tmp\runcove-v030-demo\demo-project`. `dev` prints about eight lines
  a second, every seventh to stderr, with a 30-minute guard; `burst` prints 1,200 lines
  at once.
- Rebuild recipe, if it is ever needed:
  `npm run tauri build -- --config D:\tmp\runcove-v030-demo\tauri.qa.conf.json
  --no-bundle` from `apps/desktop`, then copy the exe out of
  `src-tauri\target\release`. That config sets only the identifier.
- Current state as of the 2026-08-20 freeze: **exited**, the stored setting still
  archiving **ON**, the `runcove-demo-service / dev` profile idle, **13** runs of history
  covering every badge variant (未归档 / 归档中 / 完成 / 不完整 · 归档已被关闭 / 已删除 ·
  由你删除), and **four** archives on disk totalling 682,845 bytes (36,260 and 54,038
  toggle-closed, 82,805 and 509,742 complete). Click ▶ and the row reads 归档中
  immediately. A 15-byte placeholder from the initialization-failure proof is parked at
  `D:\tmp\runcove-v030-demo\run-log-archives.placeholder`; it is out of the data
  directory and nothing depends on it.
- The path through the UI, with the real labels: the toggle is **in the log drawer**,
  not on a settings page — open a profile's 日志 and 「归档运行日志」 sits under the log
  toolbar with the privacy warning (`archive.toggleHint`) directly beneath it. 概览 →
  「最近运行」 gives every row a badge, and an archived row offers
  「查看 <配置> 的归档日志」 and 「删除 <配置> 的归档日志」. The viewer opens on the tail,
  pages backwards with 「加载更早的日志」 until 「已到归档开头」, and the delete sits
  behind the 「删除归档日志？」 confirmation.

## 2026-08-18 v0.3.0 Writer Slice C: The Writer Is Complete

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`, and nothing was committed, pushed,
  tagged, or released. No CI, release, or `.env` file was touched, and no real
  database was opened. The working tree is still the same ten paths; the only source
  file this round changed is `apps/desktop/src-tauri/src/archive.rs`, which is
  untracked, so `git diff` still shows none of it.
- Scope: **slice C only**, as approved — `ArchiveWriter::close` (`archive.rs:2567`)
  and `ArchiveWriter::close_all` (`:2694`), with four private helpers and one shared
  struct. **No `todo!` is left in the module**, so step 4b's writer is done. `lib.rs`
  is unchanged, so `pub mod archive;` is still the module's only reference in the
  crate and the feature still has no runtime caller. The `Storage`-backed index, the
  commands, and the frontend were not begun.
- **The boundary is one critical section, and it is its own function.**
  `begin_close` (`:2729`) holds the open-session lock and then the queue lock in the
  fixed order, refuses anything that is not `SlotState::Open` with a per-state
  message like `take_slot`'s, flips the slot to `Closing`, and returns a
  `ClosingSession` (`:1655`) holding the handle, the row's current `lines`/`bytes`,
  and `queue.take_session(session_id)`. Both locks are released before any file work.
  That is what makes the boundary a single linearization point: afterwards `state` is
  not `Open`, so `enqueue` refuses the session, and the queue holds none of its
  records, so nothing can be stranded between those two facts. A refusal does no
  work and takes nothing — no file touched, no counter moved, **no index call** — so
  a second `close`, or a close of a session never begun, is inert.
- **`close` takes the pump lock first.** Not for the queue but for the *handle*: a
  pump borrows the session's handle out of its slot for the length of a write, so a
  close running concurrently would find `slot.file == None`, treat it as a refusal,
  and write nothing. The full order is `pump_lock → open → queue → total`.
- **The writes are settled as a batch and a failure is terminal.** `write_taken`
  (`:2773`) walks the taken records in arrival order, builds each payload with the
  shared `pending_write` (`:1625`), and appends it; on the first failure that record
  and **everything behind it** are charged as losses, because the handle has just
  said it will not take bytes. What landed is `release`d, the rest is `discard`ed,
  and the settling is one queue critical section *after* all the file work, so no
  lock is held across a write.
- **The residual gap is written here, and only here.** `write_residual_gap`
  (`:2806`) appends the run of losses no later record could carry, as a
  `LogStream::System` record whose text is `gap_line(gap)` and whose timestamp is
  **`ended_at`** — the writer reads no clock. It is counted as an archived line and
  charged to the quota like any other. This is exactly the line `writer_close`
  deliberately skips, and the difference is the file's state, not a change of mind: a
  file that has just failed a write or just been refused a byte by the cap must not
  be asked for one more line, and a close the user asked for is the case that can.
- **`append` (`:2837`) is the one place that counts.** It refuses immediately once
  `closing.failed` is set, so the gap line cannot follow a failed write; a missing
  handle is treated as a refusal, because from the row's side a disk that will not
  take the bytes and no disk at all are the same thing. A close **does not consult
  the quota**: its records were accepted while the session was open and its
  linearization point is already behind it, so there is nothing left to refuse them
  for. The overshoot is bounded by one session's queued bytes plus one gap line and
  is charged with `charge_written` like every other byte.
- **The verdict is a fold, not a choice.** `worsened` (`:1680`) accumulates:
  `QueueOverflow` when the row's drop counters are non-zero, `WriteError` when the
  file refused or could not be made durable, on top of whatever `reason` the caller
  gave. `ArchiveStatus::Complete` iff nothing folded, `Partial` with the most severe
  reason otherwise. `most_severe` is a total order, so the answer does not depend on
  the fold order — which is why `close_all(UserDisabled)` on a clean session still
  reports `user-disabled`.
- **Durability, then the row, in that order.** Flush and `sync_data` are both-or-
  neither (`flush_and_sync`); a handle that already failed is given up with
  `drop_without_flushing`. A count that could not be made durable is not believed —
  the file is measured and `reconcile_total` corrects the directory total, which is
  how a short write's real fragment reaches the row and the quota while the record
  that produced it stays a loss. The slot is removed **before** `index.close`, so an
  index failure returns `Err` with the bytes durable and the row still `writing` —
  the state the next startup sweep repairs to `partial` / `interrupted` — and never
  leaves a session that is open but unclosable.
- **`finish_session`'s `Err` is defaulted, with the reason written down.** It refuses
  only while the session still has queued records, and the boundary took every one of
  them, so it is unreachable; propagating it would leave the slot `Closing` for the
  rest of the run — a session no close could finish and no `enqueue` would feed.
- **`close_all` closes every session, not up to the first failure.** It snapshots the
  `Open` ids under the lock, closes each without it, remembers the **first** error and
  returns it at the end. One session's index failure must not leave the others open
  after the archive has been switched off. Sessions that are `Opening` or `Closing`
  are not on the list: they are not this call's to finish, and the refusal they would
  earn is not a failure worth reporting.
- **Three preparatory refactors, all behavior-preserving.** `discard_session`
  (`:1395`) now delegates to a new private `take_session` (`:1417`) — its old body's
  `charge_loss` + `free` is exactly `discard`, and the two callers differ in one
  thing: `discard_session` charges the lot, `close` releases what lands. The payload
  encoder moved out of `next_write` (`:2093`) into the free function `pending_write`,
  because `pump` encodes a record the queue still owns and `close` encodes ones it has
  already taken.
- **One test was added, and it is not a seam.** `a_close_writes_the_trailing_gap_no_
  later_record_could_carry` covers the one acceptance criterion the 14 red tests do
  not reach: every existing gap test has a following record that carries the gap, so
  nothing pinned the residual gap at close. It drops two records with nothing after
  them and asserts the file's last line is the `system` gap, that its `t` is the
  close's `ended_at`, `line_count == 3`, `byte_size == text.len()`, the row's drop
  counters, `partial` / `queue-overflow`, and no queue entry. No new seam, no new
  helper, no production change to make it pass. **No assertion anywhere was weakened,
  retargeted, or deleted.**
- Verification in `apps/desktop/src-tauri`. Each of the **14** C tests was run
  **alone** first and all 14 passed individually, as did the new one. The archive
  suite is **83** tests, **83 green / 0 red** (82 + the new one; 68 + 14 = 82 turned,
  and nothing that passed before stopped). `cargo test --lib` reports
  **`202 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out`** out of 203 —
  identical in parallel and under `-- --test-threads=1`. `cargo fmt --all -- --check`
  exits 0, `cargo clippy --all-targets --all-features -- -D warnings` is clean with no
  `#[allow]` added.
- **One environment failure, reported as it happened.** The first
  `--all-targets --all-features` run failed
  `commands::tests::manual_start_stays_starting_until_managed_expected_port_is_ready`
  at `commands.rs:2108` — `assertion failed: TcpStream::connect(("127.0.0.1",
  port)).is_err()`, i.e. the child bound its port before the main thread could check
  that it had not yet. It then passed **alone three times** and the whole
  `--all-targets --all-features` matrix passed on retry (`202 passed; 0 failed`). It
  is a timing assumption in a `commands.rs` test that was neither read for this
  purpose nor modified, and it is the second such port/child-timing test after
  `external_termination_with_verified_identity_stops_tree_and_releases_port` (which
  passed in every mode here). **No stable full-suite all-green baseline may be
  claimed.**
- **Still open.** The `line_count` over-count after a failed closing flush is recorded
  as **P2** by the user's instruction: the normal path is unaffected, no design was
  expanded for it this round, and whether to fix it is decided before the local demo
  is wrapped up. Not authorized: the `Storage`-backed `ArchiveIndex`, the commands,
  the frontend, `commit`, `push`, CI, Release, tags, `.env`, and any real database.
  The next milestone is the backend integration (the `Storage`-backed index, then the
  commands), and it needs the user's approval first.

## 2026-08-18 v0.3.0 Writer Slice B: The Steady-State Write Path Is Real

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`, and nothing was committed, pushed,
  tagged, or released. No CI, release, or `.env` file was touched, and no real
  database was opened. The working tree is still the same ten paths; the only source
  file this round changed is `apps/desktop/src-tauri/src/archive.rs`, which is
  untracked, so `git diff` still shows none of it.
- Scope: **slice B only**, as approved — the queue's batch/settle bodies with the
  reservation semantics, `ArchiveWriter`'s `enqueue` and `pump`, the `Closing` state
  and the private writer-initiated close, and the quota's eviction with its
  retryable-failure and short-write accounting. C was not started: exactly **two**
  `todo!("step 4b: ...")` bodies remain, `close` (`archive.rs:2506`) and `close_all`
  (`:2513`), down from ten. `lib.rs` is unchanged, so `pub mod archive;` is still the
  module's only reference in the crate and the feature still has no runtime caller.
- **The queue side.** All six settle bodies are implemented, and the ownership rule
  the last round designed now holds in code: `begin_batch` moves the queued records
  into the in-flight list and frees nothing, `front`/`peek_front`/`take_front` hand
  them out without freeing anything, and `release` (written) or `discard` (lost, and
  charged) is the only thing that frees room. `len`, `is_empty`, and `queued_bytes`
  are reservation readings — queued plus in-flight plus taken-and-unsettled — and
  `enqueue`'s `total_records` test reads that reservation rather than
  `self.queued.len()`. `drain` is gone.
- **`enqueue` is hand-over-hand, and a refusal is not a drop.** It holds the
  open-session lock only long enough to test `state == SlotState::Open`, takes the
  queue lock, and releases the open lock before touching the queue, so the fixed
  `open → queue` order holds with no window. A record for a session that is not
  `Open` is ignored and **deliberately not counted**: a closing or unknown session
  has no row left to carry a drop, so charging it would write a counter nobody can
  read. What the queue refuses for a *bound* is still charged, as before.
- **`pump` is one batch, and the loop never loses a record.** `begin_batch`, then for
  each front record: encode it (emitting its carried gap line first, as a
  `LogStream::System` record whose text is `gap_line(gap)` and whose timestamp is the
  carrier's, so `lines` is 1 or 2), read the session's counted bytes, ask `room_for`,
  `take_front`, write, settle. The three unreachable arms — a slot that vanished
  between two locks, a missing handle, a missing slot on the way back — all *charge*
  the record instead of dropping it, so no line that left a capture thread can leave
  the queue uncounted. The batch ends in `finish_batch`, which flushes once and calls
  `update_counters` once per session that actually wrote, and turns a failed flush
  into that session's `write-error` close while the rest of the batch still finishes.
- **`room_for` takes bytes, not a session id.** The per-session cap is checked first,
  because no eviction can help it — evicting someone else's archive does not make
  this session's own file smaller — and only then the directory total, in a loop:
  `Unavailable` refuses with no eviction at all (an unmeasured directory must not be
  grown), `held + cost <= total_bytes` is `Available`, and otherwise `evict_one`
  decides. The quota guard is never held across an eviction. An earlier draft passed
  the session id in and silenced it with `let _ = session_id;`; that is the bypass
  marker this repo forbids, so the parameter was removed rather than suppressed.
- **Eviction filters before it sorts and credits the disk.** `index.rows()` is read
  *before* the open-session lock to keep the order, candidates are narrowed to
  `Complete`/`Partial` rows with a non-null `ended_at` that this writer does not hold
  open, and the winner is the `min` by `ended_at`, then `started_at`, then
  `session_id`. It then goes through `verified_file_name` — a row naming another
  session's file is reported, not skipped — measures the file, removes it, credits
  the **measured** length, and only then writes `mark_removed(QuotaEvicted)`. One
  removal per call; the caller re-reads the total and asks again. A candidate that
  exists but will not go is `Err`, retried next tick with every record still
  reserved; *nothing eligible* is what closes the session `partial` /
  `quota-exceeded`.
- **The writer-initiated close is one linearization point.** `writer_close` holds the
  open-session lock and the queue lock in one critical section to flip the slot to
  `SlotState::Closing`, take its handle and counters, `discard_session`, and
  `finish_session` — then does every file operation with both released. That is why
  `enqueue` can refuse uniformly afterwards and why a record arriving during the file
  work has exactly two possible fates, written or refused, never stranded. The order
  inside a failing write matters and is deliberate: hand the handle back, charge the
  record, *then* close, so `discard_session` finds nothing of that session left and
  `finish_session` can complete instead of refusing.
- **What a writer-initiated close reports.** It is always `partial`, and its reason is
  `most_severe(reason, WriteError)` when the flush or `sync_data` also failed, so a
  quota close whose final flush breaks does not lose the worse fact. When the bytes
  are not durable it measures the file and reconciles both `byte_size` and the
  directory total from the disk — that is how a short write's real 4 096-byte fragment
  reaches the row and the quota while the record that produced it stays a loss. It
  writes **no gap line**: `finished.residual_gap` is deliberately left unwritten,
  because a session closing on a broken disk or an exhausted quota cannot be asked to
  append one more line, so the row's drop counters are the only surviving record of
  the loss. `SlotState` is load-bearing here rather than decorative — the slot stays
  `Open` while the pump has borrowed its handle out, which is the only reason a record
  can be enqueued *during* a held write.
- **One inexactness, stated rather than hidden.** A *small* record is buffered by the
  64 KiB `BufWriter`, counted into `line_count` when `write_all` returns, and then can
  be lost if the batch's closing flush fails. The byte side self-corrects (the close
  measures the file), but `line_count` can name a line the file does not hold. A
  pending-line counter would not fix it, because a flush can go out partially and
  saying which buffered lines survived needs each line's byte length. No current test
  covers it, it is reachable with the existing `fail_write_of` seam, and it is
  documented on `return_file`. It is an over-count on a failed-disk path, not a
  silently dropped line: every record the pump *refused* is charged.
- **The test migration was the authorized rewrite and nothing more.** `drain` is
  removed; a `settle_all(&mut queue)` helper (`begin_batch`, then `take_front` +
  `release` until empty) replaced it at all six sites, and `lines_and_gaps` is rebuilt
  on it. In `every_short_enqueue_sequence_keeps_the_queues_invariants` the local
  `drained` became `settled` and the wording became "a settled queue holds nothing",
  "settled in arrival order", and "the gap each settled record carries". That is
  exactly the permitted change — "drain 后为空" → "settle 后为空". **No assertion was
  weakened, retargeted, or deleted**, and the sequence sweep still checks every
  invariant it checked before.
- **Two visibilities narrowed, as CLAUDE.md required.** `ArchiveWriter::queue` is now
  a private field with `enqueue`/`pump`/`writer_close` as its production readers, and
  `ArchiveQueue::finish_session` is module-private with `writer_close` as its
  production caller — neither is "a `pub` API with no caller" any more.
  `ArchiveQueue::peek_front` stays `pub` with test-only callers (`next_write` uses the
  module-private `front()`, which returns the whole record rather than a name and a
  length); it is on a `pub` type, so that is not `dead_code`.
- Verification in `apps/desktop/src-tauri`. Each of the 15 target tests was run
  **alone** first and all 15 passed individually, as did the rewritten
  `every_short_enqueue_sequence_keeps_the_queues_invariants`. `cargo fmt --all --
  --check` exits 0 and `cargo clippy --all-targets --all-features -- -D warnings` is
  clean, so every field the earlier rounds could not add a reader for — `limits`,
  `pump_lock`, `OpenArchive::lines`/`bytes`, `SlotState::Closing` — now has one, with
  no `#[allow]` anywhere. `cargo test --lib` reports
  **`187 passed; 14 failed; 1 ignored; 0 measured; 0 filtered out`** out of 202 —
  identical in parallel, under `-- --test-threads=1`, and under
  `--all-targets --all-features` (whose second target, the binary, has no tests).
  The archive suite is **82** tests, **68 green / 14 red**, up from 53/29: 172 + 15 =
  187 and 29 − 15 = 14, so exactly the fifteen turned and nothing that passed before
  stopped passing.
- **The red evidence, by panic site.** All 14 failures are `todo!` panics — measured,
  not inferred: 14 `panicked at` lines, 14 `not yet implemented` lines, and **zero**
  `assertion` / `left ==` / `right ==` lines. Sites: `archive.rs:2506`
  (`ArchiveWriter::close`) ×12 and `:2513` (`close_all`) ×2. Three of the twelve report
  under `thread '<unnamed>'` because they panic on the helper thread the close-race
  seam spawns; `CallHold::wait_for` re-raises it rather than hanging the suite, which is
  the behavior that machinery was built for.
- **The environment failure still did not reproduce here.**
  `commands::tests::external_termination_with_verified_identity_stops_tree_and_releases_port`
  passed in all three full-suite modes this round (it is inside the 187, since all 14
  failures are archive `todo!`s). It kills a real process tree and binds a real port,
  so it stays environment-dependent, `commands.rs` was not read for this purpose and
  not modified, and **no stable full-suite all-green baseline may be claimed**.
- Next step: **C**, which the user pre-authorized to follow B directly with no new
  architecture-divergence review unless a P0/P1 appears (data loss, an out-of-bounds
  write, a wrong deletion, a deadlock, or an interface that cannot be wired up). This
  round stopped at B's boundary because the approval also said to stop and report
  first. C is two bodies and **14** red tests:
  - `close` (`archive.rs:2506`) — the caller-facing close. Its obligations, read off
    the red tests rather than invented: refuse a second close from the writer's own
    open-session state with the file, the row, and `index.calls()` unchanged; mark
    `Closing` and extract that session's accepted records in one critical section, then
    write them, so the record accepted *before* the close lands and the one arriving
    *during* it does not; write the **residual gap line** that `writer_close`
    deliberately skips, and count it as an archived line; `complete` with
    `ArchiveCounters::default()` and a genuinely empty file for a session that wrote
    nothing; `complete` for `reason == None` and `partial` + reason otherwise; flush and
    `sync_data` before the row, with a failing `sync_data` becoming
    `partial` / `write-error`; leave the bytes durable and the row `writing` for the
    sweep when the index write itself fails; and leave no queue entry behind
    (`assert_no_queue_entry` is asserted by seven of these tests).
  - `close_all` (`:2513`) — the same for every open session under one reason, `Ok` and
    inert with none open, and after it a late `enqueue` plus a `pump` must leave both
    files byte-identical, both rows equal as whole `ArchiveRow` values, and
    `index.calls()` untouched.
  - The 14, by panic site: at `close` —
    `a_failing_sync_data_closes_the_session_partial_write_error`,
    `a_quota_close_counts_every_record_it_never_wrote`,
    `a_record_accepted_before_a_close_lands_and_one_arriving_during_it_does_not`,
    `a_record_at_the_closing_boundary_is_refused_and_leaves_no_entry`,
    `a_record_for_a_closed_session_never_takes_the_queues_room`,
    `a_record_racing_a_close_never_takes_the_room_the_next_session_needs`,
    `a_refused_close_row_keeps_the_bytes_and_leaves_a_repairable_row`,
    `a_session_that_wrote_nothing_closes_complete_and_empty`,
    `a_session_writes_its_writing_row_then_its_lines_then_complete`,
    `closing_the_same_session_twice_is_refused_by_the_open_state_and_changes_nothing`,
    `deleting_an_archive_is_refused_while_its_writer_is_open`,
    `gap_records_sum_to_the_dropped_counters_of_a_closed_archive`; at `close_all` —
    `enqueueing_after_close_all_user_disabled_changes_no_file_and_no_row`,
    `turning_the_archive_off_closes_open_sessions_partial_user_disabled`.
  - After C: the backend integration milestone (the `Storage`-backed `ArchiveIndex`,
    then the commands) and the frontend demo. Still not authorized: the
    `Storage`-backed index, the commands, the frontend, `commit`, `push`, CI, Release,
    tags, `.env`, and any real database.

## 2026-08-18 v0.3.0 Retry-Contract Red Tests: The Queue Owns What It Handed Out

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`, and nothing was committed, pushed,
  tagged, or released. No CI, release, or `.env` file was touched, and no real
  database was opened. The working tree is still the same ten paths; the only source
  file this round changed is `apps/desktop/src-tauri/src/archive.rs`, which is
  untracked, so `git diff` still shows none of it.
- Scope, and it is deliberately red: the five corrections the review attached to its
  refusal of the B/C plan, as tests and design only. **No body was implemented.** The
  only production change is API surface: [`ArchiveQueue`] gained the six-method batch
  and settle protocol, all `todo!`, and `ArchiveWriter` gained the `queue` field the
  close tests read. Ten `todo!("step 4b: ...")` bodies now exist — `begin_batch`
  (`archive.rs:1294`), `peek_front` (`:1301`), `take_front` (`:1314`), `release`
  (`:1320`), `discard` (`:1334`), `discard_session` (`:1346`), `enqueue` (`:1682`),
  `pump` (`:1689`), `close` (`:1724`), `close_all` (`:1731`) — up from four, because
  the retry contract is a queue-level contract and had nowhere to be stated before.
  Slice A's `read`, `delete`, `credit_removed`, and the total's mutex are untouched
  and still green.
- **The ownership answer (review item 1).** `in_flight: usize` plus a local
  `Vec<QueuedRecord>` inside `pump` was wrong and is abandoned: a `pump` that returns
  `Err` drops that local vector, and with it the records. The queue now owns every
  record from `enqueue` until it is *settled*, and settling is exactly two calls —
  `release` (written, room comes back) or `discard` (lost, room comes back and the
  loss is charged). `begin_batch` moves the queued records into the in-flight list and
  frees nothing; `peek_front` and `take_front` hand them out one at a time without
  freeing anything either. So a failed pump needs no undo: there is nothing to put
  back, the next `begin_batch` appends the arrivals *behind* whatever the last pump
  left, and the retry resumes at the same record in the same order. That is what
  `a_failed_batch_stays_in_flight_and_the_next_batch_appends_behind_it` pins.
- **The consequence B must implement, stated once here.** `len()`, `is_empty()`, and
  `queued_bytes()` become *reservation* accounting — queued plus in-flight plus taken
  and not yet settled — and `enqueue`'s `total_records` test has to move off
  `self.queued.len()` for the same reason. Every existing green queue test is
  unaffected, because with no batch in flight the two readings coincide.
  `an_in_flight_batch_counts_against_all_four_bounds` is the one that would catch a
  bound that stopped counting an in-flight record: one case per bound, each set so
  only that bound can be the refuser, and each asking again both while a record is in
  flight and while one is taken-but-unsettled.
- **A repeatable failure had to be found (review item 2).** The requirement is that a
  pump can fail round after round with every record still there afterwards, and the
  first two candidates cannot do it. `index.fail("mark_removed")` *progresses* — the
  file is removed and its bytes credited on the first round, so later rounds need no
  eviction at all — and a write error is *terminal* for its session, which closes
  `partial` / `write-error` and can never fail again. The only genuinely repeatable
  pre-write failure is an eviction whose candidate exists and whose file will not go:
  sticky `seam.fail_remove()`. `repeated_eviction_failures_keep_every_record_and_never_relax_the_bounds`
  runs three such rounds, then lifts the fault.
- **And the design rule it forces.** *A candidate exists but could not be removed* is
  not the same state as *nothing is eligible*: the first is reported and retried on the
  next tick, the second is what closes the session `partial` / `quota-exceeded` in
  `when_nothing_can_be_evicted_the_open_session_closes_partial_quota_exceeded`. On
  Windows a failed removal is usually a transient handle — a scanner or the indexer —
  and throwing a session's logs away for another file's handle is the wrong trade. The
  four bounds are what keeps memory bounded across an indefinite run of them, which is
  why the test also asserts one refusal per round and that the two held records never
  widen anyone's room.
- **The crossed-row eviction (review item 4).** `verified_file_name` has to run on the
  eviction path too, because `file_name` is the one row field that decides which bytes
  go. `evicting_a_row_that_names_another_session_touches_nothing_and_loses_no_record`
  seeds a correct row before `initialize` — so the file is *owned* and its 400 bytes
  measured — and only then crosses it to another session's name, because a crossed row
  at sweep time leaves the file unowned and `delete_orphans` takes it before the quota
  ever looks. The pump must return `Err` rather than skip to the next candidate, and
  the writing session's record must survive that `Err`: `seam.removed()` is empty (it
  logs attempts, not successes, so empty is the strong claim), both rows compare equal
  as whole `ArchiveRow` values, and the record is still in the queue. Repairing the row
  and pumping again writes it exactly once.
- **The short write (review item 5).** `TestFile::write` can now accept part of a
  buffer and fail on everything after it, which is the only seam that leaves a fragment
  of a line on the disk and then takes the rest away.
  `a_short_write_then_a_failure_counts_the_bytes_it_left_on_disk` uses a 256 KiB line
  so the record bypasses the `BufWriter` entirely, accepts 4 096 bytes, and then pins:
  the file holds exactly that prefix and no newline, `byte_size` is 4 096 — the real
  residue, so a hard quota cannot be under-counted — `line_count` is 0 because a
  fragment is not a line, and the record is charged to `dropped_lines` /
  `dropped_bytes`. B has a consequence to face here: `io::Write::write_all` never
  reports how much it wrote, so the write-error close must measure the file (or write
  through a loop that counts) instead of assuming the buffer went in whole.
- **The leak the files and rows cannot show (review item 3).** A new
  `assert_no_queue_entry(&writer, session_id)` helper reaches straight into
  `writer.queue.lock().sessions` — the test module can see the private field, and a
  production accessor for it would only invite dependence — and it is now asserted by
  **seven** close tests: `enqueueing_after_close_all_user_disabled_changes_no_file_and_no_row`,
  `a_record_for_a_closed_session_never_takes_the_queues_room`,
  `a_record_accepted_before_a_close_lands_and_one_arriving_during_it_does_not`,
  `a_record_racing_a_close_never_takes_the_room_the_next_session_needs`,
  `a_record_at_the_closing_boundary_is_refused_and_leaves_no_entry`,
  `a_session_that_wrote_nothing_closes_complete_and_empty` (which pins that a close may
  not *create* one), and
  `closing_the_same_session_twice_is_refused_by_the_open_state_and_changes_nothing`.
  Without it, a record that reached the queue after its session closed leaves an entry
  carrying `pending` and `dropped` history no file will ever explain, and nothing
  removes it — one entry per closed session for the life of the process, growing with
  sessions rather than with open sessions, which is the same shape as the bug the
  lifecycle patch fixed.
- **The boundary test is no longer permissive.** Because the committed close marks the
  session closing *and* extracts its accepted records in one critical section, a record
  arriving during the file work that follows can only be refused. So
  `a_record_at_the_closing_boundary_is_either_written_or_refused` was renamed
  `a_record_at_the_closing_boundary_is_refused_and_leaves_no_entry` and its
  `if body == kept … else if body == with_boundary …` branch replaced by a single
  equality on the kept body plus `line_count == 1`. This is a tightening, not a
  retarget: the outcome it used to allow as a second correct answer is the one the
  design now rules out.
- **Two seams added, one refused.** `TestFs::fail_write_of(file_name)` and
  `TestFs::short_write_then_fail_of(file_name, accepted)` inject by *name*, so a
  two-session test says which session's disk failed instead of depending on which one
  the writer reached first — `a_failing_write_closes_only_that_session_partial_write_error`
  switched to it, which is also what keeps `fail_write_at` from being the only seam
  with a caller. `allow_write_of` was **not** added: a write failure is terminal for
  its session, so nothing in the writer can ever recover that file's writes, an
  uncalled test helper is `dead_code` under `-D warnings`, and an `#[allow]` bypass is
  forbidden here.
- **One deferral, stated rather than hidden.**
  `every_short_enqueue_sequence_keeps_the_queues_invariants` was *not* converted to the
  batch protocol this round. A `todo!` at the first sequence's `begin_batch` would abort
  the test before any sequence finished, taking out all of its enqueue and bounds
  coverage for a round, while the reservation contract is already stated in full by the
  new dedicated tests. The conversion belongs in B, where it can be green in the same
  round; the end-state assertions are identical either way.
- Verification in `apps/desktop/src-tauri`: `cargo fmt --all -- --check` exits 0,
  `cargo clippy --all-targets --all-features -- -D warnings` is clean, and
  `cargo test --lib --all-features` reports
  **`172 passed; 29 failed; 1 ignored; 0 measured; 0 filtered out`** out of 202 —
  identical in parallel, under `-- --test-threads=1`, and under
  `--all-targets --all-features`. The archive suite is **82** tests, 53 green / 29 red;
  53 green is unchanged from slice A, so nothing that passed before stopped passing.
- **The red evidence, by panic site.** All 29 failures are `todo!` panics — measured,
  not inferred: the output contains 29 `panicked at` lines, 29 `not yet implemented`
  lines, and **zero** `assertion` / `left ==` lines. Sites: `archive.rs:1682`
  (`ArchiveWriter::enqueue`) ×23, `:1294` (`ArchiveQueue::begin_batch`) ×3, `:1724`
  (`ArchiveWriter::close`) ×2, `:1301` (`ArchiveQueue::peek_front`) ×1. The seven new
  tests are 4 queue-level and 3 writer-level; note that
  `a_failed_batch_stays_in_flight_and_the_next_batch_appends_behind_it` stops at
  `peek_front`, not at `begin_batch`, because it asserts `peek_front() == None` *before*
  asking for a batch — the prediction that all four queue tests would stop at
  `begin_batch` was wrong by one.
- **The environment failure could not be reproduced here.**
  `commands::tests::external_termination_with_verified_identity_stops_tree_and_releases_port`
  failed with a Windows "Access denied" in the reviewer's run and reproduced there when
  run alone. In this session it passed in all three full-suite modes and again when run
  alone (`1 passed`, 5.42s). It kills a real process tree and binds a real port, so
  treat it as environment-dependent and flaky rather than as a current failure — and
  therefore still do **not** claim a stable all-green baseline for it. `commands.rs` was
  not read for this purpose and not modified; fixing it is not part of this slice.
- Next step: **B only**, and it needs approval before any body is written. B is the
  queue's six settle methods plus `enqueue` and `pump` with its eviction; C — `close`
  and `close_all` — is requested separately once B is green. The full revised plan for
  both slices is in `V0.3.0_PLAN.md` under the Execution Order's `4b, slice B` and
  `4b, slice C` entries; B also owns the `closing` state on `OpenArchive`, the private
  writer-initiated close its failure paths need, and the reservation meaning of `len` /
  `is_empty` / `queued_bytes`. The 29 red tests split **15 to B, 14 to C** by one
  mechanical criterion — a test that itself calls the public `close` or `close_all`
  belongs to C — and the split was measured by reading every red test for such a call,
  not inferred from names. It moves `a_quota_close_counts_every_record_it_never_wrote`
  into C: its subject is B's quota work, but it ends with a `close`. Two carry-forward
  rules for
  B: the `queue` field is `pub` for this one slice, for the same reason
  `ArchiveQueue::finish_session` is (a private field no body reads is `dead_code` under
  `-D warnings`), and it becomes private in the slice that makes `enqueue`/`pump` its
  production readers; and `drain` is superseded by the batch protocol and should be
  removed in that same slice, once the tests written against it have migrated. Still not
  authorized: the `Storage`-backed index, the commands, the frontend, `commit`, `push`,
  CI, Release, and tags.

## 2026-08-18 v0.3.0 Writer Slice A: The Two Paths That Only Take Bytes Away

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`, and nothing was committed, pushed,
  tagged, or released. No CI, release, or `.env` file was touched, and no real
  database was opened. The working tree is still the same ten paths; the only source
  file this round changed is `apps/desktop/src-tauri/src/archive.rs`, which is
  untracked, so `git diff` still shows none of it.
- Scope: slice A only, as approved — the quota total's mutex, `credit_removed`,
  `read`, and `delete`. `pump`, `close`, `close_all`, and eviction were deliberately
  **not** started, so four `todo!("step 4b: ...")` bodies remain: `enqueue`
  (`archive.rs:1587`), `pump` (`:1594`), `close` (`:1629`), `close_all` (`:1636`).
  The `Storage`-backed `ArchiveIndex`, the commands, the frontend, CI, the release
  workflow, tags, and `.env` were not touched, and `lib.rs` is unchanged, so
  `pub mod archive;` is still the module's only reference in the crate — the feature
  still has no runtime caller.
- **What was implemented**, all in `ArchiveWriter`:
  - `total: QuotaTotal` became `total: Mutex<QuotaTotal>` (`:1369`), read through
    `quota_total()` (`:1439`) by `total_bytes` (`:1435`). It is the leaf of the lock
    order: nothing else is taken while it is held, and it is never held across a file
    operation or an index write, so a delete on a command thread and a pump on
    another never wait on each other's disk.
  - `credit_removed(measured)` (`:1456`) — saturating, so a total that has drifted
    below one file's length cannot wrap; and a `QuotaTotal::Unavailable` total stays
    unavailable, because a delete says how long one file was, not how much the
    directory holds. Only the next startup sweep recovers a real total.
  - `read` (`:1662`) — row, then `verified_file_name`, then
    `resolve_ordinary_archive_file`, then the bytes. A file under a generated name
    with no row is not readable through this path; it is what the sweep deletes as an
    orphan.
  - `delete` (`:1700`) — refuse while open, row, `verified_file_name`, measure,
    remove, credit, then `mark_removed(UserDeleted, storage::now_ms())`. The
    measurement has to precede the removal because afterwards nobody can take it, and
    the credit has to precede the row write because the bytes are gone whatever the
    index answers: a delete whose row will not move still leaves the total telling the
    truth, returns the failure, and leaves the row for the sweep.
  - `row_of` (`:1732`) — the shared first step of both, so neither reaches the
    filesystem for a session the index does not know.
- **The one honest shortfall.** The plan said slice A would turn six tests green. It
  turns **five**. `deleting_an_archive_is_refused_while_its_writer_is_open` asserts
  the refusal, then closes the session and deletes again, so it now panics at `close`
  (`archive.rs:1629`) instead of at `delete`. Its whole delete half passes; the test
  cannot be green until the `close` slice lands, and no assertion was weakened to
  pretend otherwise.
- **Two design points worth carrying forward.** The open-session refusal in `delete`
  is a check *before* the work, not one critical section with it, because no state
  lock spans a file removal. Two other things make that safe and are documented at
  the call site: an archive file is created with `create_new`, so a `begin` racing a
  delete cannot take the file that is still there, and a session id is generated once
  for the run that produces it, so the id a user deletes is not one a later run
  begins. Separately, a row whose file is already missing is *reported* rather than
  quietly marked: that state is the sweep's to finish, and it finishes it as
  `removed` / `file-missing`, which is what actually happened.
- Two stale `// PLACEHOLDER: ...` comments above real assertions were removed
  (`a_write_error_counts_every_record_it_could_not_persist` and
  `a_delete_whose_row_will_not_move_still_frees_the_bytes_it_removed`), and that
  second test's in-line comment now says the 100 real bytes the assertion checks
  instead of 40.
- Verification in `apps/desktop/src-tauri`: each of the six target tests was run
  alone first — five `ok`, the sixth failing at `close` as described. Then
  `cargo fmt --all -- --check` exits 0, `cargo clippy --all-targets --all-features --
  -D warnings` is clean, and `cargo test --lib --all-features -- --test-threads=1`
  reports **`172 passed; 22 failed; 1 ignored; 0 measured; 0 filtered out`** out of
  195, identical in parallel and under `--all-targets --all-features`.
- **The red evidence.** All 22 failures are `todo!` panics and the runs contain
  **zero** `assertion` / `left ==` / `left !=` lines. By panic site:
  `archive.rs:1587` (`enqueue`) ×20 and `archive.rs:1629` (`close`) ×2. The five
  `delete` panics and the one `read` panic are gone; one of the six moved to `close`,
  which is the +1 there. The archive suite is **75** tests, 53 green / 22 red.
- Next step: B/C remain unauthorized and their plan must be revised first, with red
  tests for each of the five preconditions the review set (eviction through
  `verified_file_name` plus a crossed-session file-name test; in-flight/reservation
  accounting instead of `requeue_front`, with repeated-failure and extra-enqueue
  determinism tests; the `discard` rule that a carrier's `gap_before` returns only to
  `pending`, with
  `discarding_a_gap_carrier_does_not_count_the_carried_loss_twice`; and a closing
  boundary that is either a deterministic refusal or observable through a seam).
  Still not authorized: the `Storage`-backed index, the commands, the frontend,
  `commit`, `push`, CI, Release, and tags.

## 2026-08-17 v0.3.0 Review Revisions: The Closing Boundary Is A Linearization Point

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`, and nothing was committed, pushed,
  tagged, or released. No CI, release, or `.env` file was touched. The working tree
  is still the same ten paths, and the only source file this round changed is
  `apps/desktop/src-tauri/src/archive.rs`, which is untracked, so `git diff` still
  shows none of it. `V0.3.0_PLAN.md`, this file, `notes.md`, and `CLAUDE.md` carry
  the decisions below.
- Scope held, and this round is deliberately red. No writer production code was
  written: `enqueue`, `pump`, `close`, `close_all`, `read`, and `delete` are the same
  six `todo!("step 4b: ...")` bodies. Tests, test seams, and doc comments only. The
  `Storage`-backed `ArchiveIndex`, the commands, the frontend, CI, the release
  workflow, tags, and `.env` were not touched, and `lib.rs` is unchanged, so
  `pub mod archive;` is still the module's only reference in the crate.
- **Why this round exists.** The review accepted the closing-boundary tests and the
  measured baseline and refused the A/B/C implementation plan pending four revisions,
  each to be carried by a new red test. Six tests were added and all six are red at a
  `todo!`. The archive suite is now **75** tests, 48 green / 27 red.
  1. **The close boundary is one linearization point, not `pump` then a state
     flip.** `enqueue` does not take the pump's lock, so a record can still be
     accepted between the last drain and the flip. Under the fixed open → queue lock
     order the session is marked closing *and* its accepted records are extracted in
     the same critical section; only then does the file work start, with both locks
     released. `a_record_at_the_closing_boundary_is_either_written_or_refused` stands
     in exactly that window. It permits both correct answers — written, or refused —
     and forbids the third: accepted and left in the queue. That third outcome is
     caught twice over, because the close could not then finish its session, and
     because with `one_slot_bounds()` the stranded record holds the only slot the next
     live session needs.
  2. **A close the writer starts itself must count what it never wrote.** A write
     error or a crossed quota closes a session mid-batch while `enqueue` is still
     accepting, so records can be accepted and unpersisted at that instant. They leave
     the queue and are charged to `dropped_lines` / `dropped_bytes`; "silently discard
     the rest of the batch" was refused.
     `a_write_error_counts_every_record_it_could_not_persist` counts four lost lines,
     one of them handed over from inside the held `write`.
     `a_quota_close_counts_every_record_it_never_wrote` counts the record that crossed
     the cap plus the two behind it — 3 lines / 111 bytes — and pins that a quota
     close writes no gap line, which makes the row's counters the only surviving
     record of the loss.
  3. **Eviction eligibility and order.** Only `complete` or `partial` rows whose
     `ended_at` is non-null, ordered by `ended_at`, then `started_at`, then
     `session_id`. `eviction_orders_by_ended_at_and_skips_a_session_that_never_ended`
     sets the two timestamps against each other — one archive started first and ended
     last, the other started last and ended first — so exactly one key can be right.
     The existing `the_total_cap_evicts_ended_archives_oldest_first_and_never_an_open_one`
     cannot see this, because both of its rows share an `ended_at` and its verdict
     rests entirely on the tie-break. The third row is `complete` with no `ended_at`,
     which RunCove's own schema forbids
     (`CHECK ((status = 'writing') = (ended_at IS NULL))`) and only a foreign database
     can produce: it has to be skipped rather than sorted, since a missing timestamp
     read as zero would make it the first candidate of all.
  4. **A removed file frees the bytes it really held.** The in-memory total is
     credited with the length measured on disk even when the row update that follows
     fails, and the inconsistent row is left for the next startup sweep.
     `a_delete_whose_row_will_not_move_still_frees_the_bytes_it_removed` and
     `an_eviction_whose_row_will_not_move_still_frees_the_bytes_it_removed` both seed
     10 bytes in the row against a longer file — 40 bytes on the delete path, 400 on
     the eviction path — so crediting the row and crediting the
     disk are different numbers, and they assert the disk's. The delete test also
     shows the row is genuinely left for the sweep: a second `initialize` over the
     same directory reports it in `marked_file_missing` and moves it to `removed` /
     `file-missing`. One consequence for the slicing — the quota total's interior
     mutability moves into the same slice as `delete`, which decrements it.

- **Two wording rulings, applied in the code and in the plan.**
  - `enqueue` is **not** lock-free, and must not be described as never blocking. It
    enters a short in-memory critical section — the open-session state, then the
    queue — because whether a session is still accepting is part of the queue's
    answer. What it does guarantee is that no capture thread ever waits on a disk or a
    database: no file operation and no index call happens inside those sections.
  - "No lock is held across I/O" is false as a blanket claim. It holds for the three
    state locks — open sessions, queue, quota total. The pump's lock deliberately
    spans the writer's file and index work, because serializing exactly that is its
    purpose.
- **The new seam, and why `sync_data` could not serve.** The sync gate fires after
  the state change under every ordering, so it cannot stand in the disputed window.
  `TestFs::hold_write_of(file_name)` holds one named file's next `write` instead.
  `SyncGate` and `SyncHold` became `CallGate` and `CallHold`, carrying the name of the
  call they hold; `TestFsState` gained a second one-shot slot so a write gate and a
  sync gate are armed independently; and `TestFile::write` reaches its gate *before*
  the injected failure, because a test that holds a write in order to watch it fail
  would otherwise never reach the rendezvous. A record only reaches a `write` inside
  `pump` if it is longer than `WRITE_BUFFER_BYTES`, which is why the two tests that
  hold one use a 64 KiB line. No existing behavior changed, since no existing test
  arms a write gate.
- Verification in `apps/desktop/src-tauri`: `cargo fmt --all -- --check` exits 0
  (rustfmt was run first and its diff applied), `cargo clippy --all-targets
  --all-features -- -D warnings` is clean, and `cargo test --lib` reports
  **`167 passed; 27 failed; 1 ignored; 0 measured; 0 filtered out`** out of 195,
  identical in parallel, single-threaded, and under `--all-targets --all-features`.
- **The red evidence.** All 27 failures are `todo!` panics, and the runs contain
  **zero** `assertion` / `left ==` / `left !=` lines. By panic site:
  `archive.rs:1553` (`enqueue`) ×20, `archive.rs:1636` (`delete`) ×5,
  `archive.rs:1595` (`close`) ×1, `archive.rs:1626` (`read`) ×1. The six new tests
  account for the +6: five stop at `enqueue`, the first unwritten body each of them
  touches, and `a_delete_whose_row_will_not_move_still_frees_the_bytes_it_removed`
  stops at `delete`. Neither race test reaches its gate yet, so the write gate is
  still unexercised machinery, waiting for the slice that makes `enqueue` real.
- Next step: resubmit the revised writer-slice plan for approval, then implement it.
  Still not authorized: the `Storage`-backed index, the commands, the frontend,
  `commit`, `push`, CI, Release, and tags.

## 2026-08-17 v0.3.0 Closing-Boundary Red Tests: What May Not Follow A Close

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`, and nothing was committed, pushed,
  tagged, or released. No CI, release, or `.env` file was touched. The working tree
  is still the same ten paths, and the only source file this round changed is
  `apps/desktop/src-tauri/src/archive.rs`, which is untracked, so `git diff` still
  shows none of it.
- Scope held, and this round is deliberately red. No writer production code was
  written: `enqueue`, `pump`, `close`, `close_all`, `read`, and `delete` are still
  the same six `todo!("step 4b: ...")` bodies. The `Storage`-backed `ArchiveIndex`,
  the commands, the frontend, CI, the release workflow, tags, and `.env` were not
  touched. `lib.rs` is unchanged, so `pub mod archive;` is still the module's only
  reference in the crate.
- **Six tests were added, and all six fail at the boundary they are meant to
  fail at.** They state what may not happen after a session closes, which is the
  part of the write path where the queue, the file handle, and the row can disagree.
  The archive suite is now **69** tests, 48 green / 21 red.
  1. `enqueueing_after_close_all_user_disabled_changes_no_file_and_no_row` — the
     toggle goes off, then two capture threads that had not noticed hand over a
     line each and a pump runs. Both files must be byte-identical to what the close
     left, both rows must compare equal as whole `ArchiveRow` values (drop counters
     included), and `index.calls()` must be *unchanged* — no counter update, no
     second close.
  2. `a_record_for_a_closed_session_never_takes_the_queues_room` — the same
     boundary read through the bounds instead of the file. With
     `one_slot_bounds()` (one record in total) a line wrongly queued for a closed
     session would be holding the only slot, so the live session's line would be
     the one refused. The live row's `dropped_lines == 0` is the assertion.
  3. `a_record_accepted_before_a_close_lands_and_one_arriving_during_it_does_not` —
     the close runs on its own thread and is held inside `sync_data`; a record
     enqueued at that instant must not reach the file, the counters, or the drop
     counters, while the record accepted before the close must be on disk.
  4. `a_record_racing_a_close_never_takes_the_room_the_next_session_needs` — the
     same race with `one_slot_bounds()`, so a record the writer wrongly kept for a
     finished session is visible as the live session's loss.
  5. `a_session_that_wrote_nothing_closes_complete_and_empty` — an empty session
     closes `complete` with `ArchiveCounters::default()`, its file stays and stays
     empty (no gap line for a loss that never happened). It has no queue entry at
     all, which is exactly why `finish_session` answering "nothing owed" for a
     session it has never seen is right.
  6. `closing_the_same_session_twice_is_refused_by_the_open_state_and_changes_nothing`
     — the second `close` is an error found by the writer's own open-session state,
     with the file, the row (`ended_at` included), and the call log unchanged;
     `close_all` with nothing open is then `Ok` and equally inert.
- **The concurrency seam, and why it is not a timing test.** `TestFs` gained one
  new capability: `hold_sync_of(file_name)` arms a `SyncGate` that holds the next
  `sync_data` on that one file until the test lets it go. (The types were renamed
  `CallGate` and `CallHold` in the review-revision round above, which generalized
  them to hold any named call.) It is a rendezvous, not a
  delay — each side blocks until the other arrives — and it is keyed by file name
  and fires once, matching the file's existing rule that injection is by call count
  or by name and never by timing. Two details make it safe:
  - The gate is taken out from under the `TestFsState` lock before it blocks, so no
    lock is ever held across the pause, and the concurrent `enqueue` cannot deadlock
    against the filesystem double.
  - `SyncHold::wait_for` (now `CallHold::wait_for`) will not wait forever for a
    rendezvous that cannot come.
    It polls the channel, and when the channel is empty *and* the closing thread has
    finished, it drains once more and then re-raises that thread's panic with
    `resume_unwind`. That is why the two race tests report the missing `enqueue`
    body today instead of hanging the suite, and it is the only reason a duration
    appears anywhere in the file. Nothing a test asserts depends on it.
  - `TestFile` now carries its own `name` so the gate can name a file rather than
    count writes across every handle. `sync_data` was chosen because a close reaches
    it with the session's records already flushed, so a record enqueued at that
    instant is unambiguously late. It says nothing about where the closing boundary
    sits, and it cannot: under every ordering the state has already changed by the
    time `sync_data` runs. Standing in the boundary's own instant needs a gate inside
    `write`, which is what the review-revision round above added.
- **How "no queue entry was re-created" is asserted without a queue field.**
  `ArchiveWriter` still has no `ArchiveQueue` field — `initialize` still does
  `let _ = (bounds, limits);` — so no test can read the writer's queue state
  directly, and none pretends to. `one_slot_bounds()` converts the invisible into
  the visible instead: with room for exactly one queued record in total, a record
  the writer should never have queued is not invisible, because it would be holding
  the slot the next live session needs. Tests 2 and 4 read that session's loss. When
  the writer slice gives the writer its queue, these two tests keep working
  unchanged and a direct assertion on the map can be added beside them.
- **Two documented reasons were corrected in the section below, in this file, in
  `notes.md`, and in `V0.3.0_PLAN.md`.**
  - `AppResult` over `Option` for `finish_session` is **not** about `#[must_use]`:
    `Option` is `#[must_use]` too. It is about three outcomes rather than two — a
    residual gap, nothing owed, and *not now, records still queued* — where folding
    the third into the second would let a close file a row while bytes were still
    owed.
  - `finish_session` being `pub` is described as **having no runtime caller**, not
    as an unreachable API. A `pub` method of a `pub` module is reachable by
    definition; what is true is that every call site today is a test. It must be
    narrowed to module-private the moment the writer becomes its production caller.
- Verification in `apps/desktop/src-tauri`: `cargo fmt --all -- --check` exits 0
  (one rustfmt diff — a stray blank line — was applied, then re-checked clean);
  `cargo clippy --all-targets --all-features -- -D warnings` is clean;
  `cargo test --lib` reports **`167 passed; 21 failed; 1 ignored`** out of 189,
  identical in parallel, single-threaded, and under `--all-targets --all-features`.
  Focused, the archive suite is `48 passed; 21 failed; 120 filtered out`.
- **The red evidence.** Every one of the 21 failures is a `todo!` panic, and the run
  contains **zero** `assertion` / `left ==` / `left !=` lines. By panic site:
  `archive.rs:1545` (`enqueue`) ×15, `archive.rs:1575` (`close`) ×1,
  `archive.rs:1606` (`read`) ×1, `archive.rs:1616` (`delete`) ×4. The six new tests
  account for the +6: five stop at `enqueue`, which is the first unwritten body each
  of them touches, and
  `a_session_that_wrote_nothing_closes_complete_and_empty` stops at `close`, because
  it is the one test of the six that never enqueues anything. Neither race test
  reaches its seam yet for the same reason — `enqueue` panics on the main thread
  before the close thread is spawned — so the gate itself is still unexercised
  machinery, waiting for the slice that makes `enqueue` real.
- Next step: submit the writer slice's implementation plan for approval, then
  implement it. Two structural facts it has to face, both visible in the code now:
  the writer has no queue field (`initialize` discards `bounds` and `limits`), and
  `OpenArchive` holds only `file` — there is no `closing` state, which is what
  tests 3, 4, and 6 are asking for. Still not authorized: the `Storage`-backed
  index, the commands, the frontend, `commit`, `push`, CI, Release, and tags.

## 2026-08-17 v0.3.0 Queue Lifecycle Patch: A Finished Session Is Forgotten

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`, and nothing was committed, pushed,
  tagged, or released. No CI, release, or `.env` file was touched. The working tree
  is still the same ten paths, and the only source file this patch changed is
  `apps/desktop/src-tauri/src/archive.rs`, which is untracked, so `git diff` still
  shows none of it.
- Scope held. Untouched, as instructed: `ArchiveWriter`'s `enqueue`, `pump`,
  `close`, `close_all`, `read`, and `delete`; the `Storage`-backed `ArchiveIndex`;
  the commands; the frontend; CI; the release workflow; tags; `.env`. Six
  `todo!("step 4b: ...")` bodies remain, all in `ArchiveWriter`, exactly the six
  that remained before this patch. `lib.rs` is unchanged, so `pub mod archive;` is
  still the module's only reference in the crate.
- **What the review found.** `ArchiveQueue::sessions` kept a `SessionQueue` for
  every session the process had ever seen, and `drain` iterated all of them on
  every pump. Memory and pump cost therefore grew with the number of *historical*
  sessions, not with the number of open ones — which contradicts the bounded-queue
  promise the type is named for. The finding is correct: the queue section below
  states the old behavior as intentional ("is never removed", at its
  `The nine bodies are filled in` bullet), and that sentence is superseded here.
- **The fix.** `ArchiveQueue::finish_session(&mut self, session_id) ->
  AppResult<FinishedSession>` (`archive.rs:1307`) is now the only thing that
  removes an entry. It hands back both halves of the drop history in one value —
  `FinishedSession { residual_gap: Option<DropCounters>, dropped: DropCounters }`
  (`archive.rs:234`) — and then drops the entry. `drain` needed no change: with
  finished sessions gone from the map, its loop is O(open sessions).
- Design points worth keeping:
  - **One value, not two calls.** The residual gap and the cumulative totals leave
    together because a caller that got one and missed the other would either write
    a gap line twice or file a row with counters that no longer match the file.
  - **`AppResult`, not `Option`.** Not because of `#[must_use]` — `Option` carries
    it too. Because there are three outcomes, not two, and only a `Result` keeps
    them apart: a residual gap to write, *nothing owed* (an unknown or
    already-finished session, `Ok(FinishedSession::default())`), and *not now*
    (records still queued, `Err`). An `Option` would have to fold "nothing to hand
    over" together with "you may not finish yet", and a close that read the second
    as the first would file the row while bytes were still owed.
  - **Refused while records are queued** (`session.records > 0`), because those
    records' bytes and gaps are still owed. The check happens before anything is
    taken, so a refusal is a no-op and "pump, then retry" loses nothing. The
    refusal is per session, so one session finishing while others still have
    records queued is fine.
  - **Already finished, or never seen, is `FinishedSession::default()`** — owed
    nothing, no losses. That is what makes writing the same gap twice impossible,
    and it matches the existing "taken once" idiom of `take_pending_gap`.
  - **Deviation from the instruction, deliberate.** The request said 私有
    (private). A private or `pub(crate)` method with no non-test caller is
    `dead_code` under `-D warnings`, and the only ways around that are an
    `#[allow]` attribute — a bypass marker this repo forbids — or a production
    caller, which is the writer and out of scope this round. So `finish_session`
    is `pub`, like every other `ArchiveQueue` method. What is true today is that it
    has **no runtime caller**: `lib.rs` declares `pub mod archive;` and nothing else
    in the crate calls into the module, so every call site is a test. That is a
    statement about callers, not about the API surface — a `pub` method of a `pub`
    module is reachable by definition, and this one must be narrowed to
    module-private the moment the writer becomes its production caller, which is the
    next slice.
- Six tests were added, written and confirmed red first — all six stopped at the
  `finish_session` `todo!` (`archive.rs:1298` at the time) with zero assertion
  lines — then made green by the body alone:
  1. `finishing_a_session_hands_over_its_residual_and_totals_and_forgets_it` —
     the whole contract in one pass, including that the entry is gone from
     `sessions` and that a second read is owed nothing.
  2. `a_session_with_queued_records_cannot_be_finished` — the refusal, plus proof
     it took nothing: entry, queued record, and `dropped` all intact, and the same
     values still available after a drain.
  3. `finishing_one_session_leaves_the_others_untouched` — multi-session
     isolation. B's drop is enqueued *after* B's record on purpose, so B holds a
     queued record and a residual at the same time while A finishes.
  4. `a_session_can_only_be_finished_once` — the second call and a never-seen
     session both yield `FinishedSession::default()`.
  5. `finishing_sessions_keeps_the_queue_from_accumulating_them` — the regression
     itself: fifty sessions at once, then fifty in sequence, asserting the map
     holds only what is open and is empty after the batch.
  6. `a_record_after_a_finish_starts_a_fresh_entry` — the cost of forgetting,
     written down as a test: a late record gets a fresh entry with no history and
     carries no gap, which is why the writer must finish a session only after its
     capture threads are done with it.
- Tests observe the removal by reading the private `queue.sessions` directly,
  which a child test module may do, so no accessor was added just for testing.
- Four doc comments that stated the old behavior were corrected in the same patch:
  the `sessions` field, `SessionQueue::pending` and `::dropped`, the `ArchiveQueue`
  type doc, and the module doc. `take_pending_gap` now says explicitly that it is
  not the close path, and `ArchiveWriter::close`'s doc now says close must take
  both halves from `finish_session` — that is the instruction for the next slice,
  and it is in the code rather than only here.
- Verification in `apps/desktop/src-tauri`: `cargo fmt --all -- --check` exits 0
  (one rustfmt diff in a new test was applied, then re-checked clean);
  `cargo clippy --all-targets --all-features -- -D warnings` is clean;
  `cargo test --lib` reports **`167 passed; 15 failed; 1 ignored`** out of 183,
  identical in parallel, single-threaded, and under `--all-targets --all-features`.
  All 15 failures are the same writer `todo!` panics as before — `enqueue` ×10,
  `delete` ×4, `read` ×1 — and the run contains **zero** `assertion` / `left ==` /
  `left !=` lines. Focused: the 17 queue tests by `--exact` are
  `17 passed; 0 failed; 166 filtered out`. The archive suite is **63** tests, 48
  green / 15 red, up from 57 with 42 green.
- Next step: the writer slice, unchanged in scope from the section below, and still
  needing the user's approval before it begins. Two things it must honor that this
  patch created: `close` takes the residual and the row's drop counters from
  `finish_session` (never from `take_pending_gap` plus `dropped`), and it finishes a
  session only after that session's capture threads can no longer enqueue. The one
  thing still pinned but not demonstrated remains the gap's placement on disk;
  the evidence to look for is
  `gap_records_sum_to_the_dropped_counters_of_a_closed_archive` going green with its
  assertions untouched.

## 2026-08-17 v0.3.0 Step 4b Queue Slice: `ArchiveQueue` Is Implemented

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`, and nothing was committed, pushed,
  tagged, or released. No CI, release, or `.env` file was touched. The working tree
  is still the same ten paths. Everything below is inside
  `apps/desktop/src-tauri/src/archive.rs`, which is still untracked, so `git diff`
  shows none of it; the anchors in this section are how to find it.
- Scope held. Untouched, as instructed: `ArchiveWriter`'s `enqueue`, `pump`,
  `close`, `close_all`, `read`, and `delete`; the `Storage`-backed `ArchiveIndex`;
  the commands; the frontend; CI; the release workflow; tags; `.env`. `lib.rs` is
  unchanged, so `pub mod archive;` is still the module's only reference in the
  crate and the feature is still unreachable from the application.
- The section below is now history. Its "Next step" recorded that the queue slice
  was "authorized in contract but not in code" and that fifteen `todo!` bodies
  remained in two groups; both statements were true on the morning of 2026-08-17 and
  are superseded here. The same applies to the "Starting Points for the Next Session"
  subsection further down, which still lists the queue as one of its two groups: this
  section's "Next Step" is the current one. Line numbers in the dated sections below
  were accurate when written and were not retrofitted.
- Verification in `apps/desktop/src-tauri`: `cargo fmt --all -- --check` exits 0;
  `cargo clippy --all-targets --all-features -- -D warnings` is clean;
  `cargo test --lib` reports **`161 passed; 15 failed; 1 ignored`** out of 177,
  identical in parallel, single-threaded, and under
  `--all-targets --all-features`. All 15 failures are `todo!` panics —
  `ArchiveWriter::enqueue` ×10, `read` ×1, `delete` ×4 — and the run contains
  **zero** `assertion` / `left ==` / `left !=` lines. The archive suite is 57
  tests, 42 green / 15 red, up from 52 with 31 green.
- The nine bodies are filled in: `Default`, `new`, `enqueue`, `len`, `is_empty`,
  `queued_bytes`, `drain`, `take_pending_gap`, `dropped`. `ArchiveQueue` gained
  its four fields (`bounds`, `queued`, `bytes`, `sessions`) and a private
  `SessionQueue` per session holding `records`, `bytes`, `pending`, and `dropped`.
  A drain zeroes the two queued counts and leaves `pending` and `dropped` alone,
  which is the whole reason a session entry is created on a drop as well as on an
  accept and is never removed: the row's counters are made of that history, and it
  has to answer after the last record.

### The Gap Carrier: Why a Wrapper, Not a Field on `ArchiveRecord`

- `V0.3.0_PLAN.md:337-341` left the choice open between a
  `gap_before: Option<DropCounters>` field on `ArchiveRecord` and a wrapper that
  `drain` returns. The wrapper won: `pub struct QueuedRecord { record, gap_before }`
  at `archive.rs:219`, and `drain` now returns `Vec<QueuedRecord>`. That plan
  paragraph has since been rewritten to record the resolution, so the choice and its
  reasons now read at `V0.3.0_PLAN.md:335-357`.
- Two reasons, both about what the type makes impossible rather than what a comment
  asks for. First, the gap is a property of the hand-off, not of the line the child
  process wrote — a field the capture side never sets is a field that lies, and with
  the wrapper the capture side has no field to set. Second, `encode_record` writes
  exactly three keys; a `gap_before` sitting on `ArchiveRecord` would be handed to it
  and silently dropped, whereas a caller holding a `QueuedRecord` has to reach
  through `.record` to call it, which puts `gap_before` in front of whoever wrote
  that line.
- The cost the plan predicted was paid and is the whole of it: two test call sites
  gained one level of nesting (`item.line` → `item.record.line`). The values those
  two sites assert are unchanged.
- `DropCounters` gained one private helper, `record_loss` (`archive.rs:189`), so a
  drop's two counters — the pending gap and the session's running total — are
  incremented by one definition instead of four open-coded adds. It saturates and
  uses `i64::try_from(..).unwrap_or(i64::MAX)`, matching `storage.rs`-facing
  accounting already in this file at `:858`, `:922`, and `:1026`.

### The One Rewritten Assertion, and Why

- `every_short_enqueue_sequence_keeps_the_queues_invariants` (`archive.rs:3553`)
  had required, at what was `archive.rs:3369`, that the post-drain
  `take_pending_gap` hand back everything the session lost across the whole
  sequence. Under the contract approved on 2026-08-17 that is false by design:
  a loss that a later record picked up leaves with that record, so the residual is
  only the trailing run. The old assertion described the merged-gap alternative the
  user rejected, so leaving it would have pinned the losing design.
- What replaced it, at `archive.rs:3699` onward, is strictly stronger. The model now
  carries a positional `carried: Vec<Option<DropCounters>>` alongside `kept`, and the
  test compares the entire drained sequence of `gap_before` values against it in one
  `assert_eq!`. Placement is therefore checked, not just arithmetic: a queue that
  merged two runs, moved one to a later record, or handed the same run to two
  records disagrees even though its per-session totals would still add up. Five
  further assertions per session pin the residual (it equals the model's trailing
  run), that an empty gap is `None` and never `Some(zero)`, that the carried gaps
  plus the residual equal what the session lost, that no second gap is owed, and
  that `dropped` still remembers everything.
- No other existing assertion changed. The two drain call sites re-nested by one
  level assert the same values as before, and the `arrivals`/`kept` comparison
  dropped its `String` round-trip only because reading the gaps requires borrowing
  the drained vector instead of consuming it.

### Evidence That the New Assertions Have Teeth

Two deliberate mutations were compiled and run, then reverted; the tree now holds
the correct body, and `cargo fmt`/`clippy`/`cargo test --lib` were re-run after the
revert to produce the numbers above.

1. **Carry nothing** (`let gap_before = None;`) — the merged-gap behavior the old
   assertion described. Caught by `two_drop_runs_...`,
   `a_contiguous_run_of_drops_...`, `a_pending_gap_survives_a_drain_...`,
   `a_gap_attaches_only_to_a_record_of_its_own_session`, and the exhaustive test.
   `a_trailing_drop_...` correctly stayed green: it has no accepted record after the
   drop, so it pins the residual half and cannot see this mutation. That split is
   the intended partition, not a gap in coverage.
2. **Carry without clearing** (double-report) — caught by the same four plus the
   exhaustive test, which failed specifically on "the residual is the trailing run,
   and nothing else". The two older bound tests stayed green because neither reads a
   carried gap; the new tests are what cover it.

The exhaustive test's failure trace under mutation 1 was
`[A:abcd,A:abcd,A:empty,A:empty,A:empty,A:empty]` — accept, refuse, accept — which
confirms the 4096-sequence sweep really does reach drop-then-accept for one session
and that the positional assertion is not vacuous.

### The Five New Tests

All five are at `archive.rs:3363-3552`, before the exhaustive test, and share
`gap_bounds()` (`:3370`), where only the session byte bound ever refuses anything so
that which record is lost is decided by its own length and nothing else.

1. `two_drop_runs_become_two_carried_gaps_on_two_different_records` — the
   `drop → accept → drop → accept` case the user asked for first: two runs, two
   different records, nothing merged, nothing owed at close.
2. `a_contiguous_run_of_drops_becomes_one_gap_on_the_next_accepted_record` — two
   losses with nothing kept between them are one gap, and the file gains one line
   however long the run was.
3. `a_trailing_drop_has_no_record_to_carry_it_and_stays_the_residual` — the half
   `take_pending_gap` still owns, taken once.
4. `a_pending_gap_survives_a_drain_and_lands_on_the_next_accepted_record` — the pump
   boundary: a loss between two drains is carried across and lands on the first
   record of the next batch, which is exactly where it happened.
5. `a_gap_attaches_only_to_a_record_of_its_own_session` — another session's record is
   not a place this loss could be reported, because sessions are separate files.

They were written and confirmed red before any body was filled in: `26 failed`, all
26 at a `todo!`, with zero assertion lines, the five new ones stopping at
`ArchiveQueue::new`.

### Next Step

The queue is done and the slice stops here for review, as instructed. The remaining
`todo!` bodies are the writer's own — `enqueue`, `pump`, `close`, `close_all`,
`read`, `delete` — plus the quota's eviction, the `Storage`-backed `ArchiveIndex`,
the commands, and the frontend. `ArchiveWriter::enqueue` is now unblocked, since the
queue it needs exists. The next slice needs the user's approval before it begins.

Four documents were updated for this slice and nothing else was: this section,
`notes.md`'s matching section, `CLAUDE.md`'s Current Baseline (suite 57, the current
numbers, the queue removed from "not started", six `todo!` bodies rather than
fifteen), and `V0.3.0_PLAN.md` — its `:335-357` now records the carrier decision as
resolved instead of open, and the [Execution Order](#execution-order) gained a
`4b, queue slice` entry with the measured numbers. The one thing this slice pinned
but could not demonstrate is the gap's placement on disk, because `pump` is still
`todo!`; the evidence to look for in the next slice is
`gap_records_sum_to_the_dropped_counters_of_a_closed_archive` going green with its
assertions untouched.

## 2026-08-17 v0.3.0 Step 4b Unblocked: The Drop-Counter Constraint Is Corrected

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets
  `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`, and nothing was committed, pushed,
  tagged, or released. No CI, release, or `.env` file was touched. The working tree
  is still the same ten paths. Production code **did** change: the version 2 DDL
  inside `upgrade_to_version_2` (`storage.rs:715`) is not the text it was. What this
  round did not do is add a new function body, a new run path, or any part of the
  queue implementation, and it started no new slice.
- The pause recorded in the section below is resolved, and that section is now
  history rather than current state. Its "Next step" asked for approval of the
  correction, and the user granted it on 2026-08-17 as
  **「就地改 v2 DDL，不加 v3」** — correct the version 2 DDL in place, keep the schema
  version at 2, add no version 3 migration. Both copies of the version stayed at 2:
  the production `SCHEMA_VERSION` (`storage.rs:621`) and the deliberately separate
  test literal `CURRENT_SCHEMA_VERSION` (`storage.rs:773`), which exists so bumping
  the production constant cannot make the migration tests pass on its own.
- The constraint went from `CHECK ((dropped_lines = 0) = (dropped_bytes = 0))` to
  `CHECK (dropped_bytes = 0 OR dropped_lines > 0)`. The old form claimed lines and
  bytes are lost together; they are not, because `capture_stream` turns a lone
  newline into a real event whose `line` is empty, so dropping one such record costs
  `1 line / 0 bytes`. Only the reverse is impossible — every archived byte belongs
  to some line — and that is the direction the new form states, saying nothing at
  all about the other.

- The correction landed in all five places in one round, so no site is left behind:
  1. `V0.3.0_PLAN.md:634`, the design DDL;
  2. `V0.3.0_PLAN.md:844-846`, the prose invariant, now "`dropped_bytes > 0` implies
     `dropped_lines > 0`, with no converse — a dropped empty line is `1 line /
     0 bytes` and is ordinary data". The sub-bullet that had declared the old rule
     wrong was deleted, because the rule it accused is gone;
  3. `storage.rs:715`, the production migration inside `upgrade_to_version_2`;
  4. `storage.rs:1240`, the pinned `V2_ADDITION` copy the tests build a database
     from;
  5. the `"drop counters agree"` case in
     `the_archive_index_rejects_impossible_rows` (`storage.rs:1507`), whose
     `5 lines / 0 bytes` row left the rejection list. A comment at
     `storage.rs:1553` records what used to sit there and why that row is ordinary
     data, and names the two tests that now hold both directions:
     `an_archive_row_may_lose_a_line_that_carried_no_bytes` (`storage.rs:1602`) and
     `an_archive_row_may_not_lose_bytes_without_losing_a_line` (`storage.rs:1648`).
     Each runs against the migrated database **and** against the pinned schema via
     `create_pinned_version_2_database`, so a half-applied future edit fails.
- Removing that case is the only existing assertion this round touched. It is the
  contract change the pause below asked for by name, argued there before it was
  approved — not a step taken to reach green. Nothing was weakened, retargeted, or
  `#[ignore]`d, and no production code outside the two DDL copies changed.
- Both DDL copies now carry a four-line SQL comment above the constraint explaining
  why the rule is one-directional, so the next reader does not have to re-derive the
  defect from first principles. That comment is in English, unlike the Chinese draft
  shown when the question was asked: every other line of code and document in this
  repository is English, and the substance the user approved — the expression and the
  five sites — is unchanged.

- Verification, in `apps/desktop/src-tauri`:
  - `cargo fmt --all -- --check` — clean, exit 0.
  - `cargo clippy --all-targets --all-features -- -D warnings` — clean, zero
    warnings, no `allow` added.
  - `cargo test --lib` — `150 passed; 21 failed; 1 ignored; 0 measured; 0 filtered
    out` out of 172, identical in parallel and with `--test-threads=1`.
  - The delta is exactly the one predicted before the edit, and nothing else: the
    previous round's `149 passed; 22 failed; 1 ignored` minus its single storage
    failure. `an_archive_row_may_lose_a_line_that_carried_no_bytes` had been failing
    on `CHECK constraint failed: (dropped_lines = 0) = (dropped_bytes = 0)`; it
    passes now. No other test changed state, and no failing test moved where it
    stops.
  - All 21 remaining failures were classified by panic site rather than by name:
    `archive.rs:1098` ×6 (`ArchiveQueue::new`), `archive.rs:1355` ×10
    (`ArchiveWriter::enqueue`), `archive.rs:1410` ×1 (`read`), `archive.rs:1420` ×4
    (`delete`). Every one is a `todo!("step 4b: ...")`. The suite output contains
    **zero** lines matching `assertion`, `left ==`, or `left !=`, so nothing fails on
    a claim about behavior — the schema is no longer among the reasons anything is
    red.
  - The five storage tests that touch the changed DDL also pass individually under
    `--exact`, so none of them is riding on another test's side effect.
  - Counts by module are unchanged at archive 52, storage 24, processes 12, total
    172: a parametrized **case** was removed, not a test. Four archive tests are
    `#[cfg(windows)]`, so a non-Windows host runs 168.
- The constraint was proved safe against the whole crate before the edit, not just
  against the one failing test. All four `INSERT INTO run_log_archives` sites and
  every parametrized row they carry were enumerated, and none of them writes bytes
  without a line, so no existing case moved from accepted to rejected. No test reads
  DDL text out of `sqlite_master` — the only query against it is
  `SELECT 1 FROM sqlite_master WHERE type=? AND name=?` — so adding SQL comments
  inside a table definition cannot break a comparison.

- **The developer-database census was re-taken on 2026-08-17 and still holds.** Read
  from the SQLite header, bytes 60..63 of page 1, without opening a connection:
  `%LOCALAPPDATA%\com.abysswhale.runcove\runcove.sqlite3` — `user_version = 1`;
  `...com.abysswhale.runcove.qa\runcove.sqlite3` — `user_version = 1`. Neither
  directory holds a `-wal` or `-shm` file, so the header is authoritative rather than
  a stale page. The scoped claim, and the only one the evidence supports: **the two
  verified developer databases on this machine and the published `v0.2.1` baseline
  hold no version 2 database.** That is what makes the in-place correction complete
  here — a fresh migration now creates the corrected table, and neither verified
  artifact needs a version bump to signal against. It is not a claim about every
  machine; any copy of this working tree run elsewhere is outside what was measured,
  which is what the next bullet is for.
- **The rebuild recipe stays on file.** If a version 2 database is ever found — a
  copy of this working tree run on another machine — editing the DDL cannot repair
  it, because `migrate` runs `upgrade_to_version_2` only when `version <= 1`
  (`storage.rs:672`) and SQLite keeps a `CHECK` inside the table definition with no
  `ALTER TABLE ... DROP CONSTRAINT`. The repair is the `upgrade_to_version_3` table
  rebuild spelled out in the section below, including its two traps
  (`PRAGMA foreign_keys` is a no-op inside a transaction; do not sniff the old
  `CHECK` text out of `sqlite_master`). That section is history for the constraint
  but current for the recipe.

- **The gap partition contract is decided and is not implemented.** The user chose
  **「gap 挂到下一条记录上」** — the pause below calls it option 2, and it was not the
  recommendation there. When a record is refused its loss becomes the session's
  pending gap; when the next record for that session is accepted, the queue attaches
  the pending counters to that record and clears them. `take_pending_gap` therefore
  returns only the trailing residual — a loss with no accepted record behind it —
  which the writer writes at close. Placement in the file becomes exact, so
  `V0.3.0_PLAN.md:314-315` becomes the literal contract instead of an approximation
  of it. Recorded at `V0.3.0_PLAN.md:322-349`.
- That decision is the queue slice's **first** move rather than something the slice
  discovers, and it needs three things:
  1. a carrier for the counters on a queued record — either a
     `gap_before: Option<DropCounters>` field on `ArchiveRecord` that the capture
     side never sets, or a `QueuedRecord` wrapper that `drain` returns instead, which
     costs every `item.session_id` and `item.line` in the tests one level of nesting;
  2. a `take_pending_gap` that returns the residual only;
  3. a rewritten assertion at `archive.rs:3369`, where
     `every_short_enqueue_sequence_keeps_the_queues_invariants` currently requires
     the post-drain `take_pending_gap` to hand back everything the session lost
     across the whole sequence. Once carried gaps leave with the drained records that
     is no longer true, and the test must instead require that the carried gaps plus
     the residual equal the session's cumulative `dropped`, and that each carried gap
     sits on the first accepted record after its own drop run. The user's approval is
     on record for this change; it is still a contract change and must be argued as
     one, never as a way to reach green.
- **A second, smaller pass repaired stale line-number citations in
  `V0.3.0_PLAN.md`, and it is a separate change from the constraint.** Adding four
  comment lines to each DDL copy shifted every anchor past them, and some citations
  were already stale from the blocker round. Re-measured and corrected: `new_id`
  (`:741-742`), `recover_unfinished_sessions` (`:725`), the four settings and
  migration version tests (`:786`, `:795`, `:813`, `:830`), the test literal
  `CURRENT_SCHEMA_VERSION` (`:773`), `run_history_is_newest_first_and_honors_the_limit`
  (`:988`), `reopening_storage_marks_unfinished_sessions_interrupted` (`:950`),
  `V1_SCHEMA` / `V1_FIXTURE` / `V2_ADDITION` (`:1149`, `:1191`, `:1216`),
  `the_archive_index_rejects_impossible_rows` (`:1507`, and its case count is now
  **eight**, not nine), and `run_history_reports_the_archive_summary_when_one_exists`
  (`:1694`, which had drifted 128 lines). The migration-test list was rewritten to
  name all nine tests instead of listing bare offsets, and the note above it now says
  the names are authoritative and the numbers rot.
- Two factual errors in the plan were corrected while re-measuring, both found by
  this round's own evidence: the data-directory bullet had claimed `runcove.sqlite3`
  always sits beside `-wal` and `-shm` companions, which the census disproves —
  `Storage::open` never sets `journal_mode`, so the default rollback journal is
  transient and neither developer directory holds either file. The bullet's argument
  does not depend on the sidecars and is unchanged; only the false detail went.
- Line numbers in the dated sections **below** were accurate when written and were
  not retrofitted. Read them as of their own date. The current values for the anchors
  those sections use: the four `INSERT INTO run_log_archives` statements are at
  `storage.rs:1403`, `:1519`, `:1581`, and `:1704`, all still below `#[cfg(test)]`,
  which is now at `storage.rs:745`.
- Next step: the user asked to stop here for review — **「先停下让我 review」** — so the
  queue slice is authorized in contract but not in code. Nothing else about step 4b
  moved: fifteen `todo!("step 4b: ...")` bodies remain in the two independent groups
  listed under the third slice, and `pub mod archive;` in `lib.rs` is still the
  module's only reference in the crate, so the archive is still unreachable from the
  application.

## 2026-08-16 v0.3.0 Step 4b Paused: The Version 2 Drop-Counter Contract Is Wrong

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets `5e3e0d4`, and nothing was
  committed, pushed, tagged, or released. No CI, release, or `.env` file was
  touched. No production DDL, no `CURRENT_SCHEMA_VERSION`, and no queue or writer
  function body was changed. This session added tests and documentation only, and
  it stops here to wait for a separate approval of the schema correction.
- The working tree is now **ten** paths, not nine: the nine listed under the third
  slice plus modified `apps/desktop/src-tauri/src/processes.rs`, which gained one
  control test.
- The blocker: the version 2 `run_log_archives` table ends with
  `CHECK ((dropped_lines = 0) = (dropped_bytes = 0))`, which asserts that a session
  loses lines exactly when it loses bytes. That is false. `capture_stream` turns a
  lone newline into a real log event whose `line` is empty, so dropping one such
  record costs `dropped_lines = 1` and `dropped_bytes = 0` — a row the schema
  refuses. The archive would be unable to record a loss it is designed to record,
  and `close` would fail on data the queue is supposed to produce.
- The same mistake makes the existing rejection case `5 lines / 0 bytes` in
  `the_archive_index_rejects_impossible_rows` wrong about what is impossible: five
  dropped empty lines are ordinary data.
- Only one direction is genuinely impossible: bytes cannot be lost without a line,
  because every byte of archived text belongs to some line. That direction is now
  pinned by its own test.
- The defect sits in three places, all still unchanged:
  1. `V0.3.0_PLAN.md:598`, the design, with the prose invariant restated at
     `V0.3.0_PLAN.md:772-773` ("`dropped_lines` is zero exactly when
     `dropped_bytes` is zero");
  2. `apps/desktop/src-tauri/src/storage.rs:711`, the production migration;
  3. `apps/desktop/src-tauri/src/storage.rs:1232`, the `V2_ADDITION` copy the tests
     pin so that drift between the plan and production fails a test.
- Nothing compares those three texts to each other, and the two existing
  `V2_ADDITION` users only check version handling, so a correction applied to one
  site and not the others would not have been caught. Both new schema tests now run
  against the migrated database **and** against a database built from
  `V2_ADDITION`, via the new `create_pinned_version_2_database` helper
  (`storage.rs:1583`), so a half-applied fix fails.
- Seven tests were added, and every one of them states a requirement rather than a
  current behavior. Item numbers follow the request that authorized this round:
  1. `processes.rs:1104` `a_lone_newline_is_captured_as_one_empty_log_line` —
     **green**, as expected. Input `"\n"` yields exactly one event whose `line` is
     `""`, and `"a\n\nb\n"` yields `["a", "", "b"]`, so the empty line is not an
     artifact of a stream that holds nothing else. This is the control test the
     whole blocker rests on.
  2. `storage.rs:1592` `an_archive_row_may_lose_a_line_that_carried_no_bytes` —
     **red, on the CHECK constraint**, not on an assertion. The panic message is
     `CHECK constraint failed: (dropped_lines = 0) = (dropped_bytes = 0)` at
     `storage.rs:1620`. It asserts that both `1 line / 0 bytes` and
     `5 lines / 0 bytes` are accepted.
  3. `storage.rs:1638` `an_archive_row_may_not_lose_bytes_without_losing_a_line` —
     **green**. It refuses `0 lines / 40 bytes`, and also a negative count in
     either column. It is green under the present constraint and must stay green
     under the corrected one, which is exactly its purpose: it stops the fix from
     dropping the relationship between the two counters altogether.
  4. `archive.rs:3153` `dropping_an_empty_line_counts_one_line_and_no_bytes` —
     **red at `todo!`**, `archive.rs:1098` (`ArchiveQueue::new`). It requires
     `DropCounters { lines: 1, bytes: 0 }` and the matching gap text
     `[RunCove: dropped 1 line / 0 bytes]`. The refusal has to come from the record
     bound: an empty record can never exhaust a byte bound.
  5. `archive.rs:3183` `the_queue_counts_utf8_bytes_and_not_characters` — **red at
     the same `todo!`**. Written with `\u{...}` escapes so the counts do not depend
     on this file's encoding, and it asserts its own premise first
     (`("\u{4e2d}".len(), "\u{1f680}".len(), "e\u{301}".len()) == (3, 4, 3)` against
     character counts `(1, 1, 2)`). A character-counting queue fails it twice: on
     `queued_bytes()` and on accepting a record that must be refused.
  6. `archive.rs:3232` `every_short_enqueue_sequence_keeps_the_queues_invariants`
     and `archive.rs:3388`
     `a_pending_gap_is_taken_once_and_the_cumulative_total_is_never_cleared` —
     both **red at the same `todo!`**. No new dependency was added; the first is a
     deterministic exhaustive test over all `4^6 = 4096` sequences of six records
     drawn from a four-record alphabet, each step checked against a model the test
     keeps itself.
- The exhaustive test's alphabet is one empty line, one four-byte line on the same
  session, a three-byte multi-byte character on a second session, and a four-byte
  line on a third, against bounds `{session_records: 2, session_bytes: 4,
  total_records: 3, total_bytes: 8}`. Each of the four bounds is met exactly by some
  sequence and exceeded by another, so neither the accepting nor the refusing branch
  is vacuous. It pins all five requested invariants:
  1. after an accepted record, no per-session or total record or byte bound is
     exceeded;
  2. reaching a bound exactly is allowed, passing it refuses only the incoming
     record — checked by `queue.len() == kept.len()` at every step, so nothing
     already queued can be evicted to make room;
  3. `drain` hands over every kept record in global arrival order and leaves
     `len`, `queued_bytes`, and `is_empty` at zero, zero, and true;
  4. a pending gap is taken once and returns `None` afterwards, while the
     cumulative `dropped` total survives both the take and the drain;
  5. `dropped` equals the line count and the summed UTF-8 text length of exactly
     the refused records.
- One invariant was deliberately left unpinned. The plan says one `system` gap
  record per *contiguous* gap but never settles how a drop → accept → drop run
  partitions when no take intervenes, and `take_pending_gap` returns a single
  `Option<DropCounters>` per session, so there is nowhere to keep two. The tests
  therefore assert only that everything lost since the last take comes back in one
  gap and that the gaps sum to the cumulative total, which is what
  `V0.3.0_PLAN.md:319` already requires. Do not read them as approving a particular
  partition.
- **The contradiction to resolve, reported and not papered over.** As instructed, no
  existing assertion was deleted, weakened, or retargeted. The consequence is that
  `storage.rs` now holds two tests that disagree about the same data:
  `the_archive_index_rejects_impossible_rows` requires
  `('sess-exited','a.jsonl','complete',NULL,0,0,5,0,10,11)` to be **rejected**, and
  `an_archive_row_may_lose_a_line_that_carried_no_bytes` requires `5 lines /
  0 bytes` to be **accepted**. Measured today: the old test passes, the new one
  fails. No single sane constraint can satisfy both, so correcting the schema must
  also remove the `"drop counters agree"` case from the old test's rejection list —
  a change to an existing assertion, which is why it waits for approval rather than
  being folded into this round.
- Verification, in `apps/desktop/src-tauri`:
  - `cargo fmt --all -- --check` — clean. `cargo fmt --all` was run once after
    writing the tests; it reflowed one `assert_eq!` in the new gap test and touched
    nothing else.
  - `cargo clippy --all-targets --all-features -- -D warnings` — clean, zero
    warnings, no `allow` added. Clippy's `int_plus_one` rejected the model's
    `count + 1 <= bound`; the fix was to write `count < bound` and extend the
    comment, not to silence the lint.
  - `cargo test --lib` — `149 passed; 22 failed; 1 ignored; 0 measured; 0 filtered
    out` out of 172, identical with `--test-threads=1` and under
    `--all-targets --all-features`.
  - Before this round the same command gave `147 passed; 17 failed; 1 ignored` out
    of 165. The arithmetic accounts for every change: seven tests added, two green
    (items 1 and 3), five red (items 2, 4, 5, and both of item 6). **No previously
    passing test changed state, and no previously failing test changed where it
    stops.**
  - Every one of the 22 failures was classified by panic site, not by name:
    `archive.rs:1098` ×6 (`ArchiveQueue::new`), `archive.rs:1355` ×10
    (`ArchiveWriter::enqueue`), `archive.rs:1410` ×1 (`read`), `archive.rs:1420` ×4
    (`delete`), and `storage.rs:1620` ×1 (the CHECK). The suite output contains
    **zero** lines matching `assertion`, `left ==`, or `left !=`, so nothing fails
    on a claim about behavior — the 21 archive failures stop at an unwritten body
    and the one storage failure stops at the constraint under discussion.
  - Counts by module are now archive 52, storage 24, processes 12, total 172. Four
    archive tests are `#[cfg(windows)]` and no new test is platform-gated, so a
    non-Windows host runs 168.
- Next step, and the only thing this round asks for: approve or redirect the schema
  correction. Nothing else is blocked on it — the queue's own behavior is already
  settled by its tests — but writing the queue first would produce a `close` row the
  database refuses, so the order matters. When the correction is authorized it needs
  all five of these, or it will drift:
  1. `V0.3.0_PLAN.md:598` and the prose at `V0.3.0_PLAN.md:772-773`;
  2. the production DDL at `storage.rs:711`;
  3. the pinned `V2_ADDITION` copy at `storage.rs:1232`;
  4. the `"drop counters agree"` case in `the_archive_index_rejects_impossible_rows`,
     which must lose the `5 lines / 0 bytes` row;
  5. a decision about `CURRENT_SCHEMA_VERSION`. v0.3.0 is unreleased, so no user
     database is at version 2 and no version 3 migration is needed for correctness.
     But SQLite keeps a `CHECK` inside the table definition, so a **developer**
     database already migrated to version 2 by an earlier build will keep the wrong
     constraint no matter what the DDL says. Either that database is discarded by
     hand or the table is rebuilt; editing the DDL alone is not enough. This is a
     schema decision and is left entirely to the user.

### The Three Questions The Correction Has To Answer First

Asked on 2026-08-16, after the pause was accepted. Nothing here changes code. The
first two questions were settled by measurement; the third is an open contract
question and is answered with a recommendation only.

- **Is any verified database at version 2? No, and nothing published can be.** The committed
  baseline `97943d7` never mentions `run_log_archives`, and its migration tests assert
  `schema_version == 1`, so no v0.2.1 install can hold a version 2 database. The
  version 2 code is uncommitted and unpushed, so it exists only in this working tree.
  `apps/desktop/src-tauri` is also the only crate that depends on `rusqlite`, so the
  compatibility CLI cannot migrate anything either.
- **Both developer databases on this machine are at version 1.** Read from the SQLite
  header (bytes 60..63 of page 1), without opening a connection:
  `%LOCALAPPDATA%\com.abysswhale.runcove\runcove.sqlite3` — `user_version = 1`, 69632
  bytes, last written 2026-08-11; `...com.abysswhale.runcove.qa\runcove.sqlite3` —
  `user_version = 1`, 69632 bytes, last written 2026-08-08. Both predate the version 2
  code, written 2026-08-15. An ASCII scan of both files finds no `run_log_archives`
  anywhere in them, so the table has never existed in either. Neither directory holds
  a `-wal` or `-shm` file and `Storage::open` never sets `journal_mode`, so the header
  is authoritative rather than a stale page. In passing: `V0.3.0_PLAN.md:171` assumes
  those two sidecar files exist. They do not.
- **So item 5's recommendation is to keep version 2**, correct the DDL in place, and
  add no version 3 step. A schema version is a compatibility signal, and there is no
  incompatible artifact to signal against. This rests entirely on the census above; if
  the working tree was ever copied to another machine and run there, the next bullet
  applies instead.
- **If a version 2 database is ever found**, editing the DDL cannot repair it:
  `migrate` runs `upgrade_to_version_2` only when `version <= 1` (`storage.rs:672`),
  and SQLite keeps a `CHECK` inside the table definition with no
  `ALTER TABLE ... DROP CONSTRAINT`. The repair is the documented rebuild — create the
  corrected table under a temporary name, copy, drop the old one, rename, recreate
  `idx_run_log_archives_status_ended` — as `upgrade_to_version_3`, with the version 2
  DDL corrected as well so a fresh install never creates the wrong table. Two traps:
  `PRAGMA foreign_keys` is a no-op inside a transaction, so it has to be turned off
  before `BEGIN` and back on after `COMMIT` (`Storage::open` turns it on at
  `storage.rs:19`); and the rebuild should be unconditional rather than sniffing the
  old `CHECK` text out of `sqlite_master`, because it is idempotent and costs nothing
  on an empty table. Empty is the real case: all four `INSERT INTO run_log_archives`
  statements in the crate are at `storage.rs:1395`, `:1511`, `:1571`, and `:1694`,
  every one of them below `#[cfg(test)]` at `storage.rs:741`, and the archive module
  is still unreachable from production code.

- **The gap partition is a second contract conflict, not merely an unsettled detail.**
  `V0.3.0_PLAN.md:310-311` requires one gap record "per contiguous gap, written
  immediately before the next successful record for that session". The queue's API
  cannot express that: `take_pending_gap` returns one `Option<DropCounters>` per
  session (`archive.rs:1127`), so a drop, an accept, and a second drop with no take in
  between collapse into a single number. That sequence needs no drain to reach, because
  the bounds count bytes as well as records: with `session_bytes = 4`, the arrivals
  `"abcd"`, `"abcd"`, `""` accept, refuse on bytes, then accept again. It is one of the
  4096 sequences the exhaustive test enumerates, and `archive.rs:3369` requires
  `take_pending_gap` to hand back everything the session lost across the whole
  sequence. The queue's accounting is therefore already pinned to "merge until taken";
  what is genuinely unpinned is only how the file lays those gaps out.
- Three self-consistent ways out, in increasing cost:
  1. **Merge, and correct the plan's wording.** A gap record summarizes everything lost
     since the last gap record for that session, written immediately before the next
     record the writer writes for it, or at close. Counts stay exact, the sum rule at
     `V0.3.0_PLAN.md:318-320` still holds, and it costs one wording change at
     `:310-311` with no test and no API change. The cost is placement: a loss that
     happened after a surviving line is reported before it.
  2. **Carry the gap on the record.** The pending counters move onto the next accepted
     record for that session at enqueue time, and `take_pending_gap` returns only the
     trailing residual. Placement becomes exact. It costs a field on the drained item
     and, decisively, the assertion at `archive.rs:3369`: an existing test would have
     to change, which needs its own approval and must be argued as a contract change,
     never as a way to reach green.
  3. **Put gap entries in the queue.** `drain` yields records and gap markers in true
     arrival order, which fixes ordering across sessions too. It costs `drain`'s return
     type and every test that maps over it, and the markers must be exempt from the
     record and byte bounds, or a full queue could not record its own loss.
- Recommendation: option 1. A gap record exists so that a reader of the file knows
  output is missing there, and the actionable part is the counts, which are exact under
  all three options. Options 2 and 3 buy an attribution the reader cannot act on, at
  the price of API surface and a rewritten assertion.

## 2026-08-16 v0.3.0 Step 4b, Third Slice: Begin And The Open-Session Gate

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets `5e3e0d4`, and nothing was
  committed, pushed, tagged, or released. No CI or release file was touched. The
  working tree is still the same nine paths listed further below.
- Done this session, in `apps/desktop/src-tauri/src/archive.rs` only, and nothing
  beyond the authorized scope: `ArchiveWriter::begin`, `ArchiveWriter::is_open`, and
  the writer state those two need — the new private `OpenArchive` slot, the
  `open: Mutex<BTreeMap<String, OpenArchive>>` map, and the `fs` and `index` handles
  the writer now keeps instead of dropping after the sweep. `pub mod archive;` in
  `lib.rs` is still the module's only reference in the crate, so the feature remains
  unreachable from the application.
- `begin` runs in three phases so no lock is ever held across file I/O or a database
  write, which is what requirement 8 asks for:
  1. Validate and resolve, touching nothing: `archive_file_name` then
     `resolve_archive_path`. An id this build could not have generated fails here.
  2. Take the session's slot under the map lock, then release the lock. The slot is
     inserted with `file: None`, which is what makes a second concurrent `begin`
     lose immediately instead of racing for the file.
  3. Outside the lock, `create_new` the file and insert the `writing` row; on
     success re-take the lock only to store the handle, on failure only to drop the
     slot.
- Failure cleanup, in the order it happens: the file handle is dropped before
  `remove_file` is called, because on Windows this process's own open handle would
  block the delete. If the delete also fails, `begin` still returns an error, the
  slot is still released, and the message says plainly that the empty file is left
  for the next startup sweep — the orphan path that
  `an_orphan_left_by_a_failed_cleanup_is_deleted_by_the_next_sweep` already covered.
- The quota is deliberately not consulted in `begin`, and `begin`'s doc says so. A
  new archive is an empty file and adds nothing to the total; the caps and an
  `Unavailable` total are `pump`'s refusal, which is what
  `an_unavailable_total_stops_the_archive_instead_of_growing_it` requires — that test
  arranges an unavailable total and then expects `begin` to succeed. This was settled
  by reading the test, not by guessing.
- `is_open` is true from the moment the slot is taken until the archive is closed or
  the attempt fails, which deliberately includes the instant before the file exists,
  because `delete` must refuse a session that is being opened. A session the index
  knows about but this writer never opened is not open.
- Boundary held: the queue, `enqueue`, `pump`, `close`, `close_all`, `read`,
  `delete`, quota eviction, the `Storage`-backed `ArchiveIndex`, the commands, and
  all frontend work are untouched and still `todo!("step 4b: ...")`. `initialize`
  still discards `bounds` and `limits` with `let _ = (bounds, limits);`.

- Verification, in `apps/desktop/src-tauri`:
  - `cargo fmt --all -- --check` — clean.
  - `cargo clippy --all-targets --all-features -- -D warnings` — clean, zero
    warnings. No `allow` was added; `OpenArchive::file` is read by `take_slot` via
    `taken.file.is_some()`, so it is not `dead_code`, and that read is also what
    lets the two refusals differ ("already has an open archive" versus "is already
    opening its archive").
  - `cargo test --lib --all-features -- --test-threads=1` — `147 passed; 17 failed;
    1 ignored; 0 measured; 0 filtered out` out of 165, up from `140 passed;
    19 failed` out of 160 before this slice. The default parallel run and
    `--all-targets --all-features` report exactly the same three numbers, and no
    test outside `archive::tests` failed in any run.
  - The archive suite is now 48 tests: 31 green, 17 red. Five tests are new (below)
    and two that previously stopped at `begin` are green:
    `a_refused_writing_row_leaves_no_orphan_file_behind` and
    `an_orphan_left_by_a_failed_cleanup_is_deleted_by_the_next_sweep`.
  - All 18 tests that this slice could affect were run one at a time with
    `-- --exact <name>`, so nothing passes only as part of a batch and every red one
    is pinned to where it now stops. Seven are green — the five new ones plus the two
    above. Eleven advanced past `begin` to a body outside this slice: ten to
    `ArchiveWriter::enqueue` and `deleting_an_archive_is_refused_while_its_writer_is_open`
    to `delete`.
  - Every one of the 17 remaining failures is a `todo!` panic at one of four sites:
    `archive.rs:1098` `ArchiveQueue::new` ×2, `:1355` `enqueue` ×10, `:1410` `read`
    ×1, `:1420` `delete` ×4. Each line's text was read to confirm the site. A scan of
    both runs for `assertion` / `left ==` lines returns zero, so nothing is red
    because it disagrees with an implementation, and no assertion was weakened,
    retargeted, or removed to reach green.
  - `the_test_filesystem_reports_a_link_as_a_reparse_point` and the three other
    `#[cfg(windows)]` archive tests still run only on Windows; a non-Windows host
    sees 44 archive tests.
  - The root `runcove` crate was not touched and was re-verified anyway: fmt clean,
    clippy clean, `cargo test --all-targets` green (12, 0, 0, 10, 16).
  - The frontend half of the `AGENTS.md` matrix — `npm run lint`, `npm run
    typecheck`, `npm test -- --run`, `npm run build`, the Playwright E2E, and
    `cargo tauri build` — was not run, because no frontend file changed in this
    slice. The full matrix is still required, and is scheduled for plan step 7.
- Five tests added, all against requirement 7, which a `.begin(` / `.is_open(` grep
  showed no existing test covered. No existing assertion was touched:
  - `beginning_a_session_this_build_could_not_have_generated_touches_nothing` — eight
    bad ids (empty, `.`, `..`, a non-UUID, an uppercased UUID, one character short,
    one long, and an underscored one); each errors, `is_open` stays false, and
    afterwards the directory is still empty, the filesystem log has no removal, the
    index log has no call, there is no row, and the total is still `Known(0)`.
  - `beginning_a_session_whose_file_already_exists_refuses_and_keeps_the_file` — the
    file is planted after `initialize`, so the sweep cannot have seen it; `begin`
    fails on `create_new`, the planted bytes are intact, and no index call happens.
  - `beginning_the_same_session_twice_is_refused_and_keeps_the_first_archive` — the
    second call errors while the first archive stays open, its row stays `writing`
    with the first `started_at`, and the index call log does not grow.
  - `two_threads_beginning_the_same_session_leave_exactly_one_open_archive` — two
    threads share the writer through an `Arc` and meet at a `Barrier`; exactly one
    call is `Ok`, there is exactly one `insert_writing:` call, one row, one file, and
    no removal. No assertion names a winner, so the test cannot pass by luck; it was
    also repeated 20 times alone, 20 ok / 0 failed.
  - `is_open_is_false_for_a_session_this_writer_never_opened` — a `complete` archive
    seeded from an earlier run is on disk and in the index; `is_open` is false for it,
    for an unopened id, and for two invalid ids, and true only for the session this
    writer actually began.
- One regression this slice caused and fixed: a placeholder-anchored edit deleted
  `ArchiveWriter::enqueue`'s three-line doc comment. `cargo fmt --all -- --check`
  caught it (exit 1, diff at `src\archive.rs:1347`), the original text was restored
  verbatim, and fmt went back to exit 0. Because the fix shifted panic line numbers
  by two or three, the whole suite was re-run to re-record them rather than reusing
  the earlier values — the four sites above are the post-fix numbers.
- Next step: the fourth slice of 4b. Two independent groups remain, and either can be
  taken first; get the user's approval before starting, as with the first three
  slices.

### Starting Points for the Next Session

Fifteen `todo!("step 4b: ...")` bodies remain in
`apps/desktop/src-tauri/src/archive.rs`, in two independent groups:

- `ArchiveQueue`, lines 1089-1136 — nine bodies: `Default` and `new`, `enqueue`,
  `len`, `is_empty`, `queued_bytes`, `drain`, `take_pending_gap`, and `dropped`. Six
  red tests now wait here, all panicking at `archive.rs:1098` (`ArchiveQueue::new`);
  four of the six were added by the blocker round above, which also paused this
  group until the schema correction is settled.
- `ArchiveWriter`'s remaining write path and gates, lines 1353-1421 — six bodies:
  `enqueue`, `pump`, `close`, `close_all`, `read`, and `delete`. Ten of the red tests
  stop at `enqueue` (`:1355`), four at `delete` (`:1420`), and one at `read`
  (`:1410`). The writer's `enqueue` needs the queue, so taking the queue first makes
  this group's ten tests reachable in one move.

Reproduce the exact list, from `apps/desktop/src-tauri`, rather than trusting a
remembered one:

```
cargo test --lib --all-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

Both groups are still unauthorized. Read `CLAUDE.md` and `AGENTS.md` first: the
tests may use only temp directories and test databases, an assertion may not be
weakened or retargeted to reach green, and no commit, push, tag, release, CI edit,
command, or frontend work happens without the user asking for it.

## 2026-08-16 v0.3.0 Step 4b, Second Slice: Initialize And The Startup Sweep

- Superseded by the section above for the current test numbers, for what is
  implemented, and for the next session's starting points. Everything below still
  describes how the second slice was built and reviewed.

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets `5e3e0d4`, and nothing was
  committed, pushed, tagged, or released. No CI or release file was touched. The
  working tree is still the same nine paths listed further below.
- Done this session, in `apps/desktop/src-tauri/src/archive.rs` only:
  `ArchiveWriter::initialize`, the startup sweep behind it, `archive_dir`, and
  `total_bytes`. The writer now carries the two fields those bodies read —
  `archive_dir` and the measured `total` — and nothing more, because a private
  field no body reads is `dead_code` under `-D warnings`. The filesystem and index
  handles are used by the sweep and dropped, not stored, for the same reason.
  `pub mod archive;` in `lib.rs` is still the module's only reference in the crate,
  so the feature remains unreachable from the application.
- What the sweep does, in the order it does it: read the index rows, list the
  archive directory once, classify every immediate child by name first, key the
  rows by the file name each one owns, reconcile row by row, delete the eligible
  files no row remembers, then measure the byte total. New private items:
  `SweptEntry`, `Sweep`, `row_label`, and `archive_file_stem`, which
  `is_archive_file_name` now shares so the name rule and the id it yields cannot
  drift apart.
- Boundary held: the queue, the pump, the write path, close, quota eviction, the
  `Storage`-backed `ArchiveIndex`, the commands, and all frontend work are
  untouched and still `todo!("step 4b: ...")`. `initialize` accepts `bounds` and
  `limits` and discards them with `let _ = (bounds, limits);` so the pinned
  signature does not move when the bodies that read them arrive.
- Verification, in `apps/desktop/src-tauri`:
  - `cargo fmt --all -- --check` — clean.
  - `cargo clippy --all-targets --all-features -- -D warnings` — clean, zero
    warnings.
  - `cargo test --lib --all-features -- --test-threads=1` — `140 passed;
    19 failed; 1 ignored; 0 measured; 0 filtered out` out of 160, up from
    `130 passed; 27 failed` before this slice. The default parallel run reports the
    same `140 passed; 19 failed; 1 ignored`, and no test outside `archive::tests`
    failed in either run.
  - The archive suite is now 43 tests: 24 green, 19 red. Eight tests that
    previously stopped at `initialize` are green, and two are new (see below).
  - Each of the eight newly green tests was also run alone with
    `-- --exact <name>`, so none passes only as part of a batch:
    `the_sweep_repairs_a_writing_row_and_marks_a_row_whose_file_is_gone`,
    `the_sweep_deletes_an_eligible_orphan_and_measures_the_quota_total`,
    `the_sweep_leaves_what_it_does_not_recognize_and_reports_it`,
    `the_sweep_neither_reads_counts_nor_deletes_a_reparse_point`,
    `the_sweep_uses_the_last_known_byte_size_when_it_cannot_measure_an_entry`,
    `the_sweep_deletes_nothing_outside_the_archive_directory`,
    `an_orphan_that_could_not_be_deleted_still_counts_towards_the_quota`, and
    `an_unmeasurable_entry_with_no_row_makes_the_total_unavailable`.
  - Every one of the 19 remaining failures is a `todo!` panic, and each was also
    run alone to confirm where it now stops: 15 at `ArchiveWriter::begin`
    (`archive.rs:1226`), 1 at `read` (`1280`), 1 at `delete` (`1290`), and the 2
    queue tests still at `ArchiveQueue::new` (`1096`). Not one stops at
    `initialize` any more, and a scan for `assertion` / `left ==` lines returns
    zero, so nothing is red because it disagrees with an implementation and no
    assertion was weakened, retargeted, or removed to reach green.
  - Two tests that advance past `initialize` assert sweep output before they reach
    their own `todo!`, and those assertions now run and hold:
    `the_total_cap_evicts_ended_archives_oldest_first_and_never_an_open_one`
    (`measured_bytes` and `total_bytes()` both `Known(800)`, so nothing is evicted
    at initialization) and
    `an_unavailable_total_stops_the_archive_instead_of_growing_it`
    (`measured_bytes` is `Unavailable`).
- Two tests added, against code that already existed, with no change to the
  implementation scope: `the_most_severe_reason_is_the_documented_order_over_every_pair`
  checks all 64 pairs against the documented order and its symmetry, through an
  exhaustive-match rank helper so a ninth reason cannot be added without being
  placed in that order; `a_carriage_return_in_the_text_never_becomes_a_line_of_its_own`
  pins that `\r`, `\r\n`, `\n`, and `NUL` inside a captured line stay inside the
  JSON string and out of the file's line structure. The stale
  `// PLACEHOLDER-REAL-FS-ASSERTIONS` comment is gone.
- Sweep decisions worth knowing before reading the diff, all recorded in
  `notes.md`: an unrecognized name is reported and otherwise untouched, readable
  or not; a row's status is parsed before anything is decided, so an unknown status
  is never taken for a file that has gone missing; a `complete` or `partial` row
  whose file is still there is never rewritten; a file some row still names is
  never deleted as an orphan, even when the sweep refused to act on that row; an
  index write that fails leaves the row in the state the next sweep repairs, with
  no retry; and one entry nobody can size and no row remembers makes the whole
  total `Unavailable`.
- Next step: the third slice of 4b. The smallest useful one is
  `ArchiveWriter::begin` plus `is_open`, which is what 15 of the 19 red tests hit
  first. The queue's 9 bodies are independent and can be taken in either order.
  Get the user's approval before starting, as with the first two slices.

### Starting Points, As They Stood Before The Third Slice

Superseded by the subsection of the same purpose at the top of this file. The line
numbers below, and the group of eight `ArchiveWriter` bodies, describe the state
before `begin`, `is_open`, and the writer's open-session map were written.

Seventeen `todo!("step 4b: ...")` bodies remain in
`apps/desktop/src-tauri/src/archive.rs`, in two independent groups:

- `ArchiveQueue`, lines 1086-1134 — nine bodies: `Default` and `new`, `enqueue`,
  `len`, `is_empty`, `queued_bytes`, `drain`, `take_pending_gap`, and `dropped`.
  Two red tests wait here, both panicking at `archive.rs:1096`
  (`ArchiveQueue::new`).
- `ArchiveWriter`'s write path and gates, lines 1226-1290 — eight bodies: `begin`,
  `enqueue`, `pump`, `close`, `close_all`, `is_open`, `read`, and `delete`. Fifteen
  of the remaining red tests stop at `begin`, so `begin` and `is_open` unblock the
  largest share of the group at once.

Reproduce the exact list, from `apps/desktop/src-tauri`, rather than trusting a
remembered one:

```
cargo test --lib --all-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

Both groups are still unauthorized. Read `CLAUDE.md` and `AGENTS.md` first: the
tests may use only temp directories and test databases, an assertion may not be
weakened or retargeted to reach green, and no commit, push, tag, release, CI edit,
command, or frontend work happens without the user asking for it.

## 2026-08-16 v0.3.0 Step 4b, First Slice Landed

- Superseded by the section above for the current test numbers, for what is
  implemented, and for the next session's starting points. Everything below still
  describes how the first slice was reviewed and what it decided.

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets `5e3e0d4`, and nothing was
  committed, pushed, tagged, or released. No CI or release file was touched. The
  working tree is still the same nine paths listed in the section below.
- Done this session: the authorized first slice of step 4b, in
  `apps/desktop/src-tauri/src/archive.rs` only. Implemented — `ArchiveStatus` and
  `ArchiveReason` string mapping and parsing, `ArchiveReason::most_severe`,
  `gap_line`, `encode_record`, `archive_file_name`, `is_archive_file_name`,
  `resolve_archive_path`, `resolve_ordinary_archive_file`, `verified_file_name`,
  `QueueBounds::default`, `QuotaLimits::default`, and all six `RealArchiveFs`
  methods. `pub mod archive;` in `lib.rs` is still the module's only reference in
  the crate, so the feature remains unreachable from the application.
- Deliberately not started, and still `todo!("step 4b: ...")`:
  `ArchiveWriter::initialize` and the rest of the writer, the sweep, the queue,
  the quota and eviction, the `Storage`-backed `ArchiveIndex`, the commands, and
  all frontend work.
- Verification, in `apps/desktop/src-tauri`:
  - `cargo fmt --all -- --check` — clean.
  - `cargo clippy --all-targets --all-features -- -D warnings` — clean, zero
    warnings.
  - `cargo test --lib --all-features` — `130 passed; 27 failed; 1 ignored;
    0 measured; 0 filtered out` out of 158, up from `117 passed; 40 failed` before
    this slice. The archive suite is 41 tests: 14 green, 27 red.
  - Each of the 13 newly green tests was also run alone with `-- --exact`, so none
    passes only as part of a batch.
  - All 27 remaining failures are `todo!` panics: 25 at `ArchiveWriter::initialize`
    (`archive.rs:794`) and 2 at `ArchiveQueue::new` (`archive.rs:724`). A scan of
    the run's output for `assertion` / `left:` / `right:` lines returns zero, so no
    test is red because it disagrees with an implementation, and no assertion was
    weakened, retargeted, or removed to reach green.
- Decisions taken while implementing, recorded in `notes.md`:
  the id rule checks the UUID shape and not the version nibble; `most_severe` is a
  total order over all eight reasons; `RealArchiveFs::create_new` is unbuffered
  because `WRITE_BUFFER_BYTES` belongs to the writer; error messages echo a session
  id but never a `file_name` taken from the database.
- Next step, unchanged in shape: the next slice of 4b — `ArchiveWriter::initialize`
  and the startup sweep, which is what 25 of the 27 red tests are waiting on. The
  queue's 2 tests can be taken in either order. Get the user's approval before
  starting, as with this slice.

### Starting Points, As They Stood Before The Second Slice

Superseded by the subsection of the same purpose at the top of this file. The line
numbers and the group of eleven `ArchiveWriter` bodies below describe the state
before `initialize`, the sweep, `archive_dir`, and `total_bytes` were written.

Twenty `todo!("step 4b: ...")` bodies remain in
`apps/desktop/src-tauri/src/archive.rs`, in two independent groups:

- `ArchiveQueue`, lines 717-761 — nine bodies: `Default` and `new`, `enqueue`,
  `len`, `is_empty`, `queued_bytes`, `drain`, `take_pending_gap`, and `dropped`. Two
  red tests wait here, both panicking at `archive.rs:724` (`ArchiveQueue::new`).
- `ArchiveWriter` and the index gate, lines 794-883 — eleven bodies: `initialize`
  plus the sweep, the owned directory, the byte total, session open, append, pump,
  close, shutdown, the open-session predicate, and archive read and delete. The
  other 25 red tests all panic at `archive.rs:794` (`ArchiveWriter::initialize`), so
  the sweep and `initialize` unblock the whole group at once.

Reproduce the exact list, from `apps/desktop/src-tauri`, rather than trusting a
remembered one:

```
cargo test --lib --all-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

Both groups are still unauthorized. Read `CLAUDE.md` and `AGENTS.md` first: the
tests may use only temp directories and test databases, an assertion may not be
weakened or retargeted to reach green, and no commit, push, tag, release, CI edit,
command, or frontend work happens without the user asking for it.

## 2026-08-16 v0.3.0 Archive Test Hardening, Still Step 4a

- Superseded by the section above for the current test numbers and for what is
  implemented. Everything below still describes how the 41 tests came to exist.

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets `5e3e0d4`, and nothing was
  committed, pushed, tagged, or released. No CI or release file was touched. Only
  `apps/desktop/src-tauri/src/archive.rs` and these four documents changed.
- Done this session and nothing else: two reviewed batches of archive tests and the
  seam changes they required, plus this documentation sync. Still no production
  behavior — every body in `archive.rs` is `todo!("step 4b: ...")`, and
  `pub mod archive;` in `lib.rs` remains the module's only reference in the crate,
  so the feature is unreachable.
- Current test state, in `apps/desktop/src-tauri`:
  - `cargo fmt --all -- --check` — clean.
  - `cargo clippy --all-targets --all-features -- -D warnings` — clean, zero
    warnings.
  - `cargo test --lib --all-features` — `117 passed; 40 failed; 1 ignored;
    0 measured; 0 filtered out` out of 158 run.
  - `cargo test --lib --all-features -- --list` reports 41 `archive::tests`
    entries: 40 red, 1 green. The green one,
    `the_test_filesystem_reports_a_link_as_a_reparse_point`, is the test
    filesystem's own control and is supposed to pass. The other 117 green are the
    step 3 suite's 116 untouched plus that control; the 1 ignored predates this
    work.
  - All 40 failures panic at a `todo!("step 4b: ...")`. Not one is a failing
    assertion, so nothing is red because it disagrees with an implementation.
  - Twenty-five stop at `ArchiveWriter::initialize`. The other fifteen reach their
    own subject: 4 at the read gate, 2 at `ArchiveQueue::new`, and 1 each at the
    file name rule, path resolution, the row-name gate, the status mapping, the
    reason ranking, the record encoding, the gap line, the default bounds, and
    `RealArchiveFs::list_dir`.
- The count moved 27 → 36 → 41 across two review batches. The user's instruction
  named 36 with 1 green and 35 red, which was the state before this session's
  second batch; the second batch added four tests and the first had already added
  one, so 41 is the measured number and the one written into the documents.
- Group sizes, source order: 9 file name and containment, 15 writer and lifecycle,
  4 queue and gap records, 4 byte caps and eviction, 9 startup sweep. Four tests
  are `#[cfg(windows)]`; on a non-Windows host the suite is 37 with none green,
  because the one green test is itself Windows-only.
- Seam changes worth knowing before reading the diff: `list_dir` now returns one
  result per entry, so a single entry whose metadata cannot be read is an anomaly
  the sweep reports instead of an error that aborts it; the measured quota total is
  now known-or-unavailable, and unavailable means "no room"; the test filesystem
  can refuse a named entry's metadata and a named delete, keyed by name rather
  than call order; and `RealArchiveFs` is now compared against the double by a
  Windows test instead of never being executed at all.
- Decisions this session recorded rather than left in code: read and delete keep
  taking a `session_id` and require the row to name that session's own file; an
  undeletable orphan still counts toward the quota; an unmeasurable entry with a
  row contributes its last known `byte_size` and with no row makes the total
  unavailable; a reparse point contributes nothing and has its row's counters
  zeroed. All four are in `V0.3.0_PLAN.md`.
- Test boundaries held: every test works inside a `tempfile::TempDir`, no archive
  test opens a database, and the real application data directory is never opened.
  No test sleeps.
- Next step, 4b first slice, already authorized: the status and reason mappings,
  the file name rule and path resolution, the row-name gate, the JSON Lines and gap
  encodings, the default bounds and limits, and `RealArchiveFs`. Turn the thirteen
  matching tests green one at a time and stop there. `ArchiveWriter::initialize`,
  the queue, the quota, the commands, and the frontend are explicitly not in this
  slice. Do not weaken an assertion to reach green.
- Working tree: intentionally uncommitted and unpushed. Nine paths, unchanged in
  number since step 4a — modified `AGENTS.md`, `HANDOFF.md`, `notes.md`,
  `apps/desktop/src-tauri/src/lib.rs`, `apps/desktop/src-tauri/src/models.rs`,
  `apps/desktop/src-tauri/src/storage.rs`, plus untracked `CLAUDE.md`,
  `V0.3.0_PLAN.md`, and `apps/desktop/src-tauri/src/archive.rs`. Preserve all nine
  when taking over.
- Still unauthorized: commit, push, tag, CI or release changes, the real
  application database, `.env`, and any software-copyright application material.

## 2026-08-15 v0.3.0 Archive Red Tests, Step 4a

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets `5e3e0d4`, and nothing was
  committed, pushed, tagged, or released. No CI or release file was touched.
- Done this session and nothing else: the archive API surface and its 27 red
  tests, in the new `apps/desktop/src-tauri/src/archive.rs`, declared from
  `lib.rs` as `pub mod archive;`. That declaration is the only reference to the
  module anywhere in the crate, so the feature cannot be switched on by this step.
  Every body is `todo!("step 4b: ...")`. No production behavior was written.
- Verification in `apps/desktop/src-tauri`:
  - `cargo fmt --all -- --check` — clean.
  - `cargo clippy --all-targets --all-features -- -D warnings` — clean, zero
    warnings.
  - `cargo test --all-targets` — `116 passed; 27 failed; 1 ignored; 0 measured;
    0 filtered out` for the lib target out of 144 run, `0 passed; 0 failed` for
    the `main.rs` target. The one ignored test
    (`commands::tests::live_imports_detect_conflicts_without_touching_existing_processes`)
    predates this work.
  - All 27 failures are `archive::tests::*` and every one panics at a
    `todo!("step 4b: ...")`. None passes, so none is green by accident. The 116
    green are the step 3 suite, untouched.
  - Fourteen of the 27 stop at `ArchiveWriter::initialize`, the first line of
    their arrangement. Judge 4b on each test going green, not on the count.
- The full test-name list with the body each one reaches, and the four decisions
  these tests forced, are in `V0.3.0_PLAN.md` → Verification. The four:
  the gap line is singular-aware; `QueueBounds` and `QuotaLimits` are parameters
  so a bound can be crossed with a few short records instead of 200 MiB; the
  symlink test is `#[cfg(windows)]` and needs an elevated shell or Developer Mode;
  and the write-failure test uses a record larger than `WRITE_BUFFER_BYTES`
  because a short record would not leave the 64 KiB buffer until close.
- Test boundaries held: every test works inside a `tempfile::TempDir`, and the
  index double records calls in memory, so no archive test opens a database at
  all. The real application data directory is never opened.
- Two seams exist for the tests, each owing one real implementation in 4b:
  `ArchiveFs` (injectable nth-`write` and `sync_data` failure, by call count, never
  by timing) and `ArchiveIndex` (observable row transitions without SQL). No test
  sleeps; `pump(now)` runs in the calling thread and every timestamp is a literal.
- Deliberately left for 4b: the `Storage`-backed `ArchiveIndex`. An adapter would
  need `pub(crate)`, which is dead code in the library build while only tests use
  it, and would duplicate SQL that belongs in `storage.rs`.
- Next step, 4b, and only after the user reviews these signatures: replace the ten
  `todo!` bodies the 27 tests reach, keep every assertion as written, then the
  commands and the frontend. Do not weaken a test to reach green.
  - Superseded by the 2026-08-16 section above. The suite is now 41 tests reaching
    twelve `todo!` bodies, and the numbers in this section are the 2026-08-15 state,
    kept as history.
- Working tree: intentionally uncommitted and unpushed. Nine paths are expected —
  modified `AGENTS.md`, `HANDOFF.md`, `notes.md`,
  `apps/desktop/src-tauri/src/lib.rs`, `apps/desktop/src-tauri/src/models.rs`,
  `apps/desktop/src-tauri/src/storage.rs`, plus untracked `CLAUDE.md`,
  `V0.3.0_PLAN.md`, and `apps/desktop/src-tauri/src/archive.rs`. Preserve all nine
  when taking over.
- Still unauthorized: commit, push, tag, CI or release changes, the real
  application database, `.env`, and any software-copyright application material.

## 2026-08-15 v0.3.0 Schema Migration Implemented

- Status: the `v0.2.1` baseline is unchanged. Local `main` and `origin/main` are
  still at `97943d7`, the `v0.2.1` tag still targets `5e3e0d4`, and nothing was
  committed, pushed, tagged, or released. No CI or release file was touched.
- Approved scope for v0.3.0: the opt-in run log archive only, with reading, the
  history summary, the viewer, delete, and the documentation those require.
  Project Git status is deferred and out of scope.
- Done this session, in this order: `V0.3.0_PLAN.md` third draft; the migration
  red tests (`109 passed; 6 failed; 1 ignored`); then the version 1 to version 2
  migration itself, the `run_log_archives` table, and the `list_sessions` archive
  join. The conditional approval of the schema change is discharged — red first,
  implementation second.
- Verification in `apps/desktop/src-tauri`, all three run after the migration:
  - `cargo fmt --all -- --check` — clean.
  - `cargo clippy --all-targets --all-features -- -D warnings` — clean, zero
    warnings.
  - `cargo test --all-targets` — `116 passed; 0 failed; 1 ignored` for the lib
    target, `0 passed; 0 failed` for the `main.rs` target. The one ignored test
    predates this work.
  - No red test was weakened, relaxed, or deleted to reach green.
  - The `npm` half of the matrix and `cargo tauri build` were not run: no frontend
    file changed this step. The full matrix is still required before the milestone
    is called done.
- Migration semantics, one sentence used unchanged in every document, release note,
  and commit message: 迁移失败时 SQLite 事务回滚并保持 v1；迁移成功后没有应用级回退
  或数据库降级路径。In English: a failed migration rolls back the SQLite transaction
  and stays at v1; a successful migration has no application-level fallback and no
  database downgrade path.
  - A v1 database is opened unchanged by this build and by v0.2.1, and user data is
    untouched.
  - A v2 database **cannot be opened by v0.2.1** — that build rejects any
    `user_version` above 1.
  - The two halves are not a pair and neither is a rollback of the other. Do not
    write "revertible", "rollback", or "downgrade" about a successful migration in
    release notes or `README.md`.
  - A fresh install runs 0 → 1 → 2 as two separate atomic transactions; the
    v1 → v2 upgrade itself is one transaction with `PRAGMA user_version=2` last.
- Production code changed, and nothing else: `apps/desktop/src-tauri/src/models.rs`
  (`RunLogArchiveSummary`, `RunSession.archive`) and
  `apps/desktop/src-tauri/src/storage.rs` (`SCHEMA_VERSION`, the version guard,
  `upgrade_to_version_2` and its call, and the `list_sessions` `LEFT JOIN`).
- Defect the red tests caught, recorded because reading the DDL did not reveal it:
  the `CHECK` arms were written `status = 'partial' AND reason IN (...)`, which does
  not reject a null reason. `NULL IN (...)` is NULL and a SQLite `CHECK` passes on
  NULL, so `partial` and `removed` rows with no reason were accepted. Fixed with an
  explicit `reason IS NOT NULL` in both arms, in the migration and in the test's
  pinned `V2_ADDITION`, plus a ninth rejection case for the `removed` arm.
- Not started: the startup sweep, the writer thread, the queue, the quota and
  eviction, the lifecycle transitions, the commands, and all frontend work
  including `types.ts`. The sweep was deliberately moved out of this step: it
  re-measures files, deletes orphan files, and initializes the quota counter, all
  of which need an archive directory that does not exist until the writer exists.
- Next step, required in this order: write failing red tests for the startup sweep,
  the writer, the queue, the quota, and the archive lifecycle, confirm they fail
  for the right reason, and only then implement. Do not implement first.
- Working tree: intentionally uncommitted and unpushed. Seven paths are expected —
  modified `AGENTS.md`, `HANDOFF.md`, `notes.md`,
  `apps/desktop/src-tauri/src/models.rs`,
  `apps/desktop/src-tauri/src/storage.rs`, plus untracked `CLAUDE.md` and
  `V0.3.0_PLAN.md`. Preserve all seven when taking over.
- Still unauthorized: commit, push, tag, CI or release changes, the real
  application database, `.env`, and any software-copyright application material.

## 2026-08-14 External Agent Handoff / Waiting

- Status: RunCove `v0.2.1` remains published and verified. Local `main` and
  `origin/main` are synchronized at
  `97943d7fabbbd400481171568bf970b38a2c9afa`; the annotated `v0.2.1` tag
  still targets release commit `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`.
- Handoff: the user plans to let Claude temporarily continue the project and
  may later ask Codex to review the resulting changes. `CLAUDE.md` is the short
  entry point; `AGENTS.md`, this file, and `notes.md` remain authoritative.
- Direction: the user wants to expand the product and explore using RunCove for
  a software copyright registration application. No specific feature list,
  registration checklist, legal interpretation, or new-version scope has been
  approved. Current requirements must be verified from authoritative sources
  before that work is planned.
- Engineering position: preserve the released behavior and safety boundaries.
  Do not add meaningless code merely to increase source volume. Separate real
  product improvements from application-material preparation, and obtain plan
  approval before a large implementation.
- Current activity: waiting for the user's next instruction. This checkpoint
  makes no code, database, CI, release, tag, runtime-process, or remote change.
- Working tree: this handoff is intentionally uncommitted and unpushed. Five
  paths are expected — modified `AGENTS.md`, `HANDOFF.md`, and `notes.md`, plus
  untracked `CLAUDE.md` and `V0.3.0_PLAN.md`; preserve all five when taking over.
  The v0.3.0 plan is a proposal under review, not an approved milestone.
- Suggested next-session prompt:

  ```text
  接手 RunCove 项目，项目路径是 D:\CodexProject\personal-projects\runcove。
  请先阅读 CLAUDE.md、AGENTS.md、HANDOFF.md、notes.md 和 README.md，并核验
  git status、main/origin/main 以及 v0.2.1 发布基线。当前项目处于等待状态，
  先不要修改代码、提交或推送；我后续会再告诉你具体的扩充和软著方向需求。
  收到需求后，先区分真正的产品改进与申请材料工作，核验最新权威要求，提出
  差距清单、待确认问题、版本计划和验证方案，等我确认后再实施。不要为了凑
  代码量增加无意义功能，不要修改 CI/Release/标签，不要操作其他项目、.env
  或现有开发进程，并持续更新 HANDOFF.md 和 notes.md。
  ```

## 2026-08-13 v0.2.1 Published

- Status: the approved implementation and full local verification are complete.
  PR #2 merged into `main` as `a771c55f402bcdce3d0ec29fe739d4e47bd847c5`.
  Main CI run `31691103911` passed, and annotated tag `v0.2.1` targets release
  commit `5e3e0d4d63ae04fe8e27c37c4500d3bd9ef75f13`.
- Published: Release workflow `31692200475` completed successfully and created
  the latest, non-draft, non-prerelease release at
  <https://github.com/AbyssWhalen/RunCove/releases/tag/v0.2.1>.
- Assets: four cross-platform CLI archives, the Windows x64 portable desktop
  archive, and `SHA256SUMS.txt` are present. All five archive digests match the
  published checksum file.
- Completed: run history, structured conflict navigation, discovery states and
  retry, profile duplication and validation, port-detail copy controls, and
  bilingual Help are implemented. Package and application metadata now report
  `0.2.1`.
- Final audit fixes: error messages now clear stale related-port actions, and a
  conflict focus filters by the exact port plus protocol. Focused frontend
  regression tests pass (`39/39`), and the expanded bilingual Help/project/i18n
  tests pass (`22/22`).
- Final verification: root Rust `38/38`; desktop Rust `109 passed / 1 ignored`;
  frontend Vitest `114/114`, lint, typecheck, and build; Edge Playwright `6/6`
  across the required viewports and new workflows; `git diff --check` passed.
- Local release: RunCove file/product version `0.2.1`, `25,418,344` bytes,
  SHA-256 `4B00DD7F72B6AAD29646684DB7F852D691C7183A4BF78DA703C06556A9BA3A78`,
  unsigned. It was built but not launched, installed, packaged, or published.
- Residual boundaries: desktop remains Windows-first, historical logs are not
  archived, and the explicitly configured live-service acceptance remains
  ignored by default. See `notes.md` for complete evidence.
- Next session prompt: RunCove `v0.2.1` is published. Read `AGENTS.md`, this
  checkpoint, and `notes.md`, then treat further work as post-release
  maintenance or a separately planned version. Do not rebuild or republish
  `v0.2.1` merely to repeat already-green evidence.

## 2026-08-12 v0.2.1 Implementation Checkpoint

- Status: implementation started from clean `main` at `bf3d532`; no commit,
  push, release, workflow, repository, tag, or unrelated-process action is
  authorized.
- Plan: `V0.2.1_PLAN.md` is the decision-complete implementation source.
- In progress: establish the full verification baseline, then implement run
  history, actionable conflict navigation, discovery feedback, project editor
  validation/duplication, port-detail copy controls, and bilingual help.
- Required completion: update this checkpoint and `notes.md` with fresh test,
  E2E, and local release-build evidence. Keep the existing compatibility CLI.
- Next session prompt: read `AGENTS.md`, `V0.2.1_PLAN.md`, this checkpoint, and
  `notes.md`; inspect the working tree before continuing and preserve all
  completed or concurrent work.

## Current Published Checkpoint (2026-08-12)

- The GitHub repository is now canonically named `AbyssWhalen/RunCove`; the
  local `origin` remote and Cargo metadata use the renamed URL. The public
  description explains the port monitor, npm launch, process, log, and restore
  workflows in one sentence.
- RunCove `v0.2.0` is published in the public
  `AbyssWhalen/RunCove` repository. PR #1 is merged, and release source commit
  `9b935857fcc79b2811a5a1fb16df9aae55a91e7a` is the annotated `v0.2.0` tag.
  The historical `v0.1.0` tag and release remain unchanged; the repository was
  renamed without rewriting history.
- PR CI run `31561867655`, `main` CI run `31562443457`, and release workflow run
  `31563084142` all completed successfully. The release is neither a draft nor
  a prerelease and is marked latest:
  <https://github.com/AbyssWhalen/RunCove/releases/tag/v0.2.0>.
- The release contains five binary archives plus `SHA256SUMS.txt`: four
  cross-platform CLI archives and the Windows x64 portable desktop archive.
  All six GitHub asset digests were compared with fresh downloads, all five
  archive checksums match `SHA256SUMS.txt`, every archive extracts, and the
  contained binaries have the expected ELF, Mach-O, or PE formats. The Windows
  desktop executable reports file/product version `0.2.0` and is unsigned as
  documented.
- The Windows release preserves the primary `runcove.exe` CLI and a separate
  compatibility executable for existing scripts. Their JSON, range filtering,
  and error exit-code behavior matched in release-package smoke checks.
- The GitHub description and topics identify RunCove at its renamed canonical
  URL. Only standard hosted runners are
  used; no larger runner is configured. This post-release documentation
  checkpoint does not alter the `v0.2.0` tag or published assets.
- Remaining follow-ups are non-blocking: real interactive UAC cancel/success
  smoke coverage, a stricter environment-driven live-port acceptance schema,
  longer idle performance observation, and renewed name clearance before wider
  package-manager or commercial distribution.

## Pre-publication Checkpoint (Historical, 2026-08-12)

The following section records the final CI diagnosis and release preparation as
they stood before PR #1 was merged. Pending-push and pending-publication
statements below are historical, not current instructions.

- Draft PR #1 remained open at remote head `c281340`. The Windows resource split
  had been pushed and CI run `31501585406` proved that the MSVC library test harness
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
  ignored`. This follow-up had not been pushed at that checkpoint.
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
- `AbyssWhalen/RunCove` is public and both workflows use only standard
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
- Native smoke proved the final process remained PID 33824 after the title-bar X
  hides it and a second launch restores the same window; process count remains
  one. The old pre-fix PID 9728 was identity-checked, found to contain only
  RunCove/WebView2 processes, and stopped. That instance was left running at
  this historical checkpoint and is no longer a statement about current runtime
  state.
- At this historical checkpoint, GitHub publication was not complete: PR #1 was
  still Draft and the final local resource fix had not advanced the branch.

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
- Verification at that checkpoint was green: root Rust format/Clippy/37 tests;
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
- The isolated release executable at that checkpoint was
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

- Completed: cloned the original port-inspection repository at commit
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
- A compatibility CLI command remains available for existing scripts.
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

## Next Session Prompt (Historical, v0.2.0)

Superseded by the top section of this file. Kept as the record of what the prompt said
at `v0.2.0`; do not follow it as current instructions.

RunCove `v0.2.0` is already merged and published from release commit `9b93585` in
the `AbyssWhalen/RunCove` repository. Start with the published
checkpoint above and the final publication record in `notes.md`; do not repeat
  the completed resource diagnosis, PR, tag, release, or asset verification.
Treat subsequent work as post-release maintenance or a new version. Preserve
the historical `v0.1.0` tag/release and do not stop unrelated local development
services. Real interactive UAC cancel/success, a stricter live-port acceptance
schema, and longer idle observation remain residual checks and must not be
overstated.
