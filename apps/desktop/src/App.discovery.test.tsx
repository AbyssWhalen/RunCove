import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, resetMockApi } from "./api";
import App from "./App";
import type { DiscoveredProject } from "./types";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

function discoveredProject(name: string, path: string): DiscoveredProject {
  return {
    name,
    path,
    packageManager: "npm",
    workspacePatterns: [],
    profiles: [{
      name: "Dev",
      program: "npm.cmd",
      args: ["run", "dev"],
      cwd: path,
      expectedPorts: [],
    }],
  };
}

describe("RunCove automatic project discovery", () => {
  beforeEach(async () => {
    resetMockApi();
    window.localStorage.setItem("runcove.language", "en");
    await api.setLanguagePreference("en");
  });

  afterEach(() => vi.restoreAllMocks());

  it("rescans the saved root on startup without registering candidates", async () => {
    const candidate = discoveredProject(
      "Signal Console",
      "D:\\CodexProject\\personal-projects\\signal-console",
    );
    const scanSavedRoot = vi.spyOn(api, "scanSavedDevelopmentRoot").mockResolvedValue([candidate]);
    const saveProject = vi.spyOn(api, "saveProject");

    render(<App />);

    expect(await screen.findByRole("status")).toHaveTextContent(
      "Found 1 new service project. Review it from Projects.",
    );
    await waitFor(() => expect(scanSavedRoot).toHaveBeenCalledTimes(1));
    expect(saveProject).not.toHaveBeenCalled();

    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(screen.getByRole("button", { name: "Review 1 discovered" }));

    expect(screen.getByRole("checkbox", { name: "Select Signal Console" })).toBeChecked();
    expect(screen.getByRole("button", { name: "Development root" })).toHaveAttribute("aria-pressed", "true");
    expect(saveProject).not.toHaveBeenCalled();
  });

  it("filters registered paths case-insensitively and ignores trailing separators", async () => {
    const duplicate = discoveredProject(
      "Duplicate Studio",
      "D:\\CODEXPROJECT\\PERSONAL-PROJECTS\\ABYSS-STUDIO\\",
    );
    const candidate = discoveredProject(
      "Worker Lab",
      "D:\\CodexProject\\personal-projects\\worker-lab",
    );
    vi.spyOn(api, "scanSavedDevelopmentRoot").mockResolvedValue([duplicate, candidate]);
    const user = userEvent.setup();

    render(<App />);

    await screen.findByText("Found 1 new service project. Review it from Projects.");
    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(screen.getByRole("button", { name: "Review 1 discovered" }));

    expect(screen.getByRole("checkbox", { name: "Select Worker Lab" })).toBeChecked();
    expect(screen.queryByRole("checkbox", { name: "Select Duplicate Studio" })).not.toBeInTheDocument();
    expect(screen.queryByText("Duplicate Studio")).not.toBeInTheDocument();
  });

  it("opens development-root selection when no root has been saved", async () => {
    const baseline = await api.getDashboardSnapshot();
    vi.spyOn(api, "getDashboardSnapshot").mockResolvedValue({
      ...baseline,
      settings: {
        ...baseline.settings,
        recentDevelopmentRoot: null,
      },
    });
    const scanSavedRoot = vi.spyOn(api, "scanSavedDevelopmentRoot");
    const user = userEvent.setup();

    render(<App />);
    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(screen.getByRole("button", { name: "Auto discover" }));

    expect(screen.getByRole("button", { name: "Development root" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByLabelText("Development root")).toHaveValue("");
    expect(screen.getByRole("status")).toHaveTextContent(
      "Choose a development root once to enable automatic project discovery.",
    );
    expect(scanSavedRoot).not.toHaveBeenCalled();
  });

  it("reports an empty manual rescan of the saved root", async () => {
    const scanSavedRoot = vi.spyOn(api, "scanSavedDevelopmentRoot").mockResolvedValue([]);
    const user = userEvent.setup();

    render(<App />);
    await waitFor(() => expect(scanSavedRoot).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(screen.getByRole("button", { name: "Rescan saved root" }));

    await waitFor(() => expect(scanSavedRoot).toHaveBeenCalledTimes(2));
    expect(screen.getByRole("status")).toHaveTextContent(
      "No new service projects were found in your saved development root.",
    );
  });

  it("shows a saved-root failure and coalesces retry clicks", async () => {
    const retry = deferred<DiscoveredProject[]>();
    const scanSavedRoot = vi.spyOn(api, "scanSavedDevelopmentRoot")
      .mockRejectedValueOnce(new Error("root is unavailable"))
      .mockReturnValueOnce(retry.promise);
    const user = userEvent.setup();

    render(<App />);
    await waitFor(() => expect(scanSavedRoot).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("button", { name: "Projects" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("root is unavailable");

    const retryButton = screen.getByRole("button", { name: "Retry" });
    await user.click(retryButton);
    retryButton.click();
    expect(scanSavedRoot).toHaveBeenCalledTimes(2);

    retry.resolve([]);
    expect(await screen.findByText("The saved development root is up to date.")).toBeVisible();
  });
});
