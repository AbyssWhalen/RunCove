//! The run log archive as the application runs it.
//!
//! [`crate::archive`] is the engine, and it has no thread of its own: every byte it
//! writes is written by whichever thread called `pump`. This module gives it a
//! thread, a setting, and the three moments of a run it has to follow — a session
//! opening, its lines arriving, its close — so that a capture thread only ever
//! hands a record over and the disk is somebody else's problem.
//!
//! Two facts are reported separately and must not be collapsed into one. `enabled`
//! is the user's stored setting; `available` is whether this run's initialization
//! succeeded. A failed initialization leaves the setting exactly as the user left it
//! and says why, because turning the setting off on their behalf would discard what
//! they asked for, and reporting the feature as on would claim output is being
//! captured when none is.
//!
//! Nothing here fails a run. A record is handed over and forgotten; an archive that
//! cannot be opened, written, or closed is reported through the application's
//! lifecycle error channel and changes nothing else. The archive is a convenience
//! on top of a run, never a precondition for it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::thread;
use std::time::Duration;

use crate::archive::{
    ArchiveCounters, ArchiveFs, ArchiveIndex, ArchiveReason, ArchiveRecord, ArchiveRow,
    ArchiveStatus, ArchiveWriter, QueueBounds, QuotaLimits, RealArchiveFs, SweepReport,
    ARCHIVE_DIR_NAME,
};
use crate::error::{invalid, AppResult};
use crate::models::{RunLogArchivePage, RunLogArchiveState, RunLogEvent};
use crate::storage::now_ms;

/// How long the pump waits to be told there is work before looking anyway.
///
/// A wake with an empty queue costs one lock and no syscall, so the tick is cheap
/// insurance against a notification lost to a race: the archive can fall at most
/// this far behind the capture threads, never further.
const PUMP_IDLE_INTERVAL: Duration = Duration::from_millis(500);

/// The shortest gap between two refreshes of one session's row counters.
const COUNTER_REFRESH_INTERVAL_MS: i64 = 4_000;

/// How many bytes one session may gain between two refreshes of its row counters.
const COUNTER_REFRESH_BYTES: i64 = 1024 * 1024;

/// Said of a service built without an archive behind it.
const UNCONFIGURED: &str = "This build of the run log archive has no storage behind it.";

/// Said when a read or a delete arrives before anything has ever been archived.
const NOTHING_ARCHIVED: &str = "No run log archive exists yet.";

/// Where the archive's own failures go: the application's lifecycle error channel
/// in production, a vector in the tests.
pub type ArchiveReporter = Arc<dyn Fn(String) + Send + Sync>;

/// The archive's failures on their way to the application, with consecutive
/// duplicates dropped.
///
/// The dedupe is one message deep, the same depth the snapshot loop uses for scan
/// errors, and for the same reason: a pump that fails on every tick is one message
/// the user can read rather than two per second, while "it failed again after
/// recovering" is news and gets through.
struct ArchiveReporterState {
    report: ArchiveReporter,
    last: Mutex<Option<String>>,
}

impl ArchiveReporterState {
    fn report(&self, message: String) {
        let mut last = self.last.lock().expect("archive report mutex poisoned");
        if last.as_deref() == Some(message.as_str()) {
            return;
        }
        *last = Some(message.clone());
        // Released before the callback: what the application does with a message is
        // not this lock's business, and an emit must not serialize the next report.
        drop(last);
        (self.report)(message);
    }
}

/// The archive's index with one call thinned out.
///
/// [`ArchiveIndex::update_counters`] is the only method a pump makes on every batch,
/// and a batch can be a single line: left alone, a chatty child process turns every
/// line it prints into an SQLite write. Every other method passes straight through,
/// which is what makes the thinning safe — the row a reader finally sees is written
/// by [`ArchiveIndex::close`], and all that is thinned is how often a *running*
/// session's row catches up with its file.
///
/// The rule is time or bytes, whichever comes first, and a session's first refresh
/// is never thinned, so a row that has just gained its first lines reports them at
/// once.
///
/// This lives here rather than inside [`crate::archive`] on purpose: the writer's
/// contract is that it refreshes the counters after every batch, its tests assert
/// exactly that, and "how often is often enough for a user watching a row" is a
/// decision about the application, not about the writer.
pub struct ThrottledArchiveIndex {
    inner: Arc<dyn ArchiveIndex>,
    /// The last refresh this decorator let through for each session, dropped as
    /// soon as that session's row stops being `writing`. Its size therefore follows
    /// the open sessions and not the run's history.
    refreshed: Mutex<HashMap<String, Refreshed>>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

#[derive(Debug, Clone, Copy)]
struct Refreshed {
    at: i64,
    byte_size: i64,
}

/// Whether a refresh this close behind the last one can wait for the next batch.
///
/// A clock that has gone backwards is deliberately not thinned: `elapsed` outside
/// the range means "this reading makes no sense", and refreshing a row too often is
/// the harmless answer to that.
fn can_wait(last: Refreshed, now: i64, byte_size: i64) -> bool {
    let elapsed = now.saturating_sub(last.at);
    let gained = byte_size.saturating_sub(last.byte_size);
    (0..COUNTER_REFRESH_INTERVAL_MS).contains(&elapsed) && gained < COUNTER_REFRESH_BYTES
}

impl ThrottledArchiveIndex {
    pub fn new(inner: Arc<dyn ArchiveIndex>) -> Self {
        Self::with_clock(inner, Arc::new(now_ms))
    }

    fn with_clock(inner: Arc<dyn ArchiveIndex>, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        Self {
            inner,
            refreshed: Mutex::new(HashMap::new()),
            clock,
        }
    }

    fn refreshed(&self) -> MutexGuard<'_, HashMap<String, Refreshed>> {
        self.refreshed
            .lock()
            .expect("archive counter refresh mutex poisoned")
    }
}

impl ArchiveIndex for ThrottledArchiveIndex {
    fn insert_writing(&self, session_id: &str, file_name: &str, started_at: i64) -> AppResult<()> {
        // Deliberately does not seed the map: without an entry the session's first
        // refresh passes through, which is what makes a just-started row prompt.
        self.inner.insert_writing(session_id, file_name, started_at)
    }

