import { Pencil, Play, Plus, Square, Trash2 } from "lucide-react";

import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n";
import type { LaunchGroup, Project } from "../types";
import { IconButton } from "./IconButton";
import {
  deriveLaunchGroupStatus,
  resolveLaunchGroupMembers,
  type LaunchGroupStatus,
} from "./launch-group";

const STATUS_LABELS: Record<LaunchGroupStatus, MessageKey> = {
  running: "group.status.running",
  partial: "group.status.partial",
  idle: "group.status.idle",
};

/** Which whole-group action is in flight, per group id. */
export type GroupAction = "start" | "stop";

interface LaunchGroupSectionProps {
  groups: LaunchGroup[];
  projects: Project[];
  monitorOnly: boolean;
  /** Absent means idle; the value is what the buttons report while they wait. */
  busyGroups: ReadonlyMap<string, GroupAction>;
  /**
   * Whether a restore is walking the profiles right now.
   *
   * A restore starts profiles through the same per-profile reservation a group start
   * uses, so running both at once only earns a refusal from whichever arrives second.
   * Defaults to `false` so a caller that never restores need not pass it.
   */
  restoreBusy?: boolean;
  onNew: () => void;
  onEdit: (group: LaunchGroup) => void;
  onDelete: (group: LaunchGroup) => void;
  onStart: (group: LaunchGroup) => void;
  onStop: (group: LaunchGroup) => void;
}

/**
 * The launch groups band, between the restore band and the profile table.
 *
 * It sits on Overview because a group is what the user reaches for when the window
 * opens: the same place, and the same one-button shape, as restoring the previous
 * run. Every group action lives here, so there is no separate navigation page.
 */
export function LaunchGroupSection({
  groups,
  projects,
  monitorOnly,
  busyGroups,
  restoreBusy = false,
  onNew,
  onEdit,
  onDelete,
  onStart,
  onStop,
}: LaunchGroupSectionProps) {
  const { t } = useI18n();
  const processActionLabel = (label: string) =>
    monitorOnly ? `${label}: ${t("privilege.monitorOnlyAction")}` : label;

  return (
    <section className="data-section" aria-labelledby="groups-heading">
      <div className="section-heading">
        <div>
          <h2 id="groups-heading">{t("group.title")}</h2>
          <span>
            {t("count.groups", { count: groups.length })} · {t("group.subtitle")}
          </span>
        </div>
        <button type="button" className="button button--secondary button--compact" onClick={onNew}>
          <Plus size={15} />
          {t("group.new")}
        </button>
      </div>

      {groups.length === 0 ? (
        <div className="empty-state">{t("group.empty")}</div>
      ) : (
        <ul className="group-list">
          {groups.map((group) => {
            const members = resolveLaunchGroupMembers(group, projects);
            const status = deriveLaunchGroupStatus(members);
            const pending = busyGroups.get(group.id);
            // An empty group is a group the profile cascade emptied. Its buttons are
            // dead rather than hidden, so the row still says which group needs editing.
            // `restoreBusy` joins them because a restore is already walking these same
            // profiles: leaving the button live would let a click land and do nothing.
            const unusable =
              monitorOnly || restoreBusy || pending !== undefined || members.length === 0;
            return (
              <li className="group-card" key={group.id}>
                <div className="group-card__body">
                  <div className="group-card__identity">
                    <h3>{group.name}</h3>
                    <span className={`status-badge status-badge--${status}`}>
                      <span className="status-dot" aria-hidden="true" />
                      {t(STATUS_LABELS[status])}
                    </span>
                    <span className="muted">{t("count.profiles", { count: members.length })}</span>
                  </div>
                  {members.length === 0 ? (
                    <p className="group-card__warning">{t("group.noMembers")}</p>
                  ) : (
                    <ol className="restore-sequence" aria-label={t("group.order")}>
                      {members.map((member, index) => {
                        const label = member.missing
                          ? t("group.missingMember", { profile: member.profileId })
                          : member.label;
                        return (
                          <li
                            key={`${member.profileId}:${index}`}
                            title={label}
                            className={member.missing ? "sequence-item--missing" : undefined}
                          >
                            {label}
                          </li>
                        );
                      })}
                    </ol>
                  )}
                </div>
                <div className="group-card__actions">
                  <button
                    type="button"
                    className="button button--primary button--compact"
                    aria-label={processActionLabel(t("group.start", { group: group.name }))}
                    title={processActionLabel(t("group.start", { group: group.name }))}
                    onClick={() => onStart(group)}
                    disabled={unusable || status === "running"}
                  >
                    <Play size={15} fill="currentColor" />
                    {pending === "start" ? t("group.starting") : t("action.start")}
                  </button>
                  <button
                    type="button"
                    className="button button--secondary button--compact"
                    aria-label={processActionLabel(t("group.stop", { group: group.name }))}
                    title={processActionLabel(t("group.stop", { group: group.name }))}
                    onClick={() => onStop(group)}
                    disabled={unusable || status === "idle"}
                  >
                    <Square size={14} fill="currentColor" />
                    {pending === "stop" ? t("group.stopping") : t("action.stop")}
                  </button>
                  <IconButton
                    label={t("group.edit", { group: group.name })}
                    onClick={() => onEdit(group)}
                    disabled={pending !== undefined}
                  >
                    <Pencil size={15} />
                  </IconButton>
                  <IconButton
                    label={t("group.delete", { group: group.name })}
                    onClick={() => onDelete(group)}
                    disabled={pending !== undefined}
                    tone="danger"
                  >
                    <Trash2 size={15} />
                  </IconButton>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
