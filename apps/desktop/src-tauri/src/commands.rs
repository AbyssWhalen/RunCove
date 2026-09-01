use crate::archive_service::ArchiveService;
use crate::discovery;
use crate::error::{invalid, AppError, AppResult};
use crate::import_observation;
use crate::models::{
    AppSettings, AssociationSource, CloseBehavior, ConfirmAssociationRequest, DashboardSnapshot,
    DiscoveredProject, ExternalProcessRequest, LanguagePreference, LaunchGroup, LaunchGroupInput,
    LaunchGroupStartResult, LaunchGroupStopFailure, LaunchGroupStopResult, PortAssociation,
    PortSnapshot, Project, ProjectInput, RelatedPort, RestoreResult, RunLogArchivePage,
    RunLogArchiveState, RunLogEvent, RunSession, RunStatus, RunStatusEvent, RunStatusReason,
};
use crate::state::AppState;
use crate::storage::now_ms;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
#[cfg(not(windows))]
use sysinfo::Pid;
use sysinfo::{ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

/// How long a start waits for a profile's expected ports before giving up.
const PROFILE_READY_TIMEOUT_SECS: u64 = 20;
const PROFILE_READY_TIMEOUT: Duration = Duration::from_secs(PROFILE_READY_TIMEOUT_SECS);

/// How long a group member waits for a lifecycle reservation another operation holds.
///
/// Deliberately longer than the readiness wait, and derived from it rather than written
/// as its own number: the operation being waited on may spend that whole budget waiting
/// for its own ports, so a waiter that gave up first would report a failure for a start
/// that had not failed yet.
const RESERVATION_HANDOFF_TIMEOUT: Duration = Duration::from_secs(PROFILE_READY_TIMEOUT_SECS + 5);

/// How long a stop waits for a profile's process tree to disappear.
const PROFILE_STOP_TIMEOUT: Duration = Duration::from_secs(8);

/// Polling interval for the waits that only read in-memory process state. The readiness
/// wait polls far slower on purpose, because each of its rounds costs a full port scan
/// and a process-table refresh.
const STATE_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[tauri::command]
pub fn get_dashboard_snapshot(state: State<'_, AppState>) -> AppResult<DashboardSnapshot> {
    state.dashboard()
}

#[tauri::command]
pub fn get_app_settings(state: State<'_, AppState>) -> AppResult<AppSettings> {
    state.storage.settings()
}

#[tauri::command]
pub fn set_close_behavior(
    close_behavior: String,
    state: State<'_, AppState>,
) -> AppResult<AppSettings> {
    let close_behavior = parse_close_behavior(&close_behavior)?;
    persist_close_behavior(state.storage.as_ref(), close_behavior)
}

#[tauri::command]
pub fn hide_to_tray(app: AppHandle) -> AppResult<()> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| invalid("Main window is unavailable"))?;
    window
        .hide()
        .map_err(|error| invalid(format!("Could not hide main window: {error}")))
}

#[tauri::command]
pub async fn request_elevated_monitoring(app: AppHandle) -> AppResult<()> {
    run_blocking(crate::privileges::launch_elevated_copy).await?;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        app.exit(0);
    });
    Ok(())
}

