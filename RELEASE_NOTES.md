# RunCove v0.4.1

RunCove v0.4.1 is a fix release for v0.4.0. It changes no features and needs no
database migration: if you are already on v0.4.0, this is a drop-in replacement,
and the download is about a third smaller.

## Fixed

- **Two ordered starts that share a profile no longer fail each other.** Starting
  two launch groups that both contain the same database, or starting a group while
  a restore is bringing the same profiles back, used to fail whichever arrived
  second with an "already starting" message the user had done nothing to cause.
  Each one now waits for the shared profile to settle and carries on, so the
  startup order every group promises still holds and a profile the other one
  already brought up simply counts as started.
- **`SHA256SUMS.txt` now passes `sha256sum -c`.** The v0.4.0 file put three spaces
  between each hash and its filename where the format allows exactly two, so every
  line failed to open on a byte-perfect download. The workflow now writes the
  accepted format and verifies the file before publishing, in the same job that
  writes it.
- **A failed restore names the profile it stopped at** as `Project / Profile`
  rather than printing an internal id, which is what a whole-group failure has
  always done.
- **A restore and a whole-group start are no longer offered at the same time.**
  Waiting makes overlapping them correct, but the second would only sit and wait
  for work already underway, so the button that starts it is disabled while the
  first runs.
- **The desktop download is about a third smaller** — roughly 8.6 MB against 13 MB.
  The project's release build settings had never reached the desktop application:
  they were written once at the repository root, and the desktop app is a separate
  Cargo package rather than a workspace member, so it was the only shipped binary
  built without them. Nothing about how RunCove behaves changes.

## Upgrade Note

**No migration runs and no backup is needed if you are coming from v0.4.0.** This
release reads the same schema version 3 database.

Coming from v0.3.0 or earlier, the migration described in v0.4.0's notes still
applies: the database upgrades to schema version 3 on first launch, each step runs
in one transaction and stays at the previous version if it fails, and a successful
upgrade has no downgrade path — **v0.3.0 and earlier cannot open a version 3
database.** Copy `runcove.sqlite3` out of `%LOCALAPPDATA%\com.abysswhale.runcove\`
first if you may need to go back.

## Known Limitations

- A launch group starts and stops only when you press its button. There is no
  start-at-login and no automatic project startup, by design.
- In the rare case that a run log archive's final buffered flush fails, the
  archive line count can over-report which buffered line reached disk. The byte
  count is measured from the file, and normal writes are unaffected.
- The `SHA256SUMS.txt` published with **v0.4.0** still has the formatting defect
  above; it cannot be changed after the fact. Verify that older release with
  `sed -E 's/^([0-9a-f]{64})[[:space:]]+/\1  /' SHA256SUMS.txt | sha256sum -c -`.
  This release's file needs no workaround.

## Downloads

- `runcove-desktop-windows-x86_64-portable.zip` contains the unsigned Windows
  desktop application.
- `runcove-cli-*` archives contain the cross-platform `runcove` CLI and a
  compatibility executable for existing scripts.
- `SHA256SUMS.txt` contains SHA-256 checksums for every binary archive.

Windows SmartScreen may show an unknown-publisher warning because the desktop
executable is not code-signed. Download it only from this GitHub release and
verify the archive against `SHA256SUMS.txt`. Microsoft Edge WebView2 Runtime is
required and is included with current Windows 11 systems.

To verify, put `SHA256SUMS.txt` beside the archives you downloaded and run:

```bash
sha256sum -c SHA256SUMS.txt
```

```powershell
# Windows PowerShell, no sha256sum available
(Get-FileHash runcove-desktop-windows-x86_64-portable.zip -Algorithm SHA256).Hash
# compare, case-insensitively, against the matching line in SHA256SUMS.txt
```

`SHA256SUMS.txt` lists every archive in the release, so if you downloaded only one,
add `--ignore-missing` — a plain `-c` counts the archives you did not download as
failures.
