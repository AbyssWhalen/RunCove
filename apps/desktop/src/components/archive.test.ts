import { describe, expect, it } from "vitest";

import { renderMessage } from "../i18n/context";
import type { MessageKey, MessageParams } from "../i18n/messages";
import type { RunLogArchiveSummary, RunSession } from "../types";
import {
  archiveBadgeState,
  canDeleteArchive,
  canViewArchive,
  describeArchive,
  describeArchiveReason,
  formatArchiveSize,
} from "./archive";

const en = (key: MessageKey, params?: MessageParams) => renderMessage("en", key, params);
const zh = (key: MessageKey, params?: MessageParams) => renderMessage("zh-CN", key, params);

function summary(overrides: Partial<RunLogArchiveSummary> = {}): RunLogArchiveSummary {
  return {
    status: "complete",
    reason: null,
    lineCount: 12,
    byteSize: 3_500,
    droppedLines: 0,
    droppedBytes: 0,
    startedAt: 1_000,
    endedAt: 2_000,
    ...overrides,
  };
}

function session(overrides: Partial<RunSession> = {}): RunSession {
  return {
    id: "session-1",
    profileId: "profile-1",
    profileName: "dev",
    pid: 4242,
    startedAt: 1_000,
    endedAt: 2_000,
    exitCode: 0,
    status: "exited",
    archive: summary(),
    ...overrides,
  };
}

describe("archive badge state", () => {
  it("reads every status this build knows", () => {
    expect(archiveBadgeState(session({ archive: null }))).toBe("none");
    expect(archiveBadgeState(session({ archive: undefined }))).toBe("none");
    expect(archiveBadgeState(session({ archive: summary({ status: "complete" }) }))).toBe("complete");
    expect(archiveBadgeState(session({
      archive: summary({ status: "partial", reason: "write-error" }),
    }))).toBe("partial");
    expect(archiveBadgeState(session({
      archive: summary({ status: "removed", reason: "user-deleted" }),
    }))).toBe("removed");
  });

  it("separates an open archive from one whose session already ended", () => {
    const open = summary({ status: "writing", endedAt: null });
    expect(archiveBadgeState(session({ archive: open, endedAt: null }))).toBe("writing");
    expect(archiveBadgeState(session({ archive: open, endedAt: 2_000 }))).toBe("finalizing");
  });

  it("keeps a status from a newer build readable instead of rejecting it", () => {
    expect(archiveBadgeState(session({ archive: summary({ status: "sealed" }) }))).toBe("unknown");
  });
});

describe("archive capabilities", () => {
  it("offers viewing for every state that still has a file", () => {
    expect(canViewArchive("writing")).toBe(true);
    expect(canViewArchive("finalizing")).toBe(true);
    expect(canViewArchive("complete")).toBe(true);
    expect(canViewArchive("partial")).toBe(true);
    expect(canViewArchive("unknown")).toBe(true);
    expect(canViewArchive("none")).toBe(false);
    expect(canViewArchive("removed")).toBe(false);
  });

  it("never offers to delete an archive the writer still holds open", () => {
    expect(canDeleteArchive("writing")).toBe(false);
    expect(canDeleteArchive("finalizing")).toBe(false);
    expect(canDeleteArchive("none")).toBe(false);
    expect(canDeleteArchive("removed")).toBe(false);
    expect(canDeleteArchive("complete")).toBe(true);
    expect(canDeleteArchive("partial")).toBe(true);
    expect(canDeleteArchive("unknown")).toBe(true);
  });
});

describe("archive size formatting", () => {
  it("uses binary units and keeps one fraction digit below ten", () => {
    expect(formatArchiveSize(0, "en-US")).toBe("0 B");
    expect(formatArchiveSize(512, "en-US")).toBe("512 B");
    expect(formatArchiveSize(1_024, "en-US")).toBe("1 KiB");
    expect(formatArchiveSize(1_536, "en-US")).toBe("1.5 KiB");
    expect(formatArchiveSize(3_500, "en-US")).toBe("3.4 KiB");
    expect(formatArchiveSize(10 * 1_024, "en-US")).toBe("10 KiB");
    expect(formatArchiveSize(1_048_576, "en-US")).toBe("1 MiB");
    expect(formatArchiveSize(1_610_612_736, "en-US")).toBe("1.5 GiB");
  });

  it("reports a byte count it cannot use as zero rather than as text", () => {
    expect(formatArchiveSize(-1, "en-US")).toBe("0 B");
    expect(formatArchiveSize(Number.NaN, "en-US")).toBe("0 B");
    expect(formatArchiveSize(Number.POSITIVE_INFINITY, "en-US")).toBe("0 B");
  });

  it("localizes the number and leaves the unit symbol alone", () => {
    expect(formatArchiveSize(1_536, "zh-CN")).toBe("1.5 KiB");
  });
});

