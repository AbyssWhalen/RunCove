# RunCove v0.4.0

RunCove v0.4.0 adds launch groups. A group is a named, ordered set of launch
profiles that starts or stops as one unit, so the set of services you bring up
every morning becomes one button instead of several.

## Highlights

- Create as many groups as you need, each with its own name and startup order.
  Members may come from different projects, so one group can bring up a
  database, an API, and a web front end together.
- Whole-group start walks the members in the order you set and waits for each
  one's expected port before moving on, exactly as a single-profile start does.
  A member that is already running counts as started, so pressing Start again
  only fills in what is missing.
- A failed start stops before the next member and keeps everything that already
  started. The message names the member it stopped at, says how many started
  before it, and offers the same **View occupant** action as a single-profile
  port conflict.
- Whole-group stop walks the members in reverse, and a member it cannot stop
  does not stop the rest. The report counts every failure and names the first.
- Each group shows its startup order and whether it is fully running, partly
  running, or not running. Deleting a launch profile removes it from every group
  that used it.

## Fixed

- Process stop and exit messages now follow the interface language. A Simplified
  Chinese interface no longer shows English sentences such as `Stopped by user`
  in the status toast or the log drawer.
- Fields in the project editor keep their own names once validation errors
  appear, so a screen reader no longer announces `Program This field is
  required.` as a field's name.
- Saving an existing project records the time it was saved rather than the time
  it was first added.

## Upgrade Note

The desktop database migrates to schema version 3 on first launch to store
launch groups — from version 2 if you are coming from v0.3.0, and through
version 2 if you are coming from an older release. Each migration runs in one
transaction and stays at the previous version if it fails, but a successful
upgrade has no downgrade path: **v0.3.0 and earlier cannot open the resulting
version 3 database.** Copy `runcove.sqlite3` out of RunCove's application-data
directory (`%LOCALAPPDATA%\com.abysswhale.runcove\`) before launching v0.4.0 if
you may need to return to an earlier version.

## Known Limitations

- A launch group starts and stops only when you press its button. There is no
  start-at-login and no automatic project startup, by design.
- In the rare case that a run log archive's final buffered flush fails, the
  archive line count can over-report which buffered line reached disk. The byte
  count is measured from the file, and normal writes are unaffected.

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
failures. The v0.4.0 file has a formatting defect that makes `-c` report
`No such file or directory` for every line even when all five archives are present;
the checksums themselves are correct. See the v0.4.0 entry in `CHANGELOG.md` for the
one-line workaround.
