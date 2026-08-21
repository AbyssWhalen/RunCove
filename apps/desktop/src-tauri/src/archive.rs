//! Opt-in on-disk run log archive: the engine.
//!
//! [`ArchiveWriter`] owns one directory of `<session-id>.jsonl` files and the index
//! rows that describe them. It knows nothing about Tauri, settings, or threads —
//! [`crate::archive_service`] supplies all three — so everything here can be tested
//! against an ordinary temporary directory.
//!
//! The writer has no thread of its own. A caller opens a session with
//! [`ArchiveWriter::begin`], hands records over with [`ArchiveWriter::enqueue`],
//! and some thread — any thread — turns them into bytes by calling
//! [`ArchiveWriter::pump`]. That split is the point: a capture thread pays only for
//! a short in-memory critical section, never for a disk write. [`ArchiveQueue`]
//! holds what has been accepted and not yet written under a batch-and-settle
//! protocol: the queue owns a record until `release` or `discard` settles it, so a
//! pump that returns `Err` leaves everything it had taken reserved at the front and
//! the next batch appends behind it — a retry resumes at the same record in the same
//! order.
//!
//! Losses are counted, never hidden. A record refused because a queue bound, a
//! session's byte cap, or the directory's total cap left no room is charged to its
//! session's drop counters, and the next accepted record for that session carries
//! the gap forward as a `system` line in the file; whatever no later record could
//! carry is written by [`ArchiveWriter::close`]. A session therefore ends as
//! `complete` only when nothing was lost, and as `partial` with a reason when
//! something was — the file never silently omits lines.
//!
//! The lock order is `pump_lock → open → queue → total`. The three state locks are
//! never held across a file operation or an index write; `pump_lock` spans the
//! writer's I/O by design, which is what serializes a pump against a close for the
//! same file handle.
//!
//! Two seams exist for the tests, not for production configurability:
//! [`ArchiveFs`] makes a write or `sync_data` failure injectable, and
//! [`ArchiveIndex`] keeps the row transitions observable without SQL. Production
//! uses [`RealArchiveFs`] and `impl ArchiveIndex for `[`crate::storage::Storage`].

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io;
use std::io::{Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::error::{invalid, AppResult};
use crate::models::{LogStream, RunLogArchivePage, RunLogArchiveRecord};
use crate::storage;

/// Directory RunCove owns alone. It is a child of the application-local data
/// directory that holds `runcove.sqlite3`, never that directory itself.
pub const ARCHIVE_DIR_NAME: &str = "run-log-archives";

/// The only extension this build generates.
pub const ARCHIVE_FILE_EXTENSION: &str = "jsonl";

/// Hard byte cap for one session's archive file.
pub const SESSION_BYTE_CAP: u64 = 10 * 1024 * 1024;

/// Hard byte cap for the whole archive directory.
pub const TOTAL_BYTE_CAP: u64 = 200 * 1024 * 1024;

/// Queue bound for one session, in records.
pub const SESSION_QUEUE_RECORDS: usize = 2_048;

/// Queue bound for one session, in bytes of archived text.
pub const SESSION_QUEUE_BYTES: usize = 4 * 1024 * 1024;

/// Queue bound across all sessions, in records.
pub const TOTAL_QUEUE_RECORDS: usize = 4_096;

/// Queue bound across all sessions, in bytes of archived text.
pub const TOTAL_QUEUE_BYTES: usize = 8 * 1024 * 1024;

/// Buffer each open archive file writes through before a flush.
pub const WRITE_BUFFER_BYTES: usize = 64 * 1024;

/// Fewest records one page may be asked for.
pub const MIN_PAGE_RECORDS: usize = 1;

/// Records one page returns when the caller names no bound.
pub const DEFAULT_PAGE_RECORDS: usize = 500;

/// Most records one page may consume.
pub const MAX_PAGE_RECORDS: usize = 2_000;

/// Bytes of archived text one page may consume before it stops.
///
/// It bounds the payload one `read_run_log_archive` call sends over IPC:
/// [`MAX_PAGE_RECORDS`] lines at the 16 KiB per-line capture limit would
/// otherwise be a 32 MiB message.
pub const PAGE_BYTE_CAP: usize = 1024 * 1024;

/// Block the backward scan reads at a time, so a page never loads a whole file.
pub const READ_BLOCK_BYTES: usize = 64 * 1024;

/// Lifecycle state of one archive, mirroring the `run_log_archives.status`
/// column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveStatus {
    Writing,
    Complete,
    Partial,
    Removed,
}

impl ArchiveStatus {
    /// The exact string stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Writing => "writing",
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Removed => "removed",
        }
    }

    /// `None` for a value this build does not know, which a database written by
    /// a newer build may contain.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "writing" => Some(Self::Writing),
            "complete" => Some(Self::Complete),
            "partial" => Some(Self::Partial),
            "removed" => Some(Self::Removed),
            _ => None,
        }
    }
}

/// Why an archive is `partial` or `removed`. A `writing` or `complete` archive
/// has no reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveReason {
    WriteError,
    QuotaExceeded,
    QueueOverflow,
    Interrupted,
    UserDisabled,
    QuotaEvicted,
    UserDeleted,
    FileMissing,
}

impl ArchiveReason {
    /// The exact string stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WriteError => "write-error",
            Self::QuotaExceeded => "quota-exceeded",
            Self::QueueOverflow => "queue-overflow",
            Self::Interrupted => "interrupted",
            Self::UserDisabled => "user-disabled",
            Self::QuotaEvicted => "quota-evicted",
            Self::UserDeleted => "user-deleted",
            Self::FileMissing => "file-missing",
        }
    }

    /// `None` for a value this build does not know.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "write-error" => Some(Self::WriteError),
            "quota-exceeded" => Some(Self::QuotaExceeded),
            "queue-overflow" => Some(Self::QueueOverflow),
            "interrupted" => Some(Self::Interrupted),
            "user-disabled" => Some(Self::UserDisabled),
            "quota-evicted" => Some(Self::QuotaEvicted),
            "user-deleted" => Some(Self::UserDeleted),
            "file-missing" => Some(Self::FileMissing),
            _ => None,
        }
    }

    /// The reason a session keeps when several apply before it closes. A write
    /// error outranks a quota stop, which outranks a queue overflow, because the
    /// first tells the user their archive is missing bytes for a reason they can
    /// act on.
    pub fn most_severe(first: Self, second: Self) -> Self {
        if second.severity() > first.severity() {
            second
        } else {
            first
        }
    }

    /// Rank used only by [`ArchiveReason::most_severe`].
    ///
    /// Only the first three ever actually compete: they are the reasons a session
    /// can accumulate while it is still writing. The rest are ranked too, so the
    /// choice is a total order and picking between two of them cannot depend on
    /// which argument came first. Among those, what happened to the archive
    /// outranks what RunCove or the user chose to do with it.
    fn severity(self) -> u8 {
        match self {
            Self::WriteError => 8,
            Self::QuotaExceeded => 7,
            Self::QueueOverflow => 6,
            Self::Interrupted => 5,
            Self::FileMissing => 4,
            Self::QuotaEvicted => 3,
            Self::UserDeleted => 2,
            Self::UserDisabled => 1,
        }
    }
}

/// One line on its way to disk. The archive stores the already-decoded text of
/// a `RunLogEvent`, not the raw bytes the child process wrote.
#[derive(Debug, Clone)]
pub struct ArchiveRecord {
    pub session_id: String,
    pub stream: LogStream,
    pub line: String,
    pub timestamp: i64,
}

/// Lines and bytes of archived text a session lost, whatever the cause.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DropCounters {
    pub lines: i64,
    pub bytes: i64,
}

impl DropCounters {
    /// One more lost line, carrying `bytes` of archived text.
    ///
    /// Saturating, like the quota's own accounting: a counter that wrapped would
    /// be worse than one that stopped, because the row's `CHECK` refuses a
    /// negative one. A dropped line always costs a line and may cost no bytes at
    /// all — a captured line can be empty — which is the one direction the
    /// `CHECK` allows.
    fn record_loss(&mut self, bytes: usize) {
        self.lines = self.lines.saturating_add(1);
        self.bytes = self
            .bytes
            .saturating_add(i64::try_from(bytes).unwrap_or(i64::MAX));
    }

    /// Take another run of losses back into this one.
    ///
    /// Used when a record that was carrying a gap is itself lost: the run it was
    /// going to report did not stop happening, so it returns to the pending run
    /// and the next accepted record carries both. Saturating for the same reason
    /// as [`DropCounters::record_loss`].
    fn absorb(&mut self, other: Self) {
        self.lines = self.lines.saturating_add(other.lines);
        self.bytes = self.bytes.saturating_add(other.bytes);
    }
}

/// One record on its way out of the queue, with whatever was lost immediately
/// before it.
///
/// The gap is the queue's annotation on the hand-off, not part of the line the
/// child process wrote, so it lives here rather than on [`ArchiveRecord`]. Two
/// things follow. The capture side cannot set a gap, because the type it builds
/// has no field for one. And [`encode_record`] cannot quietly leave a gap out of
/// the three keys it writes, because a caller holding a `QueuedRecord` has to
/// reach through `record` to call it, which puts `gap_before` in front of
/// whoever wrote that line.
///
/// The writer emits the gap line first and the record second, which is what
/// places the marker exactly where the loss happened.
#[derive(Debug, Clone)]
pub struct QueuedRecord {
    pub record: ArchiveRecord,
    /// What this session lost since its last accepted record. `None` when
    /// nothing was lost: an empty gap is not a gap.
    pub gap_before: Option<DropCounters>,
}

/// Everything the queue still owed a session when it ended, handed over once.
///
/// This is the queue's last word on a session. Taking it is what lets the queue
/// forget the session, so the two halves of the drop history — the run nobody
/// carried, and the cumulative totals the row reports — have to leave together
/// in one value rather than in two calls the caller could get half of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FinishedSession {
    /// The trailing run of losses no accepted record carried, for the gap line
    /// written immediately before the closing row. `None` when the session is
    /// owed nothing.
    pub residual_gap: Option<DropCounters>,
    /// Everything this session ever lost, for the row's `dropped_lines` and
    /// `dropped_bytes`.
    pub dropped: DropCounters,
}

/// The four counters one index row carries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArchiveCounters {
    pub line_count: i64,
    pub byte_size: i64,
    pub dropped_lines: i64,
    pub dropped_bytes: i64,
}

/// The `system` line that stands in for a contiguous run of dropped records.
/// The text is fixed English, like the other `system` lines the run log already
/// emits.
pub fn gap_line(dropped: DropCounters) -> String {
    format!(
        "[RunCove: dropped {} {} / {} {}]",
        dropped.lines,
        if dropped.lines == 1 { "line" } else { "lines" },
        dropped.bytes,
        if dropped.bytes == 1 { "byte" } else { "bytes" },
    )
}

/// One JSON Lines record, without its trailing newline.
///
/// The session id is not part of the line: the file name already carries it, and
/// repeating it on every line would cost more bytes than the record itself.
pub fn encode_record(record: &ArchiveRecord) -> String {
    serde_json::to_string(&EncodedRecord {
        t: record.timestamp,
        s: record.stream,
        l: &record.line,
    })
    .expect("an archive record serializes")
}

/// The on-disk shape of one record. The keys are short because every line
/// repeats them, and `serde_json` does the escaping, so a line carrying a quote,
/// a backslash, or a newline still occupies exactly one line of the file.
#[derive(Serialize)]
struct EncodedRecord<'a> {
    t: i64,
    s: LogStream,
    l: &'a str,
}

/// One record read back, or `None` when those bytes are not a record this build
/// wrote.
///
/// Deliberately total: a page skips a record it cannot read and counts it, so
/// one torn or foreign line never costs the user the rest of the page. Decoding
/// from bytes rather than from `&str` is what makes invalid UTF-8 one of the
/// cases this returns `None` for instead of a separate error path.
fn decode_record(line: &[u8]) -> Option<RunLogArchiveRecord> {
    let decoded: DecodedRecord = serde_json::from_slice(line).ok()?;
    Some(RunLogArchiveRecord {
        stream: decoded.s,
        line: decoded.l,
        timestamp: decoded.t,
    })
}

/// The reading half of [`EncodedRecord`], with the same three keys.
///
/// A separate type because the writing half borrows its line and this one owns
/// it. `deny_unknown_fields` is deliberately absent: a key a later build adds
/// must not turn every record it wrote into a malformed one here.
#[derive(Deserialize)]
struct DecodedRecord {
    t: i64,
    s: LogStream,
    l: String,
}

/// Which bound ended a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PageBound {
    /// The page reached the start of the file, so nothing precedes it.
    Start,
    /// The page consumed as many records as it was allowed.
    Lines,
    /// The page consumed as much archived text as it was allowed.
    Bytes,
}

impl PageBound {
    /// The exact string the page carries to the user interface.
    fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Lines => "lines",
            Self::Bytes => "bytes",
        }
    }
}

/// What the backward scan found, before the index row's own fields join it.
struct ScannedPage {
    /// Oldest record first, the order the viewer prints them in.
    records: Vec<RunLogArchiveRecord>,
    page_start_offset: u64,
    stopped_by: PageBound,
    incomplete_tail_skipped: bool,
    malformed_lines: i64,
}

/// The records the caller asked for, clamped to what one page may carry.
fn clamped_page_records(requested: Option<usize>) -> usize {
    requested
        .unwrap_or(DEFAULT_PAGE_RECORDS)
        .clamp(MIN_PAGE_RECORDS, MAX_PAGE_RECORDS)
}

/// Fill `buf` from `offset`, insisting on every byte.
///
/// Every block a page reads lies below a length measured before the file was
/// opened, and an archive is append-only, so a short fill is not the end of the
/// file — it means the file changed underneath the reader. That is reported
/// rather than served as a page of whatever happened to arrive.
fn fill_exact(
    file: &mut dyn ArchiveReadFile,
    session_id: &str,
    offset: u64,
    buf: &mut [u8],
) -> AppResult<()> {
    let filled = file.fill_at(offset, buf).map_err(|error| {
        invalid(format!(
            "The run log archive of session {session_id} could not be read: {error}"
        ))
    })?;
    if filled != buf.len() {
        return Err(invalid(format!(
            "The run log archive of session {session_id} changed while it was being read"
        )));
    }
    Ok(())
}

/// How many bytes to read when `remaining` bytes lie below the scan.
fn block_len(remaining: u64) -> usize {
    usize::try_from(remaining.min(READ_BLOCK_BYTES as u64)).unwrap_or(READ_BLOCK_BYTES)
}

/// Where the caller's cursor puts the end of the region to read.
///
/// An absent cursor means the end of the file, because the reason to open a run
/// log is what it printed last. A present one must be a boundary this module
/// returned earlier, and anything else is refused rather than resynced to a
/// nearby newline, so a client bug is visible instead of plausible.
fn validated_cursor(
    file: &mut dyn ArchiveReadFile,
    session_id: &str,
    before_offset: Option<u64>,
    file_length: u64,
) -> AppResult<u64> {
    let Some(offset) = before_offset else {
        return Ok(file_length);
    };
    if offset == 0 || offset > file_length {
        return Err(invalid(format!(
            "Offset {offset} is not a record boundary of session {session_id}'s run log archive, which is {file_length} bytes long"
        )));
    }

    let mut terminator = [0u8; 1];
    fill_exact(file, session_id, offset - 1, &mut terminator)?;
    if terminator[0] != b'\n' {
        return Err(invalid(format!(
            "Offset {offset} is not a record boundary of session {session_id}'s run log archive"
        )));
    }
    Ok(offset)
}

/// One past the last `\n` below `end`, or `0` when the region holds none.
///
/// Bytes after that point are not a record: mid-flush for an archive still being
/// written, a torn write for a closed one. Each block is discarded once it has
/// been looked at, so a file holding no newline at all costs one block of memory
/// rather than its own size.
fn last_record_end(file: &mut dyn ArchiveReadFile, session_id: &str, end: u64) -> AppResult<u64> {
    let mut block = Vec::new();
    let mut scan_end = end;
    while scan_end > 0 {
        let take = block_len(scan_end);
        let start = scan_end - take as u64;
        block.clear();
        block.resize(take, 0);
        fill_exact(file, session_id, start, &mut block)?;
        if let Some(index) = block.iter().rposition(|byte| *byte == b'\n') {
            return Ok(start + index as u64 + 1);
        }
        scan_end = start;
    }
    Ok(0)
}

/// The records wholly inside `[0, cursor)` that fit one page, newest end first
/// and returned oldest first.
///
/// The scan reads fixed blocks from `cursor` backwards and keeps only the bytes
/// it has not yet turned into records, so the memory it holds is a block or two
/// plus the record it is currently assembling — never the file. Two rules make
/// the page always make progress: the record count is clamped to at least one,
/// and the byte bound never refuses the first record, so a single record larger
/// than the whole cap can still be read and paged past instead of standing in
/// front of everything older than it forever.
fn scan_page_backwards(
    file: &mut dyn ArchiveReadFile,
    session_id: &str,
    cursor: u64,
    max_records: usize,
) -> AppResult<ScannedPage> {
    let region_end = last_record_end(file, session_id, cursor)?;
    let mut page = ScannedPage {
        records: Vec::new(),
        // A region holding one record boundary holds at least one record, and
        // the two rules above mean the loop below always consumes it, so this
        // stands only for a region with no whole record in it at all.
        page_start_offset: region_end,
        stopped_by: PageBound::Start,
        incomplete_tail_skipped: region_end != cursor,
        malformed_lines: 0,
    };

    // The window holds the file bytes `[window_start, resolved_from)`, and
    // `resolved_from` is always one past a `\n` — or `region_end`, which is too —
    // so the window's last byte is a terminator and every record inside it is
    // whole.
    let mut window: Vec<u8> = Vec::new();
    let mut window_start = region_end;
    let mut resolved_from = region_end;
    let mut consumed = 0;
    let mut consumed_bytes = 0usize;

    loop {
        if resolved_from == 0 {
            page.stopped_by = PageBound::Start;
            break;
        }
        if consumed == max_records {
            page.stopped_by = PageBound::Lines;
            break;
        }

        // Where this record starts, reading further back only when the window
        // does not already hold its beginning.
        let line_start_in_window = loop {
            // The window's own last byte is this record's terminator, so the
            // boundary being looked for is the newline before it.
            let searchable = window.len().saturating_sub(1);
            if let Some(index) = window[..searchable].iter().rposition(|byte| *byte == b'\n') {
                break index + 1;
            }
            if window_start == 0 {
                // No boundary to the left: this is the file's first record.
                break 0;
            }
            let take = block_len(window_start);
            let start = window_start - take as u64;
            let mut prefix = vec![0u8; take];
            fill_exact(file, session_id, start, &mut prefix)?;
            prefix.append(&mut window);
            window = prefix;
            window_start = start;
        };

        let line = &window[line_start_in_window..window.len() - 1];
        let with_line = consumed_bytes.saturating_add(line.len());
        if consumed > 0 && with_line > PAGE_BYTE_CAP {
            page.stopped_by = PageBound::Bytes;
            break;
        }

        match decode_record(line) {
            Some(record) => page.records.push(record),
            // One record this build cannot read is counted and skipped. It still
            // costs the page's bounds, because what those bound is the work the
            // scan does, not how much of it the viewer can print.
            None => page.malformed_lines += 1,
        }
        consumed += 1;
        consumed_bytes = with_line;
        resolved_from = window_start + line_start_in_window as u64;
        page.page_start_offset = resolved_from;
        // Ends on this record's own terminator, or empty at offset zero, so the
        // window keeps its invariant either way.
        window.truncate(line_start_in_window);
    }

    page.records.reverse();
    Ok(page)
}

/// The file name this build generates for a session: `<session-id>.jsonl`,
/// exactly one path component.
///
/// Fails for a session id this build could not have produced, so a caller can
/// never turn a strange id into a strange path.
pub fn archive_file_name(session_id: &str) -> AppResult<String> {
    if !is_generated_session_id(session_id) {
        return Err(invalid(format!(
            "Session id {session_id:?} is not one RunCove generated, so it has no archive file name"
        )));
    }
    Ok(format!("{session_id}.{ARCHIVE_FILE_EXTENSION}"))
}

/// Whether `name` is a name this build's [`archive_file_name`] could have
/// produced.
///
/// One predicate answers both questions the archive asks: may I use this
/// `file_name` from the database, and did this build generate this directory
/// entry? It matches the generator rather than blocking a list of bad shapes, so
/// a Windows name this build never writes stays rejected without anyone having
/// to think of it first: absolute and drive-relative paths, `.` and `..`, any
/// name carrying a separator, a trailing dot or space, an alternate data stream,
/// a reserved device name, and anything whose stem is not a lowercase hyphenated
/// UUID or whose extension is not `jsonl`.
pub fn is_archive_file_name(name: &str) -> bool {
    archive_file_stem(name).is_some_and(is_generated_session_id)
}

/// What is left of `name` once the one extension this build generates is
/// stripped, or `None` when `name` does not carry it.
///
/// Two strips rather than one formatted suffix: the extension is a constant, and
/// the dot has to be there rather than being part of the stem. Whether that stem
/// is a session id this build generates is [`is_archive_file_name`]'s question,
/// so a caller that has already asked it reads the id straight back from here
/// instead of re-deriving it and risking a second, different rule.
fn archive_file_stem(name: &str) -> Option<&str> {
    name.strip_suffix(ARCHIVE_FILE_EXTENSION)
        .and_then(|rest| rest.strip_suffix('.'))
}

/// Whether `id` is a session id this build generates: a lowercase hyphenated
/// UUID, exactly as [`uuid::Uuid::new_v4`] renders one and as
/// [`crate::storage`] stores it.
///
/// The shape is checked and the version and variant nibbles deliberately are
/// not. The shape is what keeps a generated name to one harmless path component,
/// because it admits no separator, colon, dot, space, or `..`; pinning the
/// version would add nothing to that, and would turn every archive already on
/// disk into an unreadable orphan the day the id generator moved to another UUID
/// version.
fn is_generated_session_id(id: &str) -> bool {
    /// 8-4-4-4-12 lowercase hex digits, with four hyphens.
    const UUID_TEXT_LENGTH: usize = 36;

    id.len() == UUID_TEXT_LENGTH
        && id.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => matches!(byte, b'0'..=b'9' | b'a'..=b'f'),
        })
}

/// `archive_dir.join(file_name)` once `file_name` has passed
/// [`is_archive_file_name`], with the joined path's parent confirmed to be
/// `archive_dir`.
///
/// Every read, delete, and sweep goes through here, so no caller can reach a
/// path outside the archive directory even if the database says otherwise.
pub fn resolve_archive_path(archive_dir: &Path, file_name: &str) -> AppResult<PathBuf> {
    if !is_archive_file_name(file_name) {
        // The name is not echoed back: a rejected one may itself be a path, and
        // what the caller needs is the rule, not the value.
        return Err(invalid(
            "An archive file name must be \"<session-id>.jsonl\" and nothing else",
        ));
    }

    let path = archive_dir.join(file_name);
    if path.parent() != Some(archive_dir) {
        return Err(invalid(
            "An archive file must be a direct child of the archive directory",
        ));
    }
    Ok(path)
}

/// The gate read, delete, and sweep share: a resolved path that is an ordinary
/// file, plus its length in bytes.
///
/// A symbolic link, junction, or any other reparse point is refused rather than
/// followed, and a directory named like an archive is refused too. The cost is
/// that a cloud-storage placeholder is also refused; RunCove reports it instead
/// of reading through it.
pub fn resolve_ordinary_archive_file(
    fs: &dyn ArchiveFs,
    archive_dir: &Path,
    file_name: &str,
) -> AppResult<(PathBuf, u64)> {
    let path = resolve_archive_path(archive_dir, file_name)?;
    // Non-following metadata, so what is reported is the entry itself and never
    // whatever it points at.
    let info = fs.entry_info(&path)?;
    match info.kind {
        EntryKind::File => Ok((path, info.len)),
        // The name has already passed the rule above, so echoing it cannot echo a
        // path.
        EntryKind::Directory => Err(invalid(format!(
            "Archive entry {file_name} is a directory, not a file"
        ))),
        EntryKind::ReparsePoint => Err(invalid(format!(
            "Archive entry {file_name} is a reparse point, which RunCove reports instead of reading"
        ))),
    }
}

/// What one directory entry is, decided without following links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    /// A symbolic link, a junction, or anything else Windows marks with
    /// `FILE_ATTRIBUTE_REPARSE_POINT`.
    ReparsePoint,
}

/// One entry of the archive directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub name: String,
    pub kind: EntryKind,
    pub len: u64,
}

/// One entry whose metadata this build could not read: a file another process
/// holds exclusively, one whose permissions deny this user, a failing disk.
///
/// The name still arrives, because the sweep needs it to say which entry it is
/// reporting and to look for a row that remembers the entry's last known size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableEntry {
    pub name: String,
    /// Why the read failed, for the anomaly. Never a path.
    pub reason: String,
}

/// What one listed entry is, or why this build could not tell.
///
/// The failure is per entry and is data, not an error: one entry the filesystem
/// refuses must not stop the sweep from repairing every other row. Only a
/// directory that cannot be listed at all is an error.
pub type ListedEntry = Result<DirEntryInfo, UnreadableEntry>;

/// An open archive file. `sync_data` is separate from `flush` so the close path
/// can prove the bytes reached the disk, and so a test can fail exactly that
/// step.
pub trait ArchiveFile: io::Write + Send {
    fn sync_data(&mut self) -> io::Result<()>;
}

/// An archive file open for reading at arbitrary offsets.
///
/// Separate from [`ArchiveFile`] because the two are never the same handle: the
/// writer owns the one it appends through, and a read must not be able to reach
/// it. It is also why the read side has no `len`: the length a page trusts is the
/// one [`resolve_ordinary_archive_file`] measured before the file was opened.
pub trait ArchiveReadFile: Send {
    /// Fill `buf` from `offset`, returning how many bytes it got.
    ///
    /// Short only at the end of the file: an implementation loops over a short
    /// read rather than passing one on, so a caller that asked for bytes it had
    /// already measured can treat a short fill as the file having changed under
    /// it.
    fn fill_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<usize>;
}

/// The filesystem the archive uses. Production gets [`RealArchiveFs`]; the tests
/// substitute one that fails a chosen write or `sync_data`.
pub trait ArchiveFs: Send + Sync {
    fn create_dir_all(&self, dir: &Path) -> io::Result<()>;

    /// The immediate children of `dir`, never recursing, in ascending name order,
    /// classified the same way [`ArchiveFs::entry_info`] classifies one path.
    ///
    /// `Err` is only for a directory that could not be listed at all, which does
    /// stop the sweep. A single entry whose metadata could not be read is
    /// [`UnreadableEntry`] inside the vector, which does not: the sweep reports it
    /// and carries on with the rest of the directory.
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<ListedEntry>>;

    /// Metadata for one path, which must not follow a link.
    ///
    /// On Windows `FileType::is_symlink` is true only for the two name-surrogate
    /// reparse tags — a symbolic link and a mount point — so an implementation
    /// must also test `FILE_ATTRIBUTE_REPARSE_POINT` to keep the promise
    /// [`EntryKind::ReparsePoint`] makes. Otherwise a cloud-storage placeholder,
    /// a deduplicated file, or an `AppExecLink` would be reported as an ordinary
    /// file and read through.
    fn entry_info(&self, path: &Path) -> io::Result<DirEntryInfo>;

    /// Create a new file, failing if one already exists.
    fn create_new(&self, path: &Path) -> io::Result<Box<dyn ArchiveFile>>;

    /// Open an existing file for reading at arbitrary offsets, never creating or
    /// truncating one.
    fn open_read(&self, path: &Path) -> io::Result<Box<dyn ArchiveReadFile>>;

    fn read_to_string(&self, path: &Path) -> io::Result<String>;

    fn remove_file(&self, path: &Path) -> io::Result<()>;
}

/// The `std::fs` implementation the application uses.
pub struct RealArchiveFs;

impl ArchiveFs for RealArchiveFs {
    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        fs::create_dir_all(dir)
    }

    fn list_dir(&self, dir: &Path) -> io::Result<Vec<ListedEntry>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // The same call `entry_info` makes, so listing a directory and stating
            // one path inside it cannot disagree about what an entry is.
            entries.push(match fs::symlink_metadata(entry.path()) {
                Ok(metadata) => Ok(DirEntryInfo {
                    name,
                    kind: entry_kind(&metadata),
                    len: metadata.len(),
                }),
                // One entry the filesystem refuses is data, not an error: it must
                // not cost every other row in this directory its repair. The
                // message a metadata call fails with carries the reason, not a path.
                Err(error) => Err(UnreadableEntry {
                    name,
                    reason: error.to_string(),
                }),
            });
        }
        entries.sort_by(|left, right| listed_entry_name(left).cmp(listed_entry_name(right)));
        Ok(entries)
    }

    fn entry_info(&self, path: &Path) -> io::Result<DirEntryInfo> {
        let metadata = fs::symlink_metadata(path)?;
        Ok(DirEntryInfo {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            kind: entry_kind(&metadata),
            len: metadata.len(),
        })
    }

    fn create_new(&self, path: &Path) -> io::Result<Box<dyn ArchiveFile>> {
        let inner = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        Ok(Box::new(RealArchiveFile { inner }))
    }

    fn open_read(&self, path: &Path) -> io::Result<Box<dyn ArchiveReadFile>> {
        Ok(Box::new(RealArchiveReadFile {
            inner: fs::File::open(path)?,
        }))
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
}
/// What one entry is, decided from non-following metadata so a link is reported
/// as a link instead of as whatever it points at.
///
/// The attribute bit is tested first because on Windows `FileType::is_symlink` is
/// true only for the two name-surrogate reparse tags — a symbolic link and a
/// mount point — while [`EntryKind::ReparsePoint`] promises every tag, including a
/// cloud-storage placeholder, a deduplicated file, and an `AppExecLink`.
fn entry_kind(metadata: &fs::Metadata) -> EntryKind {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        // `FILE_ATTRIBUTE_REPARSE_POINT`.
        const REPARSE_POINT: u32 = 0x400;

        if metadata.file_attributes() & REPARSE_POINT != 0 {
            return EntryKind::ReparsePoint;
        }
    }

    if metadata.file_type().is_symlink() {
        EntryKind::ReparsePoint
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else {
        EntryKind::File
    }
}

/// The name of a listed entry, readable or not, so a listing can be ordered by
/// name without caring which it is.
fn listed_entry_name(entry: &ListedEntry) -> &str {
    match entry {
        Ok(info) => &info.name,
        Err(unreadable) => &unreadable.name,
    }
}

/// One open archive file.
///
/// Unbuffered on purpose: [`WRITE_BUFFER_BYTES`] belongs to [`ArchiveWriter`],
/// which owns every open file, so this seam and the one the tests substitute
/// deliver the same writes in the same order.
struct RealArchiveFile {
    inner: fs::File,
}

impl io::Write for RealArchiveFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl ArchiveFile for RealArchiveFile {
    fn sync_data(&mut self) -> io::Result<()> {
        self.inner.sync_data()
    }
}

/// One archive file open for reading.
///
/// Seek-then-read rather than a positional read, because the two positional APIs
/// are per-platform and this handle is never shared between threads: the page
/// that opened it owns it until it is done.
struct RealArchiveReadFile {
    inner: fs::File,
}

impl ArchiveReadFile for RealArchiveReadFile {
    fn fill_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.seek(io::SeekFrom::Start(offset))?;

        // Loop, because a short read is legal and only the end of the file may
        // make a fill short. `Interrupted` is retried for the same reason
        // `Read::read_exact` retries it.
        let mut filled = 0;
        while filled < buf.len() {
            match self.inner.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(taken) => filled += taken,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(filled)
    }
}

/// One `run_log_archives` row as the writer reads it back.
///
/// `status` and `reason` are strings for the same reason
/// [`crate::models::RunLogArchiveSummary`] uses strings: a database written by a
/// newer build may carry values this build does not know, and the sweep must
/// report such a row instead of failing to read the whole index. Writes go the
/// other way and always use this build's enums.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveRow {
    pub session_id: String,
    pub file_name: String,
    pub status: String,
    pub reason: Option<String>,
    pub counters: ArchiveCounters,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

/// The file name a row is allowed to name, or an error.
///
/// A row is data out of a database this build does not exclusively own, so the
/// name it carries is checked against the name this build would have generated for
/// that session instead of being trusted. Read and delete both go through here, so
/// a row naming another session's archive — or anything outside the archive
/// directory — cannot reach the filesystem through either path, and neither call
/// needs a file name from its caller.
pub fn verified_file_name(session_id: &str, row: &ArchiveRow) -> AppResult<String> {
    // The id comes first: one this build could not have produced has no valid name
    // at all, whatever the row carries.
    let expected = archive_file_name(session_id)?;
    if row.session_id != session_id {
        return Err(invalid(format!(
            "This archive row belongs to another session, not to {session_id}"
        )));
    }
    if row.file_name != expected {
        // The row's name is not echoed: it is exactly the value that may be a path.
        return Err(invalid(format!(
            "The archive row for session {session_id} does not name that session's own file"
        )));
    }
    Ok(expected)
}

/// The index rows the archive owns. Step 4b implements this over [`Storage`] in
/// one place; keeping it a trait here is what lets the writer tests observe the
/// row transitions without asserting SQL.
///
/// [`Storage`]: crate::storage::Storage
pub trait ArchiveIndex: Send + Sync {
    /// Insert the `writing` row. It exists before the first record reaches the
    /// file, so an interrupted session is always visible to the next sweep.
    fn insert_writing(&self, session_id: &str, file_name: &str, started_at: i64) -> AppResult<()>;

    fn update_counters(&self, session_id: &str, counters: ArchiveCounters) -> AppResult<()>;

    /// Move a `writing` row to `complete` or `partial`.
    fn close(
        &self,
        session_id: &str,
        status: ArchiveStatus,
        reason: Option<ArchiveReason>,
        counters: ArchiveCounters,
        ended_at: i64,
    ) -> AppResult<()>;

    /// Move any row to `removed`, for eviction, a user delete, or a file that is
    /// gone.
    fn mark_removed(&self, session_id: &str, reason: ArchiveReason, ended_at: i64)
        -> AppResult<()>;

    fn rows(&self) -> AppResult<Vec<ArchiveRow>>;

    fn row(&self, session_id: &str) -> AppResult<Option<ArchiveRow>>;
}

/// How many bytes the archive directory holds, or the fact that this build could
/// not work it out.
///
/// The quota is a hard byte cap, so a total the sweep had to guess would be worse
/// than no total: under-counting lets the directory grow past the cap silently.
/// [`QuotaTotal::Unavailable`] therefore means "no room" — the archive stops
/// instead of growing a directory it cannot measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaTotal {
    Known(u64),
    /// At least one entry's size could not be determined and no row remembered
    /// it.
    Unavailable,
}

impl Default for QuotaTotal {
    /// A report that has measured nothing has measured zero bytes, which is a
    /// known total and not an unavailable one.
    fn default() -> Self {
        Self::Known(0)
    }
}

/// What one startup sweep changed, for the log and for the tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Sessions whose `writing` row became `partial` / `interrupted`.
    pub repaired_writing: Vec<String>,
    /// Sessions whose row became `removed` / `file-missing`.
    pub marked_file_missing: Vec<String>,
    /// Eligible files with no row, deleted. A delete the filesystem refused is an
    /// anomaly instead, and the file's bytes still count below.
    pub deleted_orphan_files: Vec<String>,
    /// Bytes the quota counter starts from.
    ///
    /// What each entry contributes:
    ///
    /// - an ordinary file: its own length;
    /// - a reparse point: nothing, because the entry is known not to be a file
    ///   this build wrote, so none of our bytes are at that name;
    /// - an entry whose metadata could not be read, with a row: the row's last
    ///   known `byte_size`, which is the only number anyone has;
    /// - an entry whose metadata could not be read, with no row: nothing supplies
    ///   a size, so the total is [`QuotaTotal::Unavailable`];
    /// - an orphan whose delete was refused: its own length, because the bytes are
    ///   still on the disk.
    pub measured_bytes: QuotaTotal,
    /// Everything the sweep refused to touch, reported once and left alone.
    pub anomalies: Vec<String>,
}

/// One entry of the archive directory that this build could have written, as the
/// startup sweep classified it.
///
/// An entry whose name this build could not have generated never reaches this
/// classification: it is reported once and otherwise left entirely alone, because
/// nothing about a name RunCove does not own is RunCove's to read, to measure, or
/// to delete.
enum SweptEntry {
    /// An ordinary file, with its own length in bytes.
    File(u64),
    /// An archive name taken by something this build did not write: a directory,
    /// or a reparse point. Never read, never measured, never deleted.
    NotOurFile,
    /// An entry the filesystem refused to describe, so the disk supplies no length
    /// for it.
    Unmeasurable,
}

/// How a row names itself in an anomaly.
///
/// The `file_name` column is never echoed: it is exactly the value that may be a
/// path rather than a name. The session id is echoed only when it is one this
/// build generates, which is also the only case in which it is known to be a
/// harmless single path component.
fn row_label(row: &ArchiveRow) -> String {
    if is_generated_session_id(&row.session_id) {
        format!("session {}", row.session_id)
    } else {
        "a session id RunCove could not have generated".to_string()
    }
}

/// One startup sweep in progress.
///
/// It exists so the passes below share the handles every one of them needs without
/// threading them through free functions of six arguments each, and so the growing
/// [`SweepReport`] has one owner while they run.
struct Sweep<'a> {
    archive_dir: &'a Path,
    fs: &'a dyn ArchiveFs,
    index: &'a dyn ArchiveIndex,
    now: i64,
    report: SweepReport,
}

impl Sweep<'_> {
    /// Reconcile the archive directory with the index, once, before the first
    /// write of the run.
    ///
    /// The reach is exactly one directory: the immediate children of
    /// `archive_dir`, never recursing into one of them and never leaving it.
    /// Every failure that belongs to a single entry or a single row is reported
    /// and stepped over, because one file the filesystem refuses must not cost
    /// every other row its repair. Only two failures stop the sweep: an index
    /// that cannot be read at all, and a directory that cannot be listed at all.
    fn run(mut self) -> AppResult<SweepReport> {
        // Both sides are read before either is changed: the rows say which files
        // are still remembered, the listing says which of them are still there.
        let rows = self.index.rows()?;
        let entries = self.classify_entries()?;
        let owned = self.owned_rows(&rows);

        // The rows first, so an interrupted session is repaired from its own file
        // before the passes below decide what no row remembers.
        for (file_name, row) in &owned {
            self.reconcile_row(file_name, row, entries.get(file_name));
        }

        let known: BTreeSet<&str> = rows.iter().map(|row| row.session_id.as_str()).collect();
        self.delete_orphans(&entries, &known);

        let measured = self.measure(&entries, &owned);
        self.report.measured_bytes = measured;
        Ok(self.report)
    }

    /// Classify every immediate child of the archive directory, by name first.
    ///
    /// A name this build's [`archive_file_name`] could not have produced is
    /// reported and dropped here, readable or not: it is not RunCove's file, so
    /// its kind and its size are not RunCove's business either.
    fn classify_entries(&mut self) -> AppResult<BTreeMap<String, SweptEntry>> {
        // A directory that cannot be listed at all is the one entry-side failure
        // that stops the sweep: with no listing there is nothing to reconcile the
        // rows against, and every row would look like a file that had gone.
        let listed = self.fs.list_dir(self.archive_dir).map_err(|error| {
            invalid(format!(
                "Could not list the run log archive directory: {error}"
            ))
        })?;

        let mut entries = BTreeMap::new();
        for entry in listed {
            match entry {
                Ok(info) if is_archive_file_name(&info.name) => {
                    let classified = match info.kind {
                        EntryKind::File => SweptEntry::File(info.len),
                        EntryKind::Directory => {
                            self.report.anomalies.push(format!(
                                "Archive entry {} is a directory, not a file this build wrote",
                                info.name
                            ));
                            SweptEntry::NotOurFile
                        }
                        EntryKind::ReparsePoint => {
                            self.report.anomalies.push(format!(
                                "Archive entry {} is a reparse point, which RunCove reports instead of reading, measuring, or deleting",
                                info.name
                            ));
                            SweptEntry::NotOurFile
                        }
                    };
                    entries.insert(info.name, classified);
                }
                Ok(info) => self.report.anomalies.push(format!(
                    "Archive directory entry {} is not a name RunCove generates, so RunCove left it alone",
                    info.name
                )),
                Err(unreadable) if is_archive_file_name(&unreadable.name) => {
                    self.report.anomalies.push(format!(
                        "Archive entry {} could not be measured: {}",
                        unreadable.name, unreadable.reason
                    ));
                    entries.insert(unreadable.name, SweptEntry::Unmeasurable);
                }
                Err(unreadable) => self.report.anomalies.push(format!(
                    "Archive directory entry {} is not a name RunCove generates and could not be measured: {}",
                    unreadable.name, unreadable.reason
                )),
            }
        }
        Ok(entries)
    }

    /// The rows the sweep is willing to act on, keyed by the file name each one
    /// owns.
    ///
    /// A row is data out of a database this build does not own exclusively, so the
    /// name it carries is checked against the one its own session generates before
    /// anything is done with it. A row that fails is reported and left exactly as
    /// it is: not repaired, not measured, not marked. That is also what keeps a
    /// `file_name` like `..\other.jsonl` from ever reaching a path here.
    fn owned_rows<'r>(&mut self, rows: &'r [ArchiveRow]) -> BTreeMap<String, &'r ArchiveRow> {
        let mut owned = BTreeMap::new();
        for row in rows {
            match verified_file_name(&row.session_id, row) {
                Ok(file_name) => {
                    owned.insert(file_name, row);
                }
                Err(_) => self.report.anomalies.push(format!(
                    "The archive row for {} does not name that session's own file, so RunCove left it alone",
                    row_label(row)
                )),
            }
        }
        owned
    }

    /// Bring one row back in line with what is actually on the disk.
    fn reconcile_row(&mut self, file_name: &str, row: &ArchiveRow, entry: Option<&SweptEntry>) {
        // The status is parsed before anything is decided, so a row written by a
        // build that knows more states than this one is reported rather than
        // repaired by guessing. In particular, an unknown status is never taken for
        // a session whose file has gone missing.
        let Some(status) = ArchiveStatus::parse(&row.status) else {
            self.report.anomalies.push(format!(
                "The archive row for {} carries status {:?}, which this build does not know, so RunCove left it alone",
                row_label(row),
                row.status
            ));
            return;
        };

        match (status, entry) {
            // A session the last run was still writing when it stopped. It is closed
            // with what its file actually holds, never with what the stale row
            // claimed, because the row stopped being updated at the moment the run
            // died and the file did not.
            (ArchiveStatus::Writing, Some(SweptEntry::File(len))) => {
                let counters = ArchiveCounters {
                    line_count: self.count_lines(file_name, row),
                    byte_size: i64::try_from(*len).unwrap_or(i64::MAX),
                    ..row.counters
                };
                self.repair(row, counters);
            }
            // The name is taken by something this build did not write, so none of
            // that session's bytes are there to count. Already reported once.
            (ArchiveStatus::Writing, Some(SweptEntry::NotOurFile)) => {
                let counters = ArchiveCounters {
                    line_count: 0,
                    byte_size: 0,
                    ..row.counters
                };
                self.repair(row, counters);
            }
            // The file is there and the filesystem will not describe it, so the
            // row's own numbers are the last anyone knows. Already reported once.
            (ArchiveStatus::Writing, Some(SweptEntry::Unmeasurable)) => {
                self.repair(row, row.counters);
            }
            // A row whose file is gone: deleted behind RunCove's back, by a user or
            // by a cleaner. The row is kept and marked, not dropped, so the reason
            // the archive disappeared survives for the user to see.
            (ArchiveStatus::Writing | ArchiveStatus::Complete | ArchiveStatus::Partial, None) => {
                self.mark_file_missing(row)
            }
            // An ended row whose file is still there: nothing to repair, and
            // nothing to rewrite either. Re-measuring a row that was closed on
            // purpose would overwrite a final number with a guess.
            (ArchiveStatus::Complete | ArchiveStatus::Partial, Some(_)) => {}
            (ArchiveStatus::Removed, Some(SweptEntry::File(_))) => {
                self.report.anomalies.push(format!(
                    "The archive of {} is recorded as removed and a file is still at its name, so RunCove left it alone",
                    row_label(row)
                ));
            }
            (ArchiveStatus::Removed, _) => {}
        }
    }

    /// How many lines the archive at `file_name` actually holds.
    ///
    /// A recount rather than a trust: the row's `line_count` is what the
    /// interrupted run had managed to record before it died, and the file is what
    /// survived. Reading it whole is what [`ArchiveFs`] offers and what the
    /// per-session byte cap bounds, and the text is dropped as soon as it is
    /// counted. A read that fails leaves the row's own count, which is the only
    /// other number anyone has.
    fn count_lines(&mut self, file_name: &str, row: &ArchiveRow) -> i64 {
        // The name came out of `verified_file_name`, so this cannot fail; asking
        // anyway is what keeps every path this module opens provably inside the
        // archive directory, with no second rule to keep in step.
        let path = match resolve_archive_path(self.archive_dir, file_name) {
            Ok(path) => path,
            Err(error) => {
                self.report.anomalies.push(format!(
                    "The archive of {} could not be resolved to a path inside the archive directory: {error}",
                    row_label(row)
                ));
                return row.counters.line_count;
            }
        };

        match self.fs.read_to_string(&path) {
            Ok(text) => i64::try_from(text.lines().count()).unwrap_or(i64::MAX),
            Err(error) => {
                self.report.anomalies.push(format!(
                    "The archive of {} could not be read, so its recorded line count is kept: {error}",
                    row_label(row)
                ));
                row.counters.line_count
            }
        }
    }

    /// Close a row the last run left open, as `partial` / `interrupted`.
    ///
    /// An index write that fails here is reported and stepped over: the row stays
    /// `writing`, which is exactly the state this sweep repairs, so the next
    /// startup tries once more. Nothing is retried in a loop and nothing is lost.
    fn repair(&mut self, row: &ArchiveRow, counters: ArchiveCounters) {
        match self.index.close(
            &row.session_id,
            ArchiveStatus::Partial,
            Some(ArchiveReason::Interrupted),
            counters,
            self.now,
        ) {
            Ok(()) => self.report.repaired_writing.push(row.session_id.clone()),
            Err(error) => self.report.anomalies.push(format!(
                "The archive row for {} could not be repaired and stays open for the next sweep: {error}",
                row_label(row)
            )),
        }
    }

    /// Mark a row whose file is no longer there.
    fn mark_file_missing(&mut self, row: &ArchiveRow) {
        match self
            .index
            .mark_removed(&row.session_id, ArchiveReason::FileMissing, self.now)
        {
            Ok(()) => self.report.marked_file_missing.push(row.session_id.clone()),
            Err(error) => self.report.anomalies.push(format!(
                "The archive row for {} could not be marked as having lost its file: {error}",
                row_label(row)
            )),
        }
    }

    /// Delete every eligible file no row remembers.
    ///
    /// Eligible means exactly this: an ordinary file, under a name this build
    /// generates, resolving inside the archive directory, and with no row at all
    /// for that session — whatever such a row would have said. A file some row
    /// still names is never deleted here, not even when the sweep refused to act
    /// on that row: reporting a strange row costs a line in a log, and guessing at
    /// one costs a user their log.
    fn delete_orphans(&mut self, entries: &BTreeMap<String, SweptEntry>, known: &BTreeSet<&str>) {
        for (file_name, entry) in entries {
            let SweptEntry::File(_) = entry else {
                continue;
            };
            // The name is in this map because it passed the file name rule, so its
            // stem is the session id whose archive it claims to be.
            let Some(session_id) = archive_file_stem(file_name) else {
                continue;
            };
            if known.contains(session_id) {
                continue;
            }

            let path = match resolve_archive_path(self.archive_dir, file_name) {
                Ok(path) => path,
                Err(error) => {
                    self.report.anomalies.push(format!(
                        "Archive file {file_name} has no row and could not be resolved to a path inside the archive directory: {error}"
                    ));
                    continue;
                }
            };
            match self.fs.remove_file(&path) {
                Ok(()) => self.report.deleted_orphan_files.push(file_name.clone()),
                Err(error) => self.report.anomalies.push(format!(
                    "Archive file {file_name} has no row and could not be deleted, so its bytes still count towards the quota: {error}"
                )),
            }
        }
    }

    /// The byte total the quota counter starts from.
    ///
    /// [`SweepReport::measured_bytes`] states what each entry contributes and why.
    /// One entry nobody can size, and that no row remembers, makes the whole total
    /// [`QuotaTotal::Unavailable`]: a total this build had to guess at is worse
    /// than no total, because the cap it feeds is the only thing standing between
    /// an opt-in feature and a full disk.
    fn measure(
        &self,
        entries: &BTreeMap<String, SweptEntry>,
        owned: &BTreeMap<String, &ArchiveRow>,
    ) -> QuotaTotal {
        let mut total: u64 = 0;
        for (file_name, entry) in entries {
            match entry {
                // A file this sweep deleted is gone, so its bytes are not the
                // quota's any more.
                SweptEntry::File(_) if self.report.deleted_orphan_files.contains(file_name) => {}
                SweptEntry::File(len) => total = total.saturating_add(*len),
                SweptEntry::NotOurFile => {}
                SweptEntry::Unmeasurable => match owned.get(file_name) {
                    Some(row) => {
                        total = total
                            .saturating_add(u64::try_from(row.counters.byte_size).unwrap_or(0));
                    }
                    None => return QuotaTotal::Unavailable,
                },
            }
        }
        QuotaTotal::Known(total)
    }
}

/// The bounded hand-off from the capture threads to the writer.
///
/// Bounded four ways at once — records and bytes, per session and in total — so
/// neither one noisy session nor a hundred quiet ones can grow it without limit.
/// When a bound refuses a record the queue counts the loss instead of blocking
/// the capture thread or giving up a line it already holds.
///
/// It also owns the drop history, which outlives the records: what a session has
/// lost is what its row's counters are made of, so handing records to a pump
/// hands over records only and never the history. The history outlives the
/// records but not the session — [`ArchiveQueue::finish_session`] hands it over
/// once and forgets it, which is what bounds the queue by the sessions that are
/// open rather than by every session this process has run.
///
/// The queue owns every record from [`ArchiveQueue::enqueue`] until it is
/// settled, and a settle is either [`ArchiveQueue::release`] — written — or
/// [`ArchiveQueue::discard`] — lost, and counted. Nothing else frees a record's
/// room. That is what makes a failed pump retryable: the records it had claimed
/// are still here, in the same order, and the next
/// [`ArchiveQueue::begin_batch`] appends what arrived meanwhile *behind* them,
/// so the retry resumes at the same record with no undo step anywhere.
pub struct ArchiveQueue {
    bounds: QueueBounds,
    /// Records no pump has claimed yet, in arrival order. Claimed all at once by
    /// [`ArchiveQueue::begin_batch`], which is why a `Vec` is enough here.
    queued: Vec<QueuedRecord>,
    /// The records a pump has claimed and not yet settled, in arrival order, plus
    /// the ones a failed pump left behind. Taken from the front, one at a time.
    in_flight: VecDeque<QueuedRecord>,
    /// Records this queue is still holding room for: queued, in flight, and taken
    /// but not yet settled. This is the number the bounds are asked about, so a
    /// batch in flight cannot be overrun by fresh arrivals.
    reserved: usize,
    /// Bytes of archived text reserved, on the same three counts. Kept rather
    /// than summed on demand: a capture thread asks the bounds a question on every
    /// single line.
    bytes: usize,
    /// Per-session accounting for the sessions that have not ended. An entry is
    /// created by a session's first record or its first drop, survives every
    /// settle because the drop history has to answer after one and after the last
    /// record too, and is removed only by [`ArchiveQueue::finish_session`].
    sessions: BTreeMap<String, SessionQueue>,
}

/// One session's share of the queue.
#[derive(Debug, Clone, Copy, Default)]
struct SessionQueue {
    /// Records this session currently has reserved. Freed one at a time, as each
    /// is settled.
    records: usize,
    /// Bytes of archived text this session currently has reserved. Freed with the
    /// record they belong to.
    bytes: usize,
    /// Lost since this session's last accepted record, and not yet handed to
    /// anyone: the counters the next accepted record carries, or the residual
    /// [`ArchiveQueue::finish_session`] hands back if no further record arrives.
    pending: DropCounters,
    /// Everything this session has lost. Never cleared while the session lasts —
    /// taking a gap says where a loss is reported, not that it stopped happening
    /// — and read out one last time when the session is finished.
    dropped: DropCounters,
}

impl SessionQueue {
    /// The pending gap, cleared. `None` when nothing is pending: an empty gap is
    /// not a gap, so it is never carried and never written.
    fn take_pending(&mut self) -> Option<DropCounters> {
        if self.pending == DropCounters::default() {
            return None;
        }
        Some(std::mem::take(&mut self.pending))
    }
}

/// The queue bounds. Production uses [`QueueBounds::default`], the documented
/// numbers; a test uses small ones so an overflow can be reached in a few
/// records instead of a few thousand. This is a test seam, not a user setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueBounds {
    pub session_records: usize,
    pub session_bytes: usize,
    pub total_records: usize,
    pub total_bytes: usize,
}

impl Default for QueueBounds {
    fn default() -> Self {
        Self {
            session_records: SESSION_QUEUE_RECORDS,
            session_bytes: SESSION_QUEUE_BYTES,
            total_records: TOTAL_QUEUE_RECORDS,
            total_bytes: TOTAL_QUEUE_BYTES,
        }
    }
}

/// The byte caps the writer enforces. Production uses [`QuotaLimits::default`],
/// the documented caps; a test uses small ones so a cap can be crossed without
/// writing hundreds of megabytes. This is a test seam, not a user setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaLimits {
    pub session_bytes: u64,
    pub total_bytes: u64,
}

impl Default for QuotaLimits {
    fn default() -> Self {
        Self {
            session_bytes: SESSION_BYTE_CAP,
            total_bytes: TOTAL_BYTE_CAP,
        }
    }
}

impl Default for ArchiveQueue {
    fn default() -> Self {
        Self::new(QueueBounds::default())
    }
}

impl ArchiveQueue {
    pub fn new(bounds: QueueBounds) -> Self {
        Self {
            bounds,
            queued: Vec::new(),
            in_flight: VecDeque::new(),
            reserved: 0,
            bytes: 0,
            sessions: BTreeMap::new(),
        }
    }
    /// `false` when the record was dropped. The incoming record is always the
    /// one dropped, so a session never loses a line it already queued.
    ///
    /// A drop joins the session's pending gap. An accepted record takes that
    /// pending gap with it as [`QueuedRecord::gap_before`], which is what fixes
    /// the marker's place in the file: the loss is written immediately before the
    /// first line that survived it.
    pub fn enqueue(&mut self, record: ArchiveRecord) -> bool {
        // Bytes are the archived text, not the JSON encoding around it: the
        // bound has to mean the same thing as the counters the row reports.
        let bytes = record.line.len();
        // An entry appears even for a record that is about to be refused, because
        // the loss it causes belongs to this session's history.
        let session = self.sessions.entry(record.session_id.clone()).or_default();
        // Reaching a bound exactly still fits; one record or one byte past it does
        // not. Fields, not the accessors, so the borrow of `sessions` above stays
        // disjoint from the two totals. The totals are reservations rather than
        // list lengths, so a batch a pump is still holding takes the room it needs
        // instead of being lent out twice.
        let fits = session.records < self.bounds.session_records
            && session.bytes + bytes <= self.bounds.session_bytes
            && self.reserved < self.bounds.total_records
            && self.bytes + bytes <= self.bounds.total_bytes;
        if !fits {
            session.pending.record_loss(bytes);
            session.dropped.record_loss(bytes);
            return false;
        }
        session.records += 1;
        session.bytes += bytes;
        let gap_before = session.take_pending();
        self.reserved += 1;
        self.bytes += bytes;
        self.queued.push(QueuedRecord { record, gap_before });
        true
    }

    /// Records this queue is holding room for: queued, in flight, and taken but
    /// not yet settled.
    pub fn len(&self) -> usize {
        self.reserved
    }

    pub fn is_empty(&self) -> bool {
        self.reserved == 0
    }

    /// Bytes of archived text this queue is holding room for, on the same three
    /// counts as [`ArchiveQueue::len`].
    pub fn queued_bytes(&self) -> usize {
        self.bytes
    }

    /// Move everything queued into the in-flight list, behind whatever a previous
    /// pump left there.
    ///
    /// Nothing is freed and nothing is handed out. The queue keeps owning every
    /// record until it is settled, which is what makes a failed pump retryable: the
    /// records are still here, still in arrival order, and still charged against
    /// every bound. A pump that returns an error leaves the unsettled ones in front,
    /// so the next `begin_batch` appends behind them and the next pump retries from
    /// the same place.
    ///
    /// In-flight records count against all four bounds exactly as queued ones do.
    /// That is the whole point: a record whose fate is undecided is still occupying
    /// memory, so admitting more in its place would make the queue unbounded during
    /// a run of failures.
    pub fn begin_batch(&mut self) {
        self.in_flight.extend(self.queued.drain(..));
    }

    /// The front in-flight record, for the pump that is about to decide what it
    /// costs.
    ///
    /// Module-private and by reference, because the encoded cost of a record — what
    /// the quota is actually asked about — is not the archived-text length
    /// [`ArchiveQueue::peek_front`] reports, and encoding it must happen while the
    /// record is still owned by the queue.
    fn front(&self) -> Option<&QueuedRecord> {
        self.in_flight.front()
    }

    /// The front in-flight record's session and archived-text length, without
    /// taking it out — what a pump needs to decide about room before it commits to
    /// writing anything.
    pub fn peek_front(&self) -> Option<(&str, usize)> {
        self.front()
            .map(|item| (item.record.session_id.as_str(), item.record.line.len()))
    }

    /// Take the front in-flight record, which the caller must settle with exactly
    /// one of [`ArchiveQueue::release`] and [`ArchiveQueue::discard`].
    ///
    /// The window between taking and settling is the one place a record is owned by
    /// the pump rather than by the queue, so it is kept as narrow as the write
    /// itself: everything that can fail retryably — the quota, the eviction, the
    /// index — is done while the record is still in the queue, and the only failure
    /// that can happen after the take is the write, which settles as a
    /// [`ArchiveQueue::discard`] rather than a retry.
    pub fn take_front(&mut self) -> Option<QueuedRecord> {
        self.in_flight.pop_front()
    }

    /// The record is on the disk: its room comes back and nothing is lost.
    pub fn release(&mut self, record: &QueuedRecord) {
        self.free(&record.record.session_id, record.record.line.len());
    }

    /// The record will never be written: its room comes back and it is charged as a
    /// loss.
    ///
    /// The carried [`QueuedRecord::gap_before`] returns to the session's `pending`
    /// and to nothing else. It was already added to the cumulative `dropped` when
    /// [`ArchiveQueue::enqueue`] refused the lines it counts, so adding it again
    /// here would report the same loss twice; putting it back in `pending` is what
    /// keeps it attached to the next record that survives, or to the residual gap.
    /// The discarded record itself is a new loss, so it joins both.
    pub fn discard(&mut self, record: QueuedRecord) {
        self.charge_loss(&record);
        self.free(&record.record.session_id, record.record.line.len());
    }

    /// Charge every in-flight record of this session as a loss, in arrival order,
    /// and leave every other session's alone.
    ///
    /// This is what a session dying mid-batch needs: its file is the thing that just
    /// failed, so its remaining records can never be written, and
    /// [`ArchiveQueue::finish_session`] cannot complete while they are still counted
    /// against it.
    ///
    /// Both lists, not only the in-flight one: a record that arrived while the pump
    /// was inside the failing write is queued rather than in flight, and it is just
    /// as unwritable as the rest. Leaving it would strand it — no pump can write a
    /// session that is gone, and `finish_session` would refuse forever.
    pub fn discard_session(&mut self, session_id: &str) {
        for item in self.take_session(session_id) {
            self.discard(item);
        }
    }

    /// Take every record this session has accepted out of both lists, in arrival
    /// order, without freeing anything.
    ///
    /// The in-flight list first and the queued one second, which *is* arrival order:
    /// what a pump had already claimed came in before what arrived behind it.
    ///
    /// Nothing is settled here, exactly as [`ArchiveQueue::take_front`] settles
    /// nothing — the records leave the lists still holding their room, and the caller
    /// owes each one a [`ArchiveQueue::release`] or a [`ArchiveQueue::discard`]. Until
    /// it pays, [`ArchiveQueue::finish_session`] refuses the session, which is what
    /// makes forgetting a record impossible rather than merely unlikely.
    ///
    /// Two callers, and they differ in exactly one thing. A close the writer itself
    /// starts has a file it can no longer write, so `discard_session` charges the lot;
    /// the caller-facing [`ArchiveWriter::close`] is going to write them, so it
    /// releases the ones that land and discards only the rest.
    fn take_session(&mut self, session_id: &str) -> Vec<QueuedRecord> {
        let mut taken: Vec<QueuedRecord> = Vec::new();
        let mut kept: VecDeque<QueuedRecord> = VecDeque::with_capacity(self.in_flight.len());
        for item in std::mem::take(&mut self.in_flight) {
            if item.record.session_id == session_id {
                taken.push(item);
            } else {
                kept.push_back(item);
            }
        }
        self.in_flight = kept;
        let mut still_queued: Vec<QueuedRecord> = Vec::with_capacity(self.queued.len());
        for item in std::mem::take(&mut self.queued) {
            if item.record.session_id == session_id {
                taken.push(item);
            } else {
                still_queued.push(item);
            }
        }
        self.queued = still_queued;
        taken
    }

    /// Count one record that will never be written, exactly as
    /// [`ArchiveQueue::enqueue`] counts one it refuses.
    fn charge_loss(&mut self, item: &QueuedRecord) {
        let bytes = item.record.line.len();
        let session = self
            .sessions
            .entry(item.record.session_id.clone())
            .or_default();
        if let Some(carried) = item.gap_before {
            session.pending.absorb(carried);
        }
        session.pending.record_loss(bytes);
        session.dropped.record_loss(bytes);
    }

    /// Give one settled record's room back, to its session and to the totals.
    ///
    /// The session's entry is looked up rather than created: a record settling for a
    /// session the queue has already finished has nothing to give room back to, and
    /// creating an entry here would leave one behind that nothing removes.
    fn free(&mut self, session_id: &str, bytes: usize) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.records = session.records.saturating_sub(1);
            session.bytes = session.bytes.saturating_sub(bytes);
        }
        self.reserved = self.reserved.saturating_sub(1);
        self.bytes = self.bytes.saturating_sub(bytes);
    }

    /// Take what `session_id` has lost with no accepted record behind it — the
    /// residual, for a caller that wants it while the session is still running.
    ///
    /// A loss that a later record picked up left with that record and is not
    /// owed again here. `None` when nothing is pending.
    ///
    /// Not the close path: closing a session must go through
    /// [`ArchiveQueue::finish_session`], which returns the same residual and also
    /// removes the session's entry. Taking the residual here and closing on that
    /// would leave the entry behind for the life of the process.
    pub fn take_pending_gap(&mut self, session_id: &str) -> Option<DropCounters> {
        self.sessions.get_mut(session_id)?.take_pending()
    }

    /// Everything `session_id` has lost so far.
    pub fn dropped(&self, session_id: &str) -> DropCounters {
        self.sessions
            .get(session_id)
            .map(|session| session.dropped)
            .unwrap_or_default()
    }

    /// End this session's accounting: hand back the residual gap and the
    /// cumulative totals, then forget the session.
    ///
    /// This is what keeps the queue bounded by the sessions that are open rather
    /// than by every session this process has ever run. Nothing else removes an
    /// entry, so [`ArchiveWriter::close`] must come through here — reading the
    /// residual with [`ArchiveQueue::take_pending_gap`] instead would leave the
    /// entry behind forever.
    ///
    /// Refused while the session still has queued records, because those records
    /// have not been written yet: their gaps and their bytes are still owed, and
    /// forgetting the session now would lose both. Pump first, then finish. The
    /// refusal takes nothing, so a caller that pumps and retries loses nothing.
    ///
    /// A session that was never seen, or that was already finished, is owed
    /// nothing and reports no losses — [`FinishedSession::default`]. That is what
    /// makes the gap impossible to write twice.
    fn finish_session(&mut self, session_id: &str) -> AppResult<FinishedSession> {
        let Some(session) = self.sessions.get_mut(session_id) else {
            return Ok(FinishedSession::default());
        };
        // Decided before anything is taken, so a refusal leaves the session exactly
        // as it was and a caller that pumps and retries has lost nothing.
        if session.records > 0 {
            return Err(invalid(format!(
                "Session {session_id} still has {} queued log lines to archive",
                session.records
            )));
        }
        let residual_gap = session.take_pending();
        let dropped = session.dropped;
        self.sessions.remove(session_id);
        Ok(FinishedSession {
            residual_gap,
            dropped,
        })
    }
}

/// One open archive file as the writer holds it: buffered, so a run that writes
/// thousands of short lines does not turn each one into a syscall.
type ArchiveHandle = io::BufWriter<Box<dyn ArchiveFile>>;

/// One session's slot in the writer's open map.
///
/// A slot is taken before the file exists, so a second `begin` for the same
/// session loses without either call reaching the filesystem, and so the taking
/// and the giving back are the only things that need the lock: the file and the
/// index row are done with no lock held.
struct OpenArchive {
    /// What this slot is: taken but not yet created, archiving, or closing.
    state: SlotState,
    /// The handle, once there is one, buffered so a short line does not become a
    /// syscall. `None` before the file exists, and for as long as a pump has
    /// borrowed the handle to write without holding the lock — which is why the
    /// state above, and never this field, is what a late record is judged against.
    file: Option<ArchiveHandle>,
    /// Lines this writer has put in the file, gap lines included, because a gap
    /// line is an archived line like any other.
    lines: i64,
    /// Bytes this writer has handed to the file and charged to the quota total.
    /// The encoding and its newline, which is what the disk holds, and not the
    /// archived text the queue's bounds count.
    bytes: u64,
}

impl OpenArchive {
    /// A slot taken by [`ArchiveWriter::take_slot`], before the file exists.
    fn opening() -> Self {
        Self {
            state: SlotState::Opening,
            file: None,
            lines: 0,
            bytes: 0,
        }
    }
}

/// What one slot in the writer's open map currently is.
///
/// Three states rather than the presence of the handle, because the handle is
/// absent in two entirely different situations — before the file exists, and while
/// a pump is writing through it — and a record arriving must be accepted in the
/// second and not in the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotState {
    /// Between the slot being taken and the file being created. Only a concurrent
    /// `begin` can observe this, and only to lose.
    Opening,
    /// The session is archiving and is accepting records.
    Open,
    /// The session has stopped accepting records and has not finished closing. The
    /// slot stays in the map until the row is written, so the session still counts
    /// as open and a delete is still refused while its file is being finished.
    Closing,
}

/// One record encoded and ready for the file, with the gap line that has to go in
/// front of it already part of the payload.
///
/// The encoding is done under the queue lock and the write is done without it, so
/// this is what crosses that boundary. It carries the cost the quota weighs and the
/// line count the row gets, because both are properties of these bytes rather than
/// of the record they came from: one accepted record is two archived lines when a
/// gap precedes it.
struct PendingWrite {
    session_id: String,
    /// The bytes as they will appear in the file, newline included.
    payload: Vec<u8>,
    /// Archived lines in `payload`: one, or two when it opens with a gap line.
    lines: i64,
}

impl PendingWrite {
    /// What this write would add to the archive directory, in the encoded bytes the
    /// quota counts — never the archived text the queue's bounds count.
    fn cost(&self) -> u64 {
        self.payload.len() as u64
    }
}

/// Encode one queued record for the file, with the gap line it carries in front of it
/// in the same payload.
///
/// The placement is the annotation's whole meaning — the loss happened before this
/// line, not before whichever line the file happens to get next — and one payload
/// makes them one write, so no failure can separate them. The gap line borrows the
/// carrier's own timestamp: this writer reads no clock, and the instant a line was
/// dropped is not an instant anything recorded.
///
/// A free function rather than a method, because both callers hold the record from a
/// different side of the settle protocol: [`ArchiveWriter::pump`] encodes it while the
/// queue still owns it, and [`ArchiveWriter::close`] encodes records it has already
/// taken out.
fn pending_write(item: &QueuedRecord) -> PendingWrite {
    let mut payload = Vec::new();
    let mut lines = 0;
    if let Some(gap) = item.gap_before {
        push_line(
            &mut payload,
            &encode_record(&ArchiveRecord {
                session_id: item.record.session_id.clone(),
                stream: LogStream::System,
                line: gap_line(gap),
                timestamp: item.record.timestamp,
            }),
        );
        lines += 1;
    }
    push_line(&mut payload, &encode_record(&item.record));
    lines += 1;
    PendingWrite {
        session_id: item.record.session_id.clone(),
        payload,
        lines,
    }
}

/// One session as a close takes it over: the handle it must finish, what its row
/// already counts, the records it now owes the disk, and whether the disk has refused.
///
/// It exists because the closing boundary and the file work are deliberately in
/// different critical sections. Everything in here left the writer's state in one
/// atomic step, and everything done to it afterwards happens with no state lock held.
struct ClosingSession {
    /// The session's own handle, taken out of its slot. `None` only for a session
    /// whose handle is already gone, which an `Open` slot cannot be.
    file: Option<ArchiveHandle>,
    /// Archived lines the file holds, this close's own writes included.
    lines: i64,
    /// Encoded bytes this writer has charged for this session, this close's own writes
    /// included. Believed by the row only once the close proves the file durable.
    bytes: u64,
    /// Records the session accepted and this close owes the disk, in arrival order.
    /// Drained by the writing step, which settles every one of them.
    taken: Vec<QueuedRecord>,
    /// Set the moment the disk refuses, and never unset: the file is not asked for
    /// another line, its bytes are measured rather than counted, and the row says
    /// `write-error`.
    failed: bool,
}

/// Fold one more fact into a close's verdict: the worse of the two reasons, or this
/// one when the close had none.
///
/// A close accumulates reasons rather than choosing between them — a session told to
/// stop can also have lost lines, and can also fail its final `sync_data` — and
/// [`ArchiveReason::most_severe`] is a total order, so the answer does not depend on
/// the order they were folded in.
fn worsened(current: Option<ArchiveReason>, addition: ArchiveReason) -> ArchiveReason {
    match current {
        Some(existing) => ArchiveReason::most_severe(existing, addition),
        None => addition,
    }
}

/// Whether the caps leave room for one write.
///
/// Two values rather than a `bool`, so a caller cannot read the answer backwards:
/// the interesting one is refusal, and it is refusal that closes a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Room {
    /// The write fits, either outright or after an eviction that has already run.
    Available,
    /// The write does not fit and nothing more can be freed for it, so the session
    /// it belongs to stops here.
    Full,
}

/// Append one archived line to a payload: the encoding, then the newline that ends
/// it.
///
/// The newline is part of what the file holds, so it is part of what the quota
/// weighs and part of what a reconciliation measures.
fn push_line(payload: &mut Vec<u8>, encoded: &str) {
    payload.extend_from_slice(encoded.as_bytes());
    payload.push(b'\n');
}

/// The one place a buffered handle is given up without writing anything through it.
///
/// `BufWriter`'s own `Drop` attempts a flush, so a handle whose file has already
/// refused a write cannot simply be dropped: that would be a second write to a file
/// this build has just learned it cannot write to. `into_parts` is the documented
/// way to take the inner handle and discard the buffered bytes instead. On the path
/// this exists for the buffer is usually empty anyway, because a record larger than
/// the buffer goes straight to the file.
fn drop_without_flushing(file: ArchiveHandle) {
    let (handle, _discarded) = file.into_parts();
    drop(handle);
}

/// The one owner of every open archive file, of the byte accounting, of eviction,
/// and of the index writes. Capture threads only enqueue.
///
/// It holds the directory it owns, the filesystem and index it was built with, the
/// byte caps it enforces, the running byte total the startup sweep started off, the
/// sessions it has open, and the queue between the capture threads and the disk.
///
/// The writer has no thread of its own here on purpose: [`ArchiveWriter::pump`]
/// does the work in the caller's thread, so a test drives the whole write path
/// with an explicit clock and never sleeps. Step 5 is what parks a thread on top
/// of `pump`.
pub struct ArchiveWriter {
    archive_dir: PathBuf,
    fs: Arc<dyn ArchiveFs>,
    index: Arc<dyn ArchiveIndex>,
    /// The two byte caps: how large one archive may grow, and how large the
    /// directory may grow.
    limits: QuotaLimits,
    /// Bytes the archive directory holds, as this writer accounts for them.
    ///
    /// Behind a mutex because the number is not one thread's: a user delete
    /// arrives on a command thread and gives bytes back, while a pump on another
    /// takes them. It is the leaf of the lock order — nothing else is taken while
    /// it is held — and it is never held across a file operation or an index
    /// write, so the two callers of a delete never wait on each other's disk.
    total: Mutex<QuotaTotal>,
    /// The sessions this writer has taken, by session id. One lock, held only
    /// around the map itself: never across a file operation and never across an
    /// index write, so the mutex behind [`ArchiveIndex`] is never waited on by a
    /// thread holding this one.
    open: Mutex<BTreeMap<String, OpenArchive>>,
    /// Everything a capture thread has handed over and the disk has not taken yet,
    /// including the records a pump is part-way through. Second in the lock order,
    /// after `open` and before `total`, and never held across a file operation or
    /// an index write.
    queue: Mutex<ArchiveQueue>,
    /// Held for the length of one [`ArchiveWriter::pump`], and by the closes that
    /// touch a file the pump could be inside.
    ///
    /// It is not one of the three state locks and it is deliberately held across
    /// the disk: a pump borrows an open session's handle out of its slot so a
    /// capture thread's [`ArchiveWriter::enqueue`] never waits on a write, and this
    /// is what stops a second pump from finding the slot empty and writing the same
    /// file through a second handle.
    pump_lock: Mutex<()>,
}

impl ArchiveWriter {
    /// Create the archive directory if needed, sweep it, and start the quota
    /// counter from what the sweep measured.
    ///
    /// `archive_dir` must be the dedicated `<app-data>/run-log-archives/`, never
    /// the data directory that holds `runcove.sqlite3`.
    pub fn initialize(
        archive_dir: PathBuf,
        fs: Arc<dyn ArchiveFs>,
        index: Arc<dyn ArchiveIndex>,
        bounds: QueueBounds,
        limits: QuotaLimits,
        now: i64,
    ) -> AppResult<(Self, SweepReport)> {
        // Created before it is swept, so the first run of a build that has never
        // archived anything sweeps an empty directory instead of failing on a
        // missing one. This is one of the two failures that stop initialization
        // outright: with no directory there is nowhere to write and nothing to
        // reconcile, and the caller reports the archive as unavailable.
        fs.create_dir_all(&archive_dir).map_err(|error| {
            invalid(format!(
                "Could not create the run log archive directory: {error}"
            ))
        })?;

        let report = Sweep {
            archive_dir: &archive_dir,
            fs: &*fs,
            index: &*index,
            now,
            report: SweepReport::default(),
        }
        .run()?;

        Ok((
            Self {
                archive_dir,
                fs,
                index,
                limits,
                total: Mutex::new(report.measured_bytes),
                open: Mutex::new(BTreeMap::new()),
                queue: Mutex::new(ArchiveQueue::new(bounds)),
                pump_lock: Mutex::new(()),
            },
            report,
        ))
    }

    pub fn archive_dir(&self) -> &Path {
        &self.archive_dir
    }

    /// Bytes the quota currently accounts for, or [`QuotaTotal::Unavailable`] when
    /// the sweep could not measure the directory and nothing since has made the
    /// total knowable.
    pub fn total_bytes(&self) -> QuotaTotal {
        *self.quota_total()
    }

    fn quota_total(&self) -> MutexGuard<'_, QuotaTotal> {
        self.total.lock().expect("archive quota mutex poisoned")
    }

    /// Give back the bytes a file that is really gone was really holding.
    ///
    /// `measured` is the length the filesystem reported for that file, taken
    /// before it went, and never the `byte_size` its row carried: a row is data
    /// out of a database this build does not exclusively own, and crediting a
    /// number larger than what was there would let the directory grow past its
    /// cap. The subtraction saturates, so a total that has drifted below one
    /// file's length cannot wrap into a number the size of the disk.
    ///
    /// A [`QuotaTotal::Unavailable`] total stays unavailable. A delete says how
    /// long one file was, not how much the directory holds, so nothing here can
    /// make an unmeasurable directory measurable again; the next startup sweep is
    /// what recovers a real total.
    fn credit_removed(&self, measured: u64) {
        let mut total = self.quota_total();
        if let QuotaTotal::Known(bytes) = *total {
            *total = QuotaTotal::Known(bytes.saturating_sub(measured));
        }
    }

    /// Open the file for a session and insert its `writing` row.
    ///
    /// The file is created first and the row second. A crash between the two
    /// leaves a file with no row, which the next sweep deletes as an orphan. When
    /// the row insert fails while this process is still alive, `begin` removes the
    /// file it just created and reports the index error, so the ordinary failure
    /// leaves nothing behind; if that removal also fails, the sweep is still the
    /// backstop. Either way no session is left half-open: a failed `begin` means
    /// [`ArchiveWriter::is_open`] is false for that session.
    ///
    /// The quota is not consulted here. A new archive is an empty file, which adds
    /// nothing to the total, so there is nothing yet to weigh against a cap; the
    /// first record is what the caps and a [`QuotaTotal::Unavailable`] total stop,
    /// in `pump`.
    pub fn begin(&self, session_id: &str, started_at: i64) -> AppResult<()> {
        // Both rules before anything is taken, created, or written: an id this
        // build could not have generated has no archive file name, and a name that
        // does not resolve to a direct child of the archive directory has no path.
        // An invalid session is therefore refused without the filesystem or the
        // index seeing it at all.
        let file_name = archive_file_name(session_id)?;
        let path = resolve_archive_path(&self.archive_dir, &file_name)?;

        self.take_slot(session_id)?;

        // The slot is held and the lock is not, so the two slow steps — a file
        // creation and a database write — run without a lock every capture thread
        // will want.
        match self.create_file_and_row(session_id, &file_name, &path, started_at) {
            Ok(file) => {
                self.open_sessions().insert(
                    session_id.to_string(),
                    OpenArchive {
                        state: SlotState::Open,
                        // Buffered here rather than in the pump, so a session's
                        // buffer lasts as long as its file and a short line costs
                        // no syscall at all.
                        file: Some(io::BufWriter::with_capacity(WRITE_BUFFER_BYTES, file)),
                        lines: 0,
                        bytes: 0,
                    },
                );
                Ok(())
            }
            Err(error) => {
                // Given back before the error is returned, so a caller that
                // retries this session meets a free slot and not a stuck one.
                self.open_sessions().remove(session_id);
                Err(error)
            }
        }
    }

    /// Take this session's slot, or refuse because something already holds it.
    ///
    /// This is the whole of the duplicate rule, and it is decided under the lock,
    /// so two threads beginning the same session cannot both go on to create a
    /// file: one takes the slot and the other is told which state it lost to.
    fn take_slot(&self, session_id: &str) -> AppResult<()> {
        let mut open = self.open_sessions();
        if let Some(taken) = open.get(session_id) {
            return Err(invalid(match taken.state {
                SlotState::Opening => {
                    format!("Session {session_id} is already opening its archive")
                }
                SlotState::Open => format!("Session {session_id} already has an open archive"),
                SlotState::Closing => format!("Session {session_id} is still closing its archive"),
            }));
        }
        open.insert(session_id.to_string(), OpenArchive::opening());
        Ok(())
    }

    fn open_sessions(&self) -> MutexGuard<'_, BTreeMap<String, OpenArchive>> {
        self.open.lock().expect("archive writer mutex poisoned")
    }

    /// The file and then the row, with the file taken back out if the row is
    /// refused.
    ///
    /// The handle is dropped before the removal is attempted: Windows refuses to
    /// delete a file this process still holds open, so a cleanup that kept the
    /// handle would fail every time and leave an orphan on the ordinary failure
    /// path instead of the exceptional one.
    fn create_file_and_row(
        &self,
        session_id: &str,
        file_name: &str,
        path: &Path,
        started_at: i64,
    ) -> AppResult<Box<dyn ArchiveFile>> {
        // `create_new`, so an archive already on disk is refused and never
        // truncated: an orphan a sweep could not delete still holds a user's log
        // until the sweep gets it, and this build does not decide otherwise here.
        let file = self.fs.create_new(path).map_err(|error| {
            invalid(format!(
                "Could not create the archive file for session {session_id}: {error}"
            ))
        })?;

        match self.index.insert_writing(session_id, file_name, started_at) {
            Ok(()) => Ok(file),
            Err(error) => {
                drop(file);
                match self.fs.remove_file(path) {
                    // The row was refused and the empty file is gone, so the
                    // refusal is all the caller needs to hear.
                    Ok(()) => Err(error),
                    // Both halves failed. What is left is a file with no row, under
                    // a name this build generates, which is exactly what the next
                    // startup sweep deletes as an orphan — so both failures are
                    // reported: the first is why the session did not open, the
                    // second is what is still on disk.
                    Err(removal) => Err(invalid(format!(
                        "Could not record the archive of session {session_id} ({error}), \
                         and the empty file already created for it could not be removed, \
                         so the next startup sweep will delete it: {removal}"
                    ))),
                }
            }
        }
    }

    /// Hand one line to the writer. Called from a capture thread, so it never
    /// fails: a record that does not fit the queue is dropped and counted, and one
    /// that belongs to no session this writer is still accepting is ignored.
    ///
    /// It does no I/O and makes no index call, so a slow disk or a busy database
    /// can never reach a capture thread. It is not lock-free, and must not be
    /// described that way: it enters a short in-memory critical section — the
    /// open-session state, then the queue, in that fixed order — because whether a
    /// session is still accepting is part of the queue's answer and cannot be read
    /// without a lock. What the claim rules out is a file operation or an index
    /// write inside those sections, not the sections themselves.
    pub fn enqueue(&self, record: ArchiveRecord) {
        // Hand over hand, and the order is the closing boundary itself: the
        // open-session lock is held while this session is checked, the queue lock is
        // taken while it is still held, and only then is it released. A close flips
        // the state and empties the queue of that session inside one section holding
        // both, so this record is either in the queue before that section — and the
        // close takes it — or refused after it. There is no third outcome where a
        // record is accepted into a queue nobody will drain.
        let open = self.open_sessions();
        let accepting = open
            .get(&record.session_id)
            .is_some_and(|slot| slot.state == SlotState::Open);
        if !accepting {
            // Ignored, and deliberately not counted: a session with no open archive
            // has no row to carry a drop counter, and one that is closing has already
            // had its losses settled. Counting here would either write to a row that
            // is finished or invent one that never existed.
            return;
        }
        let mut queue = self.queue_state();
        drop(open);
        queue.enqueue(record);
    }

    /// Do the queued work: emit pending gaps, write records, update counters,
    /// and enforce the quota. Runs in the caller's thread.
    ///
    /// One batch, front to back, one record at a time. The queue owns every record
    /// until this pump settles it, so a pump that returns `Err` leaves the whole
    /// remainder exactly where it was — in order, still counted against the bounds —
    /// and the next pump resumes at the same record. That is the difference between a
    /// retry and a replay, and it is why nothing is moved into a local buffer here.
    ///
    /// What can go wrong divides in two. A record that cannot be written, or cannot
    /// be given room, stops *its own session*: that session closes `partial` and the
    /// pump returns `Ok`, because the other sessions' work is unaffected and the
    /// application keeps running. A failure of the archive's own machinery — an index
    /// write, or a candidate that exists and cannot be evicted — returns `Err` with
    /// every remaining record still reserved, so the next tick tries the same thing
    /// again.
    pub fn pump(&self, now: i64) -> AppResult<()> {
        // One pump at a time. This is not one of the state locks and it is held
        // across the disk on purpose: a pump takes an open session's handle out of
        // its slot so no capture thread waits on a write, and this is what stops a
        // second pump from finding the slot empty and opening a second way into the
        // same file.
        let _pumping = self.pump_lock.lock().expect("archive pump mutex poisoned");

        // Everything handed over up to this instant becomes this batch, appended
        // behind whatever a previous failed pump left in front of it.
        self.queue_state().begin_batch();

        // The sessions this batch wrote to, so the batch can end with one flush and
        // one counter update each rather than one per line.
        let mut written: BTreeSet<String> = BTreeSet::new();

        while let Some(next) = self.next_write() {
            let Some(counted) = self.counted_bytes(&next.session_id) else {
                // No open slot for this record. Unreachable — `enqueue` refuses a
                // session that is not `Open`, and a close takes that session's
                // records out of the queue with it — so this is the arm that must not
                // quietly drop a line: it is charged as a loss like any other and the
                // batch carries on.
                self.discard_front();
                continue;
            };

            if self.room_for(counted, next.cost(), now)? == Room::Full {
                // The record is still in flight, and the close is what accounts for
                // it along with everything behind it this session will not reach. A
                // row this pump had already written to is finalized by that close, so
                // it must not be updated again when the batch ends.
                written.remove(&next.session_id);
                self.writer_close(&next.session_id, ArchiveReason::QuotaExceeded, now, true)?;
                continue;
            }

            let Some(item) = self.queue_state().take_front() else {
                // Unreachable: nothing else takes from the front of an in-flight
                // batch while the pump lock is held.
                break;
            };
            if self.write_one(&next, item, now)? {
                written.insert(next.session_id);
            } else {
                // The write failed, so that session is already closed and its row is
                // final.
                written.remove(&next.session_id);
            }
        }

        self.finish_batch(&written, now)
    }

    fn queue_state(&self) -> MutexGuard<'_, ArchiveQueue> {
        self.queue.lock().expect("archive queue mutex poisoned")
    }

    /// Encode the record at the front of the batch without taking it out.
    ///
    /// The encoding happens here, under the queue lock, because it is cheap and
    /// touches nothing outside this value; the write that follows happens with no
    /// lock held at all. The record stays in front until it is settled, so a refusal
    /// or a failure after this point needs no undo step. What the encoding itself
    /// consists of, gap line included, is [`pending_write`].
    fn next_write(&self) -> Option<PendingWrite> {
        let queue = self.queue_state();
        let front = queue.front()?;
        Some(pending_write(front))
    }

    /// Bytes one open session's file already holds, or `None` when this writer has
    /// no open slot for it.
    ///
    /// `None` is the answer that says the record in front belongs to nothing: the
    /// per-session cap is measured against this number, so there is no cap to measure
    /// and no file to write to.
    fn counted_bytes(&self, session_id: &str) -> Option<u64> {
        self.open_sessions().get(session_id).map(|slot| slot.bytes)
    }

    /// Charge the front record as a loss and take it out of the batch.
    ///
    /// For the record that cannot belong to any session. It is charged rather than
    /// dropped because a line that left a capture thread has to be accounted for
    /// somewhere, and the session's own counters are the only place that means
    /// anything.
    fn discard_front(&self) {
        let mut queue = self.queue_state();
        if let Some(item) = queue.take_front() {
            queue.discard(item);
        }
    }

    /// Whether the two caps leave room for one write, evicting if that is what it
    /// takes.
    ///
    /// The per-session cap is checked first, and no eviction can help it: deleting
    /// another run's archive would not make this session's file allowed to be larger,
    /// so a session at its own cap stops without anything else being removed.
    ///
    /// The total is then checked in a loop, because one eviction frees one archive and
    /// one archive may not be enough. An unmeasurable total refuses the write outright
    /// and evicts nothing: a directory whose size this build could not work out must
    /// not be grown on a guess, and deleting a user's archive to make room the archive
    /// may already have would be worse than stopping.
    ///
    /// The quota guard is taken and released around each question. It is never held
    /// across an eviction — an eviction is a file removal and an index write — and
    /// that is the whole reason the loop re-reads the total instead of tracking it.
    ///
    /// It takes the bytes rather than a session, because neither cap is about who is
    /// writing: the per-session one is measured against `counted`, and eviction may
    /// never touch a session this writer holds open whoever asked.
    fn room_for(&self, counted: u64, cost: u64, now: i64) -> AppResult<Room> {
        if counted.saturating_add(cost) > self.limits.session_bytes {
            return Ok(Room::Full);
        }
        loop {
            let QuotaTotal::Known(held) = self.total_bytes() else {
                return Ok(Room::Full);
            };
            if held.saturating_add(cost) <= self.limits.total_bytes {
                return Ok(Room::Available);
            }
            if !self.evict_one(now)? {
                return Ok(Room::Full);
            }
        }
    }

    /// Free one ended archive to make room, or say that there is nothing to free.
    ///
    /// One removal per call. The caller re-reads the total and asks again, so freeing
    /// two archives is two decisions taken against two measured totals rather than one
    /// guess about how much to delete at once.
    ///
    /// `Ok(false)` means nothing was eligible: every archive left is either being
    /// written or has no end. That is the answer that stops the asking session at the
    /// cap. `Err` means a candidate existed and could not be removed — a row naming
    /// another session's file, a file the filesystem refused to delete — and the
    /// difference is what makes one of them retryable: nothing eligible will still be
    /// nothing eligible next tick, while a refused removal may well succeed.
    fn evict_one(&self, now: i64) -> AppResult<bool> {
        let Some(row) = self.eviction_candidate()? else {
            return Ok(false);
        };
        // A row's `file_name` is never used as a path on its say-so. This is the same
        // check a user delete goes through, and a row that fails it is reported rather
        // than skipped: a name in the index that is not the one this build generates
        // is a repair for the sweep, not something to route around quietly while
        // deleting the next archive along.
        let file_name = verified_file_name(&row.session_id, &row)?;
        let (path, measured) =
            resolve_ordinary_archive_file(&*self.fs, &self.archive_dir, &file_name)?;
        self.fs.remove_file(&path).map_err(|error| {
            invalid(format!(
                "Could not remove the archive of session {} to free space: {error}",
                row.session_id
            ))
        })?;
        // The disk's number, and only once the file is really gone. Credited before
        // the row is touched, because the bytes are free whatever the index says next:
        // a row that will not move must not also cost the quota room it really has.
        self.credit_removed(measured);
        self.index
            .mark_removed(&row.session_id, ArchiveReason::QuotaEvicted, now)?;
        Ok(true)
    }

    /// The archive that should go first, or `None` when none may go.
    ///
    /// Eligibility is settled before order, because they are different questions and
    /// sorting first would let an ineligible row win a comparison it should never have
    /// entered. A candidate is a row that has ended — `complete` or `partial`, with an
    /// `ended_at` — and that this writer does not hold open.
    ///
    /// A `writing` row is never a candidate: its file is being appended to, and taking
    /// it away under its own writer would leave a handle writing into nothing. A row
    /// with no `ended_at` is never a candidate either, whatever its status claims.
    /// RunCove's own schema forbids that combination, so it can only arrive from a
    /// database this build did not write — which is exactly why the rule lives here and
    /// not in the schema — and reading a missing timestamp as zero would make the
    /// malformed row the first candidate of all. A status this build does not know is
    /// not a candidate for the same reason: a newer build's row is not this one's to
    /// interpret.
    ///
    /// The order is `ended_at`, then `started_at`, then the session id: oldest end
    /// first, with two deterministic tie-breaks, so a directory of archives that ended
    /// in the same millisecond is not evicted in whatever order the index happened to
    /// return.
    ///
    /// The rows are read before the open-session lock is taken, so no index call is
    /// made with a state lock held.
    fn eviction_candidate(&self) -> AppResult<Option<ArchiveRow>> {
        let rows = self.index.rows()?;
        let held = self.open_sessions();
        Ok(rows
            .into_iter()
            .filter(|row| {
                matches!(
                    ArchiveStatus::parse(&row.status),
                    Some(ArchiveStatus::Complete | ArchiveStatus::Partial)
                ) && row.ended_at.is_some()
                    && !held.contains_key(&row.session_id)
            })
            .min_by(|left, right| {
                left.ended_at
                    .cmp(&right.ended_at)
                    .then_with(|| left.started_at.cmp(&right.started_at))
                    .then_with(|| left.session_id.cmp(&right.session_id))
            }))
    }

    /// Write one payload to its session's file and settle the record it came from.
    ///
    /// `Ok(true)`: the bytes are in the file, or in the buffer this batch ends by
    /// flushing, and the session is still archiving. `Ok(false)`: the write failed and
    /// that session has been closed `partial` / `write-error`, with the record charged
    /// as a loss — every other session is untouched and the pump carries on, because
    /// one disk error is not the archive's error. `Err` is reserved for the close's own
    /// machinery failing.
    ///
    /// The handle is taken out of the slot for the write and put back afterwards, so no
    /// state lock is held across the disk and a record arriving mid-write still meets a
    /// slot that says `Open` and is accepted. The pump lock is what makes borrowing it
    /// safe, and the close is the only thing that ever gives a handle up — which is why
    /// the failure path hands it back before closing rather than dropping it here.
    fn write_one(&self, next: &PendingWrite, item: QueuedRecord, now: i64) -> AppResult<bool> {
        let Some(mut file) = self.borrow_file(&next.session_id) else {
            // Unreachable: `counted_bytes` just read this slot, and only a close takes
            // its handle, which the pump lock serializes against. Charged rather than
            // dropped, because a line that left a capture thread has to end up in a
            // counter one way or the other.
            self.queue_state().discard(item);
            return Ok(false);
        };

        match file.write_all(&next.payload) {
            Ok(()) => {
                self.return_file(&next.session_id, file, next.lines, next.cost());
                self.charge_written(next.cost());
                self.queue_state().release(&item);
                Ok(true)
            }
            // The error's own message has nowhere to go: this build has no log sink,
            // and a session's write failure is reported through its row — `partial` /
            // `write-error` — which is the whole of what anyone downstream is told.
            Err(_refused) => {
                // Neither a line nor a byte is credited for a write that failed. What
                // it may have left behind is measured off the file itself by the close,
                // so a short write's real fragment still reaches the quota and the row
                // while the record that produced it stays a loss.
                self.return_file(&next.session_id, file, 0, 0);
                // Charged before the close, so `discard_session` finds none of this
                // session's records left and `finish_session` can complete.
                self.queue_state().discard(item);
                self.writer_close(&next.session_id, ArchiveReason::WriteError, now, false)?;
                Ok(false)
            }
        }
    }

    /// Take an open session's handle out of its slot for the length of one write.
    ///
    /// The slot keeps its state and its counters; only the handle moves. That is what
    /// lets a write happen with no state lock held while a record arriving in the
    /// middle of it is still judged `Open` and accepted — and it is why `state`, never
    /// the presence of this handle, is what [`ArchiveWriter::enqueue`] reads.
    fn borrow_file(&self, session_id: &str) -> Option<ArchiveHandle> {
        let mut open = self.open_sessions();
        open.get_mut(session_id)?.file.take()
    }

    /// Put the handle back and add what the write actually put in the file.
    ///
    /// `lines` and `bytes` are zero for a write that failed. They are what the row
    /// reports while the session is open, so a line still sitting in the buffer counts:
    /// the batch ends by flushing it, and the archive is going to hold it. The one
    /// place that is not exact is a flush that fails after a line was buffered — the
    /// close then measures the file and corrects `byte_size` and the quota from the
    /// disk, but `line_count` can still name a line the file does not hold. A count of
    /// pending lines would not fix it either: a flush can also go out *partially*, so
    /// saying which buffered lines survived needs each one's byte length, and the byte
    /// side is already corrected from the disk. Describing a failed disk that precisely
    /// is not worth the state it would take.
    fn return_file(&self, session_id: &str, file: ArchiveHandle, lines: i64, bytes: u64) {
        let mut open = self.open_sessions();
        match open.get_mut(session_id) {
            Some(slot) => {
                slot.file = Some(file);
                slot.lines = slot.lines.saturating_add(lines);
                slot.bytes = slot.bytes.saturating_add(bytes);
            }
            // Unreachable: only a close removes a slot, and the pump lock is what
            // stops one from running while this pump holds the handle. Dropping it here
            // is the safe way to lose it — the session is gone, so there is nothing
            // left to write to it.
            None => drop_without_flushing(file),
        }
    }

    /// Add bytes that reached a file to the directory's total.
    ///
    /// Saturating, and a [`QuotaTotal::Unavailable`] total stays unavailable for the
    /// same reason [`ArchiveWriter::credit_removed`] leaves it alone: one file's growth
    /// says nothing about how much a directory this build could not measure holds.
    fn charge_written(&self, bytes: u64) {
        let mut total = self.quota_total();
        if let QuotaTotal::Known(held) = *total {
            *total = QuotaTotal::Known(held.saturating_add(bytes));
        }
    }

    /// Close a session the writer itself is stopping, mid-batch, always `partial`.
    ///
    /// `flushable` is false when the file is the thing that failed. The buffer is then
    /// given up without a flush, because writing again to a handle that has just
    /// refused can only fail again and a second error would have nothing to add.
    ///
    /// The boundary is one linearization point, exactly as the public close's is: the
    /// slot is marked `Closing` and everything this session had already accepted is
    /// taken out of the queue inside one critical section holding both state locks. From
    /// that instant [`ArchiveWriter::enqueue`] refuses the session, so no record is left
    /// accepted with nobody to write it and nothing to charge it to. Neither lock is
    /// held across the file work that follows.
    ///
    /// Every record this session accepted and never got onto the disk is charged to the
    /// row's drop counters — that is what [`ArchiveQueue::discard_session`] is for — and
    /// no gap line is written for any of them. A failed file cannot take one, and a file
    /// stopping at its cap must not grow by one, so the row's counters are the only
    /// surviving record of the loss and have to be exact.
    fn writer_close(
        &self,
        session_id: &str,
        reason: ArchiveReason,
        ended_at: i64,
        flushable: bool,
    ) -> AppResult<()> {
        let (file, lines, counted, finished) = {
            let mut open = self.open_sessions();
            let mut queue = self.queue_state();
            let taken = open.get_mut(session_id).map(|slot| {
                slot.state = SlotState::Closing;
                (slot.file.take(), slot.lines, slot.bytes)
            });
            queue.discard_session(session_id);
            // Cannot refuse: the discard above is what leaves this session with no
            // record of its own, which is the one thing `finish_session` refuses for.
            let finished = queue.finish_session(session_id)?;
            let (file, lines, counted) = taken.unwrap_or((None, 0, 0));
            (file, lines, counted, finished)
        };

        // From here no lock is held: the flush, the measurement, and the row are all
        // outside the section above.
        let (durable, flush_failed) = match file {
            Some(file) if flushable => {
                let flushed = self.flush_and_sync(file).is_ok();
                (flushed, !flushed)
            }
            // Given up unwritten. After a record larger than the buffer there is
            // nothing in it anyway, and whatever is cannot be written to a file that
            // has already refused one.
            Some(file) => {
                drop_without_flushing(file);
                (false, false)
            }
            // No handle at all, which a pump cannot produce: it only closes a session
            // whose slot it has just read. Untrusted, so the file has the last word.
            None => (false, false),
        };

        // A count that could not be made durable is a claim about what was handed over
        // rather than about what landed, so the file itself is measured instead.
        let byte_size = if durable {
            counted
        } else {
            match self.measured_bytes(session_id) {
                Some(measured) => {
                    self.reconcile_total(counted, measured);
                    measured
                }
                // Nothing can say how long the file is, so the writer's own count is
                // the only number there is; the next startup sweep recovers the total.
                None => counted,
            }
        };

        // A failed flush outranks the reason that started the close, so a session
        // stopping at its cap that could not be made durable is reported as the write
        // error it also turned out to be.
        let reason = if flush_failed {
            ArchiveReason::most_severe(reason, ArchiveReason::WriteError)
        } else {
            reason
        };

        // The slot goes last of the file work and before the row: from here the session
        // is not open, so a delete for it is no longer refused.
        self.open_sessions().remove(session_id);

        // `finished.residual_gap` is deliberately left unwritten. This close has
        // nowhere to put a gap line, and the run it describes is already part of
        // `finished.dropped`, which the row below carries.
        self.index.close(
            session_id,
            ArchiveStatus::Partial,
            Some(reason),
            ArchiveCounters {
                line_count: lines,
                byte_size: i64::try_from(byte_size).unwrap_or(i64::MAX),
                dropped_lines: finished.dropped.lines,
                dropped_bytes: finished.dropped.bytes,
            },
            ended_at,
        )
    }

    /// Flush the buffer, make the bytes durable, and release the handle.
    ///
    /// Both steps or neither is claimed. A close that could not flush, or could not
    /// sync, has bytes it counted and cannot prove, so its caller stops trusting its own
    /// count and measures the file. `sync_data` is asked of the archive file underneath
    /// the buffer, because durability is the file's promise and not the buffer's.
    fn flush_and_sync(&self, mut file: ArchiveHandle) -> io::Result<()> {
        file.flush()?;
        file.get_mut().sync_data()
    }

    /// The length the filesystem reports for one session's archive, or `None` when this
    /// build cannot measure it.
    ///
    /// Taken after the handle is gone, which is the only moment the number is final: a
    /// buffered writer's own count says what was handed over, not what landed. `None`
    /// covers a file that is missing or unreadable and an id that has no valid name —
    /// none of which is this close's business to repair, and all of which the startup
    /// sweep is.
    fn measured_bytes(&self, session_id: &str) -> Option<u64> {
        let file_name = archive_file_name(session_id).ok()?;
        resolve_ordinary_archive_file(&*self.fs, &self.archive_dir, &file_name)
            .ok()
            .map(|(_path, measured)| measured)
    }

    /// Correct the directory total by the difference between what this writer charged
    /// for a session and what its file turned out to hold.
    ///
    /// Only a close whose byte count could not be trusted comes here. A write that
    /// failed may still have left a fragment behind — those bytes are as real as any
    /// others, and a later eviction has to be able to free them — while bytes that were
    /// charged and never landed must not go on occupying the user's cap.
    fn reconcile_total(&self, counted: u64, measured: u64) {
        if measured > counted {
            self.charge_written(measured - counted);
        } else {
            self.credit_removed(counted - measured);
        }
    }

    /// End the batch: one flush and one counter update per session it wrote to.
    ///
    /// Once per session rather than once per record, which is the whole point of the
    /// buffer and of batching the index writes: a run producing thousands of lines a
    /// second must not turn each one into a syscall and a database write.
    ///
    /// The flush comes before the counters, so a row is never updated to claim bytes
    /// the file has not been given. A flush that fails here is that session's write
    /// error like any other — it closes `partial` / `write-error` and the rest of the
    /// batch is still finished.
    fn finish_batch(&self, written: &BTreeSet<String>, now: i64) -> AppResult<()> {
        for session_id in written {
            let Some(mut file) = self.borrow_file(session_id) else {
                // Unreachable: this session was written to during this pump, and only a
                // close removes its slot.
                continue;
            };
            let flushed = file.flush();
            // Back in its slot either way, so the close below stays the only thing that
            // ever gives a handle up.
            self.return_file(session_id, file, 0, 0);
            if flushed.is_err() {
                self.writer_close(session_id, ArchiveReason::WriteError, now, false)?;
                continue;
            }
            self.index
                .update_counters(session_id, self.counters_of(session_id))?;
        }
        Ok(())
    }

    /// The counters one open session's row should now carry.
    ///
    /// Read after the flush, so `byte_size` is what the file has been given and
    /// `line_count` is what it holds. The drop counters are the session's cumulative
    /// history, which is what the row means by them: a loss stays reported for the rest
    /// of the run, not only until the next update.
    ///
    /// Both locks in the fixed order, and neither is held when this returns, so the
    /// index write that uses these numbers is made with nothing locked.
    fn counters_of(&self, session_id: &str) -> ArchiveCounters {
        let open = self.open_sessions();
        let Some(slot) = open.get(session_id) else {
            return ArchiveCounters::default();
        };
        let dropped = self.queue_state().dropped(session_id);
        ArchiveCounters {
            line_count: slot.lines,
            byte_size: i64::try_from(slot.bytes).unwrap_or(i64::MAX),
            dropped_lines: dropped.lines,
            dropped_bytes: dropped.bytes,
        }
    }

    /// Close one session: emit its last gap, flush, `sync_data`, then update the
    /// row. `reason` is `None` for a clean close.
    ///
    /// The file is flushed, synced, and released before the row is written, so an
    /// index failure at this point cannot leave the session writable: it reports
    /// the error with the bytes already durable and the row still `writing`, which
    /// the next sweep repairs to `partial` / `interrupted`.
    ///
    /// The closing boundary is one linearization point, not a pump followed by a
    /// state change. In a single critical section — the open-session state, then
    /// the queue, in that fixed order — the session is marked as closing and
    /// everything it had already accepted is taken out of the queue together.
    /// After that instant every `enqueue` for it is refused, so a record either
    /// won the race and is written by this close, or lost it and never existed as
    /// far as the archive is concerned. Pumping first and changing the state
    /// afterwards would leave a real window, because [`ArchiveWriter::enqueue`]
    /// does not take the pump's lock. Neither the open-session lock nor the
    /// queue's is held across the file work that follows.
    ///
    /// The row's drop counters and the last gap come from
    /// [`ArchiveQueue::finish_session`] rather than from
    /// [`ArchiveQueue::take_pending_gap`] and [`ArchiveQueue::dropped`]: finishing
    /// is the only thing that frees the session's entry, and it refuses while
    /// records are still queued — which is why the extraction above has to leave
    /// none behind.
    pub fn close(
        &self,
        session_id: &str,
        reason: Option<ArchiveReason>,
        ended_at: i64,
    ) -> AppResult<()> {
        // The pump's lock first, and for the same reason `pump` holds it across the
        // disk: this close takes the session's handle out of its slot and writes
        // through it, and a pump borrows that same handle for the length of a write.
        // Without this lock a close could find the slot handle-less in the middle of a
        // pump and write nothing at all.
        let _pumping = self.pump_lock.lock().expect("archive pump mutex poisoned");

        let mut closing = self.begin_close(session_id)?;
        self.write_taken(&mut closing);

        // Only now can the session be finished: every record it had is settled, so the
        // queue owes nothing for it. This is the one call that frees its entry, and it
        // hands back both halves of its drop history together.
        let finished = self
            .queue_state()
            .finish_session(session_id)
            // Unreachable: finishing is refused only while the session still has queued
            // records, and the boundary above took every one of them. Defaulted rather
            // than propagated, because returning here would leave the slot `Closing`
            // for the rest of the run — a session no close could finish and no
            // `enqueue` would feed.
            .unwrap_or_default();

        // The trailing loss, if the session ended on one. It goes after the last record
        // for the same reason a carried gap goes before its carrier: the file is a
        // timeline. This is the line `writer_close` has nowhere to put — a file that has
        // just failed a write, or just been refused a byte by the cap, must not be asked
        // for one more line — and a close the user asked for is the case that can.
        if let Some(gap) = finished.residual_gap {
            self.write_residual_gap(&mut closing, session_id, gap, ended_at);
        }

        // Both steps or neither: a close that could not flush, or could not sync, has
        // bytes it counted and cannot prove.
        let durable = match closing.file.take() {
            Some(file) if !closing.failed => match self.flush_and_sync(file) {
                Ok(()) => true,
                Err(_refused) => {
                    closing.failed = true;
                    false
                }
            },
            // Given up unflushed. Asking a file that has already refused one write for
            // another could only fail again, and what the buffer is still holding are
            // the bytes of the write that failed.
            Some(file) => {
                drop_without_flushing(file);
                false
            }
            // Unreachable: the slot said `Open`, and only this close takes its handle.
            None => false,
        };

        // A count that could not be made durable is a claim about what was handed over
        // rather than about what landed, so the file itself is measured instead. That is
        // how a short write's real fragment reaches the row and the quota while the
        // record that produced it stays a loss.
        let byte_size = if durable {
            closing.bytes
        } else {
            match self.measured_bytes(session_id) {
                Some(measured) => {
                    self.reconcile_total(closing.bytes, measured);
                    measured
                }
                // Nothing can say how long the file is, so the writer's own count is the
                // only number there is; the next startup sweep recovers the total.
                None => closing.bytes,
            }
        };

        let dropped = finished.dropped;
        let mut outcome = reason;
        // A close that lost lines is not complete, whatever it was asked to report. The
        // loss is in the file as gap lines and in the row's counters, and
        // `queue-overflow` is the only thing that can have caused it here: a write error
        // and a full cap each close their own session, before it can reach this call.
        if dropped.lines > 0 || dropped.bytes > 0 {
            outcome = Some(worsened(outcome, ArchiveReason::QueueOverflow));
        }
        // A file that refused bytes, or could not be made durable, is a write error too.
        if closing.failed {
            outcome = Some(worsened(outcome, ArchiveReason::WriteError));
        }

        // The slot goes last of the file work and before the row: from here the session
        // is not open, so a delete for it is no longer refused, and the index write below
        // cannot leave it writable if it fails. What that failure leaves is durable bytes
        // under a `writing` row, which the next startup sweep repairs to `partial` /
        // `interrupted`.
        self.open_sessions().remove(session_id);

        self.index.close(
            session_id,
            match outcome {
                Some(_) => ArchiveStatus::Partial,
                None => ArchiveStatus::Complete,
            },
            outcome,
            ArchiveCounters {
                line_count: closing.lines,
                byte_size: i64::try_from(byte_size).unwrap_or(i64::MAX),
                dropped_lines: dropped.lines,
                dropped_bytes: dropped.bytes,
            },
            ended_at,
        )
    }

    /// Close every open session with the same reason, for the toggle going off
    /// and for application shutdown. The files stay on disk.
    ///
    /// Every session, not up to the first failure: one session's index write failing
    /// must not leave the others open after the archive has been switched off. The first
    /// error is the one returned, because it is the one that happened before anything
    /// else went wrong.
    ///
    /// The list is taken under the lock and the closes are made without it, since each
    /// one takes that lock itself. Only sessions that are `Open` are on it — a session
    /// another thread is opening or closing at this instant is not this call's to finish,
    /// and a refusal it would earn is not a failure worth reporting.
    pub fn close_all(&self, reason: ArchiveReason, ended_at: i64) -> AppResult<()> {
        let sessions: Vec<String> = self
            .open_sessions()
            .iter()
            .filter(|(_session_id, slot)| slot.state == SlotState::Open)
            .map(|(session_id, _slot)| session_id.clone())
            .collect();

        let mut first_error = None;
        for session_id in sessions {
            if let Err(error) = self.close(&session_id, Some(reason), ended_at) {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Establish the closing boundary and take everything this close becomes
    /// responsible for: the handle, what the row already counts, and the records the
    /// session accepted and the disk has not been given yet.
    ///
    /// One critical section, holding the open-session state and then the queue in the
    /// fixed order. That is what makes the boundary a single linearization point: after
    /// it, `state` is `Closing`, so every `enqueue` for the session is refused, and the
    /// queue holds none of its records, so nothing can be left stranded between the two
    /// facts. Both locks are released before any file work begins.
    ///
    /// A session that is not `Open` is refused with no work done and nothing taken — no
    /// file touched, no counter moved, no index call made — so a second close of the
    /// same session, or a close of one that was never begun, is inert.
    fn begin_close(&self, session_id: &str) -> AppResult<ClosingSession> {
        let mut open = self.open_sessions();
        let mut queue = self.queue_state();
        let Some(slot) = open.get_mut(session_id) else {
            return Err(invalid(format!(
                "Session {session_id} has no open archive to close"
            )));
        };
        match slot.state {
            SlotState::Open => {}
            // Neither state is this call's to finish, and neither may be turned into a
            // second close: `begin` is still deciding whether the first one exists, and
            // a close already past this boundary owns the file.
            SlotState::Opening => {
                return Err(invalid(format!(
                    "Session {session_id} is still opening its archive"
                )));
            }
            SlotState::Closing => {
                return Err(invalid(format!(
                    "Session {session_id} is already closing its archive"
                )));
            }
        }
        slot.state = SlotState::Closing;
        Ok(ClosingSession {
            file: slot.file.take(),
            lines: slot.lines,
            bytes: slot.bytes,
            taken: queue.take_session(session_id),
            failed: false,
        })
    }

    /// Write the records this close took, in arrival order, and settle every one of
    /// them.
    ///
    /// A write that fails is terminal for the session: this record and everything behind
    /// it are charged as losses, because the handle has just said it will not take bytes
    /// and a second attempt could only be refused again. What did land is released, so
    /// its room comes back and nothing the file actually holds is counted as lost.
    ///
    /// The settling is one queue critical section at the end rather than one per record,
    /// and it happens after all the file work, so no lock is held across a write.
    fn write_taken(&self, closing: &mut ClosingSession) {
        let mut taken = std::mem::take(&mut closing.taken).into_iter();
        let mut written: Vec<QueuedRecord> = Vec::new();
        let mut lost: Vec<QueuedRecord> = Vec::new();
        for item in taken.by_ref() {
            let next = pending_write(&item);
            if self.append(closing, &next.payload, next.lines) {
                written.push(item);
            } else {
                lost.push(item);
                break;
            }
        }
        // Everything behind the record that failed. Charged rather than silently
        // dropped, because a line that left a capture thread has to end up in a counter
        // one way or the other.
        lost.extend(taken);

        let mut queue = self.queue_state();
        for item in &written {
            queue.release(item);
        }
        for item in lost {
            queue.discard(item);
        }
    }

    /// Write the session's trailing gap line, the run of losses no later record could
    /// carry.
    ///
    /// `ended_at` is its timestamp: this writer reads no clock, and the instant a line
    /// was dropped is not an instant anything recorded. It is counted and charged like
    /// any other archived line, because that is what it is.
    fn write_residual_gap(
        &self,
        closing: &mut ClosingSession,
        session_id: &str,
        gap: DropCounters,
        ended_at: i64,
    ) {
        let mut payload = Vec::new();
        push_line(
            &mut payload,
            &encode_record(&ArchiveRecord {
                session_id: session_id.to_string(),
                stream: LogStream::System,
                line: gap_line(gap),
                timestamp: ended_at,
            }),
        );
        self.append(closing, &payload, 1);
    }

    /// Append one payload to a closing session's own handle, counting what lands.
    ///
    /// `false` says the bytes did not reach the file. The session is marked failed with
    /// it, so nothing asks that handle for another line and the row ends up saying
    /// `write-error`.
    ///
    /// A close does not consult the quota. Its records were accepted while the session
    /// was open and its own linearization point is already behind it, so there is
    /// nothing left to refuse them for — and refusing them would be worse than the
    /// overshoot, which is bounded by one session's queued bytes plus a gap line and is
    /// charged like every other byte.
    fn append(&self, closing: &mut ClosingSession, payload: &[u8], lines: i64) -> bool {
        if closing.failed {
            return false;
        }
        let Some(file) = closing.file.as_mut() else {
            // Unreachable: the slot said `Open`, so it had its handle. Treated as a
            // refusal, because from the row's side a disk that will not take the bytes
            // and no disk at all are the same thing.
            closing.failed = true;
            return false;
        };
        // The error's own message has nowhere to go: this build has no log sink, and a
        // session's write failure is reported through its row.
        if file.write_all(payload).is_err() {
            closing.failed = true;
            return false;
        }
        closing.lines = closing.lines.saturating_add(lines);
        let cost = payload.len() as u64;
        closing.bytes = closing.bytes.saturating_add(cost);
        self.charge_written(cost);
        true
    }

    /// Whether this writer holds this session's archive.
    ///
    /// True from the moment [`ArchiveWriter::begin`] takes the session's slot until
    /// a close, or a failed `begin`, gives it back. That includes the instant a slot
    /// exists without its file, which is deliberate: [`ArchiveWriter::delete`]
    /// refuses an open session, and a session this writer is in the middle of
    /// opening must not lose its file underneath it either.
    ///
    /// A session the index knows about but this writer never opened — every archive
    /// an earlier run left behind — is not open.
    pub fn is_open(&self, session_id: &str) -> bool {
        self.open_sessions().contains_key(session_id)
    }

    /// The text of one archive, for the user interface.
    ///
    /// Takes the session, never a file name: the caller holds a session id, the
    /// name is the row's to supply, and checking it is this module's job. See
    /// [`verified_file_name`].
    ///
    /// The row is required. A file sitting under a name this build generates, with
    /// no row behind it, is not an archive this build will read — it is what the
    /// next startup sweep deletes as an orphan.
    pub fn read(&self, session_id: &str) -> AppResult<String> {
        let row = self.row_of(session_id)?;
        let file_name = verified_file_name(session_id, &row)?;
        let (path, _measured) =
            resolve_ordinary_archive_file(&*self.fs, &self.archive_dir, &file_name)?;

        self.fs.read_to_string(&path).map_err(|error| {
            invalid(format!(
                "The run log archive of session {session_id} could not be read: {error}"
            ))
        })
    }

    /// One page of an archive, the tail of it by default, for the viewer.
    ///
    /// The whole-file [`ArchiveWriter::read`] above stays for the tests that
    /// assert on a small archive's exact bytes; this is the only read a command
    /// exposes, because an archive is capped at 10 MiB and no page of it belongs
    /// in one IPC message.
    ///
    /// `before_offset` is a byte offset, not a line number, and that is what makes
    /// paging stable: an archive is append-only, so bytes below a length once
    /// measured never change, and an offset this call returned stays a record
    /// boundary forever. Absent means the end of the file. Present must be a
    /// boundary this call returned earlier — `0 < n <= file_length` with a `\n` at
    /// `n - 1` — and anything else is refused rather than resynced to a nearby
    /// newline.
    ///
    /// `line_count` and the drop counters come from the row and are as fresh as
    /// the writer's last refresh, while `file_length` is measured here and is
    /// exact. A session still being written can therefore return more records than
    /// its `line_count` claims, which is honest: reading again is what shows newer
    /// output.
    ///
    /// A `removed` archive fails with its row's reason, so a viewer left open
    /// across an eviction says what happened instead of showing an empty file.
    pub fn read_page(
        &self,
        session_id: &str,
        before_offset: Option<u64>,
        max_records: Option<usize>,
    ) -> AppResult<RunLogArchivePage> {
        let row = self.row_of(session_id)?;
        if ArchiveStatus::parse(&row.status) == Some(ArchiveStatus::Removed) {
            return Err(invalid(format!(
                "The run log archive of session {session_id} is gone: {}",
                row.reason.as_deref().unwrap_or("removed")
            )));
        }

        let file_name = verified_file_name(session_id, &row)?;
        let (path, file_length) =
            resolve_ordinary_archive_file(&*self.fs, &self.archive_dir, &file_name)?;
        let mut file = self.fs.open_read(&path).map_err(|error| {
            invalid(format!(
                "The run log archive of session {session_id} could not be opened: {error}"
            ))
        })?;

        let cursor = validated_cursor(&mut *file, session_id, before_offset, file_length)?;
        let page = scan_page_backwards(
            &mut *file,
            session_id,
            cursor,
            clamped_page_records(max_records),
        )?;

        Ok(RunLogArchivePage {
            session_id: session_id.to_string(),
            status: row.status,
            reason: row.reason,
            line_count: row.counters.line_count,
            byte_size: row.counters.byte_size,
            dropped_lines: row.counters.dropped_lines,
            dropped_bytes: row.counters.dropped_bytes,
            started_at: row.started_at,
            ended_at: row.ended_at,
            records: page.records,
            file_length,
            page_start_offset: page.page_start_offset,
            // The one reading, rather than a second rule: a page that reached
            // offset zero is exactly the page with nothing before it.
            has_more_before: page.page_start_offset > 0,
            stopped_by: page.stopped_by.as_str().to_string(),
            incomplete_tail_skipped: page.incomplete_tail_skipped,
            malformed_lines: page.malformed_lines,
        })
    }

    /// Delete one archive on the user's request.
    ///
    /// Refused while its writer is open, so a running session cannot lose the file
    /// underneath it, and refused for a row whose file name is not the one this
    /// session generates.
    ///
    /// The refusal is a check before the work rather than one critical section
    /// with it: the open-session lock is not held across a file removal. Two other
    /// things are what make that safe, and they are the reason this needs no lock
    /// spanning the disk. An archive file is created with `create_new`, so a
    /// [`ArchiveWriter::begin`] racing this delete cannot take the file that is
    /// still there; and a session id is generated once for the run that produces
    /// it, so the id a user is deleting is not one any later run begins.
    ///
    /// The order is: measure, remove, credit, then move the row. The measurement
    /// has to come first because after the removal nobody can take it, and the
    /// credit has to land before the row is touched because the bytes are gone
    /// whatever the index says next — a delete whose row will not move still
    /// leaves the total telling the truth, reports the failure, and leaves the row
    /// for the sweep.
    ///
    /// A row whose file is already missing is reported rather than quietly marked.
    /// That state is one the startup sweep already finishes, and it finishes it as
    /// `removed` / `file-missing`, which is what happened, instead of recording a
    /// user delete that removed nothing.
    pub fn delete(&self, session_id: &str) -> AppResult<()> {
        // First, and before the index or the filesystem is consulted at all: a
        // session this writer holds must not lose its file, and that includes the
        // instant a slot exists without its handle.
        if self.is_open(session_id) {
            return Err(invalid(format!(
                "Session {session_id} is still writing its run log archive"
            )));
        }

        let row = self.row_of(session_id)?;
        let file_name = verified_file_name(session_id, &row)?;
        let (path, measured) =
            resolve_ordinary_archive_file(&*self.fs, &self.archive_dir, &file_name)?;

        self.fs.remove_file(&path).map_err(|error| {
            invalid(format!(
                "The run log archive of session {session_id} could not be deleted: {error}"
            ))
        })?;
        self.credit_removed(measured);

        self.index
            .mark_removed(session_id, ArchiveReason::UserDeleted, storage::now_ms())
    }

    /// The row this session's archive has, or the fact that it has none.
    ///
    /// Both [`ArchiveWriter::read`] and [`ArchiveWriter::delete`] start here, so
    /// neither reaches the filesystem for a session the index does not know: the
    /// name a session id would produce is not permission to open the file that
    /// happens to be at it.
    fn row_of(&self, session_id: &str) -> AppResult<ArchiveRow> {
        self.index
            .row(session_id)?
            .ok_or_else(|| invalid(format!("Session {session_id} has no run log archive")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::invalid;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::panic::resume_unwind;
    use std::sync::{mpsc, Barrier, Mutex};
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Session ids shaped exactly like `storage::new_id` produces: a lowercase
    /// hyphenated v4 UUID.
    const SESSION_A: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
    const SESSION_B: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    const SESSION_C: &str = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";
    const SESSION_D: &str = "3f2504e0-4f89-41d3-9a0c-0305e82c3301";

    fn name_of(session_id: &str) -> String {
        format!("{session_id}.{ARCHIVE_FILE_EXTENSION}")
    }

    /// A temporary application data directory, with the archive directory as its
    /// child. Tests only ever touch this tree; the real application data
    /// directory is never opened.
    fn temp_data_dir() -> (TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("temporary directory");
        let archive_dir = temp.path().join(ARCHIVE_DIR_NAME);
        (temp, archive_dir)
    }

    /// How many entries the archive directory holds, so a test can say that
    /// nothing was created without having to name what a stray entry would be
    /// called.
    fn entry_count(archive_dir: &Path) -> usize {
        fs::read_dir(archive_dir)
            .expect("the archive directory")
            .count()
    }

    fn record(session_id: &str, line: &str, timestamp: i64) -> ArchiveRecord {
        ArchiveRecord {
            session_id: session_id.to_string(),
            stream: LogStream::Stdout,
            line: line.to_string(),
            timestamp,
        }
    }

    /// Settle a whole batch the way a successful pump does: begin it, then take and
    /// release every record in order.
    ///
    /// The queue-level replacement for the deleted `drain`. A test that only wants to
    /// see what the queue would hand over needs the full protocol, because taking
    /// without settling is exactly what leaves a record reserved — so this is also the
    /// only shape a queue test may use to reach an empty queue.
    fn settle_all(queue: &mut ArchiveQueue) -> Vec<QueuedRecord> {
        queue.begin_batch();
        let mut settled = Vec::new();
        while let Some(item) = queue.take_front() {
            queue.release(&item);
            settled.push(item);
        }
        settled
    }

    /// What a settled batch handed over, for a single-session test: the text of each
    /// record and the gap it carries, in order.
    fn lines_and_gaps(queue: &mut ArchiveQueue) -> Vec<(String, Option<DropCounters>)> {
        settle_all(queue)
            .into_iter()
            .map(|item| (item.record.line, item.gap_before))
            .collect()
    }

    /// Nothing of `session_id` is left in the writer's queue.
    ///
    /// The unbounded half of every close test, and the half a file and a row cannot
    /// show. A record that reached the queue after its session closed leaves an
    /// entry behind carrying `pending` and `dropped` history that no file will ever
    /// explain, and nothing removes it: only [`ArchiveQueue::finish_session`] does,
    /// and the close it belonged to has already run. One entry per closed session,
    /// for the life of the process, is a leak that grows with sessions rather than
    /// with open sessions — the same shape as the bug the lifecycle patch fixed.
    ///
    /// Reaching into the private field directly is the point: a production accessor
    /// for it would be an invitation to depend on it, and the test module can see it
    /// without one.
    fn assert_no_queue_entry(writer: &ArchiveWriter, session_id: &str) {
        assert!(
            !writer
                .queue
                .lock()
                .expect("the queue")
                .sessions
                .contains_key(session_id),
            "{session_id} still has a queue entry after its close"
        );
    }

    /// Every name this build could not have generated. The list is the test's
    /// half of the additive rule: the rule matches the generator, so each of
    /// these must be refused without the rule naming it.
    fn rejected_file_names() -> Vec<String> {
        let uuid = SESSION_A;
        vec![
            String::new(),
            ".".into(),
            "..".into(),
            format!("C:\\{uuid}.jsonl"),
            format!("\\\\?\\C:\\{uuid}.jsonl"),
            format!("\\\\server\\share\\{uuid}.jsonl"),
            format!("/{uuid}.jsonl"),
            format!("\\{uuid}.jsonl"),
            format!("C:{uuid}.jsonl"),
            format!("..\\{uuid}.jsonl"),
            format!("../{uuid}.jsonl"),
            format!("logs\\{uuid}.jsonl"),
            format!("logs/{uuid}.jsonl"),
            format!("{uuid}.jsonl\\"),
            format!("{uuid}.jsonl/"),
            format!("{uuid}.jsonl "),
            format!("{uuid}.jsonl."),
            format!("{uuid}.jsonl:stream"),
            format!("{uuid}.jsonl.txt"),
            format!("{uuid}.JSONL"),
            format!("{uuid}"),
            SESSION_A.to_uppercase(),
            format!("{}.jsonl", SESSION_A.to_uppercase()),
            format!("{}.jsonl", &uuid[..uuid.len() - 1]),
            format!("{uuid}x.jsonl"),
            "CON.jsonl".into(),
            "runcove.sqlite3".into(),
            "notes.txt".into(),
        ]
    }

    /// The filesystem the tests drive: real files under a `TempDir`, so Windows
    /// path behavior is the real thing, wrapped so a chosen write or `sync_data`
    /// fails on demand.
    ///
    /// Injection is by call count or by entry name, never by timing, so no test
    /// sleeps and no test races. Two calls can be held instead of failed —
    /// `sync_data` and `write` — and holding one is a rendezvous rather than a
    /// delay: see [`CallGate`].
    #[derive(Default)]
    struct TestFs {
        state: Arc<Mutex<TestFsState>>,
    }

    #[derive(Default)]
    struct TestFsState {
        /// Fail the nth `write` across all handles, 1-based. `0` never fails.
        fail_write_at: usize,
        writes: usize,
        /// Entry names whose next `write` must go wrong, and how. Keyed by name
        /// rather than by call order, so a test with two open sessions says which
        /// file fails instead of depending on which one the writer reached first.
        write_faults: BTreeMap<String, WriteFault>,
        fail_sync: bool,
        /// Refuse every `remove_file`, as a read-only file or a handle another
        /// process holds open would.
        fail_remove: bool,
        /// Entry names whose metadata must fail, as a file another process holds
        /// exclusively or one whose permissions deny this user would. Keyed by
        /// name, never by call order, so the injection is deterministic.
        fail_metadata_for: BTreeSet<String>,
        /// Entry names whose reads must fail, as a failing disk would.
        fail_read_for: BTreeSet<String>,
        /// Entry names that must read as if they ended here, whatever the file
        /// holds. The one way a test can make a page find the file shorter than
        /// the length it measured before opening it.
        read_ends_at: BTreeMap<String, u64>,
        /// Paths this filesystem was asked to remove, in order, whether or not the
        /// removal succeeded.
        removed: Vec<PathBuf>,
        /// The one `sync_data` a test is holding, if any.
        sync_gate: Option<CallGate>,
        /// The one `write` a test is holding, if any. A separate slot from
        /// `sync_gate` so a test can hold either without disarming the other, and
        /// so one test can hold both if it ever needs to.
        write_gate: Option<CallGate>,
    }

    /// What an injected by-name write failure does to the file it names.
    ///
    /// Sticky, and deliberately so: a session whose write has failed is closed and
    /// never written to again, so nothing in the writer can recover this file's
    /// writes and no test needs a switch that says otherwise.
    #[derive(Clone, Copy)]
    enum WriteFault {
        /// Take this many bytes of the next `write`, then behave as `Fail` for
        /// every `write` and `flush` after it.
        ///
        /// A short write is legal — `io::Write::write` may accept less than it was
        /// given — so this is the only seam that can leave a fragment of a line on
        /// the disk and then take the rest away.
        ShortThenFail(usize),
        /// Fail every `write` and `flush` on this file.
        Fail,
    }

    /// A one-shot rendezvous inside one named call on one file, so a test can hold
    /// a thread exactly where the writer is between two things it must get right,
    /// do something else on another thread, and then let it finish.
    ///
    /// Two calls can be gated. Holding `sync_data` stops a close where it has
    /// flushed what it accepted and has not yet released the handle. Holding
    /// `write` stops a pump *inside* the write, which is the only place a test can
    /// stand between the moment records left the queue and the moment the session's
    /// state changes — a distinction the sync gate cannot make, because by
    /// `sync_data` the state has changed under every ordering.
    ///
    /// Named after the file and taken out of the state when it fires, so a second
    /// call — that session's or another's — runs straight through. Nothing here
    /// sleeps: each side blocks until the other arrives.
    struct CallGate {
        file_name: String,
        reached: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    impl CallGate {
        /// A gate on one file's next `call`, and the test's end of it.
        fn pair(call: &'static str, file_name: &str) -> (Self, CallHold) {
            let (reached, reached_rx) = mpsc::channel();
            let (release, release_rx) = mpsc::channel();
            (
                Self {
                    file_name: file_name.to_string(),
                    reached,
                    release: release_rx,
                },
                CallHold {
                    call,
                    reached: reached_rx,
                    release,
                },
            )
        }
    }

    /// The gate armed on `slot` for this file, taken. `None` when the test armed
    /// none, or armed one for another file.
    ///
    /// Taking it is what makes a gate fire exactly once, and taking it here — with
    /// the state lock held by the caller and the pause happening after that lock is
    /// dropped — is what keeps no lock held across the rendezvous.
    fn take_gate(slot: &mut Option<CallGate>, file_name: &str) -> Option<CallGate> {
        match slot {
            Some(gate) if gate.file_name == file_name => slot.take(),
            _ => None,
        }
    }

    /// The test's end of a [`CallGate`].
    struct CallHold {
        /// Which call is held, for the message when the far thread never gets
        /// there.
        call: &'static str,
        reached: mpsc::Receiver<()>,
        release: mpsc::Sender<()>,
    }

    /// How long [`CallHold::wait_for`] waits before looking at whether the thread
    /// it is waiting for is still alive. It is a liveness check, not a delay: on
    /// the ordinary path the rendezvous wakes the wait, and nothing a test asserts
    /// depends on this number.
    const GATE_POLL: Duration = Duration::from_millis(25);

    impl CallHold {
        /// Block until the thread being held is inside the seam, then hand it back.
        ///
        /// A thread that ends without getting there — which is what an
        /// unimplemented body does — has its panic re-raised here, so the failure
        /// the test reports is the missing body instead of a rendezvous that was
        /// never going to come. Liveness is consulted only when the channel is
        /// empty, and the channel is drained once more before concluding, so which
        /// of the two happened first cannot change the answer.
        fn wait_for(
            &self,
            held: thread::JoinHandle<AppResult<()>>,
        ) -> thread::JoinHandle<AppResult<()>> {
            loop {
                if self.reached.recv_timeout(GATE_POLL).is_ok() {
                    return held;
                }
                if held.is_finished() {
                    if self.reached.try_recv().is_ok() {
                        return held;
                    }
                    // Nobody is left to wait on the far side; let go before
                    // reporting, so no later call blocks on this gate.
                    let _ = self.release.send(());
                    match held.join() {
                        Ok(outcome) => panic!(
                            "the held thread ended without reaching {}: {outcome:?}",
                            self.call
                        ),
                        Err(panic) => resume_unwind(panic),
                    }
                }
            }
        }

        /// Let the held call return.
        fn let_go(self) {
            let _ = self.release.send(());
        }
    }

    impl TestFs {
        fn shared() -> Arc<Self> {
            Arc::new(Self::default())
        }

        /// Make the nth `write` fail, 1-based.
        fn fail_write_at(&self, nth: usize) {
            self.state.lock().expect("test filesystem").fail_write_at = nth;
        }

        /// Make this one file's `write` fail, and every `write` and `flush` on it
        /// after that.
        ///
        /// By name rather than by call count, so a test with more than one open
        /// session names the file that fails instead of depending on the order the
        /// writer happens to reach them. The record has to be longer than
        /// [`WRITE_BUFFER_BYTES`] to reach a `write` at all; a shorter one surfaces
        /// at the flush, which fails too.
        fn fail_write_of(&self, file_name: &str) {
            self.state
                .lock()
                .expect("test filesystem")
                .write_faults
                .insert(file_name.to_string(), WriteFault::Fail);
        }

        /// Let this one file's next `write` take `accepted` bytes and fail
        /// everything after it. See [`WriteFault::ShortThenFail`].
        fn short_write_then_fail_of(&self, file_name: &str, accepted: usize) {
            self.state
                .lock()
                .expect("test filesystem")
                .write_faults
                .insert(file_name.to_string(), WriteFault::ShortThenFail(accepted));
        }

        fn fail_sync(&self) {
            self.state.lock().expect("test filesystem").fail_sync = true;
        }

        fn fail_remove(&self) {
            self.state.lock().expect("test filesystem").fail_remove = true;
        }

        fn allow_remove(&self) {
            self.state.lock().expect("test filesystem").fail_remove = false;
        }

        /// Make both `list_dir` and `entry_info` refuse this one entry, so a test
        /// does not depend on which of the two the sweep happens to call.
        fn fail_metadata_for(&self, name: &str) {
            self.state
                .lock()
                .expect("test filesystem")
                .fail_metadata_for
                .insert(name.to_string());
        }

        /// Whether this entry's metadata is one of the refused ones.
        fn metadata_refused(&self, name: &str) -> bool {
            self.state
                .lock()
                .expect("test filesystem")
                .fail_metadata_for
                .contains(name)
        }

        /// Make every read of this one entry fail, as a failing disk would.
        fn fail_read_for(&self, name: &str) {
            self.state
                .lock()
                .expect("test filesystem")
                .fail_read_for
                .insert(name.to_string());
        }

        /// Make this one entry read as if it ended at `length`.
        ///
        /// A page measures the file's length before it opens it, so this is how a
        /// test puts the file's bytes and that measurement out of step — what a
        /// truncation from outside RunCove would do between the two.
        fn read_ends_at(&self, name: &str, length: u64) {
            self.state
                .lock()
                .expect("test filesystem")
                .read_ends_at
                .insert(name.to_string(), length);
        }

        fn removed(&self) -> Vec<PathBuf> {
            self.state.lock().expect("test filesystem").removed.clone()
        }

        /// Hold the next `sync_data` on this one file until the returned handle
        /// lets it go. See [`CallGate`].
        fn hold_sync_of(&self, file_name: &str) -> CallHold {
            let (gate, hold) = CallGate::pair("sync_data", file_name);
            self.state.lock().expect("test filesystem").sync_gate = Some(gate);
            hold
        }

        /// Hold the next `write` to this one file until the returned handle lets it
        /// go. See [`CallGate`].
        ///
        /// The record being written has to be longer than [`WRITE_BUFFER_BYTES`] to
        /// reach a `write` inside `pump` at all: a shorter one sits in the buffer
        /// until the flush, by which time the pump is over and the window this gate
        /// exists for has closed.
        fn hold_write_of(&self, file_name: &str) -> CallHold {
            let (gate, hold) = CallGate::pair("write", file_name);
            self.state.lock().expect("test filesystem").write_gate = Some(gate);
            hold
        }
    }

    struct TestFile {
        inner: fs::File,
        /// This handle's own file name, so a [`CallGate`] can name the file it holds
        /// instead of counting calls across every handle.
        name: String,
        state: Arc<Mutex<TestFsState>>,
    }

    impl io::Write for TestFile {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            // The gate comes first, and before the injected failure below: a test
            // that holds this write in order to watch what happens when it fails
            // would never reach the rendezvous if the failure returned first.
            let gate = {
                let mut state = self.state.lock().expect("test filesystem");
                take_gate(&mut state.write_gate, &self.name)
            };
            if let Some(gate) = gate {
                let _ = gate.reached.send(());
                let _ = gate.release.recv();
            }
            {
                let mut state = self.state.lock().expect("test filesystem");
                state.writes += 1;
                if state.fail_write_at != 0 && state.writes == state.fail_write_at {
                    return Err(io::Error::other("injected write failure"));
                }
            }
            // Read after the count and acted on outside the lock: injecting by name
            // and injecting by order leave the same call log behind, and no lock is
            // held across the real write below.
            let fault = {
                let mut state = self.state.lock().expect("test filesystem");
                let fault = state.write_faults.get(&self.name).copied();
                if fault.is_some() {
                    // A short write arms the failure for everything after it.
                    state
                        .write_faults
                        .insert(self.name.clone(), WriteFault::Fail);
                }
                fault
            };
            match fault {
                Some(WriteFault::Fail) => Err(io::Error::other("injected write failure")),
                Some(WriteFault::ShortThenFail(accepted)) => {
                    let accepted = accepted.min(buf.len());
                    // `write_all`, so exactly this many bytes reach the disk whatever
                    // the real filesystem would have taken in one call.
                    self.inner.write_all(&buf[..accepted])?;
                    Ok(accepted)
                }
                None => self.inner.write(buf),
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            // A failure injected for this file has to reach a record that never left
            // the buffer too, or it could only ever be seen on a line longer than
            // [`WRITE_BUFFER_BYTES`].
            let fault = self
                .state
                .lock()
                .expect("test filesystem")
                .write_faults
                .get(&self.name)
                .copied();
            if matches!(fault, Some(WriteFault::Fail)) {
                return Err(io::Error::other("injected flush failure"));
            }
            self.inner.flush()
        }
    }

    impl ArchiveFile for TestFile {
        fn sync_data(&mut self) -> io::Result<()> {
            // Taken out from under the lock: the pause below must hold no lock, and
            // taking it is what makes the gate fire exactly once.
            let gate = {
                let mut state = self.state.lock().expect("test filesystem");
                take_gate(&mut state.sync_gate, &self.name)
            };
            if let Some(gate) = gate {
                let _ = gate.reached.send(());
                let _ = gate.release.recv();
            }
            if self.state.lock().expect("test filesystem").fail_sync {
                return Err(io::Error::other("injected sync_data failure"));
            }
            self.inner.sync_data()
        }
    }

    /// One archive file open for reading, wrapped so a test can fail its reads or
    /// make it end short of the length the page measured.
    struct TestReadFile {
        inner: fs::File,
        name: String,
        state: Arc<Mutex<TestFsState>>,
    }

    impl ArchiveReadFile for TestReadFile {
        fn fill_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
            let ends_at = {
                let state = self.state.lock().expect("test filesystem");
                if state.fail_read_for.contains(&self.name) {
                    return Err(io::Error::other("injected read failure"));
                }
                state.read_ends_at.get(&self.name).copied()
            };

            // Held by nothing below: the real read happens outside the lock, the
            // same way every other call on this double does its work.
            self.inner.seek(io::SeekFrom::Start(offset))?;
            let room = match ends_at {
                Some(ends_at) => buf
                    .len()
                    .min(usize::try_from(ends_at.saturating_sub(offset)).unwrap_or(usize::MAX)),
                None => buf.len(),
            };
            let mut filled = 0;
            while filled < room {
                match self.inner.read(&mut buf[filled..room]) {
                    Ok(0) => break,
                    Ok(taken) => filled += taken,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(filled)
        }
    }

    impl ArchiveFs for TestFs {
        fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
            fs::create_dir_all(dir)
        }

        fn list_dir(&self, dir: &Path) -> io::Result<Vec<ListedEntry>> {
            let mut entries = Vec::new();
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if self.metadata_refused(&name) {
                    entries.push(Err(UnreadableEntry {
                        name,
                        reason: "injected metadata failure".into(),
                    }));
                    continue;
                }
                // Explicitly non-following, and the same call `entry_info` makes,
                // so listing a directory and stating one path inside it cannot
                // disagree about what an entry is.
                let metadata = fs::symlink_metadata(entry.path())?;
                entries.push(Ok(DirEntryInfo {
                    name,
                    kind: kind_of(&metadata),
                    len: metadata.len(),
                }));
            }
            entries.sort_by(|left, right| listed_name(left).cmp(listed_name(right)));
            Ok(entries)
        }

        fn entry_info(&self, path: &Path) -> io::Result<DirEntryInfo> {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if self.metadata_refused(&name) {
                return Err(io::Error::other("injected metadata failure"));
            }
            let metadata = fs::symlink_metadata(path)?;
            Ok(DirEntryInfo {
                name,
                kind: kind_of(&metadata),
                len: metadata.len(),
            })
        }

        fn create_new(&self, path: &Path) -> io::Result<Box<dyn ArchiveFile>> {
            let inner = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?;
            Ok(Box::new(TestFile {
                inner,
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                state: Arc::clone(&self.state),
            }))
        }

        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            fs::read_to_string(path)
        }

        fn open_read(&self, path: &Path) -> io::Result<Box<dyn ArchiveReadFile>> {
            Ok(Box::new(TestReadFile {
                inner: fs::File::open(path)?,
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                state: Arc::clone(&self.state),
            }))
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            {
                let mut state = self.state.lock().expect("test filesystem");
                state.removed.push(path.to_path_buf());
                if state.fail_remove {
                    return Err(io::Error::other("injected remove_file failure"));
                }
            }
            fs::remove_file(path)
        }
    }

    /// The name of a listed entry, readable or not, so a listing can be ordered
    /// and compared without caring which it is.
    fn listed_name(entry: &ListedEntry) -> &str {
        match entry {
            Ok(info) => &info.name,
            Err(unreadable) => &unreadable.name,
        }
    }

    /// Decided from non-following metadata, so a link is reported as a link
    /// instead of as whatever it points at.
    ///
    /// The attribute bit comes first because on Windows `is_symlink` is true only
    /// for the two name-surrogate reparse tags, and [`EntryKind::ReparsePoint`]
    /// promises every tag. The double holds itself to the rule
    /// [`ArchiveFs::entry_info`] states, so no test can pass because the double
    /// was the more permissive of the two.
    fn kind_of(metadata: &fs::Metadata) -> EntryKind {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;

            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return EntryKind::ReparsePoint;
            }
        }

        if metadata.file_type().is_symlink() {
            EntryKind::ReparsePoint
        } else if metadata.is_dir() {
            EntryKind::Directory
        } else {
            EntryKind::File
        }
    }

    /// The index the writer tests observe: it records every call in order, so a
    /// test can assert the row transitions without a database. The SQLite
    /// constraints behind these rows already have their own tests in `storage`.
    ///
    /// Any one of its write methods can be made to fail, so a test can ask what
    /// the writer leaves behind when the database refuses a row while the file it
    /// belongs to already exists.
    #[derive(Default)]
    struct TestIndex {
        state: Mutex<TestIndexState>,
    }

    #[derive(Default)]
    struct TestIndexState {
        rows: BTreeMap<String, ArchiveRow>,
        calls: Vec<String>,
        /// Write methods that must refuse, by method name. Injection is by call,
        /// never by timing.
        failing: BTreeSet<String>,
    }

    impl TestIndex {
        fn shared() -> Arc<Self> {
            Arc::new(Self::default())
        }

        /// Put a row in place as if an earlier run had written it.
        fn seed(&self, row: ArchiveRow) {
            let mut state = self.state.lock().expect("test index");
            state.rows.insert(row.session_id.clone(), row);
        }

        /// Make one write method fail on every call until [`TestIndex::allow`].
        fn fail(&self, method: &str) {
            self.state
                .lock()
                .expect("test index")
                .failing
                .insert(method.to_string());
        }

        fn allow(&self, method: &str) {
            self.state
                .lock()
                .expect("test index")
                .failing
                .remove(method);
        }

        /// The first thing every write method calls. A refused write logs the
        /// refusal and leaves `rows` untouched, so a test can tell a write that
        /// never happened from one that half happened.
        fn refuse(&self, method: &str, session_id: &str) -> AppResult<()> {
            let mut state = self.state.lock().expect("test index");
            if !state.failing.contains(method) {
                return Ok(());
            }
            state.calls.push(format!("refused:{method}:{session_id}"));
            Err(invalid(format!("injected {method} failure")))
        }

        fn snapshot(&self, session_id: &str) -> Option<ArchiveRow> {
            self.state
                .lock()
                .expect("test index")
                .rows
                .get(session_id)
                .cloned()
        }

        /// `(status, reason)` of one row, for the common assertion.
        fn state_of(&self, session_id: &str) -> Option<(String, Option<String>)> {
            self.snapshot(session_id)
                .map(|row| (row.status, row.reason))
        }

        fn calls(&self) -> Vec<String> {
            self.state.lock().expect("test index").calls.clone()
        }

        fn log(&self, call: String) {
            self.state.lock().expect("test index").calls.push(call);
        }
    }

    /// A row as an earlier run would have left it.
    fn seeded_row(
        session_id: &str,
        status: &str,
        reason: Option<&str>,
        byte_size: i64,
    ) -> ArchiveRow {
        ArchiveRow {
            session_id: session_id.to_string(),
            file_name: name_of(session_id),
            status: status.to_string(),
            reason: reason.map(str::to_string),
            counters: ArchiveCounters {
                line_count: 0,
                byte_size,
                dropped_lines: 0,
                dropped_bytes: 0,
            },
            started_at: 1_000,
            ended_at: if status == "writing" {
                None
            } else {
                Some(2_000)
            },
        }
    }

    /// The database strings, written out here independently of the code under
    /// test, so a test compares against an expectation rather than against
    /// itself.
    fn status_text(status: ArchiveStatus) -> &'static str {
        match status {
            ArchiveStatus::Writing => "writing",
            ArchiveStatus::Complete => "complete",
            ArchiveStatus::Partial => "partial",
            ArchiveStatus::Removed => "removed",
        }
    }

    fn reason_text(reason: ArchiveReason) -> &'static str {
        match reason {
            ArchiveReason::WriteError => "write-error",
            ArchiveReason::QuotaExceeded => "quota-exceeded",
            ArchiveReason::QueueOverflow => "queue-overflow",
            ArchiveReason::Interrupted => "interrupted",
            ArchiveReason::UserDisabled => "user-disabled",
            ArchiveReason::QuotaEvicted => "quota-evicted",
            ArchiveReason::UserDeleted => "user-deleted",
            ArchiveReason::FileMissing => "file-missing",
        }
    }

    impl ArchiveIndex for TestIndex {
        fn insert_writing(
            &self,
            session_id: &str,
            file_name: &str,
            started_at: i64,
        ) -> AppResult<()> {
            self.refuse("insert_writing", session_id)?;
            {
                let mut state = self.state.lock().expect("test index");
                state.rows.insert(
                    session_id.to_string(),
                    ArchiveRow {
                        session_id: session_id.to_string(),
                        file_name: file_name.to_string(),
                        status: "writing".into(),
                        reason: None,
                        counters: ArchiveCounters::default(),
                        started_at,
                        ended_at: None,
                    },
                );
            }
            self.log(format!("insert_writing:{session_id}:{file_name}"));
            Ok(())
        }

        fn update_counters(&self, session_id: &str, counters: ArchiveCounters) -> AppResult<()> {
            self.refuse("update_counters", session_id)?;
            {
                let mut state = self.state.lock().expect("test index");
                if let Some(row) = state.rows.get_mut(session_id) {
                    row.counters = counters;
                }
            }
            self.log(format!(
                "update_counters:{session_id}:{}",
                counters.byte_size
            ));
            Ok(())
        }

        fn close(
            &self,
            session_id: &str,
            status: ArchiveStatus,
            reason: Option<ArchiveReason>,
            counters: ArchiveCounters,
            ended_at: i64,
        ) -> AppResult<()> {
            self.refuse("close", session_id)?;
            {
                let mut state = self.state.lock().expect("test index");
                if let Some(row) = state.rows.get_mut(session_id) {
                    row.status = status_text(status).into();
                    row.reason = reason.map(|reason| reason_text(reason).to_string());
                    row.counters = counters;
                    row.ended_at = Some(ended_at);
                }
            }
            self.log(format!(
                "close:{session_id}:{}:{}",
                status_text(status),
                reason.map(reason_text).unwrap_or("-")
            ));
            Ok(())
        }

        fn mark_removed(
            &self,
            session_id: &str,
            reason: ArchiveReason,
            ended_at: i64,
        ) -> AppResult<()> {
            self.refuse("mark_removed", session_id)?;
            {
                let mut state = self.state.lock().expect("test index");
                if let Some(row) = state.rows.get_mut(session_id) {
                    row.status = "removed".into();
                    row.reason = Some(reason_text(reason).into());
                    row.ended_at = Some(ended_at);
                }
            }
            self.log(format!("mark_removed:{session_id}:{}", reason_text(reason)));
            Ok(())
        }

        fn rows(&self) -> AppResult<Vec<ArchiveRow>> {
            Ok(self
                .state
                .lock()
                .expect("test index")
                .rows
                .values()
                .cloned()
                .collect())
        }

        fn row(&self, session_id: &str) -> AppResult<Option<ArchiveRow>> {
            Ok(self.snapshot(session_id))
        }
    }

    /// A writer over a temporary archive directory.
    fn test_writer(
        archive_dir: &Path,
        fs: Arc<TestFs>,
        index: Arc<TestIndex>,
        bounds: QueueBounds,
        limits: QuotaLimits,
    ) -> AppResult<(ArchiveWriter, SweepReport)> {
        ArchiveWriter::initialize(archive_dir.to_path_buf(), fs, index, bounds, limits, 10_000)
    }

    /// Bounds and caps a test can cross in a few short records.
    fn small_bounds() -> QueueBounds {
        QueueBounds {
            session_records: 2,
            session_bytes: 64,
            total_records: 3,
            total_bytes: 96,
        }
    }

    /// Bounds and caps wide enough that a test crossing them would be a bug in
    /// the test, used where the subject is something other than a bound.
    fn roomy_bounds() -> QueueBounds {
        QueueBounds {
            session_records: 64,
            session_bytes: 1 << 20,
            total_records: 128,
            total_bytes: 2 << 20,
        }
    }

    fn roomy_limits() -> QuotaLimits {
        QuotaLimits {
            session_bytes: 1 << 20,
            total_bytes: 4 << 20,
        }
    }

    /// Room for exactly one queued record in total, and bytes to spare.
    ///
    /// This is how a test reads queue state it cannot see: a record the writer
    /// should never have queued is not invisible if it is holding the only slot the
    /// next live session needs. That session's loss is the assertion.
    fn one_slot_bounds() -> QueueBounds {
        QueueBounds {
            session_records: 2,
            session_bytes: 1 << 20,
            total_records: 1,
            total_bytes: 1 << 20,
        }
    }

    // Group 1: the file name rule and directory containment.

    #[test]
    fn the_file_name_rule_accepts_only_a_name_this_build_generates() {
        let generated = archive_file_name(SESSION_A).expect("a generated session id");
        assert_eq!(generated, name_of(SESSION_A));

        // The positive control: a rule that refuses everything is not a rule.
        assert!(is_archive_file_name(&generated));
        assert!(is_archive_file_name(&name_of(SESSION_B)));

        for rejected in rejected_file_names() {
            assert!(!is_archive_file_name(&rejected), "accepted {rejected:?}");
        }

        assert!(archive_file_name("not-a-uuid").is_err());
        assert!(archive_file_name(&format!("..\\{SESSION_A}")).is_err());
    }

    #[test]
    fn resolving_an_archive_path_joins_only_a_generated_name() {
        let (_temp, archive_dir) = temp_data_dir();
        let generated = name_of(SESSION_A);

        let resolved = resolve_archive_path(&archive_dir, &generated).expect("a generated name");
        assert_eq!(resolved, archive_dir.join(&generated));
        assert_eq!(resolved.parent(), Some(archive_dir.as_path()));

        for rejected in rejected_file_names() {
            assert!(
                resolve_archive_path(&archive_dir, &rejected).is_err(),
                "resolved {rejected:?}"
            );
        }

        // Resolving is a decision about a name, so it creates nothing.
        assert!(!archive_dir.exists());
    }

    #[test]
    fn the_read_gate_accepts_an_ordinary_file_and_reports_its_length() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        let name = name_of(SESSION_A);
        fs::write(archive_dir.join(&name), b"{\"t\":1}\n").expect("an archive file");
        let seam = TestFs::shared();

        let (path, len) = resolve_ordinary_archive_file(&*seam, &archive_dir, &name)
            .expect("an ordinary archive file");

        assert_eq!(path, archive_dir.join(&name));
        assert_eq!(len, 8);
    }

    #[test]
    fn the_read_gate_refuses_a_directory_named_like_an_archive() {
        let (_temp, archive_dir) = temp_data_dir();
        let name = name_of(SESSION_A);
        let impostor = archive_dir.join(&name);
        fs::create_dir_all(&impostor).expect("a directory named like an archive");
        let seam = TestFs::shared();

        assert!(resolve_ordinary_archive_file(&*seam, &archive_dir, &name).is_err());

        // An anomaly is reported, never repaired and never deleted.
        assert!(impostor.is_dir());
        assert!(seam.removed().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn the_read_gate_refuses_a_symlinked_archive_entry() {
        let (temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        let target = temp.path().join("secret.txt");
        fs::write(&target, b"private").expect("a file outside the archive directory");
        let name = name_of(SESSION_A);
        std::os::windows::fs::symlink_file(&target, archive_dir.join(&name))
            .expect("creating a symbolic link needs Developer Mode or an elevated shell");
        let seam = TestFs::shared();

        assert!(resolve_ordinary_archive_file(&*seam, &archive_dir, &name).is_err());

        // The link is not followed, so its target keeps its bytes.
        assert_eq!(fs::read(&target).expect("the target"), b"private");
        assert!(seam.removed().is_empty());
    }

    #[test]
    fn the_read_gate_refuses_a_name_that_would_escape_the_archive_directory() {
        let (temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        let neighbour = temp.path().join(name_of(SESSION_A));
        fs::write(&neighbour, b"neighbour").expect("a file beside the archive directory");
        let seam = TestFs::shared();

        for escape in [
            format!("..\\{}", name_of(SESSION_A)),
            format!("../{}", name_of(SESSION_A)),
            format!("{}\\..\\{}", ARCHIVE_DIR_NAME, name_of(SESSION_A)),
        ] {
            assert!(
                resolve_ordinary_archive_file(&*seam, &archive_dir, &escape).is_err(),
                "resolved {escape:?}"
            );
        }

        assert_eq!(fs::read(&neighbour).expect("the neighbour"), b"neighbour");
        assert!(seam.removed().is_empty());
    }

    #[test]
    fn deleting_an_archive_never_escapes_the_archive_directory() {
        let (temp, archive_dir) = temp_data_dir();
        let neighbour = temp.path().join(name_of(SESSION_A));
        fs::write(&neighbour, b"neighbour").expect("a file beside the archive directory");
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        // The escape can only come from the database now that `delete` takes no
        // file name, so the row is the one carrying it.
        let mut escaping = seeded_row(SESSION_A, "complete", None, 9);
        escaping.file_name = format!("..\\{}", name_of(SESSION_A));
        index.seed(escaping.clone());
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        assert!(writer.delete(SESSION_A).is_err());

        assert_eq!(fs::read(&neighbour).expect("the neighbour"), b"neighbour");
        assert!(seam.removed().is_empty());
        // A refused delete leaves the row exactly as it was.
        assert_eq!(index.snapshot(SESSION_A), Some(escaping));
    }

    #[test]
    fn a_row_may_only_name_the_file_its_own_session_generates() {
        // The name this build would have generated is the only one accepted.
        let owned = seeded_row(SESSION_A, "complete", None, 10);
        assert_eq!(
            verified_file_name(SESSION_A, &owned).expect("the generated name"),
            name_of(SESSION_A)
        );

        // Another session's archive, reached through this session's row.
        let mut borrowed = seeded_row(SESSION_A, "complete", None, 10);
        borrowed.file_name = name_of(SESSION_B);
        assert!(verified_file_name(SESSION_A, &borrowed).is_err());

        // A row belonging to a different session than the one asked about.
        let other = seeded_row(SESSION_B, "complete", None, 10);
        assert!(verified_file_name(SESSION_A, &other).is_err());

        // A session id this build could not have produced has no valid name at
        // all, whatever the row says.
        let mut shouting = seeded_row(SESSION_A, "complete", None, 10);
        shouting.session_id = SESSION_A.to_uppercase();
        shouting.file_name = name_of(&SESSION_A.to_uppercase());
        assert!(verified_file_name(&SESSION_A.to_uppercase(), &shouting).is_err());

        // Every name this build could not have generated, carried by a row that
        // is otherwise ordinary.
        for name in rejected_file_names() {
            let mut row = seeded_row(SESSION_A, "complete", None, 10);
            row.file_name = name.clone();
            assert!(
                verified_file_name(SESSION_A, &row).is_err(),
                "accepted {name:?}"
            );
        }
    }

    #[test]
    fn the_sweep_deletes_nothing_outside_the_archive_directory() {
        let (temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");

        // A file next to the archive directory, one that looks exactly like an
        // archive, and the database the data directory holds.
        let neighbour = temp.path().join(name_of(SESSION_A));
        fs::write(&neighbour, b"neighbour").expect("neighbour");
        let database = temp.path().join("runcove.sqlite3");
        fs::write(&database, b"database").expect("database");

        // Inside the directory: an archive-shaped name one level down, which a
        // recursive sweep would find and this one must not.
        let nested = archive_dir.join("nested");
        fs::create_dir_all(&nested).expect("nested directory");
        let buried = nested.join(name_of(SESSION_B));
        fs::write(&buried, b"buried").expect("buried");

        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (_writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        assert_eq!(fs::read(&neighbour).expect("neighbour"), b"neighbour");
        assert_eq!(fs::read(&database).expect("database"), b"database");
        assert_eq!(fs::read(&buried).expect("buried"), b"buried");
        assert!(seam.removed().is_empty());
        assert_eq!(report.deleted_orphan_files, Vec::<String>::new());
        assert_eq!(
            report.anomalies.len(),
            1,
            "the nested directory is reported"
        );
    }

    // Group 2: the writer and one archive's lifecycle.

    #[test]
    fn archive_status_and_reason_round_trip_their_database_strings() {
        for status in [
            ArchiveStatus::Writing,
            ArchiveStatus::Complete,
            ArchiveStatus::Partial,
            ArchiveStatus::Removed,
        ] {
            let text = status_text(status);
            assert_eq!(status.as_str(), text);
            assert_eq!(ArchiveStatus::parse(text), Some(status));
        }

        for reason in [
            ArchiveReason::WriteError,
            ArchiveReason::QuotaExceeded,
            ArchiveReason::QueueOverflow,
            ArchiveReason::Interrupted,
            ArchiveReason::UserDisabled,
            ArchiveReason::QuotaEvicted,
            ArchiveReason::UserDeleted,
            ArchiveReason::FileMissing,
        ] {
            let text = reason_text(reason);
            assert_eq!(reason.as_str(), text);
            assert_eq!(ArchiveReason::parse(text), Some(reason));
        }

        // A value a newer build wrote is unknown, not an error to crash on.
        assert_eq!(ArchiveStatus::parse("archived"), None);
        assert_eq!(ArchiveReason::parse("disk-full"), None);
        assert_eq!(ArchiveStatus::parse("Writing"), None);
    }

    #[test]
    fn the_most_severe_partial_reason_wins() {
        use ArchiveReason::{QueueOverflow, QuotaExceeded, WriteError};

        assert_eq!(
            ArchiveReason::most_severe(QueueOverflow, WriteError),
            WriteError
        );
        assert_eq!(
            ArchiveReason::most_severe(WriteError, QueueOverflow),
            WriteError
        );
        assert_eq!(
            ArchiveReason::most_severe(QueueOverflow, QuotaExceeded),
            QuotaExceeded
        );
        assert_eq!(
            ArchiveReason::most_severe(WriteError, WriteError),
            WriteError
        );
    }

    /// The documented severity order, most severe first.
    const SEVERITY_ORDER: [ArchiveReason; 8] = [
        ArchiveReason::WriteError,
        ArchiveReason::QuotaExceeded,
        ArchiveReason::QueueOverflow,
        ArchiveReason::Interrupted,
        ArchiveReason::FileMissing,
        ArchiveReason::QuotaEvicted,
        ArchiveReason::UserDeleted,
        ArchiveReason::UserDisabled,
    ];

    /// Where a reason sits in the documented order, as an exhaustive match rather
    /// than a lookup, so a reason added without being placed in that order fails
    /// to compile instead of quietly ranking itself.
    fn documented_rank(reason: ArchiveReason) -> usize {
        match reason {
            ArchiveReason::WriteError => 0,
            ArchiveReason::QuotaExceeded => 1,
            ArchiveReason::QueueOverflow => 2,
            ArchiveReason::Interrupted => 3,
            ArchiveReason::FileMissing => 4,
            ArchiveReason::QuotaEvicted => 5,
            ArchiveReason::UserDeleted => 6,
            ArchiveReason::UserDisabled => 7,
        }
    }

    /// Every pair, checked against the documented order rather than against the
    /// private `severity` numbers: this fails if those numbers ever move without
    /// the documented order moving with them.
    #[test]
    fn the_most_severe_reason_is_the_documented_order_over_every_pair() {
        for (rank, reason) in SEVERITY_ORDER.into_iter().enumerate() {
            assert_eq!(
                documented_rank(reason),
                rank,
                "{reason:?} sits at two different places in the documented order"
            );
        }

        for first in SEVERITY_ORDER {
            for second in SEVERITY_ORDER {
                let winner = ArchiveReason::most_severe(first, second);
                let expected = if documented_rank(first) <= documented_rank(second) {
                    first
                } else {
                    second
                };
                assert_eq!(
                    winner, expected,
                    "most_severe({first:?}, {second:?}) must be whichever comes first in the documented order"
                );
                // Which of the two arrived first must never decide the answer: a
                // session collects its reasons in whatever order they happen.
                assert_eq!(winner, ArchiveReason::most_severe(second, first));
            }
        }
    }

    #[test]
    fn the_documented_bounds_are_the_ones_the_defaults_use() {
        assert_eq!(ARCHIVE_DIR_NAME, "run-log-archives");
        assert_eq!(ARCHIVE_FILE_EXTENSION, "jsonl");
        assert_eq!(SESSION_BYTE_CAP, 10 * 1024 * 1024);
        assert_eq!(TOTAL_BYTE_CAP, 200 * 1024 * 1024);
        assert_eq!(SESSION_QUEUE_RECORDS, 2_048);
        assert_eq!(SESSION_QUEUE_BYTES, 4 * 1024 * 1024);
        assert_eq!(TOTAL_QUEUE_RECORDS, 4_096);
        assert_eq!(TOTAL_QUEUE_BYTES, 8 * 1024 * 1024);
        assert_eq!(WRITE_BUFFER_BYTES, 64 * 1024);

        assert_eq!(
            QueueBounds::default(),
            QueueBounds {
                session_records: SESSION_QUEUE_RECORDS,
                session_bytes: SESSION_QUEUE_BYTES,
                total_records: TOTAL_QUEUE_RECORDS,
                total_bytes: TOTAL_QUEUE_BYTES,
            }
        );
        assert_eq!(
            QuotaLimits::default(),
            QuotaLimits {
                session_bytes: SESSION_BYTE_CAP,
                total_bytes: TOTAL_BYTE_CAP,
            }
        );
    }

    #[test]
    fn the_json_lines_record_carries_the_decoded_text_unchanged() {
        let line = "tab\there, \"quoted\", back\\slash, \u{fffd}, end";
        let encoded = encode_record(&ArchiveRecord {
            session_id: SESSION_A.to_string(),
            stream: LogStream::Stderr,
            line: line.to_string(),
            timestamp: 7,
        });

        // One record is one line, so nothing may contain a raw newline.
        assert!(!encoded.contains('\n'));
        let parsed: serde_json::Value =
            serde_json::from_str(&encoded).expect("one JSON object per line");
        assert_eq!(parsed["t"], 7);
        assert_eq!(parsed["s"], "stderr");
        assert_eq!(parsed["l"], line);
    }

    /// A captured line may carry the very characters that end lines. The encoding
    /// has to keep them inside the JSON string and out of the file's line
    /// structure, or one record would become two on the way in and neither of them
    /// would parse on the way out.
    #[test]
    fn a_carriage_return_in_the_text_never_becomes_a_line_of_its_own() {
        let line = "first\rsecond\r\nthird\nfourth\u{0}fifth";
        let encoded = encode_record(&ArchiveRecord {
            session_id: SESSION_A.to_string(),
            stream: LogStream::Stdout,
            line: line.to_string(),
            timestamp: 11,
        });

        // One record is one line: the only newline in the file is the one the
        // writer appends after this string.
        assert_eq!(encoded.lines().count(), 1);
        assert!(!encoded.contains('\n'));
        assert!(!encoded.contains('\r'));
        assert!(encoded.contains("\\r\\n"));

        let parsed: serde_json::Value =
            serde_json::from_str(&encoded).expect("one JSON object per line");
        assert_eq!(parsed["t"], 11);
        assert_eq!(parsed["s"], "stdout");
        // What a reader hands back is what the process printed, byte for byte.
        assert_eq!(parsed["l"], line);
    }

    #[test]
    fn a_session_writes_its_writing_row_then_its_lines_then_complete() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer.begin(SESSION_A, 10_001).expect("the session opens");

        // The row exists before the first record reaches the file, so an
        // interrupted session is always visible to the next sweep.
        assert_eq!(index.state_of(SESSION_A), Some(("writing".into(), None)));
        assert_eq!(
            index.calls().first().map(String::as_str),
            Some(format!("insert_writing:{SESSION_A}:{}", name_of(SESSION_A)).as_str())
        );

        writer.enqueue(record(SESSION_A, "first", 10_002));
        writer.enqueue(record(SESSION_A, "second", 10_003));
        writer.pump(10_004).expect("the queue drains");
        writer
            .close(SESSION_A, None, 10_005)
            .expect("the session closes");

        assert_eq!(index.state_of(SESSION_A), Some(("complete".into(), None)));
        assert!(!writer.is_open(SESSION_A));

        let text =
            fs::read_to_string(archive_dir.join(name_of(SESSION_A))).expect("the archive file");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(lines[0]).expect("JSON")["l"],
            "first"
        );

        // Every record ends with its newline, so a truncated archive never ends
        // in half a line.
        assert!(text.ends_with('\n'));

        let row = index.snapshot(SESSION_A).expect("the row");
        assert_eq!(row.counters.line_count, 2);
        assert_eq!(row.counters.byte_size, text.len() as i64);
        assert_eq!(row.counters.dropped_lines, 0);
        assert_eq!(row.counters.dropped_bytes, 0);
        assert_eq!(row.ended_at, Some(10_005));
    }

    #[test]
    fn a_failing_write_closes_only_that_session_partial_write_error() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer
            .begin(SESSION_A, 10_001)
            .expect("the first session opens");
        writer
            .begin(SESSION_B, 10_002)
            .expect("the second session opens");

        // Larger than the buffer, so the record reaches the file inside `pump`
        // instead of waiting for the close, which is what makes the injection
        // point deterministic without a sleep.
        let long_line = "x".repeat(WRITE_BUFFER_BYTES + 1);
        // By name, so the claim is "this session's disk failed" rather than
        // "whichever session wrote first failed".
        seam.fail_write_of(&name_of(SESSION_A));
        writer.enqueue(record(SESSION_A, &long_line, 10_003));
        writer.enqueue(record(SESSION_B, "unaffected", 10_004));

        // One session's disk error is not the writer's error.
        writer.pump(10_005).expect("the pump survives the failure");

        assert_eq!(
            index.state_of(SESSION_A),
            Some(("partial".into(), Some("write-error".into())))
        );
        assert!(!writer.is_open(SESSION_A));
        assert_eq!(index.state_of(SESSION_B), Some(("writing".into(), None)));
        assert!(writer.is_open(SESSION_B));

        // The failed archive keeps whatever reached the disk.
        assert!(archive_dir.join(name_of(SESSION_A)).is_file());
        assert!(seam.removed().is_empty());
    }

    /// A write that fails partway through a batch ends the session, and every record
    /// that session had accepted and not yet got onto the disk becomes one of its
    /// counted losses — including one a capture thread hands over while the pump is
    /// still running, because `enqueue` is accepted during a pump.
    ///
    /// No gap line can stand in for these losses: the file is the thing that just
    /// failed. The row's drop counters are the only record of them, which is exactly
    /// why they have to be exact rather than approximately right.
    ///
    /// The write is held rather than only failed, so the record arriving during the
    /// pump arrives inside the write it is racing, with no sleep anywhere.
    #[test]
    fn a_write_error_counts_every_record_it_could_not_persist() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer.begin(SESSION_A, 10_001).expect("the session opens");

        // Longer than the buffer, so it reaches a `write` inside the pump instead of
        // waiting for the close. That write is the one that fails.
        let long_line = "x".repeat(WRITE_BUFFER_BYTES + 1);
        seam.fail_write_at(1);
        writer.enqueue(record(SESSION_A, &long_line, 10_002));
        // Behind it in the same batch: drained with it, never written.
        writer.enqueue(record(SESSION_A, "second", 10_003));
        writer.enqueue(record(SESSION_A, "third", 10_004));

        let hold = seam.hold_write_of(&name_of(SESSION_A));
        let writer = Arc::new(writer);
        let pumping = {
            let writer = Arc::clone(&writer);
            thread::spawn(move || writer.pump(10_005))
        };
        let pumping = hold.wait_for(pumping);

        // Inside the failing write: accepted while the session is still pumping, so
        // it is one of this session's records and one of its losses.
        writer.enqueue(record(SESSION_A, "during the pump", 10_006));
        hold.let_go();
        pumping
            .join()
            .expect("the pump thread")
            .expect("one session's disk error is not the writer's error");

        assert_eq!(
            index.state_of(SESSION_A),
            Some(("partial".into(), Some("write-error".into())))
        );
        assert!(!writer.is_open(SESSION_A));

        // Nothing reached the disk: the injected failure returns before the bytes do,
        // and a session whose file has failed is never written to again.
        let path = archive_dir.join(name_of(SESSION_A));
        let on_disk = fs::metadata(&path).expect("the archive file").len();
        assert_eq!(on_disk, 0);

        // So every accepted record is accounted for in the one place left: the two
        // queued behind the failing write, the one whose write failed, and the one
        // that arrived while the pump was inside it.
        let row = index.snapshot(SESSION_A).expect("the row");
        assert_eq!(row.counters.line_count, 0);
        assert_eq!(row.counters.byte_size, on_disk as i64);
        assert_eq!(row.counters.dropped_lines, 4);
        assert_eq!(
            row.counters.dropped_bytes,
            (long_line.len() + "second".len() + "third".len() + "during the pump".len()) as i64
        );
        // The pump's own `now` closes the session: the writer reads no clock of its
        // own, which is what keeps a close deterministic in a test and consistent
        // with the tick that found the error in production.
        assert_eq!(row.ended_at, Some(10_005));
        assert!(seam.removed().is_empty());
    }

    /// A legal short write, then a failure. `io::Write::write` may accept less than it
    /// was given, so a real disk can leave a fragment of a line in the file and then
    /// refuse the rest — and those bytes are as real as any others.
    ///
    /// Both books have to name them. `byte_size` is what the file holds, so it is the
    /// fragment's length; the hard quota is a claim about the disk, so the same bytes
    /// are on it. Under-reporting either would let the archive grow past a cap the
    /// user set, one failed write at a time.
    ///
    /// The record is a loss all the same, and the two numbers say different things
    /// about it without contradicting each other: `dropped_bytes` counts the archived
    /// text nobody will ever read back, while the fragment on the disk is not a line —
    /// no reader can parse it, and the line count stays at zero. The seam is the only
    /// one that can produce this state; see [`WriteFault::ShortThenFail`].
    #[test]
    fn a_short_write_then_a_failure_counts_the_bytes_it_left_on_disk() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer.begin(SESSION_A, 10_001).expect("the session opens");

        // Several times the buffer, so the first `write` carries it and the remainder
        // after the short one is still large enough to need a second.
        let long_line = "x".repeat(4 * WRITE_BUFFER_BYTES);
        const ACCEPTED: usize = 4_096;
        seam.short_write_then_fail_of(&name_of(SESSION_A), ACCEPTED);
        writer.enqueue(record(SESSION_A, &long_line, 10_002));
        writer
            .pump(10_003)
            .expect("one session's disk error is not the writer's error");

        // Exactly the fragment: the encoded record's first bytes, and no newline, so
        // nothing here can be read back as a line.
        let body = fs::read(archive_dir.join(name_of(SESSION_A))).expect("the archive file");
        let encoded = encode_record(&record(SESSION_A, &long_line, 10_002));
        assert_eq!(body.len(), ACCEPTED);
        assert_eq!(body.as_slice(), &encoded.as_bytes()[..ACCEPTED]);

        let row = index.snapshot(SESSION_A).expect("the row");
        assert_eq!(row.status, "partial");
        assert_eq!(row.reason, Some("write-error".into()));
        assert_eq!(row.counters.line_count, 0, "a fragment is not a line");
        assert_eq!(
            row.counters.byte_size, ACCEPTED as i64,
            "the row says what the file holds"
        );
        assert_eq!(row.counters.dropped_lines, 1);
        assert_eq!(
            row.counters.dropped_bytes,
            long_line.len() as i64,
            "the whole line was lost, however much of its encoding landed"
        );
        assert_eq!(row.ended_at, Some(10_003));
        assert!(!writer.is_open(SESSION_A));
        assert_no_queue_entry(&writer, SESSION_A);

        // The quota counts the disk, not the intent: the fragment is occupying the
        // user's cap and a later eviction has to be able to free it.
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(ACCEPTED as u64));
        assert_eq!(entry_count(&archive_dir), 1);
        assert!(
            seam.removed().is_empty(),
            "a failed write is not a reason to delete what did land"
        );
    }

    #[test]
    fn a_failing_sync_data_closes_the_session_partial_write_error() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer.begin(SESSION_A, 10_001).expect("the session opens");
        writer.enqueue(record(SESSION_A, "a line", 10_002));
        writer.pump(10_003).expect("the queue drains");

        // The close is where durability is claimed, so a failure there is a
        // partial archive, not a complete one.
        seam.fail_sync();
        writer
            .close(SESSION_A, None, 10_004)
            .expect("the close reports through the row");

        assert_eq!(
            index.state_of(SESSION_A),
            Some(("partial".into(), Some("write-error".into())))
        );
        assert!(!writer.is_open(SESSION_A));
        assert!(archive_dir.join(name_of(SESSION_A)).is_file());
    }

    #[test]
    fn turning_the_archive_off_closes_open_sessions_partial_user_disabled() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer
            .begin(SESSION_A, 10_001)
            .expect("the first session opens");
        writer
            .begin(SESSION_B, 10_002)
            .expect("the second session opens");
        writer.enqueue(record(SESSION_A, "kept", 10_003));
        writer.pump(10_004).expect("the queue drains");

        writer
            .close_all(ArchiveReason::UserDisabled, 10_005)
            .expect("every session closes");

        for session in [SESSION_A, SESSION_B] {
            assert_eq!(
                index.state_of(session),
                Some(("partial".into(), Some("user-disabled".into()))),
                "{session}"
            );
            assert!(!writer.is_open(session));
            // Turning the archive off stops writing; it does not delete history.
            assert!(archive_dir.join(name_of(session)).is_file());
        }
        assert!(seam.removed().is_empty());
    }

    #[test]
    fn deleting_an_archive_is_refused_while_its_writer_is_open() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer.begin(SESSION_A, 10_001).expect("the session opens");
        let name = name_of(SESSION_A);

        assert!(writer.delete(SESSION_A).is_err());
        assert!(archive_dir.join(&name).is_file());
        assert!(seam.removed().is_empty());
        assert_eq!(index.state_of(SESSION_A), Some(("writing".into(), None)));

        // Once the session is closed the same delete is allowed.
        writer
            .close(SESSION_A, None, 10_002)
            .expect("the session closes");
        writer.delete(SESSION_A).expect("the archive is deleted");

        assert!(!archive_dir.join(&name).exists());
        assert_eq!(
            index.state_of(SESSION_A),
            Some(("removed".into(), Some("user-deleted".into())))
        );
    }

    #[test]
    fn deleting_an_archive_refuses_a_row_that_names_another_session() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        let owned = archive_dir.join(name_of(SESSION_A));
        fs::write(&owned, "z".repeat(12)).expect("the archive of another session");

        let index = TestIndex::shared();
        index.seed(seeded_row(SESSION_A, "complete", None, 12));
        // The row under attack: SESSION_B's row naming SESSION_A's file, as a
        // hand-edited database or a bug elsewhere could leave it.
        let mut crossed = seeded_row(SESSION_B, "complete", None, 12);
        crossed.file_name = name_of(SESSION_A);
        index.seed(crossed.clone());

        let seam = TestFs::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        assert!(writer.delete(SESSION_B).is_err());

        // Neither the file nor either row moved.
        assert_eq!(fs::read(&owned).expect("the archive"), b"z".repeat(12));
        assert!(seam.removed().is_empty());
        assert_eq!(
            index.snapshot(SESSION_A),
            Some(seeded_row(SESSION_A, "complete", None, 12))
        );
        assert_eq!(index.snapshot(SESSION_B), Some(crossed));
    }

    #[test]
    fn deleting_a_session_the_index_does_not_know_touches_nothing() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        // Planted after the sweep has run, so the only thing that could reach it
        // is a delete that trusted the name instead of the row.
        let planted = archive_dir.join(name_of(SESSION_B));
        fs::write(&planted, b"planted").expect("a file the index does not know");

        assert!(writer.delete(SESSION_B).is_err());

        assert_eq!(fs::read(&planted).expect("the planted file"), b"planted");
        assert!(seam.removed().is_empty());
        assert!(index.snapshot(SESSION_B).is_none());
    }

    /// A delete whose file goes but whose row will not move. The bytes are really
    /// gone, so the quota total has to fall by what was really there — the file's own
    /// length, not the number the row happened to carry — or the archive would refuse
    /// to grow into space it already freed.
    ///
    /// The row is left exactly as it was and the error is returned, because the one
    /// thing the writer must not do is guess. A `complete` row whose file is missing
    /// is a state the startup sweep already knows how to finish, and the second
    /// initialize below is that repair, not a comment claiming it.
    ///
    /// The row's `byte_size` is seeded wrong on purpose. It is the only way to tell
    /// "subtract what the disk said" from "subtract what the row said", and the two
    /// are the same number in every test that does not arrange for them to differ.
    #[test]
    fn a_delete_whose_row_will_not_move_still_frees_the_bytes_it_removed() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        fs::write(archive_dir.join(name_of(SESSION_A)), "z".repeat(40))
            .expect("the archive being deleted");
        fs::write(archive_dir.join(name_of(SESSION_B)), "z".repeat(100))
            .expect("an archive that stays");

        let index = TestIndex::shared();
        // 10, where the file is 40 bytes long. An ended row is never re-measured by
        // the sweep, so the disagreement survives initialize.
        let stale = seeded_row(SESSION_A, "complete", None, 10);
        index.seed(stale.clone());
        index.seed(seeded_row(SESSION_B, "complete", None, 100));

        let seam = TestFs::shared();
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");
        // Measured from the directory, so the stale row does not get a vote here.
        assert_eq!(report.measured_bytes, QuotaTotal::Known(140));

        index.fail("mark_removed");
        assert!(writer.delete(SESSION_A).is_err());

        // The file is gone, once, and by name.
        assert!(!archive_dir.join(name_of(SESSION_A)).exists());
        assert_eq!(seam.removed(), vec![archive_dir.join(name_of(SESSION_A))]);

        // The total now says what the directory holds: 100 real bytes left, not the
        // 130 the row's number would have produced.
        let remaining = fs::metadata(archive_dir.join(name_of(SESSION_B)))
            .expect("the archive that stays")
            .len();
        assert_eq!(remaining, 100);
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(remaining));

        // The row is untouched — status, reason, counters, and `ended_at` — and the
        // refusal is the last thing the index heard about it.
        assert_eq!(index.snapshot(SESSION_A), Some(stale));
        assert_eq!(
            index.calls().last().map(String::as_str),
            Some(format!("refused:mark_removed:{SESSION_A}").as_str())
        );
        assert_eq!(
            index.snapshot(SESSION_B),
            Some(seeded_row(SESSION_B, "complete", None, 100))
        );

        // What "left for the sweep" means, run rather than asserted: the next startup
        // over the same directory finds a `complete` row with no file and finishes it.
        index.allow("mark_removed");
        let (next, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("the next run initializes");
        assert_eq!(report.marked_file_missing, vec![SESSION_A.to_string()]);
        assert_eq!(
            index.state_of(SESSION_A),
            Some(("removed".into(), Some("file-missing".into())))
        );
        assert_eq!(next.total_bytes(), QuotaTotal::Known(remaining));
    }

    #[test]
    fn reading_an_archive_refuses_a_row_that_names_another_session() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        let contents = "{\"t\":1,\"s\":\"stdout\",\"l\":\"private\"}\n";
        fs::write(archive_dir.join(name_of(SESSION_A)), contents).expect("an archive");

        let index = TestIndex::shared();
        let size = contents.len() as i64;
        index.seed(seeded_row(SESSION_A, "complete", None, size));
        let mut crossed = seeded_row(SESSION_B, "complete", None, size);
        crossed.file_name = name_of(SESSION_A);
        index.seed(crossed);

        let seam = TestFs::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        // Read runs the same ownership check as delete, so one session cannot
        // read another's archive through its own row.
        assert!(writer.read(SESSION_B).is_err());
        assert_eq!(
            writer.read(SESSION_A).expect("its own archive"),
            contents,
            "the owner can still read it"
        );
        assert!(seam.removed().is_empty());
    }

    #[test]
    fn beginning_a_session_this_build_could_not_have_generated_touches_nothing() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        // Ids, not names: the name rule has its own tests, and what `begin` is
        // handed is a session id.
        let uppercased = SESSION_A.to_uppercase();
        let one_short = &SESSION_A[..SESSION_A.len() - 1];
        let one_long = format!("{SESSION_A}0");
        let underscored = SESSION_A.replace('-', "_");
        for session_id in [
            "",
            ".",
            "..",
            "not-a-uuid",
            uppercased.as_str(),
            one_short,
            one_long.as_str(),
            underscored.as_str(),
        ] {
            assert!(writer.begin(session_id, 10_001).is_err(), "{session_id:?}");
            assert!(!writer.is_open(session_id), "{session_id:?}");
        }

        // Refused before the filesystem and the index are reached at all: no file,
        // no removal, no row, and no slot left behind.
        assert_eq!(entry_count(&archive_dir), 0);
        assert!(seam.removed().is_empty());
        assert!(index.calls().is_empty());
        assert!(index.rows().expect("the rows").is_empty());
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(0));
    }

    #[test]
    fn beginning_a_session_whose_file_already_exists_refuses_and_keeps_the_file() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");
        assert_eq!(report.measured_bytes, QuotaTotal::Known(0));

        // Planted after the sweep has run, so `begin` meets it the way it would
        // meet the orphan a refused cleanup leaves behind.
        let path = archive_dir.join(name_of(SESSION_A));
        fs::write(&path, b"earlier bytes").expect("a file already in place");

        assert!(writer.begin(SESSION_A, 10_001).is_err());

        // `create_new` refused, so the bytes that were there are still there. A
        // file this call did not create is also not one it may remove: the sweep
        // owns that decision, and it takes the row into account.
        assert_eq!(fs::read(&path).expect("the planted file"), b"earlier bytes");
        assert!(seam.removed().is_empty());
        assert!(index.calls().is_empty());
        assert!(index.snapshot(SESSION_A).is_none());
        assert!(!writer.is_open(SESSION_A));
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(0));
    }

    #[test]
    fn beginning_the_same_session_twice_is_refused_and_keeps_the_first_archive() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer
            .begin(SESSION_A, 10_001)
            .expect("the first begin opens the session");
        let after_the_first = index.calls();

        assert!(writer.begin(SESSION_A, 10_002).is_err());

        // The second call is a refusal and nothing else: the same open session, the
        // same single row with the first call's timestamp, one file, no removal.
        assert!(writer.is_open(SESSION_A));
        assert_eq!(index.calls(), after_the_first);
        assert_eq!(index.state_of(SESSION_A), Some(("writing".into(), None)));
        assert_eq!(
            index.snapshot(SESSION_A).expect("the row").started_at,
            10_001
        );
        assert_eq!(entry_count(&archive_dir), 1);
        assert!(seam.removed().is_empty());
    }

    #[test]
    fn two_threads_beginning_the_same_session_leave_exactly_one_open_archive() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        // Two threads released together on one session. Which of them wins is the
        // scheduler's business; that exactly one wins is not, so every assertion
        // below holds for either order and none of them names a winner.
        let writer = Arc::new(writer);
        let gate = Arc::new(Barrier::new(2));
        let attempts: Vec<_> = [10_001, 10_002]
            .into_iter()
            .map(|started_at| {
                let writer = Arc::clone(&writer);
                let gate = Arc::clone(&gate);
                thread::spawn(move || {
                    gate.wait();
                    writer.begin(SESSION_A, started_at).is_ok()
                })
            })
            .collect();
        let opened = attempts
            .into_iter()
            .map(|attempt| attempt.join().expect("a begin thread"))
            .filter(|opened| *opened)
            .count();

        assert_eq!(opened, 1, "one of the two begins must be refused");
        assert!(writer.is_open(SESSION_A));
        assert_eq!(index.state_of(SESSION_A), Some(("writing".into(), None)));
        // The loser is refused before it can create anything, so there is one file
        // and one row, and no cleanup was ever needed.
        assert_eq!(
            index
                .calls()
                .iter()
                .filter(|call| call.starts_with("insert_writing:"))
                .count(),
            1
        );
        assert_eq!(entry_count(&archive_dir), 1);
        assert!(seam.removed().is_empty());
    }

    #[test]
    fn is_open_is_false_for_a_session_this_writer_never_opened() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        // A finished archive from an earlier run: its row and its file are both in
        // place, so the sweep leaves it exactly as it is.
        fs::write(archive_dir.join(name_of(SESSION_C)), "z".repeat(12))
            .expect("an earlier archive");
        let index = TestIndex::shared();
        index.seed(seeded_row(SESSION_C, "complete", None, 12));

        let seam = TestFs::shared();
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");
        assert_eq!(report.measured_bytes, QuotaTotal::Known(12));

        // Open means this writer holds the file, not that the index has heard of
        // the session: an archive an earlier run closed is not open here, and
        // neither is an id no session ever had.
        assert!(!writer.is_open(SESSION_A));
        assert!(!writer.is_open(SESSION_C));
        assert!(!writer.is_open("not-a-uuid"));
        assert!(!writer.is_open(""));

        writer.begin(SESSION_A, 10_001).expect("the session opens");
        assert!(writer.is_open(SESSION_A));
        assert!(!writer.is_open(SESSION_B));
        assert!(!writer.is_open(SESSION_C));
    }

    #[test]
    fn a_refused_writing_row_leaves_no_orphan_file_behind() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        index.fail("insert_writing");
        assert!(writer.begin(SESSION_A, 10_001).is_err());

        // The file is created before the row, so a refused row must take the file
        // with it: nothing unregistered is left in the directory.
        let path = archive_dir.join(name_of(SESSION_A));
        assert!(!path.exists());
        assert_eq!(seam.removed(), vec![path]);
        assert!(index.snapshot(SESSION_A).is_none());
        assert_eq!(
            index.calls(),
            vec![format!("refused:insert_writing:{SESSION_A}")]
        );

        // A failed begin leaves no half-open session and no charged bytes.
        assert!(!writer.is_open(SESSION_A));
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(0));
    }

    #[test]
    fn an_orphan_left_by_a_failed_cleanup_is_deleted_by_the_next_sweep() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        // Both halves fail: the index refuses the row and the filesystem refuses
        // the cleanup, which is the case `begin` documents the sweep as the
        // backstop for.
        index.fail("insert_writing");
        seam.fail_remove();
        assert!(writer.begin(SESSION_A, 10_001).is_err());

        let path = archive_dir.join(name_of(SESSION_A));
        assert!(path.is_file(), "the cleanup was refused, so the file stays");
        assert_eq!(seam.removed(), vec![path.clone()], "the cleanup was tried");
        assert!(index.snapshot(SESSION_A).is_none());
        assert!(!writer.is_open(SESSION_A));

        // The next run over the same directory and index recovers: the file has no
        // row and this build generated its name, so it is an eligible orphan.
        index.allow("insert_writing");
        seam.allow_remove();
        let (recovered, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("a second initialization");

        assert_eq!(report.deleted_orphan_files, vec![name_of(SESSION_A)]);
        assert!(!path.exists());
        assert_eq!(report.measured_bytes, QuotaTotal::Known(0));
        assert_eq!(recovered.total_bytes(), QuotaTotal::Known(0));
    }

    #[test]
    fn a_refused_close_row_keeps_the_bytes_and_leaves_a_repairable_row() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer.begin(SESSION_A, 10_001).expect("the session opens");
        writer.enqueue(record(SESSION_A, "kept", 10_002));
        writer.pump(10_003).expect("the queue drains");

        index.fail("close");
        assert!(writer.close(SESSION_A, None, 10_004).is_err());

        // The file was flushed, synced, and released before the row was written,
        // so the archive keeps the bytes the session had already produced.
        let path = archive_dir.join(name_of(SESSION_A));
        let written = fs::read_to_string(&path).expect("the archive");
        assert_eq!(
            written,
            format!("{}\n", encode_record(&record(SESSION_A, "kept", 10_002)))
        );
        assert!(seam.removed().is_empty());

        // Nothing is writable any more, and the row left behind is exactly the
        // one the sweep knows how to repair.
        assert!(!writer.is_open(SESSION_A));
        assert_eq!(index.state_of(SESSION_A), Some(("writing".into(), None)));

        // The next run over the same directory and index repairs it, so a refused
        // close costs the row's accuracy until then and never the bytes.
        index.allow("close");
        let (repaired, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("a second initialization");

        assert_eq!(report.repaired_writing, vec![SESSION_A.to_string()]);
        assert_eq!(
            index.state_of(SESSION_A),
            Some(("partial".into(), Some("interrupted".into())))
        );
        let row = index.snapshot(SESSION_A).expect("the repaired row");
        assert_eq!(row.counters.byte_size, written.len() as i64);
        assert_eq!(row.counters.line_count, 1);
        assert_eq!(
            repaired.total_bytes(),
            QuotaTotal::Known(written.len() as u64)
        );
        assert!(path.is_file());
        assert!(seam.removed().is_empty());
    }

    /// After the toggle goes off, a capture thread that has not noticed yet still
    /// hands lines over. The sessions are closed, so nothing may follow them: the
    /// file must not grow, the row must not move — its drop counters included — and
    /// the writer must make no further index call at all.
    #[test]
    fn enqueueing_after_close_all_user_disabled_changes_no_file_and_no_row() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer
            .begin(SESSION_A, 10_001)
            .expect("the first session opens");
        writer
            .begin(SESSION_B, 10_002)
            .expect("the second session opens");
        writer.enqueue(record(SESSION_A, "kept", 10_003));
        writer.pump(10_004).expect("the queue drains");
        writer
            .close_all(ArchiveReason::UserDisabled, 10_005)
            .expect("every session closes");

        // Everything the archive owns, exactly as the close left it.
        let closed: Vec<(String, ArchiveRow)> = [SESSION_A, SESSION_B]
            .into_iter()
            .map(|session| {
                let text = fs::read_to_string(archive_dir.join(name_of(session)))
                    .expect("the archive file");
                (text, index.snapshot(session).expect("the closed row"))
            })
            .collect();
        let calls = index.calls();

        // The late lines, and the pump that would have written them.
        writer.enqueue(record(SESSION_A, "too late", 10_006));
        writer.enqueue(record(SESSION_B, "too late", 10_007));
        writer.pump(10_008).expect("a pump with nothing open");

        for (session, (text, row)) in [SESSION_A, SESSION_B].into_iter().zip(closed) {
            assert_eq!(
                fs::read_to_string(archive_dir.join(name_of(session))).expect("the archive file"),
                text,
                "{session}"
            );
            assert_eq!(index.snapshot(session), Some(row), "{session}");
            assert!(!writer.is_open(session), "{session}");
            // The late line must not have been queued either: a file and a row that
            // did not move say nothing about a record still held in memory.
            assert_no_queue_entry(&writer, session);
        }
        // Not a counter update, not a second close, not a row for anything else.
        assert_eq!(index.calls(), calls);
        assert_eq!(entry_count(&archive_dir), 2);
        assert!(seam.removed().is_empty());
    }

    /// A closed session must not get queue state back, and the way to see that is
    /// the room: with one slot in total, a record wrongly queued for a session that
    /// is already closed would cost the next live session its line.
    #[test]
    fn a_record_for_a_closed_session_never_takes_the_queues_room() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            one_slot_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer
            .begin(SESSION_A, 10_001)
            .expect("the closing session opens");
        writer
            .begin(SESSION_B, 10_002)
            .expect("the live session opens");
        writer.enqueue(record(SESSION_A, "kept", 10_003));
        writer.pump(10_004).expect("the queue drains");
        writer
            .close(SESSION_A, None, 10_005)
            .expect("the first session closes");

        writer.enqueue(record(SESSION_A, "too late", 10_006));
        writer.enqueue(record(SESSION_B, "b line", 10_007));
        writer.pump(10_008).expect("the queue drains");
        writer
            .close(SESSION_B, None, 10_009)
            .expect("the live session closes");

        // The load-bearing assertion: the live session lost nothing. Had the
        // closed session's line been queued, this line would have been the one the
        // bounds refused.
        let live = index.snapshot(SESSION_B).expect("the live session's row");
        assert_eq!(live.counters.dropped_lines, 0);
        assert_eq!(live.counters.dropped_bytes, 0);
        assert_eq!(live.counters.line_count, 1);
        let text =
            fs::read_to_string(archive_dir.join(name_of(SESSION_B))).expect("the live archive");
        assert_eq!(
            text,
            format!("{}\n", encode_record(&record(SESSION_B, "b line", 10_007)))
        );

        // And the closed session kept exactly what it had before it closed.
        assert_eq!(
            fs::read_to_string(archive_dir.join(name_of(SESSION_A))).expect("the closed archive"),
            format!("{}\n", encode_record(&record(SESSION_A, "kept", 10_003)))
        );
        // The room came back because the record was refused, not because it was
        // queued and later dropped: a refused record leaves no entry at all.
        assert_no_queue_entry(&writer, SESSION_A);
        assert_no_queue_entry(&writer, SESSION_B);
        assert!(seam.removed().is_empty());
    }

    /// A close and a capture thread, held against each other at the instant the
    /// close is claiming durability. What the session accepted before that instant
    /// must be on disk; what arrives after it must not reach the file or the row.
    ///
    /// The rendezvous is inside `sync_data`, which the close reaches only once it has
    /// taken everything the session had accepted and flushed it, so the window this
    /// test opens is the real one and no test sleeps to find it.
    ///
    /// It says nothing about where the closing boundary itself sits: by `sync_data`
    /// the session is closing under every ordering, which is why the record arriving
    /// here must not land. The boundary's own instant — after the records left the
    /// queue, before the session stops accepting — is the subject of
    /// [`a_record_at_the_closing_boundary_is_refused_and_leaves_no_entry`].
    #[test]
    fn a_record_accepted_before_a_close_lands_and_one_arriving_during_it_does_not() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer.begin(SESSION_A, 10_001).expect("the session opens");
        // Accepted and still queued: the close is what has to get it to disk.
        writer.enqueue(record(SESSION_A, "before", 10_002));

        let hold = seam.hold_sync_of(&name_of(SESSION_A));
        let writer = Arc::new(writer);
        let closing = {
            let writer = Arc::clone(&writer);
            thread::spawn(move || writer.close(SESSION_A, None, 10_003))
        };
        let closing = hold.wait_for(closing);

        // Inside the seam: flushed, not yet synced, handle not yet released.
        writer.enqueue(record(SESSION_A, "during", 10_004));
        hold.let_go();
        closing
            .join()
            .expect("the close thread")
            .expect("the session closes");
        writer.pump(10_005).expect("a pump after the close");

        let text = fs::read_to_string(archive_dir.join(name_of(SESSION_A))).expect("the archive");
        assert_eq!(
            text,
            format!("{}\n", encode_record(&record(SESSION_A, "before", 10_002))),
            "only the record accepted before the close belongs in the file"
        );

        let row = index.snapshot(SESSION_A).expect("the closed row");
        assert_eq!(row.status, "complete");
        assert_eq!(row.reason, None);
        assert_eq!(row.counters.line_count, 1);
        assert_eq!(row.counters.byte_size, text.len() as i64);
        // Nor may the racing record be charged as a loss: a closed archive's drop
        // counters have to be explained by gap lines inside the file, and this file
        // was flushed before the record existed.
        assert_eq!(row.counters.dropped_lines, 0);
        assert_eq!(row.counters.dropped_bytes, 0);
        assert_eq!(row.ended_at, Some(10_003));
        assert!(!writer.is_open(SESSION_A));

        // Whatever the writer did with the racing record, it did not touch this
        // session's row again after closing it.
        let calls = index.calls();
        let closed_at = calls
            .iter()
            .position(|call| call.starts_with("close:"))
            .expect("a close call");
        assert!(
            calls[closed_at + 1..].is_empty(),
            "{:?}",
            &calls[closed_at + 1..]
        );
        // Nor is it still in memory: the racing record left no entry behind.
        assert_no_queue_entry(&writer, SESSION_A);
        assert_eq!(entry_count(&archive_dir), 1);
        assert!(seam.removed().is_empty());
    }

    /// The other half of the race: once a session is finished, its queue state must
    /// not come back. With one slot in total, a record the writer wrongly kept for
    /// the closing session would be holding the slot the live session needs, so the
    /// live session's line is the evidence.
    #[test]
    fn a_record_racing_a_close_never_takes_the_room_the_next_session_needs() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            one_slot_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer
            .begin(SESSION_A, 10_001)
            .expect("the closing session opens");
        writer
            .begin(SESSION_B, 10_002)
            .expect("the live session opens");
        // The only slot, which the close has to drain before it can finish.
        writer.enqueue(record(SESSION_A, "before", 10_003));

        let hold = seam.hold_sync_of(&name_of(SESSION_A));
        let writer = Arc::new(writer);
        let closing = {
            let writer = Arc::clone(&writer);
            thread::spawn(move || writer.close(SESSION_A, None, 10_004))
        };
        let closing = hold.wait_for(closing);

        // Ordered on purpose: the closing session asks first, so if it is given the
        // slot the live session goes without.
        writer.enqueue(record(SESSION_A, "during", 10_005));
        writer.enqueue(record(SESSION_B, "b line", 10_006));
        hold.let_go();
        closing
            .join()
            .expect("the close thread")
            .expect("the session closes");
        writer.pump(10_007).expect("the queue drains");
        writer
            .close(SESSION_B, None, 10_008)
            .expect("the live session closes");

        let live = index.snapshot(SESSION_B).expect("the live session's row");
        assert_eq!(live.counters.line_count, 1);
        assert_eq!(live.counters.dropped_lines, 0);
        assert_eq!(live.counters.dropped_bytes, 0);
        assert_eq!(
            fs::read_to_string(archive_dir.join(name_of(SESSION_B))).expect("the live archive"),
            format!("{}\n", encode_record(&record(SESSION_B, "b line", 10_006)))
        );

        assert_eq!(
            fs::read_to_string(archive_dir.join(name_of(SESSION_A))).expect("the closed archive"),
            format!("{}\n", encode_record(&record(SESSION_A, "before", 10_003)))
        );
        // The slot came back for the same reason it was never taken: the racing
        // record was refused outright, leaving no entry to hold it.
        assert_no_queue_entry(&writer, SESSION_A);
        assert_no_queue_entry(&writer, SESSION_B);
        assert!(seam.removed().is_empty());
    }

    /// The instant the whole closing boundary turns on: after the records a close
    /// took have left the queue, and before — under a naive close — that session
    /// stops accepting. A capture thread hands over one more record exactly there.
    ///
    /// Under the boundary this writer commits to, that window does not exist. A close
    /// marks the session closing and takes everything it has already accepted inside
    /// one critical section, and only then touches the file; the gate below fires
    /// inside that file work, so a record arriving here is always on the far side of
    /// the boundary and is always refused. The outcome is therefore a single value
    /// rather than a choice, which is why this test names one file content and not
    /// two.
    ///
    /// A close that pumped first and flipped the state afterwards would have that
    /// window, and a record landing in it would be neither written nor refused —
    /// stranded in the queue, blocking [`ArchiveQueue::finish_session`], holding room
    /// nothing will ever free. Two assertions forbid it: the closed session keeps no
    /// queue entry, and, with one slot in the whole queue, the live session still
    /// gets its line.
    ///
    /// The seam is the closing session's own `write`, the only place a test can stand
    /// between those two events: a `sync_data` hold is too late, because by then the
    /// session is closing under every ordering.
    #[test]
    fn a_record_at_the_closing_boundary_is_refused_and_leaves_no_entry() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            one_slot_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer
            .begin(SESSION_A, 10_001)
            .expect("the closing session opens");
        writer
            .begin(SESSION_B, 10_002)
            .expect("the live session opens");

        // Longer than the buffer, so the close's own write reaches the file and the
        // gate fires inside it. It also holds the only slot until the close takes it.
        let long_line = "x".repeat(WRITE_BUFFER_BYTES + 1);
        writer.enqueue(record(SESSION_A, &long_line, 10_003));

        let hold = seam.hold_write_of(&name_of(SESSION_A));
        let writer = Arc::new(writer);
        let closing = {
            let writer = Arc::clone(&writer);
            thread::spawn(move || writer.close(SESSION_A, None, 10_004))
        };
        let closing = hold.wait_for(closing);

        // On the boundary: the record above is on its way to the disk, and this
        // session has already stopped accepting.
        writer.enqueue(record(SESSION_A, "at the boundary", 10_005));
        hold.let_go();
        closing
            .join()
            .expect("the close thread")
            .expect("a record arriving at the boundary cannot make the close fail");

        let body = fs::read_to_string(archive_dir.join(name_of(SESSION_A))).expect("the archive");
        let kept = format!(
            "{}\n",
            encode_record(&record(SESSION_A, &long_line, 10_003))
        );
        assert_eq!(
            body, kept,
            "the boundary record arrived after the close had stopped accepting, so \
             only the record the close took belongs in the file"
        );

        let row = index.snapshot(SESSION_A).expect("the closed row");
        assert_eq!(row.status, "complete");
        assert_eq!(row.reason, None);
        assert_eq!(row.counters.line_count, 1);
        assert_eq!(row.counters.byte_size, body.len() as i64);
        // Refused is not lost: the record never entered the archive, and a closed
        // archive's drop counters have to be explained by gap lines inside its file.
        assert_eq!(row.counters.dropped_lines, 0);
        assert_eq!(row.counters.dropped_bytes, 0);
        assert_eq!(row.ended_at, Some(10_004));
        assert!(!writer.is_open(SESSION_A));
        // The refusal reached neither the file nor the queue. Without this, a writer
        // that quietly queued the record would still pass every assertion above.
        assert_no_queue_entry(&writer, SESSION_A);

        // And the room is the second witness: the queue holds exactly one slot, and a
        // boundary record still sitting in it would take this line instead.
        writer.enqueue(record(SESSION_B, "b line", 10_006));
        writer.pump(10_007).expect("the queue drains");
        writer
            .close(SESSION_B, None, 10_008)
            .expect("the live session closes");

        let live = index.snapshot(SESSION_B).expect("the live session's row");
        assert_eq!(live.counters.line_count, 1);
        assert_eq!(live.counters.dropped_lines, 0);
        assert_eq!(live.counters.dropped_bytes, 0);
        assert_eq!(
            fs::read_to_string(archive_dir.join(name_of(SESSION_B))).expect("the live archive"),
            format!("{}\n", encode_record(&record(SESSION_B, "b line", 10_006)))
        );
        assert_no_queue_entry(&writer, SESSION_B);
        assert_eq!(entry_count(&archive_dir), 2);
        assert!(seam.removed().is_empty());
    }

    /// close. It never reached the queue, so the queue has no entry for it to
    /// finish — which is exactly why finishing a session it has never seen answers
    /// "nothing pending" instead of failing.
    #[test]
    fn a_session_that_wrote_nothing_closes_complete_and_empty() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer.begin(SESSION_A, 10_001).expect("the session opens");
        writer
            .close(SESSION_A, None, 10_002)
            .expect("an empty session closes");

        let row = index.snapshot(SESSION_A).expect("the row");
        assert_eq!(row.status, "complete");
        assert_eq!(row.reason, None);
        assert_eq!(row.counters, ArchiveCounters::default());
        assert_eq!(row.ended_at, Some(10_002));
        assert!(!writer.is_open(SESSION_A));

        // The file `begin` created stays, and stays empty: nothing was written, and
        // no gap line stands in for a loss that never happened.
        assert_eq!(
            fs::read_to_string(archive_dir.join(name_of(SESSION_A))).expect("the archive file"),
            ""
        );
        // A close may not create the entry it then has to forget: this session never
        // reached the queue at all.
        assert_no_queue_entry(&writer, SESSION_A);
        assert_eq!(entry_count(&archive_dir), 1);
        assert!(seam.removed().is_empty());
    }

    /// The second close of the same session is an error, and the writer's own
    /// open-session state is what finds it: the queue keeps no tombstone to be asked
    /// about. Being refused, it changes nothing — not the file, not the row's
    /// `ended_at`, not one index call.
    #[test]
    fn closing_the_same_session_twice_is_refused_by_the_open_state_and_changes_nothing() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer.begin(SESSION_A, 10_001).expect("the session opens");
        writer.enqueue(record(SESSION_A, "kept", 10_002));
        writer.pump(10_003).expect("the queue drains");
        writer
            .close(SESSION_A, None, 10_004)
            .expect("the session closes");

        let text = fs::read_to_string(archive_dir.join(name_of(SESSION_A))).expect("the archive");
        let row = index.snapshot(SESSION_A).expect("the closed row");
        let calls = index.calls();

        assert!(writer.close(SESSION_A, None, 10_005).is_err());
        // Closing every open session is a different claim: with none open it has
        // nothing to refuse and nothing to do.
        writer
            .close_all(ArchiveReason::UserDisabled, 10_006)
            .expect("no open session to close");

        assert_eq!(
            fs::read_to_string(archive_dir.join(name_of(SESSION_A))).expect("the archive"),
            text
        );
        assert_eq!(index.snapshot(SESSION_A), Some(row));
        assert_eq!(index.calls(), calls);
        assert!(!writer.is_open(SESSION_A));
        // The refused close took nothing back either: the first one forgot the
        // session, and the second must not have re-created its entry.
        assert_no_queue_entry(&writer, SESSION_A);
        assert_eq!(entry_count(&archive_dir), 1);
        assert!(seam.removed().is_empty());
    }

    // Group 3: the queue's bounds and the gap records they produce.

    #[test]
    fn the_queue_drops_the_incoming_record_and_never_a_queued_one() {
        let mut queue = ArchiveQueue::new(QueueBounds {
            session_records: 2,
            session_bytes: 1_024,
            total_records: 8,
            total_bytes: 4_096,
        });

        assert!(queue.is_empty());
        assert!(queue.enqueue(record(SESSION_A, "first", 1)));
        assert!(queue.enqueue(record(SESSION_A, "second", 2)));
        assert_eq!(queue.len(), 2);

        // The session is full, so the arriving record is the one lost.
        assert!(!queue.enqueue(record(SESSION_A, "third", 3)));
        assert_eq!(queue.len(), 2);
        assert_eq!(
            queue.dropped(SESSION_A),
            DropCounters { lines: 1, bytes: 5 }
        );

        // A different session has its own room.
        assert!(queue.enqueue(record(SESSION_B, "other", 4)));

        let settled: Vec<String> = settle_all(&mut queue)
            .into_iter()
            .map(|item| item.record.line)
            .collect();
        assert_eq!(settled, vec!["first", "second", "other"]);
        assert!(queue.is_empty());
    }

    #[test]
    fn the_queue_enforces_its_byte_bounds_per_session_and_in_total() {
        let mut queue = ArchiveQueue::new(QueueBounds {
            session_records: 16,
            session_bytes: 20,
            total_records: 16,
            total_bytes: 30,
        });

        // Bytes are the archived text, not the JSON encoding around it.
        assert!(queue.enqueue(record(SESSION_A, "0123456789", 1)));
        assert_eq!(queue.queued_bytes(), 10);
        assert!(queue.enqueue(record(SESSION_A, "0123456789", 2)));
        assert_eq!(queue.queued_bytes(), 20);
        assert!(!queue.enqueue(record(SESSION_A, "x", 3)));

        // The total bound has ten bytes left, so a longer record does not fit
        // even though its own session is still empty.
        assert!(!queue.enqueue(record(SESSION_B, "01234567890", 4)));
        assert!(queue.enqueue(record(SESSION_B, "0123456789", 5)));
        assert_eq!(queue.queued_bytes(), 30);
        assert!(!queue.enqueue(record(SESSION_C, "y", 6)));

        assert_eq!(
            queue.dropped(SESSION_A),
            DropCounters { lines: 1, bytes: 1 }
        );
        assert_eq!(
            queue.dropped(SESSION_B),
            DropCounters {
                lines: 1,
                bytes: 11
            }
        );
        assert_eq!(
            queue.dropped(SESSION_C),
            DropCounters { lines: 1, bytes: 1 }
        );

        // The gap owed to a session is taken once.
        assert_eq!(
            queue.take_pending_gap(SESSION_A),
            Some(DropCounters { lines: 1, bytes: 1 })
        );
        assert_eq!(queue.take_pending_gap(SESSION_A), None);
        // Taking the gap does not forget what the session lost.
        assert_eq!(
            queue.dropped(SESSION_A),
            DropCounters { lines: 1, bytes: 1 }
        );
    }

    #[test]
    fn dropping_an_empty_line_counts_one_line_and_no_bytes() {
        // A lone newline is a real captured event whose text is empty, so losing
        // it costs one line and nothing else. The record bound has to be what
        // refuses it: an empty record can never exhaust a byte bound.
        let mut queue = ArchiveQueue::new(QueueBounds {
            session_records: 1,
            session_bytes: 64,
            total_records: 8,
            total_bytes: 512,
        });

        assert!(queue.enqueue(record(SESSION_A, "kept", 1)));
        assert!(!queue.enqueue(record(SESSION_A, "", 2)));

        assert_eq!(queue.queued_bytes(), 4, "the empty record added no bytes");
        assert_eq!(
            queue.dropped(SESSION_A),
            DropCounters { lines: 1, bytes: 0 }
        );
        assert_eq!(
            queue.take_pending_gap(SESSION_A),
            Some(DropCounters { lines: 1, bytes: 0 })
        );
        // What the user is told about the loss, in the same terms.
        assert_eq!(
            gap_line(DropCounters { lines: 1, bytes: 0 }),
            "[RunCove: dropped 1 line / 0 bytes]"
        );
    }
    #[test]
    fn the_queue_counts_utf8_bytes_and_not_characters() {
        // Written with escapes so the counts do not depend on the encoding of
        // this source file.
        const HAN: &str = "\u{4e2d}";
        const ROCKET: &str = "\u{1f680}";
        const COMBINED: &str = "e\u{301}";
        assert_eq!(
            (HAN.len(), ROCKET.len(), COMBINED.len()),
            (3, 4, 3),
            "the premise: these hold three, four, and three UTF-8 bytes"
        );
        assert_eq!(
            (
                HAN.chars().count(),
                ROCKET.chars().count(),
                COMBINED.chars().count()
            ),
            (1, 1, 2),
            "the premise: their character counts are not their byte counts"
        );

        let mut queue = ArchiveQueue::new(QueueBounds {
            session_records: 8,
            session_bytes: 7,
            total_records: 16,
            total_bytes: 64,
        });

        assert!(queue.enqueue(record(SESSION_A, HAN, 1)));
        assert_eq!(queue.queued_bytes(), 3, "one character, three bytes");
        assert!(queue.enqueue(record(SESSION_A, ROCKET, 2)));
        assert_eq!(queue.queued_bytes(), 7, "two characters, seven bytes");

        // The session's seven bytes are spoken for, so a three-byte record does
        // not fit even though six of its eight records are still free.
        assert!(!queue.enqueue(record(SESSION_A, COMBINED, 3)));
        assert_eq!(
            queue.dropped(SESSION_A),
            DropCounters { lines: 1, bytes: 3 },
            "the dropped bytes are the text's UTF-8 length, not its character count"
        );
    }
    /// The bounds the gap-placement tests share: only the session byte bound ever
    /// refuses anything, so which record is lost is decided by its own length and
    /// nothing else. Long enough to overflow the session's eight bytes on its own,
    /// short enough that two of them plus the kept records stay inside the total.
    const OVER_SESSION_BYTES: &str = "toolongtoolong";
    const ALSO_OVER: &str = "alsotoolong";

    fn gap_bounds() -> QueueBounds {
        QueueBounds {
            session_records: 4,
            session_bytes: 8,
            total_records: 8,
            total_bytes: 64,
        }
    }

    #[test]
    fn two_drop_runs_become_two_carried_gaps_on_two_different_records() {
        // drop, accept, drop, accept: each loss belongs to the record that first
        // survived it, so the file says where each one happened instead of
        // reporting both at one place.
        let mut queue = ArchiveQueue::new(gap_bounds());

        assert!(queue.enqueue(record(SESSION_A, "aa", 1)));
        assert!(!queue.enqueue(record(SESSION_A, OVER_SESSION_BYTES, 2)));
        assert!(queue.enqueue(record(SESSION_A, "bb", 3)));
        assert!(!queue.enqueue(record(SESSION_A, ALSO_OVER, 4)));
        assert!(queue.enqueue(record(SESSION_A, "cc", 5)));

        let first = DropCounters {
            lines: 1,
            bytes: OVER_SESSION_BYTES.len() as i64,
        };
        let second = DropCounters {
            lines: 1,
            bytes: ALSO_OVER.len() as i64,
        };
        assert_ne!(first, second, "the premise: the two runs are told apart");
        assert_eq!(
            lines_and_gaps(&mut queue),
            vec![
                ("aa".to_string(), None),
                ("bb".to_string(), Some(first)),
                ("cc".to_string(), Some(second)),
            ],
            "each run rides out on the first record accepted after it"
        );

        // Both runs left with a record, so nothing is owed at close, and the
        // session's running total is still the sum of the two.
        assert_eq!(queue.take_pending_gap(SESSION_A), None);
        assert_eq!(
            queue.dropped(SESSION_A),
            DropCounters {
                lines: 2,
                bytes: first.bytes + second.bytes
            }
        );
    }

    #[test]
    fn a_contiguous_run_of_drops_becomes_one_gap_on_the_next_accepted_record() {
        // Two losses with nothing kept between them are one gap, not two: the
        // marker stands for a contiguous run, so the file gains one line however
        // long the run was.
        let mut queue = ArchiveQueue::new(gap_bounds());

        assert!(queue.enqueue(record(SESSION_A, "aa", 1)));
        assert!(!queue.enqueue(record(SESSION_A, OVER_SESSION_BYTES, 2)));
        assert!(!queue.enqueue(record(SESSION_A, ALSO_OVER, 3)));
        assert!(queue.enqueue(record(SESSION_A, "bb", 4)));

        let run = DropCounters {
            lines: 2,
            bytes: (OVER_SESSION_BYTES.len() + ALSO_OVER.len()) as i64,
        };
        assert_eq!(
            lines_and_gaps(&mut queue),
            vec![("aa".to_string(), None), ("bb".to_string(), Some(run))],
            "one gap for the run, on the record that ended it"
        );
        assert_eq!(queue.take_pending_gap(SESSION_A), None);
        assert_eq!(queue.dropped(SESSION_A), run);
        // What the one line will say.
        assert_eq!(gap_line(run), "[RunCove: dropped 2 lines / 25 bytes]");
    }

    #[test]
    fn a_trailing_drop_has_no_record_to_carry_it_and_stays_the_residual() {
        // Nothing survived the loss, so there is no record to hang it on. It is
        // the residual, and `close` is what writes it.
        let mut queue = ArchiveQueue::new(gap_bounds());

        assert!(queue.enqueue(record(SESSION_A, "aa", 1)));
        assert!(!queue.enqueue(record(SESSION_A, OVER_SESSION_BYTES, 2)));

        let trailing = DropCounters {
            lines: 1,
            bytes: OVER_SESSION_BYTES.len() as i64,
        };
        assert_eq!(
            lines_and_gaps(&mut queue),
            vec![("aa".to_string(), None)],
            "the record that arrived before the loss carries nothing"
        );
        assert_eq!(queue.take_pending_gap(SESSION_A), Some(trailing));
        assert_eq!(
            queue.take_pending_gap(SESSION_A),
            None,
            "the residual is owed once"
        );
        assert_eq!(queue.dropped(SESSION_A), trailing);
    }

    #[test]
    fn a_pending_gap_survives_a_drain_and_lands_on_the_next_accepted_record() {
        // The loss falls between two pumps: after everything the first drain took
        // and before the first record the second one takes. Carrying it across the
        // drain is what keeps the marker in that spot instead of moving it to the
        // end of the earlier batch.
        let mut queue = ArchiveQueue::new(gap_bounds());

        assert!(queue.enqueue(record(SESSION_A, "aa", 1)));
        assert!(!queue.enqueue(record(SESSION_A, OVER_SESSION_BYTES, 2)));
        assert_eq!(
            lines_and_gaps(&mut queue),
            vec![("aa".to_string(), None)],
            "the first batch carries nothing: the loss came after it"
        );

        let across = DropCounters {
            lines: 1,
            bytes: OVER_SESSION_BYTES.len() as i64,
        };
        assert!(queue.enqueue(record(SESSION_A, "bb", 3)));
        assert_eq!(
            lines_and_gaps(&mut queue),
            vec![("bb".to_string(), Some(across))],
            "the next accepted record picks it up, one drain later"
        );
        assert_eq!(queue.take_pending_gap(SESSION_A), None);
        assert_eq!(queue.dropped(SESSION_A), across);
    }

    #[test]
    fn a_gap_attaches_only_to_a_record_of_its_own_session() {
        // Sessions are separate files, so another session's record is not a place
        // this loss could be reported. It waits for one of its own.
        let mut queue = ArchiveQueue::new(gap_bounds());

        assert!(queue.enqueue(record(SESSION_A, "aa", 1)));
        assert!(!queue.enqueue(record(SESSION_A, OVER_SESSION_BYTES, 2)));
        assert!(queue.enqueue(record(SESSION_B, "bb", 3)));
        assert!(queue.enqueue(record(SESSION_A, "cc", 4)));

        let owed = DropCounters {
            lines: 1,
            bytes: OVER_SESSION_BYTES.len() as i64,
        };
        let settled: Vec<(String, String, Option<DropCounters>)> = settle_all(&mut queue)
            .into_iter()
            .map(|item| (item.record.session_id, item.record.line, item.gap_before))
            .collect();
        assert_eq!(
            settled,
            vec![
                (SESSION_A.to_string(), "aa".to_string(), None),
                (SESSION_B.to_string(), "bb".to_string(), None),
                (SESSION_A.to_string(), "cc".to_string(), Some(owed)),
            ]
        );

        assert_eq!(queue.take_pending_gap(SESSION_A), None);
        assert_eq!(queue.take_pending_gap(SESSION_B), None);
        assert_eq!(queue.dropped(SESSION_A), owed);
        assert_eq!(queue.dropped(SESSION_B), DropCounters::default());
    }

    /// A deterministic stand-in for a property test, with no new dependency:
    /// every sequence of six records drawn from a four-record alphabet, each one
    /// checked against a model the test keeps itself. The alphabet meets each of
    /// the four bounds exactly and then exceeds it, one of its records is empty,
    /// and another is multi-byte, so byte accounting cannot pass here by
    /// counting characters.
    ///
    /// The model tracks where each loss ends up as well as how much was lost, so
    /// the gap partition is checked over all 4096 sequences rather than only in
    /// the hand-written cases above.
    #[test]
    fn every_short_enqueue_sequence_keeps_the_queues_invariants() {
        const HAN: &str = "\u{4e2d}";
        const LENGTH: usize = 6;
        // (label, session, line), holding 0, 4, 3, and 4 bytes.
        let alphabet = [
            ("A:empty", SESSION_A, ""),
            ("A:abcd", SESSION_A, "abcd"),
            ("B:han", SESSION_B, HAN),
            ("C:xyzw", SESSION_C, "xyzw"),
        ];
        let bounds = QueueBounds {
            session_records: 2,
            session_bytes: 4,
            total_records: 3,
            total_bytes: 8,
        };
        let sessions = [SESSION_A, SESSION_B, SESSION_C];

        fn records_of(kept: &[(&str, &str)], session_id: &str) -> usize {
            kept.iter().filter(|(id, _)| *id == session_id).count()
        }
        fn bytes_of(kept: &[(&str, &str)], session_id: &str) -> usize {
            kept.iter()
                .filter(|(id, _)| *id == session_id)
                .map(|(_, line)| line.len())
                .sum()
        }
        fn total_bytes_of(kept: &[(&str, &str)]) -> usize {
            kept.iter().map(|(_, line)| line.len()).sum()
        }
        /// Reaching a bound exactly still fits; one record or one byte past it
        /// does not. So a record fits while the count is still below its bound,
        /// and its bytes fit while the total after adding them stays at or under
        /// the byte bound.
        fn fits(kept: &[(&str, &str)], session_id: &str, line: &str, bounds: QueueBounds) -> bool {
            records_of(kept, session_id) < bounds.session_records
                && bytes_of(kept, session_id) + line.len() <= bounds.session_bytes
                && kept.len() < bounds.total_records
                && total_bytes_of(kept) + line.len() <= bounds.total_bytes
        }
        for encoded in 0..alphabet.len().pow(LENGTH as u32) {
            let mut sequence = Vec::with_capacity(LENGTH);
            let mut rest = encoded;
            for _ in 0..LENGTH {
                sequence.push(alphabet[rest % alphabet.len()]);
                rest /= alphabet.len();
            }
            let trace = sequence
                .iter()
                .map(|(label, _, _)| *label)
                .collect::<Vec<_>>()
                .join(",");

            let mut queue = ArchiveQueue::new(bounds);
            let mut kept: Vec<(&str, &str)> = Vec::new();
            // The gap each accepted record should be carrying, in the same order
            // as `kept`. Positional on purpose: the contract is not "the session
            // lost this much" but "this record, and no other, carries this run of
            // losses".
            let mut carried: Vec<Option<DropCounters>> = Vec::new();
            // Lost with no accepted record behind it yet. An entry exists only
            // while it is non-empty, so presence is the model's answer to
            // "is a gap pending".
            let mut pending: BTreeMap<&str, DropCounters> = BTreeMap::new();
            let mut lost: BTreeMap<&str, DropCounters> = BTreeMap::new();

            for (step, (_, session_id, line)) in sequence.iter().copied().enumerate() {
                let expected = fits(&kept, session_id, line, bounds);
                let accepted = queue.enqueue(record(session_id, line, step as i64));
                assert_eq!(
                    accepted, expected,
                    "[{trace}] step {step}: a record is accepted exactly when it stays inside every bound"
                );
                if accepted {
                    kept.push((session_id, line));
                    carried.push(pending.remove(session_id));
                } else {
                    let counters = lost.entry(session_id).or_default();
                    counters.lines += 1;
                    counters.bytes += line.len() as i64;
                    let owed = pending.entry(session_id).or_default();
                    owed.lines += 1;
                    owed.bytes += line.len() as i64;
                }
                // Nothing already queued is ever given up to make room, and the
                // queue never holds more than a bound allows.
                assert_eq!(
                    queue.len(),
                    kept.len(),
                    "[{trace}] step {step}: queued records"
                );
                assert_eq!(
                    queue.queued_bytes(),
                    total_bytes_of(&kept),
                    "[{trace}] step {step}: queued bytes"
                );
                assert!(
                    queue.len() <= bounds.total_records,
                    "[{trace}] step {step}: over the total record bound"
                );
                assert!(
                    queue.queued_bytes() <= bounds.total_bytes,
                    "[{trace}] step {step}: over the total byte bound"
                );
                for each in sessions {
                    assert!(
                        records_of(&kept, each) <= bounds.session_records,
                        "[{trace}] step {step}: {each} holds too many records"
                    );
                    assert!(
                        bytes_of(&kept, each) <= bounds.session_bytes,
                        "[{trace}] step {step}: {each} holds too many bytes"
                    );
                }
            }
            let expected_drops =
                |session_id: &str| lost.get(session_id).copied().unwrap_or_default();
            for each in sessions {
                assert_eq!(
                    queue.dropped(each),
                    expected_drops(each),
                    "[{trace}] {each} lost exactly the lines and UTF-8 text bytes of the refused records"
                );
            }

            // A settled batch hands over every kept record in global arrival order and
            // leaves nothing reserved behind. Settling, not draining: room comes back
            // when a record is released or discarded, so "nothing left" is a statement
            // about a batch that finished, not about one that was handed out.
            let settled = settle_all(&mut queue);
            let arrivals: Vec<(&str, &str)> = settled
                .iter()
                .map(|item| (item.record.session_id.as_str(), item.record.line.as_str()))
                .collect();
            assert_eq!(arrivals, kept, "[{trace}] settled in arrival order");
            assert!(queue.is_empty(), "[{trace}] a settled queue holds nothing");
            assert_eq!(queue.len(), 0, "[{trace}] a settled queue holds nothing");
            assert_eq!(
                queue.queued_bytes(),
                0,
                "[{trace}] a settled queue owes no bytes"
            );

            // Each drop run rides out on the first accepted record after it, and
            // on no other. Comparing the whole sequence at once is what pins the
            // placement: a queue that merged two runs, moved one onto a later
            // record, or handed the same run to two records disagrees here even
            // though its per-session totals would still add up.
            let gaps: Vec<Option<DropCounters>> =
                settled.iter().map(|item| item.gap_before).collect();
            assert_eq!(
                gaps, carried,
                "[{trace}] the gap each settled record carries"
            );

            for each in sessions {
                let expected = expected_drops(each);
                // What left with this session's records, and what is still owed
                // because no record came after it.
                let carried_away = settled
                    .iter()
                    .filter(|item| item.record.session_id == each)
                    .filter_map(|item| item.gap_before)
                    .fold(DropCounters::default(), |sum, gap| DropCounters {
                        lines: sum.lines + gap.lines,
                        bytes: sum.bytes + gap.bytes,
                    });
                let residual = queue.take_pending_gap(each);
                assert_eq!(
                    residual,
                    pending.get(each).copied(),
                    "[{trace}] {each}: the residual is the trailing run, and nothing else"
                );
                assert_ne!(
                    residual,
                    Some(DropCounters::default()),
                    "[{trace}] {each}: an empty gap is None, never Some(zero)"
                );
                // Nothing is lost between the two halves and nothing is counted
                // twice, whichever way the sequence split them.
                let owed = residual.unwrap_or_default();
                assert_eq!(
                    DropCounters {
                        lines: carried_away.lines + owed.lines,
                        bytes: carried_away.bytes + owed.bytes,
                    },
                    expected,
                    "[{trace}] {each}: the carried gaps plus the residual are everything it lost"
                );
                assert_eq!(
                    queue.take_pending_gap(each),
                    None,
                    "[{trace}] {each} is owed no second gap"
                );
                assert_eq!(
                    queue.dropped(each),
                    expected,
                    "[{trace}] {each} still remembers what it lost"
                );
            }
        }
    }
    #[test]
    fn a_pending_gap_is_taken_once_and_the_cumulative_total_is_never_cleared() {
        let mut queue = ArchiveQueue::new(QueueBounds {
            session_records: 1,
            session_bytes: 64,
            total_records: 8,
            total_bytes: 512,
        });

        // Each round fills the session's one slot, loses the record behind it,
        // then drains so the round can repeat. The gap belongs to the round; the
        // total belongs to the session. One round loses an empty line.
        let mut total = DropCounters::default();
        for (round, lost) in ["one", "", "three"].iter().enumerate() {
            let at = round as i64 * 10;
            assert!(queue.enqueue(record(SESSION_A, "kept", at + 1)));
            assert!(!queue.enqueue(record(SESSION_A, lost, at + 2)));

            let gap = DropCounters {
                lines: 1,
                bytes: lost.len() as i64,
            };
            total.lines += gap.lines;
            total.bytes += gap.bytes;

            assert_eq!(
                queue.take_pending_gap(SESSION_A),
                Some(gap),
                "round {round}"
            );
            assert_eq!(
                queue.take_pending_gap(SESSION_A),
                None,
                "round {round}: the gap is owed once"
            );
            assert_eq!(queue.dropped(SESSION_A), total, "round {round}");

            settle_all(&mut queue);
            // A batch hands over records, not the drop history.
            assert_eq!(
                queue.dropped(SESSION_A),
                total,
                "round {round} after settling"
            );
            assert_eq!(
                queue.take_pending_gap(SESSION_A),
                None,
                "round {round}: settling a batch does not owe a gap"
            );
        }

        assert_eq!(total, DropCounters { lines: 3, bytes: 8 });
        // A session that lost nothing is owed nothing and remembers nothing.
        assert_eq!(queue.take_pending_gap(SESSION_B), None);
        assert_eq!(queue.dropped(SESSION_B), DropCounters::default());
    }

    #[test]
    fn finishing_a_session_hands_over_its_residual_and_totals_and_forgets_it() {
        // The queue's last word on a session: the run nobody carried plus
        // everything it lost, in one value — and then no entry at all. An entry
        // that outlives its session is a leak the process cannot bound, because
        // a long-lived RunCove runs an unbounded number of sessions.
        let mut queue = ArchiveQueue::new(gap_bounds());

        assert!(queue.enqueue(record(SESSION_A, "aa", 1)));
        assert!(!queue.enqueue(record(SESSION_A, OVER_SESSION_BYTES, 2)));
        // The pump the writer would have done: records leave, the loss stays.
        assert_eq!(lines_and_gaps(&mut queue), vec![("aa".to_string(), None)]);
        assert_eq!(queue.sessions.len(), 1, "the premise: it is tracked");

        let lost = DropCounters {
            lines: 1,
            bytes: OVER_SESSION_BYTES.len() as i64,
        };
        assert_eq!(
            queue
                .finish_session(SESSION_A)
                .expect("nothing of this session's is queued"),
            FinishedSession {
                residual_gap: Some(lost),
                dropped: lost,
            },
            "the trailing run and the cumulative total leave together, once"
        );
        assert!(
            queue.sessions.is_empty(),
            "the entry is gone: what the queue holds is bounded by open sessions"
        );
        // And it has really forgotten, rather than kept a hollow entry: it now
        // answers exactly as it would for a session it never saw.
        assert_eq!(queue.take_pending_gap(SESSION_A), None);
        assert_eq!(queue.dropped(SESSION_A), DropCounters::default());
    }
    #[test]
    fn a_session_with_queued_records_cannot_be_finished() {
        // Those records are not on disk yet, and their bytes and their carried
        // gaps are still owed to the file. Forgetting the session now would lose
        // both, so the queue refuses and the writer has to pump first.
        let mut queue = ArchiveQueue::new(gap_bounds());

        assert!(queue.enqueue(record(SESSION_A, "aa", 1)));
        assert!(!queue.enqueue(record(SESSION_A, OVER_SESSION_BYTES, 2)));

        assert!(
            queue.finish_session(SESSION_A).is_err(),
            "one queued record is enough to refuse"
        );

        // The refusal took nothing. Everything needed to pump and then finish is
        // still here, which is what makes retrying safe.
        let lost = DropCounters {
            lines: 1,
            bytes: OVER_SESSION_BYTES.len() as i64,
        };
        assert_eq!(queue.sessions.len(), 1);
        assert_eq!(queue.len(), 1, "the record is still queued");
        assert_eq!(queue.dropped(SESSION_A), lost);

        assert_eq!(lines_and_gaps(&mut queue), vec![("aa".to_string(), None)]);
        assert_eq!(
            queue
                .finish_session(SESSION_A)
                .expect("the queue was pumped"),
            FinishedSession {
                residual_gap: Some(lost),
                dropped: lost,
            },
            "the same session finishes after the pump, with nothing lost to the refusal"
        );
        assert!(queue.sessions.is_empty());
    }
    #[test]
    fn finishing_one_session_leaves_the_others_untouched() {
        // Sessions are separate files with separate rows. One ending says nothing
        // about another — not about its counters, and not about whether it is
        // ready to end. The refusal is per session, not a property of the queue.
        let mut queue = ArchiveQueue::new(gap_bounds());
        let lost = DropCounters {
            lines: 1,
            bytes: OVER_SESSION_BYTES.len() as i64,
        };

        // A loses a line and is then pumped. B's record arrives afterwards and is
        // still queued, and B loses a line after that, so B holds both a queued
        // record and a residual of its own.
        assert!(queue.enqueue(record(SESSION_A, "aa", 1)));
        assert!(!queue.enqueue(record(SESSION_A, OVER_SESSION_BYTES, 2)));
        settle_all(&mut queue);
        assert!(queue.enqueue(record(SESSION_B, "bb", 3)));
        assert!(!queue.enqueue(record(SESSION_B, OVER_SESSION_BYTES, 4)));

        assert_eq!(
            queue
                .finish_session(SESSION_A)
                .expect("A has nothing queued, whatever B is doing"),
            FinishedSession {
                residual_gap: Some(lost),
                dropped: lost,
            }
        );
        assert_eq!(
            queue
                .sessions
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![SESSION_B],
            "only A was forgotten"
        );

        assert!(
            queue.finish_session(SESSION_B).is_err(),
            "B is the one with a record queued"
        );
        assert_eq!(queue.dropped(SESSION_B), lost, "B kept its own history");

        assert_eq!(lines_and_gaps(&mut queue), vec![("bb".to_string(), None)]);
        assert_eq!(
            queue.finish_session(SESSION_B).expect("B was pumped"),
            FinishedSession {
                residual_gap: Some(lost),
                dropped: lost,
            },
            "B's residual is its own and survived A's ending"
        );
        assert!(queue.sessions.is_empty());
    }
    #[test]
    fn a_session_can_only_be_finished_once() {
        // A second hand-over is what would write the gap line twice and count the
        // same losses into the row twice. There is nothing left to take, and the
        // queue says so instead of guessing.
        let mut queue = ArchiveQueue::new(gap_bounds());
        let lost = DropCounters {
            lines: 1,
            bytes: OVER_SESSION_BYTES.len() as i64,
        };

        assert!(!queue.enqueue(record(SESSION_A, OVER_SESSION_BYTES, 1)));
        assert_eq!(
            queue.finish_session(SESSION_A).expect("nothing is queued"),
            FinishedSession {
                residual_gap: Some(lost),
                dropped: lost,
            }
        );
        assert_eq!(
            queue
                .finish_session(SESSION_A)
                .expect("finishing a forgotten session is not an error"),
            FinishedSession::default(),
            "no gap to write again, no total to count again"
        );
        // A session this queue never saw answers the same way, which is what
        // makes forgetting safe: there is one answer for "owed nothing".
        assert_eq!(
            queue.finish_session(SESSION_C).expect("an unknown session"),
            FinishedSession::default()
        );
        assert!(queue.sessions.is_empty(), "and neither call left an entry");
    }
    #[test]
    fn finishing_sessions_keeps_the_queue_from_accumulating_them() {
        // The regression this operation exists for. A RunCove process runs an
        // unbounded number of sessions over its lifetime, so what the queue
        // remembers has to be bounded by the sessions that are open, not by the
        // ones that have ended.
        let mut queue = ArchiveQueue::new(gap_bounds());
        let lost = DropCounters {
            lines: 1,
            bytes: OVER_SESSION_BYTES.len() as i64,
        };
        let owed = FinishedSession {
            residual_gap: Some(lost),
            dropped: lost,
        };
        let ids: Vec<String> = (0..50)
            .map(|n| format!("{n:08x}-d9cb-469f-a165-70867728950e"))
            .collect();

        // Fifty at once. The map is allowed to hold them: they are open.
        for (n, id) in ids.iter().enumerate() {
            assert!(!queue.enqueue(record(id, OVER_SESSION_BYTES, n as i64)));
        }
        assert_eq!(
            queue.sessions.len(),
            ids.len(),
            "open sessions are what it holds"
        );

        for id in &ids {
            assert_eq!(
                queue.finish_session(id).expect("nothing is queued"),
                owed,
                "{id}"
            );
        }
        assert!(
            queue.sessions.is_empty(),
            "a finished batch leaves no entries behind"
        );

        // And run one after another, it never holds more than the one running.
        for (n, id) in ids.iter().enumerate() {
            let at = 1_000 + n as i64;
            assert!(queue.enqueue(record(id, "aa", at)));
            assert!(!queue.enqueue(record(id, OVER_SESSION_BYTES, at + 1)));
            assert_eq!(queue.sessions.len(), 1, "while {id} runs");
            settle_all(&mut queue);
            assert_eq!(queue.finish_session(id).expect("pumped"), owed, "{id}");
            assert!(queue.sessions.is_empty(), "after {id}");
        }
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.queued_bytes(), 0);
    }
    #[test]
    fn a_record_after_a_finish_starts_a_fresh_entry() {
        // The cost of forgetting, stated so the writer's obligation is a test and
        // not a comment: a finish is final, so a record that arrives after one is
        // a new session's worth of history. It carries no gap and reports no
        // losses, because the run that was owed left with the finish and must not
        // be written twice. The writer therefore finishes a session only once its
        // capture threads are done with it.
        let mut queue = ArchiveQueue::new(gap_bounds());
        let lost = DropCounters {
            lines: 1,
            bytes: OVER_SESSION_BYTES.len() as i64,
        };

        assert!(queue.enqueue(record(SESSION_A, "aa", 1)));
        assert!(!queue.enqueue(record(SESSION_A, OVER_SESSION_BYTES, 2)));
        assert_eq!(lines_and_gaps(&mut queue), vec![("aa".to_string(), None)]);
        assert_eq!(
            queue.finish_session(SESSION_A).expect("pumped"),
            FinishedSession {
                residual_gap: Some(lost),
                dropped: lost,
            }
        );
        assert!(queue.sessions.is_empty());

        // A late record. The queue has no reason to refuse it, and no history to
        // give it.
        assert!(queue.enqueue(record(SESSION_A, "bb", 3)));
        assert_eq!(queue.sessions.len(), 1, "a fresh entry, not a resurrection");
        assert_eq!(
            queue.dropped(SESSION_A),
            DropCounters::default(),
            "the losses were already handed over once"
        );
        assert_eq!(
            lines_and_gaps(&mut queue),
            vec![("bb".to_string(), None)],
            "a record after a finish carries no gap"
        );
        assert_eq!(
            queue.finish_session(SESSION_A).expect("pumped"),
            FinishedSession::default(),
            "and the second finish is owed nothing"
        );
        assert!(queue.sessions.is_empty());
    }

    /// The retry contract, at the queue's own level: a pump that fails must leave the
    /// records it could not write exactly where they were, and the next pump must get
    /// the same ones in the same order.
    ///
    /// That is why neither [`ArchiveQueue::begin_batch`] nor
    /// [`ArchiveQueue::take_front`] frees anything. The queue owns every record until
    /// it is settled, so a failed pump needs no undo — there is nothing to put back,
    /// and no local batch to lose when it returns. [`ArchiveQueue::release`] and
    /// [`ArchiveQueue::discard`] are the only two calls that free room, and exactly one
    /// of them settles each record.
    #[test]
    fn a_failed_batch_stays_in_flight_and_the_next_batch_appends_behind_it() {
        let mut queue = ArchiveQueue::new(roomy_bounds());

        assert!(queue.enqueue(record(SESSION_A, "a1", 1)));
        assert!(queue.enqueue(record(SESSION_A, "a2", 2)));
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.queued_bytes(), 4);
        // Nothing is in flight until a pump asks for a batch.
        assert_eq!(queue.peek_front(), None);

        // Asking for one hands nothing over and frees nothing.
        queue.begin_batch();
        assert_eq!(
            queue.len(),
            2,
            "a batch is still the queue's to account for"
        );
        assert_eq!(queue.queued_bytes(), 4);
        assert!(!queue.is_empty());
        assert_eq!(queue.peek_front(), Some((SESSION_A, 2)));

        // The pump takes the front record to write it. Its room stays charged for as
        // long as its fate is undecided.
        let first = queue.take_front().expect("the front record");
        assert_eq!(first.record.line, "a1");
        assert_eq!(queue.len(), 2, "a taken record is not a freed record");
        assert_eq!(queue.queued_bytes(), 4);
        assert_eq!(queue.peek_front(), Some((SESSION_A, 2)), "now a2");
        assert!(
            queue.finish_session(SESSION_A).is_err(),
            "a session with records in flight cannot be finished"
        );

        queue.release(&first);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.queued_bytes(), 2);

        // Here the pump fails and returns, leaving `a2` in flight and unsettled. A
        // capture thread hands over one more record, and the next pump asks for a
        // batch again.
        assert!(queue.enqueue(record(SESSION_A, "a3", 3)));
        assert_eq!(queue.len(), 2);
        queue.begin_batch();

        let mut written = Vec::new();
        while let Some(front) = queue.take_front() {
            written.push(front.record.line.clone());
            queue.release(&front);
        }
        assert_eq!(
            written,
            vec!["a2".to_string(), "a3".to_string()],
            "the retry resumes where it stopped, and the new record queues behind it"
        );

        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.queued_bytes(), 0);
        assert_eq!(queue.peek_front(), None);
        // Nothing was lost along the way: a retry is not a drop.
        assert_eq!(queue.dropped(SESSION_A), DropCounters::default());
        assert_eq!(
            queue.finish_session(SESSION_A).expect("everything settled"),
            FinishedSession::default()
        );
    }

    /// Every one of the four bounds counts a record that is in flight, because a record
    /// whose fate is undecided is still occupying memory. If any of them stopped
    /// counting it, a long run of failing pumps would admit an unbounded number of
    /// records behind the batch it cannot write — which is the whole thing the bounds
    /// exist to prevent.
    ///
    /// One case per bound, each set so that only that bound can be the one refusing:
    /// the per-session cases ask again as the same session, the total cases ask as
    /// another one whose own room is untouched.
    #[test]
    fn an_in_flight_batch_counts_against_all_four_bounds() {
        for (bound, bounds, asking) in [
            (
                "session_records",
                QueueBounds {
                    session_records: 1,
                    session_bytes: 1 << 20,
                    total_records: 64,
                    total_bytes: 1 << 20,
                },
                SESSION_A,
            ),
            (
                "session_bytes",
                QueueBounds {
                    session_records: 64,
                    session_bytes: 2,
                    total_records: 64,
                    total_bytes: 1 << 20,
                },
                SESSION_A,
            ),
            (
                "total_records",
                QueueBounds {
                    session_records: 64,
                    session_bytes: 1 << 20,
                    total_records: 1,
                    total_bytes: 1 << 20,
                },
                SESSION_B,
            ),
            (
                "total_bytes",
                QueueBounds {
                    session_records: 64,
                    session_bytes: 1 << 20,
                    total_records: 64,
                    total_bytes: 2,
                },
                SESSION_B,
            ),
        ] {
            let mut queue = ArchiveQueue::new(bounds);
            assert!(queue.enqueue(record(SESSION_A, "aa", 1)), "{bound}");

            // In flight, and still charged, so the next record does not fit.
            queue.begin_batch();
            assert!(
                !queue.enqueue(record(asking, "bb", 2)),
                "{bound}: in flight"
            );
            assert_eq!(
                queue.dropped(asking),
                DropCounters { lines: 1, bytes: 2 },
                "{bound}: the refused record is the asking session's loss"
            );

            // Taken by the pump and not yet settled, which is the same answer: the
            // record still exists and still costs what it cost.
            let front = queue.take_front().expect("the front record");
            assert!(!queue.enqueue(record(asking, "cc", 3)), "{bound}: taken");

            // Settled, so the room comes back and the next record fits.
            queue.release(&front);
            assert!(queue.enqueue(record(asking, "dd", 4)), "{bound}: released");

            assert_eq!(
                queue.dropped(asking),
                DropCounters { lines: 2, bytes: 4 },
                "{bound}: two refusals, and no third"
            );
            assert_eq!(
                queue.take_pending_gap(asking),
                None,
                "{bound}: the accepted record carried both losses out"
            );
        }
    }

    /// A discarded carrier's gap goes back to `pending`, and to nothing else.
    ///
    /// The loss it carries was counted the moment [`ArchiveQueue::enqueue`] refused the
    /// lines it stands for, so adding it to `dropped` again here would report the same
    /// lost lines twice — and the row's drop counters are the only account of them that
    /// anyone will ever read. Returning it to `pending` is what keeps it attached to
    /// whatever survives next, or to the residual gap at close.
    #[test]
    fn discarding_a_gap_carrier_does_not_count_the_carried_loss_twice() {
        let mut queue = ArchiveQueue::new(gap_bounds());
        let refused = DropCounters {
            lines: 1,
            bytes: OVER_SESSION_BYTES.len() as i64,
        };

        // A loss, then the record that has to carry it out to the file.
        assert!(!queue.enqueue(record(SESSION_A, OVER_SESSION_BYTES, 1)));
        assert!(queue.enqueue(record(SESSION_A, "aa", 2)));
        assert_eq!(queue.dropped(SESSION_A), refused);

        queue.begin_batch();
        let carrier = queue.take_front().expect("the carrier");
        assert_eq!(carrier.gap_before, Some(refused));

        // The write failed, so this carrier will never reach the disk.
        queue.discard(carrier);
        let both = DropCounters {
            lines: refused.lines + 1,
            bytes: refused.bytes + 2,
        };
        assert_eq!(
            queue.dropped(SESSION_A),
            both,
            "the discarded record is a new loss; the one it carried was already counted"
        );
        assert_eq!(queue.len(), 0, "a discarded record frees its room");
        assert_eq!(queue.queued_bytes(), 0);

        // The gap is still owed, so the next record to survive carries both losses at
        // once — one gap line, not two, and none of it counted twice.
        assert!(queue.enqueue(record(SESSION_A, "bb", 3)));
        queue.begin_batch();
        let next = queue.take_front().expect("the next record");
        assert_eq!(next.gap_before, Some(both));
        queue.release(&next);
        assert_eq!(
            queue.finish_session(SESSION_A).expect("everything settled"),
            FinishedSession {
                residual_gap: None,
                dropped: both,
            },
            "the cumulative total is both losses, counted once each"
        );
    }

    /// A session whose file has just failed takes its own in-flight records down with
    /// it and leaves every other session's alone.
    ///
    /// The close path needs exactly this: [`ArchiveQueue::finish_session`] refuses while
    /// any of the session's records are still counted against it, and after a write
    /// error there is no file left to write them to. Their loss is owed to the row's
    /// drop counters; the residual gap it also reports cannot be written for the same
    /// reason the records cannot, so the write-error close is the one caller that
    /// answers it with nothing.
    #[test]
    fn discarding_one_sessions_in_flight_records_leaves_the_others_alone() {
        let mut queue = ArchiveQueue::new(roomy_bounds());
        let lost = DropCounters { lines: 2, bytes: 4 };

        // Interleaved, so a discard that walked the batch by position rather than by
        // session would take the wrong records.
        assert!(queue.enqueue(record(SESSION_A, "a1", 1)));
        assert!(queue.enqueue(record(SESSION_B, "b1", 2)));
        assert!(queue.enqueue(record(SESSION_A, "a2", 3)));
        assert!(queue.enqueue(record(SESSION_B, "b2", 4)));
        queue.begin_batch();

        queue.discard_session(SESSION_A);
        assert_eq!(
            queue.dropped(SESSION_A),
            lost,
            "both of the failed session's records are counted losses"
        );
        assert_eq!(queue.dropped(SESSION_B), DropCounters::default());
        assert_eq!(queue.len(), 2, "only the failed session's room came back");
        assert_eq!(queue.queued_bytes(), 4);

        // This is the write-error close, in the order the writer will do it: discard the
        // failed session's records, then finish its entry for the row's counters — while
        // the other session's batch is still mid-flight. The residual gap comes back
        // because the queue owes it; the write-error close is the one caller that has
        // nowhere to write it and must answer it with nothing.
        assert_eq!(
            queue
                .finish_session(SESSION_A)
                .expect("nothing of A's is still in flight"),
            FinishedSession {
                residual_gap: Some(lost),
                dropped: lost,
            },
            "the discarded records are owed to the row"
        );
        assert_eq!(
            queue.len(),
            2,
            "finishing one session leaves the other's batch alone"
        );

        let mut written = Vec::new();
        while let Some(front) = queue.take_front() {
            written.push(front.record.line.clone());
            queue.release(&front);
        }
        assert_eq!(
            written,
            vec!["b1".to_string(), "b2".to_string()],
            "the discard took the failed session's records and nothing else, in order"
        );

        assert_eq!(
            queue
                .finish_session(SESSION_B)
                .expect("everything of B's settled"),
            FinishedSession::default(),
            "the session whose file was fine lost nothing"
        );
        assert!(queue.is_empty());
        assert_eq!(queue.queued_bytes(), 0);
        assert!(queue.sessions.is_empty());
    }

    #[test]
    fn a_gap_line_reports_the_lines_and_bytes_lost() {
        assert_eq!(
            gap_line(DropCounters {
                lines: 41,
                bytes: 6_218
            }),
            "[RunCove: dropped 41 lines / 6218 bytes]"
        );
        // The line is shown to a user, so it is written as English.
        assert_eq!(
            gap_line(DropCounters { lines: 1, bytes: 1 }),
            "[RunCove: dropped 1 line / 1 byte]"
        );
    }

    #[test]
    fn gap_records_sum_to_the_dropped_counters_of_a_closed_archive() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            small_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer.begin(SESSION_A, 10_001).expect("the session opens");

        // Two records fit, so the third and fourth are lost as one contiguous
        // gap.
        for (offset, line) in ["one", "two", "three", "four"].iter().enumerate() {
            writer.enqueue(record(SESSION_A, line, 10_002 + offset as i64));
        }
        writer.pump(10_010).expect("the queue drains");
        writer.enqueue(record(SESSION_A, "after the gap", 10_011));
        writer.pump(10_012).expect("the queue drains again");
        writer
            .close(SESSION_A, None, 10_013)
            .expect("the session closes");

        let text =
            fs::read_to_string(archive_dir.join(name_of(SESSION_A))).expect("the archive file");
        let records: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
            .collect();

        // The gap stands where the lost records were: after what was kept and
        // before what came next.
        let streams: Vec<&str> = records
            .iter()
            .map(|entry| entry["s"].as_str().expect("a stream"))
            .collect();
        assert_eq!(streams, vec!["stdout", "stdout", "system", "stdout"]);

        let dropped = DropCounters {
            lines: 2,
            bytes: ("three".len() + "four".len()) as i64,
        };
        assert_eq!(records[2]["l"], gap_line(dropped));

        let row = index.snapshot(SESSION_A).expect("the row");
        assert_eq!(row.counters.dropped_lines, dropped.lines);
        assert_eq!(row.counters.dropped_bytes, dropped.bytes);

        // The gap record itself is an archived line.
        assert_eq!(row.counters.line_count, 4);

        // A session keeps archiving after a drop, and says so when it closes.
        assert_eq!(
            index.state_of(SESSION_A),
            Some(("partial".into(), Some("queue-overflow".into())))
        );
    }

    #[test]
    fn a_close_writes_the_trailing_gap_no_later_record_could_carry() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            small_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        writer.begin(SESSION_A, 10_001).expect("the session opens");

        // Two records fit, so the third and fourth are lost — and this time nothing
        // arrives after them, so no record can carry their gap.
        for (offset, line) in ["one", "two", "three", "four"].iter().enumerate() {
            writer.enqueue(record(SESSION_A, line, 10_002 + offset as i64));
        }
        writer.pump(10_010).expect("the queue drains");
        writer
            .close(SESSION_A, None, 10_013)
            .expect("the session closes");

        let text =
            fs::read_to_string(archive_dir.join(name_of(SESSION_A))).expect("the archive file");
        let records: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
            .collect();

        // The gap is the last line of the file, after everything that was kept.
        let streams: Vec<&str> = records
            .iter()
            .map(|entry| entry["s"].as_str().expect("a stream"))
            .collect();
        assert_eq!(streams, vec!["stdout", "stdout", "system"]);

        let dropped = DropCounters {
            lines: 2,
            bytes: ("three".len() + "four".len()) as i64,
        };
        assert_eq!(records[2]["l"], gap_line(dropped));
        // The close reads no clock, so the gap carries the instant the close was given.
        assert_eq!(records[2]["t"], 10_013);

        let row = index.snapshot(SESSION_A).expect("the row");
        assert_eq!(row.counters.dropped_lines, dropped.lines);
        assert_eq!(row.counters.dropped_bytes, dropped.bytes);
        // Three archived lines, the gap among them, and a byte count that is the file's
        // real length — the gap's own bytes included.
        assert_eq!(row.counters.line_count, 3);
        assert_eq!(row.counters.byte_size, text.len() as i64);
        assert_eq!(
            index.state_of(SESSION_A),
            Some(("partial".into(), Some("queue-overflow".into())))
        );
        assert_no_queue_entry(&writer, SESSION_A);
    }

    // Group 4: the byte caps and eviction.

    #[test]
    fn the_per_session_cap_closes_that_session_partial_quota_exceeded() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            QuotaLimits {
                session_bytes: 256,
                total_bytes: 4_096,
            },
        )
        .expect("an initialized writer");

        writer
            .begin(SESSION_A, 10_001)
            .expect("the first session opens");
        writer
            .begin(SESSION_B, 10_002)
            .expect("the second session opens");

        let line = "y".repeat(100);
        writer.enqueue(record(SESSION_A, &line, 10_003));
        writer.enqueue(record(SESSION_A, &line, 10_004));
        writer.enqueue(record(SESSION_B, "short", 10_005));
        writer.pump(10_006).expect("the queue drains");

        // The cap is a limit on what reaches the disk, so the record that would
        // cross it is dropped rather than written.
        let path = archive_dir.join(name_of(SESSION_A));
        let written = fs::metadata(&path).expect("the archive file").len();
        assert!(written <= 256, "wrote {written} bytes past a 256 byte cap");

        let row = index.snapshot(SESSION_A).expect("the row");
        assert_eq!(
            (row.status.as_str(), row.reason.as_deref()),
            ("partial", Some("quota-exceeded"))
        );
        assert_eq!(row.counters.dropped_lines, 1);
        assert_eq!(row.counters.dropped_bytes, line.len() as i64);
        assert!(!writer.is_open(SESSION_A));

        // One session's cap is not another's.
        assert_eq!(index.state_of(SESSION_B), Some(("writing".into(), None)));
        assert!(writer.is_open(SESSION_B));
    }

    /// A cap reached in the middle of a batch. The record that would cross it is
    /// dropped and the session closes, and the records behind it in the same batch —
    /// accepted from a capture thread, never persisted — have to be counted as well.
    /// Discarding a batch remainder silently is what this test forbids: the row would
    /// claim one lost line where three lines never reached the disk, and nothing
    /// downstream could then tell a truncated archive from a complete one.
    ///
    /// A quota refusal writes no gap line — the session is stopping because the file
    /// is at its cap, and a gap line is more bytes in that same file — so the row's
    /// counters are the only surviving record of the loss, and they have to be exact.
    #[test]
    fn a_quota_close_counts_every_record_it_never_wrote() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            QuotaLimits {
                session_bytes: 256,
                total_bytes: 4_096,
            },
        )
        .expect("an initialized writer");

        writer
            .begin(SESSION_A, 10_001)
            .expect("the capped session opens");
        writer
            .begin(SESSION_B, 10_002)
            .expect("the other session opens");

        let line = "y".repeat(100);
        // 132 bytes on disk, which fits; a second one would make 264, which does not.
        writer.enqueue(record(SESSION_A, &line, 10_003));
        writer.enqueue(record(SESSION_A, &line, 10_004));
        // Accepted before anything was pumped, so they are the writer's to account
        // for even though the session stops before reaching them.
        writer.enqueue(record(SESSION_A, "third", 10_005));
        writer.enqueue(record(SESSION_A, "fourth", 10_006));
        writer.enqueue(record(SESSION_B, "short", 10_007));
        writer.pump(10_008).expect("the queue drains");

        let body =
            fs::read_to_string(archive_dir.join(name_of(SESSION_A))).expect("the capped archive");
        assert_eq!(
            body,
            format!("{}\n", encode_record(&record(SESSION_A, &line, 10_003))),
            "only the record that fit the cap belongs in the file, and no gap line"
        );
        assert_eq!(
            body.len(),
            132,
            "31 for the record's frame, 100 of text, 1 newline"
        );

        let row = index.snapshot(SESSION_A).expect("the capped row");
        assert_eq!(
            (row.status.as_str(), row.reason.as_deref()),
            ("partial", Some("quota-exceeded"))
        );
        assert_eq!(row.counters.line_count, 1);
        assert_eq!(row.counters.byte_size, 132);
        // The record that crossed the cap, and the two that were still behind it.
        assert_eq!(row.counters.dropped_lines, 3);
        assert_eq!(
            row.counters.dropped_bytes,
            (line.len() + "third".len() + "fourth".len()) as i64
        );
        assert_eq!(row.ended_at, Some(10_008));
        assert!(!writer.is_open(SESSION_A));

        // One session's cap is not another's, and one session's abandoned batch
        // remainder is not another session's loss.
        assert!(writer.is_open(SESSION_B));
        writer
            .close(SESSION_B, None, 10_009)
            .expect("the other session closes");
        let other = index.snapshot(SESSION_B).expect("the other row");
        assert_eq!(
            (other.status.as_str(), other.reason.as_deref()),
            ("complete", None)
        );
        assert_eq!(other.counters.line_count, 1);
        assert_eq!(other.counters.dropped_lines, 0);
        assert_eq!(other.counters.dropped_bytes, 0);
        assert_eq!(
            fs::read_to_string(archive_dir.join(name_of(SESSION_B))).expect("the other archive"),
            format!("{}\n", encode_record(&record(SESSION_B, "short", 10_007)))
        );
        assert!(seam.removed().is_empty());
    }

    #[test]
    fn the_total_cap_evicts_ended_archives_oldest_first_and_never_an_open_one() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");

        // Two archives an earlier run left behind, the first one older.
        for (session, size) in [(SESSION_A, 400_usize), (SESSION_B, 400)] {
            fs::write(archive_dir.join(name_of(session)), "z".repeat(size))
                .expect("an earlier archive");
        }
        let index = TestIndex::shared();
        let mut older = seeded_row(SESSION_A, "complete", None, 400);
        older.started_at = 500;
        index.seed(older);
        let mut newer = seeded_row(SESSION_B, "complete", None, 400);
        newer.started_at = 900;
        index.seed(newer);

        let seam = TestFs::shared();
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            QuotaLimits {
                session_bytes: 4_096,
                total_bytes: 1_000,
            },
        )
        .expect("an initialized writer");
        assert_eq!(report.measured_bytes, QuotaTotal::Known(800));
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(800));

        writer
            .begin(SESSION_C, 10_001)
            .expect("a new session opens");
        writer.enqueue(record(SESSION_C, &"w".repeat(300), 10_002));
        writer.pump(10_003).expect("the queue drains");

        // The oldest ended archive goes first, and only as far as the cap needs.
        assert_eq!(
            index.state_of(SESSION_A),
            Some(("removed".into(), Some("quota-evicted".into())))
        );
        assert!(!archive_dir.join(name_of(SESSION_A)).exists());
        assert_eq!(index.state_of(SESSION_B), Some(("complete".into(), None)));
        assert!(archive_dir.join(name_of(SESSION_B)).is_file());

        // The running session keeps its file and its row.
        assert_eq!(index.state_of(SESSION_C), Some(("writing".into(), None)));
        assert!(writer.is_open(SESSION_C));
        assert!(matches!(
            writer.total_bytes(),
            QuotaTotal::Known(bytes) if bytes <= 1_000
        ));
    }

    /// Eviction order is `ended_at`, then `started_at`, then the session id, and the
    /// first key is what this test is for. The two archives disagree: one ended last
    /// but started first, the other ended first but started last, so only one key can
    /// decide and the wrong one deletes the wrong archive.
    /// [`the_total_cap_evicts_ended_archives_oldest_first_and_never_an_open_one`]
    /// cannot see the difference, because both of its rows carry the same `ended_at`
    /// and its verdict rests entirely on the tie-break.
    ///
    /// The third archive is the eligibility half of the same rule: `complete` with no
    /// `ended_at`. RunCove's own schema forbids that row — version 2 checks
    /// `(status = 'writing') = (ended_at IS NULL)` — so it can only arrive from a
    /// database this build did not write, which is why the filter cannot be left to
    /// the schema. A row with no end has no place in an order keyed on when sessions
    /// ended, and reading a missing timestamp as zero would make it the first
    /// candidate of all.
    #[test]
    fn eviction_orders_by_ended_at_and_skips_a_session_that_never_ended() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        for (session, size) in [(SESSION_A, 400_usize), (SESSION_B, 400), (SESSION_D, 100)] {
            fs::write(archive_dir.join(name_of(session)), "z".repeat(size))
                .expect("an earlier archive");
        }

        let index = TestIndex::shared();
        // Started first, ended last.
        let mut ended_last = seeded_row(SESSION_A, "complete", None, 400);
        ended_last.started_at = 500;
        ended_last.ended_at = Some(3_000);
        index.seed(ended_last);
        // Started last, ended first, and therefore the candidate.
        let mut ended_first = seeded_row(SESSION_B, "complete", None, 400);
        ended_first.started_at = 900;
        ended_first.ended_at = Some(2_000);
        index.seed(ended_first);
        // Never ended, so never a candidate.
        let mut unended = seeded_row(SESSION_D, "complete", None, 100);
        unended.ended_at = None;
        index.seed(unended.clone());

        let seam = TestFs::shared();
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            QuotaLimits {
                session_bytes: 4_096,
                total_bytes: 1_000,
            },
        )
        .expect("an initialized writer");
        assert_eq!(report.measured_bytes, QuotaTotal::Known(900));
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(900));

        writer
            .begin(SESSION_C, 10_001)
            .expect("a new session opens");
        // 332 bytes on disk: 900 + 332 is over the cap, and freeing one 400 byte
        // archive is enough to get under it.
        writer.enqueue(record(SESSION_C, &"w".repeat(300), 10_002));
        writer.pump(10_003).expect("the queue drains");

        // Ended first, evicted first, though it started last.
        assert_eq!(
            index.state_of(SESSION_B),
            Some(("removed".into(), Some("quota-evicted".into())))
        );
        assert!(!archive_dir.join(name_of(SESSION_B)).exists());
        // Started first and kept: `started_at` breaks a tie, it does not decide.
        assert_eq!(index.state_of(SESSION_A), Some(("complete".into(), None)));
        assert!(archive_dir.join(name_of(SESSION_A)).is_file());
        // Not a candidate, so not touched in any respect.
        assert_eq!(index.snapshot(SESSION_D), Some(unended));
        assert!(archive_dir.join(name_of(SESSION_D)).is_file());
        assert_eq!(
            seam.removed(),
            vec![archive_dir.join(name_of(SESSION_B))],
            "one eviction was enough, and it was the right one"
        );

        assert_eq!(index.state_of(SESSION_C), Some(("writing".into(), None)));
        assert!(writer.is_open(SESSION_C));
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(832));
    }

    /// An eviction whose file is gone and whose row will not move. The bytes are
    /// really free — the disk says so — so the quota has to credit what the disk lost,
    /// not what the stale row claimed, or the archive would refuse room it actually
    /// has. The row is left inconsistent for the next startup sweep to repair, and the
    /// failure is reported rather than swallowed.
    ///
    /// This is [`a_delete_whose_row_will_not_move_still_frees_the_bytes_it_removed`]
    /// on the eviction path, and the discriminator is the same: the row here says ten
    /// bytes where the file held four hundred, so a total computed from the row is a
    /// different number from a total computed from the disk.
    ///
    /// The session doing the writing is not at fault and does not close: a second pump
    /// finishes its work, which is also how the test pins that its record was neither
    /// lost nor written twice, whichever side of the failed eviction the write fell on.
    #[test]
    fn an_eviction_whose_row_will_not_move_still_frees_the_bytes_it_removed() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        fs::write(archive_dir.join(name_of(SESSION_A)), "z".repeat(400))
            .expect("an earlier archive");

        let index = TestIndex::shared();
        // Ten bytes in the row, four hundred on the disk.
        let stale = seeded_row(SESSION_A, "complete", None, 10);
        index.seed(stale.clone());

        let seam = TestFs::shared();
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            QuotaLimits {
                session_bytes: 4_096,
                total_bytes: 500,
            },
        )
        .expect("an initialized writer");
        assert_eq!(report.measured_bytes, QuotaTotal::Known(400));

        writer
            .begin(SESSION_C, 10_001)
            .expect("a new session opens");
        let big = "w".repeat(300);
        writer.enqueue(record(SESSION_C, &big, 10_002));
        index.fail("mark_removed");
        assert!(
            writer.pump(10_003).is_err(),
            "an eviction that could not be recorded is reported, not swallowed"
        );

        // The file is gone and the row still describes it, which is the state the
        // sweep exists to repair.
        assert!(!archive_dir.join(name_of(SESSION_A)).exists());
        assert_eq!(seam.removed(), vec![archive_dir.join(name_of(SESSION_A))]);
        assert_eq!(index.snapshot(SESSION_A), Some(stale.clone()));
        let calls = index.calls();
        assert!(
            calls.contains(&format!("refused:mark_removed:{SESSION_A}")),
            "{calls:?}"
        );

        // Four hundred bytes came back, so the total is under four hundred whether or
        // not the record was written before the eviction failed. Crediting the row's
        // ten would leave it at or above.
        let after = writer.total_bytes();
        assert!(
            matches!(after, QuotaTotal::Known(bytes) if bytes < 400),
            "the evicted bytes are still on the books: {after:?}"
        );

        // The writing session was not the one that failed.
        assert_eq!(index.state_of(SESSION_C), Some(("writing".into(), None)));
        assert!(writer.is_open(SESSION_C));

        index.allow("mark_removed");
        writer.pump(10_004).expect("the pump recovers");
        assert_eq!(
            fs::read_to_string(archive_dir.join(name_of(SESSION_C))).expect("the new archive"),
            format!("{}\n", encode_record(&record(SESSION_C, &big, 10_002))),
            "the record is written exactly once across the two pumps"
        );
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(332));
        // Still the sweep's job, not something a later pump quietly cleaned up.
        assert_eq!(index.snapshot(SESSION_A), Some(stale));
        assert_eq!(entry_count(&archive_dir), 1);
    }

    /// A failure that can happen again and again without progressing: eviction picks a
    /// real candidate and the removal itself will not go through. On Windows that is an
    /// ordinary transient — a scanner or the indexer holds a handle for a moment — and
    /// it is a different state from
    /// [`when_nothing_can_be_evicted_the_open_session_closes_partial_quota_exceeded`],
    /// where nothing eligible exists at all. So the rule is: report it and try again on
    /// the next tick. Closing a session's archive because someone else's file could not
    /// be deleted would throw its logs away for a reason that has nothing to do with it.
    ///
    /// The two failures that look repeatable and are not: `index.fail("mark_removed")`
    /// progresses, because the file is gone and its bytes credited on the first round, so
    /// later rounds need no eviction at all; and a write error is terminal for its
    /// session, so there is nothing left open to fail a second time. A removal that keeps
    /// refusing is the only one that leaves the writer in the state it started in.
    ///
    /// What has to hold across an indefinite run of these: every queued record still
    /// there, in order, written exactly once when it finally goes through, and all four
    /// bounds still refusing new records the whole time. A retry loop that quietly grew
    /// the queue instead would be the unbounded memory the bounds exist to prevent.
    #[test]
    fn repeated_eviction_failures_keep_every_record_and_never_relax_the_bounds() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        let evictable = archive_dir.join(name_of(SESSION_A));
        fs::write(&evictable, "z".repeat(400)).expect("an earlier archive");

        let index = TestIndex::shared();
        let candidate = seeded_row(SESSION_A, "complete", None, 400);
        index.seed(candidate.clone());
        let seam = TestFs::shared();
        // Two record slots, per session and in total, so a third record can only be
        // refused. 400 on the disk plus the 332 the first record needs is over the cap,
        // and freeing the one 400 byte archive is enough to get under it.
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            QueueBounds {
                session_records: 2,
                session_bytes: 1 << 20,
                total_records: 2,
                total_bytes: 1 << 20,
            },
            QuotaLimits {
                session_bytes: 4_096,
                total_bytes: 500,
            },
        )
        .expect("an initialized writer");
        assert_eq!(report.measured_bytes, QuotaTotal::Known(400));

        writer
            .begin(SESSION_C, 10_001)
            .expect("a new session opens");
        let first = "w".repeat(300);
        let second = "second";
        writer.enqueue(record(SESSION_C, &first, 10_002));
        writer.enqueue(record(SESSION_C, second, 10_003));

        let queued = || {
            let queue = writer.queue.lock().expect("the queue");
            (queue.len(), queue.queued_bytes(), queue.dropped(SESSION_C))
        };
        assert_eq!(
            queued(),
            (2, first.len() + second.len(), DropCounters::default())
        );

        // Sticky, and nothing in the writer clears it: every round finds the same
        // candidate and fails on the same removal.
        seam.fail_remove();
        for round in 1..=3i64 {
            writer.enqueue(record(SESSION_C, "no", 10_010 + round));
            assert!(
                writer.pump(10_020 + round).is_err(),
                "round {round}: an eviction whose file will not go is reported"
            );
            assert_eq!(
                queued(),
                (
                    2,
                    first.len() + second.len(),
                    DropCounters {
                        lines: round,
                        bytes: round * 2,
                    },
                ),
                "round {round}: both records still held, and one refusal per round"
            );
            assert!(evictable.is_file(), "round {round}: nothing was removed");
            assert_eq!(
                writer.total_bytes(),
                QuotaTotal::Known(400),
                "round {round}: no bytes were credited for a file that is still there"
            );
            assert_eq!(
                index.snapshot(SESSION_A),
                Some(candidate.clone()),
                "round {round}: the candidate's row is untouched"
            );
            assert_eq!(
                index.state_of(SESSION_C),
                Some(("writing".into(), None)),
                "round {round}: the writing session is not the one at fault"
            );
            assert!(writer.is_open(SESSION_C), "round {round}");
        }
        assert_eq!(
            seam.removed(),
            vec![evictable.clone(); 3],
            "one attempt per round, all on the same candidate"
        );
        let calls = index.calls();
        assert!(
            !calls.iter().any(|call| call.starts_with("mark_removed:")),
            "a file that is still there is not a removed row: {calls:?}"
        );

        // The transient passes. One eviction is still all it takes, and the records that
        // waited through three failures are written once each, in the order they arrived.
        seam.allow_remove();
        writer.pump(10_030).expect("the pump recovers");
        assert_eq!(
            fs::read_to_string(archive_dir.join(name_of(SESSION_C))).expect("the new archive"),
            format!(
                "{}\n{}\n",
                encode_record(&record(SESSION_C, &first, 10_002)),
                encode_record(&record(SESSION_C, second, 10_003))
            ),
            "both records, in order, each written exactly once across the four pumps"
        );
        // No gap line: both survivors were queued before any refusal, so the three
        // losses are still pending and belong to whatever record survives next.
        assert_eq!(
            queued(),
            (0, 0, DropCounters { lines: 3, bytes: 6 }),
            "the queue is empty and the refusals are still on the session's books"
        );

        assert_eq!(
            seam.removed(),
            vec![evictable.clone(); 4],
            "the fourth attempt is the one that worked"
        );
        assert!(!evictable.exists());
        assert_eq!(
            index.state_of(SESSION_A),
            Some(("removed".into(), Some("quota-evicted".into())))
        );
        assert_eq!(
            index
                .calls()
                .iter()
                .filter(|call| call.starts_with("mark_removed:"))
                .count(),
            1,
            "the row moved once, not once per attempt"
        );
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(370));
        assert_eq!(entry_count(&archive_dir), 1);
        assert!(writer.is_open(SESSION_C));
    }

    /// An eviction candidate whose row names another session's file. The name is data out
    /// of a database this build does not exclusively own, so the ownership gate —
    /// [`verified_file_name`] — has to run on the eviction path too, or the quota deletes
    /// a file on the strength of a row that does not describe it. This is the one field a
    /// row cannot be trusted about: every other one is a number that might be wrong,
    /// while `file_name` decides which bytes go.
    ///
    /// A crossed row is reported, not skipped. Skipping to the next candidate would make
    /// a corrupt row invisible, and the writing session's records have to survive the
    /// failure either way: the pump returns `Err` with the record still queued, so the
    /// retry after the row is repaired writes it exactly once.
    ///
    /// The crossed row is seeded after `initialize` on purpose. At sweep time it would
    /// leave the file unowned and `delete_orphans` would take it — a different rule with
    /// its own tests — so the file would be gone before the quota ever looked at it.
    #[test]
    fn evicting_a_row_that_names_another_session_touches_nothing_and_loses_no_record() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        let owned = archive_dir.join(name_of(SESSION_A));
        fs::write(&owned, "z".repeat(400)).expect("an earlier archive");

        let index = TestIndex::shared();
        // Correct at sweep time, so the file is owned and its 400 bytes are measured.
        index.seed(seeded_row(SESSION_A, "complete", None, 400));

        let seam = TestFs::shared();
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            QuotaLimits {
                session_bytes: 4_096,
                total_bytes: 500,
            },
        )
        .expect("an initialized writer");
        assert_eq!(report.measured_bytes, QuotaTotal::Known(400));

        writer
            .begin(SESSION_B, 10_001)
            .expect("a new session opens");
        let open_row = index.snapshot(SESSION_B).expect("the writing row");

        // Now the row goes wrong: the same session, another session's file name. A gate
        // that trusted the row would remove D's file to evict A.
        let mut crossed = seeded_row(SESSION_A, "complete", None, 400);
        crossed.file_name = name_of(SESSION_D);
        index.seed(crossed.clone());

        // 400 on the disk plus the 332 this record needs is over the cap, so the write
        // cannot happen until something is evicted.
        let big = "w".repeat(300);
        writer.enqueue(record(SESSION_B, &big, 10_002));
        assert!(
            writer.pump(10_003).is_err(),
            "a candidate whose row names another session's file is reported"
        );

        // Nothing was removed, and nothing was even tried: `removed` logs the attempt,
        // not the success, so an empty log is the strong claim here.
        assert!(seam.removed().is_empty());
        let calls = index.calls();
        assert!(
            !calls.iter().any(|call| call.starts_with("mark_removed:")),
            "a file that was never removed is not a removed row: {calls:?}"
        );
        assert_eq!(
            index.snapshot(SESSION_A),
            Some(crossed),
            "the crossed row is left exactly as it was found"
        );
        assert_eq!(
            index.snapshot(SESSION_B),
            Some(open_row),
            "and the writing row is untouched"
        );
        assert!(owned.is_file(), "the candidate's own file is still there");
        assert!(
            !archive_dir.join(name_of(SESSION_D)).exists(),
            "and the file its row named was never touched either"
        );
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(400));

        // The writing session is not the one at fault: still open, still empty, and its
        // record is still the queue's to write.
        assert!(writer.is_open(SESSION_B));
        assert_eq!(
            fs::read(archive_dir.join(name_of(SESSION_B))).expect("the new archive"),
            Vec::<u8>::new(),
            "the record was refused room, not half-written"
        );
        {
            let queue = writer.queue.lock().expect("the queue");
            assert_eq!(queue.len(), 1, "the record did not vanish with the Err");
            assert_eq!(queue.queued_bytes(), big.len());
            assert_eq!(
                queue.dropped(SESSION_B),
                DropCounters::default(),
                "a failure the session did not cause is not its loss"
            );
        }

        // Repairing the row is all it takes.
        index.seed(seeded_row(SESSION_A, "complete", None, 400));
        writer.pump(10_004).expect("the pump recovers");
        assert_eq!(
            seam.removed(),
            vec![owned.clone()],
            "one eviction, and only once the row was right"
        );
        assert!(!owned.exists());
        assert_eq!(
            index.state_of(SESSION_A),
            Some(("removed".into(), Some("quota-evicted".into())))
        );
        assert_eq!(
            fs::read_to_string(archive_dir.join(name_of(SESSION_B))).expect("the new archive"),
            format!("{}\n", encode_record(&record(SESSION_B, &big, 10_002))),
            "written exactly once across the two pumps"
        );
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(332));
        assert!(writer.queue.lock().expect("the queue").is_empty());
        assert_eq!(entry_count(&archive_dir), 1);
    }

    #[test]
    fn when_nothing_can_be_evicted_the_open_session_closes_partial_quota_exceeded() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            QuotaLimits {
                session_bytes: 4_096,
                total_bytes: 200,
            },
        )
        .expect("an initialized writer");

        writer.begin(SESSION_A, 10_001).expect("the session opens");
        writer.enqueue(record(SESSION_A, &"v".repeat(400), 10_002));
        writer.pump(10_003).expect("the pump survives a full quota");

        // Nothing has ended, so nothing can be evicted, and the archive stops
        // instead of growing past the cap.
        assert_eq!(
            index.state_of(SESSION_A),
            Some(("partial".into(), Some("quota-exceeded".into())))
        );
        assert!(!writer.is_open(SESSION_A));
        assert!(matches!(
            writer.total_bytes(),
            QuotaTotal::Known(bytes) if bytes <= 200
        ));
        assert!(seam.removed().is_empty());
    }

    /// A total the sweep could not work out is treated as no room, not as zero.
    /// Guessing low would let the directory grow past a cap this build cannot
    /// check.
    #[test]
    fn an_unavailable_total_stops_the_archive_instead_of_growing_it() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        // An entry with an archive-shaped name, no row, and no readable metadata:
        // nothing on disk or in the index can say how big it is.
        fs::write(archive_dir.join(name_of(SESSION_B)), "z".repeat(20))
            .expect("an unreadable entry");

        let index = TestIndex::shared();
        let seam = TestFs::shared();
        seam.fail_metadata_for(&name_of(SESSION_B));
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an unmeasurable directory still initializes");
        assert_eq!(report.measured_bytes, QuotaTotal::Unavailable);

        writer.begin(SESSION_A, 10_001).expect("the session opens");
        writer.enqueue(record(SESSION_A, "kept", 10_002));
        writer
            .pump(10_003)
            .expect("the pump survives an unmeasurable total");

        // The caps themselves are roomy, so only the unknown total can have
        // stopped this session.
        assert_eq!(
            index.state_of(SESSION_A),
            Some(("partial".into(), Some("quota-exceeded".into())))
        );
        assert!(!writer.is_open(SESSION_A));
        assert_eq!(writer.total_bytes(), QuotaTotal::Unavailable);
        assert!(seam.removed().is_empty());
    }

    // Group 5: the startup sweep.

    #[test]
    fn the_sweep_repairs_a_writing_row_and_marks_a_row_whose_file_is_gone() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");

        // A session the last run was still writing when it stopped.
        let interrupted = "{\"t\":1,\"s\":\"stdout\",\"l\":\"kept\"}\n";
        fs::write(archive_dir.join(name_of(SESSION_A)), interrupted).expect("an open archive");

        let index = TestIndex::shared();
        index.seed(seeded_row(SESSION_A, "writing", None, 0));
        // A row whose file a user or a cleaner removed behind RunCove's back.
        index.seed(seeded_row(SESSION_B, "complete", None, 4_096));

        let seam = TestFs::shared();
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        assert_eq!(report.repaired_writing, vec![SESSION_A.to_string()]);
        assert_eq!(report.marked_file_missing, vec![SESSION_B.to_string()]);

        // The interrupted archive is closed with what the file actually holds,
        // not with what the stale row claimed.
        let row = index.snapshot(SESSION_A).expect("the repaired row");
        assert_eq!(
            (row.status.as_str(), row.reason.as_deref()),
            ("partial", Some("interrupted"))
        );
        assert_eq!(row.counters.byte_size, interrupted.len() as i64);
        assert_eq!(row.counters.line_count, 1);
        assert!(row.ended_at.is_some());
        assert!(archive_dir.join(name_of(SESSION_A)).is_file());

        let missing = index.snapshot(SESSION_B).expect("the missing row");
        assert_eq!(
            (missing.status.as_str(), missing.reason.as_deref()),
            ("removed", Some("file-missing"))
        );

        // A row with no file contributes no bytes to the quota.
        assert_eq!(
            report.measured_bytes,
            QuotaTotal::Known(interrupted.len() as u64)
        );
        assert_eq!(
            writer.total_bytes(),
            QuotaTotal::Known(interrupted.len() as u64)
        );
        assert!(seam.removed().is_empty());
        assert!(report.anomalies.is_empty());
    }

    #[test]
    fn the_sweep_deletes_an_eligible_orphan_and_measures_the_quota_total() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        fs::write(archive_dir.join(name_of(SESSION_A)), "z".repeat(30)).expect("a kept archive");
        // An eligible file with no row: this build generated it, and the row it
        // belonged to is gone.
        let orphan = archive_dir.join(name_of(SESSION_B));
        fs::write(&orphan, "z".repeat(20)).expect("an orphan archive");

        let index = TestIndex::shared();
        index.seed(seeded_row(SESSION_A, "complete", None, 30));
        let seam = TestFs::shared();
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        assert_eq!(report.deleted_orphan_files, vec![name_of(SESSION_B)]);
        assert!(!orphan.exists());
        assert_eq!(seam.removed(), vec![orphan]);

        // The quota starts from what survived, not from what was there.
        assert_eq!(report.measured_bytes, QuotaTotal::Known(30));
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(30));
        assert!(archive_dir.join(name_of(SESSION_A)).is_file());
    }

    #[test]
    fn the_sweep_leaves_what_it_does_not_recognize_and_reports_it() {
        let (temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");

        // Four things the sweep must report and not touch: a file RunCove never
        // wrote, a name this build could not have generated, a row pointing
        // outside the directory, and a status only a newer build knows.
        let foreign = archive_dir.join("notes.txt");
        fs::write(&foreign, b"mine").expect("a foreign file");
        let shouting = archive_dir.join(name_of(&SESSION_A.to_uppercase()));
        fs::write(&shouting, b"not ours").expect("a name with the wrong case");
        let outside = temp.path().join("escape.jsonl");
        fs::write(&outside, b"outside").expect("a file outside the directory");

        let index = TestIndex::shared();
        let mut escaping = seeded_row(SESSION_B, "complete", None, 7);
        escaping.file_name = "..\\escape.jsonl".into();
        index.seed(escaping);
        index.seed(seeded_row(SESSION_C, "archived", Some("cold-storage"), 11));

        let seam = TestFs::shared();
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("a bad entry does not stop the sweep");

        assert_eq!(report.anomalies.len(), 4, "{:?}", report.anomalies);
        assert!(report.deleted_orphan_files.is_empty());
        assert!(report.marked_file_missing.is_empty());
        assert!(seam.removed().is_empty());

        // Nothing was repaired by guessing and nothing was deleted.
        assert_eq!(fs::read(&foreign).expect("the foreign file"), b"mine");
        assert_eq!(fs::read(&shouting).expect("the shouting file"), b"not ours");
        assert_eq!(fs::read(&outside).expect("the outside file"), b"outside");
        assert_eq!(index.state_of(SESSION_B), Some(("complete".into(), None)));
        assert_eq!(
            index.state_of(SESSION_C),
            Some(("archived".into(), Some("cold-storage".into())))
        );

        // Nothing here is an archive of ours, so nothing here is the quota's: a row
        // whose name this build refuses to resolve has no entry to measure, which is
        // not the same case as an entry whose metadata could not be read.
        assert_eq!(report.measured_bytes, QuotaTotal::Known(0));
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(0));
    }

    /// A reparse point standing where an archive should be is reported, not
    /// followed. The target holds three archive-shaped lines, so a sweep that
    /// followed the link could not leave every count at zero.
    #[cfg(windows)]
    #[test]
    fn the_sweep_neither_reads_counts_nor_deletes_a_reparse_point() {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        let (temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");

        let secret = concat!(
            "{\"t\":1,\"s\":\"stdout\",\"l\":\"secret one\"}\n",
            "{\"t\":2,\"s\":\"stdout\",\"l\":\"secret two\"}\n",
            "{\"t\":3,\"s\":\"stdout\",\"l\":\"secret three\"}\n"
        );
        let target = temp.path().join("secret.jsonl");
        fs::write(&target, secret).expect("a file outside the archive directory");
        let dir_target = temp.path().join("elsewhere");
        fs::create_dir_all(&dir_target).expect("a directory outside the archive directory");

        let expectation = "creating a symbolic link needs Developer Mode or an elevated shell";
        // A link where an interrupted session's archive should be.
        let linked = archive_dir.join(name_of(SESSION_A));
        symlink_file(&target, &linked).expect(expectation);
        // A link with an archive-shaped name and no row, which the orphan rule
        // would delete if it took the entry for an ordinary file.
        let orphan_link = archive_dir.join(name_of(SESSION_B));
        symlink_file(&target, &orphan_link).expect(expectation);
        // The junction case: a directory link with an archive-shaped name.
        let linked_dir = archive_dir.join(name_of(SESSION_C));
        symlink_dir(&dir_target, &linked_dir).expect(expectation);

        let index = TestIndex::shared();
        // The row remembers 777 bytes. A reparse point is not the file this build
        // wrote, so none of those bytes are at that name any more, which is what
        // separates this case from an entry whose metadata merely could not be read.
        index.seed(seeded_row(SESSION_A, "writing", None, 777));

        let seam = TestFs::shared();
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("a reparse point does not stop the sweep");

        // Not read: the stuck row is repaired from the fact that the last run was
        // interrupted, never from anything behind the link.
        assert_eq!(report.repaired_writing, vec![SESSION_A.to_string()]);
        let row = index.snapshot(SESSION_A).expect("the repaired row");
        assert_eq!(
            (row.status.as_str(), row.reason.as_deref()),
            ("partial", Some("interrupted"))
        );
        assert_eq!(row.counters.byte_size, 0);
        assert_eq!(row.counters.line_count, 0);
        assert!(row.ended_at.is_some());

        // Not counted towards the quota.
        assert_eq!(report.measured_bytes, QuotaTotal::Known(0));
        assert_eq!(writer.total_bytes(), QuotaTotal::Known(0));

        // Not deleted, and each entry is still the link it was.
        assert!(report.deleted_orphan_files.is_empty());
        assert!(report.marked_file_missing.is_empty());
        assert!(seam.removed().is_empty());
        for link in [&linked, &orphan_link, &linked_dir] {
            assert!(
                fs::symlink_metadata(link)
                    .expect("the link")
                    .file_type()
                    .is_symlink(),
                "{link:?} is no longer a link"
            );
        }
        assert_eq!(fs::read_to_string(&target).expect("the target"), secret);
        assert!(dir_target.is_dir());

        // All three are reported, so nothing was silently skipped.
        assert_eq!(report.anomalies.len(), 3, "{:?}", report.anomalies);
    }

    /// An entry the filesystem refuses to describe is reported, left alone, and
    /// charged to the quota at the only size anyone still knows: the row's.
    #[test]
    fn the_sweep_uses_the_last_known_byte_size_when_it_cannot_measure_an_entry() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        // Forty bytes on the disk, twenty-five in the row. The two differ so that
        // only the row's number can satisfy the assertion below.
        fs::write(archive_dir.join(name_of(SESSION_A)), "z".repeat(40)).expect("an archive");
        let interrupted = "{\"t\":1,\"s\":\"stdout\",\"l\":\"kept\"}\n";
        fs::write(archive_dir.join(name_of(SESSION_C)), interrupted).expect("an open archive");

        let index = TestIndex::shared();
        index.seed(seeded_row(SESSION_A, "complete", None, 25));
        index.seed(seeded_row(SESSION_C, "writing", None, 0));

        let seam = TestFs::shared();
        seam.fail_metadata_for(&name_of(SESSION_A));
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an unreadable entry does not stop the sweep");

        // Reported once, and otherwise untouched: an entry that could not be read
        // is not an entry known to be gone.
        assert_eq!(report.anomalies.len(), 1, "{:?}", report.anomalies);
        assert!(report.marked_file_missing.is_empty());
        assert!(report.deleted_orphan_files.is_empty());
        assert!(seam.removed().is_empty());
        assert_eq!(index.state_of(SESSION_A), Some(("complete".into(), None)));
        assert_eq!(
            index
                .snapshot(SESSION_A)
                .expect("the row")
                .counters
                .byte_size,
            25
        );
        assert!(archive_dir.join(name_of(SESSION_A)).is_file());

        // The quota falls back to the row, and the rest of the sweep still ran.
        let expected = QuotaTotal::Known(25 + interrupted.len() as u64);
        assert_eq!(report.measured_bytes, expected);
        assert_eq!(writer.total_bytes(), expected);
        assert_eq!(report.repaired_writing, vec![SESSION_C.to_string()]);
    }

    /// A delete the filesystem refuses does not become a delete that happened.
    /// The bytes are still there, so the quota still owes them.
    #[test]
    fn an_orphan_that_could_not_be_deleted_still_counts_towards_the_quota() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        fs::write(archive_dir.join(name_of(SESSION_A)), "z".repeat(30)).expect("a kept archive");
        let orphan = archive_dir.join(name_of(SESSION_B));
        fs::write(&orphan, "z".repeat(20)).expect("an orphan archive");
        let interrupted = "{\"t\":1,\"s\":\"stdout\",\"l\":\"kept\"}\n";
        fs::write(archive_dir.join(name_of(SESSION_C)), interrupted).expect("an open archive");

        let index = TestIndex::shared();
        index.seed(seeded_row(SESSION_A, "complete", None, 30));
        index.seed(seeded_row(SESSION_C, "writing", None, 0));

        let seam = TestFs::shared();
        seam.fail_remove();
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("a refused delete does not stop the sweep");

        // Tried, refused, reported, and not claimed as deleted.
        assert_eq!(seam.removed(), vec![orphan.clone()]);
        assert!(report.deleted_orphan_files.is_empty());
        assert_eq!(report.anomalies.len(), 1, "{:?}", report.anomalies);
        assert!(orphan.is_file());

        let expected = QuotaTotal::Known(30 + 20 + interrupted.len() as u64);
        assert_eq!(report.measured_bytes, expected);
        assert_eq!(writer.total_bytes(), expected);
        assert_eq!(report.repaired_writing, vec![SESSION_C.to_string()]);
    }

    /// When no one knows an entry's size, the sweep says so instead of guessing
    /// low. An entry it cannot even classify is also never taken for an orphan.
    #[test]
    fn an_unmeasurable_entry_with_no_row_makes_the_total_unavailable() {
        let (_temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        fs::write(archive_dir.join(name_of(SESSION_A)), "z".repeat(30)).expect("a kept archive");
        // No row remembers this one, so nothing supplies a size to fall back on.
        let unreadable = archive_dir.join(name_of(SESSION_B));
        fs::write(&unreadable, "z".repeat(20)).expect("an unreadable entry");
        let interrupted = "{\"t\":1,\"s\":\"stdout\",\"l\":\"kept\"}\n";
        fs::write(archive_dir.join(name_of(SESSION_C)), interrupted).expect("an open archive");

        let index = TestIndex::shared();
        index.seed(seeded_row(SESSION_A, "complete", None, 30));
        index.seed(seeded_row(SESSION_C, "writing", None, 0));

        let seam = TestFs::shared();
        seam.fail_metadata_for(&name_of(SESSION_B));
        let (writer, report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an unmeasurable entry does not stop the sweep");

        assert_eq!(report.measured_bytes, QuotaTotal::Unavailable);
        assert_eq!(writer.total_bytes(), QuotaTotal::Unavailable);

        // Not an ordinary file as far as this build knows, so not an orphan either.
        assert!(report.deleted_orphan_files.is_empty());
        assert!(seam.removed().is_empty());
        assert!(unreadable.is_file());
        assert_eq!(report.anomalies.len(), 1, "{:?}", report.anomalies);

        // An unknown total costs the total and nothing else the sweep owed.
        assert_eq!(report.repaired_writing, vec![SESSION_C.to_string()]);
        assert_eq!(index.state_of(SESSION_A), Some(("complete".into(), None)));
    }

    /// The seam's own fidelity, which the sweep test above depends on: if the
    /// double followed links, that test would pass for the wrong reason. This one
    /// exercises only [`TestFs`], so it is green while the archive is still
    /// unimplemented.
    #[cfg(windows)]
    #[test]
    fn the_test_filesystem_reports_a_link_as_a_reparse_point() {
        let (temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        let target = temp.path().join("target.jsonl");
        fs::write(&target, "z".repeat(64)).expect("a file outside the archive directory");
        let linked = archive_dir.join(name_of(SESSION_A));
        std::os::windows::fs::symlink_file(&target, &linked)
            .expect("creating a symbolic link needs Developer Mode or an elevated shell");

        let seam = TestFs::shared();
        let listed = seam.list_dir(&archive_dir).expect("the directory listing");
        let stated = seam.entry_info(&linked).expect("the entry");

        let expected: Vec<ListedEntry> = vec![Ok(stated.clone())];
        assert_eq!(listed, expected, "listing disagrees with stating");
        assert_eq!(stated.kind, EntryKind::ReparsePoint);
        assert_eq!(stated.name, name_of(SESSION_A));
        assert_ne!(
            stated.len, 64,
            "the target's length leaked through the link"
        );
    }

    /// The shipped filesystem's own classification, on the same tree, compared
    /// against the double's. Every sweep test drives [`TestFs`]; without this one
    /// they would all be evidence about the double and none about what ships.
    ///
    /// A non-name-surrogate reparse point — a cloud placeholder, a deduplicated
    /// file, an `AppExecLink` — cannot be created through `std`, so the tree covers
    /// the two tags `std` can make. The rest are covered by the attribute-bit rule
    /// that [`ArchiveFs::entry_info`] states and both implementations follow, which
    /// is the same rule this test proves they agree on here.
    #[cfg(windows)]
    #[test]
    fn the_real_filesystem_classifies_entries_exactly_as_the_double_does() {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        let (temp, archive_dir) = temp_data_dir();
        fs::create_dir_all(&archive_dir).expect("archive directory");
        let file_target = temp.path().join("target.jsonl");
        fs::write(&file_target, "z".repeat(64)).expect("a file outside the directory");
        let dir_target = temp.path().join("target-dir");
        fs::create_dir_all(&dir_target).expect("a directory outside the directory");

        let expectation = "creating a symbolic link needs Developer Mode or an elevated shell";
        let ordinary = archive_dir.join(name_of(SESSION_A));
        fs::write(&ordinary, "z".repeat(30)).expect("an ordinary archive");
        let nested = archive_dir.join("nested");
        fs::create_dir_all(&nested).expect("a directory inside the archive directory");
        let file_link = archive_dir.join(name_of(SESSION_B));
        symlink_file(&file_target, &file_link).expect(expectation);
        let dir_link = archive_dir.join(name_of(SESSION_C));
        symlink_dir(&dir_target, &dir_link).expect(expectation);

        let real = RealArchiveFs;
        let double = TestFs::shared();

        // The whole listing, so a kind added later is covered without this test
        // naming it, and so the order both promise is part of the agreement.
        assert_eq!(
            real.list_dir(&archive_dir).expect("the shipped listing"),
            double.list_dir(&archive_dir).expect("the double's listing"),
        );

        // And the classification itself, so agreeing on the wrong answer still
        // fails.
        for (path, expected) in [
            (&ordinary, EntryKind::File),
            (&nested, EntryKind::Directory),
            (&file_link, EntryKind::ReparsePoint),
            (&dir_link, EntryKind::ReparsePoint),
        ] {
            let shipped = real.entry_info(path).expect("the shipped entry");
            assert_eq!(
                shipped,
                double.entry_info(path).expect("the double's entry"),
                "{path:?}"
            );
            assert_eq!(shipped.kind, expected, "{path:?}");
        }

        // A link's own length, never its target's.
        assert_ne!(
            real.entry_info(&file_link).expect("the link").len,
            64,
            "the target's length leaked through the link"
        );
    }

    // Group 6: the read path and its cursor.

    /// The bytes the writer would have produced for these `stdout` lines, one
    /// record each, every one terminated.
    fn encoded_lines(session_id: &str, lines: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let record = record(session_id, line, 10_000 + index as i64);
            bytes.extend_from_slice(encode_record(&record).as_bytes());
            bytes.push(b'\n');
        }
        bytes
    }

    /// A writer over an archive directory holding one file with exactly these
    /// bytes, and a `complete` row that names it.
    ///
    /// The reader's tests build the file instead of driving the writer, so a page
    /// can be asked about bytes the writer would never produce — a torn tail, a
    /// record from another build — and a failure names the reader.
    fn writer_over_bytes(
        archive_dir: &Path,
        seam: Arc<TestFs>,
        index: Arc<TestIndex>,
        session_id: &str,
        contents: &[u8],
    ) -> ArchiveWriter {
        fs::create_dir_all(archive_dir).expect("archive directory");
        fs::write(archive_dir.join(name_of(session_id)), contents).expect("an archive file");
        index.seed(seeded_row(
            session_id,
            "complete",
            None,
            contents.len() as i64,
        ));
        let (writer, _report) =
            test_writer(archive_dir, seam, index, roomy_bounds(), roomy_limits())
                .expect("an initialized writer");
        writer
    }

    /// The text of each record a page carries, in the order it carries them.
    fn page_lines(page: &RunLogArchivePage) -> Vec<String> {
        page.records.iter().map(|item| item.line.clone()).collect()
    }

    #[test]
    fn a_page_reads_back_what_the_writer_wrote_newest_last() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");
        writer.begin(SESSION_A, 10_000).expect("an open archive");
        for (offset, line) in ["first", "second", "third"].iter().enumerate() {
            writer.enqueue(record(SESSION_A, line, 10_001 + offset as i64));
        }
        writer.pump(10_010).expect("the batch reaches the disk");
        writer
            .close(SESSION_A, None, 10_020)
            .expect("a clean close");

        // The tail, and only the tail: two of the three records, oldest of the two
        // first, so the drawer can append a page above what it already shows.
        let tail = writer
            .read_page(SESSION_A, None, Some(2))
            .expect("the tail of the archive");
        assert_eq!(page_lines(&tail), vec!["second", "third"]);
        assert_eq!(tail.records[0].timestamp, 10_002);
        assert!(matches!(tail.records[0].stream, LogStream::Stdout));
        assert_eq!(tail.stopped_by, "lines");
        assert!(tail.has_more_before);
        assert_eq!(tail.session_id, SESSION_A);
        assert_eq!(tail.status, status_text(ArchiveStatus::Complete));
        assert_eq!(tail.reason, None);
        assert_eq!(tail.line_count, 3);
        assert_eq!(tail.dropped_lines, 0);
        assert_eq!(tail.dropped_bytes, 0);
        assert_eq!(tail.malformed_lines, 0);
        assert!(!tail.incomplete_tail_skipped);
        assert_eq!(
            tail.file_length,
            writer.read(SESSION_A).expect("the whole archive").len() as u64,
            "the page measured a different file than the whole-file read"
        );
        assert_eq!(tail.byte_size, tail.file_length as i64);
        assert_eq!(tail.started_at, 10_000);
        assert_eq!(tail.ended_at, Some(10_020));

        // Paging back is feeding `page_start_offset` straight back in, and the
        // first record is reached when — and only when — the scan ran out of file.
        let older = writer
            .read_page(SESSION_A, Some(tail.page_start_offset), Some(2))
            .expect("the page before the tail");
        assert_eq!(page_lines(&older), vec!["first"]);
        assert_eq!(older.stopped_by, "start");
        assert!(!older.has_more_before);
        assert_eq!(older.page_start_offset, 0);
        assert_eq!(older.file_length, tail.file_length);
        assert!(!older.incomplete_tail_skipped);
    }

    #[test]
    fn every_page_of_an_archive_walks_it_once_from_the_end() {
        let (_temp, archive_dir) = temp_data_dir();
        let lines: Vec<String> = (0..7).map(|index| format!("line-{index}")).collect();
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        let writer = writer_over_bytes(
            &archive_dir,
            TestFs::shared(),
            TestIndex::shared(),
            SESSION_A,
            &encoded_lines(SESSION_A, &borrowed),
        );

        let mut walked: Vec<String> = Vec::new();
        let mut cursor = None;
        let mut pages = 0;
        loop {
            let page = writer
                .read_page(SESSION_A, cursor, Some(3))
                .expect("a page of the archive");
            pages += 1;
            assert!(pages <= 8, "paging back did not terminate");
            // The invariant the drawer's "load older" button rests on.
            assert_eq!(
                page.has_more_before,
                page.stopped_by != "start",
                "has_more_before and stopped_by disagreed"
            );
            let mut older = page_lines(&page);
            older.append(&mut walked);
            walked = older;
            if !page.has_more_before {
                break;
            }
            cursor = Some(page.page_start_offset);
        }

        assert_eq!(walked, lines, "the walk did not reproduce the archive");
        assert_eq!(pages, 3, "7 records at 3 a page is three pages");
    }

    #[test]
    fn a_cursor_that_is_not_a_record_boundary_is_refused_rather_than_resynced() {
        let (_temp, archive_dir) = temp_data_dir();
        let contents = encoded_lines(SESSION_A, &["alpha", "beta"]);
        let length = contents.len() as u64;
        let writer = writer_over_bytes(
            &archive_dir,
            TestFs::shared(),
            TestIndex::shared(),
            SESSION_A,
            &contents,
        );

        // Zero is not "the beginning": no record lies wholly below it, and a caller
        // asking for one has lost track of where it was.
        let refused = writer
            .read_page(SESSION_A, Some(0), None)
            .expect_err("offset zero is not a page");
        assert!(
            refused.to_string().contains(&length.to_string()),
            "the refusal did not name the file's length: {refused}"
        );

        // Past the end is the same mistake in the other direction.
        let beyond = writer
            .read_page(SESSION_A, Some(length + 1), None)
            .expect_err("an offset past the end is not a page");
        assert!(
            beyond.to_string().contains(&length.to_string()),
            "the refusal did not name the file's length: {beyond}"
        );

        // Inside a record, where a silent resync would quietly serve a page the
        // caller did not ask for.
        let inside = writer
            .read_page(SESSION_A, Some(length - 2), None)
            .expect_err("an offset inside a record is not a page");
        assert!(
            inside.to_string().contains("record boundary"),
            "the refusal did not say why: {inside}"
        );

        // The one interior offset that is a boundary: exactly one newline back.
        let first_end = contents
            .iter()
            .position(|byte| *byte == b'\n')
            .expect("a terminated record") as u64
            + 1;
        let page = writer
            .read_page(SESSION_A, Some(first_end), None)
            .expect("a record boundary is a page");
        assert_eq!(page_lines(&page), vec!["alpha"]);
        assert_eq!(page.file_length, length);
    }

    #[test]
    fn a_page_size_is_clamped_at_both_ends_and_never_to_nothing() {
        let (_temp, archive_dir) = temp_data_dir();
        // Enough records that the greedy request has to stop at the cap, and enough
        // bytes that the scan crosses more than one backward block.
        let lines: Vec<String> = (0..MAX_PAGE_RECORDS + 100)
            .map(|index| format!("record-{index:07}"))
            .collect();
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        let contents = encoded_lines(SESSION_A, &borrowed);
        assert!(
            contents.len() > READ_BLOCK_BYTES,
            "the fixture must outgrow a single backward block"
        );
        let writer = writer_over_bytes(
            &archive_dir,
            TestFs::shared(),
            TestIndex::shared(),
            SESSION_A,
            &contents,
        );

        // Zero would be a page nobody can page past, so one is the floor.
        let single = writer
            .read_page(SESSION_A, None, Some(0))
            .expect("a page of one");
        assert_eq!(single.records.len(), 1);
        assert_eq!(single.stopped_by, "lines");
        assert_eq!(
            single.records[0].line,
            *lines.last().expect("a seeded record")
        );

        // A greedy request stops at the cap rather than at the file.
        let greedy = writer
            .read_page(SESSION_A, None, Some(usize::MAX))
            .expect("a capped page");
        assert_eq!(greedy.records.len(), MAX_PAGE_RECORDS);
        assert_eq!(greedy.stopped_by, "lines");
        assert!(greedy.has_more_before);
        assert_eq!(
            page_lines(&greedy),
            lines[100..].to_vec(),
            "the cap took the wrong end of the file"
        );

        // No request at all is the default, which is neither of the bounds.
        let default = writer.read_page(SESSION_A, None, None).expect("a page");
        assert_eq!(default.records.len(), DEFAULT_PAGE_RECORDS);
    }

    #[test]
    fn a_record_that_will_not_decode_is_skipped_counted_and_still_paid_for() {
        let (_temp, archive_dir) = temp_data_dir();
        let mut contents = encoded_lines(SESSION_A, &["good-one"]);
        contents.extend_from_slice(b"not json at all\n");
        // Not text either: a decode that went through `str` first would have to
        // decide what to do about this, and going straight from bytes does not.
        contents.extend_from_slice(&[0xff, 0xfe, b'\n']);
        contents.extend_from_slice(&encoded_lines(SESSION_A, &["good-two"]));
        let writer = writer_over_bytes(
            &archive_dir,
            TestFs::shared(),
            TestIndex::shared(),
            SESSION_A,
            &contents,
        );

        let whole = writer
            .read_page(SESSION_A, None, Some(10))
            .expect("a page over the damage");
        assert_eq!(page_lines(&whole), vec!["good-one", "good-two"]);
        assert_eq!(whole.malformed_lines, 2);
        assert_eq!(whole.stopped_by, "start");
        assert!(!whole.has_more_before);
        assert!(!whole.incomplete_tail_skipped);

        // The page size bounds the scan's work, not the records it hands back:
        // two lines were consumed and one of them was unreadable.
        let bounded = writer
            .read_page(SESSION_A, None, Some(2))
            .expect("a bounded page");
        assert_eq!(page_lines(&bounded), vec!["good-two"]);
        assert_eq!(bounded.malformed_lines, 1);
        assert_eq!(bounded.stopped_by, "lines");
        assert!(bounded.has_more_before);

        // And the walk still terminates: the offset it reports is the start of the
        // unreadable line, not of the record it managed to decode.
        let older = writer
            .read_page(SESSION_A, Some(bounded.page_start_offset), Some(10))
            .expect("the page before it");
        assert_eq!(page_lines(&older), vec!["good-one"]);
        assert_eq!(older.malformed_lines, 1);
        assert!(!older.has_more_before);
    }

    #[test]
    fn bytes_after_the_last_newline_are_not_a_record_and_say_so() {
        let (_temp, archive_dir) = temp_data_dir();
        let mut contents = encoded_lines(SESSION_A, &["good-one", "good-two"]);
        let complete = contents.len() as u64;
        // What a run killed between a write and its newline leaves behind.
        contents.extend_from_slice(br#"{"t":1,"s":"stdout","l":"half a rec"#);
        let writer = writer_over_bytes(
            &archive_dir,
            TestFs::shared(),
            TestIndex::shared(),
            SESSION_A,
            &contents,
        );

        let page = writer
            .read_page(SESSION_A, None, Some(10))
            .expect("a page over the torn tail");
        assert_eq!(page_lines(&page), vec!["good-one", "good-two"]);
        assert!(page.incomplete_tail_skipped);
        assert_eq!(page.malformed_lines, 0, "a fragment is not a bad record");
        assert_eq!(page.stopped_by, "start");
        assert!(!page.has_more_before);
        assert_eq!(page.file_length, contents.len() as u64);

        // The fragment's own start is a boundary the caller can name, and asking for
        // the page below it is the same page.
        let again = writer
            .read_page(SESSION_A, Some(complete), Some(10))
            .expect("a page below the fragment");
        assert_eq!(page_lines(&again), page_lines(&page));
        assert!(
            !again.incomplete_tail_skipped,
            "nothing was skipped when the cursor already excluded it"
        );
    }

    #[test]
    fn a_file_with_no_record_boundary_at_all_costs_one_page_and_yields_nothing() {
        let (_temp, archive_dir) = temp_data_dir();
        // Larger than one backward block, so a scan that kept what it inspected
        // would be holding all of it.
        let contents = vec![b'x'; READ_BLOCK_BYTES * 3];
        let writer = writer_over_bytes(
            &archive_dir,
            TestFs::shared(),
            TestIndex::shared(),
            SESSION_A,
            &contents,
        );

        let page = writer
            .read_page(SESSION_A, None, Some(10))
            .expect("a page of nothing");
        assert!(page.records.is_empty());
        assert!(page.incomplete_tail_skipped);
        assert_eq!(page.malformed_lines, 0);
        assert_eq!(page.page_start_offset, 0);
        assert_eq!(page.stopped_by, "start");
        assert!(!page.has_more_before);
        assert_eq!(page.file_length, contents.len() as u64);
    }

    #[test]
    fn a_page_stops_at_its_byte_cap_before_it_runs_out_of_records() {
        let (_temp, archive_dir) = temp_data_dir();
        let bulk = "b".repeat(400 * 1024);
        let lines: Vec<String> = (0..3).map(|index| format!("{index}-{bulk}")).collect();
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        let writer = writer_over_bytes(
            &archive_dir,
            TestFs::shared(),
            TestIndex::shared(),
            SESSION_A,
            &encoded_lines(SESSION_A, &borrowed),
        );

        // Two of the three fit under a megabyte of text; the third would not, and
        // the page size never came into it.
        let page = writer
            .read_page(SESSION_A, None, Some(MAX_PAGE_RECORDS))
            .expect("a page of bulk");
        assert_eq!(page_lines(&page), lines[1..].to_vec());
        assert_eq!(page.stopped_by, "bytes");
        assert!(page.has_more_before);

        let older = writer
            .read_page(
                SESSION_A,
                Some(page.page_start_offset),
                Some(MAX_PAGE_RECORDS),
            )
            .expect("the page before it");
        assert_eq!(page_lines(&older), lines[..1].to_vec());
        assert_eq!(older.stopped_by, "start");
    }

    #[test]
    fn a_record_larger_than_the_whole_byte_cap_is_still_delivered() {
        let (_temp, archive_dir) = temp_data_dir();
        // A page that refused this record would leave everything older than it
        // unreachable for good, so the cap yields to the first record of a page.
        let huge = "h".repeat(PAGE_BYTE_CAP + 1);
        let writer = writer_over_bytes(
            &archive_dir,
            TestFs::shared(),
            TestIndex::shared(),
            SESSION_A,
            &encoded_lines(SESSION_A, &["tiny", huge.as_str()]),
        );

        let page = writer
            .read_page(SESSION_A, None, None)
            .expect("a page of one very long record");
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].line.len(), huge.len());
        assert_eq!(page.stopped_by, "bytes");
        assert!(page.has_more_before);

        let older = writer
            .read_page(SESSION_A, Some(page.page_start_offset), None)
            .expect("the page before it");
        assert_eq!(page_lines(&older), vec!["tiny"]);
        assert_eq!(older.stopped_by, "start");
        assert!(!older.has_more_before);
    }

    #[test]
    fn an_offset_stays_a_boundary_while_the_session_keeps_writing() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            Arc::clone(&seam),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");
        writer.begin(SESSION_A, 10_000).expect("an open archive");
        for line in ["first", "second", "third"] {
            writer.enqueue(record(SESSION_A, line, 10_001));
        }
        writer
            .pump(10_002)
            .expect("the first batch reaches the disk");

        // A live archive is readable, and says it is live.
        let tail = writer
            .read_page(SESSION_A, None, Some(2))
            .expect("the tail of a live archive");
        assert_eq!(page_lines(&tail), vec!["second", "third"]);
        assert_eq!(tail.status, status_text(ArchiveStatus::Writing));
        assert_eq!(tail.reason, None);
        assert_eq!(tail.ended_at, None);
        assert_eq!(tail.line_count, 3);
        let boundary = tail.page_start_offset;

        for line in ["fourth", "fifth"] {
            writer.enqueue(record(SESSION_A, line, 10_003));
        }
        writer
            .pump(10_004)
            .expect("the second batch reaches the disk");

        // Bytes below a measured length never change, so the offset a page reported
        // before the append still names the same records after it.
        let older = writer
            .read_page(SESSION_A, Some(boundary), Some(2))
            .expect("the page before the boundary");
        assert_eq!(page_lines(&older), vec!["first"]);
        assert!(!older.has_more_before);
        assert!(older.file_length > tail.file_length);

        let newer = writer
            .read_page(SESSION_A, None, Some(2))
            .expect("the new tail");
        assert_eq!(page_lines(&newer), vec!["fourth", "fifth"]);
        assert_eq!(newer.line_count, 5);
        writer
            .close(SESSION_A, None, 10_005)
            .expect("a clean close");
    }

    #[test]
    fn a_page_of_an_archive_that_is_gone_says_why_it_is_gone() {
        let (_temp, archive_dir) = temp_data_dir();
        let index = TestIndex::shared();
        index.seed(seeded_row(
            SESSION_A,
            status_text(ArchiveStatus::Removed),
            Some(reason_text(ArchiveReason::QuotaEvicted)),
            4_096,
        ));
        let (writer, _report) = test_writer(
            &archive_dir,
            TestFs::shared(),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        // A viewer left open across an eviction must not read as an empty archive.
        let refused = writer
            .read_page(SESSION_A, None, None)
            .expect_err("a removed archive has no page");
        let message = refused.to_string();
        assert!(
            message.contains(reason_text(ArchiveReason::QuotaEvicted)),
            "the refusal did not carry the row's reason: {message}"
        );
        assert_eq!(entry_count(&archive_dir), 0, "nothing was created to read");
    }

    #[test]
    fn an_archive_that_holds_nothing_pages_to_nothing() {
        let (_temp, archive_dir) = temp_data_dir();
        let writer = writer_over_bytes(
            &archive_dir,
            TestFs::shared(),
            TestIndex::shared(),
            SESSION_A,
            b"",
        );

        let page = writer
            .read_page(SESSION_A, None, None)
            .expect("an empty archive still has a page");
        assert!(page.records.is_empty());
        assert_eq!(page.file_length, 0);
        assert_eq!(page.page_start_offset, 0);
        assert_eq!(page.stopped_by, "start");
        assert!(!page.has_more_before);
        assert!(!page.incomplete_tail_skipped);
        assert_eq!(page.malformed_lines, 0);

        // There is no boundary in an empty file, so no cursor names one.
        writer
            .read_page(SESSION_A, Some(1), None)
            .expect_err("an empty archive has no offsets");
    }

    #[test]
    fn a_read_that_fails_is_reported_rather_than_paged_around() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let writer = writer_over_bytes(
            &archive_dir,
            Arc::clone(&seam),
            TestIndex::shared(),
            SESSION_A,
            &encoded_lines(SESSION_A, &["alpha", "beta"]),
        );
        seam.fail_read_for(&name_of(SESSION_A));

        let refused = writer
            .read_page(SESSION_A, None, None)
            .expect_err("a page of an unreadable file");
        assert!(
            refused.to_string().contains("could not be read"),
            "the failure did not name the read: {refused}"
        );
    }

    #[test]
    fn a_file_that_shrinks_under_the_reader_is_reported_not_served() {
        let (_temp, archive_dir) = temp_data_dir();
        let seam = TestFs::shared();
        let contents = encoded_lines(SESSION_A, &["alpha", "beta"]);
        let writer = writer_over_bytes(
            &archive_dir,
            Arc::clone(&seam),
            TestIndex::shared(),
            SESSION_A,
            &contents,
        );
        // What something outside RunCove truncating the file between the
        // measurement and the read would do. An archive only ever grows, so a
        // short block is that, and never the end of the file.
        seam.read_ends_at(&name_of(SESSION_A), contents.len() as u64 - 1);

        let refused = writer
            .read_page(SESSION_A, None, None)
            .expect_err("a page of a file that moved");
        assert!(
            refused
                .to_string()
                .contains("changed while it was being read"),
            "the failure did not say the file moved: {refused}"
        );
    }

    #[test]
    fn a_page_carries_the_row_the_session_actually_has() {
        let (_temp, archive_dir) = temp_data_dir();
        let index = TestIndex::shared();
        let contents = encoded_lines(SESSION_A, &["one", "two"]);
        fs::create_dir_all(&archive_dir).expect("archive directory");
        fs::write(archive_dir.join(name_of(SESSION_A)), &contents).expect("an archive file");
        index.seed(ArchiveRow {
            session_id: SESSION_A.to_string(),
            file_name: name_of(SESSION_A),
            status: status_text(ArchiveStatus::Partial).to_string(),
            reason: Some(reason_text(ArchiveReason::QuotaExceeded).to_string()),
            counters: ArchiveCounters {
                line_count: 9,
                byte_size: 1_234,
                dropped_lines: 7,
                dropped_bytes: 89,
            },
            started_at: 4_000,
            ended_at: Some(5_000),
        });
        let (writer, _report) = test_writer(
            &archive_dir,
            TestFs::shared(),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        // Everything the viewer needs to say why an archive is short comes from the
        // row, unchanged, alongside the records.
        let page = writer.read_page(SESSION_A, None, None).expect("a page");
        assert_eq!(page.status, status_text(ArchiveStatus::Partial));
        assert_eq!(
            page.reason.as_deref(),
            Some(reason_text(ArchiveReason::QuotaExceeded))
        );
        assert_eq!(page.line_count, 9);
        assert_eq!(page.byte_size, 1_234);
        assert_eq!(page.dropped_lines, 7);
        assert_eq!(page.dropped_bytes, 89);
        assert_eq!(page.started_at, 4_000);
        assert_eq!(page.ended_at, Some(5_000));
        assert_eq!(page_lines(&page), vec!["one", "two"]);
        // The counters are the writer's last refresh; the length is measured now,
        // and the page does not reconcile the two.
        assert_eq!(page.file_length, contents.len() as u64);
        assert_ne!(page.file_length as i64, page.byte_size);
    }

    #[test]
    fn a_page_trusts_the_session_id_and_checks_the_rows_file_name() {
        let (_temp, archive_dir) = temp_data_dir();
        let index = TestIndex::shared();
        let (writer, _report) = test_writer(
            &archive_dir,
            TestFs::shared(),
            Arc::clone(&index),
            roomy_bounds(),
            roomy_limits(),
        )
        .expect("an initialized writer");

        // Both after the sweep, which would otherwise take the unpaired file as an
        // orphan and the unpaired row as an archive whose file is gone.
        fs::write(
            archive_dir.join(name_of(SESSION_B)),
            encoded_lines(SESSION_B, &["not yours"]),
        )
        .expect("another session's archive");
        // A row that names a file belonging to a different session. The name is the
        // row's to supply and this module's to check, so the page is refused rather
        // than served from whatever the row pointed at.
        let mut row = seeded_row(SESSION_A, status_text(ArchiveStatus::Complete), None, 32);
        row.file_name = name_of(SESSION_B);
        index.seed(row);

        writer
            .read_page(SESSION_A, None, None)
            .expect_err("a row naming another session's file has no page");
        // And a session with no row at all is not an archive this build reads.
        writer
            .read_page(SESSION_C, None, None)
            .expect_err("a session with no row has no page");
    }
}
