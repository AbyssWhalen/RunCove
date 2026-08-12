import { BadgeCheck, ExternalLink, Info, Play, Search, Square, X } from "lucide-react";
import { Fragment, useEffect, useMemo, useState } from "react";

import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n";
import { groupDisplayPorts } from "../port-display";
import type { DashboardSnapshot, PortSnapshot, Project } from "../types";
import { IconButton } from "./IconButton";
import { AssociationBadge } from "./StatusBadge";

type PortFilter = "all" | "active" | "historical" | "unassigned";

interface PortsViewProps {
  snapshot: DashboardSnapshot;
  busyProfileIds: Set<string>;
  onOpenPort: (port: PortSnapshot) => void;
  onTerminate: (port: PortSnapshot) => void;
  onConfirmAssociation: (port: PortSnapshot) => void;
  onStartProfile: (profileId: string) => void;
}

function getProjectName(port: PortSnapshot, projects: Project[]): string | null {
  return projects.find((project) => project.id === port.projectId)?.name ?? null;
}

function hasVerifiedProcessIdentity(port: PortSnapshot): boolean {
  return port.active &&
    port.pid != null &&
    port.processStartedAt != null &&
    Boolean(port.executablePath?.trim());
}

const filterLabels: Record<PortFilter, MessageKey> = {
  all: "ports.filterAll",
  active: "ports.filterActive",
  historical: "ports.filterHistorical",
  unassigned: "ports.filterUnassigned",
};

const COMPACT_PORTS_QUERY = "(max-width: 1199px)";

function isCompactPortsTable(): boolean {
  return typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia(COMPACT_PORTS_QUERY).matches;
}

