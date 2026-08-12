import {
  CircleHelp,
  FolderKanban,
  Gauge,
  Languages,
  RadioTower,
  RefreshCw,
  Shield,
  ShieldCheck,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api } from "./api";
import { CloseChoiceModal } from "./components/CloseChoiceModal";
import { ConfirmModal } from "./components/ConfirmModal";
import { HelpDrawer } from "./components/HelpDrawer";
import { IconButton } from "./components/IconButton";
import { LogDrawer } from "./components/LogDrawer";
import { OverviewView } from "./components/OverviewView";
import { PortsView } from "./components/PortsView";
import { ProjectModal } from "./components/ProjectModal";
import { ProjectsView } from "./components/ProjectsView";
import { I18nProvider, LANGUAGE_STORAGE_KEY, useI18n } from "./i18n";
import type { LanguagePreference, MessageKey } from "./i18n";
import { isLanguagePreference } from "./i18n/context";
import { getOpenableProfilePort } from "./profile-actions";
import type {
  DashboardSnapshot,
  CloseBehavior,
  DiscoveredProject,
  LaunchProfile,
  PortSnapshot,
  ProfileStatus,
  Project,
  ProjectInput,
  RunStatusEvent,
} from "./types";

type ActiveView = "overview" | "ports" | "projects";
type ProjectModalState = Project | "import" | null;

function normalizedProjectPath(path: string): string {
  return path.replace(/[\\/]+$/, "").toLowerCase();
}

const navItems: Array<{ id: ActiveView; labelKey: MessageKey; icon: typeof Gauge }> = [
  { id: "overview", labelKey: "nav.overview", icon: Gauge },
  { id: "ports", labelKey: "nav.ports", icon: RadioTower },
  { id: "projects", labelKey: "nav.projects", icon: FolderKanban },
];

function replaceProfileStatus(snapshot: DashboardSnapshot, event: RunStatusEvent): DashboardSnapshot {
  return {
    ...snapshot,
    projects: snapshot.projects.map((project) => ({
      ...project,
      profiles: project.profiles.map((profile) =>
        profile.id === event.profileId
          ? { ...profile, status: event.status, pid: event.pid }
          : profile,
      ),
    })),
  };
}

function preserveStatusesNewerThanSnapshot(
  next: DashboardSnapshot,
  statusEvents: ReadonlyMap<string, RunStatusEvent>,
): DashboardSnapshot {
  return {
    ...next,
    projects: next.projects.map((project) => ({
      ...project,
      profiles: project.profiles.map((profile) => {
        const event = statusEvents.get(profile.id);
        return event && event.timestamp > next.generatedAt
          ? { ...profile, status: event.status, pid: event.pid ?? null }
          : profile;
      }),
    })),
  };
}

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

