import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type {
  RunCoveApi,
  RunLogArchivePage,
  RunLogArchiveSummary,
  RunSession,
} from "../types";
import { RunLogArchiveDrawer } from "./RunLogArchiveDrawer";

const summary: RunLogArchiveSummary = {
  status: "complete",
  reason: null,
  lineCount: 2,
  byteSize: 2_048,
  droppedLines: 0,
  droppedBytes: 0,
  startedAt: 1_000,
  endedAt: 9_000,
};

const session: RunSession = {
  id: "session-1",
  profileId: "profile-web",
  profileName: "Web",
  pid: 1200,
  startedAt: 1_000,
  endedAt: 9_000,
  exitCode: 0,
  status: "exited",
  archive: summary,
};

function page(overrides: Partial<RunLogArchivePage> = {}): RunLogArchivePage {
  return {
    sessionId: session.id,
    status: "complete",
    reason: null,
    lineCount: 2,
    byteSize: 2_048,
    droppedLines: 0,
    droppedBytes: 0,
    startedAt: 1_000,
    endedAt: 9_000,
    records: [{ stream: "stdout", line: "newer", timestamp: 2 }],
    fileLength: 2_048,
    pageStartOffset: 0,
    hasMoreBefore: false,
    stoppedBy: "start",
    incompleteTailSkipped: false,
    malformedLines: 0,
    ...overrides,
  };
}

function archiveApi(overrides: Partial<RunCoveApi> = {}): RunCoveApi {
  return {
    readRunLogArchive: vi.fn().mockResolvedValue(page()),
    deleteRunLogArchive: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  } as unknown as RunCoveApi;
}

function renderDrawer(api: RunCoveApi, overrides: Partial<RunSession> = {}, onDelete = vi.fn()) {
  return {
    onDelete,
    ...render(
      <RunLogArchiveDrawer
        api={api}
        session={{ ...session, ...overrides }}
        projectName="Web project"
        onDelete={onDelete}
        onClose={vi.fn()}
      />,
    ),
  };
}

describe("RunLogArchiveDrawer", () => {
  it("asks for the end of the file first and says when that is all of it", async () => {
    const readRunLogArchive = vi.fn().mockResolvedValue(page());
    renderDrawer(archiveApi({ readRunLogArchive }));

    expect(await screen.findByText("newer")).toBeInTheDocument();
    // No offset: the backend answers with the tail, which is what someone opening an
    // old run wants to see without asking for it.
    expect(readRunLogArchive).toHaveBeenCalledWith("session-1");
    expect(screen.getByText("Start of archive")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Load earlier lines" })).not.toBeInTheDocument();
    expect(screen.getByText(/1 shown · 2 recorded · 2 KiB/)).toBeVisible();
  });

  it("pages towards the start with the offset the previous page reported", async () => {
    const user = userEvent.setup();
    const readRunLogArchive = vi.fn()
      .mockResolvedValueOnce(page({ pageStartOffset: 400, hasMoreBefore: true }))
      .mockResolvedValueOnce(page({
        records: [{ stream: "stderr", line: "older", timestamp: 1 }],
        pageStartOffset: 0,
        hasMoreBefore: false,
      }));
    renderDrawer(archiveApi({ readRunLogArchive }));

    await user.click(await screen.findByRole("button", { name: "Load earlier lines" }));

    expect(await screen.findByText("older")).toBeInTheDocument();
    expect(readRunLogArchive).toHaveBeenNthCalledWith(2, "session-1", 400);
    // Prepended, not appended: the file reads oldest first in both directions.
    expect(screen.getAllByText(/^(older|newer)$/).map((node) => node.textContent))
      .toEqual(["older", "newer"]);
    expect(screen.getByText("Start of archive")).toBeVisible();
  });

  it("adds up the unreadable records of every page it has loaded", async () => {
    const user = userEvent.setup();
    const readRunLogArchive = vi.fn()
      .mockResolvedValueOnce(page({ pageStartOffset: 400, hasMoreBefore: true, malformedLines: 2 }))
      .mockResolvedValueOnce(page({
        records: [{ stream: "stdout", line: "older", timestamp: 1 }],
        malformedLines: 3,
      }));
    renderDrawer(archiveApi({ readRunLogArchive }));

    expect(await screen.findByText("2 unreadable records skipped")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Load earlier lines" }));

    expect(await screen.findByText("5 unreadable records skipped")).toBeVisible();
  });

  it("warns that an open archive grows after the page it just read", async () => {
    const readRunLogArchive = vi.fn().mockResolvedValue(page({
      status: "writing",
      endedAt: null,
      incompleteTailSkipped: true,
    }));
    renderDrawer(archiveApi({ readRunLogArchive }), {
      status: "running",
      endedAt: null,
      exitCode: null,
      archive: { ...summary, status: "writing", endedAt: null },
    });

    expect(await screen.findByText(/This run is still being archived/)).toBeVisible();
    // Mid-flush rather than damaged: the record is coming, so it is not called torn.
    expect(screen.getByText("The final record was still being written and is not shown yet.")).toBeVisible();
    expect(screen.queryByText("The final record is incomplete and was skipped.")).not.toBeInTheDocument();
    // The command refuses a file its writer holds open, so delete is not offered.
    expect(screen.queryByRole("button", { name: "Delete archived Web logs" })).not.toBeInTheDocument();
  });

  it("calls a finished archive's skipped tail torn", async () => {
    const readRunLogArchive = vi.fn().mockResolvedValue(page({ incompleteTailSkipped: true }));
    renderDrawer(archiveApi({ readRunLogArchive }));

    expect(await screen.findByText("The final record is incomplete and was skipped.")).toBeVisible();
    expect(screen.queryByText(/This run is still being archived/)).not.toBeInTheDocument();
  });

  it("offers a retry that reads the file again", async () => {
    const user = userEvent.setup();
    const readRunLogArchive = vi.fn()
      .mockRejectedValueOnce(new Error("disk error"))
      .mockResolvedValueOnce(page());
    renderDrawer(archiveApi({ readRunLogArchive }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Archived logs could not be loaded: disk error",
    );
    await user.click(screen.getByRole("button", { name: "Retry loading archived logs" }));

    expect(await screen.findByText("newer")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
    expect(readRunLogArchive).toHaveBeenCalledTimes(2);
  });

  it("hands a delete to the confirmation instead of deleting the file itself", async () => {
    const user = userEvent.setup();
    const deleteRunLogArchive = vi.fn();
    const onDelete = vi.fn();
    renderDrawer(archiveApi({ deleteRunLogArchive }), {}, onDelete);

    await user.click(await screen.findByRole("button", { name: "Delete archived Web logs" }));

    expect(onDelete).toHaveBeenCalledWith(expect.objectContaining({ id: "session-1" }));
    expect(deleteRunLogArchive).not.toHaveBeenCalled();
  });

  it("says an archive holds nothing rather than looking like it is still loading", async () => {
    const readRunLogArchive = vi.fn().mockResolvedValue(page({ records: [], lineCount: 0 }));
    renderDrawer(archiveApi({ readRunLogArchive }));

    expect(await screen.findByText("This archive holds no lines")).toBeVisible();
    expect(screen.queryByText("Loading archived logs...")).not.toBeInTheDocument();
  });
});
