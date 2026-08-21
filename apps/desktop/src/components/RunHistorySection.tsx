import { FileClock, History, LocateFixed, Trash2 } from "lucide-react";
import { useMemo } from "react";

import { useI18n } from "../i18n";
import type {
  Project,
  RunSession,
  RunSessionStatus,
} from "../types";
import { IconButton } from "./IconButton";
import { type ArchiveBadge, canDeleteArchive, canViewArchive } from "./archive";
import {
  formatRunDuration,
  prepareRunHistory,
  type ResolvedRunSession,
  type RunHistoryFilter,
} from "./run-history";

export interface RunHistoryLabels {
  recentTitle: string;
  recentDescription: string;
  viewAll: string;
  drawerTitle: string;
  drawerDescription: string;
  close: string;
  searchPlaceholder: string;
  clearSearch: string;
  filterLabel: string;
  filters: Record<RunHistoryFilter, string>;
  project: string;
  profile: string;
  status: string;
  pid: string;
  startedAt: string;
  endedAt: string;
  duration: string;
  exitCode: string;
  archive: string;
  actions: string;
  statusLabels: Record<RunSessionStatus, string>;
  projectDeleted: string;
  locate: (project: string, profile: string) => string;
  // A row's archive copy depends on that row's counters, so the caller supplies a
  // function rather than a string. It stays on the labels bag because the table is
  // rendered in tests that provide no translator.
  archiveBadge: (session: RunSession) => ArchiveBadge;
  archiveView: (profile: string) => string;
  archiveDelete: (profile: string) => string;
  loading: string;
  empty: string;
  noMatches: string;
  unavailable: string;
  retry: string;
  sessionCount: (count: number) => string;
  resultCount: (visible: number, total: number) => string;
}

interface RunHistorySectionProps {
  sessions: RunSession[];
  projects: Project[];
  loading: boolean;
  error?: string | null;
  labels: RunHistoryLabels;
  onRetry: () => void;
  onLocate: (projectId: string, profileId: string) => void;
  onViewArchive: (session: RunSession) => void;
  onDeleteArchive: (session: RunSession) => void;
  onOpenAll: () => void;
}

function SessionStatus({
  status,
  labels,
}: {
  status: RunSessionStatus;
  labels: RunHistoryLabels;
}) {
  const badgeStatus = status === "interrupted" ? "exited" : status;
  return (
    <span className={`status-badge status-badge--${badgeStatus}`}>
      <span className="status-dot" aria-hidden="true" />
      {labels.statusLabels[status]}
    </span>
  );
}

function ArchiveCell({ badge }: { badge: ArchiveBadge }) {
  if (badge.state === "none") return <span className="muted">{badge.text}</span>;
  return (
    <span className={`status-badge archive-badge archive-badge--${badge.state}`} title={badge.detail}>
      {badge.text}
    </span>
  );
}

export function RunHistoryTable({
  entries,
  labels,
  onLocate,
  onViewArchive,
  onDeleteArchive,
}: {
  entries: ResolvedRunSession[];
  labels: RunHistoryLabels;
  onLocate: (projectId: string, profileId: string) => void;
  onViewArchive: (session: RunSession) => void;
  onDeleteArchive: (session: RunSession) => void;
}) {
  const { formatDateTime, locale } = useI18n();
  const dateOptions: Intl.DateTimeFormatOptions = {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  };

  return (
    <div className="table-shell run-history-table-shell">
      <table className="run-history-table">
        <thead>
          <tr>
            <th>{labels.project}</th>
            <th>{labels.profile}</th>
            <th>{labels.status}</th>
            <th>{labels.pid}</th>
            <th>{labels.startedAt}</th>
            <th>{labels.endedAt}</th>
            <th>{labels.duration}</th>
            <th>{labels.exitCode}</th>
            <th>{labels.archive}</th>
            <th className="actions-column">{labels.actions}</th>
          </tr>
        </thead>
        <tbody>
          {entries.map(({ session, status, project, profile }) => {
            const badge = labels.archiveBadge(session);
            const profileName = profile?.name ?? session.profileName;
            return (
            <tr key={session.id}>
              <td title={project?.name ?? labels.projectDeleted}>
                {project?.name ?? <span className="muted">{labels.projectDeleted}</span>}
              </td>
              <td title={profileName}>{profileName}</td>
              <td><SessionStatus status={status} labels={labels} /></td>
              <td className="mono-cell">{session.pid ?? <span className="muted">{labels.unavailable}</span>}</td>
              <td><time dateTime={new Date(session.startedAt).toISOString()}>{formatDateTime(session.startedAt, dateOptions)}</time></td>
              <td>
                {session.endedAt == null
                  ? <span className="muted">{labels.unavailable}</span>
                  : <time dateTime={new Date(session.endedAt).toISOString()}>{formatDateTime(session.endedAt, dateOptions)}</time>}
              </td>
              <td>{formatRunDuration(session, locale)}</td>
              <td className="mono-cell">{session.exitCode ?? <span className="muted">{labels.unavailable}</span>}</td>
              <td><ArchiveCell badge={badge} /></td>
              <td>
                <div className="row-actions">
                  <IconButton
                    label={labels.archiveView(profileName)}
                    onClick={() => onViewArchive(session)}
                    disabled={!canViewArchive(badge.state)}
                  >
                    <FileClock size={15} />
                  </IconButton>
                  <IconButton
                    label={labels.archiveDelete(profileName)}
                    tone="danger"
                    onClick={() => onDeleteArchive(session)}
                    disabled={!canDeleteArchive(badge.state)}
                  >
                    <Trash2 size={15} />
                  </IconButton>
                  <IconButton
                    label={labels.locate(project?.name ?? labels.projectDeleted, profileName)}
                    onClick={() => project && profile && onLocate(project.id, profile.id)}
                    disabled={!project || !profile}
                  >
                    <LocateFixed size={15} />
                  </IconButton>
                </div>
              </td>
            </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export function RunHistorySection({
  sessions,
  projects,
  loading,
  error,
  labels,
  onRetry,
  onLocate,
  onViewArchive,
  onDeleteArchive,
  onOpenAll,
}: RunHistorySectionProps) {
  const entries = useMemo(
    () => prepareRunHistory(sessions, projects).slice(0, 5),
    [projects, sessions],
  );

  return (
    <section className="data-section run-history-section" aria-labelledby="recent-run-history-heading">
      <div className="section-heading">
        <div>
          <h2 id="recent-run-history-heading"><History size={16} aria-hidden="true" /> {labels.recentTitle}</h2>
          <span>{loading ? labels.loading : labels.sessionCount(sessions.length)}</span>
        </div>
        <button className="button button--secondary" type="button" onClick={onOpenAll} disabled={loading || sessions.length === 0}>
          {labels.viewAll}
        </button>
      </div>
      <p className="section-description">{labels.recentDescription}</p>
      {error && (
        <div className="inline-error" role="alert">
          <span>{error}</span>
          <button type="button" onClick={onRetry}>{labels.retry}</button>
        </div>
      )}
      {!error && loading && <div className="empty-state" aria-live="polite">{labels.loading}</div>}
      {!error && !loading && entries.length === 0 && <div className="empty-state">{labels.empty}</div>}
      {!error && !loading && entries.length > 0 && (
        <RunHistoryTable
          entries={entries}
          labels={labels}
          onLocate={onLocate}
          onViewArchive={onViewArchive}
          onDeleteArchive={onDeleteArchive}
        />
      )}
    </section>
  );
}
