import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, resetMockApi } from "./api";
import App from "./App";
import type {
  ArchiveClosedEvent,
  DashboardSnapshot,
  RunSession,
  RunStatusEvent,
} from "./types";

describe("RunCove run history and conflict navigation", () => {
  beforeEach(async () => {
    resetMockApi();
    window.localStorage.setItem("runcove.language", "en");
    await api.setLanguagePreference("en");
    vi.spyOn(api, "scanSavedDevelopmentRoot").mockResolvedValue([]);
  });

  afterEach(() => vi.restoreAllMocks());

  it("loads history once on Overview and not for two-second port snapshots", async () => {
    const baseline = await api.getDashboardSnapshot();
    const getHistory = vi.spyOn(api, "getRunHistory");
    let pushSnapshot: ((snapshot: DashboardSnapshot) => void) | undefined;
    vi.spyOn(api, "onPortSnapshot").mockImplementation(async (handler) => {
      pushSnapshot = handler;
      return () => undefined;
    });

    render(<App />);

    expect(await screen.findByRole("heading", { name: "Recent runs" })).toBeVisible();
    await waitFor(() => expect(getHistory).toHaveBeenCalledTimes(1));
    act(() => pushSnapshot?.({ ...baseline, generatedAt: baseline.generatedAt + 2_000 }));
    await waitFor(() => expect(getHistory).toHaveBeenCalledTimes(1));

    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Refresh snapshot" }));
    await waitFor(() => expect(getHistory).toHaveBeenCalledTimes(2));
  });

  it("opens history, preserves orphaned sessions, and locates an existing project", async () => {
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Recent runs" });
    await user.click(screen.getByRole("button", { name: "View all" }));
    const drawer = screen.getByRole("dialog", { name: "Recent runs" });
    expect(within(drawer).getByText("Project deleted", { exact: true })).toBeVisible();
    expect(within(drawer).getByRole("button", { name: "View Project deleted / Removed service in Projects" })).toBeDisabled();

    await user.click(within(drawer).getByRole("button", { name: "View Abyss Studio / Web in Projects" }));
    expect(screen.getByRole("heading", { name: "Projects", level: 1 })).toBeVisible();
    expect(document.getElementById("project-section-project-studio")).toHaveClass("project-section--focused");
  });

  it("refreshes and focuses a structured conflicting port", async () => {
    let emitRunStatus: ((event: RunStatusEvent) => void) | undefined;
    vi.spyOn(api, "onRunStatus").mockImplementation(async (handler) => {
      emitRunStatus = handler;
      return () => undefined;
    });
    const user = userEvent.setup();
    render(<App />);

    await waitFor(() => expect(emitRunStatus).toBeTypeOf("function"));
    act(() => emitRunStatus?.({
      profileId: "profile-studio-web",
      status: "conflict",
      pid: null,
      message: "Expected port 5173 is occupied",
      relatedPort: { port: 5173, protocol: "tcp" },
      timestamp: Date.now(),
    }));

    await user.click(await screen.findByRole("button", { name: "View occupant" }));
    expect(screen.getByRole("heading", { name: "Ports", level: 1 })).toBeVisible();
    expect(screen.getByPlaceholderText("Search ports")).toHaveValue("5173 tcp");
    expect(screen.getByRole("region", { name: "Port 5173 details" })).toBeVisible();
  });

  it("reports when a structured conflict disappears before navigation", async () => {
    const baseline = await api.getDashboardSnapshot();
    let emitRunStatus: ((event: RunStatusEvent) => void) | undefined;
    vi.spyOn(api, "onRunStatus").mockImplementation(async (handler) => {
      emitRunStatus = handler;
      return () => undefined;
    });
    const getSnapshot = vi.spyOn(api, "getDashboardSnapshot");
    const user = userEvent.setup();
    render(<App />);

    await waitFor(() => expect(emitRunStatus).toBeTypeOf("function"));
    act(() => emitRunStatus?.({
      profileId: "profile-studio-web",
      status: "conflict",
      message: "Expected port 5173 is occupied",
      relatedPort: { port: 5173, protocol: "tcp" },
      timestamp: Date.now(),
    }));
    getSnapshot.mockResolvedValueOnce({
      ...baseline,
      ports: baseline.ports.filter((port) => port.port !== 5173),
    });

    await user.click(await screen.findByRole("button", { name: "View occupant" }));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Port 5173 is no longer occupied. The snapshot was refreshed.",
    );
    expect(screen.getByRole("heading", { name: "Overview", level: 1 })).toBeVisible();
  });

  // The exit event's own reload sees the row while it is still `writing`, because the
  // writer closes the file after the lock that event is emitted under is released. So
  // a badge that has settled on disk is only correct on screen if the close is
  // announced too.
  it("settles a finalizing archive badge when the writer reports the close", async () => {
    const startedAt = Date.parse("2026-08-07T08:12:24.000Z");
    const endedAt = Date.parse("2026-08-07T08:14:00.000Z");
    const finalizing: RunSession = {
      id: "session-web-closing",
      profileId: "profile-studio-web",
      profileName: "Web",
      pid: 18424,
      startedAt,
      endedAt,
      exitCode: 0,
      status: "exited",
      archive: {
        status: "writing",
        reason: null,
        lineCount: 0,
        byteSize: 0,
        droppedLines: 0,
        droppedBytes: 0,
        startedAt,
        endedAt: null,
      },
    };
    const settled: RunSession = {
      ...finalizing,
      archive: {
        ...finalizing.archive!,
        status: "complete",
        lineCount: 1_208,
        byteSize: 79_632,
        endedAt,
      },
    };
    const getHistory = vi.spyOn(api, "getRunHistory")
      .mockResolvedValueOnce([finalizing])
      .mockResolvedValue([settled]);
    let emitArchiveClosed: ((event: ArchiveClosedEvent) => void) | undefined;
    vi.spyOn(api, "onArchiveClosed").mockImplementation(async (handler) => {
      emitArchiveClosed = handler;
      return () => undefined;
    });

    render(<App />);

    expect(await screen.findByText("Finalizing")).toBeVisible();
    await waitFor(() => expect(emitArchiveClosed).toBeTypeOf("function"));
    act(() => emitArchiveClosed?.({ sessionId: "session-web-closing" }));

    expect(await screen.findByText("1,208 lines · 78 KiB")).toBeVisible();
    expect(screen.queryByText("Finalizing")).toBeNull();
    expect(getHistory).toHaveBeenCalledTimes(2);
  });

  // Turning the setting off closes the archives that are open, and the command has
  // finished them by the time it resolves. Nothing announces those rows —
  // `run-archive-closed` is emitted from the process exit path only — so without a
  // reload here the badge keeps claiming `Archiving` for a file that is closed.
  it("settles the badge of an archive the setting closed mid-run", async () => {
    const startedAt = Date.parse("2026-08-07T09:02:11.000Z");
    const writing: RunSession = {
      id: "session-web-live",
      profileId: "profile-studio-web",
      profileName: "Web",
      pid: 32276,
      startedAt,
      endedAt: null,
      exitCode: null,
      status: "running",
      archive: {
        status: "writing",
        reason: null,
        lineCount: 0,
        byteSize: 0,
        droppedLines: 0,
        droppedBytes: 0,
        startedAt,
        endedAt: null,
      },
    };
    const closed: RunSession = {
      ...writing,
      archive: {
        ...writing.archive!,
        status: "partial",
        reason: "user-disabled",
        lineCount: 717,
        byteSize: 47_320,
        endedAt: startedAt + 92_000,
      },
    };
    const baseline = await api.getDashboardSnapshot();
    vi.spyOn(api, "getDashboardSnapshot").mockResolvedValue({
      ...baseline,
      runLogArchive: { enabled: true, available: true, unavailableReason: null },
      settings: { ...baseline.settings, archiveRunLogs: true },
    });
    const getHistory = vi.spyOn(api, "getRunHistory")
      .mockResolvedValueOnce([writing])
      .mockResolvedValue([closed]);
    const setArchiving = vi.spyOn(api, "setRunLogArchiving")
      .mockResolvedValue({ enabled: false, available: true, unavailableReason: null });
    const user = userEvent.setup();

    render(<App />);

    expect(await screen.findByText("Archiving")).toBeVisible();
    await user.click(await screen.findByRole("button", { name: "View Web logs" }));
    await user.click(screen.getByRole("checkbox", { name: "Archive run logs" }));

    expect(await screen.findByText("Partial · archiving was turned off")).toBeVisible();
    expect(screen.queryByText("Archiving")).toBeNull();
    expect(setArchiving).toHaveBeenCalledWith(false);
    expect(getHistory).toHaveBeenCalledTimes(2);
  });
});