#[tauri::command]
pub fn set_language_preference(
    language_preference: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<AppSettings> {
    let language_preference = parse_language_preference(&language_preference)?;
    let (settings, tray_error) =
        persist_language_preference(state.storage.as_ref(), language_preference, |preference| {
            crate::refresh_tray_language(&app, preference)
        })?;
    if let Some(message) = tray_error {
        crate::emit_tray_language_update_error(&app, message);
    }
    Ok(settings)
}

fn persist_language_preference(
    storage: &crate::storage::Storage,
    language_preference: LanguagePreference,
    refresh_tray: impl FnOnce(LanguagePreference) -> AppResult<()>,
) -> AppResult<(AppSettings, Option<String>)> {
    let mut settings = storage.settings()?;
    settings.language_preference = language_preference;
    storage.save_settings(&settings)?;
    let tray_error = refresh_tray(language_preference)
        .err()
        .map(|error| error.to_string());
    Ok((settings, tray_error))
}

fn parse_language_preference(value: &str) -> AppResult<LanguagePreference> {
    LanguagePreference::parse(value).ok_or_else(|| {
        invalid(format!(
            "Unsupported language preference '{value}'; expected system, en, or zh-CN"
        ))
    })
}

fn persist_close_behavior(
    storage: &crate::storage::Storage,
    close_behavior: CloseBehavior,
) -> AppResult<AppSettings> {
    let mut settings = storage.settings()?;
    settings.close_behavior = close_behavior;
    storage.save_settings(&settings)?;
    Ok(settings)
}

fn parse_close_behavior(value: &str) -> AppResult<CloseBehavior> {
    CloseBehavior::parse(value).ok_or_else(|| {
        invalid(format!(
            "Unsupported close behavior '{value}'; expected ask, hideToTray, or quit"
        ))
    })
}

#[tauri::command]
pub async fn discover_project(directory: String) -> AppResult<DiscoveredProject> {
    run_blocking(move || {
        let mut project = discovery::discover(&directory)?;
        import_observation::overlay_local_runtime(std::slice::from_mut(&mut project));
        Ok(project)
    })
    .await
}

#[tauri::command]
pub async fn scan_development_root(
    directory: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<DiscoveredProject>> {
    scan_development_root_inner(directory, clone_app_state(&state)).await
}

#[tauri::command]
pub async fn scan_saved_development_root(
    state: State<'_, AppState>,
) -> AppResult<Vec<DiscoveredProject>> {
    let state = clone_app_state(&state);
    run_blocking(move || {
        let directory = saved_development_root(state.storage.as_ref())?;
        scan_development_root_sync(&directory, &state)
    })
    .await
}

async fn scan_development_root_inner(
    directory: String,
    state: AppState,
) -> AppResult<Vec<DiscoveredProject>> {
    run_blocking(move || scan_development_root_sync(&directory, &state)).await
}

fn scan_development_root_sync(
    directory: &str,
    state: &AppState,
) -> AppResult<Vec<DiscoveredProject>> {
    let mut projects = discovery::scan_development_root(directory)?;
    import_observation::overlay_local_runtime(&mut projects);
    state
        .storage
        .remember_development_root(Path::new(directory))?;
    Ok(projects)
}

fn saved_development_root(storage: &crate::storage::Storage) -> AppResult<String> {
    storage
        .settings()?
        .recent_development_root
        .ok_or_else(|| invalid("No development root has been saved yet"))
}

#[tauri::command]
pub fn save_project(project: ProjectInput, state: State<'_, AppState>) -> AppResult<Project> {
    if let Some(project_id) = project.id.as_deref() {
        if let Some(existing) = state.storage.get_project(project_id)? {
            let profile_ids = project_profile_ids(&existing);
            let _reservations = state.processes.reserve_many(&profile_ids)?;
            let current = state
                .storage
                .get_project(project_id)?
                .ok_or_else(|| invalid("Project no longer exists"))?;
            if project_profile_ids(&current) != profile_ids {
                return Err(invalid("Project profiles changed; reload before editing"));
            }
            if project_has_active_profile(&current, &state.processes.active_profile_ids()) {
                return Err(invalid(
                    "Stop every running profile in this project before editing it",
                ));
            }
            let saved = state.storage.save_project(project)?;
            let saved_profile_ids = project_profile_ids(&saved);
            for removed_profile_id in profile_ids
                .iter()
                .filter(|profile_id| !saved_profile_ids.contains(profile_id))
            {
                state
                    .processes
                    .clear_profile(&_reservations, removed_profile_id)?;
            }
            return Ok(saved);
        }
    }
    state.storage.save_project(project)
}

#[tauri::command]
pub fn delete_project(project_id: String, state: State<'_, AppState>) -> AppResult<()> {
    let project = state
        .storage
        .get_project(&project_id)?
        .ok_or_else(|| invalid("Project not found"))?;
    let profile_ids = project_profile_ids(&project);
    let _reservations = state.processes.reserve_many(&profile_ids)?;
    let project = state
        .storage
        .get_project(&project_id)?
        .ok_or_else(|| invalid("Project no longer exists"))?;
    if project_profile_ids(&project) != profile_ids {
        return Err(invalid("Project profiles changed; reload before deleting"));
    }
    if project_has_active_profile(&project, &state.processes.active_profile_ids()) {
        return Err(invalid(
            "Stop every running profile in this project before deleting it",
        ));
    }
    for profile in project.profiles {
        state.processes.clear_profile(&_reservations, &profile.id)?;
    }
    state.storage.delete_project(&project_id)
}

fn project_profile_ids(project: &Project) -> Vec<String> {
    project
        .profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect()
}

fn project_has_active_profile(project: &Project, active_profile_ids: &[String]) -> bool {
    project
        .profiles
        .iter()
        .any(|profile| active_profile_ids.contains(&profile.id))
}

#[tauri::command]
pub async fn start_profile(
    profile_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<RunStatusEvent> {
    crate::privileges::ensure_process_action_allowed()?;
    let state = clone_app_state(&state);
    run_blocking(move || start_profile_inner(&profile_id, &app, &state)).await
}

#[tauri::command]
pub async fn stop_profile(
    profile_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<RunStatusEvent> {
    crate::privileges::ensure_process_action_allowed()?;
    let state = clone_app_state(&state);
    run_blocking(move || stop_profile_inner(&profile_id, &app, &state)).await
}

fn stop_profile_inner<R: Runtime>(
    profile_id: &str,
    app: &AppHandle<R>,
    state: &AppState,
) -> AppResult<RunStatusEvent> {
    let reservation = state.processes.reserve(profile_id)?;
    stop_profile_inner_reserved(&reservation, profile_id, app, state)
}

fn stop_profile_inner_reserved<R: Runtime>(
    reservation: &crate::processes::ProfileReservation,
    profile_id: &str,
    app: &AppHandle<R>,
    state: &AppState,
) -> AppResult<RunStatusEvent> {
    state.processes.stop(reservation, profile_id)?;
    wait_for_profile_stopped(profile_id, state, PROFILE_STOP_TIMEOUT)?;
    sync_active_restore_set(state)?;
    let event = status_event_with_reason(
        profile_id.into(),
        RunStatus::Idle,
        None,
        RunStatusReason::StopRequested,
    );
    let _ = app.emit("run-status", &event);
    Ok(event)
}

#[tauri::command]
pub async fn restart_profile(
    profile_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<RunStatusEvent> {
    crate::privileges::ensure_process_action_allowed()?;
    let state = clone_app_state(&state);
    run_blocking(move || restart_profile_inner(&profile_id, &app, &state)).await
}

fn restart_profile_inner<R: Runtime>(
    profile_id: &str,
    app: &AppHandle<R>,
    state: &AppState,
) -> AppResult<RunStatusEvent> {
    let reservation = state.processes.reserve(profile_id)?;
    if state.processes.info(profile_id).is_some() {
        state.processes.stop(&reservation, profile_id)?;
        wait_for_profile_stopped(profile_id, state, PROFILE_STOP_TIMEOUT)?;
    }
    start_profile_inner_reserved(&reservation, profile_id, app, state)
}

#[tauri::command]
pub async fn restore_last_run_set(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<RestoreResult> {
    crate::privileges::ensure_process_action_allowed()?;
    let state = clone_app_state(&state);
    run_blocking(move || restore_last_run_set_inner(&app, &state)).await
}

fn restore_last_run_set_inner<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
) -> AppResult<RestoreResult> {
    let restore = state.storage.restore_set()?;
    Ok(restore_profiles(restore.profile_ids, |profile_id| {
        start_walk_member(profile_id, app, state)
    }))
}

fn restore_profiles(
    profile_ids: Vec<String>,
    mut restore: impl FnMut(&str) -> AppResult<()>,
) -> RestoreResult {
    let mut started = Vec::new();
    for profile_id in profile_ids {
        match restore(&profile_id) {
            Ok(_) => started.push(profile_id.clone()),
            Err(error) => {
                return RestoreResult {
                    started_profile_ids: started,
                    failed_profile_id: Some(profile_id),
                    error: Some(error.to_string()),
                    related_port: error.related_port().cloned(),
                };
            }
        }
    }
    RestoreResult {
        started_profile_ids: started,
        failed_profile_id: None,
        error: None,
        related_port: None,
    }
}

/// Start one profile of an ordered walk — a restore set or a launch group — waiting out
/// an operation that already holds it.
///
/// Two walks can overlap in membership: groups may share a member (a database two stacks
/// both need is the obvious case), and a restore set is whatever was running last, which
/// may be exactly a group's members. Reaching a profile another operation is in the
/// middle of is therefore ordinary, not a failure. Failing was the old behavior, and it
/// meant two overlapping walks could not be run close together at all.
///
/// Waiting keeps the walk's ordering promise, because the next entry still does not start
/// until this one has settled. An entry the other operation brought up costs one
/// `AlreadyRunning` event and nothing else; one it stopped is started here, which is what
/// this walk asked for.
fn start_walk_member<R: Runtime>(
    profile_id: &str,
    app: &AppHandle<R>,
    state: &AppState,
) -> AppResult<()> {
    let reservation = reserve_walk_member(profile_id, state)?;
    start_profile_inner_reserved(&reservation, profile_id, app, state)?;
    Ok(())
}

/// Take a profile's lifecycle reservation, waiting out an operation that holds it.
///
/// Polling, because a reservation is a `HashSet` entry with no condition variable to
/// wait on, and adding one to serve a wait that resolves in seconds would put a second
/// synchronization mechanism inside the lock that guards every lifecycle operation.
/// A shutdown is returned immediately rather than waited out: nothing starts or stops
/// during one, so the answer cannot change before the deadline.
fn reserve_walk_member(
    profile_id: &str,
    state: &AppState,
) -> AppResult<crate::processes::ProfileReservation> {
    let deadline = Instant::now() + RESERVATION_HANDOFF_TIMEOUT;
    loop {
        if let Some(reservation) = state.processes.try_reserve(profile_id)? {
            return Ok(reservation);
        }
        if Instant::now() >= deadline {
            return Err(invalid(
                "Another lifecycle operation is still in progress for this profile",
            ));
        }
        std::thread::sleep(STATE_POLL_INTERVAL);
    }
}

/// Writing a launch group takes no process reservation, unlike `save_project`. A
/// group holds no process state: changing its membership only changes which ids
/// the next start walks, so it is safe while its members are running.
#[tauri::command]
pub fn save_launch_group(
    group: LaunchGroupInput,
    state: State<'_, AppState>,
) -> AppResult<LaunchGroup> {
    state.storage.save_launch_group(group)
}

/// Deleting a group does not stop or otherwise touch its members. They stay
/// running and stay individually visible, which is why this needs no guard
/// against an active profile the way `delete_project` does.
#[tauri::command]
pub fn delete_launch_group(group_id: String, state: State<'_, AppState>) -> AppResult<()> {
    state.storage.delete_launch_group(&group_id)
}

#[tauri::command]
pub async fn start_launch_group(
    group_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<LaunchGroupStartResult> {
    crate::privileges::ensure_process_action_allowed()?;
    let state = clone_app_state(&state);
    run_blocking(move || start_launch_group_inner(&group_id, &app, &state)).await
}

/// Starting a group is `restore_profiles` over the group's members, deliberately
/// the same walk the restore set uses: ordered, and stopping at the first member
/// that fails while leaving the earlier ones running. Each member goes through
/// the full start path, so a member that is already running returns early and
/// counts as started — which is what makes starting a group idempotent.
fn start_launch_group_inner<R: Runtime>(
    group_id: &str,
    app: &AppHandle<R>,
    state: &AppState,
) -> AppResult<LaunchGroupStartResult> {
    let group = group_to_run(group_id, state)?;
    let restore = restore_profiles(group.profile_ids, |profile_id| {
        start_walk_member(profile_id, app, state)
    });
    Ok(LaunchGroupStartResult {
        group_id: group.id,
        started_profile_ids: restore.started_profile_ids,
        failed_profile_id: restore.failed_profile_id,
        error: restore.error,
        related_port: restore.related_port,
    })
}

#[tauri::command]
pub async fn stop_launch_group(
    group_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<LaunchGroupStopResult> {
    crate::privileges::ensure_process_action_allowed()?;
    let state = clone_app_state(&state);
    run_blocking(move || stop_launch_group_inner(&group_id, &app, &state)).await
}

fn stop_launch_group_inner<R: Runtime>(
    group_id: &str,
    app: &AppHandle<R>,
    state: &AppState,
) -> AppResult<LaunchGroupStopResult> {
    let group = group_to_run(group_id, state)?;
    let (stopped_profile_ids, failures) = stop_group_profiles(&group.profile_ids, |profile_id| {
        if state.processes.info(profile_id).is_none() {
            return Ok(false);
        }
        // Reserve the same way a group start does, and for the same reason: a member two
        // groups share can be held by the other one. The running check is repeated after
        // the wait because the operation we waited out may have been the stop this member
        // needed, and reporting that as this group's failure would be wrong.
        let reservation = reserve_walk_member(profile_id, state)?;
        if state.processes.info(profile_id).is_none() {
            return Ok(false);
        }
        stop_profile_inner_reserved(&reservation, profile_id, app, state)?;
        Ok(true)
    });
    Ok(LaunchGroupStopResult {
        group_id: group.id,
        stopped_profile_ids,
        failures,
    })
}

/// Stops a group's members in reverse launch order, so a dependent shuts down
/// before what it depends on. The closure reports `Ok(false)` for a member that
/// was not running, which is a no-op rather than a stop.
///
/// This does not give up at the first failure, and that asymmetry with
/// `restore_profiles` is deliberate: cutting a start short protects the user from
/// a half-built stack, while cutting a stop short would leave running exactly the
/// processes they asked to be rid of.
fn stop_group_profiles(
    profile_ids: &[String],
    mut stop: impl FnMut(&str) -> AppResult<bool>,
) -> (Vec<String>, Vec<LaunchGroupStopFailure>) {
    let mut stopped = Vec::new();
    let mut failures = Vec::new();
    for profile_id in profile_ids.iter().rev() {
        match stop(profile_id) {
            Ok(true) => stopped.push(profile_id.clone()),
            Ok(false) => {}
            Err(error) => failures.push(LaunchGroupStopFailure {
                profile_id: profile_id.clone(),
                error: error.to_string(),
            }),
        }
    }
    (stopped, failures)
}

/// Loads a group and refuses the empty one. Empty is reachable without a bad
/// request: a group's last member profile can be deleted out from under it, and
/// reporting that beats a success that started nothing.
fn group_to_run(group_id: &str, state: &AppState) -> AppResult<LaunchGroup> {
    let group = state
        .storage
        .launch_group(group_id)?
        .ok_or_else(|| invalid("Launch group not found"))?;
    if group.profile_ids.is_empty() {
        return Err(invalid("This launch group has no launch profiles"));
    }
    Ok(group)
}

#[tauri::command]
pub async fn terminate_external_process(
    request: ExternalProcessRequest,
    state: State<'_, AppState>,
) -> AppResult<()> {
    crate::privileges::ensure_process_action_allowed()?;
    let state = clone_app_state(&state);
    run_blocking(move || terminate_external_process_inner(&request, &state)).await
}

fn terminate_external_process_inner(
    request: &ExternalProcessRequest,
    state: &AppState,
) -> AppResult<()> {
    #[cfg(windows)]
    {
        terminate_external_windows(request, state)
    }
    #[cfg(not(windows))]
    {
        terminate_external_portable(request, state)
    }
}

#[cfg(not(windows))]
fn terminate_external_portable(
    request: &ExternalProcessRequest,
    state: &AppState,
) -> AppResult<()> {
    if request.pid == 0 || request.pid == std::process::id() {
        return Err(invalid("RunCove refuses to terminate this process"));
    }
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    if state.processes.owns_pid(request.pid, &system).is_some() {
        return Err(invalid(
            "Managed processes must be stopped through their launch profile",
        ));
    }
    let process = system
        .process(Pid::from_u32(request.pid))
        .ok_or_else(|| invalid("Process no longer exists"))?;
    let expected_started_at = request
        .started_at
        .ok_or_else(|| invalid("A verified process start time is required"))?;
    if process.start_time().saturating_mul(1_000) != expected_started_at {
        return Err(invalid(
            "Process identity changed; refresh before terminating it",
        ));
    }
    let current_executable = process
        .exe()
        .map(normalized_path)
        .ok_or_else(|| invalid("Executable path is unavailable; refusing to terminate process"))?;
    let expected_executable = request
        .executable_path
        .as_deref()
        .map(Path::new)
        .map(normalized_path)
        .ok_or_else(|| invalid("A verified executable path is required"))?;
    if current_executable != expected_executable {
        return Err(invalid(
            "Process executable changed; refresh before terminating it",
        ));
    }
    verify_requested_listener(request)?;
    terminate_external_tree(request.pid)
}

#[tauri::command]
pub async fn confirm_port_association(
    request: ConfirmAssociationRequest,
    state: State<'_, AppState>,
) -> AppResult<PortAssociation> {
    let state = clone_app_state(&state);
    run_blocking(move || confirm_port_association_inner(&request, &state)).await
}

fn confirm_port_association_inner(
    request: &ConfirmAssociationRequest,
    state: &AppState,
) -> AppResult<PortAssociation> {
    let dashboard = state.dashboard()?;
    if let Some(error) = dashboard.scan_error {
        return Err(invalid(format!(
            "Could not verify port association: {error}"
        )));
    }
    validate_confirm_association_request(request, &dashboard.ports)?;
    state.storage.confirm_association(
        &request.project_id,
        request.profile_id.as_deref(),
        request.port,
        &request.protocol,
    )
}

fn validate_confirm_association_request(
    request: &ConfirmAssociationRequest,
    ports: &[PortSnapshot],
) -> AppResult<()> {
    let port = ports.iter().find(|port| {
        port.active
            && port.state.eq_ignore_ascii_case("LISTEN")
            && port.port == request.port
            && port.protocol.eq_ignore_ascii_case(&request.protocol)
            && port.pid == Some(request.pid)
    });
    let Some(port) = port else {
        return Err(invalid(
            "Observed port identity changed; refresh before confirming association",
        ));
    };
    let same_process = port.process_started_at == Some(request.started_at)
        && port.executable_path.as_deref().is_some_and(|path| {
            normalized_path(Path::new(path)) == normalized_path(Path::new(&request.executable_path))
        });
    if !same_process {
        return Err(invalid(
            "Observed process identity changed; refresh before confirming association",
        ));
    }
    if port.association_source != Some(AssociationSource::Suggested)
        || port.project_id.as_deref() != Some(request.project_id.as_str())
        || port.profile_id != request.profile_id
    {
        return Err(invalid(
            "Suggested project association changed; refresh before confirming it",
        ));
    }
    Ok(())
}

#[tauri::command]
pub fn clear_logs(profile_id: String, state: State<'_, AppState>) -> AppResult<()> {
    state.processes.clear_logs(&profile_id);
    Ok(())
}

#[tauri::command]
pub fn get_logs(profile_id: String, state: State<'_, AppState>) -> AppResult<Vec<RunLogEvent>> {
    Ok(state.processes.logs(&profile_id))
}

#[tauri::command]
pub fn get_run_history(state: State<'_, AppState>) -> AppResult<Vec<RunSession>> {
    state.storage.list_sessions(200)
}

/// Turn run log archiving on or off for the sessions that start from now on.
///
/// The setting is persisted before it is applied, so a database that refuses the write
/// leaves the runtime and the stored value agreeing rather than archiving with nothing
/// to remember it. Turning it off closes the archives that are open — an already running
/// process keeps writing to the drawer and stops being written to disk — and turning it
/// on never backfills a running session, because a session's archive is opened at launch
/// and only there.
#[tauri::command]
pub async fn set_run_log_archiving(
    enabled: bool,
    state: State<'_, AppState>,
) -> AppResult<RunLogArchiveState> {
    let state = clone_app_state(&state);
    run_blocking(move || {
        persist_run_log_archiving(state.storage.as_ref(), state.processes.archive(), enabled)
    })
    .await
}

fn persist_run_log_archiving(
    storage: &crate::storage::Storage,
    archive: &ArchiveService,
    enabled: bool,
) -> AppResult<RunLogArchiveState> {
    let mut settings = storage.settings()?;
    settings.archive_run_logs = enabled;
    storage.save_settings(&settings)?;
    Ok(archive.set_enabled(enabled))
}

/// Read one page of an archived session, ending at `before_offset` and working backwards.
///
/// `before_offset` is the `page_start_offset` of the page the viewer already has, so the
/// first call passes `None` and gets the tail. The whole read happens on a blocking
/// thread and is bounded by both a record count and a byte cap, so a session that wrote
/// megabytes never crosses the IPC boundary in one message.
#[tauri::command]
pub async fn read_run_log_archive(
    session_id: String,
    before_offset: Option<u64>,
    max_lines: Option<usize>,
    state: State<'_, AppState>,
) -> AppResult<RunLogArchivePage> {
    let state = clone_app_state(&state);
    run_blocking(move || {
        state
            .processes
            .archive()
            .read_page(&session_id, before_offset, max_lines)
    })
    .await
}

/// Delete one archived session's file and mark its row removed.
///
/// A session whose archive is still being written is refused: the file belongs to a
/// writer that would keep appending to a deleted handle.
#[tauri::command]
pub async fn delete_run_log_archive(
    session_id: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let state = clone_app_state(&state);
    run_blocking(move || state.processes.archive().delete(&session_id)).await
}

#[tauri::command]
pub fn open_port(port: u16, protocol: String) -> AppResult<()> {
    crate::privileges::ensure_process_action_allowed()?;
    if !protocol.eq_ignore_ascii_case("TCP") {
        return Err(invalid("Only TCP ports can be opened in a browser"));
    }
    open::that(format!("http://127.0.0.1:{port}"))
        .map_err(|error| invalid(format!("Could not open browser: {error}")))
}

#[tauri::command]
pub fn open_project_directory(project_id: String, state: State<'_, AppState>) -> AppResult<()> {
    crate::privileges::ensure_process_action_allowed()?;
    let project = state
        .storage
        .get_project(&project_id)?
        .ok_or_else(|| invalid("Project not found"))?;
    open::that(&project.path)
        .map_err(|error| invalid(format!("Could not open project directory: {error}")))
}

#[tauri::command]
pub async fn shutdown_app(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let state = clone_app_state(&state);
    run_blocking(move || shutdown(&state)).await?;
    app.exit(0);
    Ok(())
}

fn clone_app_state(state: &AppState) -> AppState {
    AppState {
        storage: state.storage.clone(),
        processes: state.processes.clone(),
    }
}

async fn run_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| invalid(format!("Background command failed: {error}")))?
}

pub fn shutdown(state: &AppState) -> AppResult<()> {
    if state.processes.shutdown_is_complete() {
        return Ok(());
    }
    let reservation = state.processes.reserve_shutdown()?;
    let active = state
        .processes
        .shutdown_restore_snapshot(state.processes.active_profile_ids());
    let save_result = state.storage.save_restore_set(&active);
    let result = finish_shutdown(save_result, || {
        state
            .processes
            .stop_all_and_wait(&reservation, Duration::from_secs(8))
    });
    // Wrap the archive up whichever way stopping went. The application is exiting either
    // way, and a session whose process refused to stop still has queued bytes worth
    // flushing. This never changes the shutdown result: an archive that cannot close
    // reports itself through its own channel and leaves its row for the next run's sweep.
    state.processes.archive().shutdown();
    if result.is_ok() {
        state.processes.complete_shutdown(&reservation)?;
    }
    result
}

fn finish_shutdown(
    save_result: AppResult<()>,
    stop_all: impl FnOnce() -> AppResult<()>,
) -> AppResult<()> {
    let stop_result = stop_all();
    match (save_result, stop_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(save), Ok(())) => Err(save),
        (Ok(()), Err(stop)) => Err(stop),
        (Err(save), Err(stop)) => Err(invalid(format!(
            "Could not save restore set: {save}; process cleanup also failed: {stop}"
        ))),
    }
}

fn start_profile_inner<R: Runtime>(
    profile_id: &str,
    app: &AppHandle<R>,
    state: &AppState,
) -> AppResult<RunStatusEvent> {
    let reservation = state.processes.reserve(profile_id)?;
    start_profile_inner_reserved(&reservation, profile_id, app, state)
}

fn start_profile_inner_reserved<R: Runtime>(
    reservation: &crate::processes::ProfileReservation,
    profile_id: &str,
    app: &AppHandle<R>,
    state: &AppState,
) -> AppResult<RunStatusEvent> {
    let profile = state
        .storage
        .get_profile(profile_id)?
        .ok_or_else(|| invalid("Launch profile not found"))?;
    if let Some(info) = state.processes.info(profile_id) {
        return Ok(status_event_with_reason(
            profile_id.into(),
            RunStatus::Running,
            Some(info.pid),
            RunStatusReason::AlreadyRunning,
        ));
    }
    if let Some((port, process_name)) = first_conflict(&profile)? {
        let related_port = RelatedPort {
            port: port.port,
            protocol: port.protocol.clone(),
        };
        state.processes.set_status(profile_id, RunStatus::Conflict);
        let message = format!(
            "Expected port {} is already occupied{}",
            port.port,
            process_name
                .map(|name| format!(" by {name}"))
                .unwrap_or_default()
        );
        let event = status_event_with_related_port(
            profile_id.into(),
            RunStatus::Conflict,
            None,
            Some(message.clone()),
            Some(related_port.clone()),
        );
        let _ = app.emit("run-status", &event);
        return Err(AppError::port_conflict(
            message,
            related_port.port,
            related_port.protocol,
        ));
    }

    let starting = status_event(profile_id.into(), RunStatus::Starting, None, None);
    state.processes.set_status(profile_id, RunStatus::Starting);
    let _ = app.emit("run-status", &starting);
    let session_id = match state.storage.begin_session(profile_id, &profile.name) {
        Ok(session_id) => session_id,
        Err(error) => {
            state.processes.set_status(profile_id, RunStatus::Idle);
            return Err(error);
        }
    };
    let info = match state
        .processes
        .launch(reservation, &profile, session_id.clone(), app.clone())
    {
        Ok(info) => info,
        Err(error) => {
            let _ = state.storage.finish_session(&session_id, None);
            state.processes.set_status(profile_id, RunStatus::Exited);
            let failed = status_event(
                profile_id.into(),
                RunStatus::Exited,
                None,
                Some(error.to_string()),
            );
            let _ = app.emit("run-status", &failed);
            return Err(error);
        }
    };
    if let Err(error) = state.storage.set_session_pid(&session_id, info.pid) {
        return fail_started_profile(reservation, profile_id, app, state, error);
    }
    if let Err(error) = sync_active_restore_set(state) {
        return fail_started_profile(reservation, profile_id, app, state, error);
    }
    if let Err(error) = wait_for_profile_ready(profile_id, state, PROFILE_READY_TIMEOUT) {
        return fail_started_profile(reservation, profile_id, app, state, error);
    }
    state.processes.set_status(profile_id, RunStatus::Running);
    let running = status_event(profile_id.into(), RunStatus::Running, Some(info.pid), None);
    let _ = app.emit("run-status", &running);
    Ok(running)
}

fn fail_started_profile<R: Runtime>(
    reservation: &crate::processes::ProfileReservation,
    profile_id: &str,
    app: &AppHandle<R>,
    state: &AppState,
    error: crate::error::AppError,
) -> AppResult<RunStatusEvent> {
    let cleanup = stop_profile_after_failed_start_if_running(reservation, profile_id, state);
    let error = match cleanup {
        Ok(()) => error,
        Err(stop) => invalid(format!(
            "{error}; failed to clean up the started process: {stop}"
        )),
    };
    state.processes.set_status(profile_id, RunStatus::Exited);
    let failed = status_event(
        profile_id.into(),
        RunStatus::Exited,
        None,
        Some(error.to_string()),
    );
    let _ = app.emit("run-status", &failed);
    Err(error)
}

fn stop_profile_after_failed_start_if_running(
    reservation: &crate::processes::ProfileReservation,
    profile_id: &str,
    state: &AppState,
) -> AppResult<()> {
    if state.processes.info(profile_id).is_none() {
        return Ok(());
    }
    state
        .processes
        .stop_after_failed_start(reservation, profile_id)?;
    wait_for_profile_stopped(profile_id, state, PROFILE_STOP_TIMEOUT)
}

fn sync_active_restore_set(state: &AppState) -> AppResult<()> {
    state
        .storage
        .save_restore_set(&state.processes.active_profile_ids())?;
    state.processes.clear_shutdown_snapshot();
    Ok(())
}

fn wait_for_profile_stopped(
    profile_id: &str,
    state: &AppState,
    timeout: Duration,
) -> AppResult<()> {
    let deadline = Instant::now() + timeout;
    while state.processes.info(profile_id).is_some() && Instant::now() < deadline {
        std::thread::sleep(STATE_POLL_INTERVAL);
    }
    if state.processes.info(profile_id).is_some() {
        Err(invalid("Timed out while stopping the process tree"))
    } else {
        Ok(())
    }
}

fn first_conflict(
    profile: &crate::models::LaunchProfile,
) -> AppResult<Option<(RelatedPort, Option<String>)>> {
    if profile.expected_ports.is_empty() {
        return Ok(None);
    }
    let entries = runcove::scanner::create_scanner()
        .scan()
        .map_err(|error| invalid(format!("Could not verify expected ports: {error}")))?;
    Ok(profile.expected_ports.iter().find_map(|expected| {
        entries.iter().find_map(|entry| {
            (entry.state == runcove::model::ConnectionState::Listen
                && entry.port == expected.port
                && entry
                    .protocol
                    .to_string()
                    .eq_ignore_ascii_case(&expected.protocol))
            .then(|| {
                (
                    RelatedPort {
                        port: expected.port,
                        protocol: expected.protocol.clone(),
                    },
                    entry.process_name.clone(),
                )
            })
        })
    }))
}

fn wait_for_profile_ready(profile_id: &str, state: &AppState, timeout: Duration) -> AppResult<()> {
    let profile = state
        .storage
        .get_profile(profile_id)?
        .ok_or_else(|| invalid("Launch profile disappeared while restoring"))?;
    if profile.expected_ports.is_empty() {
        std::thread::sleep(Duration::from_millis(300));
        return state
            .processes
            .info(profile_id)
            .map(|_| ())
            .ok_or_else(|| invalid("Profile exited before it became ready"));
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if state.processes.info(profile_id).is_none() {
            return Err(invalid("Profile exited before expected ports became ready"));
        }
        if expected_ports_ready(&profile, state)? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(invalid("Timed out waiting for expected ports"))
}

fn expected_ports_ready(
    profile: &crate::models::LaunchProfile,
    state: &AppState,
) -> AppResult<bool> {
    let entries = runcove::scanner::create_scanner()
        .scan()
        .map_err(|error| invalid(format!("Could not verify expected ports: {error}")))?;
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    Ok(profile.expected_ports.iter().all(|expected| {
        entries.iter().any(|entry| {
            entry.state == runcove::model::ConnectionState::Listen
                && entry.port == expected.port
                && entry
                    .protocol
                    .to_string()
                    .eq_ignore_ascii_case(&expected.protocol)
                && entry.pid.is_some_and(|pid| {
                    state
                        .processes
                        .owns_pid(pid, &system)
                        .is_some_and(|owner| owner.profile_id == profile.id)
                })
        })
    }))
}

fn status_event(
    profile_id: String,
    status: RunStatus,
    pid: Option<u32>,
    message: Option<String>,
) -> RunStatusEvent {
    status_event_with_related_port(profile_id, status, pid, message, None)
}

/// A status event whose text RunCove itself composes, so the frontend can render
/// it in the user's language instead of showing this crate's English.
fn status_event_with_reason(
    profile_id: String,
    status: RunStatus,
    pid: Option<u32>,
    reason: RunStatusReason,
) -> RunStatusEvent {
    RunStatusEvent {
        message: Some(reason.describe()),
        reason: Some(reason),
        ..status_event(profile_id, status, pid, None)
    }
}

fn status_event_with_related_port(
    profile_id: String,
    status: RunStatus,
    pid: Option<u32>,
    message: Option<String>,
    related_port: Option<RelatedPort>,
) -> RunStatusEvent {
    RunStatusEvent {
        profile_id,
        status,
        pid,
        reason: None,
        message,
        related_port,
        unexpected: false,
        timestamp: now_ms(),
    }
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn verify_requested_listener(request: &ExternalProcessRequest) -> AppResult<()> {
    let entries = runcove::scanner::create_scanner()
        .scan()
        .map_err(|error| invalid(format!("Could not verify port ownership: {error}")))?;
    if listener_matches_request(request, &entries) {
        Ok(())
    } else {
        Err(invalid(
            "Port ownership changed; refresh before terminating it",
        ))
    }
}

fn listener_matches_request(
    request: &ExternalProcessRequest,
    entries: &[runcove::model::PortEntry],
) -> bool {
    entries.iter().any(|entry| {
        entry.state == runcove::model::ConnectionState::Listen
            && entry.port == request.port
            && entry
                .protocol
                .to_string()
                .eq_ignore_ascii_case(&request.protocol)
            && entry.pid == Some(request.pid)
    })
}

#[cfg(not(windows))]
fn terminate_external_tree(pid: u32) -> AppResult<()> {
    let status = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(invalid("Could not terminate process"))
    }
}

#[cfg(windows)]
fn terminate_external_windows(request: &ExternalProcessRequest, state: &AppState) -> AppResult<()> {
    if request.pid == 0 || request.pid == std::process::id() {
        return Err(invalid("RunCove refuses to terminate this process"));
    }

    let root = ExternalProcessHandle::open(request.pid)?;
    let expected_started_at = request
        .started_at
        .ok_or_else(|| invalid("A verified process start time is required"))?;
    if root.started_at_ms()? != expected_started_at {
        return Err(invalid(
            "Process identity changed; refresh before terminating it",
        ));
    }
    let expected_path = request
        .executable_path
        .as_deref()
        .map(Path::new)
        .map(normalized_path)
        .ok_or_else(|| invalid("A verified executable path is required"))?;
    if normalized_path(Path::new(&root.executable_path()?)) != expected_path {
        return Err(invalid(
            "Process executable changed; refresh before terminating it",
        ));
    }

    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    if state.processes.owns_pid(request.pid, &system).is_some() {
        return Err(invalid(
            "Managed processes must be stopped through their launch profile",
        ));
    }
    verify_requested_listener(request)?;
    // Holding the verified root handle prevents Windows from recycling its PID
    // while taskkill resolves and terminates the current process tree.
    let taskkill = runcove::process::windows_system32_executable("taskkill.exe")
        .map_err(|error| invalid(format!("Could not locate Windows taskkill.exe: {error}")))?;
    let output = Command::new(taskkill)
        .args(["/PID", &request.pid.to_string(), "/T", "/F"])
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(invalid(format!(
            "Could not terminate process tree: {detail}"
        )))
    }
}

#[cfg(windows)]
struct ExternalProcessHandle(usize);

#[cfg(windows)]
impl ExternalProcessHandle {
    fn open(pid: u32) -> AppResult<Self> {
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        };
        let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE;
        let handle = unsafe { OpenProcess(access, false, pid) }
            .map_err(|error| invalid(format!("Could not open process {pid}: {error}")))?;
        Ok(Self(handle.0 as usize))
    }

    fn started_at_ms(&self) -> AppResult<u64> {
        use windows::Win32::Foundation::{FILETIME, HANDLE};
        use windows::Win32::System::Threading::GetProcessTimes;
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(
                HANDLE(self.0 as *mut _),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        }
        .map_err(|error| invalid(format!("Could not read process start time: {error}")))?;
        let ticks = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        const WINDOWS_TO_UNIX_TICKS: u64 = 116_444_736_000_000_000;
        Ok(ticks.saturating_sub(WINDOWS_TO_UNIX_TICKS) / 10_000)
    }

    fn executable_path(&self) -> AppResult<String> {
        use windows::core::PWSTR;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Threading::QueryFullProcessImageNameW;
        let mut buffer = vec![0_u16; 32_768];
        let mut length = buffer.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                HANDLE(self.0 as *mut _),
                Default::default(),
                PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        }
        .map_err(|error| invalid(format!("Could not read process executable: {error}")))?;
        Ok(String::from_utf16_lossy(&buffer[..length as usize]))
    }
}

#[cfg(windows)]
impl Drop for ExternalProcessHandle {
    fn drop(&mut self) {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        let _ = unsafe { CloseHandle(HANDLE(self.0 as *mut _)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ExpectedPortInput, LaunchProfileInput, ProjectInput};
    use crate::processes::ProcessManager;
    use crate::storage::Storage;
    use std::net::{TcpListener, TcpStream};
    #[cfg(windows)]
    use std::process::{Child, Command as ProcessCommand, Stdio};
    use std::sync::Arc;
    use tauri::Manager;

    #[test]
    fn blocking_command_work_runs_on_a_worker_thread() {
        let caller_thread = std::thread::current().id();
        let worker_thread =
            tauri::async_runtime::block_on(run_blocking(move || Ok(std::thread::current().id())))
                .unwrap();

        assert_ne!(worker_thread, caller_thread);
    }

    #[test]
    fn long_running_commands_have_async_send_contract() {
        fn assert_send_future<T>(_future: impl std::future::Future<Output = AppResult<T>> + Send) {}

        fn start_contract(profile_id: String, app: AppHandle, state: State<'_, AppState>) {
            assert_send_future(start_profile(profile_id, app, state));
        }

        fn stop_contract(profile_id: String, app: AppHandle, state: State<'_, AppState>) {
            assert_send_future(stop_profile(profile_id, app, state));
        }

        fn restart_contract(profile_id: String, app: AppHandle, state: State<'_, AppState>) {
            assert_send_future(restart_profile(profile_id, app, state));
        }

        fn restore_contract(app: AppHandle, state: State<'_, AppState>) {
            assert_send_future(restore_last_run_set(app, state));
        }

        fn terminate_contract(request: ExternalProcessRequest, state: State<'_, AppState>) {
            assert_send_future(terminate_external_process(request, state));
        }

        fn confirm_contract(request: ConfirmAssociationRequest, state: State<'_, AppState>) {
            assert_send_future(confirm_port_association(request, state));
        }

        fn discover_contract(directory: String) {
            assert_send_future(discover_project(directory));
        }

        fn scan_root_contract(directory: String, state: State<'_, AppState>) {
            assert_send_future(scan_development_root(directory, state));
        }

        fn scan_saved_root_contract(state: State<'_, AppState>) {
            assert_send_future(scan_saved_development_root(state));
        }

        fn elevation_contract(app: AppHandle) {
            assert_send_future(request_elevated_monitoring(app));
        }

        fn shutdown_contract(app: AppHandle, state: State<'_, AppState>) {
            assert_send_future(shutdown_app(app, state));
        }

        let _ = (
            start_contract,
            stop_contract,
            restart_contract,
            restore_contract,
            terminate_contract,
            confirm_contract,
            discover_contract,
            scan_root_contract,
            scan_saved_root_contract,
            elevation_contract,
            shutdown_contract,
        );
    }

    #[test]
    fn language_preference_parser_rejects_unsupported_values_explicitly() {
        assert_eq!(
            parse_language_preference("zh-CN").unwrap(),
            LanguagePreference::SimplifiedChinese
        );
        assert_eq!(
            parse_language_preference("fr").unwrap_err().to_string(),
            "Unsupported language preference 'fr'; expected system, en, or zh-CN"
        );
    }

    #[test]
    fn language_preference_remains_committed_when_tray_refresh_fails() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(&temp.path().join("language-settings.sqlite3")).unwrap();

        let (settings, tray_error) =
            persist_language_preference(&storage, LanguagePreference::SimplifiedChinese, |_| {
                Err(invalid("tray update failed"))
            })
            .unwrap();

        assert_eq!(
            settings.language_preference,
            LanguagePreference::SimplifiedChinese
        );
        assert_eq!(tray_error.as_deref(), Some("tray update failed"));
        assert_eq!(
            storage.settings().unwrap().language_preference,
            LanguagePreference::SimplifiedChinese
        );
    }

    #[test]
    fn close_behavior_parser_rejects_unsupported_values_explicitly() {
        assert_eq!(
            parse_close_behavior("hideToTray").unwrap(),
            CloseBehavior::HideToTray
        );
        assert_eq!(
            parse_close_behavior("close").unwrap_err().to_string(),
            "Unsupported close behavior 'close'; expected ask, hideToTray, or quit"
        );
    }

    #[test]
    fn close_behavior_command_helper_persists_the_selected_action() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(&temp.path().join("close-settings.sqlite3")).unwrap();

        let settings = persist_close_behavior(&storage, CloseBehavior::Quit).unwrap();

        assert_eq!(settings.close_behavior, CloseBehavior::Quit);
        assert_eq!(
            storage.settings().unwrap().close_behavior,
            CloseBehavior::Quit
        );
    }

    #[test]
    fn saved_development_root_requires_and_returns_the_durable_setting() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(&temp.path().join("root-settings.sqlite3")).unwrap();

        assert_eq!(
            saved_development_root(&storage).unwrap_err().to_string(),
            "No development root has been saved yet"
        );
        storage.remember_development_root(temp.path()).unwrap();
        assert_eq!(
            Path::new(&saved_development_root(&storage).unwrap()),
            temp.path().canonicalize().unwrap()
        );
    }

    #[cfg(windows)]
    struct ExternalProcessFixture {
        child: Child,
        child_pid: Option<u32>,
        child_handle: Option<ExternalProcessHandle>,
        state: AppState,
        temp: tempfile::TempDir,
        port: u16,
        started_at: u64,
        executable_path: String,
    }

    #[cfg(windows)]
    impl ExternalProcessFixture {
        fn spawn() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let port = unused_port();
            let child_pid_path = temp.path().join("child.pid");
            std::fs::write(
                temp.path().join("listener.js"),
                r#"
setInterval(() => {}, 1000);
"#,
            )
            .unwrap();
            std::fs::write(
                temp.path().join("parent.js"),
                r#"
const fs = require('fs');
const net = require('net');
const path = require('path');
const { spawn } = require('child_process');
const child = spawn(process.execPath, [path.join(__dirname, 'listener.js')], {
  stdio: 'ignore'
});
fs.writeFileSync(process.argv[3], String(child.pid));
const server = net.createServer();
server.listen(Number(process.argv[2]), '127.0.0.1');
setInterval(() => {}, 1000);
"#,
            )
            .unwrap();

            let storage =
                Arc::new(Storage::open(&temp.path().join("runcove-test.sqlite3")).unwrap());
            let state = AppState {
                storage,
                processes: Arc::new(ProcessManager::new(10)),
            };
            let child = ProcessCommand::new("node.exe")
                .arg(temp.path().join("parent.js"))
                .arg(port.to_string())
                .arg(&child_pid_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut fixture = Self {
                child,
                child_pid: None,
                child_handle: None,
                state,
                temp,
                port,
                started_at: 0,
                executable_path: String::new(),
            };

            let root = ExternalProcessHandle::open(fixture.child.id()).unwrap();
            fixture.started_at = root.started_at_ms().unwrap();
            fixture.executable_path = root.executable_path().unwrap();

            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if fixture.child_pid.is_none() {
                    if let Some(pid) = std::fs::read_to_string(&child_pid_path)
                        .ok()
                        .and_then(|value| value.trim().parse::<u32>().ok())
                    {
                        fixture.child_pid = Some(pid);
                        fixture.child_handle = Some(ExternalProcessHandle::open(pid).unwrap());
                    }
                }
                if fixture.child_pid.is_some()
                    && TcpStream::connect(("127.0.0.1", fixture.port)).is_ok()
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "external process fixture did not open its TCP port"
                );
                assert!(fixture.child.try_wait().unwrap().is_none());
                std::thread::sleep(Duration::from_millis(25));
            }
            fixture
        }

        fn request(&self) -> ExternalProcessRequest {
            ExternalProcessRequest {
                port: self.port,
                protocol: "tcp".into(),
                pid: self.child.id(),
                started_at: Some(self.started_at),
                executable_path: Some(self.executable_path.clone()),
            }
        }

        fn assert_running(&mut self) {
            assert!(self.child.try_wait().unwrap().is_none());
            assert!(TcpStream::connect(("127.0.0.1", self.port)).is_ok());
        }

        fn wait_for_tree_exit(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.child.try_wait().unwrap().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(25));
            }
            assert!(self.child.try_wait().unwrap().is_some());

            let child_handle = self.child_handle.as_ref().unwrap();
            assert!(
                external_process_exited(child_handle, 5_000),
                "external child process remained alive"
            );

            let deadline = Instant::now() + Duration::from_secs(3);
            while TcpStream::connect(("127.0.0.1", self.port)).is_ok() && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(25));
            }
            assert!(TcpStream::connect(("127.0.0.1", self.port)).is_err());
        }
    }

    #[cfg(windows)]
    impl Drop for ExternalProcessFixture {
        fn drop(&mut self) {
            if self.child.try_wait().ok().flatten().is_none() {
                let _ = ProcessCommand::new("taskkill.exe")
                    .args(["/PID", &self.child.id().to_string(), "/T", "/F"])
                    .output();
            }
            if let (Some(pid), Some(handle)) = (self.child_pid, self.child_handle.as_ref()) {
                if external_process_is_running(handle) {
                    let _ = ProcessCommand::new("taskkill.exe")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .output();
                }
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[cfg(windows)]
    fn external_process_exited(handle: &ExternalProcessHandle, timeout_ms: u32) -> bool {
        use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::WaitForSingleObject;
        unsafe { WaitForSingleObject(HANDLE(handle.0 as *mut _), timeout_ms) == WAIT_OBJECT_0 }
    }

    #[cfg(windows)]
    fn external_process_is_running(handle: &ExternalProcessHandle) -> bool {
        use windows::Win32::Foundation::{HANDLE, WAIT_TIMEOUT};
        use windows::Win32::System::Threading::WaitForSingleObject;
        unsafe { WaitForSingleObject(HANDLE(handle.0 as *mut _), 0) == WAIT_TIMEOUT }
    }

    #[cfg(windows)]
    fn npm_fixture(
        script: &str,
        port: u16,
    ) -> (
        tempfile::TempDir,
        tauri::App<tauri::test::MockRuntime>,
        String,
    ) {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("package.json"),
            r#"{"private":true,"scripts":{"dev":"node server.js"}}"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("server.js"),
            script.replace("FIXTURE_PORT", &port.to_string()),
        )
        .unwrap();

        let storage = Arc::new(Storage::open(&temp.path().join("runcove-test.sqlite3")).unwrap());
        let project = storage
            .save_project(ProjectInput {
                id: None,
                name: "npm fixture".into(),
                path: temp.path().to_string_lossy().into_owned(),
                profiles: vec![LaunchProfileInput {
                    id: None,
                    name: "dev".into(),
                    program: "npm.cmd".into(),
                    args: vec!["run".into(), "dev".into()],
                    cwd: temp.path().to_string_lossy().into_owned(),
                    expected_ports: vec![ExpectedPortInput {
                        id: None,
                        port,
                        protocol: "tcp".into(),
                    }],
                }],
            })
            .unwrap();
        let profile_id = project.profiles[0].id.clone();
        let app = tauri::test::mock_app();
        app.manage(AppState {
            storage,
            processes: Arc::new(ProcessManager::new(100)),
        });
        (temp, app, profile_id)
    }

    #[cfg(windows)]
    fn unused_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[test]
    fn active_project_cannot_be_mutated() {
        let project = Project {
            id: "project".into(),
            name: "Project".into(),
            path: ".".into(),
            profiles: vec![crate::models::LaunchProfile {
                id: "running".into(),
                project_id: "project".into(),
                name: "dev".into(),
                program: "npm".into(),
                args: vec!["run".into(), "dev".into()],
                cwd: ".".into(),
                expected_ports: Vec::new(),
                status: RunStatus::Running,
                pid: Some(42),
            }],
            created_at: 0,
            updated_at: 0,
        };

        assert!(project_has_active_profile(&project, &["running".into()]));
        assert!(!project_has_active_profile(&project, &["other".into()]));
    }

    #[test]
    fn editing_project_clears_runtime_state_for_removed_profiles() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Arc::new(Storage::open(&temp.path().join("project-edit.sqlite3")).unwrap());
        let project = storage
            .save_project(ProjectInput {
                id: None,
                name: "Project".into(),
                path: temp.path().to_string_lossy().into_owned(),
                profiles: vec![
                    LaunchProfileInput {
                        id: None,
                        name: "dev".into(),
                        program: "npm.cmd".into(),
                        args: vec!["run".into(), "dev".into()],
                        cwd: temp.path().to_string_lossy().into_owned(),
                        expected_ports: Vec::new(),
                    },
                    LaunchProfileInput {
                        id: None,
                        name: "preview".into(),
                        program: "npm.cmd".into(),
                        args: vec!["run".into(), "preview".into()],
                        cwd: temp.path().to_string_lossy().into_owned(),
                        expected_ports: Vec::new(),
                    },
                ],
            })
            .unwrap();
        let retained_profile_id = project.profiles[0].id.clone();
        let removed_profile_id = project.profiles[1].id.clone();
        let processes = Arc::new(ProcessManager::new(10));
        processes.set_status(&retained_profile_id, RunStatus::Exited);
        processes.set_status(&removed_profile_id, RunStatus::Conflict);

        let app = tauri::test::mock_app();
        app.manage(AppState {
            storage,
            processes: processes.clone(),
        });
        let saved = save_project(
            ProjectInput {
                id: Some(project.id),
                name: "Project".into(),
                path: temp.path().to_string_lossy().into_owned(),
                profiles: vec![LaunchProfileInput {
                    id: Some(retained_profile_id.clone()),
                    name: "dev".into(),
                    program: "npm.cmd".into(),
                    args: vec!["run".into(), "dev".into()],
                    cwd: temp.path().to_string_lossy().into_owned(),
                    expected_ports: Vec::new(),
                }],
            },
            app.state::<AppState>(),
        )
        .unwrap();

        assert_eq!(saved.profiles.len(), 1);
        assert_eq!(
            processes.status(&retained_profile_id),
            Some(RunStatus::Exited)
        );
        assert_eq!(processes.status(&removed_profile_id), None);
    }

    #[test]
    fn path_identity_is_case_and_separator_insensitive_on_windows() {
        assert_eq!(
            normalized_path(Path::new("C:/Code/App/")),
            normalized_path(Path::new("c:\\code\\app"))
        );
    }

    fn suggested_port_snapshot() -> PortSnapshot {
        PortSnapshot {
            port: 5_173,
            protocol: "tcp".into(),
            state: "LISTEN".into(),
            bind_address: Some("127.0.0.1".into()),
            is_public: false,
            active: true,
            pid: Some(42),
            process_name: Some("node.exe".into()),
            executable_path: Some(r"C:\Program Files\nodejs\node.exe".into()),
            command_line: Some("node server.js".into()),
            process_started_at: Some(1_000),
            last_seen_at: Some(2_000),
            project_id: Some("project".into()),
            profile_id: Some("profile".into()),
            association_source: Some(AssociationSource::Suggested),
        }
    }

    fn confirm_request() -> ConfirmAssociationRequest {
        ConfirmAssociationRequest {
            port: 5_173,
            protocol: "tcp".into(),
            project_id: "project".into(),
            profile_id: Some("profile".into()),
            pid: 42,
            started_at: 1_000,
            executable_path: r"C:\Program Files\nodejs\node.exe".into(),
        }
    }

    #[test]
    fn association_confirmation_requires_the_current_suggested_identity() {
        let request = confirm_request();
        let port = suggested_port_snapshot();
        assert!(
            validate_confirm_association_request(&request, std::slice::from_ref(&port)).is_ok()
        );

        let mut stale_pid = port.clone();
        stale_pid.pid = Some(43);
        assert_eq!(
            validate_confirm_association_request(&request, &[stale_pid])
                .unwrap_err()
                .to_string(),
            "Observed port identity changed; refresh before confirming association"
        );

        let mut stale_process = port.clone();
        stale_process.process_started_at = Some(1_001);
        assert_eq!(
            validate_confirm_association_request(&request, &[stale_process])
                .unwrap_err()
                .to_string(),
            "Observed process identity changed; refresh before confirming association"
        );

        let mut changed_suggestion = port.clone();
        changed_suggestion.project_id = Some("other-project".into());
        assert_eq!(
            validate_confirm_association_request(&request, &[changed_suggestion])
                .unwrap_err()
                .to_string(),
            "Suggested project association changed; refresh before confirming it"
        );

        let mut no_longer_suggested = port;
        no_longer_suggested.association_source = Some(AssociationSource::Confirmed);
        assert_eq!(
            validate_confirm_association_request(&request, &[no_longer_suggested])
                .unwrap_err()
                .to_string(),
            "Suggested project association changed; refresh before confirming it"
        );
    }

    #[test]
    fn rejected_association_confirmation_is_not_persisted() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Arc::new(Storage::open(&temp.path().join("association.sqlite3")).unwrap());
        let saved = storage
            .save_project(ProjectInput {
                id: None,
                name: "Project".into(),
                path: temp.path().to_string_lossy().into_owned(),
                profiles: vec![LaunchProfileInput {
                    id: None,
                    name: "dev".into(),
                    program: "npm.cmd".into(),
                    args: vec!["run".into(), "dev".into()],
                    cwd: temp.path().to_string_lossy().into_owned(),
                    expected_ports: vec![ExpectedPortInput {
                        id: None,
                        port: 5_173,
                        protocol: "tcp".into(),
                    }],
                }],
            })
            .unwrap();
        let state = AppState {
            storage: storage.clone(),
            processes: Arc::new(ProcessManager::new(10)),
        };
        let mut request = confirm_request();
        request.project_id = saved.id;
        request.profile_id = Some(saved.profiles[0].id.clone());
        request.pid = 0;

        assert!(confirm_port_association_inner(&request, &state).is_err());
        assert!(storage.list_associations().unwrap().is_empty());
    }

    #[test]
    fn external_termination_listener_match_includes_port_protocol_and_pid() {
        let request = ExternalProcessRequest {
            port: 5_173,
            protocol: "tcp".into(),
            pid: 42,
            started_at: Some(1_000),
            executable_path: Some(r"C:\Program Files\nodejs\node.exe".into()),
        };
        let entry = runcove::model::PortEntry {
            port: 5_173,
            protocol: runcove::model::Protocol::TCP,
            state: runcove::model::ConnectionState::Listen,
            pid: Some(42),
            process_name: Some("node.exe".into()),
            bind_address: "127.0.0.1".parse().unwrap(),
            is_public: false,
        };

        assert!(listener_matches_request(
            &request,
            std::slice::from_ref(&entry)
        ));
        let mut stale_pid = request.clone();
        stale_pid.pid = 43;
        assert!(!listener_matches_request(
            &stale_pid,
            std::slice::from_ref(&entry)
        ));
        let mut stale_protocol = request;
        stale_protocol.protocol = "udp".into();
        assert!(!listener_matches_request(&stale_protocol, &[entry]));
    }

    #[test]
    fn shutdown_cleanup_runs_even_when_persistence_fails() {
        let stopped = std::cell::Cell::new(false);
        let result = finish_shutdown(Err(invalid("database failed")), || {
            stopped.set(true);
            Ok(())
        });
        assert!(result.is_err());
        assert!(stopped.get());
    }

    #[cfg(windows)]
    #[test]
    fn external_termination_rejects_mismatched_start_time_without_stopping_process() {
        let mut fixture = ExternalProcessFixture::spawn();
        let mut request = fixture.request();
        request.started_at = Some(fixture.started_at.saturating_add(1));

        let error = terminate_external_windows(&request, &fixture.state).unwrap_err();

        assert_eq!(
            error.to_string(),
            "Process identity changed; refresh before terminating it"
        );
        fixture.assert_running();
    }

    #[cfg(windows)]
    #[test]
    fn external_termination_rejects_mismatched_executable_without_stopping_process() {
        let mut fixture = ExternalProcessFixture::spawn();
        let mut request = fixture.request();
        request.executable_path = Some(
            fixture
                .temp
                .path()
                .join("not-node.exe")
                .display()
                .to_string(),
        );

        let error = terminate_external_windows(&request, &fixture.state).unwrap_err();

        assert_eq!(
            error.to_string(),
            "Process executable changed; refresh before terminating it"
        );
        fixture.assert_running();
    }

    /// Whether the machine, rather than RunCove, is what refused a termination.
    ///
    /// `taskkill /T /F` fails with `Access is denied.` for a tree it could not walk —
    /// a child that exited between enumeration and termination, or security software
    /// standing in front of one — and returns a failing status even when the root is
    /// already gone. That is the environment declining the operation, and it is what
    /// makes the one test needing a real termination flaky on some Windows machines
    /// while passing on CI.
    ///
    /// The match is on RunCove's own wrapper text and not on the reason inside it,
    /// deliberately. `taskkill` prints in the system language, and its output reaches
    /// this string through `from_utf8_lossy`, so on a non-English Windows the reason is
    /// either translated or mojibake — matching `Access is denied` would silently stop
    /// working there. Everything RunCove decides for itself still fails the test: a
    /// changed identity, a managed process, a missing `taskkill.exe`, or a refusal to
    /// terminate RunCove itself never reach this wrapper.
    #[cfg(windows)]
    fn termination_refused_by_environment(error: &AppError) -> bool {
        error
            .to_string()
            .starts_with("Could not terminate process tree:")
    }

    #[cfg(windows)]
    #[test]
    fn external_termination_with_verified_identity_stops_tree_and_releases_port() {
        let mut fixture = ExternalProcessFixture::spawn();
        let request = fixture.request();

        if let Err(refused) = terminate_external_windows(&request, &fixture.state) {
            assert!(
                termination_refused_by_environment(&refused),
                "termination failed for RunCove's own reason: {refused}"
            );
            // Visible under `--nocapture`, which is what anyone checking whether this
            // path is still covered on their machine would run.
            eprintln!("skipped after the environment refused the termination: {refused}");
            return;
        }

        fixture.wait_for_tree_exit();
    }

    #[cfg(windows)]
    #[test]
    fn external_termination_rejects_a_runcove_managed_process() {
        let port = unused_port();
        let script = r#"
const net = require('net');
const server = net.createServer();
server.listen(FIXTURE_PORT, '127.0.0.1');
setInterval(() => {}, 1000);
"#;
        let (_temp, app, profile_id) = npm_fixture(script, port);
        let state = app.state::<AppState>();
        start_profile_inner(&profile_id, app.handle(), &state).unwrap();
        let info = state.processes.info(&profile_id).unwrap();
        let process = ExternalProcessHandle::open(info.pid).unwrap();
        let request = ExternalProcessRequest {
            port,
            protocol: "tcp".into(),
            pid: info.pid,
            started_at: Some(process.started_at_ms().unwrap()),
            executable_path: Some(process.executable_path().unwrap()),
        };

        let error = terminate_external_windows(&request, &state).unwrap_err();

        assert_eq!(
            error.to_string(),
            "Managed processes must be stopped through their launch profile"
        );
        assert!(state.processes.info(&profile_id).is_some());
        assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());
        shutdown(&state).unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires explicitly configured live local services"]
    fn live_imports_detect_conflicts_without_touching_existing_processes() {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LiveCase {
            path: String,
            profile: String,
            port: u16,
        }

        fn listener_identity(port: u16) -> (u32, u64, String) {
            let mut pids = runcove::scanner::create_scanner()
                .scan()
                .unwrap()
                .into_iter()
                .filter(|entry| {
                    entry.port == port
                        && entry.state == runcove::model::ConnectionState::Listen
                        && entry.protocol.to_string().eq_ignore_ascii_case("tcp")
                })
                .filter_map(|entry| entry.pid)
                .collect::<Vec<_>>();
            pids.sort_unstable();
            pids.dedup();
            assert_eq!(pids.len(), 1, "expected one TCP listener PID on {port}");
            let process = ExternalProcessHandle::open(pids[0]).unwrap();
            (
                pids[0],
                process.started_at_ms().unwrap(),
                normalized_path(Path::new(&process.executable_path().unwrap())),
            )
        }

        let cases = serde_json::from_str::<Vec<LiveCase>>(
            &std::env::var("RUNCOVE_LIVE_PROJECT_CASES")
                .expect("RUNCOVE_LIVE_PROJECT_CASES must contain a JSON case list"),
        )
        .unwrap();
        assert!(!cases.is_empty());
        let before = cases
            .iter()
            .map(|case| (case.port, listener_identity(case.port)))
            .collect::<Vec<_>>();
        let temp = tempfile::tempdir().unwrap();
        let storage = Arc::new(Storage::open(&temp.path().join("runcove-live.sqlite3")).unwrap());
        let app = tauri::test::mock_app();
        app.manage(AppState {
            storage: storage.clone(),
            processes: Arc::new(ProcessManager::new(100)),
        });
        let state = app.state::<AppState>();

        for case in &cases {
            let discovered =
                tauri::async_runtime::block_on(discover_project(case.path.clone())).unwrap();
            let observed = discovered
                .profiles
                .iter()
                .find(|profile| profile.name == case.profile)
                .unwrap();
            if !observed.observed_runtime {
                let mut system = System::new_all();
                system.refresh_processes(ProcessesToUpdate::All, true);
                for listener in runcove::scanner::create_scanner()
                    .scan()
                    .unwrap()
                    .into_iter()
                    .filter(|entry| entry.port == case.port)
                {
                    let mut pid = listener.pid;
                    while let Some(current) = pid {
                        let Some(process) = system.process(sysinfo::Pid::from_u32(current)) else {
                            break;
                        };
                        let executable_name = process
                            .exe()
                            .and_then(Path::file_name)
                            .map(|name| name.to_string_lossy().into_owned());
                        eprintln!(
                            "LIVE_IMPORT_DIAGNOSTIC port={} pid={} cwd={:?} executable={:?}",
                            case.port,
                            current,
                            process.cwd(),
                            executable_name
                        );
                        pid = process.parent().map(sysinfo::Pid::as_u32);
                    }
                }
            }
            assert!(observed.observed_runtime, "{} was not observed", case.path);
            assert!(
                observed
                    .expected_ports
                    .iter()
                    .any(|expected| expected.port == case.port && expected.protocol == "tcp"),
                "{} did not retain expected port {}",
                case.path,
                case.port
            );
            let saved = storage
                .save_project(ProjectInput {
                    id: None,
                    name: discovered.name,
                    path: discovered.path,
                    profiles: discovered
                        .profiles
                        .into_iter()
                        .map(|profile| LaunchProfileInput {
                            id: None,
                            name: profile.name,
                            program: profile.program,
                            args: profile.args,
                            cwd: profile.cwd,
                            expected_ports: profile
                                .expected_ports
                                .into_iter()
                                .map(|expected| ExpectedPortInput {
                                    id: None,
                                    port: expected.port,
                                    protocol: expected.protocol,
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .unwrap();
            let profile_id = saved
                .profiles
                .iter()
                .find(|profile| profile.name == case.profile)
                .unwrap()
                .id
                .clone();

            let error = start_profile_inner(&profile_id, app.handle(), &state).unwrap_err();

            assert!(error.to_string().contains(&case.port.to_string()));
            assert!(state.processes.info(&profile_id).is_none());
            assert!(storage.list_sessions(100).unwrap().is_empty());
        }

        for (port, identity) in before {
            assert_eq!(listener_identity(port), identity);
        }
    }

    #[test]
    fn restore_stops_failed_profile_and_keeps_only_ready_items() {
        let events = std::cell::RefCell::new(Vec::new());
        let result = restore_profiles(
            vec!["first".into(), "second".into(), "third".into()],
            |profile| {
                events.borrow_mut().push(format!("start:{profile}"));
                events.borrow_mut().push(format!("ready:{profile}"));
                if profile == "second" {
                    events.borrow_mut().push(format!("stop:{profile}"));
                    Err(invalid("port timeout"))
                } else {
                    Ok(())
                }
            },
        );

        assert_eq!(result.started_profile_ids, vec!["first"]);
        assert_eq!(result.failed_profile_id.as_deref(), Some("second"));
        assert_eq!(result.related_port, None);
        assert_eq!(
            events.into_inner(),
            vec![
                "start:first",
                "ready:first",
                "start:second",
                "ready:second",
                "stop:second",
            ]
        );
    }

    #[test]
    fn restore_result_preserves_structured_port_conflict_context() {
        let result = restore_profiles(vec!["first".into(), "second".into()], |profile| {
            if profile == "second" {
                Err(AppError::port_conflict(
                    "Expected port 5173 is already occupied",
                    5173,
                    "tcp",
                ))
            } else {
                Ok(())
            }
        });

        assert_eq!(result.started_profile_ids, vec!["first"]);
        assert_eq!(result.failed_profile_id.as_deref(), Some("second"));
        assert_eq!(
            result.related_port,
            Some(RelatedPort {
                port: 5173,
                protocol: "tcp".into(),
            })
        );
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["relatedPort"]["port"], 5173);
        assert_eq!(json["relatedPort"]["protocol"], "tcp");
    }

    /// Two ordered walks can overlap in membership — two groups sharing a profile, or a
    /// restore set that is exactly a group's members. Before this waited, the shared
    /// profile failed the whole second walk, so overlapping walks were unusable together.
    #[test]
    fn a_walk_member_another_operation_holds_is_waited_for_rather_than_failed() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Arc::new(Storage::open(&temp.path().join("shared-member.sqlite3")).unwrap());
        let processes = Arc::new(ProcessManager::new(10));
        let state = AppState {
            storage,
            processes: processes.clone(),
        };

        let holder = processes.reserve("shared").unwrap();
        let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let releaser = {
            let released = released.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                released.store(true, std::sync::atomic::Ordering::SeqCst);
                drop(holder);
            })
        };

        let reservation = match reserve_walk_member("shared", &state) {
            Ok(reservation) => reservation,
            Err(error) => panic!("waiting for a held member failed: {error}"),
        };
        assert!(
            released.load(std::sync::atomic::Ordering::SeqCst),
            "returned a reservation before the holder released it"
        );
        releaser.join().unwrap();
        drop(reservation);

        // A shutdown is the one refusal that must not be waited out, because nothing can
        // start during one. It has to come back well inside the handoff budget.
        let _shutdown = processes.reserve_shutdown().unwrap();
        let started = Instant::now();
        let error = match reserve_walk_member("shared", &state) {
            Err(error) => error,
            Ok(_) => panic!("reserved a member during shutdown"),
        };
        assert!(error.to_string().contains("shutdown"));
        assert!(started.elapsed() < RESERVATION_HANDOFF_TIMEOUT);
    }

    #[test]
    fn stopping_a_group_walks_it_backwards_and_reports_every_member_that_refused() {
        let visited = std::cell::RefCell::new(Vec::new());
        let (stopped, failures) = stop_group_profiles(
            &[
                "db".into(),
                "api".into(),
                "worker".into(),
                "web".into(),
                "proxy".into(),
            ],
            |profile| {
                visited.borrow_mut().push(profile.to_owned());
                match profile {
                    "proxy" | "worker" => Err(invalid(format!("{profile} would not stop"))),
                    "api" => Ok(false),
                    _ => Ok(true),
                }
            },
        );

        assert_eq!(
            visited.into_inner(),
            ["proxy", "web", "worker", "api", "db"]
        );
        assert_eq!(stopped, ["web", "db"]);
        assert_eq!(
            failures,
            vec![
                LaunchGroupStopFailure {
                    profile_id: "proxy".into(),
                    error: "proxy would not stop".into(),
                },
                LaunchGroupStopFailure {
                    profile_id: "worker".into(),
                    error: "worker would not stop".into(),
                },
            ]
        );
    }

    #[test]
    fn a_group_the_profile_cascade_emptied_can_neither_be_started_nor_stopped() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Arc::new(Storage::open(&temp.path().join("empty-group.sqlite3")).unwrap());
        let project = storage
            .save_project(ProjectInput {
                id: None,
                name: "Project".into(),
                path: temp.path().to_string_lossy().into_owned(),
                profiles: vec![
                    LaunchProfileInput {
                        id: None,
                        name: "dev".into(),
                        program: "npm.cmd".into(),
                        args: vec!["run".into(), "dev".into()],
                        cwd: temp.path().to_string_lossy().into_owned(),
                        expected_ports: Vec::new(),
                    },
                    LaunchProfileInput {
                        id: None,
                        name: "preview".into(),
                        program: "npm.cmd".into(),
                        args: vec!["run".into(), "preview".into()],
                        cwd: temp.path().to_string_lossy().into_owned(),
                        expected_ports: Vec::new(),
                    },
                ],
            })
            .unwrap();
        let group = storage
            .save_launch_group(LaunchGroupInput {
                id: None,
                name: "Full stack".into(),
                profile_ids: vec![project.profiles[0].id.clone()],
            })
            .unwrap();
        storage
            .save_project(ProjectInput {
                id: Some(project.id.clone()),
                name: "Project".into(),
                path: temp.path().to_string_lossy().into_owned(),
                profiles: vec![LaunchProfileInput {
                    id: Some(project.profiles[1].id.clone()),
                    name: "preview".into(),
                    program: "npm.cmd".into(),
                    args: vec!["run".into(), "preview".into()],
                    cwd: temp.path().to_string_lossy().into_owned(),
                    expected_ports: Vec::new(),
                }],
            })
            .unwrap();
        let app = tauri::test::mock_app();
        app.manage(AppState {
            storage,
            processes: Arc::new(ProcessManager::new(10)),
        });
        let state = app.state::<AppState>();

        let start = start_launch_group_inner(&group.id, app.handle(), &state).unwrap_err();
        let stop = stop_launch_group_inner(&group.id, app.handle(), &state).unwrap_err();
        let missing = start_launch_group_inner("no-such-group", app.handle(), &state).unwrap_err();

        assert!(start.to_string().contains("no launch profiles"));
        assert!(stop.to_string().contains("no launch profiles"));
        assert!(missing.to_string().contains("not found"));
        assert!(state.storage.launch_group(&group.id).unwrap().is_some());
    }

    #[cfg(windows)]
    #[test]
    fn a_group_start_reports_its_own_id_with_the_member_that_failed_and_its_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (_temp, app, profile_id) = npm_fixture("setInterval(() => {}, 1000);", port);
        let state = app.state::<AppState>();
        let group = state
            .storage
            .save_launch_group(LaunchGroupInput {
                id: None,
                name: "Full stack".into(),
                profile_ids: vec![profile_id.clone()],
            })
            .unwrap();

        let result = start_launch_group_inner(&group.id, app.handle(), &state).unwrap();

        assert_eq!(result.group_id, group.id);
        assert!(result.started_profile_ids.is_empty());
        assert_eq!(
            result.failed_profile_id.as_deref(),
            Some(profile_id.as_str())
        );
        assert_eq!(
            result.related_port,
            Some(RelatedPort {
                port,
                protocol: "tcp".into(),
            })
        );
        assert!(result
            .error
            .as_deref()
            .is_some_and(|message| message.contains(&port.to_string())));
        assert!(state.processes.info(&profile_id).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn restore_last_run_set_reports_the_detected_conflicting_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (_temp, app, profile_id) = npm_fixture("setInterval(() => {}, 1000);", port);
        let state = app.state::<AppState>();
        state
            .storage
            .save_restore_set(std::slice::from_ref(&profile_id))
            .unwrap();

        let result = restore_last_run_set_inner(app.handle(), &state).unwrap();

        assert!(result.started_profile_ids.is_empty());
        assert_eq!(
            result.failed_profile_id.as_deref(),
            Some(profile_id.as_str())
        );
        assert_eq!(
            result.related_port,
            Some(RelatedPort {
                port,
                protocol: "tcp".into(),
            })
        );
        assert!(result
            .error
            .as_deref()
            .is_some_and(|message| message.contains(&port.to_string())));
        assert!(state.processes.info(&profile_id).is_none());
        assert!(state.storage.list_sessions(10).unwrap().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn detects_conflict_from_real_tcp_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let profile = crate::models::LaunchProfile {
            id: "profile".into(),
            project_id: "project".into(),
            name: "dev".into(),
            program: "node.exe".into(),
            args: Vec::new(),
            cwd: std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            expected_ports: vec![crate::models::ExpectedPort {
                id: "port".into(),
                profile_id: "profile".into(),
                port,
                protocol: "tcp".into(),
            }],
            status: RunStatus::Idle,
            pid: None,
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some((found, _)) = first_conflict(&profile).unwrap() {
                assert_eq!(found.port, port);
                assert_eq!(found.protocol, "tcp");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "listener was not reported as a conflict"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[cfg(windows)]
    #[test]
    fn start_profile_returns_structured_conflict_without_starting_a_session() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (_temp, app, profile_id) = npm_fixture("setInterval(() => {}, 1000);", port);
        let state = app.state::<AppState>();

        let error = match start_profile_inner(&profile_id, app.handle(), &state) {
            Err(error) if error.related_port().is_some() => error,
            Err(error) => panic!("expected structured conflict, got {error}"),
            Ok(_) => panic!("occupied port unexpectedly started the profile"),
        };

        assert_eq!(
            error.related_port(),
            Some(&RelatedPort {
                port,
                protocol: "tcp".into(),
            })
        );
        assert!(error.to_string().contains(&port.to_string()));
        assert!(state.processes.info(&profile_id).is_none());
        assert!(state.storage.list_sessions(10).unwrap().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn manual_start_stays_starting_until_managed_expected_port_is_ready() {
        let port = unused_port();
        let script = r#"
const net = require('net');
setTimeout(() => {
  const server = net.createServer();
  server.listen(FIXTURE_PORT, '127.0.0.1');
}, 5000);
setInterval(() => {}, 1000);
"#;
        let (_temp, app, profile_id) = npm_fixture(script, port);
        let app_handle = app.handle().clone();
        let worker_profile = profile_id.clone();
        let worker = std::thread::spawn(move || {
            let state = app_handle.state::<AppState>();
            start_profile_inner(&worker_profile, &app_handle, &state)
        });
        let state = app.state::<AppState>();
        let deadline = Instant::now() + Duration::from_secs(10);
        while (state.processes.status(&profile_id) != Some(RunStatus::Starting)
            || state.processes.info(&profile_id).is_none())
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            state.processes.status(&profile_id),
            Some(RunStatus::Starting)
        );
        let dashboard = state.dashboard().unwrap();
        let dashboard_profile = dashboard
            .projects
            .iter()
            .flat_map(|project| &project.profiles)
            .find(|profile| profile.id == profile_id)
            .unwrap();
        assert_eq!(dashboard_profile.status, RunStatus::Starting);
        assert!(dashboard_profile.pid.is_some());
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());

        let event = worker.join().unwrap().unwrap();
        assert_eq!(event.status, RunStatus::Running);
        assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());
        let restore = state.storage.restore_set().unwrap();
        assert_eq!(restore.profile_ids, vec![profile_id.clone()]);

        shutdown(&state).unwrap();
        assert_eq!(
            state.storage.restore_set().unwrap().profile_ids,
            vec![profile_id.clone()]
        );
        shutdown(&state).unwrap();
        assert_eq!(
            state.storage.restore_set().unwrap().profile_ids,
            vec![profile_id]
        );
    }

    #[cfg(windows)]
    #[test]
    fn process_manager_captures_npm_logs_exit_and_releases_port() {
        let port = unused_port();
        let script = r#"
const net = require('net');
const server = net.createServer();
server.listen(FIXTURE_PORT, '127.0.0.1', () => {
  console.log('fixture stdout line');
  console.error('fixture stderr line');
  process.stdout.write('fixture stdout tail');
  process.stderr.write('fixture stderr tail');
  setTimeout(() => server.close(() => process.exit(7)), 8000);
});
"#;
        let (_temp, app, profile_id) = npm_fixture(script, port);
        let state = app.state::<AppState>();
        let event = start_profile_inner(&profile_id, app.handle(), &state).unwrap();
        assert_eq!(event.status, RunStatus::Running);

        let deadline = Instant::now() + Duration::from_secs(15);
        while state.processes.info(&profile_id).is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(state.processes.info(&profile_id).is_none());
        let logs = state.processes.logs(&profile_id);
        assert!(logs.iter().any(|log| log.line == "fixture stdout line"));
        assert!(logs.iter().any(|log| log.line == "fixture stderr line"));
        assert!(logs.iter().any(|log| log.line == "fixture stdout tail"));
        assert!(logs.iter().any(|log| log.line == "fixture stderr tail"));

        let sessions = state.storage.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].exit_code, Some(7));
        assert_eq!(sessions[0].status, "exited");
        assert!(sessions[0].ended_at.is_some());
        assert!(state.processes.is_unexpected_exit(&profile_id));
        assert!(state.storage.restore_set().unwrap().profile_ids.is_empty());

        let deadline = Instant::now() + Duration::from_secs(3);
        while TcpStream::connect(("127.0.0.1", port)).is_ok() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }
}
