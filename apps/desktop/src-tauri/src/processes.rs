use crate::error::{invalid, AppResult};
use crate::models::{LaunchProfile, LogStream, RunLogEvent, RunStatus, RunStatusEvent};
use crate::state::AppState;
use crate::storage::now_ms;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use sysinfo::{Pid, System};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const SHUTDOWN_IDLE: u8 = 0;
const SHUTDOWN_ACTIVE: u8 = 1;
const SHUTDOWN_COMPLETE: u8 = 2;
const LOG_LINE_BYTE_LIMIT: usize = 16 * 1024;
const LOG_LINE_TRUNCATED_MARKER: &str = " [RunCove: log line truncated]";

#[derive(Clone)]
pub struct ManagedInfo {
    pub profile_id: String,
    pub project_id: String,
    pub pid: u32,
    pub session_id: String,
    sequence: u64,
    tree: Arc<OwnedProcessTree>,
}

pub struct ProcessManager {
    managed: Arc<Mutex<HashMap<String, ManagedInfo>>>,
    lifecycle_reservations: Arc<Mutex<LifecycleReservations>>,
    statuses: Arc<Mutex<HashMap<String, RunStatus>>>,
    exit_intents: Arc<Mutex<HashMap<String, ExitIntent>>>,
    unexpected_exits: Arc<Mutex<HashSet<String>>>,
    logs: Arc<Mutex<HashMap<String, VecDeque<RunLogEvent>>>>,
    restore_sync_suspended: Arc<AtomicBool>,
    shutdown_phase: Arc<AtomicU8>,
    shutdown_snapshot: Mutex<Option<Vec<String>>>,
    sequence: AtomicU64,
    log_capacity: usize,
}

impl ProcessManager {
    pub fn new(log_capacity: usize) -> Self {
        Self {
            managed: Arc::new(Mutex::new(HashMap::new())),
            lifecycle_reservations: Arc::new(Mutex::new(LifecycleReservations::default())),
            statuses: Arc::new(Mutex::new(HashMap::new())),
            exit_intents: Arc::new(Mutex::new(HashMap::new())),
            unexpected_exits: Arc::new(Mutex::new(HashSet::new())),
            logs: Arc::new(Mutex::new(HashMap::new())),
            restore_sync_suspended: Arc::new(AtomicBool::new(false)),
            shutdown_phase: Arc::new(AtomicU8::new(SHUTDOWN_IDLE)),
            shutdown_snapshot: Mutex::new(None),
            sequence: AtomicU64::new(0),
            log_capacity,
        }
    }

