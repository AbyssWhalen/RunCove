import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { Project, RunLogArchiveSummary, RunSession } from "../types";
import { archiveBadgeState } from "./archive";
import { RunHistoryDrawer } from "./RunHistoryDrawer";
import {
  formatRunDuration,
  normalizeRunSessionStatus,
  prepareRunHistory,
} from "./run-history";
import { type RunHistoryLabels, RunHistorySection } from "./RunHistorySection";

vi.mock("../i18n", () => ({
  useI18n: () => ({
    locale: "en-US",
    formatDateTime: (value: number) => `date:${value}`,
  }),
}));

const labels: RunHistoryLabels = {
  recentTitle: "Recent runs",
  recentDescription: "Recent managed sessions",
  viewAll: "View all",
  drawerTitle: "Run history",
  drawerDescription: "Up to 200 sessions",
  close: "Close history",
  searchPlaceholder: "Search project or profile",
  clearSearch: "Clear search",
  filterLabel: "Filter sessions",
  filters: {
    all: "All",
    active: "Active",
    exited: "Exited",
    interrupted: "Interrupted",
  },
  project: "Project",
  profile: "Profile",
  status: "Status",
  pid: "PID",
  startedAt: "Started",
  endedAt: "Ended",
  duration: "Duration",
  exitCode: "Exit code",
  archive: "Archive",
  actions: "Actions",
  statusLabels: {
    starting: "Starting",
    running: "Running",
    exited: "Exited",
    interrupted: "Interrupted",
    unknown: "Unknown",
  },
  projectDeleted: "Project deleted",
  locate: (project, profile) => `Locate ${project} ${profile}`,
  // The real badge copy is pinned in archive.test.ts; these labels keep the state
  // derivation honest while giving this file stable strings to query by.
  archiveBadge: (item) => {
    const state = archiveBadgeState(item);
    return { state, text: `archive:${state}` };
  },
  archiveView: (profile) => `view:${profile}`,
  archiveDelete: (profile) => `delete:${profile}`,
  loading: "Loading history",
  empty: "No run history",
  noMatches: "No matching sessions",
  unavailable: "Unavailable",
  retry: "Retry",
  sessionCount: (count) => `${count} sessions`,
  resultCount: (visible, total) => `${visible} of ${total}`,
};

const project: Project = {
  id: "project-web",
  name: "Web",
  path: "C:\\work\\web",
  createdAt: 1,
  updatedAt: 1,
  profiles: [{
    id: "profile-dev",
    projectId: "project-web",
    name: "Dev",
    program: "npm.cmd",
    args: ["run", "dev"],
    cwd: "C:\\work\\web",
    expectedPorts: [],
    status: "idle",
  }],
};

function session(
  id: string,
  overrides: Partial<RunSession> = {},
): RunSession {
  return {
    id,
    profileId: "profile-dev",
    profileName: "Dev",
    pid: 42,
    startedAt: Number(id.replace(/\D/g, "")) || 1,
    endedAt: 2_000,
    exitCode: 0,
    status: "exited",
    ...overrides,
  };
}

function archiveSummary(
  overrides: Partial<RunLogArchiveSummary> = {},
): RunLogArchiveSummary {
  return {
    status: "complete",
    reason: null,
    lineCount: 12,
    byteSize: 3_072,
    droppedLines: 0,
    droppedBytes: 0,
    startedAt: 1,
    endedAt: 2_000,
    ...overrides,
  };
}

/** A session whose profile is gone, so its own `profileName` reaches the buttons. */
function orphan(id: string, profileName: string, overrides: Partial<RunSession> = {}): RunSession {
  return session(id, { profileId: "deleted-profile", profileName, ...overrides });
}

