import { Search, X } from "lucide-react";
import { useMemo, useState } from "react";

import type { Project, RunSession } from "../types";
import { IconButton } from "./IconButton";
import { prepareRunHistory, type RunHistoryFilter } from "./run-history";
import { type RunHistoryLabels, RunHistoryTable } from "./RunHistorySection";
import { useDialogFocus } from "./useDialogFocus";

interface RunHistoryDrawerProps {
  sessions: RunSession[];
  projects: Project[];
  loading: boolean;
  error?: string | null;
  labels: RunHistoryLabels;
  onRetry: () => void;
  onLocate: (projectId: string, profileId: string) => void;
  onClose: () => void;
}

export function RunHistoryDrawer({
  sessions,
  projects,
  loading,
  error,
  labels,
  onRetry,
  onLocate,
  onClose,
}: RunHistoryDrawerProps) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<RunHistoryFilter>("all");
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLElement>(onClose);
  const availableSessions = useMemo(
    () => [...sessions].sort((left, right) => right.startedAt - left.startedAt).slice(0, 200),
    [sessions],
  );
  const entries = useMemo(
    () => prepareRunHistory(availableSessions, projects, query, filter),
    [availableSessions, filter, projects, query],
  );

  return (
    <div className="drawer-backdrop" role="presentation" onMouseDown={onClose}>
      <aside
        ref={dialogRef}
        className="run-history-drawer"
        role="dialog"
        aria-modal="true"
        aria-labelledby="run-history-drawer-title"
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="drawer-header">
          <div>
            <h2 id="run-history-drawer-title">{labels.drawerTitle}</h2>
            <span>{labels.drawerDescription}</span>
          </div>
          <IconButton label={labels.close} onClick={onClose}><X size={16} /></IconButton>
        </header>
        <div className="drawer-toolbar run-history-toolbar">
          <label className="search-field">
            <Search size={14} aria-hidden="true" />
            <input
              type="search"
              value={query}
              placeholder={labels.searchPlaceholder}
              aria-label={labels.searchPlaceholder}
              onChange={(event) => setQuery(event.currentTarget.value)}
            />
            {query && (
              <button type="button" aria-label={labels.clearSearch} title={labels.clearSearch} onClick={() => setQuery("")}>
                <X size={13} />
              </button>
            )}
          </label>
          <div className="segmented-control" aria-label={labels.filterLabel}>
            {(["all", "active", "exited", "interrupted"] as const).map((value) => (
              <button
                key={value}
                type="button"
                aria-pressed={filter === value}
                className={filter === value ? "is-selected" : ""}
                onClick={() => setFilter(value)}
              >
                {labels.filters[value]}
              </button>
            ))}
          </div>
          <span className="run-history-result-count" aria-live="polite">
            {labels.resultCount(entries.length, availableSessions.length)}
          </span>
        </div>
        {error && (
          <div className="drawer-error" role="alert">
            <span>{error}</span>
            <button type="button" onClick={onRetry}>{labels.retry}</button>
          </div>
        )}
        <div className="run-history-content">
          {!error && loading && <div className="empty-state" aria-live="polite">{labels.loading}</div>}
          {!error && !loading && availableSessions.length === 0 && <div className="empty-state">{labels.empty}</div>}
          {!error && !loading && availableSessions.length > 0 && entries.length === 0 && <div className="empty-state">{labels.noMatches}</div>}
          {!error && !loading && entries.length > 0 && (
            <RunHistoryTable entries={entries} labels={labels} onLocate={onLocate} />
          )}
        </div>
      </aside>
    </div>
  );
}