describe("archive reasons", () => {
  it("has a sentence for every reason the backend can write", () => {
    expect(describeArchiveReason("write-error", en)).toBe("writing failed");
    expect(describeArchiveReason("quota-exceeded", en)).toBe("size limit reached");
    expect(describeArchiveReason("queue-overflow", en)).toBe(
      "output arrived faster than it could be written",
    );
    expect(describeArchiveReason("interrupted", en)).toBe("RunCove exited first");
    expect(describeArchiveReason("user-disabled", en)).toBe("archiving was turned off");
    expect(describeArchiveReason("quota-evicted", en)).toBe("removed to free space");
    expect(describeArchiveReason("user-deleted", en)).toBe("deleted by you");
    expect(describeArchiveReason("file-missing", en)).toBe("the file is gone");
  });

  it("shows an unrecognized reason instead of hiding it", () => {
    expect(describeArchiveReason("sealed-by-policy", en)).toBe(
      "unrecognized reason: sealed-by-policy",
    );
  });

  it("has nothing to say when there is no reason", () => {
    expect(describeArchiveReason(null, en)).toBe("");
    expect(describeArchiveReason(undefined, en)).toBe("");
    expect(describeArchiveReason("", en)).toBe("");
  });
});

describe("archive badges", () => {
  it("states that a session was never archived", () => {
    expect(describeArchive(session({ archive: null }), en, "en-US")).toEqual({
      state: "none",
      text: "Not archived",
    });
  });

  it("reports lines and size for a complete archive", () => {
    expect(describeArchive(session(), en, "en-US")).toEqual({
      state: "complete",
      text: "12 lines · 3.4 KiB",
      detail: undefined,
    });
    expect(describeArchive(session(), zh, "zh-CN").text).toBe("12 行 · 3.4 KiB");
  });

  it("groups a large line count for the locale", () => {
    const large = session({ archive: summary({ lineCount: 12_345, byteSize: 1_048_576 }) });
    expect(describeArchive(large, en, "en-US").text).toBe("12,345 lines · 1 MiB");
  });

  it("names why an archive is partial", () => {
    const partial = session({
      archive: summary({ status: "partial", reason: "quota-exceeded" }),
    });
    expect(describeArchive(partial, en, "en-US").text).toBe("Partial · size limit reached");
    expect(describeArchive(partial, zh, "zh-CN").text).toBe("不完整 · 达到容量上限");
  });

  it("names why an archive is gone", () => {
    const removed = session({ archive: summary({ status: "removed", reason: "user-deleted" }) });
    expect(describeArchive(removed, en, "en-US").text).toBe("Removed · deleted by you");
  });

  it("still renders a status or reason it does not recognize", () => {
    const sealed = session({ archive: summary({ status: "sealed", reason: "sealed-by-policy" }) });
    expect(describeArchive(sealed, en, "en-US")).toEqual({
      state: "unknown",
      text: "Unrecognized archive state: sealed",
      detail: undefined,
    });
    const oddReason = session({ archive: summary({ status: "partial", reason: "gremlins" }) });
    expect(describeArchive(oddReason, en, "en-US").text).toBe("Partial · unrecognized reason: gremlins");
  });

  it("distinguishes an archive being written from one being closed", () => {
    const open = summary({ status: "writing", endedAt: null });
    expect(describeArchive(session({ archive: open, endedAt: null }), en, "en-US").text)
      .toBe("Archiving");
    expect(describeArchive(session({ archive: open, endedAt: 2_000 }), en, "en-US").text)
      .toBe("Finalizing");
  });

  it("reports what an archive lost, and says nothing when it lost nothing", () => {
    expect(describeArchive(session(), en, "en-US").detail).toBeUndefined();
    const lossy = session({
      archive: summary({
        status: "partial",
        reason: "queue-overflow",
        droppedLines: 184,
        droppedBytes: 61_440,
      }),
    });
    expect(describeArchive(lossy, en, "en-US").detail).toBe("184 lines were not archived (60 KiB)");
    expect(describeArchive(lossy, zh, "zh-CN").detail).toBe("有 184 行未被归档（60 KiB）");
  });

  it("counts a single dropped line in the singular", () => {
    const one = session({
      archive: summary({ status: "partial", reason: "write-error", droppedLines: 1, droppedBytes: 0 }),
    });
    expect(describeArchive(one, en, "en-US").detail).toBe("1 line was not archived (0 B)");
  });

  it("reports a loss that a removed archive carried before it was removed", () => {
    const removed = session({
      archive: summary({
        status: "removed",
        reason: "quota-evicted",
        droppedLines: 3,
        droppedBytes: 120,
      }),
    });
    expect(describeArchive(removed, en, "en-US").detail).toBe("3 lines were not archived (120 B)");
  });
});
