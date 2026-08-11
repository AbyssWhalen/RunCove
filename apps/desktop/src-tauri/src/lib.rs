mod commands;
mod discovery;
mod error;
mod import_observation;
mod language;
mod models;
mod privileges;
mod processes;
mod single_instance;
mod state;
mod storage;

use crate::commands::{
    clear_logs, confirm_port_association, delete_project, discover_project, get_app_settings,
    get_dashboard_snapshot, get_logs, get_run_history, hide_to_tray, open_port,
    open_project_directory, request_elevated_monitoring, restart_profile, restore_last_run_set,
    save_project, scan_development_root, scan_saved_development_root, set_close_behavior,
    set_language_preference, shutdown, shutdown_app, start_profile, stop_profile,
    terminate_external_process,
};
use crate::language::{tray_labels, tray_status_text, DisplayLanguage};
use crate::models::{CloseBehavior, LanguagePreference};
use crate::processes::ProcessManager;
use crate::state::AppState;
use crate::storage::Storage;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, RunEvent, WindowEvent};

const TRAY_STOP_ALL_ERROR_EVENT: &str = "tray-stop-all-error";
const TRAY_LANGUAGE_ERROR_EVENT: &str = "tray-language-update-error";
const DASHBOARD_REFRESH_ERROR_EVENT: &str = "dashboard-refresh-error";
const SHUTDOWN_ERROR_EVENT: &str = "shutdown-error";
const WINDOW_CLOSE_CHOICE_EVENT: &str = "window-close-choice-requested";
const INSTANCE_MUTEX_NAME: &str = r"Local\RunCove.com.abysswhale.runcove.v1";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LifecycleActionError {
    action: &'static str,
    message: String,
    timestamp: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowCloseDecision {
    Ask,
    HideToTray,
    Quit,
}

fn window_close_decision(behavior: CloseBehavior) -> WindowCloseDecision {
    match behavior {
        CloseBehavior::Ask => WindowCloseDecision::Ask,
        CloseBehavior::HideToTray => WindowCloseDecision::HideToTray,
        CloseBehavior::Quit => WindowCloseDecision::Quit,
    }
}

fn emit_lifecycle_error(app: &tauri::AppHandle, action: &'static str, message: String) {
    let _ = app.emit(
        SHUTDOWN_ERROR_EVENT,
        LifecycleActionError {
            action,
            message,
            timestamp: crate::storage::now_ms(),
        },
    );
}

struct TrayItems {
    status: MenuItem<tauri::Wry>,
    open: MenuItem<tauri::Wry>,
    restore: MenuItem<tauri::Wry>,
    stop_all: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

#[derive(Clone, Copy)]
struct TrayRuntime {
    language: DisplayLanguage,
    summary: Option<StatusSummary>,
    monitor_only: bool,
}

struct TrayUiState {
    items: TrayItems,
    runtime: Mutex<TrayRuntime>,
}

fn update_tray_runtime<T>(
    runtime: &Mutex<TrayRuntime>,
    update: impl FnOnce(&mut TrayRuntime),
    apply: impl FnOnce(TrayRuntime) -> T,
) -> T {
    let snapshot = {
        let mut runtime = runtime.lock().expect("tray state mutex poisoned");
        update(&mut runtime);
        *runtime
    };
    apply(snapshot)
}

impl TrayUiState {
    fn set_language(
        &self,
        app: &tauri::AppHandle,
        language: DisplayLanguage,
    ) -> crate::error::AppResult<()> {
        update_tray_runtime(
            &self.runtime,
            |runtime| runtime.language = language,
            |runtime| self.apply_text(app, runtime),
        )
    }

    fn set_summary(
        &self,
        app: &tauri::AppHandle,
        summary: StatusSummary,
    ) -> crate::error::AppResult<()> {
        update_tray_runtime(
            &self.runtime,
            |runtime| runtime.summary = Some(summary),
            |runtime| self.apply_text(app, runtime),
        )
    }

    fn apply_text(
        &self,
        app: &tauri::AppHandle,
        runtime: TrayRuntime,
    ) -> crate::error::AppResult<()> {
        let labels = tray_labels(runtime.language);
        let restore_text = if runtime.monitor_only {
            labels.restore_monitor_only
        } else {
            labels.restore
        };
        let stop_all_text = if runtime.monitor_only {
            labels.stop_all_monitor_only
        } else {
            labels.stop_all
        };
        self.items.open.set_text(labels.open).map_err(|error| {
            crate::error::invalid(format!("Could not update tray menu: {error}"))
        })?;
        self.items.restore.set_text(restore_text).map_err(|error| {
            crate::error::invalid(format!("Could not update tray menu: {error}"))
        })?;
        self.items
            .stop_all
            .set_text(stop_all_text)
            .map_err(|error| {
                crate::error::invalid(format!("Could not update tray menu: {error}"))
            })?;
        self.items.quit.set_text(labels.quit).map_err(|error| {
            crate::error::invalid(format!("Could not update tray menu: {error}"))
        })?;
        self.items
            .restore
            .set_enabled(!runtime.monitor_only)
            .map_err(|error| {
                crate::error::invalid(format!("Could not update tray menu: {error}"))
            })?;
        self.items
            .stop_all
            .set_enabled(!runtime.monitor_only)
            .map_err(|error| {
                crate::error::invalid(format!("Could not update tray menu: {error}"))
            })?;
        let status_text = tray_status_text_for_mode(
            runtime.language,
            runtime.summary.unwrap_or_default(),
            runtime.monitor_only,
        );
        self.items.status.set_text(&status_text).map_err(|error| {
            crate::error::invalid(format!("Could not update tray status: {error}"))
        })?;
        if let Some(tray) = app.tray_by_id("main-tray") {
            let tooltip = runtime
                .summary
                .map(|_| format!("RunCove - {status_text}"))
                .unwrap_or_else(|| labels.tagline.into());
            tray.set_tooltip(Some(tooltip)).map_err(|error| {
                crate::error::invalid(format!("Could not update tray tooltip: {error}"))
            })?;
        }
        Ok(())
    }
}

pub(crate) fn refresh_tray_language(
    app: &tauri::AppHandle,
    preference: LanguagePreference,
) -> crate::error::AppResult<()> {
    let tray = app
        .try_state::<TrayUiState>()
        .ok_or_else(|| crate::error::invalid("Tray state is unavailable"))?;
    tray.set_language(app, language::resolve(preference))
}

pub(crate) fn emit_tray_language_update_error(app: &tauri::AppHandle, message: String) {
    let _ = app.emit(
        TRAY_LANGUAGE_ERROR_EVENT,
        LifecycleActionError {
            action: "language",
            message,
            timestamp: crate::storage::now_ms(),
        },
    );
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let mut instance_guard = if std::env::args().any(|arg| arg == "--elevated-monitor") {
                privileges::validate_elevated_relaunch()?;
                single_instance::SingleInstanceGuard::acquire_after_previous(
                    INSTANCE_MUTEX_NAME,
                    Duration::from_secs(15),
                )?
            } else {
                single_instance::SingleInstanceGuard::acquire(INSTANCE_MUTEX_NAME)?
            };
            let app_handle = app.handle().clone();
            instance_guard.start_wake_listener(move || show_main_window(&app_handle))?;
            app.manage(instance_guard);
            let data_dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let storage = Arc::new(Storage::open(&data_dir.join("runcove.sqlite3"))?);
            let settings = storage.settings()?;
            let processes = Arc::new(ProcessManager::new(settings.log_capacity));
            app.manage(AppState { storage, processes });
            let tray = build_tray(app, language::resolve(settings.language_preference))?;
            app.manage(tray);
            start_snapshot_loop(app.handle().clone(), settings.poll_interval_ms);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_dashboard_snapshot,
            get_app_settings,
            discover_project,
            scan_development_root,
            scan_saved_development_root,
            save_project,
            delete_project,
            start_profile,
            stop_profile,
            restart_profile,
            restore_last_run_set,
            request_elevated_monitoring,
            terminate_external_process,
            confirm_port_association,
            clear_logs,
            get_logs,
            get_run_history,
            open_port,
            open_project_directory,
            hide_to_tray,
            set_close_behavior,
            set_language_preference,
            shutdown_app,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build RunCove desktop application");

    app.run(|app, event| match event {
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } => {
            if label == "main" {
                let state = app.state::<AppState>();
                if state.processes.shutdown_is_in_progress() {
                    api.prevent_close();
                    return;
                }
                api.prevent_close();
                let behavior = match state.storage.settings() {
                    Ok(settings) => settings.close_behavior,
                    Err(error) => {
                        show_main_window(app);
                        emit_lifecycle_error(app, "closeBehavior", error.to_string());
                        CloseBehavior::Ask
                    }
                };
                match window_close_decision(behavior) {
                    WindowCloseDecision::Ask => {
                        show_main_window(app);
                        let _ = app.emit(WINDOW_CLOSE_CHOICE_EVENT, ());
                    }
                    WindowCloseDecision::HideToTray => {
                        if let Err(error) = hide_to_tray(app.clone()) {
                            show_main_window(app);
                            emit_lifecycle_error(app, "hideToTray", error.to_string());
                        }
                    }
                    WindowCloseDecision::Quit => {
                        spawn_native_shutdown(app, &state);
                    }
                }
            }
        }
        RunEvent::ExitRequested { api, .. } => {
            let state = app.state::<AppState>();
            if let Err(error) = shutdown(&state) {
                api.prevent_exit();
                show_main_window(app);
                emit_lifecycle_error(app, "shutdown", error.to_string());
            }
        }
        _ => {}
    });
}

fn build_tray(
    app: &tauri::App,
    language: DisplayLanguage,
) -> Result<TrayUiState, Box<dyn std::error::Error>> {
    let labels = tray_labels(language);
    let monitor_only = privileges::current_status()?.monitor_only;
    let status = MenuItem::with_id(
        app,
        "status",
        tray_status_text_for_mode(language, StatusSummary::default(), monitor_only),
        false,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, "open", labels.open, true, None::<&str>)?;
    let restore = MenuItem::with_id(
        app,
        "restore",
        if monitor_only {
            labels.restore_monitor_only
        } else {
            labels.restore
        },
        !monitor_only,
        None::<&str>,
    )?;
    let stop_all = MenuItem::with_id(
        app,
        "stop_all",
        if monitor_only {
            labels.stop_all_monitor_only
        } else {
            labels.stop_all
        },
        !monitor_only,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", labels.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&status, &open, &restore, &stop_all, &quit])?;
    let mut builder = TrayIconBuilder::with_id("main-tray")
        .tooltip(labels.tagline)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "restore" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("tray-restore-requested", ());
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "stop_all" => {
                if let Err(error) = stop_all_from_tray(&app.state::<AppState>()) {
                    show_main_window(app);
                    let _ = app.emit(
                        TRAY_STOP_ALL_ERROR_EVENT,
                        LifecycleActionError {
                            action: "stopAll",
                            message: error.to_string(),
                            timestamp: crate::storage::now_ms(),
                        },
                    );
                }
            }
            "quit" => {
                show_main_window(app);
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("tray-quit-requested", ());
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(TrayUiState {
        items: TrayItems {
            status,
            open,
            restore,
            stop_all,
            quit,
        },
        runtime: Mutex::new(TrayRuntime {
            language,
            summary: None,
            monitor_only,
        }),
    })
}

fn stop_all_from_tray(state: &AppState) -> crate::error::AppResult<()> {
    privileges::ensure_process_action_allowed()?;
    let reservation = state.processes.reserve_shutdown()?;
    let stop_result = state
        .processes
        .stop_all_by_user_and_wait(&reservation, Duration::from_secs(8));
    let save_result = if stop_result.is_ok() {
        state.storage.save_restore_set(&[])
    } else {
        Ok(())
    };
    drop(reservation);

    match (stop_result, save_result) {
        (Ok(()), Ok(())) => {
            state.processes.clear_shutdown_snapshot();
            Ok(())
        }
        (Ok(()), Err(save)) => Err(save),
        (Err(stop), _) => {
            let active = state.processes.active_profile_ids();
            match state.storage.save_restore_set(&active) {
                Ok(()) => Err(stop),
                Err(save) => Err(crate::error::invalid(format!(
                    "{stop}; could not synchronize the remaining restore set: {save}"
                ))),
            }
        }
    }
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn spawn_native_shutdown(app: &tauri::AppHandle, state: &AppState) {
    let app = app.clone();
    let state = AppState {
        storage: state.storage.clone(),
        processes: state.processes.clone(),
    };
    tauri::async_runtime::spawn(async move {
        let result = tauri::async_runtime::spawn_blocking(move || shutdown(&state)).await;
        match result {
            Ok(Ok(())) => app.exit(0),
            Ok(Err(error)) => {
                show_main_window(&app);
                emit_lifecycle_error(&app, "shutdown", error.to_string());
            }
            Err(error) => {
                show_main_window(&app);
                emit_lifecycle_error(
                    &app,
                    "shutdown",
                    format!("Background shutdown failed: {error}"),
                );
            }
        }
    });
}

fn start_snapshot_loop(app: tauri::AppHandle, interval_ms: u64) {
    std::thread::spawn(move || {
        let mut last_error = None;
        loop {
            std::thread::sleep(Duration::from_millis(interval_ms.max(500)));
            let state = app.state::<AppState>();
            match state.dashboard() {
                Ok(snapshot) => {
                    last_error = None;
                    let summary = status_summary(&snapshot.projects, |profile_id| {
                        state.processes.is_unexpected_exit(profile_id)
                    });
                    let _ = app.state::<TrayUiState>().set_summary(&app, summary);
                    let _ = app.emit("port-snapshot", snapshot);
                }
                Err(error) => {
                    let message = error.to_string();
                    if last_error.as_deref() != Some(message.as_str()) {
                        let _ = app.emit(
                            DASHBOARD_REFRESH_ERROR_EVENT,
                            LifecycleActionError {
                                action: "refresh-dashboard",
                                message: message.clone(),
                                timestamp: crate::storage::now_ms(),
                            },
                        );
                    }
                    last_error = Some(message);
                }
            }
        }
    });
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StatusSummary {
    running: usize,
    conflicts: usize,
    unexpected_exits: usize,
}

fn tray_status_text_for(language: DisplayLanguage, summary: StatusSummary) -> String {
    tray_status_text(
        language,
        summary.running,
        summary.conflicts,
        summary.unexpected_exits,
    )
}

fn tray_status_text_for_mode(
    language: DisplayLanguage,
    summary: StatusSummary,
    monitor_only: bool,
) -> String {
    let status = tray_status_text_for(language, summary);
    if monitor_only {
        format!("{} | {status}", tray_labels(language).monitor_only_status)
    } else {
        status
    }
}

fn status_summary(
    projects: &[crate::models::Project],
    is_unexpected_exit: impl Fn(&str) -> bool,
) -> StatusSummary {
    let mut summary = StatusSummary::default();
    for profile in projects.iter().flat_map(|project| &project.profiles) {
        match profile.status {
            crate::models::RunStatus::Running => summary.running += 1,
            crate::models::RunStatus::Conflict => summary.conflicts += 1,
            crate::models::RunStatus::Exited if is_unexpected_exit(&profile.id) => {
                summary.unexpected_exits += 1;
            }
            _ => {}
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CloseBehavior, ExpectedPortInput, LaunchProfile, LaunchProfileInput, Project, ProjectInput,
        RunStatus,
    };

    #[test]
    fn window_close_behavior_maps_to_one_explicit_native_decision() {
        assert_eq!(
            window_close_decision(CloseBehavior::Ask),
            WindowCloseDecision::Ask
        );
        assert_eq!(
            window_close_decision(CloseBehavior::HideToTray),
            WindowCloseDecision::HideToTray
        );
        assert_eq!(
            window_close_decision(CloseBehavior::Quit),
            WindowCloseDecision::Quit
        );
    }

    #[test]
    fn tray_status_summary_counts_relevant_states() {
        let profiles = [RunStatus::Running, RunStatus::Conflict, RunStatus::Exited]
            .into_iter()
            .enumerate()
            .map(|(index, status)| LaunchProfile {
                id: index.to_string(),
                project_id: "project".into(),
                name: "profile".into(),
                program: "program".into(),
                args: Vec::new(),
                cwd: ".".into(),
                expected_ports: Vec::new(),
                status,
                pid: None,
            })
            .collect();
        let projects = vec![Project {
            id: "project".into(),
            name: "Project".into(),
            path: ".".into(),
            profiles,
            created_at: 0,
            updated_at: 0,
        }];
        assert_eq!(
            status_summary(&projects, |profile_id| profile_id == "2"),
            StatusSummary {
                running: 1,
                conflicts: 1,
                unexpected_exits: 1,
            }
        );
    }

    #[test]
    fn tray_status_text_exposes_all_counts_in_the_menu_label() {
        assert_eq!(
            tray_status_text_for(
                DisplayLanguage::English,
                StatusSummary {
                    running: 2,
                    conflicts: 1,
                    unexpected_exits: 3,
                }
            ),
            "2 running | 1 conflict | 3 unexpected exits"
        );
    }

    #[test]
    fn tray_runtime_lock_is_released_before_native_updates() {
        let runtime = Mutex::new(TrayRuntime {
            language: DisplayLanguage::English,
            summary: None,
            monitor_only: false,
        });

        update_tray_runtime(
            &runtime,
            |state| state.summary = Some(StatusSummary::default()),
            |_| assert!(runtime.try_lock().is_ok()),
        );

        assert!(runtime.lock().unwrap().summary.is_some());
    }

    #[test]
    fn tray_status_makes_monitor_only_mode_explicit() {
        let status =
            tray_status_text_for_mode(DisplayLanguage::English, StatusSummary::default(), true);
        assert!(status.starts_with("Administrator monitor-only |"));
    }

    #[test]
    fn successful_tray_stop_all_synchronizes_an_empty_restore_set() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Arc::new(Storage::open(&temp.path().join("tray-test.sqlite3")).unwrap());
        let project = storage
            .save_project(ProjectInput {
                id: None,
                name: "fixture".into(),
                path: temp.path().to_string_lossy().into_owned(),
                profiles: vec![LaunchProfileInput {
                    id: None,
                    name: "dev".into(),
                    program: "node.exe".into(),
                    args: Vec::new(),
                    cwd: temp.path().to_string_lossy().into_owned(),
                    expected_ports: vec![ExpectedPortInput {
                        id: None,
                        port: 1,
                        protocol: "tcp".into(),
                    }],
                }],
            })
            .unwrap();
        storage
            .save_restore_set(&[project.profiles[0].id.clone()])
            .unwrap();
        let state = AppState {
            storage: storage.clone(),
            processes: Arc::new(ProcessManager::new(10)),
        };

        stop_all_from_tray(&state).unwrap();

        assert!(storage.restore_set().unwrap().profile_ids.is_empty());
    }
}