    pub fn launch<R: Runtime>(
        &self,
        reservation: &ProfileReservation,
        profile: &LaunchProfile,
        session_id: String,
        app: AppHandle<R>,
    ) -> AppResult<ManagedInfo> {
        self.verify_profile_reservation(reservation, &profile.id)?;
        if self.info(&profile.id).is_some() {
            return Err(invalid("Profile is already running"));
        }
        if !std::path::Path::new(&profile.cwd).is_dir() {
            return Err(invalid("Launch working directory no longer exists"));
        }

        let mut command = Command::new(&profile.program);
        command
            .args(&profile.args)
            .current_dir(&profile.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_child(&mut command);

        let mut child = command.spawn().map_err(|error| {
            invalid(format!(
                "Failed to start '{}' in '{}': {error}",
                profile.program, profile.cwd
            ))
        })?;
        let pid = child.id();
        let tree = match OwnedProcessTree::attach(&child) {
            Ok(tree) => Arc::new(tree),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        if let Err(error) = resume_child(pid) {
            let _ = tree.terminate();
            let _ = child.wait();
            return Err(error);
        }
        let info = ManagedInfo {
            profile_id: profile.id.clone(),
            project_id: profile.project_id.clone(),
            pid,
            session_id,
            sequence: self.sequence.fetch_add(1, Ordering::SeqCst),
            tree,
        };
        self.managed
            .lock()
            .expect("managed process mutex poisoned")
            .insert(profile.id.clone(), info.clone());
        self.exit_intents
            .lock()
            .expect("exit intent mutex poisoned")
            .remove(&profile.id);
        self.unexpected_exits
            .lock()
            .expect("unexpected exit mutex poisoned")
            .remove(&profile.id);
        let log_context = LogCaptureContext {
            profile_id: profile.id.clone(),
            logs: self.logs.clone(),
            capacity: self.log_capacity,
            app: app.clone(),
        };

        let mut log_threads = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            log_threads.push(capture_stream(
                stdout,
                LogStream::Stdout,
                log_context.clone(),
            ));
        }
        if let Some(stderr) = child.stderr.take() {
            log_threads.push(capture_stream(stderr, LogStream::Stderr, log_context));
        }
        watch_child(
            child,
            info.clone(),
            log_threads,
            ProcessWatchContext {
                managed: self.managed.clone(),
                statuses: self.statuses.clone(),
                exit_intents: self.exit_intents.clone(),
                unexpected_exits: self.unexpected_exits.clone(),
                logs: self.logs.clone(),
                capacity: self.log_capacity,
                restore_sync_suspended: self.restore_sync_suspended.clone(),
                app,
            },
        );
        Ok(info)
    }

    pub fn stop(&self, reservation: &ProfileReservation, profile_id: &str) -> AppResult<()> {
        self.stop_with_intent(reservation, profile_id, ExitIntent::UserStop)
    }

    pub fn stop_after_failed_start(
        &self,
        reservation: &ProfileReservation,
        profile_id: &str,
    ) -> AppResult<()> {
        self.stop_with_intent(reservation, profile_id, ExitIntent::StartupFailure)
    }

    fn stop_with_intent(
        &self,
        reservation: &ProfileReservation,
        profile_id: &str,
        intent: ExitIntent,
    ) -> AppResult<()> {
        self.verify_profile_reservation(reservation, profile_id)?;
        let info = self
            .info(profile_id)
            .ok_or_else(|| invalid("Profile is not running"))?;
        self.exit_intents
            .lock()
            .expect("exit intent mutex poisoned")
            .insert(profile_id.to_owned(), intent);
        if let Err(error) = info.tree.terminate() {
            self.exit_intents
                .lock()
                .expect("exit intent mutex poisoned")
                .remove(profile_id);
            return Err(error);
        }
        Ok(())
    }

    pub fn stop_all_and_wait(
        &self,
        reservation: &ShutdownReservation,
        timeout: std::time::Duration,
    ) -> AppResult<()> {
        self.stop_all_with_intent(reservation, timeout, ExitIntent::Shutdown)
    }

    pub fn stop_all_by_user_and_wait(
        &self,
        reservation: &ShutdownReservation,
        timeout: std::time::Duration,
    ) -> AppResult<()> {
        self.stop_all_with_intent(reservation, timeout, ExitIntent::UserStop)
    }

    fn stop_all_with_intent(
        &self,
        reservation: &ShutdownReservation,
        timeout: std::time::Duration,
        intent: ExitIntent,
    ) -> AppResult<()> {
        self.verify_shutdown_reservation(reservation)?;
        let infos = self.active_infos();
        let mut failures = Vec::new();
        let mut failed_profiles = HashSet::new();
        for info in &infos {
            self.exit_intents
                .lock()
                .expect("exit intent mutex poisoned")
                .insert(info.profile_id.clone(), intent);
            if let Err(error) = info.tree.terminate() {
                self.exit_intents
                    .lock()
                    .expect("exit intent mutex poisoned")
                    .remove(&info.profile_id);
                failed_profiles.insert(info.profile_id.clone());
                failures.push(format!("{}: {error}", info.profile_id));
            }
        }

        let deadline = std::time::Instant::now() + timeout;
        while self
            .managed
            .lock()
            .expect("managed process mutex poisoned")
            .keys()
            .any(|profile_id| !failed_profiles.contains(profile_id))
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let timed_out = self
            .managed
            .lock()
            .expect("managed process mutex poisoned")
            .keys()
            .any(|profile_id| !failed_profiles.contains(profile_id));
        if timed_out {
            failures.push("timed out waiting for managed process sessions to finish".into());
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(invalid(format!(
                "Could not stop all managed process trees: {}",
                failures.join("; ")
            )))
        }
    }

    pub fn active_profile_ids(&self) -> Vec<String> {
        self.active_infos()
            .into_iter()
            .map(|info| info.profile_id)
            .collect()
    }

    pub fn reserve(&self, profile_id: &str) -> AppResult<ProfileReservation> {
        let mut reservations = self
            .lifecycle_reservations
            .lock()
            .expect("lifecycle reservation mutex poisoned");
        if reservations.shutdown {
            return Err(invalid("Application shutdown is already in progress"));
        }
        if !reservations.profiles.insert(profile_id.to_owned()) {
            return Err(invalid(
                "Another lifecycle operation is already in progress",
            ));
        }
        Ok(ProfileReservation {
            profile_id: profile_id.to_owned(),
            reservations: self.lifecycle_reservations.clone(),
        })
    }

    pub fn reserve_many(&self, profile_ids: &[String]) -> AppResult<Vec<ProfileReservation>> {
        let profile_ids: HashSet<_> = profile_ids.iter().cloned().collect();
        let mut reservations = self
            .lifecycle_reservations
            .lock()
            .expect("lifecycle reservation mutex poisoned");
        if let Some(profile_id) = profile_ids
            .iter()
            .find(|profile_id| reservations.profiles.contains(*profile_id))
        {
            return Err(invalid(format!(
                "Another lifecycle operation is already in progress for profile {profile_id}"
            )));
        }
        if reservations.shutdown {
            return Err(invalid("Application shutdown is already in progress"));
        }
        reservations.profiles.extend(profile_ids.iter().cloned());
        drop(reservations);
        Ok(profile_ids
            .into_iter()
            .map(|profile_id| ProfileReservation {
                profile_id,
                reservations: self.lifecycle_reservations.clone(),
            })
            .collect())
    }

    pub fn reserve_shutdown(&self) -> AppResult<ShutdownReservation> {
        let mut reservations = self
            .lifecycle_reservations
            .lock()
            .expect("lifecycle reservation mutex poisoned");
        if reservations.shutdown || !reservations.profiles.is_empty() {
            return Err(invalid(
                "Wait for current project operations to finish before exiting",
            ));
        }
        if self.shutdown_phase.load(Ordering::SeqCst) == SHUTDOWN_COMPLETE {
            return Err(invalid("Application shutdown is already complete"));
        }
        reservations.shutdown = true;
        self.shutdown_phase.store(SHUTDOWN_ACTIVE, Ordering::SeqCst);
        self.restore_sync_suspended.store(true, Ordering::SeqCst);
        Ok(ShutdownReservation {
            reservations: self.lifecycle_reservations.clone(),
            shutdown_phase: self.shutdown_phase.clone(),
            restore_sync_suspended: self.restore_sync_suspended.clone(),
        })
    }

    pub fn info(&self, profile_id: &str) -> Option<ManagedInfo> {
        self.managed
            .lock()
            .expect("managed process mutex poisoned")
            .get(profile_id)
            .cloned()
    }

    pub fn owns_pid(&self, pid: u32, system: &System) -> Option<ManagedInfo> {
        self.active_infos().into_iter().find(|info| {
            info.pid == pid || is_descendant(pid, info.pid, |child| parent_pid(system, child))
        })
    }

    pub fn logs(&self, profile_id: &str) -> Vec<RunLogEvent> {
        self.logs
            .lock()
            .expect("log mutex poisoned")
            .get(profile_id)
            .map(|logs| logs.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn clear_logs(&self, profile_id: &str) {
        self.logs
            .lock()
            .expect("log mutex poisoned")
            .remove(profile_id);
    }

    pub fn set_status(&self, profile_id: &str, status: RunStatus) {
        self.statuses
            .lock()
            .expect("run status mutex poisoned")
            .insert(profile_id.to_owned(), status);
    }

    pub fn status(&self, profile_id: &str) -> Option<RunStatus> {
        self.statuses
            .lock()
            .expect("run status mutex poisoned")
            .get(profile_id)
            .copied()
    }

    pub fn is_unexpected_exit(&self, profile_id: &str) -> bool {
        self.unexpected_exits
            .lock()
            .expect("unexpected exit mutex poisoned")
            .contains(profile_id)
    }

    pub fn shutdown_is_complete(&self) -> bool {
        self.shutdown_phase.load(Ordering::SeqCst) == SHUTDOWN_COMPLETE
    }

    pub fn shutdown_is_in_progress(&self) -> bool {
        self.shutdown_phase.load(Ordering::SeqCst) == SHUTDOWN_ACTIVE
    }

    pub fn shutdown_restore_snapshot(&self, active: Vec<String>) -> Vec<String> {
        let mut snapshot = self
            .shutdown_snapshot
            .lock()
            .expect("shutdown snapshot mutex poisoned");
        snapshot.get_or_insert(active).clone()
    }

    pub fn clear_shutdown_snapshot(&self) {
        self.shutdown_snapshot
            .lock()
            .expect("shutdown snapshot mutex poisoned")
            .take();
    }

    pub fn complete_shutdown(&self, reservation: &ShutdownReservation) -> AppResult<()> {
        self.verify_shutdown_reservation(reservation)?;
        self.shutdown_phase
            .store(SHUTDOWN_COMPLETE, Ordering::SeqCst);
        Ok(())
    }

    pub fn clear_profile(
        &self,
        reservations: &[ProfileReservation],
        profile_id: &str,
    ) -> AppResult<()> {
        if !reservations
            .iter()
            .any(|reservation| self.profile_reservation_matches(reservation, profile_id))
        {
            return Err(invalid(format!(
                "A lifecycle reservation is required for profile {profile_id}"
            )));
        }
        self.clear_logs(profile_id);
        self.statuses
            .lock()
            .expect("run status mutex poisoned")
            .remove(profile_id);
        self.exit_intents
            .lock()
            .expect("exit intent mutex poisoned")
            .remove(profile_id);
        self.unexpected_exits
            .lock()
            .expect("unexpected exit mutex poisoned")
            .remove(profile_id);
        Ok(())
    }

    fn active_infos(&self) -> Vec<ManagedInfo> {
        let mut infos: Vec<_> = self
            .managed
            .lock()
            .expect("managed process mutex poisoned")
            .values()
            .cloned()
            .collect();
        infos.sort_by_key(|info| info.sequence);
        infos
    }

    fn verify_profile_reservation(
        &self,
        reservation: &ProfileReservation,
        profile_id: &str,
    ) -> AppResult<()> {
        if self.profile_reservation_matches(reservation, profile_id) {
            Ok(())
        } else {
            Err(invalid(format!(
                "A lifecycle reservation is required for profile {profile_id}"
            )))
        }
    }

    fn profile_reservation_matches(
        &self,
        reservation: &ProfileReservation,
        profile_id: &str,
    ) -> bool {
        reservation.profile_id == profile_id
            && Arc::ptr_eq(&reservation.reservations, &self.lifecycle_reservations)
    }

    fn verify_shutdown_reservation(&self, reservation: &ShutdownReservation) -> AppResult<()> {
        if Arc::ptr_eq(&reservation.reservations, &self.lifecycle_reservations) {
            Ok(())
        } else {
            Err(invalid("A global lifecycle reservation is required"))
        }
    }
}

fn watch_child<R: Runtime>(
    mut child: Child,
    info: ManagedInfo,
    log_threads: Vec<std::thread::JoinHandle<()>>,
    context: ProcessWatchContext<R>,
) {
    std::thread::spawn(move || {
        let result = child.wait();
        for thread in log_threads {
            let _ = thread.join();
        }
        finalize_current_entry(
            &context.managed,
            &info.profile_id,
            |current| same_managed_instance(current, info.pid, &info.session_id),
            |managed| {
                let intent = context
                    .exit_intents
                    .lock()
                    .expect("exit intent mutex poisoned")
                    .remove(&info.profile_id);
                let (exit_code, status, unexpected, message) = match (intent, result) {
                    (Some(ExitIntent::UserStop), Ok(status)) => (
                        status.code(),
                        RunStatus::Idle,
                        false,
                        Some("Stopped by user".into()),
                    ),
                    (Some(ExitIntent::Shutdown), Ok(status)) => (
                        status.code(),
                        RunStatus::Idle,
                        false,
                        Some("Stopped during application shutdown".into()),
                    ),
                    (Some(ExitIntent::StartupFailure), Ok(status)) => (
                        status.code(),
                        RunStatus::Exited,
                        false,
                        Some("Stopped because startup did not become ready".into()),
                    ),
                    (_, Ok(status)) if status.success() => (
                        status.code(),
                        RunStatus::Exited,
                        false,
                        Some("Process exited normally".into()),
                    ),
                    (_, Ok(status)) => (
                        status.code(),
                        RunStatus::Exited,
                        true,
                        Some(format!("Process exited unexpectedly with {status}")),
                    ),
                    (_, Err(error)) => (
                        None,
                        RunStatus::Unknown,
                        true,
                        Some(format!("Could not wait for process: {error}")),
                    ),
                };
                context
                    .statuses
                    .lock()
                    .expect("run status mutex poisoned")
                    .insert(info.profile_id.clone(), status);
                let mut unexpected_profiles = context
                    .unexpected_exits
                    .lock()
                    .expect("unexpected exit mutex poisoned");
                if unexpected {
                    unexpected_profiles.insert(info.profile_id.clone());
                } else {
                    unexpected_profiles.remove(&info.profile_id);
                }
                drop(unexpected_profiles);
                if let Some(state) = context.app.try_state::<AppState>() {
                    if let Err(error) = state.storage.finish_session(&info.session_id, exit_code) {
                        emit_lifecycle_error(
                            &context.app,
                            &info.profile_id,
                            format!("Could not persist the completed run session: {error}"),
                        );
                    }
                    if !context.restore_sync_suspended.load(Ordering::SeqCst) {
                        let mut active: Vec<_> = managed
                            .values()
                            .filter(|current| {
                                !same_managed_instance(current, info.pid, &info.session_id)
                            })
                            .cloned()
                            .collect();
                        active.sort_by_key(|current| current.sequence);
                        let active: Vec<_> = active
                            .into_iter()
                            .map(|current| current.profile_id)
                            .collect();
                        if let Err(error) = state.storage.save_restore_set(&active) {
                            emit_lifecycle_error(
                                &context.app,
                                &info.profile_id,
                                format!(
                                    "Could not update the restore set after process exit: {error}"
                                ),
                            );
                        }
                    }
                }
                let log = RunLogEvent {
                    profile_id: info.profile_id.clone(),
                    stream: LogStream::System,
                    line: message.clone().unwrap_or_else(|| "Process exited".into()),
                    timestamp: now_ms(),
                };
                push_log(&context.logs, context.capacity, log.clone());
                let _ = context.app.emit("run-log", &log);
                let _ = context.app.emit(
                    "run-status",
                    process_exit_status_event(info.profile_id.clone(), status, unexpected, message),
                );
            },
        );
    });
}

fn finalize_current_entry<K, V>(
    entries: &Mutex<HashMap<K, V>>,
    key: &K,
    is_current: impl FnOnce(&V) -> bool,
    finalize: impl FnOnce(&HashMap<K, V>),
) -> bool
where
    K: Eq + Hash,
{
    let mut entries = entries.lock().expect("managed process mutex poisoned");
    if !entries.get(key).is_some_and(is_current) {
        return false;
    }
    finalize(&entries);
    entries.remove(key);
    true
}

fn process_exit_status_event(
    profile_id: String,
    status: RunStatus,
    unexpected: bool,
    message: Option<String>,
) -> RunStatusEvent {
    RunStatusEvent {
        profile_id,
        status,
        pid: None,
        message,
        unexpected,
        timestamp: now_ms(),
    }
}

struct ProcessWatchContext<R: Runtime> {
    managed: Arc<Mutex<HashMap<String, ManagedInfo>>>,
    statuses: Arc<Mutex<HashMap<String, RunStatus>>>,
    exit_intents: Arc<Mutex<HashMap<String, ExitIntent>>>,
    unexpected_exits: Arc<Mutex<HashSet<String>>>,
    logs: Arc<Mutex<HashMap<String, VecDeque<RunLogEvent>>>>,
    capacity: usize,
    restore_sync_suspended: Arc<AtomicBool>,
    app: AppHandle<R>,
}

fn emit_lifecycle_error<R: Runtime>(app: &AppHandle<R>, profile_id: &str, message: String) {
    let _ = app.emit(
        "process-lifecycle-error",
        RunStatusEvent {
            profile_id: profile_id.into(),
            status: RunStatus::Unknown,
            pid: None,
            message: Some(message),
            unexpected: true,
            timestamp: now_ms(),
        },
    );
}

fn same_managed_instance(current: &ManagedInfo, pid: u32, session_id: &str) -> bool {
    current.pid == pid && current.session_id == session_id
}

#[must_use = "the profile reservation must be held for the complete lifecycle operation"]
pub struct ProfileReservation {
    profile_id: String,
    reservations: Arc<Mutex<LifecycleReservations>>,
}

impl Drop for ProfileReservation {
    fn drop(&mut self) {
        self.reservations
            .lock()
            .expect("lifecycle reservation mutex poisoned")
            .profiles
            .remove(&self.profile_id);
    }
}

#[derive(Default)]
struct LifecycleReservations {
    profiles: HashSet<String>,
    shutdown: bool,
}

#[must_use = "the shutdown reservation must be held until process cleanup completes"]
pub struct ShutdownReservation {
    reservations: Arc<Mutex<LifecycleReservations>>,
    shutdown_phase: Arc<AtomicU8>,
    restore_sync_suspended: Arc<AtomicBool>,
}

impl Drop for ShutdownReservation {
    fn drop(&mut self) {
        if self.shutdown_phase.load(Ordering::SeqCst) != SHUTDOWN_COMPLETE {
            self.shutdown_phase.store(SHUTDOWN_IDLE, Ordering::SeqCst);
            self.restore_sync_suspended.store(false, Ordering::SeqCst);
            self.reservations
                .lock()
                .expect("lifecycle reservation mutex poisoned")
                .shutdown = false;
        }
    }
}

#[derive(Clone, Copy)]
enum ExitIntent {
    UserStop,
    Shutdown,
    StartupFailure,
}

struct LogCaptureContext<R: Runtime> {
    profile_id: String,
    logs: Arc<Mutex<HashMap<String, VecDeque<RunLogEvent>>>>,
    capacity: usize,
    app: AppHandle<R>,
}

impl<R: Runtime> Clone for LogCaptureContext<R> {
    fn clone(&self) -> Self {
        Self {
            profile_id: self.profile_id.clone(),
            logs: self.logs.clone(),
            capacity: self.capacity,
            app: self.app.clone(),
        }
    }
}

fn capture_stream<S: Read + Send + 'static, R: Runtime>(
    stream: S,
    kind: LogStream,
    context: LogCaptureContext<R>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        loop {
            let mut bytes = Vec::with_capacity(LOG_LINE_BYTE_LIMIT);
            let mut truncated = false;
            let mut has_data = false;
            let terminated = loop {
                let available = match reader.fill_buf() {
                    Ok([]) => break false,
                    Ok(available) => available,
                    Err(error) => {
                        let event = RunLogEvent {
                            profile_id: context.profile_id.clone(),
                            stream: kind,
                            line: format!("[log read error: {error}]"),
                            timestamp: now_ms(),
                        };
                        push_log(&context.logs, context.capacity, event.clone());
                        let _ = context.app.emit("run-log", &event);
                        return;
                    }
                };
                let newline = available.iter().position(|byte| *byte == b'\n');
                let line_bytes = newline.unwrap_or(available.len());
                has_data |= line_bytes > 0;
                let available_capacity = LOG_LINE_BYTE_LIMIT.saturating_sub(bytes.len());
                let captured = line_bytes.min(available_capacity);
                bytes.extend_from_slice(&available[..captured]);
                truncated |= captured < line_bytes;
                reader.consume(line_bytes + usize::from(newline.is_some()));
                if newline.is_some() {
                    break true;
                }
            };

            if !has_data && !terminated {
                break;
            }
            if terminated && !truncated && bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            let line = render_log_line(&bytes, truncated);
            let event = RunLogEvent {
                profile_id: context.profile_id.clone(),
                stream: kind,
                line,
                timestamp: now_ms(),
            };
            push_log(&context.logs, context.capacity, event.clone());
            let _ = context.app.emit("run-log", &event);
        }
    })
}