describe("run history helpers", () => {
  it("falls back to unknown for unrecognized persisted statuses", () => {
    expect(normalizeRunSessionStatus("future-status")).toBe("unknown");
    const unknown = session("1", { status: "future-status" as RunSession["status"] });
    expect(prepareRunHistory([unknown], [project])[0].status).toBe("unknown");
  });

  it("sorts, searches project and profile names, and applies status filters", () => {
    const entries = [
      session("1", { startedAt: 100, status: "exited" }),
      session("2", { startedAt: 300, status: "running", endedAt: null }),
      session("3", { startedAt: 200, status: "interrupted" }),
    ];

    expect(prepareRunHistory(entries, [project]).map(({ session: item }) => item.id)).toEqual(["2", "3", "1"]);
    expect(prepareRunHistory(entries, [project], "web")).toHaveLength(3);
    expect(prepareRunHistory(entries, [project], "DEV", "active").map(({ session: item }) => item.id)).toEqual(["2"]);
    expect(prepareRunHistory(entries, [project], "", "interrupted").map(({ session: item }) => item.id)).toEqual(["3"]);
  });

  it("formats ended and active durations without negative values", () => {
    expect(formatRunDuration({ startedAt: 0, endedAt: 65_000 }, "en-US")).toBe("1m 5s");
    expect(formatRunDuration({ startedAt: 2_000, endedAt: 1_000 }, "en-US")).toBe("0s");
    expect(formatRunDuration({ startedAt: 1_000, endedAt: null }, "en-US", 4_000)).toBe("3s");
  });
});

describe("RunHistorySection", () => {
  it("shows only the five most recent sessions and locates a current profile", async () => {
    const user = userEvent.setup();
    const onLocate = vi.fn();
    render(
      <RunHistorySection
        sessions={Array.from({ length: 7 }, (_, index) => session(String(index + 1)))}
        projects={[project]}
        loading={false}
        labels={labels}
        onRetry={vi.fn()}
        onLocate={onLocate}
        onViewArchive={vi.fn()}
        onDeleteArchive={vi.fn()}
        onOpenAll={vi.fn()}
      />,
    );

    expect(screen.getAllByRole("row")).toHaveLength(6);
    await user.click(screen.getAllByRole("button", { name: "Locate Web Dev" })[0]);
    expect(onLocate).toHaveBeenCalledWith("project-web", "profile-dev");
  });

  it("keeps orphan sessions visible and disables project location", () => {
    render(
      <RunHistorySection
        sessions={[session("orphan", { profileId: "deleted-profile", profileName: "Old dev" })]}
        projects={[project]}
        loading={false}
        labels={labels}
        onRetry={vi.fn()}
        onLocate={vi.fn()}
        onViewArchive={vi.fn()}
        onDeleteArchive={vi.fn()}
        onOpenAll={vi.fn()}
      />,
    );

    expect(screen.getByText("Project deleted")).toBeVisible();
    expect(screen.getByText("Old dev")).toBeVisible();
    expect(screen.getByRole("button", { name: "Locate Project deleted Old dev" })).toBeDisabled();
  });

  it("offers a finished archive, refuses an open one, and says when there is none", async () => {
    const user = userEvent.setup();
    const onViewArchive = vi.fn();
    const onDeleteArchive = vi.fn();
    const finished = orphan("1", "Finished", { archive: archiveSummary() });
    const open = orphan("2", "Open", {
      status: "running",
      endedAt: null,
      exitCode: null,
      archive: archiveSummary({ status: "writing", endedAt: null }),
    });
    const removed = orphan("3", "Removed", {
      archive: archiveSummary({ status: "removed", reason: "user-deleted" }),
    });
    const plain = orphan("4", "Plain");
    render(
      <RunHistorySection
        sessions={[finished, open, removed, plain]}
        projects={[project]}
        loading={false}
        labels={labels}
        onRetry={vi.fn()}
        onLocate={vi.fn()}
        onViewArchive={onViewArchive}
        onDeleteArchive={onDeleteArchive}
        onOpenAll={vi.fn()}
      />,
    );

    expect(screen.getByRole("columnheader", { name: "Archive" })).toBeVisible();
    for (const state of ["complete", "writing", "removed", "none"]) {
      expect(screen.getByText(`archive:${state}`)).toBeVisible();
    }
    // An open file can be read but not deleted, a removed one offers neither, and a
    // session that was never archived has nothing to offer at all.
    expect(screen.getByRole("button", { name: "view:Open" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "delete:Open" })).toBeDisabled();
    for (const name of ["view:Removed", "delete:Removed", "view:Plain", "delete:Plain"]) {
      expect(screen.getByRole("button", { name })).toBeDisabled();
    }

    await user.click(screen.getByRole("button", { name: "view:Finished" }));
    await user.click(screen.getByRole("button", { name: "delete:Finished" }));
    expect(onViewArchive).toHaveBeenCalledWith(finished);
    expect(onDeleteArchive).toHaveBeenCalledWith(finished);
  });

  it("renders a retryable failure instead of stale rows", async () => {
    const user = userEvent.setup();
    const onRetry = vi.fn();
    render(
      <RunHistorySection
        sessions={[session("1")]}
        projects={[project]}
        loading={false}
        error="History unavailable"
        labels={labels}
        onRetry={onRetry}
        onLocate={vi.fn()}
        onViewArchive={vi.fn()}
        onDeleteArchive={vi.fn()}
        onOpenAll={vi.fn()}
      />,
    );

    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });
});

