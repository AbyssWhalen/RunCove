import type { AssociationSource, ProfileStatus } from "../types";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n";

const statusLabels: Record<ProfileStatus, MessageKey> = {
  idle: "status.idle",
  starting: "status.starting",
  running: "status.running",
  conflict: "status.conflict",
  exited: "status.exited",
  unknown: "status.unknown",
};

export function StatusBadge({ status }: { status: ProfileStatus }) {
  const { t } = useI18n();
  return (
    <span className={`status-badge status-badge--${status}`}>
      <span className="status-dot" aria-hidden="true" />
      {t(statusLabels[status])}
    </span>
  );
}

const sourceLabels: Record<AssociationSource, MessageKey> = {
  managed: "association.managed",
  confirmed: "association.confirmed",
  suggested: "association.suggested",
};

export function AssociationBadge({ source }: { source?: AssociationSource | null }) {
  const { t } = useI18n();
  if (!source) return <span className="muted">{t("association.unassigned")}</span>;
  return <span className={`association-badge association-badge--${source}`}>{t(sourceLabels[source])}</span>;
}
