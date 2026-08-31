import {
  Activity,
  ExternalLink,
  FolderOpen,
  Play,
  RotateCw,
  ScrollText,
  Square,
} from "lucide-react";

import { activeDisplayPortCount } from "../port-display";
import { canOpenProfilePort } from "../profile-actions";
import { useI18n } from "../i18n";
import type { DashboardSnapshot, LaunchGroup, LaunchProfile, Project } from "../types";
import type { RunSession } from "../types";
import { IconButton } from "./IconButton";
import { LaunchGroupSection, type GroupAction } from "./LaunchGroupSection";
import { RunHistorySection, type RunHistoryLabels } from "./RunHistorySection";
import { StatusBadge } from "./StatusBadge";

interface OverviewViewProps {
  snapshot: DashboardSnapshot;
  busyProfileIds: Set<string>;
  restoreBusy: boolean;
  onRestore: () => void;
  onStart: (profileId: string) => void;
  onStop: (profileId: string) => void;
  onRestart: (profileId: string) => void;
  onOpenPort: (profile: LaunchProfile) => void;
  onOpenDirectory: (projectId: string) => void;
  onOpenLogs: (profile: LaunchProfile, project: Project) => void;
  busyGroups?: ReadonlyMap<string, GroupAction>;
  onNewGroup?: () => void;
  onEditGroup?: (group: LaunchGroup) => void;
  onDeleteGroup?: (group: LaunchGroup) => void;
  onStartGroup?: (group: LaunchGroup) => void;
  onStopGroup?: (group: LaunchGroup) => void;
  runHistory?: RunSession[];
  runHistoryLoading?: boolean;
  runHistoryError?: string | null;
  runHistoryLabels?: RunHistoryLabels;
  onRetryRunHistory?: () => void;
  onOpenRunHistory?: () => void;
  onLocateRunHistory?: (projectId: string, profileId: string) => void;
  onViewArchive?: (session: RunSession) => void;
  onDeleteArchive?: (session: RunSession) => void;
}

