import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

import { createMockApi } from "./mock-data";
import { initialPreference, renderMessage, resolveLanguage } from "./i18n/context";
import type {
  AppSettings,
  ArchiveClosedEvent,
  ConfirmAssociationRequest,
  DashboardSnapshot,
  DiscoveredProject,
  ExternalProcessRequest,
  Project,
  ProjectInput,
  RestoreResult,
  RunCoveApi,
  RunLogArchivePage,
  RunLogArchiveState,
  RunLogEvent,
  RunSession,
  RunStatusEvent,
} from "./types";

const tauriApi: RunCoveApi = {
  getDashboardSnapshot: () => invoke<DashboardSnapshot>("get_dashboard_snapshot"),
  discoverProject: (directory) =>
    invoke<DiscoveredProject>("discover_project", { directory }),
  scanDevelopmentRoot: (directory) =>
    invoke<DiscoveredProject[]>("scan_development_root", { directory }),
  scanSavedDevelopmentRoot: () =>
    invoke<DiscoveredProject[]>("scan_saved_development_root"),
  saveProject: (project: ProjectInput) => invoke<Project>("save_project", { project }),
  deleteProject: (projectId) => invoke<void>("delete_project", { projectId }),
  startProfile: (profileId) =>
    invoke<RunStatusEvent>("start_profile", { profileId }),
  stopProfile: (profileId) => invoke<RunStatusEvent>("stop_profile", { profileId }),
  restartProfile: (profileId) =>
    invoke<RunStatusEvent>("restart_profile", { profileId }),
  restoreLastRunSet: () => invoke<RestoreResult>("restore_last_run_set"),
  terminateExternalProcess: (request: ExternalProcessRequest) =>
    invoke<void>("terminate_external_process", { request }),
  confirmPortAssociation: (request: ConfirmAssociationRequest) =>
    invoke<void>("confirm_port_association", { request }),
  clearLogs: (profileId) => invoke<void>("clear_logs", { profileId }),
  getLogs: (profileId) => invoke<RunLogEvent[]>("get_logs", { profileId }),
  getRunHistory: () => invoke<RunSession[]>("get_run_history"),
  setRunLogArchiving: (enabled) =>
    invoke<RunLogArchiveState>("set_run_log_archiving", { enabled }),
  // `null` rather than `undefined`: an omitted key and an explicit null both read
  // back as `None`, but only null survives the JSON boundary unambiguously.
  readRunLogArchive: (sessionId, beforeOffset, maxLines) =>
    invoke<RunLogArchivePage>("read_run_log_archive", {
      sessionId,
      beforeOffset: beforeOffset ?? null,
      maxLines: maxLines ?? null,
    }),
  deleteRunLogArchive: (sessionId) =>
    invoke<void>("delete_run_log_archive", { sessionId }),
  openPort: (port, protocol) => invoke<void>("open_port", { port, protocol }),
  openProjectDirectory: (projectId) =>
    invoke<void>("open_project_directory", { projectId }),
  shutdownApp: () => invoke<void>("shutdown_app"),
  hideToTray: () => invoke<void>("hide_to_tray"),
  setCloseBehavior: (behavior) =>
    invoke<AppSettings>("set_close_behavior", { closeBehavior: behavior }),
  setLanguagePreference: (preference) =>
    invoke<AppSettings>("set_language_preference", { languagePreference: preference }),
  requestElevatedMonitoring: () => invoke<void>("request_elevated_monitoring"),
  async pickProjectDirectory() {
    const selected = await open({ directory: true, multiple: false });
    return typeof selected === "string" ? selected : null;
  },
  async onRunStatus(handler) {
    return listen<RunStatusEvent>("run-status", (event) => handler(event.payload));
  },
  async onRunLog(handler) {
    return listen<RunLogEvent>("run-log", (event) => handler(event.payload));
  },
  async onArchiveClosed(handler) {
    return listen<ArchiveClosedEvent>("run-archive-closed", (event) => handler(event.payload));
  },
  async onPortSnapshot(handler) {
    return listen<DashboardSnapshot>("port-snapshot", (event) => handler(event.payload));
  },
  async onLifecycleError(handler) {
    const listeners = await Promise.all([
      listen<{ message: string }>("tray-stop-all-error", (event) => handler(event.payload.message)),
      listen<{ message: string }>("tray-language-update-error", (event) =>
        handler(
          renderMessage(resolveLanguage(initialPreference()), "error.languageTray", {
            detail: event.payload.message,
          }),
        ),
      ),
      listen<{ message: string }>("shutdown-error", (event) => handler(event.payload.message)),
      listen<{ message: string }>("dashboard-refresh-error", (event) =>
        handler(event.payload.message),
      ),
      listen<RunStatusEvent>("process-lifecycle-error", (event) =>
        handler(
          event.payload.message ??
            renderMessage(resolveLanguage(initialPreference()), "error.lifecycle"),
        ),
      ),
    ]);
    return () => listeners.forEach((dispose) => dispose());
  },
  async onTrayRestoreRequested(handler) {
    return listen<void>("tray-restore-requested", () => handler());
  },
  async onTrayQuitRequested(handler) {
    return listen<void>("tray-quit-requested", () => handler());
  },
  async onWindowCloseChoiceRequested(handler) {
    return listen<void>("window-close-choice-requested", () => handler());
  },
};

export const isTauri = typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
const mockApi = createMockApi();
export const api: RunCoveApi = isTauri ? tauriApi : mockApi;

export function resetMockApi() {
  if (!isTauri) mockApi.reset();
}
