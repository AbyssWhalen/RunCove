import { ArrowDownToLine, Check, Copy, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { useI18n } from "../i18n";
import type {
  LaunchProfile,
  Project,
  RunCoveApi,
  RunLogArchiveState,
  RunLogEvent,
} from "../types";
import { IconButton } from "./IconButton";
import { StatusBadge } from "./StatusBadge";
import { useDialogFocus } from "./useDialogFocus";

type StreamFilter = "all" | RunLogEvent["stream"];

interface LogDrawerProps {
  api: RunCoveApi;
  profile: LaunchProfile;
  project: Project;
  capacity: number;
  archive: RunLogArchiveState;
  /** Resolves with the state the backend ended up in, which may differ from the request. */
  onToggleArchive: (enabled: boolean) => Promise<RunLogArchiveState>;
  onClose: () => void;
}

function describeError(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason);
}

function logKey(entry: RunLogEvent) {
  return `${entry.profileId}\u0000${entry.timestamp}\u0000${entry.stream}\u0000${entry.line}`;
}

function mergeLogs(history: RunLogEvent[], live: RunLogEvent[], capacity: number) {
  const overlappingCounts = new Map<string, number>();
  history.forEach((entry) => {
    const key = logKey(entry);
    overlappingCounts.set(key, (overlappingCounts.get(key) ?? 0) + 1);
  });
  return [
    ...history,
    ...live.filter((entry) => {
      const key = logKey(entry);
      const overlap = overlappingCounts.get(key) ?? 0;
      if (overlap === 0) return true;
      overlappingCounts.set(key, overlap - 1);
      return false;
    }),
  ].slice(-capacity);
}

