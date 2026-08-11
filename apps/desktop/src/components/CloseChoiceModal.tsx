import { LogOut, Minimize2, X } from "lucide-react";

import { useI18n } from "../i18n";
import type { CloseBehavior } from "../types";
import { IconButton } from "./IconButton";
import { useDialogFocus } from "./useDialogFocus";

interface CloseChoiceModalProps {
  remember: boolean;
  busyAction?: Exclude<CloseBehavior, "ask"> | null;
  onRememberChange: (remember: boolean) => void;
  onCancel: () => void;
  onChoose: (behavior: Exclude<CloseBehavior, "ask">) => void;
}

export function CloseChoiceModal({
  remember,
  busyAction,
  onRememberChange,
  onCancel,
  onChoose,
}: CloseChoiceModalProps) {
  const { t } = useI18n();
  const busy = Boolean(busyAction);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLElement>(onCancel, busy);

  return (
    <div className="modal-backdrop close-choice-backdrop" role="presentation" onMouseDown={busy ? undefined : onCancel}>
      <section
        ref={dialogRef}
        className="modal close-choice-modal"
        role="dialog"
        aria-modal="true"
        aria-busy={busy}
        aria-labelledby="close-choice-title"
        aria-describedby="close-choice-detail"
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="modal-header">
          <div>
            <h2 id="close-choice-title">{t("dialog.closeTitle")}</h2>
            <p id="close-choice-detail" className="modal-subtitle">{t("dialog.closeDetail")}</p>
          </div>
          <IconButton label={t("action.cancel")} onClick={onCancel} disabled={busy}>
            <X size={16} />
          </IconButton>
        </header>

        <div className="close-choice-options">
          <button
            type="button"
            className="close-choice-option"
            autoFocus
            disabled={busy}
            aria-describedby="close-choice-hide-detail"
            onClick={() => onChoose("hideToTray")}
          >
            <span className="close-choice-icon" aria-hidden="true"><Minimize2 size={18} /></span>
            <span className="close-choice-copy">
              <strong>{busyAction === "hideToTray" ? t("action.working") : t("dialog.closeHide")}</strong>
              <small id="close-choice-hide-detail">{t("dialog.closeHideDetail")}</small>
            </span>
          </button>
          <button
            type="button"
            className="close-choice-option close-choice-option--danger"
            disabled={busy}
            aria-describedby="close-choice-quit-detail"
            onClick={() => onChoose("quit")}
          >
            <span className="close-choice-icon" aria-hidden="true"><LogOut size={18} /></span>
            <span className="close-choice-copy">
              <strong>{busyAction === "quit" ? t("action.working") : t("dialog.closeQuit")}</strong>
              <small id="close-choice-quit-detail">{t("dialog.closeQuitDetail")}</small>
            </span>
          </button>
        </div>

        <label className="close-choice-remember">
          <input
            type="checkbox"
            checked={remember}
            disabled={busy}
            onChange={(event) => onRememberChange(event.target.checked)}
          />
          <span>{t("dialog.closeRemember")}</span>
        </label>
        <p className="close-choice-reset-hint">{t("dialog.closeResetHint")}</p>
      </section>
    </div>
  );
}
