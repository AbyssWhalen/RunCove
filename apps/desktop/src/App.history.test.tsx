import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, resetMockApi } from "./api";
import App from "./App";
import type { DashboardSnapshot, RunStatusEvent } from "./types";

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
});