fn render_log_line(bytes: &[u8], truncated: bool) -> String {
    let decoded = String::from_utf8_lossy(bytes);
    if !truncated && decoded.len() <= LOG_LINE_BYTE_LIMIT {
        return decoded.into_owned();
    }

    let prefix_limit = LOG_LINE_BYTE_LIMIT - LOG_LINE_TRUNCATED_MARKER.len();
    let mut prefix_end = decoded.len().min(prefix_limit);
    while !decoded.is_char_boundary(prefix_end) {
        prefix_end -= 1;
    }
    let mut line = String::with_capacity(LOG_LINE_BYTE_LIMIT);
    line.push_str(&decoded[..prefix_end]);
    line.push_str(LOG_LINE_TRUNCATED_MARKER);
    line
}

fn push_log(
    logs: &Mutex<HashMap<String, VecDeque<RunLogEvent>>>,
    capacity: usize,
    event: RunLogEvent,
) {
    let mut logs = logs.lock().expect("log mutex poisoned");
    let profile_logs = logs.entry(event.profile_id.clone()).or_default();
    profile_logs.push_back(event);
    while profile_logs.len() > capacity {
        profile_logs.pop_front();
    }
    while logs.values().map(VecDeque::len).sum::<usize>() > capacity {
        let oldest_profile = logs
            .iter()
            .filter_map(|(profile_id, events)| {
                events.front().map(|event| (profile_id, event.timestamp))
            })
            .min_by(|(left_id, left_time), (right_id, right_time)| {
                left_time
                    .cmp(right_time)
                    .then_with(|| left_id.cmp(right_id))
            })
            .map(|(profile_id, _)| profile_id.clone());
        let Some(profile_id) = oldest_profile else {
            break;
        };
        let remove_profile = logs.get_mut(&profile_id).is_some_and(|events| {
            events.pop_front();
            events.is_empty()
        });
        if remove_profile {
            logs.remove(&profile_id);
        }
    }
}