    fn update_counters(&self, session_id: &str, counters: ArchiveCounters) -> AppResult<()> {
        let now = (self.clock)();
        {
            let mut refreshed = self.refreshed();
            if let Some(last) = refreshed.get(session_id) {
                if can_wait(*last, now, counters.byte_size) {
                    return Ok(());
                }
            }
            // Recorded before the write, not after it, so a database that is refusing
            // writes is asked once per interval instead of once per line: the row is
            // behind either way, and the error the writer sees is the same one.
            refreshed.insert(
                session_id.to_string(),
                Refreshed {
                    at: now,
                    byte_size: counters.byte_size,
                },
            );
        }
        self.inner.update_counters(session_id, counters)
    }

    fn close(
        &self,
        session_id: &str,
        status: ArchiveStatus,
        reason: Option<ArchiveReason>,
        counters: ArchiveCounters,
        ended_at: i64,
    ) -> AppResult<()> {
        // Removed before the write and regardless of its outcome: this decorator has
        // nothing more to thin for a session the writer is done with, and an entry
        // kept for a failed close would be a leak.
        self.refreshed().remove(session_id);
        self.inner
            .close(session_id, status, reason, counters, ended_at)
    }

    fn mark_removed(
        &self,
        session_id: &str,
        reason: ArchiveReason,
        ended_at: i64,
    ) -> AppResult<()> {
        self.refreshed().remove(session_id);
        self.inner.mark_removed(session_id, reason, ended_at)
    }

    fn rows(&self) -> AppResult<Vec<ArchiveRow>> {
        self.inner.rows()
    }

    fn row(&self, session_id: &str) -> AppResult<Option<ArchiveRow>> {
        self.inner.row(session_id)
    }
}

/// The handshake between the threads that hand records over and the one thread that
/// writes them.
#[derive(Default)]
struct PumpSignal {
    state: Mutex<PumpState>,
    ready: Condvar,
}

#[derive(Debug, Default, Clone, Copy)]
struct PumpState {
    /// A record has arrived since the last pump.
    pending: bool,
    /// The application is closing: pump once more, then stop.
    stopping: bool,
}

impl PumpSignal {
    fn state(&self) -> MutexGuard<'_, PumpState> {
        self.state.lock().expect("archive pump mutex poisoned")
    }

    /// Wait until there may be work, and report whether this is the last round.
    ///
    /// The wait is an `if` rather than the usual `while`: a spurious wake and a
    /// timeout are both reasons to look at the queue, and looking at an empty queue
    /// costs one lock. Under-waiting is free here; over-waiting would strand records.
    fn wait(&self) -> bool {
        let mut state = self.state();
        if !state.pending && !state.stopping {
            let (next, _timed_out) = self
                .ready
                .wait_timeout(state, PUMP_IDLE_INTERVAL)
                .expect("archive pump mutex poisoned");
            state = next;
        }
        state.pending = false;
        state.stopping
    }

    /// There is a record to write.
    ///
    /// A notification is skipped while one is already outstanding, which cannot lose
    /// a wake-up: `pending` is cleared under this same lock by the pump before it
    /// works, so a flag that is still set means no pump has consumed it yet.
    fn nudge(&self) {
        let mut state = self.state();
        if state.pending {
            return;
        }
        state.pending = true;
        drop(state);
        self.ready.notify_one();
    }

    /// This is the last round.
    fn stop(&self) {
        let mut state = self.state();
        state.stopping = true;
        drop(state);
        self.ready.notify_all();
    }
}

/// What the archive writes through and where, fixed when the service is built.
struct ArchiveBackend {
    archive_dir: PathBuf,
    fs: Arc<dyn ArchiveFs>,
    index: Arc<dyn ArchiveIndex>,
    bounds: QueueBounds,
    limits: QuotaLimits,
}

/// The run log archive, as the rest of the application sees it.
pub struct ArchiveService {
    /// `None` for a service with no archive behind it — the process manager's
    /// default, used everywhere nothing archives.
    backend: Option<ArchiveBackend>,
    /// The user's stored setting, read once per log line.
    enabled: AtomicBool,
    /// Set exactly once, by the initialization that succeeded.
    writer: OnceLock<Arc<ArchiveWriter>>,
    /// Why this run cannot archive, and the lock that makes initialization happen
    /// once.
    ///
    /// [`ArchiveService::record`] never takes it. That is the whole reason the writer
    /// is a [`OnceLock`] and not a `Mutex<Option<_>>`: initialization holds this lock
    /// across a directory sweep, and a capture thread must not wait behind a sweep to
    /// hand over a line.
    failure: Mutex<Option<String>>,
    signal: Arc<PumpSignal>,
    reporter: Arc<ArchiveReporterState>,
}

impl ArchiveService {
    /// The service the application runs: the archive directory beside the database,
    /// the real filesystem, and the index the database backs, with the row counter
    /// refresh thinned on the way in.
    pub fn new(
        data_dir: &Path,
        index: Arc<dyn ArchiveIndex>,
        enabled: bool,
        report: ArchiveReporter,
    ) -> Self {
        Self::with_backend(
            Some(ArchiveBackend {
                archive_dir: data_dir.join(ARCHIVE_DIR_NAME),
                fs: Arc::new(RealArchiveFs),
                index: Arc::new(ThrottledArchiveIndex::new(index)),
                bounds: QueueBounds::default(),
                limits: QuotaLimits::default(),
            }),
            enabled,
            report,
        )
    }

    /// A service with no archive behind it, for a process manager that does not
    /// archive. Every lifecycle call is inert and every read says why.
    ///
    /// Test-only: the application always has a data directory and a database, so a
    /// backend-less service in a shipped build would be a bug rather than a
    /// configuration.
    #[cfg(test)]
    pub fn unconfigured() -> Self {
        Self::with_backend(None, false, Arc::new(|_message| {}))
    }

    fn with_backend(
        backend: Option<ArchiveBackend>,
        enabled: bool,
        report: ArchiveReporter,
    ) -> Self {
        Self {
            backend,
            enabled: AtomicBool::new(enabled),
            writer: OnceLock::new(),
            failure: Mutex::new(None),
            signal: Arc::new(PumpSignal::default()),
            reporter: Arc::new(ArchiveReporterState {
                report,
                last: Mutex::new(None),
            }),
        }
    }