function AppShell() {
  const { preference, setPreference, t, formatTime } = useI18n();
  const [activeView, setActiveView] = useState<ActiveView>("overview");
  const [snapshot, setSnapshot] = useState<DashboardSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busyProfileIds, setBusyProfileIds] = useState<Set<string>>(new Set());
  const [restoreBusy, setRestoreBusy] = useState(false);
  const [projectModal, setProjectModal] = useState<ProjectModalState>(null);
  const [projectToDelete, setProjectToDelete] = useState<Project | null>(null);
  const [projectDeleteBusy, setProjectDeleteBusy] = useState(false);
  const [discoverySuggestions, setDiscoverySuggestions] = useState<DiscoveredProject[] | null>(null);
  const [autoDiscoveryMode, setAutoDiscoveryMode] = useState(false);
  const [logSelection, setLogSelection] = useState<{ profileId: string; projectId: string } | null>(null);
  const [externalProcess, setExternalProcess] = useState<PortSnapshot | null>(null);
  const [terminateBusy, setTerminateBusy] = useState(false);
  const [quitOpen, setQuitOpen] = useState(false);
  const [quitBusy, setQuitBusy] = useState(false);
  const [closeChoiceOpen, setCloseChoiceOpen] = useState(false);
  const [closeChoiceBusy, setCloseChoiceBusy] = useState<Exclude<CloseBehavior, "ask"> | null>(null);
  const [rememberCloseChoice, setRememberCloseChoice] = useState(false);
  const [closeBehaviorBusy, setCloseBehaviorBusy] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [elevationOpen, setElevationOpen] = useState(false);
  const [elevationBusy, setElevationBusy] = useState(false);
  const [languageBusy, setLanguageBusy] = useState(false);
  const languageSynced = useRef(false);
  const snapshotRequest = useRef(0);
  const profileActionsInFlight = useRef(new Set<string>());
  const restoreInFlight = useRef(false);
  const projectDeleteInFlight = useRef(false);
  const shutdownInFlight = useRef(false);
  const closeChoiceRequested = useRef(false);
  const closeChoiceActionInFlight = useRef(false);
  const profileStatusEvents = useRef(new Map<string, RunStatusEvent>());
  const autoDiscoveryChecked = useRef(false);

  const loadSnapshot = useCallback(async (quiet = false) => {
    const request = ++snapshotRequest.current;
    if (!quiet) setRefreshing(true);
    try {
      const next = await api.getDashboardSnapshot();
      if (request !== snapshotRequest.current) return;
      const statusEvents = new Map(profileStatusEvents.current);
      setSnapshot(preserveStatusesNewerThanSnapshot(next, statusEvents));
      for (const [profileId, event] of profileStatusEvents.current) {
        if (event.timestamp <= next.generatedAt) profileStatusEvents.current.delete(profileId);
      }
      setError(null);
    } catch (reason) {
      if (request === snapshotRequest.current) {
        setError(t("error.operationFailed", { detail: errorMessage(reason) }));
      }
    } finally {
      if (request === snapshotRequest.current) {
        setLoading(false);
        setRefreshing(false);
      }
    }
  }, [t]);

  useEffect(() => {
    void loadSnapshot();
  }, [loadSnapshot]);

  const discoverKnownRoots = useCallback(async (interactive = false) => {
    if (!snapshot?.settings.recentDevelopmentRoot) {
      if (interactive) {
        setAutoDiscoveryMode(true);
        setProjectModal("import");
        setDiscoverySuggestions(null);
        setNotice(t("projects.autoDiscoveryChooseRoot"));
      }
      return;
    }
    const registered = new Set((snapshot?.projects ?? []).map((project) => normalizedProjectPath(project.path)));
    try {
      const suggestions = (await api.scanSavedDevelopmentRoot())
        .filter((project) => project.profiles.length > 0 && !registered.has(normalizedProjectPath(project.path)));
      if (suggestions.length > 0) {
        setAutoDiscoveryMode(true);
        setDiscoverySuggestions(suggestions);
        if (interactive) {
          setProjectModal("import");
          setActiveView("projects");
        } else {
          setNotice(t("projects.autoDiscoveryFound", { count: suggestions.length }));
        }
      } else if (interactive) {
        setNotice(t("projects.autoDiscoveryEmpty"));
      }
    } catch {
      if (interactive) setError(t("projects.autoDiscoveryError"));
    }
  }, [snapshot?.projects, snapshot?.settings.recentDevelopmentRoot, t]);

  useEffect(() => {
    if (!snapshot?.settings.recentDevelopmentRoot || autoDiscoveryChecked.current) return;
    autoDiscoveryChecked.current = true;
    void discoverKnownRoots();
  }, [discoverKnownRoots, snapshot]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api.onRunStatus((event) => {
      const previous = profileStatusEvents.current.get(event.profileId);
      if (previous && event.timestamp < previous.timestamp) return;
      profileStatusEvents.current.set(event.profileId, event);
      setSnapshot((current) => current ? replaceProfileStatus(current, event) : current);
      if (event.message) {
        if (event.status === "conflict" || event.status === "unknown" || event.unexpected) {
          setError(t("error.lifecycleDetail", { detail: event.message }));
        } else {
          setNotice(event.message);
        }
      }
    }).then((dispose) => {
      if (cancelled) dispose();
      else unlisten = dispose;
    }).catch((reason) => {
      if (!cancelled) {
        setError(t("error.operationFailed", { detail: errorMessage(reason) }));
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [t]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api.onLifecycleError((message) => {
      setError(t("error.lifecycleDetail", { detail: message }));
    }).then((dispose) => {
      if (cancelled) dispose();
      else unlisten = dispose;
    }).catch((reason) => {
      if (!cancelled) {
        setError(t("error.operationFailed", { detail: errorMessage(reason) }));
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [t]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api.onPortSnapshot((next) => {
      snapshotRequest.current += 1;
      const statusEvents = new Map(profileStatusEvents.current);
      setSnapshot(preserveStatusesNewerThanSnapshot(next, statusEvents));
      for (const [profileId, event] of profileStatusEvents.current) {
        if (event.timestamp <= next.generatedAt) profileStatusEvents.current.delete(profileId);
      }
      setLoading(false);
      setRefreshing(false);
    }).then((dispose) => {
      if (cancelled) dispose();
      else unlisten = dispose;
    }).catch((reason) => {
      if (!cancelled) setError(errorMessage(reason));
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!snapshot || languageSynced.current) return;
    languageSynced.current = true;
    const backendPreference = snapshot.settings.languagePreference;
    let localPreference: LanguagePreference | null = null;
    try {
      const storedPreference = window.localStorage.getItem(LANGUAGE_STORAGE_KEY);
      if (isLanguagePreference(storedPreference)) {
        localPreference = storedPreference;
      } else if (storedPreference !== null) {
        window.localStorage.removeItem(LANGUAGE_STORAGE_KEY);
      }
    } catch {
      // The backend remains the durable source when WebView storage is unavailable.
    }
    if (localPreference === null) {
      setPreference(backendPreference);
    } else if (backendPreference !== localPreference) {
      void api.setLanguagePreference(localPreference).catch((reason) => setError(errorMessage(reason)));
    }
  }, [preference, setPreference, snapshot]);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 3_200);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const runProfileAction = async (
    profileId: string,
    action: (profileId: string) => Promise<RunStatusEvent>,
    completedMessage: MessageKey,
  ) => {
    if (restoreInFlight.current || profileActionsInFlight.current.has(profileId)) return;
    profileActionsInFlight.current.add(profileId);
    setBusyProfileIds((current) => new Set(current).add(profileId));
    try {
      const event = await action(profileId);
      setSnapshot((current) => current ? replaceProfileStatus(current, event) : current);
      setNotice(t(completedMessage));
      await loadSnapshot(true);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      profileActionsInFlight.current.delete(profileId);
      setBusyProfileIds((current) => {
        const next = new Set(current);
        next.delete(profileId);
        return next;
      });
    }
  };

  const restoreLastRunSet = useCallback(async () => {
    if (restoreInFlight.current) return;
    restoreInFlight.current = true;
    setRestoreBusy(true);
    try {
      const result = await api.restoreLastRunSet();
      await loadSnapshot(true);
      if (result.error) {
        setError(t("error.restorePartial", {
          count: result.startedProfileIds.length,
          profile: result.failedProfileId ?? "-",
          detail: result.error,
        }));
      } else {
        setNotice(t("notice.restored", { count: result.startedProfileIds.length }));
      }
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      restoreInFlight.current = false;
      setRestoreBusy(false);
    }
  }, [loadSnapshot, t]);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    void api.onTrayRestoreRequested(() => void restoreLastRunSet()).then((dispose) => {
      if (cancelled) dispose();
      else unlisteners.push(dispose);
    }).catch((reason) => {
      if (!cancelled) setError(errorMessage(reason));
    });
    void api.onTrayQuitRequested(() => {
      if (shutdownInFlight.current) return;
      setQuitOpen(true);
    }).then((dispose) => {
      if (cancelled) dispose();
      else unlisteners.push(dispose);
    }).catch((reason) => {
      if (!cancelled) setError(errorMessage(reason));
    });
    void api.onWindowCloseChoiceRequested(() => {
      if (shutdownInFlight.current || closeChoiceRequested.current) return;
      closeChoiceRequested.current = true;
      setRememberCloseChoice(false);
      setCloseChoiceOpen(true);
    }).then((dispose) => {
      if (cancelled) dispose();
      else unlisteners.push(dispose);
    }).catch((reason) => {
      if (!cancelled) setError(errorMessage(reason));
    });
    return () => {
      cancelled = true;
      unlisteners.forEach((dispose) => dispose());
    };
  }, [restoreLastRunSet]);

  const saveProject = async (input: ProjectInput) => {
    await api.saveProject(input);
    await loadSnapshot(true);
    setProjectModal(null);
    setNotice(t(input.id ? "notice.projectUpdated" : "notice.projectImported"));
  };

  const saveProjects = async (inputs: ProjectInput[]) => {
    let imported = 0;
    try {
      for (const input of inputs) {
        await api.saveProject(input);
        imported += 1;
      }
    } catch (reason) {
      await loadSnapshot(true);
      const detail = errorMessage(reason);
      throw new Error(
        imported > 0
          ? t("error.partialImport", { count: imported, detail })
          : detail,
      );
    }
    await loadSnapshot(true);
    setProjectModal(null);
    setDiscoverySuggestions(null);
    setAutoDiscoveryMode(false);
    setNotice(t("notice.projectsImported", { count: inputs.length }));
  };

  const deleteProject = async (projectId: string) => {
    await api.deleteProject(projectId);
    await loadSnapshot(true);
    setProjectModal(null);
    setNotice(t("notice.projectDeleted"));
  };

  const confirmProjectDelete = async () => {
    if (!projectToDelete || projectDeleteInFlight.current) return;
    projectDeleteInFlight.current = true;
    setProjectDeleteBusy(true);
    try {
      await deleteProject(projectToDelete.id);
      setProjectToDelete(null);
    } catch (reason) {
      setError(t("error.operationFailed", { detail: errorMessage(reason) }));
    } finally {
      projectDeleteInFlight.current = false;
      setProjectDeleteBusy(false);
    }
  };

  const openProfilePort = async (profile: LaunchProfile) => {
    const expected = snapshot ? getOpenableProfilePort(profile, snapshot.ports) : null;
    if (!expected) return;
    try {
      await api.openPort(expected.port, expected.protocol);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  const openPort = async (port: PortSnapshot) => {
    try {
      await api.openPort(port.port, port.protocol);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  const terminateExternal = async () => {
    if (
      externalProcess?.pid == null ||
      externalProcess.processStartedAt == null ||
      !externalProcess.executablePath
    ) return;
    setTerminateBusy(true);
    try {
      await api.terminateExternalProcess({
        port: externalProcess.port,
        protocol: externalProcess.protocol,
        pid: externalProcess.pid,
        startedAt: externalProcess.processStartedAt,
        executablePath: externalProcess.executablePath,
      });
      setExternalProcess(null);
      setNotice(t("notice.processTerminated", { pid: externalProcess.pid }));
      await loadSnapshot(true);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setTerminateBusy(false);
    }
  };

  const confirmPortAssociation = async (port: PortSnapshot) => {
    if (
      !port.projectId ||
      port.pid == null ||
      port.processStartedAt == null ||
      !port.executablePath
    ) return;
    try {
      await api.confirmPortAssociation({
        port: port.port,
        protocol: port.protocol,
        projectId: port.projectId,
        profileId: port.profileId,
        pid: port.pid,
        startedAt: port.processStartedAt,
        executablePath: port.executablePath,
      });
      setNotice(t("notice.associationConfirmed", { port: port.port }));
      await loadSnapshot(true);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  const openDirectory = async (projectId: string) => {
    try {
      await api.openProjectDirectory(projectId);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  const quitApp = async () => {
    if (shutdownInFlight.current) return;
    shutdownInFlight.current = true;
    setQuitBusy(true);
    try {
      await api.shutdownApp();
      setQuitOpen(false);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      shutdownInFlight.current = false;
      setQuitBusy(false);
    }
  };

  const chooseWindowCloseBehavior = async (behavior: Exclude<CloseBehavior, "ask">) => {
    if (closeChoiceActionInFlight.current || shutdownInFlight.current) return;
    closeChoiceActionInFlight.current = true;
    if (behavior === "quit") shutdownInFlight.current = true;
    setCloseChoiceBusy(behavior);
    let savingPreference = false;
    try {
      if (rememberCloseChoice) {
        savingPreference = true;
        const settings = await api.setCloseBehavior(behavior);
        setSnapshot((current) => current ? { ...current, settings } : current);
        savingPreference = false;
      }
      if (behavior === "hideToTray") {
        await api.hideToTray();
      } else {
        await api.shutdownApp();
      }
      closeChoiceRequested.current = false;
      setCloseChoiceOpen(false);
      setRememberCloseChoice(false);
    } catch (reason) {
      setError(
        savingPreference
          ? t("error.closeBehaviorSave", { detail: errorMessage(reason) })
          : t("error.operationFailed", { detail: errorMessage(reason) }),
      );
    } finally {
      closeChoiceActionInFlight.current = false;
      if (behavior === "quit") shutdownInFlight.current = false;
      setCloseChoiceBusy(null);
    }
  };

  const resetCloseBehavior = async () => {
    if (closeBehaviorBusy) return;
    setCloseBehaviorBusy(true);
    try {
      const settings = await api.setCloseBehavior("ask");
      setSnapshot((current) => current ? { ...current, settings } : current);
      setNotice(t("notice.closeBehaviorReset"));
    } catch (reason) {
      setError(t("error.closeBehaviorSave", { detail: errorMessage(reason) }));
    } finally {
      setCloseBehaviorBusy(false);
    }
  };

  const requestElevatedMonitoring = async () => {
    setElevationBusy(true);
    try {
      await api.requestElevatedMonitoring();
      setElevationOpen(false);
      setNotice(t("notice.elevationRequested"));
      await loadSnapshot(true);
    } catch (reason) {
      setElevationOpen(false);
      setError(t("error.elevationFailed", { detail: errorMessage(reason) }));
    } finally {
      setElevationBusy(false);
    }
  };

  const changeLanguage = async (next: LanguagePreference) => {
    const previous = preference;
    setPreference(next);
    setLanguageBusy(true);
    try {
      await api.setLanguagePreference(next);
    } catch (reason) {
      setPreference(previous);
      setError(`${t("error.languageSave")}: ${errorMessage(reason)}`);
    } finally {
      setLanguageBusy(false);
    }
  };

  const selectedLog = useMemo(() => {
    if (!snapshot || !logSelection) return null;
    const project = snapshot.projects.find((item) => item.id === logSelection.projectId);
    const profile = project?.profiles.find((item) => item.id === logSelection.profileId);
    return project && profile ? { project, profile } : null;
  }, [logSelection, snapshot]);

  const effectiveBusyProfileIds = useMemo(() => {
    if (!restoreBusy) return busyProfileIds;
    return new Set(
      snapshot?.projects.flatMap((project) => project.profiles.map((profile) => profile.id)) ?? [],
    );
  }, [busyProfileIds, restoreBusy, snapshot]);

  const profileActionProps = {
    busyProfileIds: effectiveBusyProfileIds,
    onStart: (profileId: string) => void runProfileAction(profileId, api.startProfile, "notice.startCompleted"),
    onStop: (profileId: string) => void runProfileAction(profileId, api.stopProfile, "notice.stopCompleted"),
    onRestart: (profileId: string) => void runProfileAction(profileId, api.restartProfile, "notice.restartCompleted"),
    onOpenPort: (profile: LaunchProfile) => void openProfilePort(profile),
    onOpenDirectory: (projectId: string) => void openDirectory(projectId),
    onOpenLogs: (profile: LaunchProfile, project: Project) => setLogSelection({ profileId: profile.id, projectId: project.id }),
  };

  const counts = useMemo(() => {
    const profiles = snapshot?.projects.flatMap((project) => project.profiles) ?? [];
    const countStatus = (status: ProfileStatus) => profiles.filter((profile) => profile.status === status).length;
    return { running: countStatus("running"), conflicts: countStatus("conflict") };
  }, [snapshot]);
  const registeredProjectPaths = useMemo(
    () => snapshot?.projects.map((project) => project.path) ?? [],
    [snapshot?.projects],
  );

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand" aria-label="RunCove">
          <img className="brand-mark" src="/runcove.png" alt="" />
          <span>RunCove</span>
        </div>
        <nav className="primary-nav" aria-label={t("nav.label")}>
          {navItems.map((item) => {
            const Icon = item.icon;
            return (
              <button
                type="button"
                key={item.id}
                className={activeView === item.id ? "is-active" : ""}
                aria-current={activeView === item.id ? "page" : undefined}
                onClick={() => setActiveView(item.id)}
              >
                <Icon size={17} />
                <span>{t(item.labelKey)}</span>
              </button>
            );
          })}
        </nav>
        <div className="sidebar-status">
          <div><span className="sidebar-dot sidebar-dot--running" />{t("count.running", { count: counts.running })}</div>
          <div><span className="sidebar-dot sidebar-dot--conflict" />{t("count.conflicts", { count: counts.conflicts })}</div>
        </div>
      </aside>

      <main className="workspace">
        <header className="topbar">
          <div>
            <h1>{t(navItems.find((item) => item.id === activeView)?.labelKey ?? "nav.overview")}</h1>
            <span className="scan-status">
              <span className={snapshot?.scanError ? "scan-dot scan-dot--error" : "scan-dot"} />
              {snapshot ? t("scan.scanned", { time: formatTime(snapshot.generatedAt) }) : t("scan.connecting")}
            </span>
          </div>
          <div className="window-actions">
            <IconButton label={t("help.open")} onClick={() => setHelpOpen(true)}>
              <CircleHelp size={15} />
            </IconButton>
            <IconButton
              label={t(
                snapshot?.privilege.elevated
                  ? "privilege.enhanced"
                  : snapshot?.privilege.elevationAvailable
                    ? "privilege.standard"
                    : "privilege.unavailable",
              )}
              onClick={() => setElevationOpen(true)}
              disabled={!snapshot || snapshot.privilege.elevated || !snapshot.privilege.elevationAvailable}
              tone={snapshot?.privilege.elevated ? "success" : "default"}
            >
              {snapshot?.privilege.elevated ? <ShieldCheck size={15} /> : <Shield size={15} />}
            </IconButton>
            <span className="toolbar-divider" />
            <label className="language-picker" title={t("language.label")}>
              <Languages size={14} aria-hidden="true" />
              <span className="sr-only">{t("language.label")}</span>
              <select
                aria-label={t("language.label")}
                value={preference}
                disabled={languageBusy}
                onChange={(event) => void changeLanguage(event.target.value as LanguagePreference)}
              >
                <option value="system">{t("language.system")}</option>
                <option value="en">{t("language.english")}</option>
                <option value="zh-CN">{t("language.chinese")}</option>
              </select>
            </label>
            <span className="toolbar-divider" />
            <IconButton label={t("scan.refresh")} onClick={() => void loadSnapshot()} disabled={refreshing} className={refreshing ? "is-spinning" : ""}>
              <RefreshCw size={15} />
            </IconButton>
          </div>
        </header>

        {snapshot?.privilege.monitorOnly && (
          <div className="monitor-only-banner" role="status">
            <ShieldCheck size={16} aria-hidden="true" />
            <div>
              <strong>{t("privilege.monitorOnlyTitle")}</strong>
              <span>{t("privilege.monitorOnlyDetail")}</span>
            </div>
          </div>
        )}

        <div className="content-area">
          {loading && <div className="loading-state"><RefreshCw size={20} className="is-spinning" /> {t("app.loading")}</div>}
          {!loading && !snapshot && <div className="error-state" role="alert"><strong>{t("app.unavailable")}</strong><span>{error}</span><button className="button button--secondary" onClick={() => void loadSnapshot()}>{t("app.retry")}</button></div>}
          {snapshot && snapshot.scanError && <div className="scan-error" role="status">{t("scan.degraded", { detail: snapshot.scanError })}</div>}
          {snapshot && activeView === "overview" && (
            <OverviewView snapshot={snapshot} restoreBusy={restoreBusy} onRestore={() => void restoreLastRunSet()} {...profileActionProps} />
          )}
          {snapshot && activeView === "ports" && (
            <PortsView
              snapshot={snapshot}
              busyProfileIds={effectiveBusyProfileIds}
              onOpenPort={(port) => void openPort(port)}
              onTerminate={setExternalProcess}
              onConfirmAssociation={(port) => void confirmPortAssociation(port)}
              onStartProfile={(profileId) => void runProfileAction(profileId, api.startProfile, "notice.startCompleted")}
            />
          )}
          {snapshot && activeView === "projects" && (
            <ProjectsView
              projects={snapshot.projects}
              ports={snapshot.ports}
              onImport={() => {
                setAutoDiscoveryMode(false);
                setDiscoverySuggestions(null);
                setProjectModal("import");
              }}
              onAutoDiscover={() => {
                if (discoverySuggestions && discoverySuggestions.length > 0) {
                  setAutoDiscoveryMode(true);
                  setProjectModal("import");
                } else {
                  void discoverKnownRoots(true);
                }
              }}
              discoveredCount={discoverySuggestions?.length ?? 0}
              hasSavedDiscoveryRoot={Boolean(snapshot.settings.recentDevelopmentRoot)}
              monitorOnly={snapshot.privilege.monitorOnly}
              onEdit={setProjectModal}
              onDelete={setProjectToDelete}
              {...profileActionProps}
            />
          )}
        </div>
      </main>

      <div
        className={closeChoiceOpen ? "background-overlays background-overlays--suspended" : "background-overlays"}
        aria-hidden={closeChoiceOpen || undefined}
      >
        {projectModal && !projectToDelete && !quitOpen && !elevationOpen && !helpOpen && (
          <ProjectModal
          project={projectModal === "import" ? null : projectModal}
          initialImportMode={autoDiscoveryMode ? "root" : undefined}
          initialRoot={autoDiscoveryMode ? snapshot?.settings.recentDevelopmentRoot ?? "" : undefined}
          initialRootProjects={discoverySuggestions ?? undefined}
          onDiscover={api.discoverProject}
          onScanDevelopmentRoot={api.scanDevelopmentRoot}
          onPickDirectory={api.pickProjectDirectory}
          onSave={saveProject}
          onSaveMany={saveProjects}
          registeredPaths={registeredProjectPaths}
          onClose={() => {
            setProjectModal(null);
            setDiscoverySuggestions(null);
            setAutoDiscoveryMode(false);
          }}
          />
        )}
        {projectToDelete && !quitOpen && !elevationOpen && !helpOpen && (
          <ConfirmModal
            title={t("project.deleteProject")}
            detail={t("project.deleteWarning", { project: projectToDelete.name })}
            confirmLabel={t("project.deleteProject")}
            busy={projectDeleteBusy}
            onCancel={() => setProjectToDelete(null)}
            onConfirm={() => void confirmProjectDelete()}
          />
        )}
        {selectedLog && snapshot && !quitOpen && !elevationOpen && !helpOpen && (
          <LogDrawer api={api} profile={selectedLog.profile} project={selectedLog.project} capacity={snapshot.settings.logCapacity} onClose={() => setLogSelection(null)} />
        )}
        {externalProcess && !quitOpen && !elevationOpen && !helpOpen && (
          <ConfirmModal
          title={t("dialog.terminateTitle", { pid: externalProcess.pid ?? "-" })}
          detail={t("dialog.terminateDetail", {
            process: externalProcess.processName ?? t("dialog.unknownProcess"),
            address: externalProcess.bindAddress ?? t("dialog.unknownAddress"),
            port: externalProcess.port,
          })}
          confirmLabel={t("dialog.terminateConfirm")}
          busy={terminateBusy}
          onCancel={() => setExternalProcess(null)}
          onConfirm={() => void terminateExternal()}
          />
        )}
        {elevationOpen && !quitOpen && !helpOpen && (
          <ConfirmModal
          title={t("dialog.elevationTitle")}
          detail={t("dialog.elevationDetail")}
          confirmLabel={t("dialog.elevationConfirm")}
          busy={elevationBusy}
          danger={false}
          onCancel={() => setElevationOpen(false)}
          onConfirm={() => void requestElevatedMonitoring()}
          />
        )}
        {helpOpen && !quitOpen && (
          <HelpDrawer
            initialTopic={activeView === "ports" ? "ports" : activeView === "projects" ? "projects" : "quickStart"}
            closeBehavior={snapshot?.settings.closeBehavior ?? "ask"}
            closeBehaviorBusy={closeBehaviorBusy}
            onResetCloseBehavior={() => void resetCloseBehavior()}
            onClose={() => setHelpOpen(false)}
            onNavigate={(view) => {
              setActiveView(view);
              setHelpOpen(false);
            }}
          />
        )}
        {quitOpen && (
          <ConfirmModal
          title={t("dialog.quitTitle")}
          detail={t("dialog.quitDetail")}
          confirmLabel={t("dialog.quitConfirm")}
          busy={quitBusy}
          onCancel={() => setQuitOpen(false)}
          onConfirm={() => void quitApp()}
          />
        )}
      </div>
      {closeChoiceOpen && (
        <CloseChoiceModal
          remember={rememberCloseChoice}
          busyAction={closeChoiceBusy}
          onRememberChange={setRememberCloseChoice}
          onCancel={() => {
            closeChoiceRequested.current = false;
            setCloseChoiceOpen(false);
          }}
          onChoose={(behavior) => void chooseWindowCloseBehavior(behavior)}
        />
      )}
      {(error && snapshot || notice) && (
        <div className="toast-stack">
          {error && snapshot && (
            <div className="toast toast--error" role="alert">
              <span>{error}</span>
              <button type="button" onClick={() => setError(null)} aria-label={t("app.dismissError")}>×</button>
            </div>
          )}
          {notice && (
            <div className="toast toast--success" role="status">{notice}</div>
          )}
        </div>
      )}
    </div>
  );
}

export default function App() {
  return (
    <I18nProvider>
      <AppShell />
    </I18nProvider>
  );
}