describe("RunHistoryDrawer", () => {
  it("caps the list at 200 sessions and supports search and status filtering", async () => {
    const user = userEvent.setup();
    const sessions = Array.from({ length: 205 }, (_, index) => session(String(index + 1), {
      status: index === 204 ? "running" : "exited",
      endedAt: index === 204 ? null : 2_000,
    }));
    render(
      <RunHistoryDrawer
        sessions={sessions}
        projects={[project]}
        loading={false}
        labels={labels}
        onRetry={vi.fn()}
        onLocate={vi.fn()}
        onViewArchive={vi.fn()}
        onDeleteArchive={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    const dialog = screen.getByRole("dialog", { name: "Run history" });
    expect(within(dialog).getByText("200 of 200")).toBeVisible();
    expect(within(dialog).getAllByRole("row")).toHaveLength(201);

    await user.click(within(dialog).getByRole("button", { name: "Active" }));
    expect(within(dialog).getByText("1 of 200")).toBeVisible();
    expect(within(dialog).getAllByRole("row")).toHaveLength(2);

    const search = within(dialog).getByRole("searchbox", { name: "Search project or profile" });
    await user.type(search, "missing");
    expect(within(dialog).getByText("No matching sessions")).toBeVisible();
  });

  it("forwards the archive actions of the rows it renders", async () => {
    const user = userEvent.setup();
    const onViewArchive = vi.fn();
    const onDeleteArchive = vi.fn();
    const archived = orphan("1", "Archived", { archive: archiveSummary() });
    render(
      <RunHistoryDrawer
        sessions={[archived]}
        projects={[project]}
        loading={false}
        labels={labels}
        onRetry={vi.fn()}
        onLocate={vi.fn()}
        onViewArchive={onViewArchive}
        onDeleteArchive={onDeleteArchive}
        onClose={vi.fn()}
      />,
    );

    const dialog = screen.getByRole("dialog", { name: "Run history" });
    expect(within(dialog).getByText("archive:complete")).toBeVisible();
    await user.click(within(dialog).getByRole("button", { name: "view:Archived" }));
    await user.click(within(dialog).getByRole("button", { name: "delete:Archived" }));
    expect(onViewArchive).toHaveBeenCalledWith(archived);
    expect(onDeleteArchive).toHaveBeenCalledWith(archived);
  });

  it("closes on Escape and restores focus to the opening control", () => {
    const trigger = document.createElement("button");
    trigger.textContent = "Open history";
    document.body.append(trigger);
    trigger.focus();
    const onClose = vi.fn();
    const view = render(
      <RunHistoryDrawer
        sessions={[]}
        projects={[]}
        loading={false}
        labels={labels}
        onRetry={vi.fn()}
        onLocate={vi.fn()}
        onViewArchive={vi.fn()}
        onDeleteArchive={vi.fn()}
        onClose={onClose}
      />,
    );

    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
    view.unmount();
    expect(trigger).toHaveFocus();
    trigger.remove();
  });
});
