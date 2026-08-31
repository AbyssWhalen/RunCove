import type { MessageKey, MessageParams } from "./i18n/messages";
import type { RunStatusReason } from "./types";

export type Translate = (key: MessageKey, params?: MessageParams) => string;

/**
 * The fixed reasons this build can render.
 *
 * The wire values are the backend's `RunStatusReason` variants in kebab case. A
 * value missing from this table is not an error: the backend also sends its own
 * English sentence, and showing that is better than showing nothing.
 */
const REASON_KEYS: Record<string, MessageKey> = {
  "stop-requested": "runStatus.stopRequested",
  "user-stop": "runStatus.userStop",
  shutdown: "runStatus.shutdown",
  "startup-not-ready": "runStatus.startupNotReady",
  "exited-normally": "runStatus.exitedNormally",
  "already-running": "runStatus.alreadyRunning",
};

/**
 * One lifecycle reason in the window's language, or `null` when this build cannot
 * name it.
 *
 * `null` is the caller's cue to fall back to the backend's English text, so the two
 * reasons that carry data are only rendered here when that data is actually
 * present: an `exited-unexpectedly` without a code still has a sentence of its
 * own, but a `wait-failed` without its detail would lose the only thing that
 * explains it.
 */
export function describeRunStatusReason(
  reason: RunStatusReason | null | undefined,
  t: Translate,
): string | null {
  if (!reason?.kind) return null;
  const key = REASON_KEYS[reason.kind];
  if (key) return t(key);
  if (reason.kind === "exited-unexpectedly") {
    return reason.code == null
      ? t("runStatus.exitedUnexpectedly")
      : t("runStatus.exitedUnexpectedlyWithCode", { code: reason.code });
  }
  if (reason.kind === "wait-failed" && reason.detail) {
    return t("runStatus.waitFailed", { detail: reason.detail });
  }
  return null;
}

/**
 * What a status change should say: RunCove's own reason in the user's language,
 * otherwise whatever text the backend sent.
 *
 * Backend text is not always English copy that could have been localized — an
 * `AppError` or an operating-system message has no reason attached and passes
 * through unchanged.
 */
export function runStatusText(
  event: { reason?: RunStatusReason | null; message?: string | null },
  t: Translate,
): string {
  return describeRunStatusReason(event.reason, t) ?? event.message ?? "";
}
