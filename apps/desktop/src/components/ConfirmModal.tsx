import { AlertTriangle, X } from "lucide-react";

import { useI18n } from "../i18n";
import { IconButton } from "./IconButton";
import { useDialogFocus } from "./useDialogFocus";

interface ConfirmModalProps {
  title: string;
  detail: string;
  confirmLabel: string;
  busy?: boolean;
  danger?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}

export function ConfirmModal({
  title,
  detail,
  confirmLabel,
  busy,
  danger = true,
  onCancel,
  onConfirm,
}: ConfirmModalProps) {
  const { t } = useI18n();
  const { dialogRef, onDialogKeyDown } = useDialogFocus(onCancel, busy);
  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={busy ? undefined : onCancel}>
      <section
        ref={dialogRef}
        className="modal confirm-modal"
        role="alertdialog"
        aria-modal="true"
        aria-busy={busy}
        aria-labelledby="confirm-title"
        aria-describedby="confirm-detail"
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="modal-header">
          <div className="modal-title-group">
            <span className={`modal-symbol ${danger ? "modal-symbol--danger" : ""}`}>
              <AlertTriangle size={17} />
            </span>
            <h2 id="confirm-title">{title}</h2>
          </div>
          <IconButton label={t("action.close")} onClick={onCancel} disabled={busy}>
            <X size={16} />
          </IconButton>
        </div>
        <p id="confirm-detail" className="confirm-detail">{detail}</p>
        <div className="modal-actions">
          <button className="button button--secondary" onClick={onCancel} disabled={busy}>{t("action.cancel")}</button>
          <button className={danger ? "button button--danger" : "button button--primary"} onClick={onConfirm} disabled={busy}>
            {busy ? t("action.working") : confirmLabel}
          </button>
        </div>
      </section>
    </div>
  );
}