    /// Whether the user's setting is on.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// What the toggle and the viewer show.
    pub fn state(&self) -> RunLogArchiveState {
        let enabled = self.is_enabled();
        match self.unavailable_reason() {
            Some(reason) => RunLogArchiveState {
                enabled,
                available: false,
                unavailable_reason: Some(reason),
            },
            None => RunLogArchiveState {
                enabled,
                available: true,
                unavailable_reason: None,
            },
        }
    }

    /// Why this run cannot archive, or `None` when it can.
    ///
    /// A service that has not initialized yet and has not failed is available: with
    /// the setting off there is nothing to initialize, and reporting that as broken
    /// would put an error in front of a user who has simply left the feature alone.
    fn unavailable_reason(&self) -> Option<String> {
        if self.writer.get().is_some() {
            return None;
        }
        match &self.backend {
            None => Some(UNCONFIGURED.to_string()),
            Some(_) => self
                .failure
                .lock()
                .expect("archive failure mutex poisoned")
                .clone(),
        }
    }

    /// Initialize now if this run has a reason to, at application startup.
    ///
    /// Nothing is created for a user who has never enabled archiving — not even an
    /// empty directory — which is why the setting is checked before the sweep rather
    /// than after it. A directory that is already there is the other reason: it holds
    /// files and rows from earlier runs, and a `writing` row left behind by a crash
    /// stays `writing` until a sweep sees it.
    ///
    /// The gap that leaves is deliberate and small: with the setting off and no
    /// directory, there is nothing on disk to reconcile, so there is nothing to
    /// reconcile it against.
    pub fn start(&self) {
        let Some(backend) = &self.backend else {
            return;
        };
        if !self.has_reason_to_initialize(backend) {
            return;
        }
        self.initialize();
    }

    /// The setting is on, or an archive directory from an earlier run is there to
    /// reconcile.
    ///
    /// A directory that exists but cannot be listed reads as absent here. With the
    /// setting off that is right — nothing will be written, so nothing needs
    /// reconciling — and with it on the sweep runs anyway and reports what it found.
    fn has_reason_to_initialize(&self, backend: &ArchiveBackend) -> bool {
        self.is_enabled() || backend.fs.list_dir(&backend.archive_dir).is_ok()
    }

    /// Create the writer, sweep the directory, and park the pump on it. Idempotent,
    /// and the only place a writer is made.
    ///
    /// Reports whether this run has a writer afterwards. A failure becomes the
    /// unavailable reason and is reported once; the next call tries again, because
    /// what failed can be temporary — a directory that was locked, a disk that was
    /// full.
    fn initialize(&self) -> bool {
        if self.writer.get().is_some() {
            return true;
        }
        let Some(backend) = &self.backend else {
            return false;
        };
        let mut failure = self.failure.lock().expect("archive failure mutex poisoned");
        // Checked again under the lock: two threads enabling at once must sweep once.
        if self.writer.get().is_some() {
            return true;
        }
        let started = ArchiveWriter::initialize(
            backend.archive_dir.clone(),
            backend.fs.clone(),
            backend.index.clone(),
            backend.bounds,
            backend.limits,
            now_ms(),
        );
        let (writer, sweep) = match started {
            Ok(ready) => ready,
            Err(error) => return self.fail_initialization(failure, error.to_string()),
        };
        let writer = Arc::new(writer);
        // The pump is started before the writer is published, so a thread that cannot
        // be spawned is an initialization failure rather than an archive that accepts
        // records and drains them only when a session closes.
        if let Err(error) = self.start_pump(&writer) {
            let message = format!("The run log archive could not start its writer thread: {error}");
            return self.fail_initialization(failure, message);
        }
        let _ = self.writer.set(writer);
        *failure = None;
        drop(failure);
        self.report_sweep(&sweep);
        true
    }

    /// Remember and report why this run has no writer. Always `false`.
    fn fail_initialization(
        &self,
        mut failure: MutexGuard<'_, Option<String>>,
        message: String,
    ) -> bool {
        *failure = Some(message.clone());
        // Released before reporting: the reporter calls out of this module, and
        // nothing it does needs the initialization lock held.
        drop(failure);
        self.reporter.report(message);
        false
    }

    /// Park a thread on the writer's `pump`.
    ///
    /// The handle is dropped rather than kept. Exit is [`ArchiveService::shutdown`],
    /// which drains and closes on the calling thread; joining a pump that is blocked
    /// in a write on a stalled disk would turn a slow disk into an application that
    /// will not close.
    fn start_pump(&self, writer: &Arc<ArchiveWriter>) -> std::io::Result<()> {
        let signal = self.signal.clone();
        let reporter = self.reporter.clone();
        // A `Weak` handle, so a service dropped without a `shutdown` — every test that
        // ends, and nothing in production — takes its pump with it instead of leaving
        // a thread pumping a writer nobody can reach.
        let writer = Arc::downgrade(writer);
        thread::Builder::new()
            .name("runcove-archive-pump".into())
            .spawn(move || pump_loop(&signal, &writer, &reporter))?;
        Ok(())
    }

    /// Surface what the startup sweep refused to touch.
    ///
    /// The repairs are not reported: a `writing` row left by a crash becoming
    /// `partial` / `interrupted` is the sweep doing its job, and a message about it
    /// would reach the user as a failure. An anomaly is the opposite — something the
    /// sweep will not act on, and only the user can.
    fn report_sweep(&self, sweep: &SweepReport) {
        if sweep.anomalies.is_empty() {
            return;
        }
        self.reporter
            .report(format!("Run log archive: {}", sweep.anomalies.join(" ")));
    }

    /// Open an archive for a session that has just started, and report whether it
    /// has one.
    ///
    /// A failure is reported and otherwise swallowed: a run must not fail because its
    /// archive could not be created.
    pub fn begin_session(&self, session_id: &str, started_at: i64) -> bool {
        if !self.is_enabled() || !self.initialize() {
            return false;
        }
        let Some(writer) = self.writer.get() else {
            return false;
        };
        match writer.begin(session_id, started_at) {
            Ok(()) => true,
            Err(error) => {
                self.reporter.report(error.to_string());
                false
            }
        }
    }

