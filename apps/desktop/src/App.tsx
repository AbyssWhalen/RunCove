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
import { RunHistoryDrawer } from "./components/RunHistoryDrawer";
import type { RunHistoryLabels } from "./components/RunHistorySection";
import { RunLogArchiveDrawer } from "./components/RunLogArchiveDrawer";
import { describeArchive, formatArchiveSize } from "./components/archive";
import { resolveRunSession } from "./components/run-history";
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
  RunSession,
  RunStatusEvent,
} from "./types";

type ActiveView = "overview" | "ports" | "projects";
type ProjectModalState = Project | "import" | null;
type DiscoveryState = "idle" | "scanning" | "candidates" | "empty" | "error";
type PortFocus = { port: number; protocol: PortSnapshot["protocol"]; nonce: number };

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
  const { preference, setPreference, t, locale, formatTime } = useI18n();
  const [activeView, setActiveView] = useState<ActiveView>("overview");
  const [snapshot, setSnapshot] = useState<DashboardSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [runHistory, setRunHistory] = useState<RunSession[]>([]);
  const [runHistoryLoading, setRunHistoryLoading] = useState(false);
  const [runHistoryError, setRunHistoryError] = useState<string | null>(null);
  const [runHistoryDrawerOpen, setRunHistoryDrawerOpen] = useState(false);
  // The viewer keeps only the session id: the row it renders comes from `runHistory`,
  // so a history reload updates the badge instead of leaving a stale copy on screen.
  const [archiveViewerId, setArchiveViewerId] = useState<string | null>(null);
  const [archiveToDelete, setArchiveToDelete] = useState<RunSession | null>(null);
  const [archiveDeleteBusy, setArchiveDeleteBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busyProfileIds, setBusyProfileIds] = useState<Set<string>>(new Set());
  const [restoreBusy, setRestoreBusy] = useState(false);
  const [projectModal, setProjectModal] = useState<ProjectModalState>(null);
  const [projectToDelete, setProjectToDelete] = useState<Project | null>(null);
  const [projectDeleteBusy, setProjectDeleteBusy] = useState(false);
  const [discoverySuggestions, setDiscoverySuggestions] = useState<DiscoveredProject[] | null>(null);
  const [discoveryState, setDiscoveryState] = useState<DiscoveryState>("idle");
  const [discoveryError, setDiscoveryError] = useState<string | null>(null);
  const [autoDiscoveryMode, setAutoDiscoveryMode] = useState(false);
  const [portFocus, setPortFocus] = useState<PortFocus | null>(null);
  const [errorRelatedPort, setErrorRelatedPort] = useState<PortFocus | null>(null);
  const [focusedProjectId, setFocusedProjectId] = useState<string | null>(null);
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
  const archiveDeleteInFlight = useRef(false);
  const shutdownInFlight = useRef(false);
  const closeChoiceRequested = useRef(false);
  const closeChoiceActionInFlight = useRef(false);
  const profileStatusEvents = useRef(new Map<string, RunStatusEvent>());
  const autoDiscoveryChecked = useRef(false);
  const discoveryInFlight = useRef(false);
  const runHistoryLoaded = useRef(false);
  const runHistoryRequest = useRef(0);

  const showError = useCallback((message: string | null, relatedPort?: Omit<PortFocus, "nonce"> | null) => {
    setError(message);
    setErrorRelatedPort(message && relatedPort
      ? { ...relatedPort, nonce: Date.now() }
      : null);
  }, []);

  const loadRunHistory = useCallback(async () => {
    const request = ++runHistoryRequest.current;
    setRunHistoryLoading(true);
    try {
      const next = await api.getRunHistory();
      if (request !== runHistoryRequest.current) return;
      setRunHistory(next.slice(0, 200));
      setRunHistoryError(null);
      runHistoryLoaded.current = true;
    } catch (reason) {
      if (request === runHistoryRequest.current) {
        setRunHistoryError(t("history.loadError", { detail: errorMessage(reason) }));
      }
    } finally {
      if (request === runHistoryRequest.current) setRunHistoryLoading(false);
    }
  }, [t]);

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
      showError(null);
    } catch (reason) {
      if (request === snapshotRequest.current) {
        showError(t("error.operationFailed", { detail: errorMessage(reason) }));
      }
    } finally {
      if (request === snapshotRequest.current) {
        setLoading(false);
        setRefreshing(false);
      }
    }
  }, [showError, t]);

  useEffect(() => {
    void loadSnapshot();
  }, [loadSnapshot]);

  useEffect(() => {
    if (activeView === "overview" && !runHistoryLoaded.current) void loadRunHistory();
  }, [activeView, loadRunHistory]);

  const discoverKnownRoots = useCallback(async (interactive = false) => {
    if (discoveryInFlight.current) return;
    if (!snapshot?.settings.recentDevelopmentRoot) {
      if (interactive) {
        setAutoDiscoveryMode(true);
        setProjectModal("import");
        setDiscoverySuggestions(null);
        setNotice(t("projects.autoDiscoveryChooseRoot"));
      }
      return;
    }
    discoveryInFlight.current = true;
    setDiscoveryState("scanning");
    setDiscoveryError(null);
    const registered = new Set((snapshot?.projects ?? []).map((project) => normalizedProjectPath(project.path)));
    try {
      const suggestions = (await api.scanSavedDevelopmentRoot())
        .filter((project) => project.profiles.length > 0 && !registered.has(normalizedProjectPath(project.path)));
      if (suggestions.length > 0) {
        setAutoDiscoveryMode(true);
        setDiscoverySuggestions(suggestions);
        setDiscoveryState("candidates");
        if (interactive) {
          setProjectModal("import");
          setActiveView("projects");
        } else {
          setNotice(t("projects.autoDiscoveryFound", { count: suggestions.length }));
        }
      } else {
        setDiscoverySuggestions([]);
        setDiscoveryState("empty");
        if (interactive) setNotice(t("projects.autoDiscoveryEmpty"));
      }
    } catch (reason) {
      const detail = errorMessage(reason);
      setDiscoveryState("error");
      setDiscoveryError(detail);
      if (interactive) showError(t("projects.autoDiscoveryErrorDetail", { detail }));
    } finally {
      discoveryInFlight.current = false;
    }
  }, [showError, snapshot?.projects, snapshot?.settings.recentDevelopmentRoot, t]);

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
          showError(t("error.lifecycleDetail", { detail: event.message }), event.relatedPort);
        } else {
          setNotice(event.message);
        }
      }
      if (event.status === "exited" || event.unexpected) void loadRunHistory();
    }).then((dispose) => {
      if (cancelled) dispose();
      else unlisten = dispose;
    }).catch((reason) => {
      if (!cancelled) {
        showError(t("error.operationFailed", { detail: errorMessage(reason) }));
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [loadRunHistory, showError, t]);

  // The exit event's reload above runs while the archive row is still `writing`,
  // because the writer closes the file after the lock that event is emitted under is
  // released. Reload again once the close has happened, so a finished archive stops
  // rendering as "finalizing" without waiting for some other refetch.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api.onArchiveClosed(() => {
      void loadRunHistory();
    }).then((dispose) => {
      if (cancelled) dispose();
      else unlisten = dispose;
    }).catch((reason) => {
      if (!cancelled) {
        showError(t("error.operationFailed", { detail: errorMessage(reason) }));
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [loadRunHistory, showError, t]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api.onLifecycleError((message) => {
      showError(t("error.lifecycleDetail", { detail: message }));
    }).then((dispose) => {
      if (cancelled) dispose();
      else unlisten = dispose;
    }).catch((reason) => {
      if (!cancelled) {
        showError(t("error.operationFailed", { detail: errorMessage(reason) }));
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [showError, t]);

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
      if (!cancelled) showError(errorMessage(reason));
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [showError]);

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
      void api.setLanguagePreference(localPreference).catch((reason) => showError(errorMessage(reason)));
    }
  }, [preference, setPreference, showError, snapshot]);

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
    const previousStatusEvent = profileStatusEvents.current.get(profileId);
    const entry = snapshot?.projects.flatMap((project) =>
      project.profiles.map((profile) => ({ project, profile })),
    ).find(({ profile }) => profile.id === profileId);
    try {
      const event = await action(profileId);
      setSnapshot((current) => current ? replaceProfileStatus(current, event) : current);
      setNotice(t(completedMessage));
      await loadSnapshot(true);
      await loadRunHistory();
    } catch (reason) {
      const failureEvent = profileStatusEvents.current.get(profileId);
      showError(t("error.profileActionFailed", {
        profile: entry ? `${entry.project.name} / ${entry.profile.name}` : profileId,
        detail: errorMessage(reason),
      }), failureEvent !== previousStatusEvent && failureEvent?.status === "conflict"
        ? failureEvent.relatedPort
        : null);
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
      await loadRunHistory();
      if (result.error) {
        showError(t("error.restorePartial", {
          count: result.startedProfileIds.length,
          profile: result.failedProfileId ?? "-",
          detail: result.error,
        }), result.relatedPort);
      } else {
        setNotice(t("notice.restored", { count: result.startedProfileIds.length }));
      }
    } catch (reason) {
      showError(errorMessage(reason));
    } finally {
      restoreInFlight.current = false;
      setRestoreBusy(false);
    }
  }, [loadRunHistory, loadSnapshot, showError, t]);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    void api.onTrayRestoreRequested(() => void restoreLastRunSet()).then((dispose) => {
      if (cancelled) dispose();
      else unlisteners.push(dispose);
    }).catch((reason) => {
      if (!cancelled) showError(errorMessage(reason));
    });
    void api.onTrayQuitRequested(() => {
      if (shutdownInFlight.current) return;
      setQuitOpen(true);
    }).then((dispose) => {
      if (cancelled) dispose();
      else unlisteners.push(dispose);
    }).catch((reason) => {
      if (!cancelled) showError(errorMessage(reason));
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
      if (!cancelled) showError(errorMessage(reason));
    });
    return () => {
      cancelled = true;
      unlisteners.forEach((dispose) => dispose());
    };
  }, [restoreLastRunSet, showError]);

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
        setDiscoverySuggestions((current) => current?.filter(
          (candidate) => normalizedProjectPath(candidate.path) !== normalizedProjectPath(input.path),
        ) ?? current);
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
    setDiscoverySuggestions((current) => {
      const next = current?.filter((candidate) =>
        !inputs.some((input) => normalizedProjectPath(input.path) === normalizedProjectPath(candidate.path)),
      ) ?? null;
      setDiscoveryState(next && next.length > 0 ? "candidates" : "empty");
      return next;
    });
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
      showError(t("error.operationFailed", { detail: errorMessage(reason) }));
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
      showError(errorMessage(reason));
    }
  };

  const openPort = async (port: PortSnapshot) => {
    try {
      await api.openPort(port.port, port.protocol);
    } catch (reason) {
      showError(errorMessage(reason));
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
      showError(errorMessage(reason));
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
      showError(errorMessage(reason));
    }
  };

  const openDirectory = async (projectId: string) => {
    try {
      await api.openProjectDirectory(projectId);
    } catch (reason) {
      showError(errorMessage(reason));
    }
  };

  const focusRelatedPort = async () => {
    if (!errorRelatedPort) return;
    try {
      const next = await api.getDashboardSnapshot();
      const active = next.ports.some((port) =>
        port.active && port.port === errorRelatedPort.port && port.protocol === errorRelatedPort.protocol,
      );
      setSnapshot(next);
      showError(null);
      if (!active) {
        setNotice(t("ports.conflictChanged", { port: errorRelatedPort.port }));
        return;
      }
      setActiveView("ports");
      setPortFocus({ ...errorRelatedPort, nonce: Date.now() });
    } catch (reason) {
      showError(t("error.operationFailed", { detail: errorMessage(reason) }));
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
      showError(errorMessage(reason));
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
      showError(
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
      showError(t("error.closeBehaviorSave", { detail: errorMessage(reason) }));
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
      showError(t("error.elevationFailed", { detail: errorMessage(reason) }));
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
      showError(`${t("error.languageSave")}: ${errorMessage(reason)}`);
    } finally {
      setLanguageBusy(false);
    }
  };

  /**
   * Persists the archive preference and reports the state the backend ended up in.
   *
   * Turning it on re-runs initialization, so this doubles as the retry after a failed
   * session; the drawer renders what comes back rather than what was asked for. It
   * deliberately does not catch: the caller shows the failure next to the toggle.
   */
  const toggleRunLogArchiving = useCallback(async (enabled: boolean) => {
    const next = await api.setRunLogArchiving(enabled);
    setSnapshot((current) => current
      ? {
        ...current,
        runLogArchive: next,
        settings: { ...current.settings, archiveRunLogs: next.enabled },
      }
      : current);
    // Every archive this changed is already final on disk once the command resolves:
    // turning it off closes the open ones, and turning it on re-runs the sweep that
    // finishes rows an earlier run left `writing`. Nothing announces them —
    // `run-archive-closed` is emitted from the process exit path only — so a badge
    // already on screen would keep claiming `Archiving` for a closed file until
    // something else refetched. Loading here rather than on the next Overview visit is
    // why this is gated on history having been loaded at all.
    if (runHistoryLoaded.current) await loadRunHistory();
    return next;
  }, [loadRunHistory]);

  const confirmArchiveDelete = async () => {
    if (!archiveToDelete || archiveDeleteInFlight.current) return;
    const session = archiveToDelete;
    archiveDeleteInFlight.current = true;
    setArchiveDeleteBusy(true);
    try {
      await api.deleteRunLogArchive(session.id);
      setArchiveToDelete(null);
      // The file is gone, so a viewer still open on it would be reading nothing.
      setArchiveViewerId((current) => current === session.id ? null : current);
      setNotice(t("archive.deleted"));
      await loadRunHistory();
    } catch (reason) {
      showError(t("archive.deleteFailed", { detail: errorMessage(reason) }));
    } finally {
      archiveDeleteInFlight.current = false;
      setArchiveDeleteBusy(false);
    }
  };

  const runHistoryLabels: RunHistoryLabels = useMemo(() => ({
    recentTitle: t("history.title"),
    recentDescription: t("history.subtitle"),
    viewAll: t("history.viewAll"),
    drawerTitle: t("history.title"),
    drawerDescription: t("history.subtitle"),
    close: t("history.close"),
    searchPlaceholder: t("history.search"),
    clearSearch: t("history.clearSearch"),
    filterLabel: t("history.filterLabel"),
    filters: {
      all: t("history.filter.all"),
      active: t("history.filter.active"),
      exited: t("history.filter.exited"),
      interrupted: t("history.filter.interrupted"),
    },
    project: t("history.project"),
    profile: t("history.profile"),
    status: t("history.status"),
    pid: t("history.pid"),
    startedAt: t("history.started"),
    endedAt: t("history.ended"),
    duration: t("history.duration"),
    exitCode: t("history.exitCode"),
    archive: t("history.archive"),
    actions: t("history.actions"),
    statusLabels: {
      starting: t("history.status.starting"),
      running: t("history.status.running"),
      exited: t("history.status.exited"),
      interrupted: t("history.status.interrupted"),
      unknown: t("history.status.unknown"),
    },
    projectDeleted: t("history.projectDeleted"),
    locate: (project, profile) => t("history.locateProject", { project, profile }),
    archiveBadge: (session) => describeArchive(session, t, locale),
    archiveView: (profile) => t("archive.view", { profile }),
    archiveDelete: (profile) => t("archive.delete", { profile }),
    loading: t("history.loading"),
    empty: t("history.empty"),
    noMatches: t("history.noMatches"),
    unavailable: t("history.unavailable"),
    retry: t("history.retry"),
    sessionCount: (count) => t("history.sessionCount", { count }),
    resultCount: (visible, total) => t("history.resultCount", { visible, total }),
  }), [locale, t]);

  const locateRunHistory = useCallback((projectId: string, profileId: string) => {
    setActiveView("projects");
    setFocusedProjectId(projectId);
    setRunHistoryDrawerOpen(false);
    setNotice(t("history.locateProject", {
      project: snapshot?.projects.find((project) => project.id === projectId)?.name ?? projectId,
      profile: snapshot?.projects.flatMap((project) => project.profiles).find((profile) => profile.id === profileId)?.name ?? profileId,
    }));
  }, [snapshot?.projects, t]);

  const clearPortFocus = useCallback(() => setPortFocus(null), []);
  const clearProjectFocus = useCallback(() => setFocusedProjectId(null), []);

  const refreshAll = useCallback(async () => {
    await Promise.all([loadSnapshot(), loadRunHistory()]);
  }, [loadRunHistory, loadSnapshot]);

  const selectedLog = useMemo(() => {
    if (!snapshot || !logSelection) return null;
    const project = snapshot.projects.find((item) => item.id === logSelection.projectId);
    const profile = project?.profiles.find((item) => item.id === logSelection.profileId);
    return project && profile ? { project, profile } : null;
  }, [logSelection, snapshot]);

  const archiveViewer = useMemo(() => {
    if (!snapshot || !archiveViewerId) return null;
    const session = runHistory.find((entry) => entry.id === archiveViewerId);
    if (!session) return null;
    const { project } = resolveRunSession(session, snapshot.projects);
    return { session, projectName: project?.name ?? t("history.projectDeleted") };
  }, [archiveViewerId, runHistory, snapshot, t]);

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
            <IconButton label={t("scan.refresh")} onClick={() => void refreshAll()} disabled={refreshing || runHistoryLoading} className={refreshing ? "is-spinning" : ""}>
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
            <OverviewView
              snapshot={snapshot}
              restoreBusy={restoreBusy}
              onRestore={() => void restoreLastRunSet()}
              runHistory={runHistory}
              runHistoryLoading={runHistoryLoading}
              runHistoryError={runHistoryError}
              runHistoryLabels={runHistoryLabels}
              onRetryRunHistory={() => void loadRunHistory()}
              onOpenRunHistory={() => setRunHistoryDrawerOpen(true)}
              onLocateRunHistory={locateRunHistory}
              onViewArchive={(session) => setArchiveViewerId(session.id)}
              onDeleteArchive={setArchiveToDelete}
              {...profileActionProps}
            />
          )}
          {snapshot && activeView === "ports" && (
            <PortsView
              snapshot={snapshot}
              busyProfileIds={effectiveBusyProfileIds}
              onOpenPort={(port) => void openPort(port)}
              onTerminate={setExternalProcess}
              onConfirmAssociation={(port) => void confirmPortAssociation(port)}
              onStartProfile={(profileId) => void runProfileAction(profileId, api.startProfile, "notice.startCompleted")}
              focusRequest={portFocus}
              onFocusHandled={clearPortFocus}
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
              discoveryState={discoveryState}
              discoveryError={discoveryError}
              discoveredCount={discoverySuggestions?.length ?? 0}
              hasSavedDiscoveryRoot={Boolean(snapshot.settings.recentDevelopmentRoot)}
              monitorOnly={snapshot.privilege.monitorOnly}
              onEdit={setProjectModal}
              onDelete={setProjectToDelete}
              focusedProjectId={focusedProjectId}
              onFocusedProjectHandled={clearProjectFocus}
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
          <LogDrawer
            api={api}
            profile={selectedLog.profile}
            project={selectedLog.project}
            capacity={snapshot.settings.logCapacity}
            archive={snapshot.runLogArchive}
            onToggleArchive={toggleRunLogArchiving}
            onClose={() => setLogSelection(null)}
          />
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
        {runHistoryDrawerOpen && snapshot && !quitOpen && !elevationOpen && !helpOpen && (
          <RunHistoryDrawer
            sessions={runHistory}
            projects={snapshot.projects}
            loading={runHistoryLoading}
            error={runHistoryError}
            labels={runHistoryLabels}
            onRetry={() => void loadRunHistory()}
            onLocate={locateRunHistory}
            onViewArchive={(session) => setArchiveViewerId(session.id)}
            onDeleteArchive={setArchiveToDelete}
            onClose={() => setRunHistoryDrawerOpen(false)}
          />
        )}
        {archiveViewer && !quitOpen && !elevationOpen && !helpOpen && (
          <RunLogArchiveDrawer
            api={api}
            session={archiveViewer.session}
            projectName={archiveViewer.projectName}
            onDelete={setArchiveToDelete}
            onClose={() => setArchiveViewerId(null)}
          />
        )}
        {archiveToDelete && !quitOpen && !elevationOpen && !helpOpen && (
          <ConfirmModal
            title={t("archive.deleteTitle")}
            detail={t("archive.deleteDetail", {
              profile: archiveToDelete.profileName,
              lines: new Intl.NumberFormat(locale).format(archiveToDelete.archive?.lineCount ?? 0),
              size: formatArchiveSize(archiveToDelete.archive?.byteSize ?? 0, locale),
            })}
            confirmLabel={t("action.delete")}
            busy={archiveDeleteBusy}
            onCancel={() => setArchiveToDelete(null)}
            onConfirm={() => void confirmArchiveDelete()}
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
              {errorRelatedPort && (
                <button type="button" className="toast-action" onClick={() => void focusRelatedPort()}>
                  {t("ports.viewOccupant")}
                </button>
              )}
              <button type="button" onClick={() => showError(null)} aria-label={t("app.dismissError")}>×</button>
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