fn parent_pid(system: &System, pid: u32) -> Option<u32> {
    system
        .process(Pid::from_u32(pid))
        .and_then(|process| process.parent())
        .map(Pid::as_u32)
}

pub fn is_descendant<F>(mut pid: u32, ancestor: u32, mut parent: F) -> bool
where
    F: FnMut(u32) -> Option<u32>,
{
    let mut visited = HashSet::new();
    while let Some(next) = parent(pid) {
        if next == ancestor {
            return true;
        }
        if next == 0 || !visited.insert(next) {
            return false;
        }
        pid = next;
    }
    false
}

#[cfg(windows)]
fn configure_child(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED};
    command.creation_flags(CREATE_NEW_PROCESS_GROUP.0 | CREATE_SUSPENDED.0);
}

#[cfg(not(windows))]
fn configure_child(_command: &mut Command) {}

#[cfg(windows)]
fn resume_child(pid: u32) -> AppResult<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }
        .map_err(|error| invalid(format!("Could not inspect suspended process: {error}")))?;
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut resumed = false;
    if unsafe { Thread32First(snapshot, &mut entry) }.is_ok() {
        loop {
            if entry.th32OwnerProcessID == pid {
                if let Ok(thread) =
                    unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID) }
                {
                    resumed = unsafe { ResumeThread(thread) } != u32::MAX;
                    let _ = unsafe { CloseHandle(thread) };
                    if resumed {
                        break;
                    }
                }
            }
            if unsafe { Thread32Next(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }
    let _ = unsafe { CloseHandle(snapshot) };
    if resumed {
        Ok(())
    } else {
        Err(invalid(
            "Could not resume the managed process after Job assignment",
        ))
    }
}