export function LogDrawer({
  api,
  profile,
  project,
  capacity,
  archive,
  onToggleArchive,
  onClose,
}: LogDrawerProps) {
  const { t, formatTime } = useI18n();
  const [logs, setLogs] = useState<RunLogEvent[]>([]);
  const [filter, setFilter] = useState<StreamFilter>("all");
  const [autoScroll, setAutoScroll] = useState(true);
  const [copied, setCopied] = useState(false);
  const [loading, setLoading] = useState(true);
  const [clearing, setClearing] = useState(false);
  const [copying, setCopying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [archiveBusy, setArchiveBusy] = useState(false);
  const [archiveError, setArchiveError] = useState<string | null>(null);
  const [archiveNotice, setArchiveNotice] = useState<string | null>(null);
  const [retryVersion, setRetryVersion] = useState(0);
  const endRef = useRef<HTMLDivElement>(null);
  const { dialogRef, onDialogKeyDown } = useDialogFocus(onClose, clearing);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    setLoading(true);
    setError(null);

    const initialize = async () => {
      try {
        const dispose = await api.onRunLog((event) => {
          if (!cancelled && event.profileId === profile.id) {
            setLogs((current) => [...current, event].slice(-capacity));
          }
        });
        if (cancelled) {
          dispose();
          return;
        }
        unlisten = dispose;
      } catch (reason) {
        if (cancelled) return;
        setError(t("logs.loadFailed", { detail: describeError(reason) }));
      }

      try {
        const entries = await api.getLogs(profile.id);
        if (!cancelled) setLogs((current) => mergeLogs(entries, current, capacity));
      } catch (reason) {
        if (!cancelled) setError(t("logs.loadFailed", { detail: describeError(reason) }));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    void initialize();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [api, capacity, profile.id, retryVersion, t]);

  const visibleLogs = useMemo(
    () => filter === "all" ? logs : logs.filter((entry) => entry.stream === filter),
    [filter, logs],
  );

  useEffect(() => {
    if (autoScroll) endRef.current?.scrollIntoView({ block: "end" });
  }, [autoScroll, visibleLogs]);

  const copyLogs = async () => {
    const text = visibleLogs.map((entry) => `[${entry.stream}] ${entry.line}`).join("\n");
    setError(null);
    if (!navigator.clipboard) {
      setError(t("logs.copyUnavailable"));
      return;
    }
    setCopying(true);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1_200);
    } catch (reason) {
      setError(t("logs.copyFailed", { detail: describeError(reason) }));
    } finally {
      setCopying(false);
    }
  };

  const clearLogs = async () => {
    setClearing(true);
    setError(null);
    try {
      await api.clearLogs(profile.id);
      setLogs([]);
      const entries = await api.getLogs(profile.id);
      setLogs((current) => mergeLogs(entries, current, capacity));
    } catch (reason) {
      setError(t("logs.clearFailed", { detail: describeError(reason) }));
    } finally {
      setClearing(false);
    }
  };

  const toggleArchive = async (enabled: boolean) => {
    setArchiveBusy(true);
    setArchiveError(null);
    setArchiveNotice(null);
    try {
      const next = await onToggleArchive(enabled);
      // Turning it on while the archive cannot start says nothing reassuring: the
      // unavailable warning below is the honest report in that case.
      if (next.enabled && next.available) setArchiveNotice(t("archive.enabledNotice"));
      else if (!enabled) setArchiveNotice(t("archive.disabledNotice"));
    } catch (reason) {
      setArchiveError(t("archive.toggleFailed", { detail: describeError(reason) }));
    } finally {
      setArchiveBusy(false);
    }
  };

  return (
    <div className="drawer-backdrop" role="presentation" onMouseDown={clearing ? undefined : onClose}>
      <aside
        ref={dialogRef}
        className="log-drawer"
        role="dialog"
        aria-modal="true"
        aria-labelledby="log-drawer-title"
        aria-busy={clearing}
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="drawer-header">
          <div>
            <div className="drawer-title-row">
              <h2 id="log-drawer-title">{profile.name}</h2>
              <StatusBadge status={profile.status} />
            </div>
            <span>{project.name} · {logs.length}/{capacity}</span>
          </div>
          <IconButton label={t("logs.close")} onClick={onClose} disabled={clearing}><X size={16} /></IconButton>
        </header>
        <div className="drawer-toolbar">
          <div className="segmented-control segmented-control--dark" aria-label={t("logs.filterLabel")}>
            {(["all", "stdout", "stderr", "system"] as const).map((stream) => (
              <button
                key={stream}
                type="button"
                aria-pressed={filter === stream}
                className={filter === stream ? "is-selected" : ""}
                onClick={() => setFilter(stream)}
              >
                {stream === "all" ? t("logs.all") : stream === "system" ? t("logs.system") : stream}
              </button>
            ))}
          </div>
          <div className="row-actions">
            <IconButton label={autoScroll ? t("logs.disableAutoScroll") : t("logs.enableAutoScroll")} className={autoScroll ? "is-active" : ""} onClick={() => setAutoScroll((value) => !value)}>
              <ArrowDownToLine size={15} />
            </IconButton>
            <IconButton label={t("logs.copy")} onClick={() => void copyLogs()} disabled={visibleLogs.length === 0 || copying}>
              {copied ? <Check size={15} /> : <Copy size={15} />}
            </IconButton>
            <IconButton label={t("logs.clear")} tone="danger" onClick={() => void clearLogs()} disabled={logs.length === 0 || clearing}>
              <Trash2 size={15} />
            </IconButton>
          </div>
        </div>
        <div className="archive-toggle">
          <label className="archive-toggle-control">
            <input
              type="checkbox"
              checked={archive.enabled}
              disabled={archiveBusy}
              aria-describedby="archive-toggle-hint"
              onChange={(event) => void toggleArchive(event.target.checked)}
            />
            <span>{archiveBusy ? t("action.working") : t("archive.toggleLabel")}</span>
          </label>
          <p className="archive-toggle-hint" id="archive-toggle-hint">{t("archive.toggleHint")}</p>
          {!archive.available && (
            <p className="archive-toggle-status archive-toggle-status--warning" role="alert">
              {archive.unavailableReason
                ? t("archive.unavailable", { detail: archive.unavailableReason })
                : t("archive.unavailableGeneric")}
            </p>
          )}
          {archiveError && (
            <p className="archive-toggle-status archive-toggle-status--warning" role="alert">
              {archiveError}
            </p>
          )}
          {archiveNotice && (
            <p className="archive-toggle-status" role="status">{archiveNotice}</p>
          )}
        </div>
        {error && (
          <div className="drawer-error" role="alert">
            <span>{error}</span>
            <button type="button" onClick={() => setRetryVersion((value) => value + 1)}>{t("logs.retry")}</button>
          </div>
        )}
        <div className="log-output" aria-live="polite">
          {loading && <div className="log-empty">{t("logs.loading")}</div>}
          {!loading && visibleLogs.length === 0 && <div className="log-empty">{t("logs.empty")}</div>}
          {visibleLogs.map((entry, index) => (
            <div className={`log-line log-line--${entry.stream}`} key={`${entry.profileId}-${entry.timestamp}-${index}`}>
              <time>{formatTime(entry.timestamp)}</time>
              <span className="log-stream">
                {entry.stream === "system" ? t("logs.system") : entry.stream}
              </span>
              <pre>{entry.line}</pre>
            </div>
          ))}
          <div ref={endRef} />
        </div>
      </aside>
    </div>
  );
}
