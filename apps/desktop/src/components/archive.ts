import type { MessageKey, MessageParams } from "../i18n/messages";
import type { RunSession } from "../types";

export type Translate = (key: MessageKey, params?: MessageParams) => string;

/**
 * What a run history row can say about one archive.
 *
 * `finalizing` is not a wire status. It is a `writing` row whose session has
 * already ended, which is the window between the child process exiting and the
 * writer closing the file. The schema's `CHECK ((status = 'writing') = (ended_at
 * IS NULL))` makes "the row is still open" and "the archive has no end time" the
 * same fact, so the session's own `endedAt` is what separates the two.
 *
 * `unknown` is for a status string this build has never heard of: a database
 * written by a newer RunCove is read, not rejected.
 */
export type ArchiveBadgeState =
  | "none"
  | "writing"
  | "finalizing"
  | "complete"
  | "partial"
  | "removed"
  | "unknown";

type ArchiveSession = Pick<RunSession, "endedAt" | "archive">;

export function archiveBadgeState(session: ArchiveSession): ArchiveBadgeState {
  const archive = session.archive;
  if (!archive) return "none";
  switch (archive.status) {
    case "writing":
      return session.endedAt == null ? "writing" : "finalizing";
    case "complete":
      return "complete";
    case "partial":
      return "partial";
    case "removed":
      return "removed";
    default:
      return "unknown";
  }
}

/**
 * Whether the viewer can be opened.
 *
 * Reading needs a file: a session that was never archived has none, and reading a
 * `removed` archive fails by design with that row's reason. An open archive can be
 * read — the reader skips a record still being flushed — and an unrecognized status
 * is attempted rather than refused, so a newer build's archive stays reachable.
 */
export function canViewArchive(state: ArchiveBadgeState): boolean {
  return state !== "none" && state !== "removed";
}

/**
 * Whether delete can be offered.
 *
 * The command refuses a session its writer still holds open, so offering it for
 * `writing` or `finalizing` would only produce an error the user cannot act on.
 */
export function canDeleteArchive(state: ArchiveBadgeState): boolean {
  return state === "complete" || state === "partial" || state === "unknown";
}

const REASON_KEYS: Record<string, MessageKey> = {
  "write-error": "archive.reason.writeError",
  "quota-exceeded": "archive.reason.quotaExceeded",
  "queue-overflow": "archive.reason.queueOverflow",
  interrupted: "archive.reason.interrupted",
  "user-disabled": "archive.reason.userDisabled",
  "quota-evicted": "archive.reason.quotaEvicted",
  "user-deleted": "archive.reason.userDeleted",
  "file-missing": "archive.reason.fileMissing",
};

/** A wire reason as a sentence, keeping an unrecognized value visible. */
export function describeArchiveReason(reason: string | null | undefined, t: Translate): string {
  if (!reason) return "";
  const key = REASON_KEYS[reason];
  return key ? t(key) : t("archive.reason.unknown", { reason });
}

const SIZE_SUFFIXES = ["B", "KiB", "MiB", "GiB", "TiB"] as const;

/**
 * A byte count in binary units.
 *
 * The suffixes are IEC rather than SI because every limit on the Rust side is a
 * power of two, so `1 KiB` is exactly the 1024 bytes it was divided by. The number
 * is localized; the unit symbol is written the same way in both languages.
 */
export function formatArchiveSize(bytes: number, locale: string): string {
  let value = Number.isFinite(bytes) && bytes > 0 ? Math.trunc(bytes) : 0;
  let index = 0;
  while (value >= 1024 && index < SIZE_SUFFIXES.length - 1) {
    value /= 1024;
    index += 1;
  }
  const formatted = new Intl.NumberFormat(locale, {
    maximumFractionDigits: index === 0 || value >= 10 ? 0 : 1,
  }).format(value);
  return `${formatted} ${SIZE_SUFFIXES[index]}`;
}

export interface ArchiveBadge {
  state: ArchiveBadgeState;
  text: string;
  /** What the archive lost, for a tooltip. Absent when it lost nothing. */
  detail?: string;
}

/**
 * One archive as a badge.
 *
 * The dropped counters are reported for every state that has them rather than only
 * for `partial`: a `complete` archive cannot have lost anything, but a `removed`
 * one can still carry what it lost before it was removed.
 */
export function describeArchive(
  session: ArchiveSession,
  t: Translate,
  locale: string,
): ArchiveBadge {
  const state = archiveBadgeState(session);
  const archive = session.archive;
  if (!archive) return { state, text: t("archive.badge.none") };

  const number = new Intl.NumberFormat(locale);
  const reason = describeArchiveReason(archive.reason, t);
  const detail = archive.droppedLines > 0 || archive.droppedBytes > 0
    ? t("archive.badge.dropped", {
      lines: number.format(archive.droppedLines),
      size: formatArchiveSize(archive.droppedBytes, locale),
    })
    : undefined;

  switch (state) {
    case "writing":
      return { state, text: t("archive.badge.writing"), detail };
    case "finalizing":
      return { state, text: t("archive.badge.finalizing"), detail };
    case "partial":
      return { state, text: t("archive.badge.partial", { reason }), detail };
    case "removed":
      return { state, text: t("archive.badge.removed", { reason }), detail };
    case "unknown":
      return { state, text: t("archive.badge.unknown", { status: archive.status }), detail };
    default:
      return {
        state,
        text: t("archive.badge.complete", {
          lines: number.format(archive.lineCount),
          size: formatArchiveSize(archive.byteSize, locale),
        }),
        detail,
      };
  }
}