#[cfg(not(windows))]
fn resume_child(_pid: u32) -> AppResult<()> {
    Ok(())
}

#[cfg(windows)]
struct OwnedProcessTree(usize);

#[cfg(windows)]
impl OwnedProcessTree {
    fn attach(child: &Child) -> AppResult<Self> {
        use std::mem::size_of;
        use std::os::windows::io::AsRawHandle;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|error| invalid(format!("Could not create process job: {error}")))?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Err(error) = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } {
            let _ = unsafe { CloseHandle(job) };
            return Err(invalid(format!("Could not configure process job: {error}")));
        }
        let process = HANDLE(child.as_raw_handle());
        if let Err(error) = unsafe { AssignProcessToJobObject(job, process) } {
            let _ = unsafe { CloseHandle(job) };
            return Err(invalid(format!("Could not assign process to job: {error}")));
        }
        Ok(Self(job.0 as usize))
    }

    fn terminate(&self) -> AppResult<()> {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::TerminateJobObject;
        unsafe { TerminateJobObject(HANDLE(self.0 as *mut _), 1) }
            .map_err(|error| invalid(format!("Could not terminate process tree: {error}")))
    }
}

#[cfg(windows)]
impl Drop for OwnedProcessTree {
    fn drop(&mut self) {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        let _ = unsafe { CloseHandle(HANDLE(self.0 as *mut _)) };
    }
}

