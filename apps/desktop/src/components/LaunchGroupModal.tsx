import { ArrowDown, ArrowUp, X } from "lucide-react";
import { type FormEvent, useState } from "react";

import { useI18n } from "../i18n";
import type { LaunchGroup, LaunchGroupInput, Project } from "../types";
import { IconButton } from "./IconButton";
import {
  hasLaunchGroupValidationErrors,
  resolveLaunchGroupMembers,
  validateLaunchGroupInput,
} from "./launch-group";
import { useDialogFocus } from "./useDialogFocus";

interface LaunchGroupModalProps {
  /** `null` opens the editor on a new group. */
  group: LaunchGroup | null;
  /** Every saved group, so a duplicate name is refused before the round trip. */
  groups: LaunchGroup[];
  projects: Project[];
  saving: boolean;
  error?: string | null;
  onClose: () => void;
  onSave: (input: LaunchGroupInput) => void;
}

/**
 * Create or edit one launch group.
 *
 * The selected list is ordered and the order is the startup order, so selecting a
 * profile appends it rather than slotting it in by project: the user builds the
 * sequence in the order they intend it to run, then adjusts with the arrows.
 */
export function LaunchGroupModal({
  group,
  groups,
  projects,
  saving,
  error,
  onClose,
  onSave,
}: LaunchGroupModalProps) {
  const { t } = useI18n();
  const [name, setName] = useState(group?.name ?? "");
  const [selected, setSelected] = useState<string[]>(group?.profileIds ?? []);
  const [showValidation, setShowValidation] = useState(false);
  const { dialogRef, onDialogKeyDown } = useDialogFocus(onClose, saving);

  const validation = validateLaunchGroupInput({ id: group?.id, name, profileIds: selected }, groups);
  const invalid = hasLaunchGroupValidationErrors(validation);
  const members = resolveLaunchGroupMembers({ profileIds: selected }, projects);
  const offered = projects.filter((project) => project.profiles.length > 0);

  const toggle = (profileId: string) => {
    setSelected((current) =>
      current.includes(profileId)
        ? current.filter((id) => id !== profileId)
        : [...current, profileId],
    );
  };

  const move = (index: number, delta: number) => {
    setSelected((current) => {
      const target = index + delta;
      if (target < 0 || target >= current.length) return current;
      const next = [...current];
      const [moved] = next.splice(index, 1);
      next.splice(target, 0, moved);
      return next;
    });
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    setShowValidation(true);
    if (invalid) return;
    onSave({ id: group?.id, name: name.trim(), profileIds: selected });
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={saving ? undefined : onClose}>
      <section
        ref={dialogRef}
        className="modal group-modal"
        role="dialog"
        aria-modal="true"
        aria-busy={saving}
        aria-labelledby="group-modal-title"
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="modal-header">
          <div>
            <h2 id="group-modal-title">
              {group ? t("group.edit", { group: group.name }) : t("group.newTitle")}
            </h2>
            <span className="modal-subtitle">{t("group.subtitle")}</span>
          </div>
          <IconButton label={t("action.close")} onClick={onClose} disabled={saving}>
            <X size={16} />
          </IconButton>
        </div>

        <form className="group-form" noValidate onSubmit={submit}>
          <label className="field">
            <span id="group-name-label">{t("group.name")}</span>
            <input
              value={name}
              placeholder={t("group.namePlaceholder")}
              // Named explicitly because the validation message below is inside this
              // label: without it, showing an error would rename the field to "Group
              // name Another launch group already uses this name."
              aria-labelledby="group-name-label"
              aria-invalid={showValidation && Boolean(validation.name)}
              aria-describedby={showValidation && validation.name ? "group-name-error" : undefined}
              onChange={(event) => setName(event.target.value)}
            />
            {showValidation && validation.name && (
              <span className="field-error" id="group-name-error">
                {t(
                  validation.name === "duplicate"
                    ? "group.validation.nameDuplicate"
                    : "group.validation.nameRequired",
                )}
              </span>
            )}
          </label>

          <div className="group-picker">
            <div className="group-picker__column">
              <h3>{t("group.members")}</h3>
              <p className="group-picker__hint">{t("group.membersHint")}</p>
              {offered.length === 0 ? (
                <div className="empty-state">{t("group.noProfiles")}</div>
              ) : (
                <div className="group-picker__scroll">
                  {offered.map((project) => (
                    <fieldset className="group-picker__project" key={project.id}>
                      <legend>{project.name}</legend>
                      {project.profiles.map((profile) => (
                        <label className="group-picker__option" key={profile.id}>
                          <input
                            type="checkbox"
                            checked={selected.includes(profile.id)}
                            onChange={() => toggle(profile.id)}
                          />
                          <span>{profile.name}</span>
                        </label>
                      ))}
                    </fieldset>
                  ))}
                </div>
              )}
            </div>

            <div className="group-picker__column">
              <h3>{t("group.order")}</h3>
              {members.length === 0 ? (
                <div className="empty-state">{t("group.selectedEmpty")}</div>
              ) : (
                <ol className="group-order">
                  {members.map((member, index) => (
                    <li key={member.profileId}>
                      <span title={member.label}>{member.label}</span>
                      <div className="row-actions">
                        <IconButton
                          label={t("group.moveUp", { profile: member.label })}
                          onClick={() => move(index, -1)}
                          disabled={index === 0}
                        >
                          <ArrowUp size={14} />
                        </IconButton>
                        <IconButton
                          label={t("group.moveDown", { profile: member.label })}
                          onClick={() => move(index, 1)}
                          disabled={index === members.length - 1}
                        >
                          <ArrowDown size={14} />
                        </IconButton>
                        <IconButton
                          label={t("group.remove", { profile: member.label })}
                          onClick={() => toggle(member.profileId)}
                          tone="danger"
                        >
                          <X size={14} />
                        </IconButton>
                      </div>
                    </li>
                  ))}
                </ol>
              )}
              {showValidation && validation.members && (
                <div className="inline-error" role="alert">
                  {t("group.validation.membersRequired")}
                </div>
              )}
            </div>
          </div>

          {error && (
            <div className="inline-error" role="alert">
              {error}
            </div>
          )}
          <div className="modal-actions">
            <div className="modal-action-group">
              <button
                className="button button--secondary"
                type="button"
                onClick={onClose}
                disabled={saving}
              >
                {t("action.cancel")}
              </button>
              <button className="button button--primary" type="submit" disabled={saving}>
                {saving ? t("group.saving") : t("group.save")}
              </button>
            </div>
          </div>
        </form>
      </section>
    </div>
  );
}