export function PortsView({ snapshot, busyProfileIds, onOpenPort, onTerminate, onConfirmAssociation, onStartProfile }: PortsViewProps) {
  const { t, formatDateTime } = useI18n();
  const monitorOnly = snapshot.privilege.monitorOnly;
  const processActionLabel = (label: string) =>
    monitorOnly ? `${label}: ${t("privilege.monitorOnlyAction")}` : label;
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<PortFilter>("all");
  const [expandedKey, setExpandedKey] = useState<string | null>(null);
  const [compactTable, setCompactTable] = useState(isCompactPortsTable);
  const displayPorts = useMemo(() => groupDisplayPorts(snapshot.ports), [snapshot.ports]);

  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const media = window.matchMedia(COMPACT_PORTS_QUERY);
    const update = (event: MediaQueryListEvent) => setCompactTable(event.matches);
    setCompactTable(media.matches);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  const filteredPorts = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return displayPorts.filter((port) => {
      const projectName = getProjectName(port, snapshot.projects);
      const matchesFilter =
        filter === "all" ||
        (filter === "active" && port.active) ||
        (filter === "historical" && !port.active) ||
        (filter === "unassigned" && !port.projectId);
      const haystack = [
        String(port.port),
        port.protocol,
        port.bindAddress,
        String(port.pid ?? ""),
        port.processName ?? "",
        port.executablePath ?? "",
        port.commandLine ?? "",
        projectName ?? "",
      ].join(" ").toLowerCase();
      return matchesFilter && (!needle || haystack.includes(needle));
    });
  }, [displayPorts, filter, query, snapshot.projects]);

  return (
    <section className="data-section view-stack" aria-labelledby="ports-heading">
      <div className="section-heading section-heading--toolbar">
        <div>
          <h2 id="ports-heading">{t("ports.title")}</h2>
          <span>{t("count.activeHistorical", {
            active: displayPorts.filter((port) => port.active).length,
            historical: displayPorts.filter((port) => !port.active).length,
          })}</span>
        </div>
        <div className="table-tools">
          <div className="segmented-control" aria-label={t("ports.filterLabel")}>
            {(["all", "active", "historical", "unassigned"] as const).map((value) => (
              <button
                type="button"
                key={value}
                className={filter === value ? "is-selected" : ""}
                aria-pressed={filter === value}
                onClick={() => setFilter(value)}
              >
                {t(filterLabels[value])}
              </button>
            ))}
          </div>
          <label className="search-field">
            <Search size={15} aria-hidden="true" />
            <span className="sr-only">{t("ports.search")}</span>
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("ports.search")} />
            {query && (
              <button type="button" aria-label={t("ports.clearSearch")} title={t("ports.clearSearch")} onClick={() => setQuery("")}>
                <X size={14} />
              </button>
            )}
          </label>
        </div>
      </div>

      <div className="table-shell">
        <table className="ports-table">
          <thead>
            <tr>
              <th>{t("ports.port")}</th>
              <th>{t("ports.state")}</th>
              <th>{t("ports.binding")}</th>
              <th>{t("ports.process")}</th>
              <th>{t("table.project")}</th>
              <th>{t("ports.association")}</th>
              <th>{t("ports.lastSeen")}</th>
              <th className="actions-column">{t("table.actions")}</th>
            </tr>
          </thead>
          <tbody>
            {filteredPorts.map((port, index) => {
              const key = `${port.protocol}:${port.bindAddress}:${port.port}:${port.projectId}:${port.profileId}:${port.associationSource}:${index}`;
              const projectName = getProjectName(port, snapshot.projects);
              const hasVerifiedIdentity = hasVerifiedProcessIdentity(port);
              const canConfirm = hasVerifiedIdentity && port.associationSource === "suggested" && Boolean(port.projectId);
              const canTerminate = hasVerifiedIdentity && port.associationSource !== "managed";
              const expanded = expandedKey === key;
              return (
                <Fragment key={key}>
                <tr className={expanded ? "is-expanded" : ""}>
                  <td className="port-cell"><strong>{port.port}</strong><span>/{port.protocol}</span></td>
                  <td>
                    <span className={`port-state ${port.active ? "port-state--active" : "port-state--historical"}`}>
                      {port.active ? t("ports.active") : t("ports.historical")}
                    </span>
                  </td>
                  <td className="mono-cell" title={port.bindAddress ?? t("ports.unavailable")}>
                    {port.bindAddress ?? t("ports.unavailable")}
                    {port.isPublic && <span className="public-flag">{t("ports.public")}</span>}
                  </td>
                  <td>
                    <div className="cell-stack" title={port.processName ?? t("ports.notRunning")}>
                      <strong>{port.processName ?? t("ports.notRunning")}</strong>
                      <span>{port.pid ? `PID ${port.pid}` : port.state}</span>
                    </div>
                  </td>
                  <td title={projectName ?? t("association.unassigned")}>{projectName ?? <span className="muted">{t("association.unassigned")}</span>}</td>
                  <td><AssociationBadge source={port.associationSource} /></td>
                  <td>{port.lastSeenAt ? formatDateTime(port.lastSeenAt, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }) : "-"}</td>
                  <td>
                    <div className="row-actions">
                      <IconButton label={t("ports.viewDetails", { port: port.port })} onClick={() => setExpandedKey(expanded ? null : key)}>
                        <Info size={15} />
                      </IconButton>
                      <IconButton label={processActionLabel(t("ports.openBrowser", { port: port.port }))} onClick={() => onOpenPort(port)} disabled={monitorOnly || !port.active || port.protocol !== "tcp"}>
                        <ExternalLink size={15} />
                      </IconButton>
                      {canConfirm && (
                        <IconButton label={t("ports.confirmAssociation", { port: port.port })} onClick={() => onConfirmAssociation(port)} tone="success">
                          <BadgeCheck size={15} />
                        </IconButton>
                      )}
                      {!port.active && port.profileId && (
                        <IconButton
                          label={processActionLabel(t("ports.startHistorical", { port: port.port }))}
                          onClick={() => onStartProfile(port.profileId!)}
                          disabled={monitorOnly || busyProfileIds.has(port.profileId)}
                          tone="success"
                        >
                          <Play size={15} fill="currentColor" />
                        </IconButton>
                      )}
                      {canTerminate && (
                        <IconButton label={processActionLabel(t("ports.terminate", { pid: port.pid!, port: port.port }))} onClick={() => onTerminate(port)} disabled={monitorOnly} tone="danger">
                          <Square size={14} fill="currentColor" />
                        </IconButton>
                      )}
                    </div>
                  </td>
                </tr>
                {expanded && (
                  <tr className="port-detail-row">
                    <td colSpan={compactTable ? 4 : 8}>
                      <div className="port-detail-panel" role="region" aria-label={t("ports.details", { port: port.port })}>
                        <dl>
                          <div><dt>{t("ports.binding")}</dt><dd>{port.bindAddress ?? t("ports.unavailable")}</dd></div>
                          <div><dt>{t("table.project")}</dt><dd>{projectName ?? t("association.unassigned")}</dd></div>
                          <div><dt>{t("ports.association")}</dt><dd><AssociationBadge source={port.associationSource} /></dd></div>
                          <div><dt>{t("ports.lastSeen")}</dt><dd>{port.lastSeenAt ? formatDateTime(port.lastSeenAt, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }) : "-"}</dd></div>
                          <div><dt>{t("ports.executable")}</dt><dd>{port.executablePath ?? t("ports.unavailable")}</dd></div>
                          <div><dt>{t("table.command")}</dt><dd>{port.commandLine?.trim() || t("ports.unavailable")}</dd></div>
                          <div><dt>{t("ports.processStarted")}</dt><dd>{port.processStartedAt ? formatDateTime(port.processStartedAt, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }) : "-"}</dd></div>
                        </dl>
                      </div>
                    </td>
                  </tr>
                )}
                </Fragment>
              );
            })}
          </tbody>
        </table>
        {filteredPorts.length === 0 && <div className="empty-state">{t("ports.noMatches")}</div>}
      </div>
    </section>
  );
}
