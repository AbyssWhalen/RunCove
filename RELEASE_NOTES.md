# RunCove v0.3.0

RunCove v0.3.0 adds an optional persistent archive for managed-session logs.
The archive is off by default, stays inside RunCove's local application data,
and does not change the existing in-memory log or process-safety model unless
you explicitly enable it.

## Highlights

- Enable **Archive run logs** from the log drawer for future runs. RunCove does
  not backfill output from a session that was already running.
- Reopen archived stdout and stderr from run history. The viewer starts at the
  tail and loads earlier records in bounded pages.
- See whether an archive is writing, finalizing, complete, partial, or removed,
  together with its line count, size, and any dropped output.
- Delete an archive after confirmation while retaining its run-history entry.
- Keep disk use bounded at 10 MiB per session and 200 MiB in total. RunCove
  reclaims the oldest finished archives first and never evicts one still open.

## Reliability And Privacy

- A bounded background queue prevents archive I/O from slowing the child
  process. If output arrives too quickly, the dropped lines and bytes are
  reported instead of silently disappearing.
- Startup recovery repairs interrupted rows and accounts for existing files.
  Archive paths are restricted to RunCove-generated names inside its dedicated
  archive directory; links and Windows reparse points are not followed.
- Archive initialization failure does not stop port scanning, project launch,
  process control, or the existing in-memory log drawer.
- Archived output is not filtered. A service can print credentials, tokens,
  personal data, or other sensitive content, so enable archiving only when that
  local persistence is appropriate. RunCove uploads no archive data.

## Upgrade Note

The desktop database migrates from schema version 1 to version 2 on first
launch. The migration itself is transactional, but a successful upgrade has no
downgrade path: RunCove v0.2.1 cannot open the upgraded database. Back up the
RunCove application-data directory before launching v0.3.0 if you may need to
return to v0.2.1.

## Known Limitations

- In the rare case that the final buffered flush fails, the archive line count
  can over-report which buffered line reached disk. The byte count is measured
  from the file, and normal writes are unaffected.
- Process stop and normal-exit messages produced by the backend remain English
  when the interface language is Simplified Chinese.

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