export function OverviewView({
  snapshot,
  busyProfileIds,
  restoreBusy,
  onRestore,
  onStart,
  onStop,
  onRestart,
  onOpenPort,
  onOpenDirectory,
  onOpenLogs,
  busyGroups = new Map(),
  onNewGroup = () => undefined,
  onEditGroup = () => undefined,
  onDeleteGroup = () => undefined,
  onStartGroup = () => undefined,
  onStopGroup = () => undefined,
  runHistory = [],
  runHistoryLoading = false,
  runHistoryError,
  runHistoryLabels,
  onRetryRunHistory = () => undefined,
  onOpenRunHistory = () => undefined,
  onLocateRunHistory = () => undefined,
  onViewArchive = () => undefined,
  onDeleteArchive = () => undefined,
}: OverviewViewProps) {
  const { t, formatDateTime, formatTime } = useI18n();
  const profiles = snapshot.projects.flatMap((project) =>
    project.profiles.map((profile) => ({ project, profile })),
  );
  const running = profiles.filter(({ profile }) => profile.status === "running").length;
  const conflict = profiles.filter(({ profile }) => profile.status === "conflict").length;
  const activePorts = activeDisplayPortCount(snapshot.ports);
  const profileLookup = new Map(profiles.map((entry) => [entry.profile.id, entry]));
  const restoreSequence = snapshot.restoreSet.profileIds.map((profileId) => {
    const entry = profileLookup.get(profileId);
    return entry ? `${entry.project.name} / ${entry.profile.name}` : profileId;
  });
  const monitorOnly = snapshot.privilege.monitorOnly;
  const processActionLabel = (label: string) =>
    monitorOnly ? `${label}: ${t("privilege.monitorOnlyAction")}` : label;

  return (
    <div className="view-stack">
      <section className="metric-strip" aria-label={t("overview.runtimeSummary")}>
        <div className="metric-cell">
          <span className="metric-label">{t("overview.running")}</span>
          <strong>{running}</strong>
        </div>
        <div className="metric-cell">
          <span className="metric-label">{t("overview.activePorts")}</span>
          <strong>{activePorts}</strong>
        </div>
        <div className="metric-cell">
          <span className="metric-label">{t("overview.conflicts")}</span>
          <strong className={conflict > 0 ? "text-danger" : ""}>{conflict}</strong>
        </div>
        <div className="metric-cell metric-cell--wide">
          <span className="metric-label">{t("overview.lastScan")}</span>
          <strong className="metric-time">{formatTime(snapshot.generatedAt)}</strong>
        </div>
      </section>

      <section className="restore-band" aria-labelledby="restore-heading">
        <div className="restore-icon" aria-hidden="true"><Activity size={18} /></div>
        <div className="restore-copy">
          <h2 id="restore-heading">{t("overview.lastRunSet")}</h2>
          <span>
            {t("count.profiles", { count: snapshot.restoreSet.profileIds.length })} · {snapshot.restoreSet.savedAt
              ? formatDateTime(snapshot.restoreSet.savedAt, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })
              : t("overview.notSaved")}
          </span>
          {restoreSequence.length > 0 && (
            <ol className="restore-sequence" aria-label={t("overview.restoreOrder")}>
              {restoreSequence.map((label, index) => (
                <li key={`${snapshot.restoreSet.profileIds[index]}:${index}`} title={label}>{label}</li>
              ))}
            </ol>
          )}
        </div>
        <button
          className="button button--primary"
          onClick={onRestore}
          disabled={monitorOnly || restoreBusy || snapshot.restoreSet.profileIds.length === 0}
          title={monitorOnly ? t("privilege.monitorOnlyAction") : undefined}
        >
          <Play size={15} fill="currentColor" />
          {restoreBusy ? t("overview.restoring") : t("overview.restoreSet")}
        </button>
      </section>

      <LaunchGroupSection
        groups={snapshot.launchGroups}
        projects={snapshot.projects}
        monitorOnly={monitorOnly}
        busyGroups={busyGroups}
        onNew={onNewGroup}
        onEdit={onEditGroup}
        onDelete={onDeleteGroup}
        onStart={onStartGroup}
        onStop={onStopGroup}
      />

      <section className="data-section" aria-labelledby="profiles-heading">
        <div className="section-heading">
          <div>
            <h2 id="profiles-heading">{t("overview.launchProfiles")}</h2>
            <span>{t("count.configured", { count: profiles.length })}</span>
          </div>
        </div>
        <div className="table-shell">
          <table className="profiles-table profiles-table--overview">
            <thead>
              <tr>
                <th>{t("table.project")}</th>
                <th>{t("table.profile")}</th>
                <th>{t("table.status")}</th>
                <th>{t("table.expectedPorts")}</th>
                <th>{t("table.pid")}</th>
                <th className="actions-column">{t("table.actions")}</th>
              </tr>
            </thead>
            <tbody>
              {profiles.map(({ project, profile }) => {
                const busy = busyProfileIds.has(profile.id);
                const runningProfile = profile.status === "running" || profile.status === "starting";
                return (
                  <tr key={profile.id}>
                    <td title={project.name}>
                      <button className="link-button table-primary" onClick={() => onOpenDirectory(project.id)} disabled={monitorOnly} title={monitorOnly ? t("privilege.monitorOnlyAction") : undefined}>
                        {project.name}
                      </button>
                    </td>
                    <td title={profile.name}>{profile.name}</td>
                    <td><StatusBadge status={profile.status} /></td>
                    <td className="mono-cell">
                      {profile.expectedPorts.length > 0
                        ? profile.expectedPorts.map((port) => `${port.port}/${port.protocol}`).join(", ")
                        : <span className="muted">{t("table.none")}</span>}
                    </td>
                    <td className="mono-cell">{profile.pid ?? <span className="muted">-</span>}</td>
                    <td>
                      <div className="row-actions">
                        {runningProfile ? (
                          <IconButton label={processActionLabel(t("profile.stop", { project: project.name, profile: profile.name }))} onClick={() => onStop(profile.id)} disabled={monitorOnly || busy} tone="danger">
                            <Square size={14} fill="currentColor" />
                          </IconButton>
                        ) : (
                          <IconButton label={processActionLabel(t("profile.start", { project: project.name, profile: profile.name }))} onClick={() => onStart(profile.id)} disabled={monitorOnly || busy} tone="success">
                            <Play size={15} fill="currentColor" />
                          </IconButton>
                        )}
                        <IconButton label={processActionLabel(t("profile.restart", { project: project.name, profile: profile.name }))} onClick={() => onRestart(profile.id)} disabled={monitorOnly || busy}>
                          <RotateCw size={15} />
                        </IconButton>
                        <IconButton label={processActionLabel(t("profile.openBrowser", { profile: profile.name }))} onClick={() => onOpenPort(profile)} disabled={monitorOnly || !canOpenProfilePort(profile, snapshot.ports)}>
                          <ExternalLink size={15} />
                        </IconButton>
                        <IconButton label={processActionLabel(t("profile.openFolder", { project: project.name }))} onClick={() => onOpenDirectory(project.id)} disabled={monitorOnly}>
                          <FolderOpen size={15} />
                        </IconButton>
                        <IconButton label={t("profile.viewLogs", { profile: profile.name })} onClick={() => onOpenLogs(profile, project)}>
                          <ScrollText size={15} />
                        </IconButton>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {profiles.length === 0 && <div className="empty-state">{t("overview.noProfiles")}</div>}
        </div>
      </section>
      {runHistoryLabels && <RunHistorySection
        sessions={runHistory}
        projects={snapshot.projects}
        loading={runHistoryLoading}
        error={runHistoryError}
        labels={runHistoryLabels}
        onRetry={onRetryRunHistory}
        onLocate={onLocateRunHistory}
        onViewArchive={onViewArchive}
        onDeleteArchive={onDeleteArchive}
        onOpenAll={onOpenRunHistory}
      />}
    </div>
  );
}