#[cfg(not(windows))]
struct OwnedProcessTree(u32);

#[cfg(not(windows))]
impl OwnedProcessTree {
    fn attach(child: &Child) -> AppResult<Self> {
        Ok(Self(child.id()))
    }

    fn terminate(&self) -> AppResult<()> {
        let status = Command::new("kill")
            .args(["-TERM", &self.0.to_string()])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(invalid("Could not terminate owned process"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descendant_walk_handles_chains_and_cycles() {
        let parents = HashMap::from([(30, 20), (20, 10), (50, 60), (60, 50)]);
        assert!(is_descendant(30, 10, |pid| parents.get(&pid).copied()));
        assert!(!is_descendant(30, 99, |pid| parents.get(&pid).copied()));
        assert!(!is_descendant(50, 10, |pid| parents.get(&pid).copied()));
    }

    #[test]
    fn exited_status_event_does_not_expose_stale_pid() {
        let event = process_exit_status_event(
            "profile".into(),
            RunStatus::Exited,
            false,
            Some("Process exited normally".into()),
        );

        assert_eq!(event.profile_id, "profile");
        assert_eq!(event.status, RunStatus::Exited);
        assert_eq!(event.pid, None);
        assert_eq!(event.message.as_deref(), Some("Process exited normally"));
        assert!(!event.unexpected);
    }

    #[test]
    fn oversized_unterminated_log_line_is_truncated_to_the_byte_limit() {
        const EXPECTED_LINE_LIMIT: usize = 16 * 1024;
        const EXPECTED_MARKER: &str = " [RunCove: log line truncated]";

        let app = tauri::test::mock_app();
        let logs = Arc::new(Mutex::new(HashMap::new()));
        let capture = capture_stream(
            std::io::Cursor::new(vec![b'x'; EXPECTED_LINE_LIMIT * 4]),
            LogStream::Stdout,
            LogCaptureContext {
                profile_id: "profile".into(),
                logs: logs.clone(),
                capacity: 10,
                app: app.handle().clone(),
            },
        );

        capture.join().unwrap();
        let logs = logs.lock().unwrap();
        let line = &logs["profile"][0].line;
        assert!(line.len() <= EXPECTED_LINE_LIMIT);
        assert!(line.ends_with(EXPECTED_MARKER));
    }

    #[test]
    fn unterminated_log_tail_is_preserved_when_it_fits() {
        let app = tauri::test::mock_app();
        let logs = Arc::new(Mutex::new(HashMap::new()));
        let capture = capture_stream(
            std::io::Cursor::new(b"tail without newline"),
            LogStream::Stderr,
            LogCaptureContext {
                profile_id: "profile".into(),
                logs: logs.clone(),
                capacity: 10,
                app: app.handle().clone(),
            },
        );

        capture.join().unwrap();
        let logs = logs.lock().unwrap();
        assert_eq!(logs["profile"][0].line, "tail without newline");
    }

    #[test]
    fn log_capacity_bounds_the_complete_in_memory_session() {
        let logs = Mutex::new(HashMap::new());
        for (profile_id, timestamp) in [("alpha", 1), ("beta", 2), ("alpha", 3), ("beta", 4)] {
            push_log(
                &logs,
                3,
                RunLogEvent {
                    profile_id: profile_id.into(),
                    stream: LogStream::Stdout,
                    line: format!("line-{timestamp}"),
                    timestamp,
                },
            );
        }

        let logs = logs.lock().unwrap();
        assert_eq!(logs.values().map(VecDeque::len).sum::<usize>(), 3);
        assert!(!logs.values().flatten().any(|event| event.timestamp == 1));
    }

    #[test]
    fn current_entry_stays_visible_until_exit_commit_finishes() {
        use std::sync::{mpsc, Arc, Barrier};
        use std::time::Duration;

        let key = "profile".to_owned();
        let entries = Arc::new(Mutex::new(HashMap::from([(key.clone(), 1_u64)])));
        let status = Arc::new(Mutex::new(RunStatus::Running));
        let commit_started = Arc::new(Barrier::new(2));
        let release_commit = Arc::new(Barrier::new(2));

        let finalizer = {
            let entries = entries.clone();
            let status = status.clone();
            let key = key.clone();
            let commit_started = commit_started.clone();
            let release_commit = release_commit.clone();
            std::thread::spawn(move || {
                assert!(finalize_current_entry(
                    &entries,
                    &key,
                    |generation| *generation == 1,
                    |_| {
                        *status.lock().unwrap() = RunStatus::Exited;
                        commit_started.wait();
                        release_commit.wait();
                    },
                ));
            })
        };

        commit_started.wait();
        let (restarted, restarted_rx) = mpsc::channel();
        let restart = {
            let entries = entries.clone();
            let status = status.clone();
            let key = key.clone();
            std::thread::spawn(move || {
                let mut entries = entries.lock().unwrap();
                assert!(!entries.contains_key(&key));
                *status.lock().unwrap() = RunStatus::Starting;
                entries.insert(key, 2);
                restarted.send(()).unwrap();
            })
        };

        assert!(restarted_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err());
        release_commit.wait();
        finalizer.join().unwrap();
        restart.join().unwrap();
        restarted_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        assert_eq!(*status.lock().unwrap(), RunStatus::Starting);
        assert_eq!(entries.lock().unwrap().get(&key), Some(&2));
    }

    #[test]
    fn stale_generation_cannot_commit_or_remove_the_current_entry() {
        let key = "profile".to_owned();
        let entries = Mutex::new(HashMap::from([(key.clone(), 2_u64)]));
        let committed = std::cell::Cell::new(false);

        let finalized = finalize_current_entry(
            &entries,
            &key,
            |generation| *generation == 1,
            |_| committed.set(true),
        );

        assert!(!finalized);
        assert!(!committed.get());
        assert_eq!(entries.lock().unwrap().get(&key), Some(&2));
    }

    #[test]
    fn concurrent_profile_reservations_have_one_winner() {
        use std::sync::{mpsc, Arc, Barrier};
        use std::time::{Duration, Instant};

        let manager = Arc::new(ProcessManager::new(10));
        let barrier = Arc::new(Barrier::new(9));
        let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let mut threads = Vec::new();
        for _ in 0..8 {
            let manager = manager.clone();
            let barrier = barrier.clone();
            let release = release.clone();
            let sender = sender.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                if let Ok(_reservation) = manager.reserve("profile") {
                    sender.send(true).unwrap();
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while !release.load(Ordering::SeqCst) && Instant::now() < deadline {
                        std::thread::yield_now();
                    }
                } else {
                    sender.send(false).unwrap();
                }
            }));
        }
        drop(sender);
        barrier.wait();
        let results: Vec<_> = (0..8)
            .map(|_| receiver.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        release.store(true, Ordering::SeqCst);
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(results.iter().filter(|won| **won).count(), 1);
        assert!(manager.reserve("profile").is_ok());
    }

    #[test]
    fn reserve_many_is_all_or_nothing_and_deduplicates_ids() {
        let manager = ProcessManager::new(10);
        let first = manager.reserve("first").unwrap();
        assert!(manager
            .reserve_many(&["first".into(), "second".into()])
            .is_err());
        assert!(manager.reserve("second").is_ok());
        drop(first);

        let reservations = manager
            .reserve_many(&["first".into(), "second".into(), "first".into()])
            .unwrap();
        assert_eq!(reservations.len(), 2);
        assert!(manager.reserve("first").is_err());
        assert!(manager.reserve("second").is_err());
        drop(reservations);
        assert!(manager.reserve("first").is_ok());
        assert!(manager.reserve("second").is_ok());
    }

    #[test]
    fn shutdown_reservation_excludes_profile_operations() {
        let manager = ProcessManager::new(10);
        assert!(!manager.shutdown_is_in_progress());
        let profile = manager.reserve("profile").unwrap();
        assert!(manager.reserve_shutdown().is_err());
        drop(profile);
        let shutdown = manager.reserve_shutdown().unwrap();
        assert!(manager.shutdown_is_in_progress());
        assert!(manager.reserve("profile").is_err());
        assert!(manager.reserve_many(&["other".into()]).is_err());
        drop(shutdown);
        assert!(!manager.shutdown_is_in_progress());
        assert!(manager.reserve("profile").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn job_object_releases_port_owned_by_npm_process_tree() {
        use std::io::BufRead;
        use std::net::TcpStream;
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("package.json"),
            r#"{"private":true,"scripts":{"dev":"node server.js"}}"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("server.js"),
            "const net=require('net');const s=net.createServer();s.listen(0,'127.0.0.1',()=>console.log(`READY ${s.address().port}`));",
        )
        .unwrap();

        let mut command = Command::new("cmd.exe");
        command
            .args(["/D", "/S", "/C", "npm.cmd run dev"])
            .current_dir(temp.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_child(&mut command);
        let mut child = command.spawn().unwrap();
        let tree = OwnedProcessTree::attach(&child).unwrap();
        resume_child(child.id()).unwrap();

        let stdout = child.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(port) = line
                    .split_once("READY ")
                    .and_then(|(_, port)| port.trim().parse::<u16>().ok())
                {
                    let _ = sender.send(port);
                    break;
                }
            }
        });
        let port = receiver
            .recv_timeout(Duration::from_secs(15))
            .expect("npm fixture did not open its port");
        assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());

        tree.terminate().unwrap();
        child.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while TcpStream::connect(("127.0.0.1", port)).is_ok() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }
}