    /// Hand one log line over. Does no file and no index work.
    ///
    /// A line for a session with no open archive is dropped here or ignored by the
    /// writer, and both are the same thing to the caller, which is why every line of
    /// every run can be handed over unconditionally.
    pub fn record(&self, session_id: &str, event: &RunLogEvent) {
        if !self.is_enabled() {
            return;
        }
        let Some(writer) = self.writer.get() else {
            return;
        };
        writer.enqueue(ArchiveRecord {
            session_id: session_id.to_string(),
            stream: event.stream,
            line: event.line.clone(),
            timestamp: event.timestamp,
        });
        self.signal.nudge();
    }

    /// Close one session's archive, if this run opened one.
    ///
    /// Safe to call for a session that was never archived: the writer holds no slot
    /// for it, and that is the ordinary case here rather than an error worth
    /// reporting — a session that ran while archiving was off reaches this exactly
    /// like one that ran while it was on.
    ///
    /// No pump precedes it. A close writes everything its own session still has
    /// queued, and draining every *other* session's backlog on a thread that is
    /// finishing one process would make one child's exit wait on another child's
    /// output.
    pub fn close_session(&self, session_id: &str) {
        let Some(writer) = self.writer.get() else {
            return;
        };
        if !writer.is_open(session_id) {
            return;
        }
        if let Err(error) = writer.close(session_id, None, now_ms()) {
            self.reporter.report(error.to_string());
        }
    }

    /// Apply a change of the user's setting. Persisting it is the caller's business;
    /// this is what the change means to a run already in progress.
    ///
    /// Turning it on affects only the sessions that start afterwards: a session
    /// already running has no `writing` row, and the writer ignores a record for a
    /// session it never opened, so there is nothing to backfill and no half archive
    /// can appear. Turning it off closes every open archive as `user-disabled`
    /// instead of abandoning it, so each one still ends as a readable file with a row
    /// that says why it stopped.
    pub fn set_enabled(&self, enabled: bool) -> RunLogArchiveState {
        let was_enabled = self.enabled.swap(enabled, Ordering::SeqCst);
        if enabled {
            self.initialize();
        } else if was_enabled {
            self.close_open_archives(ArchiveReason::UserDisabled);
        }
        self.state()
    }

    /// Stop archiving for good: drain what is queued, end every archive still open,
    /// and stop the pump.
    ///
    /// It returns nothing because closing the application must not fail on account of
    /// the archive and must not wait on it either. The bound is the queue's own caps
    /// rather than a clock — one pump drains everything queued, and what can be queued
    /// is capped by [`QueueBounds`] — so there is no timeout to tune and no file left
    /// half written by one.
    pub fn shutdown(&self) {
        self.signal.stop();
        self.close_open_archives(ArchiveReason::Interrupted);
    }

    /// Persist what is queued, then end every open archive with one reason.
    ///
    /// The pump comes first so that the bytes it can account for go through the
    /// quota, which a close does not consult; whatever a close then writes overshoots
    /// the cap by at most one session's queue. A pump that fails is reported and does
    /// not stop the closes: an unwritable disk is exactly when the rows matter.
    fn close_open_archives(&self, reason: ArchiveReason) {
        let Some(writer) = self.writer.get() else {
            return;
        };
        if let Err(error) = writer.pump(now_ms()) {
            self.reporter.report(error.to_string());
        }
        if let Err(error) = writer.close_all(reason, now_ms()) {
            self.reporter.report(error.to_string());
        }
    }

    /// One page of an archived session, oldest record of the page first.
    pub fn read_page(
        &self,
        session_id: &str,
        before_offset: Option<u64>,
        max_records: Option<usize>,
    ) -> AppResult<RunLogArchivePage> {
        self.reader()?
            .read_page(session_id, before_offset, max_records)
    }

    /// Delete one archived session: its file, then its row.
    pub fn delete(&self, session_id: &str) -> AppResult<()> {
        self.reader()?.delete(session_id)
    }

    /// The writer a read or a delete goes through, or why there is none.
    ///
    /// Unlike the lifecycle calls, these two have a user waiting for an answer, so
    /// they say what is wrong instead of doing nothing. They also initialize: an
    /// archive written by an earlier run outlives the setting, and a user who has
    /// just turned archiving off must still be able to read and delete what is
    /// already there.
    fn reader(&self) -> AppResult<&Arc<ArchiveWriter>> {
        if let Some(writer) = self.writer.get() {
            return Ok(writer);
        }
        let Some(backend) = &self.backend else {
            return Err(invalid(UNCONFIGURED));
        };
        if !self.has_reason_to_initialize(backend) {
            return Err(invalid(NOTHING_ARCHIVED));
        }
        if self.initialize() {
            if let Some(writer) = self.writer.get() {
                return Ok(writer);
            }
        }
        Err(invalid(
            self.unavailable_reason()
                .unwrap_or_else(|| NOTHING_ARCHIVED.to_string()),
        ))
    }
}

