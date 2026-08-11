import {
  ExternalLink,
  FolderOpen,
  Pencil,
  Play,
  Plus,
  RotateCw,
  ScanSearch,
  ScrollText,
  Search,
  Square,
  Trash2,
} from "lucide-react";
import { useMemo, useState } from "react";

import { canOpenProfilePort } from "../profile-actions";
import { useI18n } from "../i18n";
import type { LaunchProfile, PortSnapshot, Project } from "../types";
import { IconButton } from "./IconButton";
import { StatusBadge } from "./StatusBadge";

interface ProjectsViewProps {
  projects: Project[];
  ports: PortSnapshot[];
  busyProfileIds: Set<string>;
  monitorOnly: boolean;
  onImport: () => void;
  onAutoDiscover: () => void;
  discoveredCount?: number;
  hasSavedDiscoveryRoot: boolean;
  onEdit: (project: Project) => void;
  onDelete: (project: Project) => void;
  onStart: (profileId: string) => void;
  onStop: (profileId: string) => void;
  onRestart: (profileId: string) => void;
  onOpenPort: (profile: LaunchProfile) => void;
  onOpenDirectory: (projectId: string) => void;
  onOpenLogs: (profile: LaunchProfile, project: Project) => void;
}

export function ProjectsView({
  projects,
  ports,
  busyProfileIds,
  monitorOnly,
  onImport,
  onAutoDiscover,
  discoveredCount = 0,
  hasSavedDiscoveryRoot,
  onEdit,
  onDelete,
  onStart,
  onStop,
  onRestart,
  onOpenPort,
  onOpenDirectory,
  onOpenLogs,
}: ProjectsViewProps) {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const processActionLabel = (label: string) =>
    monitorOnly ? `${label}: ${t("privilege.monitorOnlyAction")}` : label;
  const filteredProjects = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return projects;
    return projects.filter((project) =>
      [project.name, project.path, ...project.profiles.flatMap((profile) => [profile.name, profile.program, ...profile.args])]
        .join(" ")
        .toLowerCase()
        .includes(needle),
    );
  }, [projects, query]);

  return (
    <div className="view-stack">
      <div className="section-heading section-heading--toolbar">
        <div>
          <h2>{t("projects.title")}</h2>
          <span>{t("count.registered", { count: projects.length })}</span>
        </div>
        <div className="table-tools">
          <label className="search-field">
            <Search size={15} aria-hidden="true" />
            <span className="sr-only">{t("projects.search")}</span>
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("projects.search")} />
          </label>
          <button className="button button--primary" onClick={onImport}>
            <Plus size={16} />
            {t("projects.import")}
          </button>
          <button className="button button--secondary" onClick={onAutoDiscover}>
            <ScanSearch size={16} />
            {discoveredCount > 0
              ? t("projects.reviewDiscovered", { count: discoveredCount })
              : t(hasSavedDiscoveryRoot ? "projects.rescanSavedRoot" : "projects.autoDiscover")}
          </button>
        </div>
      </div>

      <div className="project-list">
        {filteredProjects.map((project) => {
          const deleteBlocked = project.profiles.some((profile) =>
            profile.status === "running" ||
            profile.status === "starting" ||
            profile.pid != null ||
            busyProfileIds.has(profile.id),
          );
          return (
            <section className="project-section" key={project.id} aria-labelledby={`project-${project.id}`}>
            <header className="project-header">
              <div className="project-identity">
                <span className="project-monogram" aria-hidden="true">{project.name.slice(0, 1).toUpperCase()}</span>
                <div>
                  <h3 id={`project-${project.id}`}>{project.name}</h3>
                  <span title={project.path}>{project.path}</span>
                </div>
              </div>
              <div className="row-actions">
                <IconButton label={processActionLabel(t("profile.openFolder", { project: project.name }))} onClick={() => onOpenDirectory(project.id)} disabled={monitorOnly}>
                  <FolderOpen size={15} />
                </IconButton>
                <IconButton label={t("profile.edit", { project: project.name })} onClick={() => onEdit(project)}>
                  <Pencil size={15} />
                </IconButton>
                <IconButton
                  label={t(deleteBlocked ? "project.stopBeforeDelete" : "project.deleteAction", { project: project.name })}
                  onClick={() => onDelete(project)}
                  disabled={deleteBlocked}
                  tone="danger"
                >
                  <Trash2 size={15} />
                </IconButton>
              </div>
            </header>
            <div className="table-shell table-shell--flush">
              <table className="profiles-table profiles-table--projects">
                <thead>
                  <tr>
                    <th>{t("table.profile")}</th>
                    <th>{t("table.status")}</th>
                    <th>{t("table.command")}</th>
                    <th>{t("table.expectedPorts")}</th>
                    <th>{t("table.pid")}</th>
                    <th className="actions-column">{t("table.actions")}</th>
                  </tr>
                </thead>
                <tbody>
                  {project.profiles.map((profile) => {
                    const busy = busyProfileIds.has(profile.id);
                    const running = profile.status === "running" || profile.status === "starting";
                    return (
                      <tr key={profile.id}>
                        <td className="table-primary" title={profile.name}>{profile.name}</td>
                        <td><StatusBadge status={profile.status} /></td>
                        <td className="command-cell" title={[profile.program, ...profile.args].join(" ")}>
                          <code>{profile.program} {profile.args.join(" ")}</code>
                        </td>
                        <td className="mono-cell">
                          {profile.expectedPorts.length > 0
                            ? profile.expectedPorts.map((port) => `${port.port}/${port.protocol}`).join(", ")
                            : <span className="muted">{t("table.none")}</span>}
                        </td>
                        <td className="mono-cell">{profile.pid ?? <span className="muted">-</span>}</td>
                        <td>
                          <div className="row-actions">
                            {running ? (
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
                            <IconButton label={processActionLabel(t("profile.openBrowser", { profile: profile.name }))} onClick={() => onOpenPort(profile)} disabled={monitorOnly || !canOpenProfilePort(profile, ports)}>
                              <ExternalLink size={15} />
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
            </div>
            </section>
          );
        })}
        {filteredProjects.length === 0 && <div className="empty-state empty-state--large">{t("projects.noMatches")}</div>}
      </div>
    </div>
  );
}
