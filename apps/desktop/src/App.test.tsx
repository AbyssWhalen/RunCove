import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, resetMockApi } from "./api";
import App from "./App";
import type { DashboardSnapshot, RunStatusEvent } from "./types";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

describe("RunCove desktop shell", () => {
  beforeEach(async () => {
    resetMockApi();
    window.localStorage.setItem("runcove.language", "en");
    await api.setLanguagePreference("en");
    vi.spyOn(api, "scanSavedDevelopmentRoot").mockResolvedValue([]);
  });

  afterEach(() => vi.restoreAllMocks());

  it("switches to Chinese and restores the saved language", async () => {
    const user = userEvent.setup();
    const firstRender = render(<App />);

    const language = await screen.findByRole("combobox", { name: "Language" });
    await user.selectOptions(language, "zh-CN");

    expect(screen.getByRole("button", { name: "概览" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "启动配置" })).toBeInTheDocument();
    expect(window.localStorage.getItem("runcove.language")).toBe("zh-CN");
    expect(document.documentElement.lang).toBe("zh-CN");

    firstRender.unmount();
    render(<App />);
    expect(await screen.findByRole("button", { name: "概览" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "语言" })).toHaveValue("zh-CN");
  });

  it.each([
    ["an unsupported value", "fr"],
    ["a blank value", "   "],
    ["a missing value", null],
  ])("uses the backend language when local storage contains %s", async (_case, storedPreference) => {
    await api.setLanguagePreference("zh-CN");
    if (storedPreference === null) {
      window.localStorage.removeItem("runcove.language");
    } else {
      window.localStorage.setItem("runcove.language", storedPreference);
    }
    const saveLanguage = vi.spyOn(api, "setLanguagePreference");

    render(<App />);

    expect(await screen.findByRole("button", { name: "概览" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "语言" })).toHaveValue("zh-CN");
    expect(window.localStorage.getItem("runcove.language")).toBe("zh-CN");
    expect(saveLanguage).not.toHaveBeenCalled();
  });

  it("shows runtime state and filters active and historical ports", async () => {
    const user = userEvent.setup();
    render(<App />);

    expect(await screen.findByRole("heading", { name: "Launch profiles" })).toBeInTheDocument();
    expect(screen.getAllByText("Abyss Studio").length).toBeGreaterThan(0);

    await user.click(screen.getByRole("button", { name: "Ports" }));
    expect(await screen.findByRole("heading", { name: "Ports", level: 2 })).toBeInTheDocument();
    expect(screen.getByText("5173")).toBeInTheDocument();
    expect(screen.getByText("8787")).toBeInTheDocument();

    const suggestedRow = screen.getByText("4000").closest("tr");
    expect(suggestedRow).not.toBeNull();
    await user.click(within(suggestedRow!).getByRole("button", { name: "Confirm project association for port 4000" }));
    const confirmedPort = await screen.findByText("4000");
    expect(within(confirmedPort.closest("tr")!).getByText("Confirmed")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Historical" }));
    expect(screen.queryByText("5173")).not.toBeInTheDocument();
    expect(screen.getByText("8787")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Start profile for historical port 8787" }));
    expect(await screen.findByText("Start completed")).toBeInTheDocument();
  });

  it("hides dangerous port actions when verified process identity is incomplete", async () => {
    const baseline = await api.getDashboardSnapshot();
    vi.spyOn(api, "getDashboardSnapshot").mockResolvedValue({
      ...baseline,
      ports: baseline.ports.map((port) => {
        if (port.port === 4000) return { ...port, executablePath: null };
        if (port.port === 5432) return { ...port, processStartedAt: null };
        return port;
      }),
    });
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Ports" }));

    const suggestedRow = screen.getByText("4000").closest("tr");
    const externalRow = screen.getByText("5432").closest("tr");
    expect(suggestedRow).not.toBeNull();
    expect(externalRow).not.toBeNull();
    expect(within(suggestedRow!).queryByRole("button", { name: "Confirm project association for port 4000" })).not.toBeInTheDocument();
    expect(within(suggestedRow!).queryByRole("button", { name: "Terminate process 9208 on port 4000" })).not.toBeInTheDocument();
    expect(within(externalRow!).queryByRole("button", { name: "Terminate process 6612 on port 5432" })).not.toBeInTheDocument();
  });

  it("sends the complete observed identity when confirming a port association", async () => {
    const confirmAssociation = vi.spyOn(api, "confirmPortAssociation");
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Ports" }));
    const suggestedRow = screen.getByText("4000").closest("tr");
    await user.click(within(suggestedRow!).getByRole("button", { name: "Confirm project association for port 4000" }));

    expect(confirmAssociation).toHaveBeenCalledWith({
      port: 4000,
      protocol: "tcp",
      projectId: "project-studio",
      profileId: null,
      pid: 9208,
      startedAt: Date.parse("2026-08-07T07:52:03.000Z"),
      executablePath: "C:\\Program Files\\nodejs\\node.exe",
    });
  });

  it("sends the port tuple with a verified external termination request", async () => {
    const terminate = vi.spyOn(api, "terminateExternalProcess");
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Ports" }));
    const externalRow = screen.getByText("5432").closest("tr");
    await user.click(within(externalRow!).getByRole("button", { name: "Terminate process 6612 on port 5432" }));
    await user.click(screen.getByRole("button", { name: "Terminate process tree" }));

    expect(terminate).toHaveBeenCalledWith({
      port: 5432,
      protocol: "tcp",
      pid: 6612,
      startedAt: Date.parse("2026-08-07T00:02:18.000Z"),
      executablePath: "C:\\Program Files\\PostgreSQL\\bin\\postgres.exe",
    });
  });

  it("keeps a historical port start disabled while its profile action is pending", async () => {
    const pending = deferred<RunStatusEvent>();
    const startProfile = vi.spyOn(api, "startProfile").mockReturnValue(pending.promise);
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Ports" }));
    await user.click(screen.getByRole("button", { name: "Historical" }));
    const start = screen.getByRole("button", { name: "Start profile for historical port 8787" });
    await user.click(start);

    expect(start).toBeDisabled();
    expect(startProfile).toHaveBeenCalledTimes(1);

    pending.resolve({
      profileId: "profile-docs",
      status: "starting",
      pid: 4400,
      timestamp: Date.now(),
    });
    await waitFor(() => expect(startProfile).toHaveBeenCalledTimes(1));
  });

  it("keeps partial restore details visible after refreshing the dashboard", async () => {
    const restore = vi.spyOn(api, "restoreLastRunSet").mockResolvedValue({
      startedProfileIds: ["profile-studio-web"],
      failedProfileId: "profile-docs",
      error: "Expected port 4321 is already occupied",
    });
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Restore previous run" }));
    await waitFor(() => expect(restore).toHaveBeenCalledTimes(1));

    expect(await screen.findByRole("alert")).toHaveTextContent("Expected port 4321 is already occupied");
  });

  it("shows the saved restore order with project and profile names", async () => {
    render(<App />);

    const heading = await screen.findByRole("heading", { name: "Previously running profiles" });
    const restoreBand = heading.closest("section");
    expect(restoreBand).not.toBeNull();
    const order = within(restoreBand!).getByRole("list", { name: "Startup order" });

    expect(within(order).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
      "Abyss Studio / Web",
      "Docs Lab / Astro",
    ]);
  });

  it("exposes direct project deletion while protecting running projects", async () => {
    const removeProject = vi.spyOn(api, "deleteProject");
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Projects" }));

    expect(screen.getByRole("button", {
      name: "Stop every running profile in Abyss Studio before deleting it",
    })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Delete Docs Lab" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Delete project" });
    expect(dialog).toHaveTextContent("files in the project folder will not be deleted");
    await user.click(within(dialog).getByRole("button", { name: "Delete project" }));

    await waitFor(() => expect(removeProject).toHaveBeenCalledWith("project-docs"));
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Docs Lab" })).not.toBeInTheDocument());
    expect(screen.getByRole("status")).toHaveTextContent("Project deleted");
  });

  it("keeps the project deletion dialog open when deletion fails", async () => {
    vi.spyOn(api, "deleteProject").mockRejectedValue(new Error("database is busy"));
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(screen.getByRole("button", { name: "Delete Docs Lab" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Delete project" });
    await user.click(within(dialog).getByRole("button", { name: "Delete project" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("database is busy");
    expect(screen.getByRole("alertdialog", { name: "Delete project" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "Docs Lab" })).toBeInTheDocument();
  });

  it("submits project deletion only once when confirmation is repeated quickly", async () => {
    const pending = deferred<void>();
    const removeProject = vi.spyOn(api, "deleteProject").mockReturnValue(pending.promise);
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(screen.getByRole("button", { name: "Delete Docs Lab" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Delete project" });
    const confirm = within(dialog).getByRole("button", { name: "Delete project" });

    act(() => {
      confirm.click();
      confirm.click();
    });

    expect(removeProject).toHaveBeenCalledTimes(1);
    pending.resolve();
    await waitFor(() => expect(screen.queryByRole("alertdialog", { name: "Delete project" })).not.toBeInTheDocument());
  });

  it("keeps only one modal active when tray quit arrives over project import", async () => {
    let requestQuit: (() => void) | undefined;
    vi.spyOn(api, "onTrayQuitRequested").mockImplementation(async (handler) => {
      requestQuit = handler;
      return () => {};
    });
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(screen.getByRole("button", { name: "Import project" }));
    expect(screen.getAllByRole("dialog")).toHaveLength(1);

    act(() => requestQuit?.());

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getAllByRole("alertdialog")).toHaveLength(1);
    expect(screen.getByRole("heading", { name: "Quit RunCove?" })).toBeInTheDocument();
  });

  it("removes duplicate toolbar window actions and opens one close choice over existing work", async () => {
    let requestClose: (() => void) | undefined;
    const subscribe = vi.spyOn(api, "onWindowCloseChoiceRequested").mockImplementation(async (handler) => {
      requestClose = handler;
      return () => {};
    });
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await waitFor(() => expect(subscribe).toHaveBeenCalledOnce());
    expect(screen.queryByRole("button", { name: "Hide to system tray" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Quit RunCove" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(screen.getByRole("button", { name: "Import project" }));
    expect(screen.getByRole("dialog", { name: "Import project" })).toBeVisible();

    act(() => requestClose?.());
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(screen.getByRole("dialog", { name: "When closing the window" })).toBeVisible();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "When closing the window" })).not.toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Import project" })).toBeVisible();
  });

  it("hides through native IPC without saving when remember is unchecked", async () => {
    let requestClose: (() => void) | undefined;
    vi.spyOn(api, "onWindowCloseChoiceRequested").mockImplementation(async (handler) => {
      requestClose = handler;
      return () => {};
    });
    const savePreference = vi.spyOn(api, "setCloseBehavior");
    const hide = vi.spyOn(api, "hideToTray");
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await waitFor(() => expect(requestClose).toBeTypeOf("function"));
    act(() => requestClose?.());
    await user.click(screen.getByRole("button", { name: /Hide to system tray/ }));

    await waitFor(() => expect(hide).toHaveBeenCalledOnce());
    expect(savePreference).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog", { name: "When closing the window" })).not.toBeInTheDocument();
  });

  it("persists a remembered quit before using the safe shutdown path", async () => {
    let requestClose: (() => void) | undefined;
    vi.spyOn(api, "onWindowCloseChoiceRequested").mockImplementation(async (handler) => {
      requestClose = handler;
      return () => {};
    });
    const settings = (await api.getDashboardSnapshot()).settings;
    const order: string[] = [];
    vi.spyOn(api, "setCloseBehavior").mockImplementation(async (behavior) => {
      order.push(`save:${behavior}`);
      return { ...settings, closeBehavior: behavior };
    });
    vi.spyOn(api, "shutdownApp").mockImplementation(async () => {
      order.push("shutdown");
    });
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await waitFor(() => expect(requestClose).toBeTypeOf("function"));
    act(() => requestClose?.());
    await user.click(screen.getByRole("checkbox", { name: "Remember this choice and don't ask again" }));
    await user.click(screen.getByRole("button", { name: /Quit RunCove/ }));

    await waitFor(() => expect(order).toEqual(["save:quit", "shutdown"]));
  });

  it("ignores native close and tray quit requests while tray shutdown is pending", async () => {
    const pendingShutdown = deferred<void>();
    const shutdown = vi.spyOn(api, "shutdownApp").mockReturnValue(pendingShutdown.promise);
    let requestQuit: (() => void) | undefined;
    let requestClose: (() => void) | undefined;
    vi.spyOn(api, "onTrayQuitRequested").mockImplementation(async (handler) => {
      requestQuit = handler;
      return () => {};
    });
    vi.spyOn(api, "onWindowCloseChoiceRequested").mockImplementation(async (handler) => {
      requestClose = handler;
      return () => {};
    });
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await waitFor(() => {
      expect(requestQuit).toBeTypeOf("function");
      expect(requestClose).toBeTypeOf("function");
    });
    act(() => requestQuit?.());
    const quitDialog = await screen.findByRole("alertdialog", { name: "Quit RunCove?" });
    await user.click(within(quitDialog).getByRole("button", { name: "Stop all and quit" }));
    await waitFor(() => expect(shutdown).toHaveBeenCalledOnce());

    act(() => {
      requestClose?.();
      requestQuit?.();
    });
    expect(screen.queryByRole("dialog", { name: "When closing the window" })).not.toBeInTheDocument();
    expect(screen.getByRole("alertdialog", { name: "Quit RunCove?" })).toHaveAttribute("aria-busy", "true");

    pendingShutdown.resolve();
    await waitFor(() => expect(screen.queryByRole("alertdialog", { name: "Quit RunCove?" })).not.toBeInTheDocument());
  });

  it("ignores new close requests while the title-bar quit action is pending", async () => {
    const pendingShutdown = deferred<void>();
    const shutdown = vi.spyOn(api, "shutdownApp").mockReturnValue(pendingShutdown.promise);
    let requestQuit: (() => void) | undefined;
    let requestClose: (() => void) | undefined;
    vi.spyOn(api, "onTrayQuitRequested").mockImplementation(async (handler) => {
      requestQuit = handler;
      return () => {};
    });
    vi.spyOn(api, "onWindowCloseChoiceRequested").mockImplementation(async (handler) => {
      requestClose = handler;
      return () => {};
    });
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await waitFor(() => {
      expect(requestQuit).toBeTypeOf("function");
      expect(requestClose).toBeTypeOf("function");
    });
    act(() => requestClose?.());
    const closeDialog = await screen.findByRole("dialog", { name: "When closing the window" });
    await user.click(within(closeDialog).getByRole("button", { name: /Quit RunCove/ }));
    await waitFor(() => expect(shutdown).toHaveBeenCalledOnce());

    act(() => {
      requestClose?.();
      requestQuit?.();
    });
    expect(screen.getByRole("dialog", { name: "When closing the window" })).toHaveAttribute("aria-busy", "true");
    expect(screen.queryByRole("alertdialog", { name: "Quit RunCove?" })).not.toBeInTheDocument();

    pendingShutdown.resolve();
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "When closing the window" })).not.toBeInTheDocument());
  });

  it("keeps the close choice open when a remembered preference cannot be saved", async () => {
    let requestClose: (() => void) | undefined;
    vi.spyOn(api, "onWindowCloseChoiceRequested").mockImplementation(async (handler) => {
      requestClose = handler;
      return () => {};
    });
    vi.spyOn(api, "setCloseBehavior").mockRejectedValue(new Error("settings are read-only"));
    const hide = vi.spyOn(api, "hideToTray");
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await waitFor(() => expect(requestClose).toBeTypeOf("function"));
    act(() => requestClose?.());
    await user.click(screen.getByRole("checkbox", { name: "Remember this choice and don't ask again" }));
    await user.click(screen.getByRole("button", { name: /Hide to system tray/ }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Window close behavior could not be saved: settings are read-only",
    );
    expect(hide).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "When closing the window" })).toBeVisible();
  });

  it("coalesces repeated tray restore requests while restore is running", async () => {
    const pending = deferred<{ startedProfileIds: string[] }>();
    const restore = vi.spyOn(api, "restoreLastRunSet").mockReturnValue(pending.promise);
    let requestRestore: (() => void) | undefined;
    const subscribe = vi.spyOn(api, "onTrayRestoreRequested").mockImplementation(async (handler) => {
      requestRestore = handler;
      return () => {};
    });
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await waitFor(() => expect(subscribe).toHaveBeenCalledTimes(1));
    act(() => {
      requestRestore?.();
      requestRestore?.();
    });

    expect(restore).toHaveBeenCalledTimes(1);
    pending.resolve({ startedProfileIds: [] });
    await waitFor(() => expect(screen.getByRole("button", { name: "Restore previous run" })).toBeEnabled());
  });

  it("disables profile actions while an ordered restore is running", async () => {
    const pending = deferred<{ startedProfileIds: string[] }>();
    vi.spyOn(api, "restoreLastRunSet").mockReturnValue(pending.promise);
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await user.click(screen.getByRole("button", { name: "Restore previous run" }));

    expect(screen.getByRole("button", { name: "Start Abyss Studio API" })).toBeDisabled();
    pending.resolve({ startedProfileIds: [] });
    await waitFor(() => expect(screen.getByRole("button", { name: "Restore previous run" })).toBeEnabled());
  });

  it("does not let an older initial load overwrite a newer pushed snapshot", async () => {
    const baseline = await api.getDashboardSnapshot();
    const initialLoad = deferred<DashboardSnapshot>();
    let pushSnapshot: ((snapshot: DashboardSnapshot) => void) | undefined;
    vi.spyOn(api, "getDashboardSnapshot").mockReturnValueOnce(initialLoad.promise);
    vi.spyOn(api, "onPortSnapshot").mockImplementation(async (handler) => {
      pushSnapshot = handler;
      return () => undefined;
    });

    render(<App />);
    await waitFor(() => expect(pushSnapshot).toBeTypeOf("function"));

    const newer = {
      ...baseline,
      generatedAt: baseline.generatedAt + 2,
      projects: baseline.projects.map((project, index) =>
        index === 0 ? { ...project, name: "Newest snapshot" } : project,
      ),
    };
    act(() => pushSnapshot?.(newer));
    expect(await screen.findAllByText("Newest snapshot")).not.toHaveLength(0);

    initialLoad.resolve({
      ...baseline,
      generatedAt: baseline.generatedAt + 1,
      projects: baseline.projects.map((project, index) =>
        index === 0 ? { ...project, name: "Stale snapshot" } : project,
      ),
    });
    await act(async () => {
      await initialLoad.promise;
    });

    expect(screen.queryByText("Stale snapshot")).not.toBeInTheDocument();
    expect(screen.getAllByText("Newest snapshot")).not.toHaveLength(0);
  });

  it("does not let an older pushed snapshot roll back a newer status event", async () => {
    const baseline = await api.getDashboardSnapshot();
    let pushSnapshot: ((snapshot: DashboardSnapshot) => void) | undefined;
    let emitRunStatus: ((event: RunStatusEvent) => void) | undefined;
    vi.spyOn(api, "onPortSnapshot").mockImplementation(async (handler) => {
      pushSnapshot = handler;
      return () => undefined;
    });
    vi.spyOn(api, "onRunStatus").mockImplementation(async (handler) => {
      emitRunStatus = handler;
      return () => undefined;
    });
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await waitFor(() => {
      expect(pushSnapshot).toBeTypeOf("function");
      expect(emitRunStatus).toBeTypeOf("function");
    });
    const event: RunStatusEvent = {
      profileId: "profile-studio-api",
      status: "starting",
      pid: 2400,
      timestamp: baseline.generatedAt + 100,
    };
    act(() => emitRunStatus?.(event));
    const apiRow = screen.getByText("API").closest("tr");
    expect(apiRow).not.toBeNull();
    expect(within(apiRow!).getByText("Starting")).toBeInTheDocument();

    act(() => pushSnapshot?.({ ...baseline, generatedAt: event.timestamp - 1 }));

    const refreshedApiRow = screen.getByText("API").closest("tr");
    expect(refreshedApiRow).not.toBeNull();
    expect(within(refreshedApiRow!).getByText("Starting")).toBeInTheDocument();
  });

  it("surfaces IPC subscription setup failures", async () => {
    vi.spyOn(api, "onLifecycleError").mockRejectedValue(new Error("event channel unavailable"));
    render(<App />);

    expect(await screen.findByRole("alert")).toHaveTextContent("event channel unavailable");
  });

  it("adds a localized conclusion to initial backend failures", async () => {
    window.localStorage.setItem("runcove.language", "zh-CN");
    vi.spyOn(api, "getDashboardSnapshot").mockRejectedValue(new Error("database unavailable"));

    render(<App />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "操作失败：database unavailable",
    );
  });

  it("adds a localized conclusion to lifecycle failures", async () => {
    window.localStorage.setItem("runcove.language", "zh-CN");
    let emitLifecycleError: ((message: string) => void) | undefined;
    vi.spyOn(api, "onLifecycleError").mockImplementation(async (handler) => {
      emitLifecycleError = handler;
      return () => undefined;
    });
    render(<App />);

    await screen.findByRole("heading", { name: "启动配置" });
    await waitFor(() => expect(emitLifecycleError).toBeTypeOf("function"));
    act(() => emitLifecycleError?.("process watcher failed"));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "运行状态异常：process watcher failed",
    );
  });

  it("stacks a lifecycle error without covering an active success notice", async () => {
    let emitRunStatus: ((event: RunStatusEvent) => void) | undefined;
    let emitLifecycleError: ((message: string) => void) | undefined;
    vi.spyOn(api, "onRunStatus").mockImplementation(async (handler) => {
      emitRunStatus = handler;
      return () => undefined;
    });
    vi.spyOn(api, "onLifecycleError").mockImplementation(async (handler) => {
      emitLifecycleError = handler;
      return () => undefined;
    });
    render(<App />);

    await screen.findByRole("heading", { name: "Launch profiles" });
    await waitFor(() => {
      expect(emitRunStatus).toBeTypeOf("function");
      expect(emitLifecycleError).toBeTypeOf("function");
    });
    act(() => emitRunStatus?.({
      profileId: "profile-studio-api",
      status: "running",
      pid: 2400,
      message: "API started",
      timestamp: Date.now(),
    }));
    expect(await screen.findByRole("status")).toHaveTextContent("API started");

    act(() => emitLifecycleError?.("process watcher failed"));

    const stack = document.querySelector(".toast-stack");
    expect(stack).not.toBeNull();
    expect(within(stack as HTMLElement).getByRole("alert")).toHaveTextContent("process watcher failed");
    expect(within(stack as HTMLElement).getByRole("status")).toHaveTextContent("API started");
  });

  it("does not reuse an old conflict action for a later unrelated error", async () => {
    let emitRunStatus: ((event: RunStatusEvent) => void) | undefined;
    let emitLifecycleError: ((message: string) => void) | undefined;
    vi.spyOn(api, "onRunStatus").mockImplementation(async (handler) => {
      emitRunStatus = handler;
      return () => undefined;
    });
    vi.spyOn(api, "onLifecycleError").mockImplementation(async (handler) => {
      emitLifecycleError = handler;
      return () => undefined;
    });
    render(<App />);

    await waitFor(() => {
      expect(emitRunStatus).toBeTypeOf("function");
      expect(emitLifecycleError).toBeTypeOf("function");
    });
    act(() => emitRunStatus?.({
      profileId: "profile-studio-api",
      status: "conflict",
      pid: null,
      message: "Expected port 5173 is occupied",
      relatedPort: { port: 5173, protocol: "tcp" },
      timestamp: Date.now(),
    }));
    expect(await screen.findByRole("button", { name: "View occupant" })).toBeInTheDocument();

    act(() => emitLifecycleError?.("process watcher failed"));

    expect(await screen.findByRole("alert")).toHaveTextContent("process watcher failed");
    expect(screen.queryByRole("button", { name: "View occupant" })).not.toBeInTheDocument();
  });

  it.each([
    { status: "conflict" as const, unexpected: false, message: "Expected port 4010 is occupied" },
    { status: "exited" as const, unexpected: true, message: "Process exited with code 7" },
  ])("shows $status failure events as errors", async (event) => {
    let emitRunStatus: ((event: RunStatusEvent) => void) | undefined;
    vi.spyOn(api, "onRunStatus").mockImplementation(async (handler) => {
      emitRunStatus = handler;
      return () => undefined;
    });
    render(<App />);

    await waitFor(() => expect(emitRunStatus).toBeTypeOf("function"));
    act(() => emitRunStatus?.({
      profileId: "profile-studio-api",
      status: event.status,
      pid: null,
      message: event.message,
      unexpected: event.unexpected,
      timestamp: Date.now(),
    }));

    expect(await screen.findByRole("alert")).toHaveTextContent(event.message);
  });

  it("spans only visible columns for port details in compact layouts", async () => {
    const originalMatchMedia = window.matchMedia;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn().mockReturnValue({
        matches: true,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    });

    try {
      const user = userEvent.setup();
      render(<App />);
      await screen.findByRole("heading", { name: "Launch profiles" });
      await user.click(screen.getByRole("button", { name: "Ports" }));
      await user.click(screen.getByRole("button", { name: "View details for port 5173" }));

      expect(document.querySelector(".port-detail-row td")).toHaveAttribute("colspan", "4");
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
    }
  });

  it("imports a discovered project with structured launch arguments", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("heading", { name: "Launch profiles" });

    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(screen.getByRole("button", { name: "Import project" }));

    const pathInput = screen.getByLabelText("Project directory");
    await user.type(pathInput, "D:\\CodexProject\\personal-projects\\signal-board");
    await user.click(screen.getByRole("button", { name: "Inspect directory" }));

    expect(await screen.findByDisplayValue("Signal Board")).toBeInTheDocument();
    expect(screen.getByDisplayValue("npm.cmd")).toBeInTheDocument();
    expect(screen.getByText("Observed")).toBeInTheDocument();
    expect(screen.getByLabelText("Profile 1 argument 1")).toHaveValue("run");
    expect(screen.getByLabelText("Profile 1 argument 2")).toHaveValue("dev");
    expect(screen.getByLabelText("Profile 1 expected port 1")).toHaveValue(3100);
    await user.click(screen.getByRole("button", { name: "Save project" }));

    expect(await screen.findByText("Signal Board")).toBeInTheDocument();
  });

  it("scans a development root and imports only selected projects", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("heading", { name: "Launch profiles" });

    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(screen.getByRole("button", { name: "Import project" }));
    await user.click(screen.getByRole("button", { name: "Development root" }));

    await user.type(
      screen.getByLabelText("Development root"),
      "D:\\CodexProject\\personal-projects\\batch-root",
    );
    await user.click(screen.getByRole("button", { name: "Scan root" }));

    expect(await screen.findByText("Signal Console")).toBeInTheDocument();
    expect(screen.getByText("Worker Lab")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Select Signal Console" })).toBeChecked();
    const worker = screen.getByRole("checkbox", { name: "Select Worker Lab" });
    expect(worker).toBeChecked();
    await user.click(worker);
    await user.click(screen.getByRole("button", { name: "Import 1 project" }));

    expect(await screen.findByRole("heading", { name: "Signal Console" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Worker Lab" })).not.toBeInTheDocument();
  });

  it("opens a bounded session log drawer and clears it", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("heading", { name: "Launch profiles" });

    await user.click(screen.getByRole("button", { name: "View Web logs" }));
    const drawer = await screen.findByRole("dialog", { name: "Web" });
    expect(within(drawer).getByText("VITE ready in 361 ms")).toBeInTheDocument();

    await user.click(within(drawer).getByRole("button", { name: "Clear session logs" }));
    expect(await within(drawer).findByText("No session logs")).toBeInTheDocument();
  });
});
