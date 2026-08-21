import { Trash2, X } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { useI18n } from "../i18n";
import type {
  RunCoveApi,
  RunLogArchivePage,
  RunLogArchiveRecord,
  RunSession,
} from "../types";
import { IconButton } from "./IconButton";
import { canDeleteArchive, describeArchive, formatArchiveSize } from "./archive";
import { useDialogFocus } from "./useDialogFocus";

interface RunLogArchiveDrawerProps {
  api: RunCoveApi;
  session: RunSession;
  projectName: string;
  /** Opens the shared confirmation; the drawer never deletes on its own. */
  onDelete: (session: RunSession) => void;
  onClose: () => void;
}

function describeError(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason);
}

interface KeyedRecord {
  key: string;
  record: RunLogArchiveRecord;
}

/**
 * Records keyed by where they came from rather than by their position.
 *
 * A page's start offset is unique within a file and the index is unique within the
 * page, so prepending an earlier page leaves every key already on screen unchanged.
 * A record carries no identity of its own — two identical lines a second apart are
 * indistinguishable — so the position in the file is the only stable key available.
 */
function keyRecords(page: RunLogArchivePage): KeyedRecord[] {
  return page.records.map((record, index) => ({
    key: `${page.pageStartOffset}:${index}`,
    record,
  }));
}

/**
 * One archived session, read from its file and never from the live buffer.
 *
 * The first read asks for no offset and gets the end of the file, which is what a
 * user opening an old run wants to see. Earlier pages are fetched on demand with the
 * `pageStartOffset` the previous page reported, so the cursor is always a boundary
 * the backend itself produced. Nothing polls: a session still being written grows
 * after this page was read, and the copy says so rather than the view jumping.
 */
export function RunLogArchiveDrawer({
  api,
  session,
  projectName,
  onDelete,
  onClose,
}: RunLogArchiveDrawerProps) {
  const { t, locale, formatTime } = useI18n();
  const [records, setRecords] = useState<KeyedRecord[]>([]);
  const [page, setPage] = useState<RunLogArchivePage | null>(null);
  const [cursor, setCursor] = useState<number | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [malformed, setMalformed] = useState(0);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [retryVersion, setRetryVersion] = useState(0);
  const outputRef = useRef<HTMLDivElement>(null);
  // Distance from the bottom to restore after older records are prepended; null means
  // "this update is a fresh tail, so show the end".
  const anchorRef = useRef<number | null>(null);
  const { dialogRef, onDialogKeyDown } = useDialogFocus(onClose);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    anchorRef.current = null;

    const readTail = async () => {
      try {
        const tail = await api.readRunLogArchive(session.id);
        if (cancelled) return;
        setPage(tail);
        setRecords(keyRecords(tail));
        setCursor(tail.pageStartOffset);
        setHasMore(tail.hasMoreBefore);
        setMalformed(tail.malformedLines);
      } catch (reason) {
        if (!cancelled) setError(t("archive.viewer.loadFailed", { detail: describeError(reason) }));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    void readTail();
    return () => {
      cancelled = true;
    };
  }, [api, session.id, retryVersion, t]);

  useLayoutEffect(() => {
    const node = outputRef.current;
    if (!node) return;
    if (anchorRef.current == null) {
      node.scrollTop = node.scrollHeight;
      return;
    }
    node.scrollTop = node.scrollHeight - anchorRef.current;
    anchorRef.current = null;
  }, [records]);

  const loadEarlier = async () => {
    if (cursor == null || cursor === 0) return;
    const node = outputRef.current;
    anchorRef.current = node ? node.scrollHeight - node.scrollTop : 0;
    setLoadingMore(true);
    setError(null);
    try {
      const earlier = await api.readRunLogArchive(session.id, cursor);
      setRecords((current) => [...keyRecords(earlier), ...current]);
      setCursor(earlier.pageStartOffset);
      setHasMore(earlier.hasMoreBefore);
      setMalformed((current) => current + earlier.malformedLines);
    } catch (reason) {
      anchorRef.current = null;
      setError(t("archive.viewer.loadFailed", { detail: describeError(reason) }));
    } finally {
      setLoadingMore(false);
    }
  };

  const badge = describeArchive(session, t, locale);
  const stillWriting = page?.status === "writing";
  const canDelete = canDeleteArchive(badge.state);

  return (
    <div className="drawer-backdrop" role="presentation" onMouseDown={onClose}>
      <aside
        ref={dialogRef}
        className="log-drawer archive-drawer"
        role="dialog"
        aria-modal="true"
        aria-labelledby="archive-drawer-title"
        aria-busy={loading}
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="drawer-header">
          <div>
            <div className="drawer-title-row">
              <h2 id="archive-drawer-title">{session.profileName}</h2>
              <span className={`status-badge archive-badge archive-badge--${badge.state}`} title={badge.detail}>
                {badge.text}
              </span>
            </div>
            <span>
              {projectName}
              {page && ` · ${t("archive.viewer.counts", {
                loaded: new Intl.NumberFormat(locale).format(records.length),
                lines: new Intl.NumberFormat(locale).format(page.lineCount),
                size: formatArchiveSize(page.fileLength, locale),
              })}`}
            </span>
          </div>
          <div className="row-actions">
            {canDelete && (
              <IconButton
                label={t("archive.delete", { profile: session.profileName })}
                tone="danger"
                onClick={() => onDelete(session)}
              >
                <Trash2 size={15} />
              </IconButton>
            )}
            <IconButton label={t("archive.viewer.close")} onClick={onClose}><X size={16} /></IconButton>
          </div>
        </header>
        {(stillWriting || malformed > 0 || page?.incompleteTailSkipped) && (
          <div className="archive-notes">
            {stillWriting && <p>{t("archive.viewer.writingHint")}</p>}
            {page?.incompleteTailSkipped && (
              <p>{stillWriting ? t("archive.viewer.truncatedTail") : t("archive.viewer.tornTail")}</p>
            )}
            {malformed > 0 && <p>{t("archive.viewer.malformed", { count: malformed })}</p>}
          </div>
        )}
        {error && (
          <div className="drawer-error" role="alert">
            <span>{error}</span>
            <button type="button" onClick={() => setRetryVersion((value) => value + 1)}>
              {t("archive.viewer.retry")}
            </button>
          </div>
        )}
        <div className="log-output" ref={outputRef}>
          {loading && <div className="log-empty">{t("archive.viewer.loading")}</div>}
          {!loading && (
            hasMore ? (
              <div className="archive-page-control">
                <button
                  className="button button--secondary"
                  type="button"
                  onClick={() => void loadEarlier()}
                  disabled={loadingMore}
                >
                  {loadingMore ? t("archive.viewer.loadingOlder") : t("archive.viewer.loadOlder")}
                </button>
              </div>
            ) : (
              records.length > 0 && <div className="archive-page-control muted">{t("archive.viewer.atStart")}</div>
            )
          )}
          {!loading && records.length === 0 && !error && (
            <div className="log-empty">{t("archive.viewer.empty")}</div>
          )}
          {records.map(({ key, record }) => (
            <div className={`log-line log-line--${record.stream}`} key={key}>
              <time>{formatTime(record.timestamp)}</time>
              <span className="log-stream">
                {record.stream === "system" ? t("logs.system") : record.stream}
              </span>
              <pre>{record.line}</pre>
            </div>
          ))}
        </div>
      </aside>
    </div>
  );
}