/// The pump: one thread, one writer, for as long as the writer is reachable.
///
/// It pumps on every wake, a timeout included, so a notification lost to a race
/// costs the archive one [`PUMP_IDLE_INTERVAL`] rather than stranding records until
/// the next line arrives. A pump with an empty queue touches neither the filesystem
/// nor the index, so an idle tick is two locks and no syscall.
fn pump_loop(signal: &PumpSignal, writer: &Weak<ArchiveWriter>, reporter: &ArchiveReporterState) {
    loop {
        let stopping = signal.wait();
        let Some(writer) = writer.upgrade() else {
            return;
        };
        if let Err(error) = writer.pump(now_ms()) {
            reporter.report(error.to_string());
        }
        // Dropped before the wait, so a service being dropped is not kept alive by a
        // thread that is only waiting to be told about work that will never come.
        drop(writer);
        if stopping {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LogStream;
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::TempDir;

    const SESSION_ONE: &str = "11111111-1111-4111-8111-111111111111";
    const SESSION_TWO: &str = "22222222-2222-4222-8222-222222222222";

    fn archive_name(session_id: &str) -> String {
        format!("{session_id}.jsonl")
    }

    fn counters(line_count: i64, byte_size: i64) -> ArchiveCounters {
        ArchiveCounters {
            line_count,
            byte_size,
            dropped_lines: 0,
            dropped_bytes: 0,
        }
    }

    fn log_event(stream: LogStream, line: &str, timestamp: i64) -> RunLogEvent {
        RunLogEvent {
            profile_id: "profile".into(),
            stream,
            line: line.into(),
            reason: None,
            timestamp,
        }
    }

    /// Wait for something the pump thread has to do first.
    ///
    /// The interval is a liveness check and not a timing assertion: every test that
    /// waits also asserts the outcome, so a slow machine only makes the wait longer.
    fn wait_until(mut ready: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if ready() {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        ready()
    }

    /// An in-memory [`ArchiveIndex`] holding the rows and counting the writes.
    ///
    /// The Storage-backed index has its own tests over real SQL. What these tests need
    /// is somewhere for the rows to go that they can read without a database, and a
    /// count of the counter writes, so that what the throttle thins is observable.
    #[derive(Default)]
    struct TestIndex {
        rows: Mutex<BTreeMap<String, ArchiveRow>>,
        counter_updates: Mutex<Vec<(String, ArchiveCounters)>>,
    }

    impl TestIndex {
        fn rows_of(&self) -> MutexGuard<'_, BTreeMap<String, ArchiveRow>> {
            self.rows.lock().expect("test index rows")
        }

        fn row_of(&self, session_id: &str) -> Option<ArchiveRow> {
            self.rows_of().get(session_id).cloned()
        }

        fn counter_updates(&self) -> Vec<(String, ArchiveCounters)> {
            self.counter_updates
                .lock()
                .expect("test index counter updates")
                .clone()
        }
    }

    impl ArchiveIndex for TestIndex {
        fn insert_writing(
            &self,
            session_id: &str,
            file_name: &str,
            started_at: i64,
        ) -> AppResult<()> {
            let mut rows = self.rows_of();
            if rows.contains_key(session_id) {
                return Err(invalid(format!("{session_id} already has an archive row")));
            }
            rows.insert(
                session_id.to_string(),
                ArchiveRow {
                    session_id: session_id.to_string(),
                    file_name: file_name.to_string(),
                    status: ArchiveStatus::Writing.as_str().to_string(),
                    reason: None,
                    counters: ArchiveCounters::default(),
                    started_at,
                    ended_at: None,
                },
            );
            Ok(())
        }

        fn update_counters(&self, session_id: &str, counters: ArchiveCounters) -> AppResult<()> {
            self.counter_updates
                .lock()
                .expect("test index counter updates")
                .push((session_id.to_string(), counters));
            let mut rows = self.rows_of();
            let row = rows
                .get_mut(session_id)
                .ok_or_else(|| invalid(format!("{session_id} has no archive row")))?;
            row.counters = counters;
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
            let mut rows = self.rows_of();
            let row = rows
                .get_mut(session_id)
                .ok_or_else(|| invalid(format!("{session_id} has no archive row")))?;
            row.status = status.as_str().to_string();
            row.reason = reason.map(|reason| reason.as_str().to_string());
            row.counters = counters;
            row.ended_at = Some(ended_at);
            Ok(())
        }

        fn mark_removed(
            &self,
            session_id: &str,
            reason: ArchiveReason,
            ended_at: i64,
        ) -> AppResult<()> {
            let mut rows = self.rows_of();
            let row = rows
                .get_mut(session_id)
                .ok_or_else(|| invalid(format!("{session_id} has no archive row")))?;
            row.status = ArchiveStatus::Removed.as_str().to_string();
            row.reason = Some(reason.as_str().to_string());
            row.ended_at = Some(ended_at);
            Ok(())
        }

        fn rows(&self) -> AppResult<Vec<ArchiveRow>> {
            let mut rows: Vec<ArchiveRow> = self.rows_of().values().cloned().collect();
            rows.sort_by(|left, right| {
                (left.started_at, &left.session_id).cmp(&(right.started_at, &right.session_id))
            });
            Ok(rows)
        }

        fn row(&self, session_id: &str) -> AppResult<Option<ArchiveRow>> {
            Ok(self.row_of(session_id))
        }
    }

    /// One service over real files in a temporary directory, with the rows and the
    /// reported failures kept where the test can read them.
    struct Fixture {
        temp: TempDir,
        index: Arc<TestIndex>,
        reports: Arc<Mutex<Vec<String>>>,
        service: Arc<ArchiveService>,
    }

    impl Fixture {
        fn new(enabled: bool) -> Self {
            let temp = tempfile::tempdir().expect("temporary directory");
            let index = Arc::new(TestIndex::default());
            let reports = Arc::new(Mutex::new(Vec::new()));
            let sink = reports.clone();
            let service = Arc::new(ArchiveService::new(
                temp.path(),
                index.clone(),
                enabled,
                Arc::new(move |message| sink.lock().expect("report sink").push(message)),
            ));
            Self {
                temp,
                index,
                reports,
                service,
            }
        }

        fn archive_dir(&self) -> PathBuf {
            self.temp.path().join(ARCHIVE_DIR_NAME)
        }

        fn archive_file(&self, session_id: &str) -> PathBuf {
            self.archive_dir().join(archive_name(session_id))
        }

        fn reports(&self) -> Vec<String> {
            self.reports.lock().expect("report sink").clone()
        }
    }

    impl Drop for Fixture {
        /// Stop the pump before the temporary directory goes away, so no test leaves a
        /// thread writing into a directory that is being deleted. It runs after the
        /// test's assertions and before `temp` is dropped, and a service with no writer
        /// ignores it.
        fn drop(&mut self) {
            self.service.shutdown();
        }
    }

    #[test]
    fn an_unconfigured_service_is_inert_and_says_why() {
        let service = ArchiveService::unconfigured();

        let state = service.state();
        assert!(!state.enabled);
        assert!(!state.available);
        assert_eq!(state.unavailable_reason.as_deref(), Some(UNCONFIGURED));

        assert!(!service.begin_session(SESSION_ONE, 1_000));
        service.record(SESSION_ONE, &log_event(LogStream::Stdout, "ignored", 1_100));
        service.close_session(SESSION_ONE);
        service.shutdown();
        assert!(service.read_page(SESSION_ONE, None, None).is_err());
        assert!(service.delete(SESSION_ONE).is_err());

        // Enabling one is not refused, because the setting is the user's; it simply
        // stays unavailable, which is what the toggle then shows.
        let state = service.set_enabled(true);
        assert!(state.enabled);
        assert!(!state.available);
    }

    #[test]
    fn the_setting_off_writes_nothing_and_creates_no_directory() {
        let fixture = Fixture::new(false);
        fixture.service.start();

        let state = fixture.service.state();
        assert!(!state.enabled);
        assert!(state.available, "off is not broken");
        assert_eq!(state.unavailable_reason, None);

        assert!(!fixture.service.begin_session(SESSION_ONE, 1_000));
        fixture
            .service
            .record(SESSION_ONE, &log_event(LogStream::Stdout, "ready", 1_100));
        fixture.service.close_session(SESSION_ONE);
        fixture.service.shutdown();

        assert!(!fixture.archive_dir().exists(), "no directory was created");
        assert!(fixture.index.rows().unwrap().is_empty());
        assert!(fixture.reports().is_empty());

        // A read says there is nothing rather than creating a directory to prove it.
        assert!(fixture.service.read_page(SESSION_ONE, None, None).is_err());
        assert!(!fixture.archive_dir().exists());
    }

    #[test]
    fn a_started_session_archives_both_streams_and_reads_them_back() {
        let fixture = Fixture::new(true);
        fixture.service.start();
        assert!(fixture.service.begin_session(SESSION_ONE, 1_000));

        fixture
            .service
            .record(SESSION_ONE, &log_event(LogStream::Stdout, "ready", 1_100));
        fixture.service.record(
            SESSION_ONE,
            &log_event(LogStream::Stderr, "warning: slow", 1_200),
        );
        fixture.service.record(
            SESSION_ONE,
            &log_event(LogStream::System, "Process exited", 1_300),
        );
        fixture.service.close_session(SESSION_ONE);

        let page = fixture
            .service
            .read_page(SESSION_ONE, None, None)
            .expect("the archived page");
        let lines: Vec<&str> = page
            .records
            .iter()
            .map(|record| record.line.as_str())
            .collect();
        assert_eq!(lines, vec!["ready", "warning: slow", "Process exited"]);
        assert!(matches!(page.records[1].stream, LogStream::Stderr));
        assert_eq!(page.records[2].timestamp, 1_300);
        assert_eq!(page.line_count, 3);
        assert_eq!(page.dropped_lines, 0);
        assert_eq!(page.status, "complete");
        assert!(!page.has_more_before);

        let row = fixture.index.row_of(SESSION_ONE).expect("the closed row");
        assert_eq!(row.status, "complete");
        assert_eq!(row.reason, None);
        assert_eq!(row.counters.line_count, 3);
        assert!(row.ended_at.is_some());
        assert!(fixture.reports().is_empty(), "{:?}", fixture.reports());
    }

    #[test]
    fn the_pump_writes_a_record_without_waiting_for_the_close() {
        let fixture = Fixture::new(true);
        fixture.service.start();
        assert!(fixture.service.begin_session(SESSION_ONE, 1_000));
        fixture.service.record(
            SESSION_ONE,
            &log_event(LogStream::Stdout, "listening on 5173", 1_100),
        );

        let path = fixture.archive_file(SESSION_ONE);
        assert!(
            wait_until(|| fs::metadata(&path)
                .map(|meta| meta.len() > 0)
                .unwrap_or(false)),
            "the pump thread never wrote the record"
        );
        // The row catches up too, which is the first refresh the throttle never thins.
        assert!(
            wait_until(|| fixture
                .index
                .row_of(SESSION_ONE)
                .is_some_and(|row| row.counters.line_count == 1)),
            "the pump thread never refreshed the row"
        );

        let row = fixture.index.row_of(SESSION_ONE).expect("the writing row");
        assert_eq!(row.status, "writing", "the session is still open");
        assert!(row.ended_at.is_none());
        assert!(fixture.service.state().available);
    }

    #[test]
    fn enabling_the_setting_does_not_backfill_a_running_session() {
        let fixture = Fixture::new(false);
        fixture.service.start();
        assert!(!fixture.service.begin_session(SESSION_ONE, 1_000));
        fixture
            .service
            .record(SESSION_ONE, &log_event(LogStream::Stdout, "before", 1_100));

        let state = fixture.service.set_enabled(true);
        assert!(state.enabled);
        assert!(state.available);

        // The session that was already running stays unarchived: it has no row to
        // append to, so half of its output would be a lie about what it printed.
        fixture
            .service
            .record(SESSION_ONE, &log_event(LogStream::Stdout, "after", 1_200));
        fixture.service.close_session(SESSION_ONE);

        // The next session to start does archive.
        assert!(fixture.service.begin_session(SESSION_TWO, 2_000));
        fixture
            .service
            .record(SESSION_TWO, &log_event(LogStream::Stdout, "second", 2_100));
        fixture.service.shutdown();

        assert!(fixture.index.row_of(SESSION_ONE).is_none());
        assert!(!fixture.archive_file(SESSION_ONE).exists());

        // Closed by the shutdown rather than by its own process, so it is `partial`.
        let row = fixture.index.row_of(SESSION_TWO).expect("the new row");
        assert_eq!(row.status, "partial");
        assert_eq!(row.reason.as_deref(), Some("interrupted"));
        assert_eq!(row.counters.line_count, 1);
        let page = fixture.service.read_page(SESSION_TWO, None, None).unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].line, "second");
    }

    #[test]
    fn disabling_the_setting_closes_the_open_archive_and_stops_new_ones() {
        let fixture = Fixture::new(true);
        fixture.service.start();
        assert!(fixture.service.begin_session(SESSION_ONE, 1_000));
        fixture
            .service
            .record(SESSION_ONE, &log_event(LogStream::Stdout, "running", 1_100));

        let state = fixture.service.set_enabled(false);
        assert!(!state.enabled);
        assert!(state.available);

        let row = fixture.index.row_of(SESSION_ONE).expect("the closed row");
        assert_eq!(row.status, "partial");
        assert_eq!(row.reason.as_deref(), Some("user-disabled"));
        assert_eq!(row.counters.line_count, 1);
        assert!(row.ended_at.is_some());

        // What it did capture stays readable with the setting off.
        let page = fixture.service.read_page(SESSION_ONE, None, None).unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].line, "running");

        // And nothing starts archiving afterwards.
        assert!(!fixture.service.begin_session(SESSION_TWO, 2_000));
        fixture
            .service
            .record(SESSION_TWO, &log_event(LogStream::Stdout, "second", 2_100));
        fixture.service.close_session(SESSION_TWO);
        assert!(fixture.index.row_of(SESSION_TWO).is_none());
        assert!(!fixture.archive_file(SESSION_TWO).exists());
    }

    #[test]
    fn an_initialization_failure_keeps_the_setting_and_reports_once() {
        let fixture = Fixture::new(true);
        // A file where the directory belongs, so no `create_dir_all` can succeed.
        fs::write(fixture.archive_dir(), b"not a directory").expect("the blocking file");

        fixture.service.start();

        let state = fixture.service.state();
        assert!(state.enabled, "a failure must not turn the setting off");
        assert!(!state.available);
        assert!(state.unavailable_reason.is_some());

        // Nothing else in the run is affected: every call is inert and none panics.
        assert!(!fixture.service.begin_session(SESSION_ONE, 1_000));
        fixture
            .service
            .record(SESSION_ONE, &log_event(LogStream::Stdout, "ready", 1_100));
        fixture.service.close_session(SESSION_ONE);
        assert!(fixture.service.read_page(SESSION_ONE, None, None).is_err());
        assert!(fixture.service.delete(SESSION_ONE).is_err());
        assert!(fixture.index.rows().unwrap().is_empty());

        // Four retries, one message: the same failure repeated is not news.
        assert_eq!(fixture.reports().len(), 1, "{:?}", fixture.reports());
    }

    #[test]
    fn a_directory_from_an_earlier_run_is_reconciled_with_the_setting_off() {
        let fixture = Fixture::new(false);
        // What a crash leaves behind: a `writing` row and the file it names.
        fs::create_dir_all(fixture.archive_dir()).expect("the archive directory");
        fs::write(
            fixture.archive_file(SESSION_ONE),
            b"{\"t\":1100,\"s\":\"stdout\",\"l\":\"before the crash\"}\n",
        )
        .expect("the archive file");
        fixture
            .index
            .insert_writing(SESSION_ONE, &archive_name(SESSION_ONE), 1_000)
            .unwrap();

        fixture.service.start();

        let row = fixture.index.row_of(SESSION_ONE).expect("the repaired row");
        assert_eq!(row.status, "partial");
        assert_eq!(row.reason.as_deref(), Some("interrupted"));
        let state = fixture.service.state();
        assert!(!state.enabled, "reconciling is not enabling");
        assert!(state.available);

        // Readable afterwards, which is the point of repairing it.
        let page = fixture.service.read_page(SESSION_ONE, None, None).unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].line, "before the crash");
    }

    #[test]
    fn deleting_an_archive_removes_its_file_but_never_an_open_one() {
        let fixture = Fixture::new(true);
        fixture.service.start();
        assert!(fixture.service.begin_session(SESSION_ONE, 1_000));
        fixture
            .service
            .record(SESSION_ONE, &log_event(LogStream::Stdout, "first", 1_100));
        fixture.service.close_session(SESSION_ONE);
        assert!(fixture.service.begin_session(SESSION_TWO, 2_000));

        // An archive still being written is refused, file and row untouched.
        assert!(fixture.service.delete(SESSION_TWO).is_err());
        assert!(fixture.archive_file(SESSION_TWO).exists());
        assert_eq!(
            fixture.index.row_of(SESSION_TWO).map(|row| row.status),
            Some("writing".to_string())
        );

        fixture
            .service
            .delete(SESSION_ONE)
            .expect("a closed archive is deletable");
        assert!(!fixture.archive_file(SESSION_ONE).exists());
        let row = fixture.index.row_of(SESSION_ONE).expect("the row survives");
        assert_eq!(row.status, "removed");
        assert_eq!(row.reason.as_deref(), Some("user-deleted"));
        assert_eq!(row.counters.line_count, 1, "the counters are kept");
        assert!(fixture.service.read_page(SESSION_ONE, None, None).is_err());
    }

    #[test]
    fn the_counter_refresh_is_thinned_by_time_and_by_bytes_per_session() {
        let inner = Arc::new(TestIndex::default());
        let clock = Arc::new(Mutex::new(1_000_i64));
        let reading = clock.clone();
        let index = ThrottledArchiveIndex::with_clock(
            inner.clone(),
            Arc::new(move || *reading.lock().expect("test clock")),
        );
        let set_clock = |value: i64| *clock.lock().expect("test clock") = value;

        index
            .insert_writing(SESSION_ONE, &archive_name(SESSION_ONE), 1_000)
            .unwrap();

        // A session's first refresh is never thinned.
        index.update_counters(SESSION_ONE, counters(1, 10)).unwrap();
        assert_eq!(inner.counter_updates().len(), 1);

        // Too soon and too few bytes: the row waits for the next batch.
        set_clock(2_000);
        index.update_counters(SESSION_ONE, counters(2, 20)).unwrap();
        assert_eq!(inner.counter_updates().len(), 1);

        // A megabyte in the same instant goes through, and so does the next interval.
        index
            .update_counters(SESSION_ONE, counters(3, 10 + COUNTER_REFRESH_BYTES))
            .unwrap();
        assert_eq!(inner.counter_updates().len(), 2);
        set_clock(2_000 + COUNTER_REFRESH_INTERVAL_MS);
        index
            .update_counters(SESSION_ONE, counters(4, 10 + COUNTER_REFRESH_BYTES))
            .unwrap();
        assert_eq!(inner.counter_updates().len(), 3);

        // The row a reader finally sees is the one the close wrote, whole.
        index
            .close(
                SESSION_ONE,
                ArchiveStatus::Complete,
                None,
                counters(9, 900),
                3_000,
            )
            .unwrap();
        let row = inner.row_of(SESSION_ONE).expect("the closed row");
        assert_eq!(row.counters, counters(9, 900));
        assert_eq!(row.status, "complete");
        assert_eq!(
            inner.counter_updates().len(),
            3,
            "a close is not a counter refresh"
        );

        // Another session's first refresh is its own, however recently this one wrote.
        index
            .insert_writing(SESSION_TWO, &archive_name(SESSION_TWO), 3_000)
            .unwrap();
        index.update_counters(SESSION_TWO, counters(1, 5)).unwrap();
        assert_eq!(inner.counter_updates().len(), 4);

        // And the closed session left nothing behind, so the map follows the open
        // sessions rather than the run's history.
        assert_eq!(index.refreshed().len(), 1);
        assert!(index.refreshed().contains_key(SESSION_TWO));
    }

    /// The archive as the application assembles it, over the real database, across a
    /// restart.
    ///
    /// Every other test here swaps the index for [`TestIndex`], which is what makes
    /// them focused — and it is also the one thing a user cannot do. This one uses
    /// `Storage`, a real file, and a real session row, then drops the whole service
    /// and reopens the database the way relaunching RunCove does, because that is the
    /// only way to prove the archive is still *findable*: the file surviving proves
    /// nothing if the row that names it did not.
    ///
    /// The second service is built with the setting **off** on purpose. Reading and
    /// deleting an archive written earlier must not depend on the setting still being
    /// on, and nothing here re-enables it.
    #[test]
    fn a_restart_still_finds_pages_and_deletes_an_archive_in_the_real_database() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let database = temp.path().join("runcove.sqlite3");
        let archive_dir = temp.path().join(ARCHIVE_DIR_NAME);
        let reports = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = || {
            let sink = reports.clone();
            Arc::new(move |message: String| sink.lock().expect("report sink").push(message))
                as ArchiveReporter
        };
        let session_id;
        let archive_file;

        {
            let storage = Arc::new(crate::storage::Storage::open(&database).expect("database"));
            // A run session's `profile_id` is a real foreign key, so the archive gets
            // its session the way the application does: a project, its profile, a run.
            let project = storage
                .save_project(crate::models::ProjectInput {
                    id: None,
                    name: "Demo".into(),
                    path: temp.path().to_string_lossy().into_owned(),
                    profiles: vec![crate::models::LaunchProfileInput {
                        id: None,
                        name: "Web".into(),
                        program: "cmd.exe".into(),
                        args: vec!["/c".into(), "echo ready".into()],
                        cwd: temp.path().to_string_lossy().into_owned(),
                        expected_ports: Vec::new(),
                    }],
                })
                .expect("a project with one profile");
            let profile = &project.profiles[0];
            session_id = storage
                .begin_session(&profile.id, &profile.name)
                .expect("a run session");
            archive_file = archive_dir.join(archive_name(&session_id));

            let service = ArchiveService::new(temp.path(), storage.clone(), true, sink());
            service.start();
            assert!(service.state().available, "{:?}", service.state());
            assert!(service.begin_session(&session_id, 10_000));

            for index in 0..9_i64 {
                let stream = if index % 3 == 2 {
                    LogStream::Stderr
                } else {
                    LogStream::Stdout
                };
                service.record(
                    &session_id,
                    &log_event(stream, &format!("line {index}"), 10_100 + index),
                );
            }
            service.record(
                &session_id,
                &log_event(LogStream::System, "Process exited with code 0", 10_200),
            );
            service.close_session(&session_id);
            storage
                .finish_session(&session_id, Some(0))
                .expect("the run session ends");
            service.shutdown();
            // Both handles go out of scope here: the pump thread is stopped, the
            // database connection is closed, and what is left is only what is on disk.
        }

        let storage = Arc::new(crate::storage::Storage::open(&database).expect("reopened"));
        let session = storage
            .list_sessions(10)
            .expect("run history")
            .into_iter()
            .find(|candidate| candidate.id == session_id)
            .expect("the session is still in run history");
        let summary = session.archive.expect("the archive summary survived");
        assert_eq!(summary.status, "complete");
        assert_eq!(summary.reason, None);
        assert_eq!(summary.line_count, 10);
        assert_eq!(summary.dropped_lines, 0);
        assert!(summary.byte_size > 0);
        assert!(summary.ended_at.is_some());

        let service = ArchiveService::new(temp.path(), storage.clone(), false, sink());
        service.start();
        let state = service.state();
        assert!(!state.enabled, "the setting stayed off across the restart");
        assert!(state.available, "{state:?}");

        // A viewer opens at the end of the file, so the first page is the last records.
        let tail = service
            .read_page(&session_id, None, Some(4))
            .expect("the last page");
        let lines: Vec<&str> = tail
            .records
            .iter()
            .map(|record| record.line.as_str())
            .collect();
        assert_eq!(
            lines,
            vec!["line 6", "line 7", "line 8", "Process exited with code 0"]
        );
        assert!(matches!(tail.records[2].stream, LogStream::Stderr));
        assert!(matches!(tail.records[3].stream, LogStream::System));
        assert_eq!(tail.line_count, 10);
        assert!(tail.has_more_before, "nine earlier records are still there");
        assert_eq!(tail.status, "complete");

        // And pages backwards from where that page began, until it reports the start.
        let earlier = service
            .read_page(&session_id, Some(tail.page_start_offset), Some(4))
            .expect("the page before it");
        let lines: Vec<&str> = earlier
            .records
            .iter()
            .map(|record| record.line.as_str())
            .collect();
        assert_eq!(lines, vec!["line 2", "line 3", "line 4", "line 5"]);
        assert!(earlier.has_more_before);
        let first = service
            .read_page(&session_id, Some(earlier.page_start_offset), Some(4))
            .expect("the first page");
        let lines: Vec<&str> = first
            .records
            .iter()
            .map(|record| record.line.as_str())
            .collect();
        assert_eq!(lines, vec!["line 0", "line 1"]);
        assert!(!first.has_more_before, "that was the start of the archive");
        assert_eq!(first.page_start_offset, 0);

        // Deleting takes the file and keeps the history entry, saying who removed it.
        assert!(archive_file.is_file());
        service.delete(&session_id).expect("the delete");
        assert!(!archive_file.exists(), "the file is gone");
        let session = storage
            .list_sessions(10)
            .expect("run history")
            .into_iter()
            .find(|candidate| candidate.id == session_id)
            .expect("the session is still in run history after the delete");
        let summary = session.archive.expect("the row outlives the file");
        assert_eq!(summary.status, "removed");
        assert_eq!(summary.reason.as_deref(), Some("user-deleted"));
        assert_eq!(summary.line_count, 10, "what it held is still recorded");

        // A read after the delete says so instead of resurrecting an empty page.
        assert!(service.read_page(&session_id, None, Some(4)).is_err());
        service.shutdown();
        assert!(
            reports.lock().expect("report sink").is_empty(),
            "{:?}",
            reports.lock().expect("report sink")
        );
    }
}
